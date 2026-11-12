//! 事件总线的**线上契约与发布端口** —— 通道在这里声明，投递由实现声明（`D-487`）。
//!
//! 三件事要说清：
//!
//! | 面 | 实际形状 |
//! |---|---|
//! | 频道 | 闭集只有两条：`vdfs`（**一切资源与转写的实时变更**，唯一的业务发布方是 vdfs 宿主的变更投递）、`system`（总线自身给新连接的握手） |
//! | 帧 | 只有两种：变更、`resync` 指令。解包入口只有一个对：[`event_bus_vdfs_change_of`] / [`event_bus_resync_of`] |
//! | 投递 | **按地址在消费方自己那一侧分派**——通道不做服务端过滤；订阅连接由 event_bus 插件的 `subscribe` 路由建立（前端、CLI、网关、子智能体桥走**同一条协议路径**，无特权后门） |
//!
//! ## `D-487` 之后本域只留两样：线上契约 + 发布端口
//!
//! 订阅登记表、扇出投递、慢消费者处置与 resync 补送是**投递实现**，住 event_bus 插件
//! （`crates/symbio-plugin-event_bus/src/bus.rs`）——它们是该插件作为「总线」的全部机制，
//! 与内核语法、sdk 的其它机制没有共享面；住在 sdk 只会让每个插件引用一张它不该认识的全局表。
//! 本域为此声明一个**发布端口**：[`EventBusPublisher`] trait + 装配 id
//! [`EVENT_BUS_SINK`]（creator 按 id 装配，`D-036` 的第四个类型族），发布方
//! （[`EventBus::try_publish`]）只认端口，不认表。
//!
//! 名字叫「总线」而它不承担路由，是有意的取舍而不是待补的债：`event_bus` 是插件身份，
//! `bus_event` 是**跨栈线上名**（前端、`cli`、telegram、子智能体转播桥四方都在解它），
//! 改它是一次协议变更（旧连接当场失联），而那份认知债的账主是**这段说明**——协议面与
//! 判据的权威表述在 `../../../../docs/003-design/005-协议规范.md` §事件总线频道。
//! 真要做成多频道总线（服务端按 `kind` 分流）时，第一个改动点就是这里：先加一条 ADR，
//! 再谈 `kind` 过滤与新增频道。

use serde::Deserialize;
use serde_json::{Map, Value};
use std::sync::{Arc, LazyLock};
use symbio_kernel::vdfs::VdfsChange;

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
/// 调 `symbio_sdk::vdfs::host::vdfs_notify_change`，经声明的发布端口原样投到总线上
/// （规范 §9；信封与 `VdfsChange` 形状重合，不再有独立的 `VdfsChangeEvent`）。
///
/// 常量放这里而非 vdfs 插件，是贯彻上面那条「闭集只有一个家」——发布方只是引用者。
/// 历史上并存的 `kind = "entity"` 频道（`publish_entity_changed` /
/// `publish_entity_status`）已随实体机制一并废除。
pub const EVENT_BUS_KIND_VDFS: &str = "vdfs";

// ==================== 发布端口（`D-487`：一次「函数调用」改成一次「声明」） ====================

/// 事件总线的**投递实现**端口。
///
/// `try_publish` 的调用方（vdfs 宿主的变更投递）只认这个 trait；把一帧投给全部
/// 已订阅连接、慢消费者处置、resync 补送都是实现的事——它们住 event_bus 插件
/// （`crates/symbio-plugin-event_bus/src/bus.rs`），经 [`submit_object_creator!`]
/// 按本域 id 声明进来。sdk 因此不持有订阅者登记表：订阅登记表是插件自己的机制，
/// 与「插件互不可见」同一条分层纪律（没有第二个消费者认识它）。
///
/// 走 creator（`D-036`）而不是新立收集类型：本机制的本分就是「按 id 装出对象」，
/// 当前已服务 `dyn Plugin` / `dyn ModelProtocol` / `dyn EmbeddingService` 三个类型族，
/// 这里是第四个——不为一个端口发明第二条装配路径。
pub trait EventBusPublisher: Send + Sync {
    /// 投递一帧。`kind` / `session_id` / `data` 与信封三层一一对应
    /// （见 [`event_bus_build_envelope`]）；无订阅者时实现自行静默。
    fn publish(&self, kind: &str, session_id: Option<&str>, data: Value);
}

/// 投递实现的**装配 id**（creator 按 `(id, TypeId)` 索引；与 `EMBEDDING_LOCAL`
/// 同一条纪律——id 常量只有一个住处，插件声明与 sdk 取用各引一份）。
pub const EVENT_BUS_SINK: &str = "event_bus_sink";

/// 声明出的发布实现，进程内懒装配一次（creator 无缓存，端口侧只装一次）。
///
/// 没有声明（进程里没装 event_bus 插件）⇒ `Err` ⇒ `None` ⇒ [`EventBus::try_publish`]
/// 静默——与旧实现里「空订阅表」的语义一字不差。
static PUBLISHER: LazyLock<Option<Arc<dyn EventBusPublisher>>> = LazyLock::new(|| {
    let ctx: Arc<dyn symbio_kernel::plugin::PluginInvokeRequest> =
        Arc::new(symbio_kernel::plugin::PluginSimpleRequest::new(None, None));
    crate::creator::creator_create_object::<dyn EventBusPublisher>(EVENT_BUS_SINK, ctx)
});

/// 事件总线的**发布入口**——vdfs 宿主投递变更走这里。
///
/// 只做一件事：把帧交给声明的发布实现。没有声明（进程里没装 event_bus 插件）⇒
/// 没有订阅者，静默——与旧实现里「空订阅表」的语义一字不差。
///
/// 载荷按**所有权**搬进信封（零拷贝），信封形状由 [`event_bus_build_envelope`] 定义；
/// 扇出、慢消费者处置与 resync 补送在实现那一侧（`D-487`）。
pub struct EventBus;

impl EventBus {
    /// 推送事件给全部已订阅连接（经声明的发布端口）。
    pub fn try_publish(kind: &str, session_id: Option<&str>, data: Value) {
        if let Some(publisher) = PUBLISHER.as_ref() {
            publisher.publish(kind, session_id, data);
        }
    }
}

// ==================== 线上契约：信封与解包 ====================

/// 构建事件信封 `{type:"bus_event", data:{kind, session_id, data}}`。
///
/// **信封形状的唯一构建入口**：投帧方一律走这里，不要手搓同形状的 `json!`——
/// `crates/symbio-plugin-event_bus/src/plugin.rs` 的 `connected` 帧与实现侧的
/// `resync_marker` 都是它的调用方，手搓的那份在形状漂移时不会有任何编译错误。
/// 解包侧有两个对偶：变更走 [`event_bus_vdfs_change_of`]，指令走 [`event_bus_resync_of`]。
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
/// **两侧分属不同 crate**：生产方是 event_bus 插件的补送任务，消费方在 `cli` 与各插件
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
pub fn event_bus_vdfs_change_of(frame: &symbio_kernel::plugin::PluginFrame) -> Option<VdfsChange> {
    use symbio_kernel::plugin::PluginFrame;
    let PluginFrame::Data(v) = frame else {
        return None;
    };
    let bus = v.get("data")?;
    if bus.get("kind").and_then(Value::as_str) != Some(EVENT_BUS_KIND_VDFS) {
        return None;
    }
    VdfsChange::deserialize(bus.get("data")?).ok()
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
/// 这里曾有两个消费方各写一份（`crates/symbio-plugin-telegram/src/turn.rs::is_resync`、`crates/symbio-cli/src/client.rs`），
/// 而第三个订阅者（`crates/symbio-plugin-agent/src/host/subagent.rs` 的转播桥）**一份都没写**：它把解不出
/// 变更的帧一律 `continue`，于是把「你可能漏了帧」这条指令也丢了——而那条指令正是它
/// 「本轮结束」的唯一兜底（漏掉会话态帧 ⇒ 一直等到父会话中止）。
/// 产帧方与解帧方各写一份、漏写无人发现，正是 [`event_bus_vdfs_change_of`] 存在的那个理由。
pub fn event_bus_resync_of(frame: &symbio_kernel::plugin::PluginFrame, kind: &str) -> bool {
    use symbio_kernel::plugin::PluginFrame;
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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
