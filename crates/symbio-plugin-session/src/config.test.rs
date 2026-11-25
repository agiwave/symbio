//! `session/config.rs` 的单元测试 —— 会话基底开关与建树策略。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 缺省值 / 向后兼容 / 「面板默认值与 serde 同源」的不变式由 `plugin.test.rs`
//! 的 `config_definition_defaults_come_from_session_config` 锁定，这里**刻意不重复**。
//!
//! ## 能力开关的默认值不在这里
//!
//! `classify_enabled` / `compose_enabled` / `conation_enabled` /
//! `skill_compile_enabled` / `task_cap` 的出厂值随 D-427 迁到各自能力插件，
//! 由那里的 `config.test.rs` 钉住（`classify` / `compose` / `conation` / `skill` /
//! `task` 各一条）。本文件只钉**仍归会话基底**的那几格：开场白、中途汇报、
//! 建树策略——它们的实现体还在本插件里，翻默认值仍会改变用户可见行为。
//! 记忆闸门已随 D-513 归 `memory` 插件，由那里的 `config.test.rs` 钉住。

use super::*;
use symbio_platform::kernel::chat::chat_node::{NodeKind, CHAT_STYLE_TAG_RUN, CHAT_STYLE_TAG_TOOL};
use symbio_platform::kernel::chat::principal::{Principal, PrincipalKind};
fn marked_scope_root(node: &ChatNode) -> Option<&str> {
    node.meta
        .as_ref()
        .and_then(|meta| meta.get("scope_root"))
        .and_then(serde_json::Value::as_str)
}

/// 出厂**关闭**中途汇报，但三个上界仍是"打开即生效"的有意义取值。
///
/// 关掉它的理由：机制化前端已按节点自身呈现「在跑什么 / 跑了多久」（工具子骨架运行态）
/// 与「还没出声」（Typing 占位），轮边界那句"已完成 N 轮"与执行过程信息重复、反而干扰。
///
/// 仍断言两个上界，是因为总开关关掉不代表这两个数可以乱配：用户把 `progress_enabled`
/// 打开时，若 `progress_max_per_turn = 0`，"打开"仍成一句空话。两个数各自要有意义：
/// 间隔足够长（不打扰短任务）、每轮配额 > 0。
#[test]
fn progress_reporting_is_off_by_default_but_bounds_stay_meaningful() {
    let c = SessionConfig::default();
    assert!(!c.progress_enabled);
    assert!(!default_progress_enabled());
    assert!(
        c.progress_interval_ms >= 10_000,
        "间隔太短会让长任务反复刷屏（实得 {}）",
        c.progress_interval_ms
    );
    assert!(
        c.progress_max_per_turn >= 1,
        "配额为 0 会让重新打开变成空话"
    );
}

/// 出厂**关闭**对话面开场白（`Escalate` 首响）。
///
/// 与 `progress_enabled` 同一条理由：机制化前端的 Typing 占位已接管「点发送后立刻有动静」
/// 这一职责，那句模板旁白（"我先看一下" / "好，我来处理"）与执行过程信息重复。
/// 关掉后本轮照旧进工具循环干活，只是不说这句；真实答复（`Answered`）不受影响——
/// 那是 `compose_enabled` 管的另一句话（该开关已归 `compose` 插件，D-427）。
#[test]
fn opening_remark_is_off_by_default() {
    let c = SessionConfig::default();
    assert!(!c.opening_enabled);
    assert!(!default_opening_enabled());
}

/// 存量 `PLUGIN.yml` 没有这个键时按缺省补齐（`#[serde(default)]` 的意义）
#[test]
fn missing_opening_key_falls_back_to_default() {
    let c: SessionConfig = serde_json::from_str(r#"{"max_messages": 42}"#).unwrap();
    assert_eq!(c.opening_enabled, SessionConfig::default().opening_enabled);
}

/// 开场白与中途汇报**相互独立**：两个开关回答两个不同问题（派活前说不说一句 /
/// 长任务中途要不要打断），关掉一个不该连累另一个。
#[test]
fn the_opening_switch_is_independent_of_progress() {
    let only_opening = SessionConfig {
        opening_enabled: true,
        progress_enabled: false,
        ..SessionConfig::default()
    };
    assert!(only_opening.opening_enabled && !only_opening.progress_enabled);

    let only_progress = SessionConfig {
        opening_enabled: false,
        progress_enabled: true,
        ..SessionConfig::default()
    };
    assert!(!only_progress.opening_enabled && only_progress.progress_enabled);
}

/// 存量 `PLUGIN.yml` 没有这三个键时按缺省补齐（`#[serde(default)]` 的意义）
#[test]
fn missing_progress_keys_fall_back_to_defaults() {
    let c: SessionConfig = serde_json::from_str(r#"{"max_messages": 42}"#).unwrap();
    let d = SessionConfig::default();
    assert_eq!(c.progress_enabled, d.progress_enabled);
    assert_eq!(c.progress_interval_ms, d.progress_interval_ms);
    assert_eq!(c.progress_max_per_turn, d.progress_max_per_turn);
}

/// 建树策略出厂 = `grouped`（保持今天行为）：`flat` 才改组树，故默认档必须与
/// 「本开关引入之前」逐字一致。这条默认值一旦翻转，每个存量会话的节点树都从
/// 「本轮挂响应骨架下」变成「铺平到会话根」，属必须钉住的出厂决定。
#[test]
fn presentation_tree_defaults_to_grouped_and_parses_known_values() {
    let c = SessionConfig::default();
    assert_eq!(c.presentation_tree(), PresentationTree::Grouped);
    assert_eq!(default_presentation_tree(), "grouped");

    let flat = SessionConfig {
        presentation_tree: "flat".into(),
        ..SessionConfig::default()
    };
    assert_eq!(flat.presentation_tree(), PresentationTree::Flat);

    let deep = SessionConfig {
        presentation_tree: "deep".into(),
        ..SessionConfig::default()
    };
    assert_eq!(deep.presentation_tree(), PresentationTree::Deep);
    assert_eq!(PresentationTree::Deep.as_str(), "deep");
}

/// 认不出的写法按出厂默认处理（与 `SettingConfig::verbosity` 同一条读侧容错纪律）：
/// 手写 `PLUGIN.yml` 拼错、或写了尚未接线的档，都不该改变组树。
#[test]
fn unknown_presentation_tree_falls_back_to_grouped() {
    for raw in ["", "  flat  ", "GROUPED", "deeply", "nonsense"] {
        let c = SessionConfig {
            presentation_tree: raw.into(),
            ..SessionConfig::default()
        };
        // 带空白的 "flat" 仍解析为 Flat（trim 生效）；其余落到 Grouped。
        let want = if raw.trim() == "flat" {
            PresentationTree::Flat
        } else {
            PresentationTree::Grouped
        };
        assert_eq!(c.presentation_tree(), want, "取值 {:?} 的折叠不对", raw);
    }
}

/// 存量 `PLUGIN.yml` 没有本键时按缺省补齐（`#[serde(default = ...)]` 的意义）。
#[test]
fn missing_presentation_tree_key_falls_back_to_default() {
    let c: SessionConfig = serde_json::from_str(r#"{"max_messages": 42}"#).unwrap();
    assert_eq!(c.presentation_tree, default_presentation_tree());
    assert_eq!(c.presentation_tree(), PresentationTree::Grouped);
}

// ── 建树策略 flat 折叠（原 message_build.test.rs）──────────────

fn node(id: &str, parent: Option<&str>) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        parent_id: parent.map(str::to_string),
        ..Default::default()
    }
}

fn run_node(id: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        msg_type: Some(NodeKind::Skeleton),
        ..Default::default()
    }
    .with_style_tag(CHAT_STYLE_TAG_RUN)
}

fn tool_node(id: &str, parent: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        parent_id: Some(parent.to_string()),
        msg_type: Some(NodeKind::Skeleton),
        ..Default::default()
    }
    .with_style_tag(CHAT_STYLE_TAG_TOOL)
}

#[test]
fn flatten_whole_tree_detaches_run_children_keeps_tool_results() {
    let mut tree = vec![
        run_node("run-1"),
        node("reasoning-1", Some("run-1")),
        node("text-1", Some("run-1")),
        tool_node("tool-1", "run-1"),
        node("result-1", Some("tool-1")),
    ];
    flatten_whole_tree(&mut tree, PresentationTree::Flat);
    // 骨架自身留在根；它的直接子节点（含工具调用骨架）升根 + 打记号。
    assert_eq!(tree[0].parent_id, None);
    for row in tree.iter().skip(1).take(3) {
        assert_eq!(row.parent_id, None, "本轮直接子节点升根");
        assert_eq!(marked_scope_root(row), Some("run-1"));
    }
    // 工具结果挂在工具调用骨架下——与建树策略正交，flat 不碰。
    assert_eq!(tree[4].parent_id.as_deref(), Some("tool-1"));
    assert_eq!(marked_scope_root(&tree[4]), None);
}

/// D-335：Turn 并入用户消息节点后，读侧重建树里**没有** `Skeleton{run}`；
/// 该轮的轮锚就是用户消息节点，flat 必须按它折叠（不能继续只认 `is_run()`）。
#[test]
fn flatten_whole_tree_detaches_children_of_user_turn_anchor() {
    let mut user = node("user-1", None);
    user.msg_type = Some(NodeKind::Body);
    user.principal = Some(Principal::new(PrincipalKind::Person, "person/local"));
    let mut tree = vec![
        user,
        node("reasoning-1", Some("user-1")),
        tool_node("tool-1", "user-1"),
        node("result-1", Some("tool-1")),
    ];
    flatten_whole_tree(&mut tree, PresentationTree::Flat);
    assert_eq!(tree[1].parent_id, None, "轮锚的直接子节点升根");
    assert_eq!(marked_scope_root(&tree[1]), Some("user-1"));
    assert_eq!(tree[2].parent_id, None, "工具卡也升根");
    assert_eq!(marked_scope_root(&tree[2]), Some("user-1"));
    assert_eq!(tree[3].parent_id.as_deref(), Some("tool-1"));
    assert_eq!(marked_scope_root(&tree[3]), None);
}

#[test]
fn flatten_whole_tree_grouped_is_noop() {
    let mut tree = vec![run_node("run-1"), node("text-1", Some("run-1"))];
    flatten_whole_tree(&mut tree, PresentationTree::Grouped);
    assert_eq!(
        tree[1].parent_id.as_deref(),
        Some("run-1"),
        "grouped 保持今天形状"
    );
    assert_eq!(marked_scope_root(&tree[1]), None);
}

/// flat 折叠**不拆子会话**：转播桥把子会话顶层节点无条件挂到父工具卡下，那个
/// 转发来的 `Skeleton{run}` 仍 `is_run()`，但它的父是一张工具卡——必须跳过，否则
/// 子会话的孙节点会被错误升到会话根、拆散嵌套树。
#[test]
fn flatten_whole_tree_skips_run_skeleton_nested_in_tool_card() {
    let subrun = ChatNode {
        id: "subrun-1".to_string(),
        parent_id: Some("tool-1".to_string()),
        msg_type: Some(NodeKind::Skeleton),
        ..Default::default()
    }
    .with_style_tag(CHAT_STYLE_TAG_RUN);
    let mut tree = vec![
        run_node("run-1"),
        tool_node("tool-1", "run-1"),
        subrun,
        node("subtext-1", Some("subrun-1")),
    ];
    flatten_whole_tree(&mut tree, PresentationTree::Flat);
    // 顶层本轮骨架的直接子节点（工具卡）照常升根。
    assert_eq!(tree[1].parent_id, None, "工具卡升根");
    assert_eq!(marked_scope_root(&tree[1]), Some("run-1"));
    // 子会话的 run 骨架仍挂在工具卡下（未被当作折叠根）。
    assert_eq!(
        tree[2].parent_id.as_deref(),
        Some("tool-1"),
        "夹在工具卡内的子会话骨架不折叠"
    );
    // 其孙节点保持嵌套，绝不升根。
    assert_eq!(
        tree[3].parent_id.as_deref(),
        Some("subrun-1"),
        "子会话孙节点不被错误升根"
    );
    assert_eq!(marked_scope_root(&tree[3]), None);
}

/// `deep` 与 `grouped` 逐字相同：读侧投影都不升根，区别只在前端如何展开嵌套树。
#[test]
fn flatten_whole_tree_deep_is_noop() {
    let mut tree = vec![run_node("run-1"), node("text-1", Some("run-1"))];
    let before = serde_json::to_value(&tree).unwrap();
    flatten_whole_tree(&mut tree, PresentationTree::Deep);
    assert_eq!(
        serde_json::to_value(&tree).unwrap(),
        before,
        "deep 不做任何升根折叠"
    );
    assert_eq!(tree[1].parent_id.as_deref(), Some("run-1"));
    assert_eq!(marked_scope_root(&tree[1]), None);
}

#[test]
fn flatten_whole_tree_is_idempotent() {
    let mut once = vec![run_node("run-1"), node("text-1", Some("run-1"))];
    flatten_whole_tree(&mut once, PresentationTree::Flat);
    let mut twice = once.clone();
    flatten_whole_tree(&mut twice, PresentationTree::Flat);
    assert_eq!(
        serde_json::to_value(&once).unwrap(),
        serde_json::to_value(&twice).unwrap(),
        "投影叠在已升根的树上不二次变形"
    );
}
