//! 事件总线的**线上契约** —— 信封形状、频道词表与解包入口（`D-486` 规则 B / `D-530`）。
//!
//! 这里只装**插件间共享、内核不读**的纯形状：三个 `&str` 频道/标记常量与三个纯函数
//! （构信封、解变更、判指令）。零状态、零 IO——依赖面只有 kernel 的语法类型
//! （`PluginFrame` / `VdfsChange`）与 serde。
//!
//! 投递机制面（`EventBus` 发布入口 / `EventBusPublisher` 端口 / `EVENT_BUS_SINK`
//! 装配 id）**不在这里**——它们有状态（creator 装配），按 `D-486` 规则 C 留在
//! `symbio-sdk::event_bus`；发布端口化已在 `D-487` 完成。
//!
//! 名字叫「总线」而本层不承担路由，是有意的取舍：这里只钉跨进程边界的线上形状。
//! `bus_event` 线上名是协议面（改名即旧连接失联），权威表述在
//! `docs/003-design/005-协议规范.md` §事件总线频道，本层不改它。

use crate::kernel::vdfs::VdfsChange;
use serde::Deserialize;
use serde_json::{Map, Value};

/// 事件类型（`kind`）词表 —— **发布方一律引本常量，不要写裸字面量**。
///
/// `kind` 是**跨进程边界的字符串**，改名不会编译失败，只会让消费方的分发静默失效。
/// **闭集只有一个家**：词表住在这里，哪怕发布方在别的插件（`EVENT_BUS_KIND_VDFS`
/// 由 vdfs 发）。文档见 `docs/003-design/005-协议规范.md` §事件总线频道——那一行带
/// `<!-- vocab:KIND_ -->` 标记，由 `plugin-entry-audit` 的 E-008 与代码**双向**比对；
/// 「发布点写裸字面量」由 `grep-audit` 的 S-009 拦。
///
/// `KIND_SESSION`（会话域专用事件频道）**已废除**：会话状态与转写都是 VDFS 节点，
/// 走 `EVENT_BUS_KIND_VDFS`。常量留在词表里只会让「再发一条会话事件」看起来仍合法。
pub const EVENT_BUS_KIND_SYSTEM: &str = "system";

/// VDFS 变更频道 —— **资源实时的唯一频道**。
///
/// 不由本模块发布：一切资源的生命周期与状态变化都是 VDFS 变更，由 provider 写成功后
/// 调 `crate::sdk::vdfs::host::vdfs_notify_change`，经声明的发布端口原样投到总线上
/// （规范 §9；信封与 `VdfsChange` 形状重合，不再有独立的 `VdfsChangeEvent`）。
/// 常量放这里而非 vdfs 插件，是贯彻上面那条「闭集只有一个家」——发布方只是引用者。
pub const EVENT_BUS_KIND_VDFS: &str = "vdfs";

/// 构建事件信封 `{type:"bus_event", data:{kind, session_id, data}}`。
///
/// **信封形状的唯一构建入口**：投帧方一律走这里，不要手搓同形状的 `json!`——
/// event_bus 插件的 `connected` 帧与实现侧的 `resync_marker` 都是它的调用方，手搓的
/// 那份在形状漂移时不会有任何编译错误。解包侧有两个对偶：`event_bus_vdfs_change_of`
/// 与 `event_bus_resync_of`。
///
/// ## 为什么手搓 `Map` 而不用 `json!`
///
/// `json!` 对**表达式**参数展开成 `serde_json::to_value(&expr).unwrap()`，那份 `data`
/// 会被 serde **完整遍历一遍再重建**——出帧路径上凭空多一次整树分配 + 拷贝。
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
/// **两侧分属不同 crate**：生产方是 event_bus 插件的补送任务，消费方在 cli 与各插件
/// ——是**线上字面量**，改名不会编译失败、只会让消费方的判别静默失效。
/// **不要按这个常量手写判别**：走 `event_bus_resync_of`，它把「信封 + 频道 + 指令类型」
/// 三步一次做完（漏写判别的那个消费方就是本常量存在的理由）。
pub const EVENT_BUS_RESYNC_MARKER_TYPE: &str = "resync";

/// **VDFS 变更唯一的解包入口** —— CLI 与 agent 转播桥都走这里。
///
/// 输入是本总线投递的一帧，信封形状由 `event_bus_build_envelope` 定义。判定分两步：
/// 外层 `type == "bus_event"`，内层 `kind == EVENT_BUS_KIND_VDFS`；任一不符 → `None`
/// （帧不是给本频道消费的，属正常情况，不是错误）。
///
/// 借用 `&Value` 反序列化，**不克隆载荷**——帧是热路径，每帧一份整树深拷贝很贵。
pub fn event_bus_vdfs_change_of(frame: &crate::kernel::plugin::PluginFrame) -> Option<VdfsChange> {
    use crate::kernel::plugin::PluginFrame;
    let PluginFrame::Data(v) = frame else {
        return None;
    };
    let bus = v.get("data")?;
    if bus.get("kind").and_then(Value::as_str) != Some(EVENT_BUS_KIND_VDFS) {
        return None;
    }
    VdfsChange::deserialize(bus.get("data")?).ok()
}

/// **resync 指令的判别入口** —— 与 `event_bus_vdfs_change_of` 成对，覆盖本频道的全部帧。
///
/// 一条指令，不是变更：`kind` 是触发它的频道，载荷只有 `{type: "resync"}`、**刻意不带
/// `path`**。产帧方与解帧方各写一份、漏写无人发现，正是这两个解包入口存在的理由。
pub fn event_bus_resync_of(frame: &crate::kernel::plugin::PluginFrame, kind: &str) -> bool {
    use crate::kernel::plugin::PluginFrame;
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
#[path = "event_bus.test.rs"]
mod tests;
