//! 本插件上下文 provider 的判据：段身份、空返回、节奏、提交数。
//!
//! 召回段的 provider 自 L2b 起住能力包（`crates/symbio-plugin-memory` 的
//! `recall::provider`，DE-026 D-264）：段长什么样是检索读数的排版口径，判据跟着走
//! （那边有 `recall::provider.test.rs` 钉段身份与空态，`recall::section.test.rs` 钉排版）。
//! 本文件只剩委派段——它是控制面的判断（Q1 结论 + 工具清单），不是任何能力包的判据。
//!
//! 调度段的 provider 随任务能力迁出（`symbio-plugin-task`），它的判据在那边。

use super::*;
use symbio_plugin_api::context::{slots, Cadence};
use symbio_plugin_api::{ContextProvider, DelegateVerdict, LiveInputs};

fn request<'a>(facts: &'a [symbio_core::Event]) -> ContextRequest<'a> {
    ContextRequest {
        request_round: 0,
        principal: "session/main",
        query: Some("上次那份设计"),
        facts,
        recall: None,
        live: None,
    }
}

/// 本插件**只**提交一条 provider（委派）；召回的那条归能力包。
///
/// 这条判据同时钉住次序的依据：owner 字典序 `loop` < `memory` < `task` ⇒ 发言级仍是
/// 「召回 → 调度」、请求级仍是「委派」。**改提交数或 owner 就是改模型读到的先后**——
/// 那是行为变化，不是重构。
#[test]
fn only_the_delegate_provider_is_submitted_under_loop() {
    let loop_owned = symbio_plugin_api::context_providers()
        .iter()
        .filter(|p| p.owner() == "loop")
        .count();
    assert_eq!(
        loop_owned, 1,
        "本插件必须恰好提交一条上下文 provider（委派；召回已随能力迁出）"
    );
}

/// 委派段是**请求级**节奏：发言级收集不问它，工具收集完成后才由 `live` 供给输入。
#[test]
fn the_delegate_section_waits_for_the_request_cadence() {
    let tools = [symbio_core::CapabilityMeta {
        name: "file_read".to_string(),
        ..Default::default()
    }];
    let live = LiveInputs {
        tools: &tools,
        delegate: DelegateVerdict::Start {
            reason: "explicit_prefix",
        },
        workers: &[],
    };
    let mut req = request(&[]);
    req.live = Some(&live);

    let utterance_slots: Vec<&str> = symbio_plugin_api::context_sections(&req, Cadence::Utterance)
        .iter()
        .map(|s| s.slot)
        .collect();
    assert!(
        !utterance_slots.contains(&slots::DELEGATE),
        "发言级收集不该产出委派段：{utterance_slots:?}"
    );

    let request_sections = symbio_plugin_api::context_sections(&req, Cadence::Request);
    assert_eq!(request_sections.len(), 1, "{request_sections:?}");
    assert_eq!(request_sections[0].slot, slots::DELEGATE);
    let text = &request_sections[0].text;
    assert!(text.contains("reason=explicit_prefix"), "{text}");
    assert!(
        text.contains("- FileOperation") || text.contains("file_read"),
        "{text}"
    );
}

/// 轮首没有 `live`（例如宿主误以请求级收集轮首值）⇒ 委派段一段都不给，不印空壳。
#[test]
fn delegate_without_live_inputs_contributes_nothing() {
    let req = request(&[]);
    assert!(DelegateProvider.provide(&req).is_empty());
}
