//! `scheduler` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。
//!
//! 判空闲的算术（[`super::scheduler::due`]）在切片前长在宿主实例的方法里，混着 IO
//! 因此没法单独测；切出来后它是纯函数，本文件逐条钉它的判据与**顺序**。

use super::*;
use symbio_plugin_api::VdfsAccess;

fn hb_meta(enabled: bool, interval: u64, prompt: &str) -> Value {
    json!({
        "heartbeat": {
            "enabled": enabled,
            "interval_seconds": interval,
            "prompt": prompt,
        }
    })
}

fn view(id: &str, metadata: Value, updated_at: Option<i64>, working: bool) -> SessionView {
    SessionView {
        id: id.to_string(),
        working,
        updated_at,
        metadata,
    }
}

/// 空闲基线取较新者：正常路径 updated_at 推进到回合结束（活动真正结束），
/// 锚点较旧不拖慢触发；异常路径（零写盘退出）锚点较新，防止逐 tick 热循环。
#[test]
fn idle_baseline_takes_newer_of_anchor_and_updated_at() {
    // 无内存锚点（进程重启后）：回退到磁盘侧 updated_at
    assert_eq!(idle_baseline(None, 1_000), 1_000);
    // 正常路径：updated_at（回合最后一次写盘）较新 → 取 updated_at
    assert_eq!(idle_baseline(Some(2_000), 5_000), 5_000);
    // 异常路径：锚点（触发时刻）较新 → 取锚点（防热循环下限）
    assert_eq!(idle_baseline(Some(8_000), 5_000), 8_000);
    // 相等时取任一即可
    assert_eq!(idle_baseline(Some(5_000), 5_000), 5_000);
}

/// 抖动是**确定性**的（同一 id 每次相同）且有界（0..`JITTER_SECS` 秒）。
///
/// 确定性这一半是判据：随机抖动会让同一个会话在不同 tick 得到不同间隔，
/// 「刚触发过又差一点」与「这次差一点」都是热循环的来源。
#[test]
fn phase_ms_is_deterministic_and_bounded() {
    let upper = JITTER_SECS as i64 * 1_000;
    for id in ["s-1", "s-2", "", "会话/含斜杠的 id"] {
        let p = phase_ms(id);
        assert_eq!(p, phase_ms(id), "同一 id 的抖动必须稳定");
        assert!(p >= 0 && p < upper, "抖动越界: {id} → {p}");
    }
}

/// 判据一：**未启用**什么都不看（配置在前，时间在后的顺序是判据的一部分）。
#[test]
fn due_skips_sessions_without_a_enabled_task() {
    let v = view("s", hb_meta(false, 60, "巡检"), Some(0), false);
    assert_eq!(due(&v, None, 1_000_000), None);
}

/// 判据二：启用但**提示词空白**同样不触发——发了等于发一条空消息，
/// 那一轮除了消耗一次模型往返不产出任何事。
#[test]
fn due_skips_an_enabled_task_with_blank_prompt() {
    let v = view("s", hb_meta(true, 60, "   "), Some(0), false);
    assert_eq!(due(&v, None, 1_000_000), None);
}

/// 判据三：**不打断在跑的回合**（provider 读不到运行态时保守报 working，
/// 这一侧因此继承「宁可不触发」）。
#[test]
fn due_never_interrupts_a_working_session() {
    let v = view("s", hb_meta(true, 60, "巡检"), Some(0), true);
    assert_eq!(due(&v, None, 1_000_000), None);
}

/// `updated_at` 缺失 ⇒ 不触发：「不知道它什么时候活动过」不能读成「它一直空闲」，
/// 否则一个刚建好、尚未落过盘的新会话会在首个 tick 被开一轮。
#[test]
fn due_does_not_treat_unknown_activity_as_idle() {
    let v = view("s", hb_meta(true, 60, "巡检"), None, false);
    assert_eq!(due(&v, None, 1_000_000), None);
}

/// 时间判据：间隔含按 id 的抖动，边界因此不是 `interval` 而是 `interval + phase_ms`。
#[test]
fn due_fires_once_the_idle_reaches_interval_plus_jitter() {
    let id = "s-due";
    let interval_ms = 60_000 + phase_ms(id);
    let v = view(id, hb_meta(true, 60, "巡检"), Some(0), false);

    assert_eq!(due(&v, None, interval_ms - 1), None, "差一毫秒不该触发");
    assert_eq!(due(&v, None, interval_ms), Some(interval_ms), "到点触发");
}

/// 锚点抬高的正是**基线**：一次触发成功后，下个 tick 不能拿旧 `updated_at` 再触发一次
/// （切片前这条判据写在循环体里，测不到；现在它是 `due` 的一个入参）。
#[test]
fn anchor_raises_the_baseline_so_a_fire_is_not_repeated() {
    let v = view("s", hb_meta(true, 60, "巡检"), Some(0), false);
    // 只看 updated_at 早已超时；锚点是刚才那次触发的时刻，于是还没到下一轮
    assert_eq!(due(&v, Some(50_000), 60_000), None);
    // 再等过 interval + 抖动才重新成立
    let again = 50_000 + 60_000 + phase_ms("s");
    assert_eq!(due(&v, Some(50_000), again), Some(again - 50_000));
}

/// 清单里的**非会话项**不参与判定（本插件是按名字取数的外来读者）。
#[test]
fn session_view_only_accepts_session_kind_nodes() {
    let mut node = VdfsNode::file("s-1", "会话", VdfsAccess::READ_WRITE)
        .with_kind(PLUGIN_ID_SESSION)
        .with_attribute("metadata", hb_meta(true, 60, "巡检"));
    node.updated_at = Some(1_234);
    assert!(SessionView::is_session_kind(&node));
    let v = SessionView::of(&node);
    assert_eq!(v.id, "s-1");
    assert_eq!(v.updated_at, Some(1_234));
    assert!(!v.working, "status 缺席不等于在跑");

    // 别的 kind（挂载根、消息、收件箱）一律跳过
    let other = VdfsNode::dir("inbox", "收件箱", VdfsAccess::LIST).with_kind("dir");
    assert!(!SessionView::is_session_kind(&other));
}

/// `status = working` 投影成 `working`（唯一一条会被地址面报的运行态）。
#[test]
fn session_view_reads_the_working_status() {
    let node = VdfsNode::file("s-1", "会话", VdfsAccess::READ_WRITE)
        .with_kind(PLUGIN_ID_SESSION)
        .with_status(VDFS_STATUS_WORKING);
    assert!(SessionView::of(&node).working);
}

/// metadata 缺席时投影成**空对象**而不是崩溃：`from_metadata` 据此回落默认配置。
#[test]
fn session_view_tolerates_a_node_without_metadata() {
    let node = VdfsNode::file("s-1", "会话", VdfsAccess::READ_WRITE).with_kind(PLUGIN_ID_SESSION);
    assert_eq!(SessionView::of(&node).metadata, json!({}));
}

/// 触发消息带**两片** `meta`，各有其消费者：`heartbeat` 给前端（这条是自动发的），
/// `trigger` 给会话写臂（就地折叠成事实源里那四格）。
#[test]
fn trigger_message_carries_both_meta_slices() {
    let hb = HeartbeatConfig {
        enabled: true,
        interval_seconds: 120,
        prompt: "检查待办".to_string(),
        include_history: false,
    };
    let m = trigger_message("s-7", &hb, 4_321);

    assert_eq!(m.node_role(), Some(cm::NodeRole::User));
    assert_eq!(
        m.content.as_ref().map(|c| c.to_text()),
        Some("检查待办".to_string())
    );
    let id = m.id.clone();
    assert!(
        id.starts_with("hb_s-7_"),
        "id 带触发时刻，两次心跳不同 id: {id}"
    );

    let meta = m.meta.clone().unwrap_or_else(|| json!({}));
    assert_eq!(meta["heartbeat"], json!(true));
    assert_eq!(meta[EVENT_TRIGGER_META_KEY]["kind"], json!("scheduled"));
    assert_eq!(
        meta[EVENT_TRIGGER_META_KEY]["kind"].as_str(),
        Some(EVENT_TRIGGER_KIND_SCHEDULED),
        "写死的字面量必须与 core 的词汇同源"
    );
    assert_eq!(meta[EVENT_TRIGGER_META_KEY]["idle_ms"], json!(4_321));
}

/// 消息可被 `serde_json` 序列化：写臂那一跳传的是 JSON 文本，序列化失败等于不触发。
#[test]
fn trigger_message_serializes_for_the_inbox_write() {
    let hb = HeartbeatConfig {
        enabled: true,
        interval_seconds: 60,
        prompt: "巡检".to_string(),
        include_history: true,
    };
    let body = serde_json::to_string(&trigger_message("s-1", &hb, 10));
    assert!(body.is_ok(), "心跳消息必须能进收件箱写体");
}
