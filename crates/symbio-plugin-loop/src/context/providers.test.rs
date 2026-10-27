//! 三个上下文 provider 的判据：段身份、空返回、节奏，以及「渲染不变」。
//!
//! 这一组用例判的是**搬移前后输出一致**：召回段与调度段的排版原先在
//! `memory_events::prompt_section` / `task_events::prompt_section`，现在由 provider
//! 调用同一对函数产出——所以这里不重复断言排版细节（那些由各自模块的用例钉），
//! 只断言「provider 在什么情况下给段、给的哪一段、次序如何」。

use super::*;
use symbio_plugin_api::context::{slots, Cadence, RecallFacts, RecallLine};
use symbio_plugin_api::{ContextProvider, DelegateVerdict, LiveInputs};

fn request<'a>(
    facts: &'a [symbio_core::Event],
    recall: Option<&'a RecallFacts<'a>>,
) -> ContextRequest<'a> {
    ContextRequest {
        turn: 1,
        request_round: 0,
        principal: "session/main",
        query: Some("上次那份设计"),
        facts,
        recall,
        task_cap: 8,
        live: None,
    }
}

/// 召回段：控制面没给视图 ⇒ 一段都不给（整段省略，不印空壳标题）。
#[test]
fn recall_without_a_view_contributes_nothing() {
    let req = request(&[], None);
    assert!(RecallProvider.provide(&req).is_empty());
}

/// 召回段：给了视图 ⇒ 交出 `slots::RECALL`，正文里带着原文与标签。
#[test]
fn recall_renders_the_judged_view() {
    let lines = [RecallLine {
        content: "把总结写进 tmp/a.md",
        tag: "经验",
    }];
    let recall = RecallFacts {
        degraded: false,
        lines: &lines,
    };
    let req = request(&[], Some(&recall));
    let got = RecallProvider.provide(&req);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].slot, slots::RECALL);
    assert!(
        got[0].text.contains("把总结写进 tmp/a.md"),
        "正文要带原文：{}",
        got[0].text
    );
    assert!(got[0].text.contains("经验"), "正文要带认知标签");
}

/// 召回段：降级要说出来（`degraded` 从投影口径走到用户眼前的唯一出口）。
#[test]
fn recall_says_it_when_degraded() {
    let lines = [RecallLine {
        content: "一条",
        tag: "经验",
    }];
    let recall = RecallFacts {
        degraded: true,
        lines: &lines,
    };
    let req = request(&[], Some(&recall));
    let text = &RecallProvider.provide(&req)[0].text;
    assert!(text.contains("部分"), "降级说明没出现在正文里：{text}");
}

/// 调度段：没有就绪任务 ⇒ 一段都不给（「算不出来」就是没有那一段，不是空占位）。
#[test]
fn readyset_without_ready_tasks_contributes_nothing() {
    let req = request(&[], None);
    assert!(ReadySetProvider.provide(&req).is_empty());
}

/// 三条 provider 都在 `loop` 名下各提交一次。
///
/// 提交序 = 同节奏内的段序（同一 owner 内 `inventory` 保持提交序，`providers()` 的
/// 排序是稳定的）：端口化之前段的次序写死在 `build_request_sections` 的数组里，
/// 现在是「召回先提交、调度次之、委派最后」。**改提交序就是改模型读到的先后**——
/// 那是行为变化，不是重构，所以这条判据至少要把「各提交了一次」钉住。
#[test]
fn all_providers_are_submitted_under_loop() {
    let loop_owned = symbio_plugin_api::context_providers()
        .iter()
        .filter(|p| p.owner() == "loop")
        .count();
    assert_eq!(
        loop_owned, 3,
        "本插件必须恰好提交三条上下文 provider（召回 + 调度 + 委派）"
    );
}

/// 收集一次请求：召回段出现在结果里，且段身份是协议里登记的那个名字。
#[test]
fn a_collected_request_carries_the_recall_slot() {
    let lines = [RecallLine {
        content: "一条",
        tag: "经验",
    }];
    let recall = RecallFacts {
        degraded: false,
        lines: &lines,
    };
    let req = request(&[], Some(&recall));
    let slots_in_order: Vec<&str> = symbio_plugin_api::context_sections(&req, Cadence::Turn)
        .iter()
        .map(|s| s.slot)
        .collect();
    assert!(
        slots_in_order.contains(&slots::RECALL),
        "召回段没被收集：{slots_in_order:?}"
    );
}

/// 委派段是**请求级**节奏：轮首收集不问它，工具收集完成后才由 `live` 供给输入。
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
    let mut req = request(&[], None);
    req.live = Some(&live);

    let turn_slots: Vec<&str> = symbio_plugin_api::context_sections(&req, Cadence::Turn)
        .iter()
        .map(|s| s.slot)
        .collect();
    assert!(
        !turn_slots.contains(&slots::DELEGATE),
        "轮首不该产出委派段：{turn_slots:?}"
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
    let req = request(&[], None);
    assert!(DelegateProvider.provide(&req).is_empty());
}
