//! `session/config.rs` 的单元测试 —— 记忆两道闸门、会话基底开关与建树策略。
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
//! `task` 各一条）。本文件只钉**仍归会话基底**的那几格：记忆闸门、开场白、
//! 中途汇报、建树策略——它们的实现体还在本插件里，翻默认值仍会改变用户可见行为。

use super::*;

/// 缺省即出厂行为：写入 16 KiB、注入 4 KiB（与 memory 插件的两腿同口径）
#[test]
fn memory_gate_defaults_are_the_shipped_behaviour() {
    let c = SessionConfig::default();
    assert_eq!(c.memory_max_bytes, 16 * 1024);
    assert_eq!(c.memory_inject_max_bytes, 4 * 1024);
}

/// 两道闸门**独立**：写入上限通常远大于注入预算，改一个不该动另一个
#[test]
fn the_two_gates_are_independent() {
    let c = SessionConfig {
        memory_max_bytes: 4096,
        memory_inject_max_bytes: 512,
        ..SessionConfig::default()
    };
    assert_eq!(c.effective_memory_max_bytes(), 4096);
    assert_eq!(c.effective_memory_inject_bytes(), 512);
}

/// 配成 0 不能让一切写入都失败却看不出原因——下界兜到 1
#[test]
fn zero_limits_are_clamped_to_one() {
    let c = SessionConfig {
        memory_max_bytes: 0,
        memory_inject_max_bytes: 0,
        ..SessionConfig::default()
    };
    assert_eq!(c.effective_memory_max_bytes(), 1);
    assert_eq!(c.effective_memory_inject_bytes(), 1);
}

/// 存量 `PLUGIN.yml` 缺这两个键时按缺省补齐（`#[serde(default)]` 的意义）
#[test]
fn missing_memory_keys_fall_back_to_defaults() {
    let c: SessionConfig = serde_json::from_str(r#"{"max_messages": 42}"#).unwrap();
    assert_eq!(c.max_messages, 42);
    assert_eq!(
        c.memory_max_bytes,
        SessionConfig::default().memory_max_bytes
    );
    assert_eq!(
        c.memory_inject_max_bytes,
        SessionConfig::default().memory_inject_max_bytes
    );
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
