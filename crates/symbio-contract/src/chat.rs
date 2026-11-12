//! 一次性问答的**共享载荷**与转写地址段（D-336 / D-420 / D-486）。
//!
//! ## 为什么在契约层
//!
//! 只在端口之间传递的**词汇形状**，不是机制、也不是会话历史。消费方是三个互不相认的
//! 扩展层插件——提问方（`session` / `compact` 经 `model.assist` 行发起）、执行体
//! （`model` 的 assist 实现）、读回方——≥2 个扩展消费方 ∧ 纯 serde 形状 ∧ 零状态零 IO
//! ∧ 内核不读 ⇒ 规则 B（[DE-023](../../../docs/003-design/023-内核边界与扩展局部性.md "DE-023-内核边界与扩展局部性")
//! 的 D-486）全过，住契约层。
//!
//! ## 与「对话轮」的区分是判据而非命名偏好
//!
//! 对话轮（`Request` / `round_trip` 的 `ChatNode` 历史）参与会话、可带工具、留历史；
//! 一次性问答只发一轮、不带工具、不留历史
//! （[D-336](../../../docs/003-design/053-环驱动与写口裁决.md "DE-053-环驱动与写口裁决")）。
//!
//! `SEG_MESSAGES` 同批搬入：它是转写地址的路径段字面量（ASCII，进地址），读面
//! （agent 转播桥）与写面（session）两个插件各用一份拼写就会漂移。同族的
//! `message_of_node` **留在服务层**：它读 keys 域的 `PRINCIPAL_*`（宿主地址知识，
//! keys 域按规划判服务层），与 D-420 的原判同方向。

/// 一次性问答的请求（`model.assist` 的行载荷）。
///
/// `system` 是提问方自己的指令（谁欠这一行谁给它），`user` 是待处理的正文；
/// `user` 为空时由执行体从事实读面拼出本轮转写——载荷只带身份，不带正文。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AssistRequest {
    #[serde(default)]
    pub system: String,
    #[serde(default)]
    pub user: String,
    /// 提问方**偏好的 provider id**（`<根>/model/<id>`）。
    ///
    /// 能力方知道该用什么档（分类要便宜快的、措辞要好写的），这个偏好就该随请求
    /// 带进来，而不是让会话替它猜。缺席 / 空串 / 取不到该条目 ⇒ 回落**会话选定**
    /// 的那个 provider（降级而不失效）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// 一次性问答的回复（`model.assist` 的应收载荷）。
///
/// `text` 是正文；`reasoning` / `reasoning_id` 是执行体如实带回的思考段（没有思考
/// 产出就是 `None`，读方不得自造）；`cost_ms` 是这一次往返的耗时。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AssistReply {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_id: Option<String>,
    #[serde(default)]
    pub cost_ms: u64,
}

/// 会话内部：转写列表的**路径段**（ASCII，进地址）。
///
/// 转写是**列表**：`<根>/session/<id>/message` 的每一项是一条消息，顺序由
/// `seq`（唯一权威顺序锚点）决定。这个地址只服务**读面**（一次 `read` 拿整份
/// 历史）与**写面**（`vdfs/action` 的截断 / 清空）。
///
/// **实时面不在这个目录上**：一条消息的流式 = 它**自己那个地址**
/// （`<id>/message/<mid>`）上的 `delta` 增量，目录只承载列表。把实时面挂在目录上
/// 会让 `path` 的含义随帧类型漂移，且无法推广到第二类集合——理由见
/// D-025 追记。
pub const SEG_MESSAGES: &str = "message";

