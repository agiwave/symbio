//! 账本：五类事实的唯一内存写口。重复投递按 `(kind, key)` 吸收。

use crate::ring::facts::{DebtKey, Fact, FactKind};

/// 追加失败：同一 `(kind, key)` 已在账上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendError {
    /// 重复投递。
    Duplicate,
}

/// 事实账本。
#[derive(Debug, Default, Clone)]
pub struct Ledger {
    facts: Vec<Fact>,
}

impl Ledger {
    /// 空账本。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从既有事实重建账本（重放入口）。
    pub fn from_facts(facts: Vec<Fact>) -> Self {
        let mut ledger = Self::new();
        for fact in facts {
            // 既有事实的重复项按同一口径吸收。
            let _ = ledger.append(fact);
        }
        ledger
    }

    /// 唯一写口：同一 `(kind, key)` 只许落一次。
    pub fn append(&mut self, fact: Fact) -> Result<(), AppendError> {
        if self.has(fact.kind, &fact.key) {
            return Err(AppendError::Duplicate);
        }
        self.facts.push(fact);
        Ok(())
    }

    /// 全部事实（按落账顺序）。
    pub fn facts(&self) -> &[Fact] {
        &self.facts
    }

    /// 某个身份是否已有某类事实。
    pub fn has(&self, kind: FactKind, key: &DebtKey) -> bool {
        self.facts
            .iter()
            .any(|f| f.kind == kind && &f.key == key)
    }

    /// 某个身份的收束事实（应答或判死）。
    pub fn closing(&self, key: &DebtKey) -> Option<&Fact> {
        self.facts
            .iter()
            .rev()
            .find(|f| f.kind.closes() && &f.key == key)
    }

    /// 某个 (端口, id) 当前尝试号：账上出现的最大 `attempt`；从未出现 ⇒ 0。
    pub fn attempt(&self, port: &str, id: &str) -> u32 {
        self.facts
            .iter()
            .filter(|f| f.key.port == port && f.key.id == id)
            .map(|f| f.key.attempt)
            .max()
            .unwrap_or(0)
    }

    /// 某个 (端口, id) 最新一笔 `sent`。
    pub fn last_sent(&self, port: &str, id: &str) -> Option<&Fact> {
        self.facts
            .iter()
            .rev()
            .find(|f| f.kind == FactKind::Sent && f.key.port == port && f.key.id == id)
    }

    /// 已派发的笔数（预算读它，不读序号）。
    pub fn dispatch_count(&self) -> usize {
        self.facts
            .iter()
            .filter(|f| f.kind == FactKind::Sent)
            .count()
    }

    /// 已开账但未收束的债（重放读它）。
    pub fn unresolved(&self) -> Vec<DebtKey> {
        let mut out = Vec::new();
        for fact in &self.facts {
            if fact.kind != FactKind::Request {
                continue;
            }
            if self.closing(&fact.key).is_none() && !out.contains(&fact.key) {
                out.push(fact.key.clone());
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "ledger.test.rs"]
mod tests;
