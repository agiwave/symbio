//! `session/config.rs` 的单元测试 —— 记忆两道闸门 + 三个能力开关的取值与下界。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 缺省值 / 向后兼容 / 「面板默认值与 serde 同源」的不变式由 `plugin.test.rs`
//! 的 `config_definition_defaults_come_from_session_config` 锁定，这里**刻意不重复**。
//! 本文件只回答「取值与出厂决定」这一件事——「出厂决定」指那些一旦翻转就会改变
//! 用户可见行为的默认值（`classify_enabled` / `compose_enabled` / `progress_enabled`），
//! 它们必须被断言钉住。

use super::*;

/// 缺省即出厂行为：写入 16 KiB、注入 4 KiB（与 work 插件同口径）
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

/// 出厂**打开**判决与措辞：S2 的"先交机制、默认关闭"到 S3 结束——`compose` 的两条
/// 产线已落地，`Answered` 必然有话说（最差是变体兜底），沉默的前提不复存在。
///
/// 本断言把「翻默认值」这一动作钉在**同一批**里：翻转它的人必须同时让措辞侧就位，
/// 否则「你好」会变成沉默（那是回归，不是"少说一句"）。
#[test]
fn classify_and_compose_are_on_by_default() {
    let c = SessionConfig::default();
    assert!(c.classify_enabled);
    assert!(c.compose_enabled);
    assert!(default_classify_enabled());
    assert!(default_compose_enabled());
}

/// 存量 `PLUGIN.yml` 没有这两个键时按缺省补齐（`#[serde(default)]` 的意义）
#[test]
fn missing_classify_and_compose_keys_fall_back_to_defaults() {
    let c: SessionConfig = serde_json::from_str(r#"{"max_messages": 42}"#).unwrap();
    let d = SessionConfig::default();
    assert_eq!(c.classify_enabled, d.classify_enabled);
    assert_eq!(c.compose_enabled, d.compose_enabled);
}

/// 两级开关**相互独立**：措辞关掉不会连累判决，判决关掉也不影响措辞配置。
///
/// 这不是"两个布尔恰好不同"——它们回答两个不同的问题（要不要干活 / 说什么），
/// 组合起来四种形态都合法（见 `ChatOrchestrator::compose_enabled` 的说明）。
#[test]
fn classify_and_compose_switches_are_independent() {
    let only_classify = SessionConfig {
        classify_enabled: true,
        compose_enabled: false,
        ..SessionConfig::default()
    };
    assert!(only_classify.classify_enabled && !only_classify.compose_enabled);

    let only_reply = SessionConfig {
        classify_enabled: false,
        compose_enabled: true,
        ..SessionConfig::default()
    };
    assert!(!only_reply.classify_enabled && only_reply.compose_enabled);
}

/// 出厂**打开**中途汇报，且三个上界都不是"关掉它"的取值。
///
/// 断言三个上界一起，是因为单看总开关不够：`progress_enabled = true` 配上
/// `progress_max_per_turn = 0`（或 `progress_min_rounds` 大得离谱）同样一次都不汇报
/// ——那样"默认开启"就成了一句空话。三个数各自要有意义：
/// 间隔足够长（不打扰短任务）、最少轮次 > 0（第一轮不算进展）、每轮配额 > 0。
#[test]
fn progress_reporting_is_on_by_default_with_meaningful_bounds() {
    let c = SessionConfig::default();
    assert!(c.progress_enabled);
    assert!(default_progress_enabled());
    assert!(
        c.progress_interval_ms >= 10_000,
        "间隔太短会让长任务反复刷屏（实得 {}）",
        c.progress_interval_ms
    );
    assert!(c.progress_min_rounds >= 1, "至少要走过一轮才有进展可言");
    assert!(c.progress_max_per_turn >= 1, "配额为 0 等于关掉了这个特性");
}

/// 存量 `PLUGIN.yml` 没有这四个键时按缺省补齐（`#[serde(default)]` 的意义）
#[test]
fn missing_progress_keys_fall_back_to_defaults() {
    let c: SessionConfig = serde_json::from_str(r#"{"max_messages": 42}"#).unwrap();
    let d = SessionConfig::default();
    assert_eq!(c.progress_enabled, d.progress_enabled);
    assert_eq!(c.progress_interval_ms, d.progress_interval_ms);
    assert_eq!(c.progress_min_rounds, d.progress_min_rounds);
    assert_eq!(c.progress_max_per_turn, d.progress_max_per_turn);
}

/// 汇报开关与判决 / 措辞**相互独立**：关掉汇报不影响另外两个，反之亦然。
///
/// 与上面那条同一条理由——它们回答三个不同的问题（要不要判 / 说什么 / 要不要
/// 中途打断）。合并成一个开关会让"只想要其中两个"变成不可能。
#[test]
fn the_progress_switch_is_independent_of_the_other_two() {
    let c = SessionConfig {
        classify_enabled: true,
        compose_enabled: true,
        progress_enabled: false,
        ..SessionConfig::default()
    };
    assert!(c.classify_enabled && c.compose_enabled && !c.progress_enabled);
}

/// v2 切换档位：出厂即 `bridge`（有数据、可关闭——见 `V2Mode` 文档的理由）；
/// serde 用小写词，`off` 可显式配置回去。
#[test]
fn v2_mode_defaults_to_bridge_and_roundtrips() {
    let c = SessionConfig::default();
    assert_eq!(
        c.v2_mode,
        V2Mode::Bridge,
        "出厂档位 = bridge（转写开、可关）"
    );
    assert_eq!(serde_json::to_string(&c.v2_mode).unwrap(), "\"bridge\"");
    let off: SessionConfig = serde_json::from_str(r#"{"v2_mode":"off"}"#).unwrap();
    assert_eq!(off.v2_mode, V2Mode::Off);
    // 空配置（存量 PLUGIN.yml 不写这个键）= 出厂档位。
    let from_empty: SessionConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(from_empty.v2_mode, V2Mode::Bridge);
}

/// 「欲」升格开关出厂关着（E3 平凡值，02 §2.3）：关掉后心跳照常触发、「欲」照常
/// 入格，只是**一条自主任务都不产生**——退化为纯响应式且仍完整运行。
///
/// 这条默认值一旦翻转，每个开了心跳的会话都会凭空多出 `budget_ms = 86400000` 的
/// 长目标进就绪集（模型唯一的调度候选来源），属于必须钉住的出厂决定。
#[test]
fn conation_is_off_by_default_and_falls_back_when_the_key_is_missing() {
    let d = SessionConfig::default();
    assert!(!d.conation_enabled);
    assert!(!default_conation_enabled());
    let from_empty: SessionConfig = serde_json::from_str("{}").unwrap();
    assert!(!from_empty.conation_enabled);
}

/// 技能编译开关出厂关着（S11 §4 平凡值 `projection = recall`：不编译，只检索）。
///
/// 这条默认值一旦翻转，每个会话都会在收束时凭空多出一条 `memory.encoded`——
/// 技能集随对话长度增长，而它的回退机制还没被用户认可过。关掉时整条
/// 编译 → 校准 → 回退链路**原地待命且不产生任何事实**，系统退化为纯检索。
#[test]
fn skill_compile_is_off_by_default_and_falls_back_when_the_key_is_missing() {
    let d = SessionConfig::default();
    assert!(!d.skill_compile_enabled);
    assert!(!default_skill_compile_enabled());
    let from_empty: SessionConfig = serde_json::from_str("{}").unwrap();
    assert!(!from_empty.skill_compile_enabled);
    // 键是**可配**的（默认值不是不可覆盖的常量）。
    let on: SessionConfig = serde_json::from_str(r#"{"skill_compile_enabled":true}"#).unwrap();
    assert!(on.skill_compile_enabled);
}
