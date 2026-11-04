//! 债表：应用交来的行数据。环只按行里的端口名与身份取债，不解释行名。

use crate::ring::facts::{DebtKey, Fact};
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
            lease_ms: 60_000,
            max_attempts: 3,
        }
    }

    /// 声明前置债：它收束之前本行不可还。
    pub fn after(mut self, key: DebtKey) -> Self {
        self.after = Some(key);
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
pub trait DebtSource {
    /// 当前可还的债（应用按自己的事实投影决定）。
    fn rows(&self, facts: &[Fact]) -> Vec<DebtRow>;
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
    fn rows(&self, _facts: &[Fact]) -> Vec<DebtRow> {
        self.rows.clone()
    }
}

#[cfg(test)]
#[path = "table.test.rs"]
mod tests;
