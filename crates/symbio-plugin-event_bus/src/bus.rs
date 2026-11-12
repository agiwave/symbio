//! 事件总线的**投递实现**（`D-487` 随订阅声明化从 `symbio-sdk/src/event_bus` 随迁）。
//!
//! 本插件是「总线」身份的本体：订阅登记表、扇出投递、慢消费者处置与 resync 补送
//! 都住这里——它们与内核语法、sdk 的其它机制没有共享面；住在 sdk 只会让每个插件
//! 引用一张它不该认识的全局表。sdk 一侧只留线上契约（信封 / `kind` 词表 / 解包对）
//! 与发布端口（`EventBusPublisher` + `submit_event_bus_sink!`），本模块在 crate 根
//! 一行声明进来（`plugin.rs` 末尾）。
//!
//! 判据（`D-487`）：把一次「函数调用」（`event_bus_register_subscriber` 的特权后门）
//! 改成一次「声明」（发布端口）+ 一条协议路径（`event_bus/subscribe` 路由）——
//! 前端、CLI、网关、子智能体转播桥走**同一条**订阅路径，服务臂随迁移净减。

use dashmap::DashMap;
use serde_json::{json, Value};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use symbio_plugin_api::event_bus_build_envelope;
use symbio_plugin_api::PluginFrame;
use symbio_plugin_api::{EventBusPublisher, EVENT_BUS_RESYNC_MARKER_TYPE};
use tokio::sync::mpsc;
use {symbio_plugin_api::plugin_info, symbio_plugin_api::plugin_warn};

/// resync 标记重试次数 × 间隔（20 × 100ms = 2s）——与已退役的转写流（`session/stream`
/// ，2026-09-23 随 D-025 一起删）同量级。
const RESYNC_RETRY: usize = 20;
const RESYNC_RETRY_INTERVAL: Duration = Duration::from_millis(100);

/// 同一订阅者的「消费过慢」告警限流窗口。
///
/// 慢消费者会**持续**满通道，每次投递都告警会把日志刷爆；而这条告警的价值在于
/// 「有这件事」，不在于「发生了多少次」。5s 一次足以定位。
const SLOW_WARN_THROTTLE: Duration = Duration::from_secs(5);

/// 事件 Bus 订阅者容器（本插件私有——订阅表随 `D-487` 归插件本体）。
///
/// Key 是 `connection_id`（每个前端连接一个），Value 是它的发送端。
pub(crate) static SUBSCRIBERS: LazyLock<DashMap<String, mpsc::Sender<PluginFrame>>> =
    LazyLock::new(DashMap::new);

/// 「消费过慢」告警的上次告警时刻（限流用；订阅注销时一并清理）。
static SLOW_WARNED_AT: LazyLock<DashMap<String, Instant>> = LazyLock::new(DashMap::new);

/// 正在补送 resync 标记的订阅者集合（防止慢消费者反复触发补送任务）。
static RESYNC_INFLIGHT: LazyLock<DashMap<String, ()>> = LazyLock::new(DashMap::new);

/// 注册一个订阅者发送端（`subscribe` 路由建立连接时调用）。
pub(crate) fn register_subscriber(connection_id: String, tx: mpsc::Sender<PluginFrame>) {
    SUBSCRIBERS.insert(connection_id, tx);
}

/// 反注册订阅者（连接清理时调用；`Drop` 配对由调用方的清理任务持有）。
pub(crate) fn unregister_subscriber(connection_id: &str) {
    SUBSCRIBERS.remove(connection_id);
    SLOW_WARNED_AT.remove(connection_id);
    RESYNC_INFLIGHT.remove(connection_id);
}

/// 发布端口的实现——[`EventBusPublisher`] 的唯一实现者，`plugin.rs` 在 crate 根
/// 经 `submit_object_creator!` 按 [`symbio_plugin_api::EVENT_BUS_SINK`] 声明进 sdk 端口。
///
/// 无状态（登记表是进程级静态），构造廉价，满足 creator 的隐含契约。
pub struct BusPublisher;

impl EventBusPublisher for BusPublisher {
    /// 投递一帧给全部已订阅连接（信封形状由 sdk 的 [`event_bus_build_envelope`] 定义）。
    ///
    /// ## 慢消费者的处置：丢帧，但**绝不摘除订阅**
    ///
    /// 通道满（`TrySendError::Full`）与对端已断开（`TrySendError::Closed`）是**两件
    /// 不同的事**，处置也必须不同：
    ///
    /// | 情形 | 处置 | 依据 |
    /// |---|---|---|
    /// | `Closed` | 摘除订阅 | 消费者走了，留着只泄漏 |
    /// | `Full` | **保留订阅** + 限流告警 + 补送 resync 标记 | 丢一帧可由「下一次变更 / 快照重读」自愈；摘除是**永久失联** |
    ///
    /// 这条区分是必须的，因为本频道的载荷（VDFS 变更）是**幂等全量节点视图**：
    /// 丢一帧的代价只是「少一次状态更新」，而摘除订阅的代价是「此后一条都收不到，
    /// 且前端拿不到任何信号」——Tauri 连接仍然活着，不会触发 `disconnected`，
    /// 前端因此**永远不知道自己在收空气**。两者严重性不对称，故不能一并处理。
    ///
    /// （历史上这里把 `Full` 与 `Closed` 合并成一句 `is_err()` 并静默摘除，
    /// 且不写日志。已退役的转写流当初正是作为这条缺陷的机制级纠正而写的；
    /// 本次把同一条纠正补到本频道。）
    fn publish(&self, kind: &str, session_id: Option<&str>, data: Value) {
        // 载荷按**所有权**搬进信封（零拷贝），整个信封只分配两个小 Map：
        // 出帧路径上因此**没有任何整树遍历**，见 `event_bus_build_envelope`。
        let frame = PluginFrame::data(event_bus_build_envelope(kind, session_id, data));

        // 先收集再处理（DashMap 迭代期不删除）。
        let mut gone: Vec<String> = Vec::new();
        let mut full: Vec<(String, mpsc::Sender<PluginFrame>)> = Vec::new();
        for entry in SUBSCRIBERS.iter() {
            let (id, tx) = (entry.key(), entry.value());
            if tx.is_closed() {
                gone.push(id.clone());
            } else if let Err(mpsc::error::TrySendError::Full(_)) = tx.try_send(frame.clone()) {
                full.push((id.clone(), tx.clone()));
            }
        }
        for id in gone {
            unregister_subscriber(&id);
        }
        for (id, tx) in full {
            warn_slow_once(&id);
            schedule_resync(kind, id, tx);
        }
    }
}

/// 构造 resync 标记帧（`kind` 与触发它的频道一致）。
///
/// 一律走 [`event_bus_build_envelope`]：它声明自己是信封形状的唯一构建入口，
/// 而这里曾是第二份手搓的 `json!`——判别入口一旦与构建分家，改名就只在运行期
/// 表现为「消费端收不到指令」，没有编译错误。
fn resync_marker(kind: &str) -> PluginFrame {
    PluginFrame::data(event_bus_build_envelope(
        kind,
        None,
        json!({ "type": EVENT_BUS_RESYNC_MARKER_TYPE }),
    ))
}

/// 「消费过慢」告警（按订阅者限流）。
fn warn_slow_once(id: &str) {
    let now = Instant::now();
    let should_warn = match SLOW_WARNED_AT.get(id) {
        Some(last) => now.duration_since(*last) >= SLOW_WARN_THROTTLE,
        None => true,
    };
    if should_warn {
        SLOW_WARNED_AT.insert(id.to_string(), now);
        plugin_warn!(
            "event_bus",
            "[Bus] 订阅者 {id} 消费过慢（通道满）：本帧已丢弃，但**订阅保留**并补送 resync 标记——消费端重读作用域后自愈"
        );
    }
}

/// 补送 resync 标记（fire-and-forget，同一订阅者同时只跑一个补送任务）。
///
/// **无论补送成功与否都不摘除订阅**——这是与已退役的转写流的有意差异：
/// 它有**流内帧序号**（`NodeEvent.seq`），摘除后消费端能靠跳号自愈；本频道没有
/// 序号（D-025：顺序是节点属性），摘除即永久失联。
/// 因此这里宁可让一个卡死的订阅者留在表里（每次变更限流告警一次，可观测），
/// 也不做那个不可逆的动作。
fn schedule_resync(kind: &str, id: String, tx: mpsc::Sender<PluginFrame>) {
    if RESYNC_INFLIGHT.contains_key(&id) {
        return;
    }
    // 同步上下文（如单测）没有运行时：不 spawn，靠「下一次变更 / 快照重读」自愈。
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    RESYNC_INFLIGHT.insert(id.clone(), ());
    let kind = kind.to_string();
    handle.spawn(async move {
        let marker = resync_marker(&kind);
        for attempt in 0..RESYNC_RETRY {
            match tx.try_send(marker.clone()) {
                Ok(()) => {
                    plugin_info!(
                        "event_bus",
                        "[Bus] 订阅者 {id} resync 标记已送达（第 {} 次尝试）：消费端将重读作用域",
                        attempt + 1
                    );
                    break;
                }
                Err(mpsc::error::TrySendError::Full(_)) if attempt + 1 < RESYNC_RETRY => {
                    tokio::time::sleep(RESYNC_RETRY_INTERVAL).await;
                }
                Err(_) => break,
            }
        }
        RESYNC_INFLIGHT.remove(&id);
    });
}

#[cfg(test)]
#[path = "bus.test.rs"]
mod tests;
