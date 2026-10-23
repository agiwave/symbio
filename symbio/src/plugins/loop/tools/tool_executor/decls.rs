//! 工具自声明的行为位：执行器**按声明判，不按名字判**。
//!
//! 三处闸门（淘汰闸 / 承诺记账 / 任务表）若各自把工具名字面量抄一遍，
//! 工具改名时**没有测试会失败**——闸门静默失效（J3 要消灭的形态）。本位把它们
//! 的管辖对象改成 `CapabilityMeta` 上的
//! **自声明字段**，与 `risk` / `context_retention` 同一条纪律：工具说「我受这项
//! 管辖」，闸门只负责读声明。
//!
//! ## 读的时机与来源
//!
//! 批首经收集管线的读侧助手 `tool_metas`
//! 读一次工具元数据——与
//! `execute_tool_async` 解析线上名同一来源；一次读给
//! 整批复用，与 `BreakerInputs` / `EliminationInputs` 的批首读同口径（每个调用点
//! 各重放一次事实源，闸门自己就变成时延源了）。
//!
//! ## 匹配口径
//!
//! - **淘汰闸 / 承诺记账**：线上全名**等值**（`agent_run` 的线上名就是短名本身）。
//! - **任务表**：线上全名**取末段**比短名（`local/todo_write` ⇒ `todo_write`）。
//!
//! 集合以注册表给出的 `meta.name` 为键——作用域投影（子智能体的改名包装器）只换
//! 名字、原样透传行为位，所以**闸门跟随工具进入子作用域**：那正是「换个会话绕不
//! 开淘汰」要的读法（按字面量等值判，前缀名一个都不命中）。

use std::collections::HashSet;

use crate::symbio_core::CapabilityMeta;

/// 三类按名闸门的管辖集合（键 = 注册表里的 `meta.name`）。
#[derive(Debug, Default)]
pub struct ToolDecls {
    /// 提供 `assign.work`：淘汰闸管辖（`elimination.delegation_revoked` 时拒止）
    assign_work_tools: HashSet<String>,
    /// 跨主体立约型：承诺记账对象
    promise_tools: HashSet<String>,
    /// 任务表写入：写侧资格闸与任务声明记账的管辖对象
    task_table_tools: HashSet<String>,
}

impl ToolDecls {
    /// 按值复用同一份元数据（ 的执行策略与本类型是两个消费者，批首只遍历
    /// 一次收集管线；没有 visitor 时元数据为空 ⇒ 三个集合皆空 ⇒ 三道闸都不命中）。
    pub(crate) fn of_metas(metas: &[CapabilityMeta]) -> Self {
        let mut decls = Self::default();
        for meta in metas {
            decls.absorb(meta);
        }
        decls
    }

    /// 把一份能力元数据的行为位收进集合（键取 `meta.name`）。
    fn absorb(&mut self, meta: &CapabilityMeta) {
        if meta.provides_assign_work {
            self.assign_work_tools.insert(meta.name.clone());
        }
        if meta.records_delegation_promise {
            self.promise_tools.insert(meta.name.clone());
        }
        if meta.owns_task_table {
            self.task_table_tools.insert(meta.name.clone());
        }
    }

    /// 本调用是否受淘汰闸管辖（线上全名等值判据，与机制化之前逐字一致）。
    pub fn provides_assign_work(&self, wire_name: &str) -> bool {
        self.assign_work_tools.contains(wire_name.trim())
    }

    /// 本调用是否立约（同上，线上全名等值判据）。
    pub fn records_delegation_promise(&self, wire_name: &str) -> bool {
        self.promise_tools.contains(wire_name.trim())
    }

    /// 本调用是否写任务表（线上全名取末段比短名，与机制化之前逐字一致）。
    pub fn owns_task_table(&self, wire_name: &str) -> bool {
        self.task_table_tools
            .contains(wire_name.trim().rsplit('/').next().unwrap_or_default())
    }
}

#[cfg(test)]
#[path = "decls.test.rs"]
mod tests;
