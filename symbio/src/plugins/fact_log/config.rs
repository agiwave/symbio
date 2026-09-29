//! fact_log 的配置（`<本插件目录>/PLUGIN.yml`）。
//!
//! 两个字段：**派生不派生**（开关）、**派生多少**（上限）。默认值即出厂行为，
//! 字段真源是本结构——表单定义从 [`FactLogConfig::default`] 读出，不写第二份字面量。

use serde::{Deserialize, Serialize};

/// 事实日志配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactLogConfig {
    /// 是否派生并登记事实源。
    ///
    /// 关掉 = 本插件不贡献事实，但**不报错**：消费方取不到事实源时按"无事实"处理
    /// （J2 平凡值）。这是"可选插件"的字面实现——关掉它，系统退化成没有事实日志的
    /// 形态，其余部分完好。
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// 单次派生返回的最大事实条数（防一次查询扫爆内存）。
    ///
    /// 派生是只读的，**超限不报错**而是截断并随结果标注——静默截断会让消费方
    /// 以为"只有这些事实"，所以截断必须可见（返回 `truncated` 标记）。
    #[serde(default = "default_max_facts")]
    pub max_facts: usize,
}

fn default_true() -> bool {
    true
}

/// 4096 条：一次对话数十轮的量级，远超正常单次查询所需
fn default_max_facts() -> usize {
    4096
}

impl Default for FactLogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_facts: default_max_facts(),
        }
    }
}

impl FactLogConfig {
    /// 生效的条数上限（下界 1，避免配成 0 后一切查询都空却看不出原因）
    pub fn effective_max_facts(&self) -> usize {
        self.max_facts.max(1)
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
