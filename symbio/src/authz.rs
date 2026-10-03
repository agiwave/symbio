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

use std::sync::OnceLock;

use crate::symbio_core::{PermissionMatrix, VisScope};

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
        "produce.artifact",
        "assign.work",
    ],
    VisScope::ThreadPrivate,
)];

static MATRIX: OnceLock<PermissionMatrix> = OnceLock::new();

/// 本机授权矩阵（懒构造一次；构造失败 ⇒ 空矩阵 = 谁都不许，并告警）。
///
/// 两道闸（写侧收束、读侧可见域）都取这一份，保证「表只有一张」。
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
