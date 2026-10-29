//! 上下文端口的判据：按 owner 序收集、空返回不算错、段身份是宿主词汇。

use super::*;
use symbio_core::{Entity, Verb};

/// 一个只为测试存在的 provider：贡献一段固定正文。
struct Fixed;

impl ContextProvider for Fixed {
    fn owner(&self) -> &'static str {
        "tests/fixed"
    }

    fn provide(&self, _req: &ContextRequest<'_>) -> Vec<ContextSection> {
        vec![ContextSection::new(
            slots::RECALL,
            "固定一段".to_string(),
            50,
        )]
    }
}

/// 一个永远给空的 provider：多数来源在多数轮次上就是这个形态。
struct Silent;

impl ContextProvider for Silent {
    fn owner(&self) -> &'static str {
        "tests/silent"
    }
}

/// 一个只在请求级节奏取值的 provider：证明两种节奏互不串场，且 `live` 真的递到了。
struct Live;

impl ContextProvider for Live {
    fn owner(&self) -> &'static str {
        "tests/live"
    }

    fn cadence(&self) -> Cadence {
        Cadence::Request
    }

    fn provide(&self, req: &ContextRequest<'_>) -> Vec<ContextSection> {
        let Some(live) = req.live else {
            return Vec::new();
        };
        let reason = match live.delegate {
            DelegateVerdict::Start { reason } => reason,
            _ => "none",
        };
        vec![ContextSection::new(
            slots::DELEGATE,
            format!("tools={} reason={reason}", live.tools.len()),
            50,
        )]
    }
}

// 提交三条只在本测试二进制里存在的实现。
//
// 判据用**进程内**的提交：本 crate 的单测里没有别的插件，`inventory` 里只有这三条
// ——「按 owner 序」因此可以精确断言。
crate::submit_context_provider!(Fixed);
crate::submit_context_provider!(Silent);
crate::submit_context_provider!(Live);

fn facts() -> Vec<Event> {
    vec![Event::pending(
        "e-0",
        "memory.encoded",
        Entity::Memory,
        Verb::Asserted,
        0,
        "session/main",
    )]
}

fn request<'a>(facts: &'a [Event], recall: Option<&'a RecallFacts<'a>>) -> ContextRequest<'a> {
    ContextRequest {
        turn: 3,
        request_round: 0,
        principal: "session/main",
        query: Some("上周那份设计"),
        facts,
        recall,
        task_cap: 8,
        live: None,
    }
}

/// 收集按 owner 字典序：装配次序不影响模型读到的先后。
#[test]
fn providers_are_sorted_by_owner() {
    let owners: Vec<&str> = providers().iter().map(|p| p.owner()).collect();
    let mut sorted = owners.clone();
    sorted.sort_unstable();
    assert_eq!(owners, sorted);
    assert!(owners.contains(&"tests/fixed"));
    assert!(owners.contains(&"tests/silent"));
}

/// 空返回不算错：`sections` 只收集有内容的那些，不补占位。
#[test]
fn a_silent_provider_contributes_nothing() {
    let f = facts();
    let got = sections(&request(&f, None), Cadence::Turn);
    assert_eq!(got.len(), 1, "只有 Fixed 给了一段：{got:?}");
    assert_eq!(got[0].slot, slots::RECALL);
    assert_eq!(got[0].text, "固定一段");
}

/// 两种节奏互不串场：轮首收集不问请求级 provider，请求级收集也不问轮首 provider。
#[test]
fn cadence_selects_which_providers_are_asked() {
    let f = facts();
    let live_tools = [];
    let live = LiveInputs {
        tools: &live_tools,
        delegate: DelegateVerdict::Idle,
        workers: &[],
    };
    let mut req = request(&f, None);
    req.live = Some(&live);

    let turn_slots: Vec<&str> = sections(&req, Cadence::Turn)
        .iter()
        .map(|s| s.slot)
        .collect();
    assert_eq!(
        turn_slots,
        [slots::RECALL],
        "轮首只该问 Turn 节奏的 provider"
    );

    let request_sections = sections(&req, Cadence::Request);
    assert_eq!(
        request_sections.len(),
        1,
        "请求级只该问 Request 节奏的 provider"
    );
    assert_eq!(request_sections[0].slot, slots::DELEGATE);
    assert_eq!(request_sections[0].text, "tools=0 reason=none");
}

/// 请求期产物按值递入：工具清单、委派结论都是 provider 能直接读的值。
#[test]
fn live_inputs_carry_the_collected_values() {
    let f = facts();
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
    let mut req = request(&f, None);
    req.live = Some(&live);

    let got = sections(&req, Cadence::Request);
    assert_eq!(got[0].text, "tools=1 reason=explicit_prefix");
}

/// 段身份是宿主词汇：`KNOWN` 里的名字互不相同，且都是 `snake_case` 线格式。
///
/// 与 `behaviors::KNOWN` 同一条判据，但不数个数（DE-025 D-210）——逐名列出，
/// 加一个段要在这里写一行。
#[test]
fn known_slots_are_unique_and_snake_case() {
    let mut names: Vec<&str> = slots::KNOWN.to_vec();
    names.sort_unstable();
    assert_eq!(names, [slots::DELEGATE, slots::READYSET, slots::RECALL]);
    for name in slots::KNOWN {
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{name:?} 不是 snake_case 线格式"
        );
        assert_eq!(
            slots::KNOWN.iter().filter(|other| *other == name).count(),
            1,
            "{name:?} 重复登记"
        );
    }
}

/// 段载荷的拼写只有一处：`section_meta` 与 `slots` 同源。
#[test]
fn section_meta_carries_the_slot_verbatim() {
    assert_eq!(
        section_meta(slots::READYSET),
        serde_json::json!({ "kind": "readyset" })
    );
}

/// 成本由宿主计量回填：provider 构造时是 0，`fit_sections` 用注入的尺写一次。
#[test]
fn the_host_measures_the_cost() {
    let secs = vec![ContextSection::new(slots::RECALL, "abc".to_string(), 10)];
    assert_eq!(secs[0].cost, 0, "provider 侧不声明成本");
    let (kept, fit) = fit_sections(secs, 100, |t| t.len());
    assert_eq!(kept[0].cost, 3);
    assert_eq!(fit.demanded, 3);
    assert_eq!(fit.kept_cost, 3);
    assert!(!fit.trimmed());
}

/// 放得下 ⇒ 一段不裁，输出保持**收集次序**（不是裁决次序）。
#[test]
fn everything_stays_when_it_fits_and_collection_order_is_kept() {
    let secs = vec![
        ContextSection::new("low", "aaa".to_string(), 10),
        ContextSection::new("high", "bb".to_string(), 90),
    ];
    let (kept, fit) = fit_sections(secs, 5, |t| t.len());
    let slots: Vec<&str> = kept.iter().map(|s| s.slot).collect();
    assert_eq!(slots, ["low", "high"]);
    assert_eq!(fit.over(), 0);
}

/// 放不下 ⇒ 先裁优先级低的，报告说清裁了谁、超了多少。
#[test]
fn the_lowest_priority_goes_first_and_the_report_names_it() {
    let secs = vec![
        ContextSection::new("low", "aaaa".to_string(), 10),
        ContextSection::new("high", "bbbb".to_string(), 90),
    ];
    let (kept, fit) = fit_sections(secs, 6, |t| t.len());
    assert_eq!(
        kept.iter().map(|s| s.slot).collect::<Vec<_>>(),
        ["high"],
        "高优先级先留"
    );
    assert_eq!(fit.dropped, vec![("low", 4)]);
    assert_eq!(fit.over(), 2);
    assert!(fit.trimmed());
}

/// 放不下的高优先级段被**跳过**而不是停：剩下的余量让更小的段继续试。
#[test]
fn a_section_that_cannot_fit_is_skipped_not_a_hard_stop() {
    let secs = vec![
        ContextSection::new("huge", "0123456789".to_string(), 90),
        ContextSection::new("small", "ab".to_string(), 10),
    ];
    let (kept, fit) = fit_sections(secs, 5, |t| t.len());
    assert_eq!(
        kept.iter().map(|s| s.slot).collect::<Vec<_>>(),
        ["small"],
        "大段放不下的余量不该被浪费：{fit:?}"
    );
    assert_eq!(fit.dropped, vec![("huge", 10)]);
}

/// 请求里的读侧值包是**借用**：provider 改不动它，也不需要句柄。
///
/// 判据形态：`ContextRequest` 不含任何 `Arc` / `async`，构造它只要一份事实切片与
/// 几个标量——这是「provider 是纯函数」在类型上的样子。
#[test]
fn the_request_is_pure_values() {
    let f = facts();
    let line = [RecallLine {
        content: "过往的一句",
        tag: "决策",
    }];
    let recall = RecallFacts {
        degraded: false,
        lines: &line,
    };
    let req = request(&f, Some(&recall));
    assert_eq!(req.facts.len(), 1);
    assert_eq!(req.recall.map(|r| r.lines.len()), Some(1));
    assert_eq!(req.query, Some("上周那份设计"));
}
