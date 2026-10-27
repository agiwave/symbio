//! 工具自声明的行为名：执行器**按声明判，不按名字判**。
//!
//! 三处闸门（淘汰闸 / 承诺记账 / 任务表）若各自把工具名字面量抄一遍，
//! 工具改名时**没有测试会失败**——闸门静默失效（J3 要消灭的形态）。本位把它们
//! 的管辖对象改成 `CapabilityMeta.behaviors` 上的**开放集合**（DE-025 D-210），
//! 与 `risk` / `context_retention` 同一条纪律：工具说「我受这项管辖」，闸门只负责
//! 读名字。名字本身住 `symbio_plugin_api::behaviors`——本模块不认识工具，也不往
//! 内核加字段。
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
//! 名字、原样透传行为名，所以**闸门跟随工具进入子作用域**：那正是「换个会话绕不
//! 开淘汰」要的读法（按字面量等值判，前缀名一个都不命中）。

use std::collections::{HashMap, HashSet};

use symbio_core::CapabilityMeta;
use symbio_plugin_api::behaviors;

/// 两道按名闸门的管辖集合（键 = 注册表里的 `meta.name`）+ 效果端口的选实现依据。
///
/// 承诺那一族**不再在这里**：它的「管不管这次调用」由效果端口按声明判
/// （[`Self::behaviors_of`]，DE-025 D-207）——宿主不需要为它维护第四个集合。
#[derive(Debug, Default)]
pub struct ToolDecls {
    /// 提供 `assign.work`：淘汰闸管辖（`elimination.delegation_revoked` 时拒止）
    assign_work_tools: HashSet<String>,
    /// 任务表写入：写侧资格闸与任务声明记账的管辖对象
    task_table_tools: HashSet<String>,
    /// 线上全名 → 声明的行为名（效果端口的选实现依据，DE-025 §2.1）。
    declared: HashMap<String, &'static [&'static str]>,
}

impl ToolDecls {
    /// 按值复用同一份元数据（执行策略与本类型是两个消费者，批首只遍历
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
        self.declared.insert(meta.name.clone(), meta.behaviors);
        if meta.declares(behaviors::ASSIGN_WORK) {
            self.assign_work_tools.insert(meta.name.clone());
        }
        if meta.declares(behaviors::TASK_TABLE) {
            self.task_table_tools.insert(meta.name.clone());
        }
    }

    /// 本调用是否受淘汰闸管辖（线上全名等值判据，与机制化之前逐字一致）。
    pub fn provides_assign_work(&self, wire_name: &str) -> bool {
        self.assign_work_tools.contains(wire_name.trim())
    }

    /// 本调用是否写任务表（线上全名取末段比短名，与机制化之前逐字一致）。
    pub fn owns_task_table(&self, wire_name: &str) -> bool {
        self.task_table_tools
            .contains(wire_name.trim().rsplit('/').next().unwrap_or_default())
    }

    /// 本调用声明的**全部**行为名（原样切片，交效果端口选实现用）。
    ///
    /// 与上面三个谓词同一份元数据：谓词是「某道闸管不管它」，本函数是「它的效果由
    /// 谁记」。口径取线上全名**等值**——效果端口按注册表的 `meta.name` 选实现，
    /// 不做末段匹配（末段是任务表闸的历史口径，不是通用规则）。
    pub fn behaviors_of(&self, wire_name: &str) -> &'static [&'static str] {
        self.declared.get(wire_name.trim()).copied().unwrap_or(&[])
    }
}

#[cfg(test)]
#[path = "decls.test.rs"]
mod tests;
