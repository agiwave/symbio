//! 全局事件总线门面 —— **一条进程内广播通道**
//!
//! 它承诺的和它兑现的一样多，也正好这么多：任何插件都能向一条**闭集频道**发一帧，
//! 已注册的订阅连接原样收到。三件事因此要说清：
//!
//! | 面 | 实际形状 |
//! |---|---|
//! | 频道 | 闭集只有两条：`vdfs`（**一切资源与转写的实时变更**，唯一的业务发布方是 vdfs 宿主的变更投递）、`system`（总线自身给新连接的握手） |
//! | 帧 | 只有两种：变更、`resync` 指令。解包入口只有一个对：[`event_bus_vdfs_change_of`] / [`event_bus_resync_of`] |
//! | 分发 | **按地址在消费方自己那一侧分派**——通道不做服务端过滤，订阅请求不带过滤器（一个看着能用、实际被静默忽略的字段比没有更危险） |
//!
//! 名字叫「总线」而它不承担路由，是有意的取舍而不是待补的债：`event_bus` 是插件身份，
//! `bus_event` 是**跨栈线上名**（前端、`cli`、telegram、子智能体转播桥四方都在解它），
//! 改它是一次协议变更（旧连接当场失联），而那份认知债的账主是**这段说明**——协议面与
//! 判据的权威表述在 `../../../../docs/003-design/005-协议规范.md` §事件总线频道，裁决记录在
//! 。
//! 真要做成多频道总线（服务端按 `kind` 分流）时，第一个改动点就是这里：先加一条 ADR，
//! 再谈 `kind` 过滤与新增频道。
//!
//! 把它放在 `symbio_core`（而非某个具体插件）是为了遵循「插件互不可见」的分层原则：
//! 任何插件都通过 `crate::EventBus` 访问，而非直接引用
//! `plugins::event_bus` 模块。

use crate::{PluginFrame, VdfsChange};
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use {crate::plugin_info, crate::plugin_warn};

/// resync 标记重试次数 × 间隔（20 × 100ms = 2s）——与已退役的转写流（`session/stream`
/// ，2026-09-23 随 D-025 一起删）同量级。
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
/// 的分发静默失效（与 `crate::routes` 同一类风险）。发布点曾直接写
/// `"session"` / `"system"`，于是这两个常量声明出来后无人引用——正是
/// `chat_node.rs` 那段「枚举改名时就会出现不一致」警告的形状。
///
/// **闭集只有一个家**：词表住在这里，哪怕发布方在别的插件（`EVENT_BUS_KIND_VDFS` 由 vdfs 发）。
/// 文档见 `../../../../docs/003-design/005-协议规范.md` §事件总线频道——那一行带
/// `<!-- vocab:KIND_ -->` 标记，由 `plugin-entry-audit` 的 E-008 与代码**双向**比对；
/// 「发布点写裸字面量」由 `grep-audit` 的 S-009 拦。
///
/// `KIND_SESSION`（会话域专用事件频道）**已废除**：会话状态与
/// 转写都是 VDFS 节点，走 `EVENT_BUS_KIND_VDFS`（见 `crates/symbio-plugin-session/src/docs/105-node-state-streaming.md`）。
/// 常量留在词表里只会让「再发一条会话事件」看起来仍然合法。
pub const EVENT_BUS_KIND_SYSTEM: &str = "system";

/// VDFS 变更频道 —— **资源实时的唯一频道**。
///
/// 不由本模块发布：一切资源的生命周期与状态变化都是 VDFS 变更，由 provider 写成功后
/// 调 `symbio_core::vdfs::host::vdfs_notify_change`，经 `watch` 的 sink
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
/// ## 依赖方对照表（D-023 决策 2）
///
/// | 角色 | 谁 |
/// |---|---|
/// | 生产方 | `cli`（`crates/symbio-cli/src/client.rs` 构造并发往 `event_bus/subscribe`） |
/// | 消费方 | `plugins/event_bus`（`plugin.rs` 把路由载荷反序列化成它） |
///
/// 两侧**分属不同 crate 且互相不可见**（cli 是独立 crate，只能经 `symbio` 的公开面
/// = `symbio_core` 取用），core 是唯一共同可见处——这正是它不能下沉的原因，
/// 与 `symbio_core::session_chat` 同构。
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
pub fn event_bus_register_subscriber(connection_id: String, tx: mpsc::Sender<PluginFrame>) {
    SUBSCRIBERS.insert(connection_id, tx);
}

/// 反注册订阅者（连接断开时调用）
pub fn event_bus_unregister_subscriber(connection_id: &str) {
    SUBSCRIBERS.remove(connection_id);
    SLOW_WARNED_AT.remove(connection_id);
    RESYNC_INFLIGHT.remove(connection_id);
}

/// 全局事件总线门面
///
/// 其他插件（vdfs 等）调用静态方法 `publish` 推送事件。会话域的 `kind = "session"`
/// 频道已废除——会话状态与转写都是 VDFS 节点变更，走 `EVENT_BUS_KIND_VDFS`（见
/// `crates/symbio-plugin-session/src/docs/105-node-state-streaming.md`）。
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
            event_bus_unregister_subscriber(&id);
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
/// **信封形状的唯一构建入口**：投帧方一律走这里，不要手搓同形状的 `json!`——
/// `plugins/event_bus/plugin.rs` 的 `connected` 帧与下面的 `resync_marker` 都是它的调用方，
/// 手搓的那一份在形状漂移时不会有任何编译错误。解包侧有两个对偶：变更走
/// [`event_bus_vdfs_change_of`]，指令走 [`event_bus_resync_of`]。
///
/// ## 为什么手搓 `Map` 而不用 `json!`
///
/// `json!` 对**表达式**参数展开成 `serde_json::to_value(&expr).unwrap()`。
/// 若写成 `json!({ "data": { ..., "data": data } })`，那份 `data` 会被 serde
/// **完整遍历一遍再重建**——出帧路径上凭空多一次整树分配 + 拷贝。
/// 直接建 `Map` 并把 `Value` 移动进去则零遍历、零拷贝。
pub fn event_bus_build_envelope(kind: &str, session_id: Option<&str>, data: Value) -> Value {
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
/// **两侧分属不同 crate**：生产方是 core 自身的 `resync_marker`，消费方在 `cli` 与各插件
/// ——是**线上字面量**，改名不会编译失败、只会让消费方的判别静默失效，故留在本层（与
/// `crate::routes` 的地址常量同类，D-023）。
///
/// **不要按这个常量手写判别**：走 [`event_bus_resync_of`]，它把「信封 + 频道 + 指令类型」
/// 三步一次做完（漏写判别的那个消费方就是本常量存在的理由）。
///
/// 与已退役的转写流的 `transcript_resync` 同构：都是「你可能漏了帧，
/// 请按自己的作用域重读」的指令。这里**不改用 `VdfsChange` 的形状**——那是
/// 「一条变更」，而这是「一类指令」。消费端现有的作用域判定
/// （`vdfsChangeInScope` 要求 `path` 是字符串）会自然忽略它，因此新增这条指令
/// **不会**污染既有消费者的输入。
pub const EVENT_BUS_RESYNC_MARKER_TYPE: &str = "resync";

// ==================== VDFS 频道帧的解包（信封契约的唯一实现） ====================

/// **VDFS 变更唯一的解包入口**——CLI 与 agent 转播桥都走这里。
///
/// 输入是本总线投递的一帧，信封形状由 [`event_bus_build_envelope`] 定义：
///
/// ```text
/// { type: "bus_event", data: { kind, session_id, data: <VdfsChange> } }
/// ```
///
/// 因此判定分两步：外层 `type == "bus_event"`（本模块的约定），内层
/// `kind == EVENT_BUS_KIND_VDFS`（VDFS 频道）。任一不符 → `None`
/// （帧不是给本频道消费的，属正常情况，不是错误）。
///
/// ## 为什么住本域而不住 `vdfs`
///
/// 它匹配的**外层信封**与**频道判别值**都是本域的契约（见
/// [`event_bus_build_envelope`] / [`EVENT_BUS_KIND_VDFS`]）；只有载荷形状
/// `VdfsChange` 是 vdfs 域的。住在 vdfs 域会让资源域认识总线（`vdfs → event_bus`
/// 与 `vdfs → plugin` 两条边，构成依赖环族的一条路径）；搬来本域只多一条
/// `event_bus → vdfs`（载荷类型），而 vdfs 拆边后是汇点，不成环（D-095）。
/// 「拆信封 → 取 `data` → 反序列化」因此仍只有一份实现：消费端各手写一份
/// 正是 telegram 侧那次事故的形态（产帧方与解帧方各写一份，漂移无人发现）。
///
/// 借用 `&Value` 反序列化，**不克隆载荷**——帧是热路径，每帧一份整树深拷贝很贵。
pub fn event_bus_vdfs_change_of(frame: &crate::PluginFrame) -> Option<VdfsChange> {
    use crate::PluginFrame;
    let PluginFrame::Data(v) = frame else {
        return None;
    };
    let bus = v.get("data")?;
    if bus.get("kind").and_then(Value::as_str) != Some(EVENT_BUS_KIND_VDFS) {
        return None;
    }
    VdfsChange::deserialize(bus.get("data")?).ok()
}

/// 构造 resync 标记帧（`kind` 与触发它的频道一致）。
///
/// 一律走 [`event_bus_build_envelope`]：它声明自己是信封形状的唯一构建入口，
/// 而这里曾是第二份手搓的 `json!`——判别 [`event_bus_resync_of`] 一旦与构建
/// 分家，改名就只在运行期表现为「消费端收不到指令」，没有编译错误。
fn resync_marker(kind: &str) -> PluginFrame {
    PluginFrame::data(event_bus_build_envelope(
        kind,
        None,
        json!({ "type": EVENT_BUS_RESYNC_MARKER_TYPE }),
    ))
}

/// **resync 指令的判别入口**——与 [`event_bus_vdfs_change_of`] 成对，二者合起来覆盖本频道的全部帧。
///
/// 一条指令，不是变更：`kind` 是触发它的频道，载荷只有 `{type: "resync"}`、**刻意不带
/// `path`**，所以 `event_bus_vdfs_change_of` 解不出它。消费端的正确姿势是「按自己的作用域
/// 重读」，而重读的对象各不相同——telegram 与会话转播桥都只重读会话运行态，因为消息面
/// 另有「未知身份 ⇒ 回读基线」这条幂等兜底。
///
/// ## 为什么必须由 core 给这一个入口
///
/// 判别读的是**线上字面量**（信封的 `data.kind` 与 `data.data.type`），两侧分属不同 crate。
/// 这里曾有两个消费方各写一份（`plugins/telegram/turn.rs::is_resync`、`crates/symbio-cli/src/client.rs`），
/// 而第三个订阅者（`plugins/agent/host/subagent.rs` 的转播桥）**一份都没写**：它把解不出
/// 变更的帧一律 `continue`，于是把「你可能漏了帧」这条指令也丢了——而那条指令正是它
/// 「本轮结束」的唯一兜底（漏掉会话态帧 ⇒ 一直等到父会话中止）。
/// 产帧方与解帧方各写一份、漏写无人发现，正是 [`event_bus_vdfs_change_of`] 存在的那个理由。
pub fn event_bus_resync_of(frame: &crate::PluginFrame, kind: &str) -> bool {
    use crate::PluginFrame;
    let PluginFrame::Data(v) = frame else {
        return false;
    };
    let Some(bus) = v.get("data") else {
        return false;
    };
    bus.get("kind").and_then(Value::as_str) == Some(kind)
        && bus
            .get("data")
            .and_then(|d| d.get("type"))
            .and_then(Value::as_str)
            == Some(EVENT_BUS_RESYNC_MARKER_TYPE)
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
mod tests;
