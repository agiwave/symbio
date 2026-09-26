//! 全局事件总线门面
//!
//! `EventBus` 是一个跨插件的全局发布设施，供 session、explorer 等插件推送事件，
//! 由前端通过 `event_bus` 插件建立的一条长连接统一订阅。
//!
//! 把它放在 `symbio_core`（而非某个具体插件）是为了遵循"插件互不可见"的分层原则：
//! 任何插件都通过 `crate::symbio_core::event_bus::EventBus` 访问，而非直接引用
//! `plugins::event_bus` 模块。

use crate::symbio_core::PluginFrame;
use crate::{plugin_info, plugin_warn};
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// resync 标记重试次数 × 间隔（20 × 100ms = 2s）——与已退役的转写流（`session/stream`
/// ，2026-09-23 随 ADR-025 一起删）同量级。
const RESYNC_RETRY: usize = 20;
const RESYNC_RETRY_INTERVAL: Duration = Duration::from_millis(100);

/// 同一订阅者的「消费过慢」告警限流窗口。
///
/// 慢消费者会**持续**满通道，每次投递都告警会把日志刷爆；而这条告警的价值在于
/// 「有这件事」，不在于「发生了多少次」。5s 一次足以定位。
const SLOW_WARN_THROTTLE: Duration = Duration::from_secs(5);

/// 事件类型（`kind`）词表 —— **发布方一律引本常量，不要写裸字面量**。
///
/// 为什么要有它：`kind` 是**跨进程边界的字符串**，改名不会编译失败，只会让消费方
/// 的分发静默失效（与 `symbio_core::plugin::route` 同一类风险）。发布点曾直接写
/// `"session"` / `"system"`，于是这两个常量声明出来后无人引用——正是
/// `chat_message.rs` 那段「枚举改名时就会出现不一致」警告的形状。
///
/// **闭集只有一个家**：词表住在这里，哪怕发布方在别的插件（`EVENT_BUS_KIND_VDFS` 由 vdfs 发）。
/// 文档见 `docs/architecture/PROTOCOLS.md` §事件总线频道——那一行带
/// `<!-- vocab:KIND_ -->` 标记，由 `plugin-entry-audit` 的 E-008 与代码**双向**比对；
/// 「发布点写裸字面量」由 `grep-audit` 的 S-009 拦。
///
/// 曾经的 `KIND_SESSION`（会话域专用事件频道）已随该频道一并废除：会话状态与
/// 转写都是 VDFS 节点，走 `EVENT_BUS_KIND_VDFS`（见 `session/docs/node-state-streaming.md`）。
/// 常量留在词表里只会让「再发一条会话事件」看起来仍然合法。
pub const EVENT_BUS_KIND_SYSTEM: &str = "system";

/// VDFS 变更频道 —— **资源实时的唯一频道**。
///
/// 不由本模块发布：一切资源的生命周期与状态变化都是 VDFS 变更，由 provider 写成功后
/// 调 `symbio_core::vdfs::host::notify_change`，经 `watch` 的 sink
/// （`plugins/vdfs/host.rs::event_bus_sink`）原样投到总线上（规范 §9；信封与 `VdfsChange` 形状重合，S27 起不再有独立的 `VdfsChangeEvent`）。
///
/// 常量放这里而非 vdfs 插件，是贯彻上面那条「闭集只有一个家」——发布方只是引用者。
/// 历史上并存的 `kind = "entity"` 频道（`publish_entity_changed` /
/// `publish_entity_status`）已随实体机制一并废除。
pub const EVENT_BUS_KIND_VDFS: &str = "vdfs";

/// 事件 Bus 全局订阅者容器
///
/// Key 是 `connection_id`（每个前端连接一个），Value 是它的发送端。
static SUBSCRIBERS: LazyLock<DashMap<String, mpsc::Sender<PluginFrame>>> =
    LazyLock::new(DashMap::new);

/// 「消费过慢」告警的上次告警时刻（限流用；订阅注销时一并清理）。
static SLOW_WARNED_AT: LazyLock<DashMap<String, Instant>> = LazyLock::new(DashMap::new);

/// 正在补送 resync 标记的订阅者集合（防止慢消费者反复触发补送任务）。
static RESYNC_INFLIGHT: LazyLock<DashMap<String, ()>> = LazyLock::new(DashMap::new);

/// 订阅请求。
///
/// ## 依赖方对照表（ADR-023 决策 2）
///
/// | 角色 | 谁 |
/// |---|---|
/// | 生产方 | `cli`（`cli/src/client.rs` 构造并发往 `event_bus/subscribe`） |
/// | 消费方 | `plugins/event_bus`（`plugin.rs` 把路由载荷反序列化成它） |
///
/// 两侧**分属不同 crate 且互相不可见**（cli 是独立 crate，只能经 `symbio` 的公开面
/// = `symbio_core` 取用），core 是唯一共同可见处——这正是它不能下沉的原因，
/// 与 `schemas::session::session_chat` 同构。
///
/// ## 为什么是空结构体（不是遗漏）
///
/// 这里原本有一个 `kinds: Option<Vec<String>>`（「限定只接收某些 kind 的事件」），
/// 但**没有任何生产者发它**：前端 `eventBus.ts` 传 `{}`、CLI 传 `kinds: None`；
/// 而后端也只是 `let _ = _req.kinds;` 把它丢掉。一个「看起来能用、实际被静默忽略」
/// 的字段比没有更危险——下一个人会以为服务端过滤已生效，再去查「为什么事件还是
/// 全量到达」。故整字段删除。
///
/// 空结构体 `Deserialize` 会**忽略一切未知字段**，因此旧客户端发
/// `{"kinds":["vdfs"]}` 仍然成功（只是没有过滤效果），前后兼容。
///
/// ## 若将来真要做服务端过滤
///
/// `kind` 是**闭集**（`EVENT_BUS_KIND_SYSTEM` / `EVENT_BUS_KIND_VDFS`，见本模块词表），按它过滤省不了
/// 多少流量。真正省流量的那一维是**路径**，而它已经实现了——`vdfs/watch` 的登记表
/// 决定哪些变更进总线（未登记的路径根本不会走到这里）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct EventBusSubscribeRequest {}

/// 注册一个订阅者发送端（由 `event_bus` 插件在建立连接时调用）
pub fn register_subscriber(connection_id: String, tx: mpsc::Sender<PluginFrame>) {
    SUBSCRIBERS.insert(connection_id, tx);
}

/// 反注册订阅者（连接断开时调用）
pub fn unregister_subscriber(connection_id: &str) {
    SUBSCRIBERS.remove(connection_id);
    SLOW_WARNED_AT.remove(connection_id);
    RESYNC_INFLIGHT.remove(connection_id);
}

/// 全局事件总线门面
///
/// 其他插件（vdfs 等）调用静态方法 `publish` 推送事件。会话域曾经的 `kind = "session"`
/// 频道已废除——会话状态与转写都是 VDFS 节点变更，走 `EVENT_BUS_KIND_VDFS`（见
/// `session/docs/node-state-streaming.md`）。
pub struct EventBus;

impl EventBus {
    /// 推送事件到所有订阅者（同步版，供 watcher 回调等非异步上下文使用）
    ///
    /// - `kind`: 事件类型（如 `EVENT_BUS_KIND_VDFS`）
    /// - `session_id`: 可选，关联到具体会话（VDFS 变更不填，身份在载荷的地址里）
    /// - `data`: 原始业务数据（任意 JSON）
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
    pub fn try_publish(kind: &str, session_id: Option<&str>, data: Value) {
        // 载荷按**所有权**搬进信封（零拷贝），整个信封只分配两个小 Map：
        // 出帧路径上因此**没有任何整树遍历**，见 `build_envelope`。
        let frame = PluginFrame::data(build_envelope(kind, session_id, data));

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

    /// 当前订阅者数量（用于调试）
    pub fn subscriber_count() -> usize {
        SUBSCRIBERS.len()
    }
}

/// 构建事件信封 `{type:"bus_event", data:{kind, session_id, data}}`。
///
/// **信封形状的唯一构建入口**：投帧方一律走这里，不要手搓同形状的 `json!`
/// ——`plugins/event_bus/plugin.rs` 的 `connected` 帧曾自己拼过一份，形状漂移
/// 时不会有任何编译错误。`vdfs::vdfs_change_of` 是它的解包对偶。
///
/// ## 为什么手搓 `Map` 而不用 `json!`
///
/// `json!` 对**表达式**参数展开成 `serde_json::to_value(&expr).unwrap()`。
/// 若写成 `json!({ "data": { ..., "data": data } })`，那份 `data` 会被 serde
/// **完整遍历一遍再重建**——出帧路径上凭空多一次整树分配 + 拷贝。
/// 直接建 `Map` 并把 `Value` 移动进去则零遍历、零拷贝。
pub fn build_envelope(kind: &str, session_id: Option<&str>, data: Value) -> Value {
    let mut inner = Map::with_capacity(3);
    inner.insert("kind".to_string(), Value::String(kind.to_string()));
    inner.insert(
        "session_id".to_string(),
        match session_id {
            Some(s) => Value::String(s.to_string()),
            None => Value::Null,
        },
    );
    inner.insert("data".to_string(), data);

    let mut outer = Map::with_capacity(2);
    outer.insert("type".to_string(), Value::String("bus_event".to_string()));
    outer.insert("data".to_string(), Value::Object(inner));
    Value::Object(outer)
}

/// resync 标记的判别值（消费端按 `data.data.type` 识别）。
///
/// **两侧分属不同 crate**：生产方是 core 自身的 `resync_marker`，消费方是 `cli`
/// （`cli/src/client.rs` 判别该字段）——是**线上字面量**，改名不会编译失败、只会让
/// 消费方的判别静默失效，故留在本层（与 `plugin::route` 的地址常量同类，ADR-023）。
///
/// 与已退役的转写流的 `transcript_resync` 同构：都是「你可能漏了帧，
/// 请按自己的作用域重读」的指令。这里**不改用 `VdfsChange` 的形状**——那是
/// 「一条变更」，而这是「一类指令」。消费端现有的作用域判定
/// （`vdfsChangeInScope` 要求 `path` 是字符串）会自然忽略它，因此新增这条指令
/// **不会**污染既有消费者的输入。
pub const EVENT_BUS_RESYNC_MARKER_TYPE: &str = "resync";

/// 构造 resync 标记帧（`kind` 与触发它的频道一致）。
fn resync_marker(kind: &str) -> PluginFrame {
    PluginFrame::data(json!({
        "type": "bus_event",
        "data": {
            "kind": kind,
            "session_id": null,
            "data": { "type": EVENT_BUS_RESYNC_MARKER_TYPE },
        }
    }))
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
/// 序号（ADR-025：顺序是节点属性），摘除即永久失联。
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
mod tests;
