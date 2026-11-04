//! 债表：应用交来的行数据。环只按行里的端口名与身份取债，不解释行名。

use crate::ring::facts::{DebtKey, FactView};
use serde_json::Value;

/// 一笔待还的债（应用侧的数据）。
#[derive(Debug, Clone)]
pub struct DebtRow {
    /// 行名（应用侧的标签；环不解释，只用于回读）。
    pub name: &'static str,
    /// 欠哪个端口（数据）。
    pub port: &'static str,
    /// 债身份，由欠债方从产生这笔债的事实派生（禁计数）。
    pub id: String,
    /// 交给执行体的载荷草稿。
    pub payload: Value,
    /// 只有这笔先前的债已收束，本行才可还；`None` = 无前置。
    pub after: Option<DebtKey>,
    /// 触发条件：该**种类**的事实在本可见域存在才欠；`None` = 无条件。
    /// 环只把字符串原样交给宿主的 `exists`，不解释它的语义。
    pub when: Option<String>,
    /// 该身份的事实已存在 ⇒ 本行不欠（排除条件）。环只读 id，不解释语义。
    pub unless: Option<String>,
    /// 一次派发的租约时长（毫秒）。
    pub lease_ms: u64,
    /// 最多尝试次数（含首次）。
    pub max_attempts: u32,
}

impl DebtRow {
    /// 一行无前置的债；租约 60s、最多 3 次尝试。
    pub fn new(name: &'static str, port: &'static str, id: impl Into<String>, payload: Value) -> Self {
        Self {
            name,
            port,
            id: id.into(),
            payload,
            after: None,
            when: None,
            unless: None,
            lease_ms: 60_000,
            max_attempts: 3,
        }
    }

    /// 声明前置债：它收束之前本行不可还。
    pub fn after(mut self, key: DebtKey) -> Self {
        self.after = Some(key);
        self
    }

    /// 声明触发条件：该种类的事实存在才欠（字符串由宿主判定）。
    pub fn when(mut self, kind: impl Into<String>) -> Self {
        self.when = Some(kind.into());
        self
    }

    /// 声明排除条件：该 id 的事实已存在 ⇒ 本行不欠。
    pub fn unless(mut self, id: impl Into<String>) -> Self {
        self.unless = Some(id.into());
        self
    }

    /// 调整租约与尝试上限。
    pub fn lease(mut self, lease_ms: u64, max_attempts: u32) -> Self {
        self.lease_ms = lease_ms;
        self.max_attempts = max_attempts.max(1);
        self
    }
}

/// 债表数据源：环每圈重新问一次，应用可据既有事实增减行。
///
/// 触发条件与排除条件都是**行数据的表达力**，不是环的机制：应用读事实投影，
/// 决定这一圈欠哪些债；环只按行选债，不解释世界条件。
pub trait DebtSource {
    /// 当前可还的债（应用按自己的事实投影决定）。
    fn rows(&self, facts: &FactView<'_>, scope: u64) -> Vec<DebtRow>;

    /// 「这种事实、这个可见域，存在吗」——由宿主实现（宿主看得见全部事实，
    /// 不止环的五事实）。环只把行里的种类字符串原样透传，不解释它的语义。
    fn exists(&self, kind: &str, scope: u64) -> bool {
        let _ = (kind, scope);
        false
    }
}

/// 静态债表。
#[derive(Debug, Default, Clone)]
pub struct Table {
    rows: Vec<DebtRow>,
}

impl Table {
    /// 由行数据造表。
    pub fn new(rows: Vec<DebtRow>) -> Self {
        Self { rows }
    }

    /// 追加一行。
    pub fn push(&mut self, row: DebtRow) {
        self.rows.push(row);
    }

    /// 行数。
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// 是否没有行。
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

impl DebtSource for Table {
    fn rows(&self, _facts: &FactView<'_>, _scope: u64) -> Vec<DebtRow> {
        self.rows.clone()
    }
}

#[cfg(test)]
#[path = "table.test.rs"]
mod tests;
