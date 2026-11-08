//! 收束载荷上的**语义章**契约：读者角色 / 台账在场 / 失败卡形状。
//!
//! 环（`symbio-plugin-loop`）在把执行体的 `response` / `failed` 草稿落格之前，
//! 按债行声明的这几格盖一枚 `stamp`。读侧（`model` / `projection` / `rebuild` /
//! 两个台账）只认章与载荷数据，**不再比较端口名字面量**——否则每新增一个世界端口，
//! 都要在机制层或读侧补一处 `port == "model"` 一类的分支。
//!
//! 这些键是跨 crate 的线格式常量：`plugin-api` 转出它们给插件，`sdk` 的读侧直接用。

/// 应答在读者面的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyRole {
    /// 这次往返的正文是对读者说的话（正文 / 思考节点进转写面与下一轮请求）。
    Reader,
    /// 端口内部往返（措辞 / 裁判 / 摘要 / 工具结果），只进机制账。
    Internal,
}

impl ReplyRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reader => "reader",
            Self::Internal => "internal",
        }
    }
}

/// 失败是否补一张结果卡。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureProjection {
    /// 失败就是失败，不补卡（默认）。
    None,
    /// 工具调用的失败要在转写面补一张「结果卡」。
    ToolCall,
}

impl FailureProjection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ToolCall => "tool_call",
        }
    }
}

/// 环在收束载荷上盖的**语义章**键名。读侧只认这些键与载荷数据，不再比较端口名。
pub const STAMP_KEY: &str = "stamp";
/// 读者角色，取值见 [`ReplyRole`]。
pub const STAMP_ROLE: &str = "role";
/// 本笔进用量台账（`usage` 数据自身仍必须存在）。
pub const STAMP_USAGE: &str = "usage";
/// 本笔进金额台账（`spend` 数据自身仍必须存在）。
pub const STAMP_SPEND: &str = "spend";
/// 失败的落格形状，取值见 [`FailureProjection`]。
pub const STAMP_FAILURE: &str = "failure";
