//! 工具自声明的行为名：执行器**按声明判，不按名字判**。
//!
//! 声明闸（能力包的 `ToolBehavior::before`）与效果端口（`after` / `annotate`）若各自
//! 把工具名字面量抄一遍，工具改名时**没有测试会失败**——闸门静默失效（J3 要消灭的
//! 形态）。本位把「谁受什么管辖」改成 `CapabilityMeta.behaviors` 上的**开放集合**
//! （DE-025 D-210），与 `risk` / `context_retention` 同一条纪律：工具说「我受这项
//! 管辖」，宿主只负责把这份声明原样交给端口。名字本身住
//! `symbio_plugin_api::behaviors`——本模块不认识工具，也不往内核加字段。
//!
//! ## 读的时机与来源
//!
//! 批首经收集管线的读侧助手 `tool_metas`
//! 读一次工具元数据——与
//! `execute_tool_async` 解析线上名同一来源；一次读给
//! 整批复用，与 `BreakerInputs` 的批首读同口径（每个调用点各重放一次事实源，
//! 闸门自己就变成时延源了）。
//!
//! ## 匹配口径
//!
//! [`ToolDecls::behaviors_of`]：线上全名先**等值**、再**取末段**——挂载点前缀不改变
//! 「这个工具声明了什么」，子作用域的改名（`agent_probe-agent_todo_write`）没有 `/`
//! 走等值那一支。选实现（声明闸 / 效果端口）与「管不管这次调用」因此是同一个口径：
//! 一份声明只有一个读法。
//!
//! 集合以注册表给出的 `meta.name` 为键——作用域投影（子智能体的改名包装器）只换
//! 名字、原样透传行为名，所以**闸门跟随工具进入子作用域**：那正是「换个会话绕不
//! 开淘汰」要的读法（按字面量等值判，前缀名一个都不命中）。

use std::collections::HashMap;

use symbio_core::CapabilityMeta;

/// 线上全名 → 声明的行为名（声明闸与效果端口共用的选实现依据，DE-025 §2.1）。
#[derive(Debug, Default)]
pub struct ToolDecls {
    declared: HashMap<String, &'static [&'static str]>,
}

impl ToolDecls {
    /// 按值复用同一份元数据（执行策略与本类型是两个消费者，批首只遍历
    /// 一次收集管线；没有 visitor 时元数据为空 ⇒ 声明为空 ⇒ 没有闸命中）。
    pub(crate) fn of_metas(metas: &[CapabilityMeta]) -> Self {
        let mut decls = Self::default();
        for meta in metas {
            decls.absorb(meta);
        }
        decls
    }

    /// 把一份能力元数据的行为位收进表（键取 `meta.name`）。
    fn absorb(&mut self, meta: &CapabilityMeta) {
        self.declared.insert(meta.name.clone(), meta.behaviors);
    }

    /// 本调用声明的**全部**行为名（原样切片，交声明闸与效果端口选实现用）。
    ///
    /// 口径先**等值**、再**取末段**：注册表的键是 `CapabilityMeta.name`（短名），而
    /// 线上名带挂载点前缀（`local/todo_write`、`agent_scope/todo_write`）——只判等值
    /// 会让「挂了前缀的工具」在端口里静默没有效果（J3 的形态：不是报错，是少
    /// 记账）。子作用域的改名（`agent_probe-agent_todo_write`）没有 `/`，走等值那一支。
    pub fn behaviors_of(&self, wire_name: &str) -> &'static [&'static str] {
        let wire = wire_name.trim();
        if let Some(declared) = self.declared.get(wire) {
            return declared;
        }
        let short = wire.rsplit('/').next().unwrap_or_default();
        self.declared.get(short).copied().unwrap_or(&[])
    }
}

#[cfg(test)]
#[path = "decls.test.rs"]
mod tests;
