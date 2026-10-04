//! 本机部署的**授权策略表**（写侧 grants + 读侧可见域，[plan/01 §7](../../docs/plan/01-核心架构.md)）。
//!
//! ## 为什么这张表住 crate 根
//!
//! 表里写的是**部署事实**——本机有哪几个主体、各持有什么能力。三方的边界是：
//!
//! | 谁 | 拥有 | 不拥有 |
//! |---|---|---|
//! | core `governance` | 能力闭集（7 个封顶）、成对性与两条判定的**机制** | 「谁」是部署事实，core 不知道 |
//! | 本模块 | 主体清单与各自的 grants / 可见域 | 判定语义（复用 core 的函数） |
//! | 各插件 | 自己那道闸上「现在该判谁」 | 表本身——插件互不可见（E-009），任一插件持有它都会逼别的插件跨插件引用 |
//!
//! 同时对宿主与全部插件可见的位置只有 core 与 crate 根（与 [`init`](crate::init)
//! 同层），而 core 不该持有部署事实 ⇒ 落在这里。
//!
//! ## fail-closed 三层
//!
//! 1. **构造期**：认不出的能力名 ⇒ 拒绝整体构造（`PolicyError`，core 定义），
//!    降级为**空矩阵**并告警——不是「跳过那一行」，笔误不能变成一条悄悄生效的拒绝；
//! 2. **写侧**：`v2_bridge` 收束入格前 [`can_reply`](PermissionMatrix::can_reply)，
//!    未持有 ⇒ 拒绝入格（该轮留在未收束态，`check_all` 会把它报出来）；
//! 3. **读侧**：`stats` 按读方声明判 [`can_see`](PermissionMatrix::can_see)——属主
//!    是**内容的属主**（`SESSION_OWNER`，部署事实）而非事件作者，于是今天的判读
//!    就是「属主放行、其余整体拒绝」；`thread_private` 缺省（C10）。
//!
//! ## 两张表里的「人」
//!
//! `user.message` **不入能力表**：能力按「智能体的权威动作种类」划分，人的发言
//! 不是动作种类（落不进去就走 [03 §3](../../docs/plan/03-演进与验证.md) 的 ADR）。
//! 读侧因此也不靠矩阵成员身份，而靠 **owner 相等**：`thread_private` 下属主是
//! 本机会话的属主，语义见 `governance::can_see`。
//!
//! ## 本表会怎么长
//!
//! 多主体加固（[plan/11 批 2](../../docs/plan/11-多执行器与多主体加固实施方案.md) /
//! 04 §3.1 批⑧）落地时，子智能体各成一行——行的形状不变，仍是
//! `(主体, 能力名, 可见域)`。

use std::borrow::Cow;
use std::sync::OnceLock;

use crate::symbio_core::{PermissionMatrix, VisScope, AGENT_PREFIX};

/// 本机主智能体的身份——**表里那一行的名字**与 `v2_bridge` 收束事件的 `actor`
/// 是同一个常量：授权判定的对象若与实际写入的主体各写一份字符串，两边迟早漂移
/// 到「判的是 A、写的是 B」而没有任何东西会变红。
pub(crate) const PRINCIPAL_MAIN: &str = "agent:main";

/// 本机用户的身份——`v2_bridge` 用户发言事件的 `actor`。
pub(crate) const PRINCIPAL_USER: &str = "user";

/// 本机会话**内容的属主**：`thread_private`（C10 缺省）下「谁能读到」等同于
/// 「是不是属主」，所以属主取值就是读侧闸的答案。
///
/// 今天它 = 本机用户（`[会话] ≈ [线程]`，S4 的 thread 实体落地后取线程属主）。
/// 属主是**部署事实**，与读方的 `principal` 一样不来自请求——请求方声明自己是谁
/// 已是上限，再声明「内容归谁」等于给自己授权。
pub(crate) const SESSION_OWNER: &str = PRINCIPAL_USER;

/// **外部执行**能力名（`ProduceArtifact` 的书写形态）——熔断闸门的授权判据
/// （[roadmap/S09 §6](../../docs/plan/roadmap/S09-外部执行与熔断.md) 验收 1）。
///
/// 提成常量而不是在闸门那里再写一遍字面量：能力名只有一个拼法，写两处迟早漂移成
/// 「表授予 A、闸门判 B」，而这种错位**没有任何东西会变红**（两边都是合法能力名）。
pub(crate) const CAP_EXTERNAL_EXECUTION: &str = "produce.artifact";

/// 本机主体清单：`(主体, 能力名, 可见域)`。能力名的闭集与拼写由
/// [`PermissionMatrix::from_names`] 校验（认不出即拒绝构造）。
///
/// `agent:main`（主智能体）持 6 项，**刻意不持 `assert.verification`**：C15 不自验
/// ——验证者 ≠ 产出者是构造期约束（`PermissionMatrix::new` 会拒绝双持），本机目前
/// 没有独立验证者行，验证能力因此无人持有。
const ROWS: &[(&str, &[&str], VisScope)] = &[(
    PRINCIPAL_MAIN,
    &[
        "judge.intent",
        "reply.first",
        "reply.append",
        "define.work",
        CAP_EXTERNAL_EXECUTION,
        "assign.work",
    ],
    VisScope::ThreadPrivate,
)];

static MATRIX: OnceLock<PermissionMatrix> = OnceLock::new();

/// 本机会话的**主体派生**（部署事实 → [`PRINCIPAL_MAIN`] 同族的取值点）：
/// 会话选定了哪个 agent，这轮所有断言就是哪个主体；未选 ⇒ 主智能体。
///
/// 前缀取自 core 的可见域约定 [`AGENT_PREFIX`](crate::symbio_core::view::AGENT_PREFIX)
/// ——「什么形状算智能体身份」只定义一次，可见域判据与身份派生共用它，
/// 两处各写一个字面量迟早漂移成「判的是 A、写的是 B」。
///
/// 平凡值（S08 §4）：今天所有会话未选智能体 ⇒ 恒为 `agent:main`，与接线前
/// `v2_bridge` 里那个写死的常量**逐字一致**。
pub(crate) fn principal_of(agent_id: Option<&str>) -> String {
    match agent_id.map(str::trim).filter(|s| !s.is_empty()) {
        None => PRINCIPAL_MAIN.to_string(),
        //调用方给的已是主体名（`agent:<id>`）就原样认下——派生点不二次加前缀。
        Some(id) if id.starts_with(AGENT_PREFIX) => id.to_string(),
        Some(id) => format!("{AGENT_PREFIX}{id}"),
    }
}

/// **该主体当次判定**用的矩阵（行的形状不变：`(主体, 能力名, 可见域)`，
/// 见本模块文档「本表会怎么长」）。
///
/// 本机今天只有一张静态表，但**判定的对象是实际写入的那个主体**：主智能体之外
/// 的 `agent:<id>`（子智能体 / 对等体）要能写自己的收束格，就必须有自己那一行。
/// 行的**内容** = 主智能体那一行的能力集——**身份分层 ≠ 权限分层**，本机所有
/// 智能体同角色（S08 §4 平凡值）；按主体展开 grants 做能力分级是 S13 的工序。
///
/// 非 `agent:*` 的主体（`user` 及一切未登记名）走静态表：表里没有它 ⇒
/// `can_reply` 为假 ⇒ fail-closed。少一行是一条**拒绝**，不是一条漏判。
pub(crate) fn matrix_for(principal: &str) -> Cow<'static, PermissionMatrix> {
    if principal == PRINCIPAL_MAIN || !principal.starts_with(AGENT_PREFIX) {
        return Cow::Borrowed(production_matrix());
    }
    let (main_caps, main_scope) = ROWS
        .first()
        .map(|(_, caps, scope)| (*caps, *scope))
        .unwrap_or((&[], VisScope::ThreadPrivate));
    Cow::Owned(
        PermissionMatrix::from_names(&[(principal, main_caps, main_scope)]).unwrap_or_else(|why| {
            crate::plugin_warn!(
                "authz",
                "[governance] 主体 `{principal}` 的行构造失败，按 fail-closed 降级为空矩阵：{why}"
            );
            PermissionMatrix::from_names(&[]).expect("空授权表无主体可校验，不可能违反成对性")
        }),
    )
}

/// 本机授权矩阵（懒构造一次；构造失败 ⇒ 空矩阵 = 谁都不许，并告警）。
///
/// 写侧收束、读侧可见域与外部执行闸门的授权判据都取这一份，保证「表只有一张」——
/// 各判各的就会漂移成「闸放行、表说没有」。
pub(crate) fn production_matrix() -> &'static PermissionMatrix {
    MATRIX.get_or_init(|| {
        PermissionMatrix::from_names(ROWS).unwrap_or_else(|why| {
            crate::plugin_warn!(
                "authz",
                "[governance] 授权表构造失败，按 fail-closed 降级为空矩阵（{}）：{}",
                ROWS.len(),
                why
            );
            PermissionMatrix::from_names(&[]).expect("空授权表无主体可校验，不可能违反成对性")
        })
    })
}

#[cfg(test)]
#[path = "authz.test.rs"]
mod tests;
