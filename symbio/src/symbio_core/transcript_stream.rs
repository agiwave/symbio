//! 转写流（Transcript Stream）订阅设施
//!
//! 会话消息**实时面**的唯一发布通道：`Transcript`（session 插件）分配 seq 后把
//! [`NodeEvent`] 发给这里的全部订阅者。与 `event_bus`（VDFS 资源域 + system 握手）
//! 的关系是**领域分治**而非平行机制——消息不是 VDFS 节点，实时归事件流、
//! 历史归存储。
//!
//! ## 背压：显式 resync，绝不静默丢帧
//!
//! 订阅者通道满 = 慢消费者。策略（对 `EventBus::try_send` 静默丢帧缺陷的
//! 机制级纠正）：
//!
//! 1. 投递 **resync 标记帧**（重试至送达）：消费端收到即清空本地转写、从存储
//!    整份重读；期间与之后的常规帧继续按 seq 应用（重读与增量幂等共存）。
//! 2. 标记持续送不进（消费端彻底卡死）→ 摘除订阅并 warn 留痕——
//!    消费端只能靠自身重连恢复，但**这一步永远有日志坐标**。
//!
//! 丢帧在机制上不可能发生：要么送达，要么消费者收到 resync 指令，
//! 要么摘除留痕。加上每帧单调 seq，任何丢失都当场可见。
//!
//! 放在 `symbio_core`（而非 session 插件）与 `event_bus` 同理：插件互不可见，
//! 订阅方包括 session 自身（发布）、agent 转播桥与 CLI（消费）。

use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::PluginFrame;
use crate::{plugin_info, plugin_warn};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;
use tokio::sync::mpsc;

/// resync 标记重试次数 × 间隔（20 × 100ms = 2s）后才摘除订阅。
const RESYNC_RETRY: usize = 20;
const RESYNC_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// 转写流订阅表。Key 是连接 id，Value 是它的发送端。
static STREAM_SUBS: LazyLock<DashMap<String, mpsc::Sender<PluginFrame>>> =
    LazyLock::new(DashMap::new);

/// 注册一个转写流订阅者（由 `session/stream` 路由在建立连接时调用）。
pub fn register_transcript_subscriber(connection_id: String, tx: mpsc::Sender<PluginFrame>) {
    let n = STREAM_SUBS.len();
    STREAM_SUBS.insert(connection_id.clone(), tx);
    plugin_info!(
        "session",
        "[Stream] 订阅注册：conn={connection_id} subs={}",
        n + 1
    );
}

/// 反注册订阅者（连接断开时调用）。
pub fn unregister_transcript_subscriber(connection_id: &str) {
    if STREAM_SUBS.remove(connection_id).is_some() {
        plugin_info!(
            "session",
            "[Stream] 订阅注销：conn={connection_id} subs={}",
            STREAM_SUBS.len()
        );
    }
}

/// 一帧转写事件（线上格式）：`session_id` 归属 + 流内单调 `seq` + 消息本身。
///
/// ## 载荷就是一条消息，不是一层「操作」
///
/// 帧里没有独立的操作枚举：**帧携带什么，消息图就变更什么**——
///
/// | 帧里的字段 | 接收端动作 |
/// |---|---|
/// | `delta` | 追加到该节点正文尾部 |
/// | `content` | 整条替换该节点正文（幂等） |
/// | `status` | 状态迁移（`removed` = 就地移除） |
/// | `parent_id` / `role` / `type` / `name` / `tool_call_id` | 身份合并（有则覆盖） |
/// | `meta` / `seq` / `timestamp` | 覆盖（发射端持有当前完整值） |
///
/// 于是「消息现在是什么样」只有一个来源，消费端不需要把两套结构对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeEvent {
    pub session_id: String,
    /// 流内**帧序号**：单调递增，缺口即丢帧（与会话内排序锚点 `message.seq` 是两回事）。
    pub seq: u64,
    /// 消息本身。嵌套而非平铺：外层 `seq` 是帧序号、内层 `message.seq` 是存储序号，
    /// 平铺会让两个语义不同的 `seq` 撞进同一个 JSON 键。
    pub message: ChatMessage,
}

/// 一帧**会话运行态**（与会话内消息帧**共用同一个 `seq` 空间**）。
///
/// ## 为什么它必须在这条流上
///
/// 会话运行态原先走 VDFS 变更流（`kind = "vdfs"`），与消息走两条独立的
/// `mpsc` + 两条独立泵任务。于是「会话报不忙」**推不出**「本轮消息节点都已收到
/// 终态帧」——到达顺序只靠两个泵任务的调度巧合（两条通道缓冲区大小都不同：
/// 4096 vs 2048）。前端因此被迫挂一张宽限期复查的兜底网（`reconcileTranscript`）。
///
/// 挪到同一条流、共用同一个 `seq` 计数器之后，这条假设变成**保证**：
/// 单条 `mpsc` 保序，`seq` 严格递增，因此「读到 `status != working` 的这一帧」
/// ⇒ 「所有 `seq` 更小的帧（含本轮全部终态帧）都已在它之前被应用」。
/// 兜底网随之删除——不是被更强的网替代，而是它要补的那个缺口不再存在。
///
/// ## 载荷为什么是全量节点视图
///
/// 与消息帧「语义全在字段上」同源：帧**自给自足**、**幂等**。消费端不必知道
/// 「什么变了」，只按「现在是什么」收敛（`status` / `attributes.outcome` /
/// `.error` / `.warning`），因此重发、乱序重放都不会把状态带歪。
///
/// 与 VDFS 侧的分工：**运行态**（本轮 `working` / `finished` / 警告）走这里；
/// **会话节点自身的资源变更**（`created` / `deleted` / `renamed` / 标题与
/// metadata 覆盖）仍走 VDFS——它们发生在没有在途轮次的时候，**没有 transcript
/// 可以分配 `seq`**（`seq` 的唯一分配点是 `Transcript`，见 `transcript.rs`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStateEvent {
    pub session_id: String,
    /// 与消息帧**同一个计数器**发出的帧序号（这正是本类型存在的意义）。
    pub seq: u64,
    /// 会话节点的**全量视图**（`status` + `attributes.{outcome,error,warning}`）。
    pub node: crate::symbio_core::vdfs::VdfsNode,
}

/// 数据帧的 `type` 判别值（信封顶层）。
pub const EVENT_TYPE: &str = "transcript_event";

/// 会话运行态帧的 `type` 判别值（信封顶层）。
pub const SESSION_TYPE: &str = "transcript_session";

/// 背压标记帧的 `type` 判别值（信封顶层）。
pub const RESYNC_TYPE: &str = "transcript_resync";

/// 取信封载荷：本帧须是数据帧且 `type == type_tag`，返回其 `data` 字段。
fn payload_of<'a>(frame: &'a PluginFrame, type_tag: &str) -> Option<&'a Value> {
    let PluginFrame::Data(v) = frame else {
        return None;
    };
    if v.get("type").and_then(Value::as_str) != Some(type_tag) {
        return None;
    }
    v.get("data")
}

/// **转写事件唯一的解包入口**——CLI、agent 转播桥、telegram 都走这里。
///
/// ## 为什么它必须是唯一的
///
/// 「拆信封 → 取 `data` → 反序列化」这段逻辑曾经在三个地方各手写一份
/// （`cli/src/client.rs`、`plugins/agent/host/subagent.rs`、`plugins/telegram/plugin.rs`），
/// 而 telegram 那份漏了一层——它把**信封**当事件解（`from_value::<NodeEvent>(信封)`），
/// 于是每一帧都反序列化失败、`if let Ok` 把错误吞掉、正文恒为空，
/// 表现为「每条回复都回『无响应』」。
///
/// 信封形状是跨模块契约：副本数 ≥2 时，其中一份出错只是时间问题。所以这里
/// 只留一份实现，且它住在**产出信封的模块**里（`publish_frame` 的邻居）——
/// 形状改了，编译器会在这里先响。
pub fn event_of(frame: &PluginFrame) -> Option<NodeEvent> {
    // 借用 `&Value` 反序列化：帧是热路径，不为此克隆整棵载荷树。
    NodeEvent::deserialize(payload_of(frame, EVENT_TYPE)?).ok()
}

/// **会话运行态唯一的解包入口**（与 [`event_of`] 并列，同一套信封规则）。
///
/// 判别值不同（[`SESSION_TYPE`]）但形状规则完全一致——因此复用同一个
/// [`payload_of`]。消费端必须**按 `type` 分派**，不能靠「解不出 `NodeEvent`
/// 就是状态帧」这种推断：那样一旦有第三种帧，分派就退化成猜。
pub fn session_state_of(frame: &PluginFrame) -> Option<SessionStateEvent> {
    SessionStateEvent::deserialize(payload_of(frame, SESSION_TYPE)?).ok()
}

/// 是否为转写流**背压标记**（[`RESYNC_TYPE`]）：后端明示本连接曾漏帧。
///
/// 标记无载荷，[`event_of`] 对它必然返回 `None`——两者要分开判，否则
/// 「收到标记」会被当成「收到一个解不出来的事件」而被静默忽略。
pub fn is_resync(frame: &PluginFrame) -> bool {
    matches!(frame, PluginFrame::Data(v)
        if v.get("type").and_then(Value::as_str) == Some(RESYNC_TYPE))
}

/// resync 标记帧：消费端收到即清空本地转写并从存储整份重读（唯一恢复路径）。
///
/// 与数据帧用同一信封（`{type, data}`）——消费端按 `type` 分派，不必猜形状：
/// 缺 `data` 会落到 transport 的「裸数据帧」分支，类型判定随即失效。
fn resync_marker() -> PluginFrame {
    PluginFrame::data(serde_json::json!({ "type": RESYNC_TYPE, "data": null }))
}

/// 把「信封字段 + 载荷」组装成一帧，**载荷按所有权搬入**。
///
/// ## 为什么不用 `json!({ "type": ..., "data": data })`
///
/// `json!` 对**表达式**参数展开成 `serde_json::to_value(&expr).unwrap()`——于是
/// `data: Value` 会被 serde 完整遍历一遍再重建一棵新树。也就是说：
/// 「先 `to_value(event)`，再 `json!` 包一层」= **同一份数据走两遍 serde**，
/// 出帧路径上白白多一次整树分配 + 拷贝。
///
/// 这里改为直接建 `Map` 并把 `Value` **移动**进去：遍历次数从 2 降到 1，
/// 且那唯一一次遍历（`NodeEvent` → `Value`）是省不掉的。
fn envelope(type_tag: &str, data: Value) -> PluginFrame {
    let mut map = serde_json::Map::with_capacity(2);
    map.insert("type".to_string(), Value::String(type_tag.to_string()));
    map.insert("data".to_string(), data);
    PluginFrame::data(Value::Object(map))
}

/// 序列化载荷并包成信封；序列化失败返回 `None`（调用方丢弃该帧）。
///
/// 三种帧（消息 / 会话运行态 / 背压标记）共用这一个出口——「一次序列化」
/// 是出帧路径的硬要求，不该由每个发布函数各自实现一遍。
fn encode<T: Serialize>(type_tag: &str, payload: &T) -> Option<PluginFrame> {
    match serde_json::to_value(payload) {
        Ok(v) => Some(envelope(type_tag, v)),
        Err(e) => {
            crate::plugin_error!("session", "[Stream] 帧序列化失败（帧丢弃）：{}", e);
            None
        }
    }
}

/// 向全部订阅者发布一帧转写事件（消息帧）。
///
/// 出帧路径上只有**一次**序列化，扇出时只做引用计数自增（见 `PluginFrame` 的说明）。
pub fn publish_frame(event: &NodeEvent) {
    if let Some(frame) = encode(EVENT_TYPE, event) {
        fan_out(frame);
    }
}

/// 向全部订阅者发布一帧**会话运行态**（见 [`SessionStateEvent`]）。
///
/// 与 [`publish_frame`] 是同一条流、同一个扇出：**顺序与 `seq` 空间因此共享**，
/// 这正是「会话不忙 ⇒ 本轮节点已终态」得以成立的全部机制。
pub fn publish_session_state(event: &SessionStateEvent) {
    if let Some(frame) = encode(SESSION_TYPE, event) {
        fan_out(frame);
    }
}

/// 把一帧交给全部订阅者。
///
/// 先收集再处理（DashMap 迭代期不删除）。满 = 慢消费者 → 踢入 resync 流程；
/// 已关闭 = 消费者走了 → 静默摘除。
fn fan_out(frame: PluginFrame) {
    let mut full: Vec<(String, mpsc::Sender<PluginFrame>)> = Vec::new();
    let mut gone: Vec<String> = Vec::new();
    for entry in STREAM_SUBS.iter() {
        let (id, tx) = (entry.key(), entry.value());
        if tx.is_closed() {
            gone.push(id.clone());
        } else if let Err(mpsc::error::TrySendError::Full(_)) = tx.try_send(frame.clone()) {
            full.push((id.clone(), tx.clone()));
        }
    }
    for id in gone {
        STREAM_SUBS.remove(&id);
    }
    for (id, tx) in full {
        STREAM_SUBS.remove(&id);
        tokio::spawn(deliver_resync(id, tx));
    }
}

/// 给慢消费者补送 resync 标记：送达前该订阅者收不到常规帧（已在表外），
/// 送达后重新入表。持续 2s 送不进 = 消费端彻底卡死 → 摘除 + warn 留痕。
async fn deliver_resync(id: String, tx: mpsc::Sender<PluginFrame>) {
    let marker = resync_marker();
    for attempt in 0..RESYNC_RETRY {
        match tx.try_send(marker.clone()) {
            Ok(()) => {
                plugin_warn!(
                    "session",
                    "[Stream] 订阅者 {id} 消费过慢（通道满），resync 标记已送达（第 {} 次尝试）：消费端将整份重读转写",
                    attempt + 1
                );
                // 重连窗口内同 id 不会重复注册（连接 id 唯一）；还在表里才补回。
                if !STREAM_SUBS.contains_key(&id) {
                    STREAM_SUBS.insert(id.clone(), tx);
                    plugin_info!("session", "[Stream] 订阅者 {id} resync 后重新入表");
                }
                return;
            }
            Err(mpsc::error::TrySendError::Full(_)) if attempt + 1 < RESYNC_RETRY => {
                tokio::time::sleep(RESYNC_RETRY_INTERVAL).await;
            }
            Err(_) => {
                plugin_warn!(
                    "session",
                    "[Stream] 订阅者 {id} 已断开，resync 标记无法送达，订阅已摘除"
                );
                return;
            }
        }
    }
    plugin_warn!(
        "session",
        "[Stream] 订阅者 {id} 连续 {} 次尝试（约 {}ms）仍无法接收 resync 标记（消费端彻底卡死），订阅已摘除——恢复依赖消费端重连",
        RESYNC_RETRY,
        RESYNC_RETRY * RESYNC_RETRY_INTERVAL.as_millis() as usize
    );
}

#[cfg(test)]
#[path = "transcript_stream.test.rs"]
mod tests;
