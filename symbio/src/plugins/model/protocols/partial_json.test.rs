//! `JsonLineExtractor` 自身的测试：路径判定、转义解码、跨块不变量。

use super::*;
use std::sync::{Arc, Mutex};

/// 只提取指定路径的字符串，产出 `ContentDelta`——方便用 [`text_of`] 拼回文本。
struct Probe {
    target: Vec<&'static str>,
}

impl Probe {
    fn new(target: Vec<&'static str>) -> Self {
        Self { target }
    }
}

impl PartialJsonSink for Probe {
    fn begin_string(&mut self, path: &FieldPath<'_>) -> StrAction {
        if path.is(self.target.as_slice()) {
            StrAction::Emit
        } else {
            StrAction::Skip
        }
    }

    fn text(&mut self, t: &str, out: &mut Vec<ProtocolEvent>) {
        out.push(ProtocolEvent::ContentDelta(t.to_string()));
    }
}

/// 记录标量回调（`scalar` 没有 `out`，只能走共享状态）。
#[derive(Default)]
struct ScalarLog {
    seen: Vec<(String, String)>,
}

struct ScalarProbe {
    log: Arc<Mutex<ScalarLog>>,
}

impl PartialJsonSink for ScalarProbe {
    fn begin_string(&mut self, _path: &FieldPath<'_>) -> StrAction {
        StrAction::Skip
    }

    fn text(&mut self, _t: &str, _out: &mut Vec<ProtocolEvent>) {}

    fn scalar(&mut self, path: &FieldPath<'_>, raw: &str) {
        self.log
            .lock()
            .unwrap()
            .seen
            .push((path.keys().join("."), raw.to_string()));
    }
}

fn extract(line: &str, target: Vec<&'static str>, chunk: usize) -> String {
    let mut ext = JsonLineExtractor::new(Probe::new(target));
    text_of(&feed_chunked(&mut ext, line, chunk))
}

/// **本模块最重要的一条**：无论块边界落在哪里，增量吐出的文本都必须与完整值一致。
///
/// core 会把已发出的长度记成前缀、完整行到达时按前缀截断，两边一旦不一致就会
/// 重复或吃字。逐字节切分是最狠的验证方式。
#[test]
fn extracts_target_across_every_chunk_boundary() {
    let line = r#"data: {"a":{"b":"hello \u4e2d\u6587 \"q\" \\ / \n end"},"c":1}"#;
    let expected = "hello 中文 \"q\" \\ / \n end";
    for chunk in 1..=line.len() {
        assert_eq!(
            extract(line, vec!["a", "b"], chunk),
            expected,
            "chunk={chunk}"
        );
    }
}

/// 解码规则必须与 `serde_json` 一致——否则「增量前缀」与「完整行文本」对不上。
#[test]
fn decode_matches_serde_json() {
    let line = r#"{"a":{"b":"line1\nline2\ttab \u4e2d\u6587 \uD83D\uDE00 \"q\" \\ \/ \u0001"}}"#;
    let expected = serde_json::from_str::<serde_json::Value>(line).unwrap()["a"]["b"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(extract(line, vec!["a", "b"], 1), expected);
    assert_eq!(extract(line, vec!["a", "b"], 7), expected);
}

/// 不完整的转义序列**不产出**：`\` 之后的字符决定它是什么，猜不得。
/// 但它前面已经确定的部分照吐（那确实是最终文本的前缀）。
#[test]
fn holds_incomplete_escape_until_next_chunk() {
    let mut ext = JsonLineExtractor::new(Probe::new(vec!["b"]));
    let mut out = Vec::new();

    ext.push(r#"{"b":"abc\"#, &mut out);
    assert_eq!(text_of(&out), "abc");

    ext.push(r#"n def"}"#, &mut out);
    assert_eq!(text_of(&out), "abc\n def");
}

/// 代理对要等低代理项到齐才产出，否则会吐半个 emoji（且长度对不上）。
#[test]
fn holds_surrogate_pair_until_low_surrogate_arrives() {
    let mut ext = JsonLineExtractor::new(Probe::new(vec!["b"]));
    let mut out = Vec::new();

    ext.push(r#"{"b":"\uD83D"#, &mut out);
    assert_eq!(text_of(&out), "");

    ext.push(r#"\uDE00"}"#, &mut out);
    assert_eq!(text_of(&out), "😀");
}

/// 数组下标**不占路径位**：`a[0].b` 的路径就是 `["a", "b"]`。
#[test]
fn array_indices_do_not_contribute_path_segments() {
    assert_eq!(extract(r#"{"a":[{"b":"x"}]}"#, vec!["a", "b"], 1), "x");
    // 根是数组也成立
    assert_eq!(extract(r#"[{"b":"x"}]"#, vec!["b"], 1), "x");
}

/// 字符串里的 `{` `}` `[` `]` `,` `:` 都是普通字符，不得干扰结构判定。
#[test]
fn structural_chars_inside_strings_do_not_confuse_the_scanner() {
    let line = r#"{"a":"} ] { \" , : [","b":"x"}"#;
    assert_eq!(extract(line, vec!["b"], 1), "x");
    assert_eq!(extract(line, vec!["a"], 1), "} ] { \" , : [");
}

/// 键名不同就不匹配——「全等」而非「包含」，宁可漏不可错。
#[test]
fn only_exact_key_paths_match() {
    assert_eq!(extract(r#"{"a":{"bb":"x"}}"#, vec!["a", "b"], 1), "");
    assert_eq!(extract(r#"{"a":{"b":"x"}}"#, vec!["a", "b", "c"], 1), "");
}

/// 非 JSON 前缀（`data: ` / Gemini 的 `,` 包裹 / 数组包裹）必须跳过。
#[test]
fn skips_non_json_prefix() {
    assert_eq!(extract(r#"data: {"b":"x"}"#, vec!["b"], 1), "x");
    assert_eq!(extract(r#",{"b":"x"}"#, vec!["b"], 1), "x");
    assert_eq!(extract(r#"data:{"b":"x"}"#, vec!["b"], 1), "x");
    // 没有任何 JSON 起点 ⇒ 什么都不吐（`[DONE]` / 注释行 / keep-alive）
    assert_eq!(extract("data: [DONE]", vec!["b"], 1), "");
    assert_eq!(extract(": keep-alive", vec!["b"], 1), "");
}

/// 标量也要报出路径与原文——`output_index` / `index` 靠它决定事件归属。
#[test]
fn scalar_values_are_reported_with_path() {
    let log = Arc::new(Mutex::new(ScalarLog::default()));
    let mut ext = JsonLineExtractor::new(ScalarProbe {
        log: Arc::clone(&log),
    });
    let mut out = Vec::new();
    // 逐字节喂，覆盖「标量跨块」的情况
    for ch in r#"{"index":12,"x":[1,-2,true]}"#.chars() {
        let mut buf = [0u8; 4];
        ext.push(ch.encode_utf8(&mut buf), &mut out);
    }
    let seen = log.lock().unwrap().seen.clone();
    assert_eq!(
        seen,
        vec![
            ("index".to_string(), "12".to_string()),
            ("x".to_string(), "1".to_string()),
            ("x".to_string(), "-2".to_string()),
            ("x".to_string(), "true".to_string()),
        ]
    );
}
