//! 五类记账事实、债身份与只读事实投影。

use serde_json::Value;
use symbio_plugin_api::port::PortReply;

/// 环只知道这五类事实。名字是契约，不是世界词汇。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FactKind {
    /// 开账：这一笔债被认领。
    Request,
    /// 已派发：执行体已拿到这一笔。
    Sent,
    /// 已配对：这一笔有了应答。
    Response,
    /// 已配对：这一笔判死。
    Failed,
    /// 租约推进：下一笔尝试获准。
    Retry,
}

impl FactKind {
    /// 五类事实的全集（顺序固定）。
    pub(crate) const ALL: [FactKind; 5] = [
        FactKind::Request,
        FactKind::Sent,
        FactKind::Response,
        FactKind::Failed,
        FactKind::Retry,
    ];

    /// 事实名（账本里的字面值）。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FactKind::Request => "request",
            FactKind::Sent => "sent",
            FactKind::Response => "response",
            FactKind::Failed => "failed",
            FactKind::Retry => "retry",
        }
    }

    /// 从名字读回；不是五类之一 ⇒ `None`（环不落别的事实）。
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }

    /// 是否为收束事实（配对完成）。
    pub(crate) fn closes(self) -> bool {
        matches!(self, FactKind::Response | FactKind::Failed)
    }
}

/// 债身份：端口名 + 欠债方派生的 id + 租约尝试号。
///
/// `id` 由欠债方从产生这笔债的事实派生，环不生成、不计数；`attempt` 只由租约推进。
/// 事实源本身绑定单一会话，同名债因此不会跨会话串台。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DebtKey {
    /// 端口名（数据；环不解释）。拥有所有权：投影里的端口位来自入库身份，
    /// 不保证是编译期常量。
    pub(crate) port: String,
    /// 欠债方派生的身份。
    pub(crate) id: String,
    /// 租约尝试号，从 1 起。
    pub(crate) attempt: u32,
}

impl DebtKey {
    /// 组一笔债的身份。
    pub(crate) fn new(port: impl Into<String>, id: impl Into<String>, attempt: u32) -> Self {
        Self {
            port: port.into(),
            id: id.into(),
            attempt,
        }
    }

    /// 同一个 (端口, id) 的下一次尝试。
    #[cfg(test)]
    pub(crate) fn next_attempt(&self) -> Self {
        Self {
            port: self.port.clone(),
            id: self.id.clone(),
            attempt: self.attempt + 1,
        }
    }
}

/// 一条记账事实。
#[derive(Debug, Clone)]
pub(crate) struct Fact {
    /// 五类之一。
    pub(crate) kind: FactKind,
    /// 这笔账的身份。
    pub(crate) key: DebtKey,
    /// 载荷：环原样保存，不解释内容。
    pub(crate) payload: Value,
    /// 记账时刻（毫秒，由调用方给的相对时间轴）。
    pub(crate) at_ms: u64,
    /// 收束事实携带的应答承载（环只传递，不读世界字段）。
    pub(crate) reply: Option<PortReply>,
}

impl Fact {
    /// 记账事实；无应答承载。
    pub(crate) fn new(kind: FactKind, key: DebtKey, payload: Value, at_ms: u64) -> Self {
        Self {
            kind,
            key,
            payload,
            at_ms,
            reply: None,
        }
    }

    /// 带应答承载的收束事实。
    pub(crate) fn with_reply(mut self, reply: PortReply) -> Self {
        self.reply = Some(reply);
        self
    }
}

/// 事实的只读投影：环每一步都从这里重算，不持有写侧状态。
#[derive(Debug, Clone, Copy)]
pub(crate) struct FactView<'a> {
    facts: &'a [Fact],
}

impl<'a> FactView<'a> {
    /// 由宿主交来的事实快照造投影。
    pub(crate) fn new(facts: &'a [Fact]) -> Self {
        Self { facts }
    }

    /// 全部事实（按落账顺序）。
    pub(crate) fn all(&self) -> &'a [Fact] {
        self.facts
    }

    /// 某个身份的收束事实（应答或判死）。
    pub(crate) fn closing(&self, key: &DebtKey) -> Option<&'a Fact> {
        self.facts
            .iter()
            .rev()
            .find(|f| f.kind.closes() && &f.key == key)
    }

    /// 某个 (端口, id) 当前尝试号：账上出现的最大 `attempt`；从未出现 ⇒ 0。
    pub(crate) fn attempt(&self, port: &str, id: &str) -> u32 {
        self.facts
            .iter()
            .filter(|f| f.key.port.as_str() == port && f.key.id == id)
            .map(|f| f.key.attempt)
            .max()
            .unwrap_or(0)
    }

    /// 这一笔身份最新的一次 `sent`（按尝试号精确匹配）。
    pub(crate) fn last_sent(&self, key: &DebtKey) -> Option<&'a Fact> {
        self.facts
            .iter()
            .rev()
            .find(|f| f.kind == FactKind::Sent && &f.key == key)
    }

    /// 某个 `(端口)` 的最后一笔收束事实（重放与读回都只看这一笔）。
    #[cfg(test)]
    pub(crate) fn last_closing(&self, port: &str) -> Option<&'a Fact> {
        self.facts
            .iter()
            .rev()
            .find(|f| f.kind.closes() && f.key.port.as_str() == port)
    }

    /// 本会话里已开账但未收束的债（重放读它）。
    pub(crate) fn unresolved(&self) -> Vec<DebtKey> {
        let mut out = Vec::new();
        for fact in self.facts {
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
#[path = "facts.test.rs"]
mod tests;
