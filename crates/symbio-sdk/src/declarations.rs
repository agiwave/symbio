//! 装配期的声明表 —— **链接期收集、进程内冻结一次**。
//!
//! ## 它补的是哪一格
//!
//! 声明表（事件 / 事实 / 视图 / 端口 / 执行体 / 行）本来由 `crates/symbio-plugin-loop` 私有装载：
//! `loop_rows::load` 是 `Builder::declare` 的唯一生产调用点。于是别的插件即使想声明
//! 一个新事实或一条新视图，也只能改 loop 的文件——这就是「加能力要改别人的模块」的
//! 剩余来源（DE-024 D-198）。
//!
//! 本模块把声明表变成**装配期产物**：
//!
//! ```text
//! owner 插件 ──submit_declarations!──▶ inventory（编译期随二进制生效）
//! 装配方     ──assemble_declarations──▶ Builder::declare × N ──seal──▶ Frozen
//! 读侧       ──sealed_declarations──▶ 只读表（词汇读法 `vocab` 住 `symbio-plugin-api`）
//! ```
//!
//! ## 为什么是 `inventory`，不是生成清单
//!
//! 声明是 Rust 数据（`FactDecl` / `ViewDecl` / `Decl` 的每个字段都是 `'static` 切片），
//! 不是 YAML 能表达的形状；而「哪些 crate 在构建里」已经是链接事实。`inventory` 与
//! `submit_object_creator!` 同一条机制：提交点住插件，收集点住装配方，插件之间零依赖
//! （E-009 仍然成立——它们只依赖本 crate，本 crate 不认识任何具体插件）。
//!
//! ## 失败是**整体失败**，不是逐条降级
//!
//! `seal` 一次报全所有缺口；`sealed_declarations` 缓存的是同一个 `Result`。任何一条
//! 声明不合格 ⇒ 冻结表不存在 ⇒ 读侧拿到同一个 `Err`。不存在「半张表继续跑」——
//! 这正是把装载口从 loop 私有单例挪到装配方的理由（DE-015 §3.3）。

use std::sync::{Arc, OnceLock};

use symbio_kernel::decl::ring::Gap;
use symbio_kernel::decl::{Builder, Frozen, PluginDeclarations};
use symbio_kernel::event::EventDecl;

/// 一个 owner 交来的声明表。`make` 是**构造器**而不是值：表里有 `Vec`（执行体绑定
/// 投影），每次装配都要现取一份；静态切片本身仍是 `'static`，不产生运行期注册通道。
pub struct DeclarationsDecl {
    pub owner: &'static str,
    pub make: fn() -> PluginDeclarations<'static>,
}

inventory::collect!(DeclarationsDecl);

/// 提交一份声明表。在插件 crate 的模块作用域调用一次即可。
///
/// ```ignore
/// symbio_plugin_api::submit_declarations!("crates/symbio-plugin-glossary", glossary_declarations);
///
/// fn glossary_declarations() -> PluginDeclarations<'static> {
///     todo!("由插件自己的声明模块返回六张表")
/// }
/// ```
///
/// 宏用 `$crate::inventory` 与 `$crate::DeclarationsDecl` 展开（卫生）：调用方不需要
/// 声明 inventory，也不需要认识收集类型。
#[macro_export]
macro_rules! submit_declarations {
    ($owner:expr, $make:expr) => {
        $crate::inventory::submit! {
            $crate::DeclarationsDecl { owner: $owner, make: $make }
        }
    };
}

/// 把链接进来的全部声明表装成一张冻结表。
///
/// 顺序按 owner 排序：行的裁决只看 `(phase, anchor, name, key)`，注册顺序不参与；排序只是让
/// 缺口报告的条目顺序稳定。同一 owner 的多次提交保持 inventory 的顺序。
pub fn assemble_declarations() -> Result<Arc<Frozen>, Vec<Gap>> {
    let mut submissions: Vec<&DeclarationsDecl> = inventory::iter::<DeclarationsDecl>().collect();
    submissions.sort_by_key(|d| d.owner);

    let mut builder = Builder::new();
    for d in submissions {
        builder.declare(d.owner, (d.make)());
    }
    // 逐行写入名单随装配一起交：共享执行体（`plugin_row`）的多行各自写哪一格只有
    // 实现知道，执行体的并集表达不了（D-248）。冻结表与运行时对账因此同源。
    let row_writes = crate::plugin::rows::row_writes();
    // 「每一档守卫角色都有人住」由整仓判据 CS-011 判（`shared-event-vocab-audit`），
    // 不在这里判：本函数也会被**只链接了部分插件**的用例调到（session 的单测里没有
    // loop 的声明表），那种子集装配报红是假红。
    let frozen = builder.seal(&row_writes)?;
    // D-339：`EventDecl::owner` 的第二态必须是登记过的具名主体。查表用的是**部署表**
    // （各插件 `PLUGIN.yml` 的 `principal_*` 行），因此在子集装配里同样成立——注册表
    // 不随链接了哪些插件变化。
    assert_owners_registered(&frozen);
    Ok(frozen)
}

/// 装配期的归属校验（D-339）：`@principal` 之外，owner 必须能在主体注册表里查到。
///
/// 为什么放在这里而不是内核 `seal`：注册表是部署事实（`symbio-sdk` 的授权表），
/// 内核不依赖 `symbio-sdk`；装配方同时看得见两张表，校验因此只有一处。
///
/// 为什么是断言而不是一条 `Gap`：声明里写了一个不存在的主体不是「表还能凑合用」的
/// 缺口——它是**声明表本身被改坏了**，装配必须在启动前停住（写口没有第二个可问的表）。
/// 名字来自 `'static` 声明，运行期拼出来的字符串进不来。
fn assert_owners_registered(frozen: &Frozen) {
    let matrix = crate::authz::production_matrix();
    for event in frozen.events() {
        let owner = event.owner;
        if owner == symbio_kernel::event::OWNER_PRINCIPAL {
            continue;
        }
        // 人的主体不在能力表里（发言不是「智能体的权威动作种类」，D-099），但它是
        // 登记过的主体常量——属第二态的合法值。
        if owner == crate::keys::PRINCIPAL_USER {
            continue;
        }
        assert!(
            matrix.is_deployed(owner),
            "事件 `{}` 的声明归属 `{}` 不在主体注册表里（各插件 PLUGIN.yml 的 `principal_*` 行）；\
             装配期拒绝未登记的主体名（D-339）",
            event.name,
            owner
        );
    }
}

/// 进程内冻结一次。`OnceLock` 缓存的是**同一个 `Result`**（成功或全部缺口），
/// 不是可变注册表——`Frozen` 没有 `&mut self` 方法。
static SEALED: OnceLock<Result<Arc<Frozen>, Vec<Gap>>> = OnceLock::new();

fn seal_once() -> Result<Arc<Frozen>, Vec<Gap>> {
    let sealed = assemble_declarations();
    if let Err(gaps) = &sealed {
        tracing::error!(
            target: "symbio::declarations",
            gaps = ?gaps,
            "声明表装载失败：写闸与义务环将拒绝使用未冻结的词汇"
        );
    }
    sealed
}

pub fn sealed_declarations() -> Result<Arc<Frozen>, Vec<Gap>> {
    SEALED.get_or_init(seal_once).clone()
}

/// 冻结表的事件面：不复制表，只借出缓存里的那一份（与进程同寿）。
///
/// 词汇读法（冻结表缺席时回落清单生成物）住 `symbio-plugin-api`：回落表是生成物，
/// 而本 crate 的生产代码零事件名字面量（CS-008 / CP-004，`D-415`）。
pub fn declared_vocab() -> Option<&'static [EventDecl]> {
    SEALED
        .get_or_init(seal_once)
        .as_ref()
        .ok()
        .map(|frozen| frozen.events())
}

#[cfg(test)]
#[path = "declarations.test.rs"]
mod tests;
