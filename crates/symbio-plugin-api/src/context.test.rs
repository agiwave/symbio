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
        vec![ContextSection {
            slot: slots::RECALL,
            text: "固定一段".to_string(),
        }]
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
        vec![ContextSection {
            slot: slots::DELEGATE,
            text: format!("tools={} reason={reason}", live.tools.len()),
        }]
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
