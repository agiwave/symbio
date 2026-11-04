//! 五类记账事实与债身份。

use serde_json::Value;
use symbio_plugin_api::port::PortReply;

/// 环只知道这五类事实。名字是契约，不是世界词汇。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FactKind {
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
    pub const ALL: [FactKind; 5] = [
        FactKind::Request,
        FactKind::Sent,
        FactKind::Response,
        FactKind::Failed,
        FactKind::Retry,
    ];

    /// 事实名（账本里的字面值）。
    pub fn as_str(self) -> &'static str {
        match self {
            FactKind::Request => "request",
            FactKind::Sent => "sent",
            FactKind::Response => "response",
            FactKind::Failed => "failed",
            FactKind::Retry => "retry",
        }
    }

    /// 从名字读回；不是五类之一 ⇒ `None`（环不落别的事实）。
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }

    /// 是否为收束事实（配对完成）。
    pub fn closes(self) -> bool {
        matches!(self, FactKind::Response | FactKind::Failed)
    }
}

/// 债身份：端口名 + 欠债方派生的 id + 租约尝试号。
///
/// `id` 由欠债方从产生这笔债的事实派生，环不生成、不计数；`attempt` 只由租约推进。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DebtKey {
    /// 端口名（数据；环不解释）。
    pub port: &'static str,
    /// 欠债方派生的身份。
    pub id: String,
    /// 租约尝试号，从 1 起。
    pub attempt: u32,
}

impl DebtKey {
    /// 组一笔债的身份。
    pub fn new(port: &'static str, id: impl Into<String>, attempt: u32) -> Self {
        Self {
            port,
            id: id.into(),
            attempt,
        }
    }

    /// 同一个 (端口, id) 的下一次尝试。
    pub fn next_attempt(&self) -> Self {
        Self {
            port: self.port,
            id: self.id.clone(),
            attempt: self.attempt + 1,
        }
    }
}

/// 一条记账事实。
#[derive(Debug, Clone)]
pub struct Fact {
    /// 五类之一。
    pub kind: FactKind,
    /// 这笔账的身份。
    pub key: DebtKey,
    /// 载荷：环原样保存，不解释内容。
    pub payload: Value,
    /// 记账时刻（毫秒，由调用方给的相对时间轴）。
    pub at_ms: u64,
    /// 收束事实携带的应答承载（环只传递，不读世界字段）。
    pub reply: Option<PortReply>,
}

impl Fact {
    /// 记账事实；无应答承载。
    pub fn new(kind: FactKind, key: DebtKey, payload: Value, at_ms: u64) -> Self {
        Self {
            kind,
            key,
            payload,
            at_ms,
            reply: None,
        }
    }

    /// 带应答承载的收束事实。
    pub fn with_reply(mut self, reply: PortReply) -> Self {
        self.reply = Some(reply);
        self
    }
}

#[cfg(test)]
#[path = "facts.test.rs"]
mod tests;
