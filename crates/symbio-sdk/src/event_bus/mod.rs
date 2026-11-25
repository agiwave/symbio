//! 事件总线的**发布端口** —— 投递由实现声明（`D-487`）。
//!
//! `D-530` 之后本域**只留投递机制面**：发布入口 [`EventBus`]、端口 trait
//! [`EventBusPublisher`] 与装配 id [`EVENT_BUS_SINK`]。线上契约（信封构建、解包、
//! 频道词表）已迁入 `symbio-contract::event_bus`——它们是插件间共享、内核不读的
//! 纯形状，按 `D-486` 规则 B 归契约层；本域的三个符号有状态（creator 装配），按
//! 规则 C 留服务层。
//!
//! 投递实现（订阅登记表、扇出、慢消费者处置、resync 补送）住 event_bus 插件
//! （`crates/symbio-plugin-event_bus/src/bus.rs`）——它们是该插件作为「总线」的全部机制，
//! 与内核语法、sdk 的其它机制没有共享面；住在 sdk 只会让每个插件引用一张它不该认识的
//! 全局表。发布方（[`EventBus::try_publish`]）只认端口，不认表。
//!
//! 名字叫「总线」而它不承担路由，是有意的取舍而不是待补的债：`event_bus` 是插件身份，
//! `bus_event` 是**跨栈线上名**（前端、cli、telegram、子智能体转播桥四方都在解它），
//! 改它是一次协议变更（旧连接当场失联），而那份认知债的账主是
//! `docs/003-design/005-协议规范.md` §事件总线频道——协议面与判据的权威表述在那里。

use serde_json::Value;
use std::sync::{Arc, LazyLock};

use symbio_kernel::plugin::PluginFrame;

// 投递实现的装配 id —— 与 `EMBEDDING_LOCAL` 同一条纪律：id 常量只有一个住处。
// `D-530` 后与契约层解耦：本常量是 creator 装配键（机制面），不是线上词表，故留本层。
pub const EVENT_BUS_SINK: &str = "event_bus_sink";

/// 事件总线的**投递实现**端口。
///
/// `try_publish` 的调用方（vdfs 宿主的变更投递）只认这个 trait；把一帧投给全部
/// 已订阅连接、慢消费者处置、resync 补送都是实现的事——它们住 event_bus 插件
/// （`crates/symbio-plugin-event_bus/src/bus.rs`），经 `submit_object_creator!`
/// 按本域 id 声明进来。sdk 因此不持有订阅者登记表：订阅登记表是插件自己的机制。
///
/// 走 creator（`D-036`）而不是新立收集类型：本机制的本分就是「按 id 装出对象」，
/// 当前已服务 `dyn Plugin` / `dyn ModelProtocol` / `dyn EmbeddingService` 三个类型族，
/// 这里是第四个——不为一个端口发明第二条装配路径。
pub trait EventBusPublisher: Send + Sync {
    /// 投递一帧。`kind` / `session_id` / `data` 与信封三层一一对应
    /// （信封形状见 `symbio_contract::event_bus::event_bus_build_envelope`）；
    /// 无订阅者时实现自行静默。
    fn publish(&self, kind: &str, session_id: Option<&str>, data: Value);

    fn publish_frame(&self, frame: PluginFrame);
}

/// 声明出的发布实现，进程内懒装配一次（creator 无缓存，端口侧只装一次）。
///
/// 没有声明（进程里没装 event_bus 插件）⇒ `Err` ⇒ `None` ⇒ [`EventBus::try_publish`]
/// 静默——与旧实现里「空订阅表」的语义一字不差。
static PUBLISHER: LazyLock<Option<Arc<dyn EventBusPublisher>>> = LazyLock::new(|| {
    let ctx: Arc<dyn symbio_kernel::plugin::PluginInvokeRequest> =
        Arc::new(symbio_kernel::plugin::PluginSimpleRequest::new(None, None));
    crate::creator::creator_create_object::<dyn EventBusPublisher>(EVENT_BUS_SINK, ctx)
});

/// 事件总线的**发布入口** —— vdfs 宿主投递变更走这里。
///
/// 只做一件事：把帧交给声明的发布实现。没有声明（进程里没装 event_bus 插件）⇒
/// 没有订阅者，静默——与旧实现里「空订阅表」的语义一字不差。
pub struct EventBus;

impl EventBus {
    /// 推送事件给全部已订阅连接（经声明的发布端口）。
    pub fn try_publish(kind: &str, session_id: Option<&str>, data: Value) {
        if let Some(publisher) = PUBLISHER.as_ref() {
            publisher.publish(kind, session_id, data);
        }
    }

    pub fn try_publish_frame(frame: PluginFrame) {
        if let Some(publisher) = PUBLISHER.as_ref() {
            publisher.publish_frame(frame);
        }
    }
}
