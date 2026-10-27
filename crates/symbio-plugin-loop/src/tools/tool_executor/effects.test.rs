//! 效果记账口的判据：只收声明过的调用、同一次调用只记一笔、没成功带上原因。

use super::record_effect;
use serde_json::json;
use symbio_core::CapabilityMeta;
use symbio_plugin_api::behaviors::{ASSIGN_WORK, DELEGATION_PROMISE};

/// 按行为名构造一份元数据（其余字段走 `Default`）。
fn meta(name: &str, declared: &'static [&'static str]) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        behaviors: declared,
        ..Default::default()
    }
}

fn decls() -> super::super::ToolDecls {
    super::super::ToolDecls::of_metas(&[
        meta("agent_run", &[ASSIGN_WORK, DELEGATION_PROMISE]),
        meta("shell", &[]),
    ])
}

/// 没声明任何行为的工具不进效果出参——出参里不留空壳（没有效果可记）。
#[test]
fn undeclared_tool_is_skipped() {
    let mut out = Vec::new();
    record_effect(
        &mut out,
        &decls(),
        "c-1",
        "local/shell",
        &json!({}),
        true,
        "ok",
        "",
    );
    assert!(out.is_empty());
}

/// 声明过的调用记一笔，且**声明的行为名原样带过去**（写口据此选实现）。
#[test]
fn declared_tool_records_one_input() {
    let mut out = Vec::new();
    record_effect(
        &mut out,
        &decls(),
        "c-1",
        "agent_run",
        &json!({ "prompt": "评审" }),
        true,
        "完成",
        "",
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].tool, "agent_run");
    assert_eq!(out[0].call_id, "c-1");
    assert!(out[0].declared.contains(&DELEGATION_PROMISE));
    assert_eq!(out[0].why, "", "成功时不带原因");
}

/// **先到先得**：同一次调用在两个分支各记一笔时，先记的那次留下。
///
/// 收口路径（未执行 / 中止）会在末尾再记一次，它不能把跑成功的那次改写成没跑。
#[test]
fn first_record_wins() {
    let mut out = Vec::new();
    record_effect(
        &mut out,
        &decls(),
        "c-1",
        "agent_run",
        &json!({ "prompt": "评审" }),
        true,
        "完成",
        "",
    );
    record_effect(
        &mut out,
        &decls(),
        "c-1",
        "agent_run",
        &json!({ "prompt": "评审" }),
        false,
        "",
        "未执行（中止或同批前序未结束）",
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].success, "跑成功的那次不能被收口改写成没跑");
}

/// 没成功 ⇒ 原因随值走：违约理由由调用点给（协议失败 / 超限 / 拒止 / 中止各有各的话）。
#[test]
fn failure_carries_why() {
    let mut out = Vec::new();
    record_effect(
        &mut out,
        &decls(),
        "c-2",
        "agent_run",
        &json!({}),
        false,
        "",
        "参数 JSON 解析失败，未执行",
    );
    assert_eq!(out[0].why, "参数 JSON 解析失败，未执行");
}
