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

use crate::symbio_core::PluginFrame;
use crate::{plugin_info, plugin_warn};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
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

/// 一帧转写事件（线上格式）：`session_id` 归属 + 流内单调 `seq` + 显式操作。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeEvent {
    pub session_id: String,
    pub seq: u64,
    #[serde(flatten)]
    pub op: crate::symbio_core::schemas::session::session_chat_response::NodeOp,
}

/// resync 标记帧：消费端收到即清空本地转写并从存储整份重读（唯一恢复路径）。
///
/// 与数据帧用同一信封（`{type, data}`）——消费端按 `type` 分派，不必猜形状：
/// 缺 `data` 会落到 transport 的「裸数据帧」分支，类型判定随即失效。
fn resync_marker() -> PluginFrame {
    PluginFrame::Data(serde_json::json!({ "type": "transcript_resync", "data": null }))
}

/// 向全部订阅者发布一帧转写事件。
pub fn publish_frame(event: &NodeEvent) {
    let data = match serde_json::to_value(event) {
        Ok(v) => v,
        Err(e) => {
            crate::plugin_error!("session", "[Stream] 事件序列化失败（帧丢弃）：{}", e);
            return;
        }
    };
    let frame = PluginFrame::Data(serde_json::json!({
        "type": "transcript_event",
        "data": data,
    }));

    // 先收集再处理（DashMap 迭代期不删除）。满 = 慢消费者 → 踢入 resync 流程；
    // 已关闭 = 消费者走了 → 静默摘除。
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
