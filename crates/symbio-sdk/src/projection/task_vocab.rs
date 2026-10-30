//! 任务族的词汇表（[DE-024](../../../../docs/003-design/024-能力横向扩充的通路.md "DE-024-能力横向扩充的通路") D-206）。
//!
//! `readyset` 按**一族**名字分支（五格：建节点 / 推进 / 挂起 / 终态 / 租约）。
//! 名字不写在 core 里——和 D-205 那条单名字的通路同一形状：折叠读哪几条名字由调用方在
//! 构造期交进来，core 只留折叠的形状。
//!
//! ## 字段为什么按算法角色命名
//!
//! 不是修辞。`task.asserted`（终态）与 `task.assigned`（租约）落在**同一格**
//! `Task × Asserted`：按格子命字段会得到两个同名字段，按事件名命字段则这张表降级成
//! 名字表的别名。按「这一格在我的算法里算什么」命名字段，才是这一族真正的闭集。
//!
//! ## 交出整族的代价，也是它的收益
//!
//! 读一个折叠就要交出整族——`symbio_plugin_session::autonomy` 只用 `held` 与 `terminal`
//! 两格，仍然收到五格。换来的是：owner 交出的是这一族的**完整**拼写，同族的几个折叠
//! 不可能各读各的拼写而漂移。

/// 任务族的一格在折叠算法里的角色（不是事件名，也不是格子）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskRole {
    /// 建节点：调度候选集在这里成立。
    Opened,
    /// 推进：一次普通执行事实；在 `readyset` 里同时是**解除挂起**。
    Progressed,
    /// 挂起：抢占式干预留下的痕迹。
    Held,
    /// 终态：验收通过，任务闭环。
    Terminal,
    /// 租约：派出去但未到期时不再被派。
    Lease,
}

/// 任务族的拼写表，由 owner 在构造期交给折叠（D-206 裁决一）。
///
/// 字段是 `&'a str` 而非 `'static`：词汇是数据，夹具与测试交自己的字面量不必升级成常量。
/// 折叠内部持有的是 [`TaskVocab::bind`] 之后的所有权表（`Projection` 的闭包是 `'static`）。
pub struct TaskVocab<'a> {
    pub opened: &'a str,
    pub progressed: &'a str,
    pub held: &'a str,
    pub terminal: &'a str,
    pub lease: &'a str,
}

/// 折叠闭包里持有的所有权表（同一族，字段换成 `String`）。
pub(crate) struct BoundTaskVocab {
    opened: String,
    progressed: String,
    held: String,
    terminal: String,
    lease: String,
}

impl TaskVocab<'_> {
    /// 交给闭包持有的那一份（D-206：词汇进构造期，一次拷贝，不在热路径上重复问）。
    pub(crate) fn bind(&self) -> BoundTaskVocab {
        BoundTaskVocab {
            opened: self.opened.to_string(),
            progressed: self.progressed.to_string(),
            held: self.held.to_string(),
            terminal: self.terminal.to_string(),
            lease: self.lease.to_string(),
        }
    }
}

impl BoundTaskVocab {
    /// 这一格事件在任务族里的角色——不认识的那一格返回 `None`，由折叠显式跳过。
    ///
    /// 名字比对只做这一次、且只在这张表上做：折叠的分支变成枚举分支之后，「这一族有哪几格」
    /// 在折叠内部是编译期的闭集（D-206 裁决三）。
    pub(crate) fn role(&self, kind: &str) -> Option<TaskRole> {
        match kind {
            k if k == self.opened => Some(TaskRole::Opened),
            k if k == self.progressed => Some(TaskRole::Progressed),
            k if k == self.held => Some(TaskRole::Held),
            k if k == self.terminal => Some(TaskRole::Terminal),
            k if k == self.lease => Some(TaskRole::Lease),
            _ => None,
        }
    }
}
