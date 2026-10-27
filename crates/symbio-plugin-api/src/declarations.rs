//! 装配期的声明表 —— **链接期收集、进程内冻结一次**。
//!
//! ## 它补的是哪一格
//!
//! 声明表（事件 / 事实 / 视图 / 端口 / 执行体 / 行）本来由 `plugins/loop` 私有装载：
//! `loop_rows::load` 是 `Builder::declare` 的唯一生产调用点。于是别的插件即使想声明
//! 一个新事实或一条新视图，也只能改 loop 的文件——这就是「加能力要改别人的模块」的
//! 剩余来源（DE-024 D-198）。
//!
//! 本模块把声明表变成**装配期产物**：
//!
//! ```text
//! owner 插件 ──submit_declarations!──▶ inventory（编译期随二进制生效）
//! 装配方     ──assemble_declarations──▶ Builder::declare × N ──seal──▶ Frozen
//! 读侧       ──sealed_declarations / declared_event_cell──▶ 只读表
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
//!
//! 迁移期例外：写闸在冻结表查不到名字时回退 `event::grid::CELLS`（D-199）。回退只在
//! **查表未命中**时发生，不掩盖 `seal` 的失败；最后一批业务事件迁出后回退删除。

use std::sync::{Arc, OnceLock};

use symbio_core::ring::Gap;
use symbio_core::{Builder, Entity, EventDecl, Frozen, PluginDeclarations, Verb};

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
/// symbio_plugin_api::submit_declarations!("plugins/notes", notes_declarations);
///
/// fn notes_declarations() -> PluginDeclarations<'static> {
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
/// 顺序按 owner 排序：行的裁决只看 `(prio, name, key)`，注册顺序不参与；排序只是让
/// 缺口报告的条目顺序稳定。同一 owner 的多次提交保持 inventory 的顺序。
pub fn assemble_declarations() -> Result<Arc<Frozen>, Vec<Gap>> {
    let mut submissions: Vec<&DeclarationsDecl> = inventory::iter::<DeclarationsDecl>().collect();
    submissions.sort_by_key(|d| d.owner);

    let mut builder = Builder::new();
    for d in submissions {
        builder.declare(d.owner, (d.make)());
    }
    // 「每一档守卫角色都有人住」由门禁判仓库（CS-008），不在这里判：本函数也会被
    // **只链接了部分插件**的用例调到（session 的单测里没有 loop 的声明表），
    // 那种子集装配报红是假红。
    builder.seal()
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

/// 判据的**词汇面**（`&[EventDecl]`）：core 的守卫问「这条名字扮演什么角色」时读它。
///
/// 返回 `'static` 是因为表住在进程冻结里；冻结失败 ⇒ `None`，调用方回退内核表
/// （迁移期口径，DE-024 D-199 / D-203）。
pub fn declared_vocab() -> Option<&'static [EventDecl]> {
    SEALED
        .get_or_init(seal_once)
        .as_ref()
        .ok()
        .map(|frozen| frozen.events())
}

/// 这个名字的格位：先问冻结表，未命中返回 `None`（迁移期由写闸回退 `CELLS`）。
///
/// 冻结失败同样返回 `None`——调用方要么回退内核表（已知名字），要么按「网格之外」
/// 拒绝新名字（fail-closed）。这里**不**把失败降级成 `CELLS` 的静默通过。
pub fn declared_event_cell(name: &str) -> Option<(Entity, Verb)> {
    declared_vocab()
        .and_then(|vocab| vocab.iter().find(|e| e.name == name))
        .map(|e| e.cell)
}

#[cfg(test)]
#[path = "declarations.test.rs"]
mod tests;
