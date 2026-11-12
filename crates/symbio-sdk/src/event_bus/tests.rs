//! `crates/symbio-sdk/src/event_bus/mod.rs` 的单元测试——**线上契约**的钉子。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。
//! 投递实现（登记表 / 扇出 / 慢消费者处置）随 `D-487` 搬进 event_bus 插件，
//! 那一侧的测试跟着实现走（`crates/symbio-plugin-event_bus/src/bus.test.rs`）；
//! 这里只钉**信封与解包契约**——它们是跨 crate 的线上形状，测试一律**手搓**帧
//! （不调 `event_bus_build_envelope` / 不调实现侧的标记构造器）：
//! 产方改了形状而这里没跟着改，本文件必须红。

use super::*;
use serde_json::json;
use symbio_kernel::plugin::PluginFrame;

// ── VDFS 频道帧的解包（信封契约的唯一实现）──

/// **信封契约**：`event_bus_vdfs_change_of` 必须能解出本总线产出的帧。
///
/// 信封在测试里**手搓**（不调 `event_bus_build_envelope`）：它是跨模块契约，
/// 测试要独立于产帧方来钉形状——产帧方改了形状而这里没跟着改，本用例必须红。
/// 这正是 telegram 侧那次事故的形态（产帧方与解帧方各写一份，漂移无人发现）。
#[test]
fn vdfs_change_of_unwraps_the_bus_envelope() {
    let frame = PluginFrame::data(json!({
        "type": "bus_event",
        "data": {
            "kind": "vdfs",
            "session_id": null,
            "data": { "path": "a.md" }
        }
    }));
    let change = event_bus_vdfs_change_of(&frame).expect("应能解出 VdfsChange");
    assert_eq!(change.path, "a.md");
    assert!(change.data.is_none());
}

/// 非 `kind = "vdfs"` 的帧返回 `None`——不是错误，只是不归本频道消费。
#[test]
fn vdfs_change_of_rejects_other_kinds() {
    let frame = PluginFrame::data(json!({
        "type": "bus_event",
        "data": { "kind": "system", "session_id": null, "data": {} }
    }));
    assert!(event_bus_vdfs_change_of(&frame).is_none());
}

/// 非 `Data` 帧（`Error`）返回 `None` 而**不 panic**。
///
/// `Error` 的载荷是 `Option<Value>` 且不是信封；「先解 `Data`，其余一律 `None`」
/// 是这条路径的安全前提。
#[test]
fn vdfs_change_of_handles_non_data_frames() {
    let err = PluginFrame::Error("boom".to_string(), None);
    assert!(event_bus_vdfs_change_of(&err).is_none());
}

// ── resync 指令的判别（[`event_bus_resync_of`]）──
//
// 本域的两条解包入口必须**互斥且成对**：一帧要么是变更、要么是指令、要么不归本频道。
// 子智能体转播桥曾两个都不认地一律 `continue`，于是把「你可能漏了帧」也丢了——
// 下面三条钉的就是那个漏写判别的位置。

/// **产帧方与判别入口必须对上**：线上形状的 resync 标记由 `event_bus_resync_of` 认出，
/// 且**解不出变更**（否则它会被当成一条 `path` 缺失的变更消费）。
///
/// 标记帧**手搓**（生产方在 event_bus 插件，与本文件跨 crate）：两侧按同一条线上
/// 契约各写一份，任何一侧漂移都会在这里红。
#[test]
fn resync_of_recognizes_the_markers_this_module_produces() {
    let marker = PluginFrame::data(json!({
        "type": "bus_event",
        "data": {
            "kind": EVENT_BUS_KIND_VDFS,
            "session_id": null,
            "data": { "type": EVENT_BUS_RESYNC_MARKER_TYPE }
        }
    }));
    assert!(
        event_bus_resync_of(&marker, EVENT_BUS_KIND_VDFS),
        "本模块产的标记必须由本模块的入口认出（两侧分属不同 crate，改名只会在运行期静默）"
    );
    assert!(
        event_bus_vdfs_change_of(&marker).is_none(),
        "resync 是指令而非变更：两条入口不得同时命中同一帧"
    );
    // 频道也要对上：vdfs 的指令不该被当成 system 的。
    assert!(
        !event_bus_resync_of(&marker, EVENT_BUS_KIND_SYSTEM),
        "判别含频道，跨频道不得误认"
    );
}

/// 判别读的是**信封契约**而非本模块的构造函数：手搓一份线上形状，入口同样要认出。
#[test]
fn resync_of_unwraps_the_bus_envelope() {
    let frame = PluginFrame::data(json!({
        "type": "bus_event",
        "data": {
            "kind": "vdfs",
            "session_id": null,
            "data": { "type": EVENT_BUS_RESYNC_MARKER_TYPE }
        }
    }));
    assert!(event_bus_resync_of(&frame, EVENT_BUS_KIND_VDFS));
}

/// 真变更**不是**指令；`Error` 帧、别的频道的指令也都不算。
///
/// 「先试变更、再试指令」的顺序靠这两者互斥才成立（`cli` 与各消费方都按这个顺序写）。
#[test]
fn a_real_change_is_not_a_resync_marker() {
    let change = PluginFrame::data(event_bus_build_envelope(
        EVENT_BUS_KIND_VDFS,
        None,
        json!({ "path": "a.md" }),
    ));
    assert!(event_bus_vdfs_change_of(&change).is_some());
    assert!(!event_bus_resync_of(&change, EVENT_BUS_KIND_VDFS));

    assert!(
        !event_bus_resync_of(
            &PluginFrame::Error("boom".into(), None),
            EVENT_BUS_KIND_VDFS
        ),
        "非 Data 帧不得被认成指令，也不得 panic"
    );
    let system_marker = PluginFrame::data(json!({
        "type": "bus_event",
        "data": {
            "kind": EVENT_BUS_KIND_SYSTEM,
            "session_id": null,
            "data": { "type": EVENT_BUS_RESYNC_MARKER_TYPE }
        }
    }));
    assert!(
        !event_bus_resync_of(&system_marker, EVENT_BUS_KIND_VDFS),
        "system 频道的指令不算 vdfs 频道的"
    );
}
