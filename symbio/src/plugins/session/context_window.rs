//! 上下文窗口策略纯函数 —— 历史工具调用的分层滑窗骨架化。
//!
//! Phase sink：自 `symbio_core/context_window.rs` 下沉至 session 插件——E-② 后
//! 该纯函数的唯一消费者是 session 请求视图构建（`compression.rs::build_request_view`），
//! "跨插件共享"的前提（model 构建 request view）已随 Phase E-② 循环族下沉消失，
//! 属单一模块私有设施，不再置于 core 共享层。

use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use crate::symbio_core::ToolContextRetention;
use std::collections::{HashMap, HashSet};

use super::text_split::truncate_tokens;

/// 骨架化结果摘要的 token 预算（成功/失败摘要共用上限，≈ 4 行文本）。
const SKELETON_DIGEST_TOKEN_CAP: usize = 48;

/// 判断工具结果是否为失败结果。
///
/// 结构化优先：`meta.success == false` 或 `meta.failure_kind` 存在即为失败
/// （tool_executor 对失败结果统一打 `success:false` + `failure_kind` 标记）；
/// 文本启发式（"Error"/"failed"）仅作无 meta 时的兜底。
fn is_failed_result(msg: &ChatMessage, preview: &str) -> bool {
    if let Some(meta) = msg.meta.as_ref().and_then(|m| m.as_object()) {
        // 结构化标记存在即直接采信、短路返回，不再回落文本启发式：
        // 否则正文里偶然出现的 "failed"/"Error" 字样（如测试统计
        // "0 failed tests"）会把成功结果误判为失败
        if let Some(success) = meta.get("success").and_then(|v| v.as_bool()) {
            return !success;
        }
        if meta.get("failure_kind").is_some() {
            return true;
        }
    }
    preview.contains("Error") || preview.contains("failed")
}

/// 生成失败结果的错误摘要（一行）。
///
/// 失败是信息性的：错误类型与一句话原因是模型决定"换路径重试还是放弃"的
/// 关键输入，骨架化时必须保留，否则模型只能对同批文件反复盲试。
fn error_digest(msg: &ChatMessage, preview: &str) -> String {
    let kind = msg
        .meta
        .as_ref()
        .and_then(|m| m.get("failure_kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("error");
    let tool_name = msg
        .name
        .clone()
        .or_else(|| {
            msg.meta
                .as_ref()
                .and_then(|m| m.get("tool_name"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();
    // 错误正文里最像"原因"的一行：优先取含 Error/error 的首行，否则取首个非空行
    let cause = preview
        .lines()
        .filter(|l| !l.trim().is_empty())
        .find(|l| l.contains("Error") || l.contains("error"))
        .or_else(|| preview.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("");
    let prefix = if tool_name.is_empty() {
        kind.to_string()
    } else {
        format!("{} ({})", kind, tool_name)
    };
    format!(
        "{}: {}",
        prefix,
        truncate_tokens(cause, SKELETON_DIGEST_TOKEN_CAP / 2)
    )
}

/// 判断一行是否为"无信息量碎片"：骨架化摘要的候选行若只由闭合括号、
/// 行号、结构标点、空白构成，取它作摘要等于没取（真实会话实证：
/// `Summary: 150: }]` 迫使模型整文件重读）。
///
/// 判定口径：剥离空白、`{}[],:;.-=…` 结构字符与 ASCII 数字后无剩余字符，
/// 即视为碎片（覆盖 `}`、`]`、`150: }]`、`---`、`====` 等形态）；
/// 含任何字母/文字（含 CJK）的行不算碎片。
fn is_fragment_line(line: &str) -> bool {
    line.chars().all(|c| {
        c.is_whitespace()
            || c.is_ascii_digit()
            || matches!(
                c,
                '{' | '}' | '[' | ']' | ',' | ':' | ';' | '.' | '-' | '=' | '…'
            )
    })
}

/// 生成成功结果的首行摘要（一行）。
///
/// 质量底线（真实会话实证驱动）：首个非空行若是碎片行（`}`、`]`、`150: }]`），
/// 取它作摘要毫无信息量；跳过碎片行找首个**有内容**的行。全碎片/全空白时
/// 返回空串——由调用方省略 Summary 子句（占位符仍保留 successfully 标记）。
///
/// 只取首个非空行（多为文件首行标题、命令输出首行、JSON 首键），预算内
/// 截断——既给模型"读过什么"的锚点，又不让摘要本身变成新的开销。
/// JSON 输入走 `json_digest` 语义摘要（P2-1）：裸切片 `{"count":16,"entries…`
/// 对模型毫无信息量，应提取 count/首条目关键字段。
fn first_line_digest(preview: &str) -> String {
    let first = preview
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !is_fragment_line(l))
        .unwrap_or("");
    if first.starts_with('{') || first.starts_with('[') {
        if let Some(digest) = json_digest(first) {
            return digest;
        }
    }
    truncate_tokens(first, SKELETON_DIGEST_TOKEN_CAP)
}

/// JSON 输出的语义摘要（P2-1）：提取对"下一步决策"最有用的关键字段。
///
/// 提取优先级：count/total/total_count（规模感）→ 首条目关键字段
/// （name/path/title/id 等定位键）→ 顶层键名列表（结构感）。
/// 摘要预算仍受 `SKELETON_DIGEST_TOKEN_CAP` 约束；解析失败返回 None
/// （退回首行切片，不因摘要逻辑引入新故障面）。
fn json_digest(first: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(first).ok()?;
    let mut parts: Vec<String> = Vec::new();

    // 规模字段：数组输入不占 count——`items=string/string` 已同时表达规模与元素类型，
    // 数组走 count 只会让结构兜底分支变成死代码（测试暴露的逻辑冲突）。
    let count = match &value {
        serde_json::Value::Object(obj) => ["count", "total", "total_count", "size"]
            .iter()
            .find_map(|k| obj.get(*k).and_then(|v| v.as_u64()))
            .map(|c| c as usize),
        _ => None,
    };
    if let Some(c) = count {
        parts.push(format!("count={c}"));
    }

    // 首条目定位字段（数组首项或对象中名为 entries/results/items 的数组首项）
    let first_item = match &value {
        serde_json::Value::Array(items) => items.first().cloned(),
        serde_json::Value::Object(obj) => ["entries", "results", "items", "data", "files"]
            .iter()
            .find_map(|k| obj.get(*k).and_then(|v| v.as_array()))
            .and_then(|items| items.first().cloned()),
        _ => None,
    };
    if let Some(item) = first_item {
        if let Some(obj) = item.as_object() {
            let fields: Vec<String> = obj
                .iter()
                .filter(|(k, _)| {
                    ["name", "path", "file", "title", "id", "type", "status"].contains(&k.as_str())
                })
                .map(|(k, v)| match v {
                    serde_json::Value::String(s) => format!("{k}={}", truncate_tokens(s, 24)),
                    other => format!("{k}={other}"),
                })
                .collect();
            if !fields.is_empty() {
                parts.push(format!("first[{}]", fields.join(",")));
            }
        }
    }

    // 兜底结构感：顶层键名（对象）或元素类型（数组）
    if parts.is_empty() {
        match &value {
            serde_json::Value::Object(obj) => {
                let keys: Vec<&String> = obj.keys().take(8).collect();
                if !keys.is_empty() {
                    parts.push(format!(
                        "keys={}",
                        keys.iter()
                            .map(|k| k.as_str())
                            .collect::<Vec<_>>()
                            .join(",")
                    ));
                }
            }
            serde_json::Value::Array(items) => {
                let kinds: Vec<&str> = items
                    .iter()
                    .take(3)
                    .map(|v| match v {
                        serde_json::Value::Object(_) => "object",
                        serde_json::Value::String(_) => "string",
                        serde_json::Value::Number(_) => "number",
                        _ => "other",
                    })
                    .collect();
                if !kinds.is_empty() {
                    parts.push(format!("items={}", kinds.join("/")));
                }
            }
            _ => {}
        }
    }

    if parts.is_empty() {
        return None;
    }
    Some(truncate_tokens(&parts.join(" "), SKELETON_DIGEST_TOKEN_CAP))
}

/// 骨架化 ToolCall 参数时保留的定位参数键（按优先级取首个命中项）。
/// 模型凭锚点把历史调用与其结果对上号（"读过哪个文件/跑过什么命令"）——
/// 否则滚出窗口后只剩"[行 585-602，共 621 行]"这类无主摘要，链路断裂
/// （真实会话实证：模型面对结果摘要却不知对应哪个文件，只能整目录重读）。
const ANCHOR_PARAM_KEYS: &[&str] = &[
    "path",
    "file_path",
    "command",
    "url",
    "pattern",
    "query",
    "name",
];

/// 从参数 JSON 提取首个命中的定位参数作为锚点（截断至一行）。
/// 非 JSON 参数或无命中键时返回 None（退回通用占位符）。
fn anchor_of_args(args: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(args).ok()?;
    let obj = value.as_object()?;
    for key in ANCHOR_PARAM_KEYS {
        if let Some(v) = obj.get(*key).and_then(|v| v.as_str()) {
            if !v.trim().is_empty() {
                return Some(format!("{key}={}", truncate_tokens(v, 24)));
            }
        }
    }
    None
}

/// 混合滑动窗口过滤历史工具调用 (Layered Sliding Window)
///
/// 两级压缩规则（只骨架化、不删除，保持 tool_call/tool_result 配对合法）：
/// 1. 全局窗口：超出 `max_active_tool_calls` 的历史 ToolCall —— 自身参数骨架化、
///    其结果子节点骨架化；
/// 2. 工具级保留策略：`retention` 按工具 CapabilityMeta.name（裸名，如
///    "todo_write"）→ 声明策略映射。声明了 `LastOnly` / `LastN(n)` 的工具，其更早
///    的调用即使仍在全局窗口内，参数与结果同样骨架化（同工具"重复全量写入"的
///    历史对后续推理无参考价值）。映射由调用方在运行时按工具声明动态构建
///    （session 会话循环内直接用 CapabilityVisitor），**不持久化**、
///    不写入任何消息 meta。
///
/// **优先级：工具级保留策略 > 全局窗口。**声明了保留策略的工具，其"最近 N 次"
/// 调用即使已滚出全局窗口也完整保留——全局窗口按 ToolCall 总数计数，长会话中
/// 清单/状态类工具的最新调用必然滚出窗口，而最新状态恰是唯一有效状态。
///
/// 骨架化占位符**保留一行摘要**：失败结果保留错误类型与首行原因（失败是
/// 信息性的，错误详情决定模型换路径重试还是放弃）；成功结果保留首行摘要
/// （给模型"读过什么"的锚点，避免被迫整文件重读）。摘要判定结构化优先
/// （`meta.success` / `meta.failure_kind`），文本启发式兜底。
///
/// 骨架化占位符同时**保留定位锚点**：参数骨架化保留首个定位参数
/// （path/command/url/pattern/query/name），结果摘要前缀配对调用的锚点——
/// 模型凭锚点把历史调用与结果对上号，避免"读过某文件第 N 行却不知是哪个
/// 文件"的链路断裂。
///
/// ToolCall 节点的 name 是 LLM 可见全名（如 "local/todo_write"），此处匹配时
/// 取最后一个 '/' 后的短名。
pub fn apply_layered_sliding_window(
    messages: &[ChatMessage],
    max_active_tool_calls: usize,
    retention: &HashMap<String, ToolContextRetention>,
) -> Vec<ChatMessage> {
    // ── 收集全部 ToolCall（保序），计算两级"失效"判定 ──────────────────────
    // info:         ToolCall id → (全局序号, 工具短名, 自声明保留策略)
    // per_tool_seq: 工具短名 → 该工具的调用 id 序列（工具级保留策略用）
    let mut info: HashMap<String, (usize, String, Option<ToolContextRetention>)> = HashMap::new();
    let mut per_tool_seq: HashMap<String, Vec<String>> = HashMap::new();
    for msg in messages {
        if msg.msg_type == Some(MessageType::ToolCall) {
            let idx = info.len();
            let full_name = msg.name.clone().unwrap_or_default();
            // "local/todo_write" → "todo_write"（与 CapabilityMeta.name 对齐）
            let name = full_name
                .rsplit('/')
                .next()
                .unwrap_or(&full_name)
                .to_string();
            let declared = retention.get(&name).copied();
            info.insert(msg.id.clone(), (idx, name.clone(), declared));
            per_tool_seq.entry(name).or_default().push(msg.id.clone());
        }
    }

    let active_threshold = info.len().saturating_sub(max_active_tool_calls);

    // 判定某个 ToolCall 是否"失效"（需骨架化）。
    // **优先级：工具级保留策略 > 全局窗口**——声明了 LastOnly/LastN 的工具，
    // 即使最新调用已滚出全局窗口（全局窗口只按 ToolCall 总数计数，长会话必然
    // 发生），其"最近 N 次"仍完整保留：这类工具的最新状态是唯一有效状态，
    // 骨架化它等于让模型丢失任务清单/最新探测结果（曾在真实会话中引发重写）。
    let is_stale = |tc_id: &str| -> bool {
        let Some((idx, name, declared)) = info.get(tc_id) else {
            return false;
        };
        let Some(ret) = declared else {
            // 未声明策略的工具：仅受全局窗口约束
            return *idx < active_threshold;
        };
        let keep = ret.keep_count() as usize;
        let Some(seq) = per_tool_seq.get(name) else {
            return false;
        };
        match seq.iter().position(|id| id == tc_id) {
            Some(pos) => pos + keep < seq.len(),
            None => false,
        }
    };

    let tool_call_ids_set: HashSet<&String> = info.keys().collect();

    let mut filtered = Vec::with_capacity(messages.len());
    for msg in messages {
        let mut new_msg = msg.clone();

        if msg.msg_type == Some(MessageType::ToolCall) {
            if is_stale(&msg.id) {
                // ToolCall 组合节点自身 content = 请求参数 JSON → 骨架化。
                // 保留定位锚点（path/command/url 等），让历史调用可与结果对号：
                let placeholder = msg
                    .content
                    .as_ref()
                    .map(|c| c.to_text())
                    .and_then(|args| anchor_of_args(&args))
                    .map(|a| {
                        format!(
                            "[System Info: Tool call input parameters skeletonized to save context. ({a})]"
                        )
                    })
                    .unwrap_or_else(|| {
                        "[System Info: Tool call input parameters skeletonized to save context.]"
                            .to_string()
                    });
                new_msg.content = Some(MessageContent::Text(placeholder));
                if let Some(mut meta) = new_msg.meta.as_ref().and_then(|m| m.as_object()).cloned() {
                    meta.remove("tool_calls");
                    new_msg.meta = Some(serde_json::Value::Object(meta));
                }
            }
        } else {
            // 父节点是失效的 ToolCall → 子节点（工具结果/请求参数）骨架化
            let parent_stale = msg
                .parent_id
                .as_ref()
                .map(|pid| tool_call_ids_set.contains(pid) && is_stale(pid))
                .unwrap_or(false);

            if parent_stale {
                let preview = msg
                    .content
                    .as_ref()
                    .map(|c| c.to_text())
                    .unwrap_or_default();
                // 结果摘要前拼接配对 ToolCall 的锚点（path/command 等），
                // 无主摘要（"读过某文件第 585 行"却不知是哪个文件）是链路断裂的根源
                let anchor = msg
                    .parent_id
                    .as_deref()
                    .and_then(|pid| messages.iter().find(|m| m.id == *pid))
                    .and_then(|tc| tc.content.as_ref().map(|c| c.to_text()))
                    .and_then(|args| anchor_of_args(&args))
                    .map(|a| format!(" ({a})"))
                    .unwrap_or_default();
                let label = if msg.role == Some(MessageRole::Tool) {
                    // 失败结果保留错误摘要、成功结果保留首行摘要：
                    // 失败是信息性的（错误详情决定下一步动作），成功摘要避免
                    // "隔轮即忘"迫使模型整文件重读。摘要远短于原文，仍净省 token。
                    if is_failed_result(msg, &preview) {
                        let detail = error_digest(msg, &preview);
                        format!(
                            "[System Info: Tool result failed{anchor}: {}. Full output skeletonized.]",
                            detail
                        )
                    } else {
                        let hint = first_line_digest(&preview);
                        if hint.is_empty() {
                            format!(
                                "[System Info: Tool result received successfully{anchor}. Output skeletonized.]"
                            )
                        } else {
                            format!(
                                "[System Info: Tool result received successfully{anchor}. Output skeletonized. Summary: {}]",
                                hint
                            )
                        }
                    }
                } else {
                    format!(
                        "[System Info: Tool call input parameters skeletonized to save context.{anchor}]"
                    )
                };
                new_msg.content = Some(MessageContent::Text(label));
            }
        }

        filtered.push(new_msg);
    }

    filtered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::schemas::session::chat_message::MessageStatus;

    fn tc_msg(id: &str, name: &str, args: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            name: Some(name.to_string()),
            content: Some(MessageContent::Text(args.to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        }
    }

    fn tool_result(parent: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: format!("{parent}-result"),
            parent_id: Some(parent.to_string()),
            role: Some(MessageRole::Tool),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(text.to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        }
    }

    fn text(content: &str) -> ChatMessage {
        ChatMessage {
            id: format!("t-{}", content),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(content.to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        }
    }

    /// 构建 短工具名 → 保留策略 映射（模拟会话循环运行时从 CapabilityVisitor 动态解析）
    fn retention_map(
        entries: &[(&str, ToolContextRetention)],
    ) -> HashMap<String, ToolContextRetention> {
        entries.iter().map(|(n, r)| (n.to_string(), *r)).collect()
    }

    fn param_of(msg: &ChatMessage) -> String {
        msg.content
            .as_ref()
            .map(|c| c.to_text())
            .unwrap_or_default()
    }

    /// LastOnly：同名工具多次调用，仅最新一次参数/结果完整，旧的骨架化
    #[test]
    fn last_only_keeps_only_latest_call_of_same_tool() {
        let messages = vec![
            text("第一轮"),
            tc_msg("tc1", "local/todo_write", r#"{"todos":"v1 很长很长"}"#),
            tool_result("tc1", "ok"),
            tc_msg("tc2", "local/todo_write", r#"{"todos":"v2 很长很长"}"#),
            tool_result("tc2", "ok"),
            text("第二轮"),
        ];
        let ret = retention_map(&[("todo_write", ToolContextRetention::LastOnly)]);
        let out = apply_layered_sliding_window(&messages, 15, &ret);

        let tc1 = out.iter().find(|m| m.id == "tc1").unwrap();
        let tc2 = out.iter().find(|m| m.id == "tc2").unwrap();
        assert!(
            param_of(tc1).contains("skeletonized"),
            "旧调用参数应被骨架化: {:?}",
            tc1.content
        );
        assert_eq!(
            param_of(tc2),
            r#"{"todos":"v2 很长很长"}"#,
            "最新调用参数应原样保留"
        );

        let r1 = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("tc1"))
            .unwrap();
        let r2 = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("tc2"))
            .unwrap();
        assert!(param_of(r1).contains("skeletonized"), "旧结果应被骨架化");
        assert_eq!(param_of(r2), "ok", "最新结果应原样保留");
    }

    /// LastN(2)：保留最近 2 次，更早的骨架化
    #[test]
    fn last_n_keeps_n_latest_calls() {
        let messages = vec![
            tc_msg("a1", "local/search", r#"{"q":"1"}"#),
            tool_result("a1", "r1"),
            tc_msg("a2", "local/search", r#"{"q":"2"}"#),
            tool_result("a2", "r2"),
            tc_msg("a3", "local/search", r#"{"q":"3"}"#),
            tool_result("a3", "r3"),
        ];
        let ret = retention_map(&[("search", ToolContextRetention::LastN(2))]);
        let out = apply_layered_sliding_window(&messages, 15, &ret);

        let a1 = out.iter().find(|m| m.id == "a1").unwrap();
        let a2 = out.iter().find(|m| m.id == "a2").unwrap();
        let a3 = out.iter().find(|m| m.id == "a3").unwrap();
        assert!(param_of(a1).contains("skeletonized"));
        assert_eq!(param_of(a2), r#"{"q":"2"}"#);
        assert_eq!(param_of(a3), r#"{"q":"3"}"#);
    }

    /// 未声明策略的工具不受影响（回归保护：默认行为与旧版一致）
    #[test]
    fn tools_without_retention_are_unaffected() {
        let messages = vec![
            tc_msg("x1", "vdfs_read", r#"{"path":"a.rs"}"#),
            tool_result("x1", "content"),
            tc_msg("x2", "vdfs_read", r#"{"path":"b.rs"}"#),
            tool_result("x2", "content2"),
        ];
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 15, &ret);
        let x1 = out.iter().find(|m| m.id == "x1").unwrap();
        let x2 = out.iter().find(|m| m.id == "x2").unwrap();
        assert_eq!(param_of(x1), r#"{"path":"a.rs"}"#);
        assert_eq!(param_of(x2), r#"{"path":"b.rs"}"#);
    }

    /// 全局窗口：超出的 ToolCall 自身参数也骨架化（压缩 tool_call 参数占比）
    #[test]
    fn global_window_skeletonizes_tool_call_params() {
        let mut messages = Vec::new();
        for i in 0..6 {
            messages.push(tc_msg(
                &format!("g{i}"),
                "vdfs_read",
                &format!(r#"{{"path":"f{i}.rs","blob":"x".repeat(50)}}"#),
            ));
            messages.push(tool_result(&format!("g{i}"), "done"));
        }
        // 全局窗口只保留最近 2 个
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 2, &ret);
        let g0 = out.iter().find(|m| m.id == "g0").unwrap();
        let g3 = out.iter().find(|m| m.id == "g3").unwrap();
        let g4 = out.iter().find(|m| m.id == "g4").unwrap();
        let g5 = out.iter().find(|m| m.id == "g5").unwrap();
        assert!(param_of(g0).contains("skeletonized"));
        assert!(param_of(g3).contains("skeletonized"));
        assert!(!param_of(g4).contains("skeletonized"));
        assert!(!param_of(g5).contains("skeletonized"));
        // 结果子节点同样骨架化
        let r0 = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("g0"))
            .unwrap();
        assert!(param_of(r0).contains("skeletonized"));
        // 状态保持 Completed（不产生 Failed/孤儿，配对合法）
        assert_eq!(g0.status, Some(MessageStatus::Completed));
    }

    /// 工具级 LastOnly 优先于全局窗口（即使仍在全局窗口内也骨架化旧调用）
    #[test]
    fn retention_applies_even_within_global_window() {
        let messages = vec![
            tc_msg("p1", "local/todo_write", r#"{"v":1}"#),
            tool_result("p1", "ok"),
            tc_msg("p2", "local/todo_write", r#"{"v":2}"#),
            tool_result("p2", "ok"),
        ];
        // 全局窗口 15 足够大，但 LastOnly 仍骨架化 p1
        let ret = retention_map(&[("todo_write", ToolContextRetention::LastOnly)]);
        let out = apply_layered_sliding_window(&messages, 15, &ret);
        let p1 = out.iter().find(|m| m.id == "p1").unwrap();
        let p2 = out.iter().find(|m| m.id == "p2").unwrap();
        assert!(param_of(p1).contains("skeletonized"));
        assert_eq!(param_of(p2), r#"{"v":2}"#);
    }

    /// 回归：声明 LastOnly 的工具，其**最新一次**调用滚出全局窗口后仍完整保留。
    /// 旧实现全局窗口判定优先，导致长会话中任务清单/最新状态被骨架化丢失
    /// （真实会话中发生过：模型因看不到清单而整单重写）。
    #[test]
    fn last_only_latest_call_survives_beyond_global_window() {
        let mut messages = Vec::new();
        // 20 个无策略工具调用把全局窗口（15）挤满，todo_write 最新调用被推出窗口
        for i in 0..20 {
            messages.push(tc_msg(
                &format!("f{i}"),
                "vdfs_read",
                &format!(r#"{{"path":"f{i}.rs"}}"#),
            ));
            messages.push(tool_result(&format!("f{i}"), "content"));
        }
        messages.push(tc_msg(
            "latest",
            "local/todo_write",
            r#"{"todos":"清单 v3"}"#,
        ));
        messages.push(tool_result("latest", "已更新任务清单，共 3 项。"));
        let ret = retention_map(&[("todo_write", ToolContextRetention::LastOnly)]);
        let out = apply_layered_sliding_window(&messages, 15, &ret);

        // 无策略工具：全局窗口语义不变（前 6 条被骨架化）
        assert!(param_of(out.iter().find(|m| m.id == "f0").unwrap()).contains("skeletonized"));
        // LastOnly 工具：最新调用虽在全局窗口之外（第 21 个 ToolCall），仍完整保留
        let latest = out.iter().find(|m| m.id == "latest").unwrap();
        assert_eq!(
            param_of(latest),
            r#"{"todos":"清单 v3"}"#,
            "LastOnly 最新调用不应被全局窗口骨架化"
        );
        let latest_result = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("latest"))
            .unwrap();
        assert_eq!(param_of(latest_result), "已更新任务清单，共 3 项。");
    }

    /// 骨架化占位符保留摘要：失败结果保留错误类型与首行原因
    #[test]
    fn skeletonized_failure_keeps_error_digest() {
        let messages = vec![
            tc_msg("e1", "vdfs_read", r#"{"path":"agent/README.md"}"#),
            {
                let mut m = tool_result("e1", "读取失败：os error 2 (系统找不到指定的文件。)");
                m.meta = Some(serde_json::json!({
                    "success": false,
                    "failure_kind": "not_found",
                }));
                m
            },
        ];
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 0, &ret);
        let r = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("e1"))
            .unwrap();
        let content = param_of(r);
        assert!(
            content.contains("failed"),
            "失败骨架化应标注 failed: {content}"
        );
        assert!(
            content.contains("not_found"),
            "失败骨架化应保留 failure_kind: {content}"
        );
        assert!(
            content.contains("os error 2"),
            "失败骨架化应保留错误原因: {content}"
        );
    }

    /// 骨架化占位符保留摘要：成功结果保留首行摘要
    #[test]
    fn skeletonized_success_keeps_first_line_digest() {
        let messages = vec![
            tc_msg("s1", "vdfs_read", r#"{"path":"session/README.md"}"#),
            tool_result("s1", "# session 插件\n\n会话编排唯一入口……（后续 300 行）"),
        ];
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 0, &ret);
        let r = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("s1"))
            .unwrap();
        let content = param_of(r);
        assert!(
            content.contains("successfully"),
            "成功骨架化应标注 successfully: {content}"
        );
        assert!(
            content.contains("# session 插件"),
            "成功骨架化应保留首行摘要: {content}"
        );
        assert!(
            !content.contains("后续 300 行"),
            "骨架化不应保留全文: {content}"
        );
    }

    /// 质量底线：首个非空行是碎片行（`150: }]`）时跳过取下一个有内容的行，
    /// 而非产出 `Summary: 150: }]` 这类无信息量摘要（真实会话实证的痛点）。
    #[test]
    fn skeletonized_success_skips_fragment_first_line() {
        let messages = vec![
            tc_msg("s2", "local/grep", r#"{"pattern":"TODO"}"#),
            tool_result(
                "s2",
                "150: }]\n\nsrc/main.rs:12: TODO refactor\nsrc/lib.rs:3: TODO docs",
            ),
        ];
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 0, &ret);
        let r = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("s2"))
            .unwrap();
        let content = param_of(r);
        assert!(
            content.contains("src/main.rs:12: TODO refactor"),
            "应跳过碎片行取首个有内容行: {content}"
        );
        assert!(
            !content.contains("150: }]"),
            "碎片行不应出现在摘要: {content}"
        );
    }

    /// 质量底线：全碎片内容（仅闭合括号/标点）时省略 Summary 子句而非输出空摘要
    #[test]
    fn skeletonized_success_all_fragment_omits_summary() {
        let messages = vec![
            tc_msg("s3", "local/grep", r#"{"pattern":"TODO"}"#),
            tool_result("s3", "}]"),
        ];
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 0, &ret);
        let r = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("s3"))
            .unwrap();
        let content = param_of(r);
        assert!(
            content.contains("successfully"),
            "占位符仍应标注 successfully: {content}"
        );
        assert!(
            !content.contains("Summary:"),
            "全碎片时应省略 Summary 子句: {content}"
        );
    }

    /// 骨架化摘要判定：meta.success 优先于文本启发式
    #[test]
    fn failure_detection_prefers_structured_meta() {
        let messages = vec![tc_msg("m1", "local/shell", r#"{"command":"dir"}"#), {
            // 文本含 "failed" 但 meta.success=true → 仍视为成功
            let mut m = tool_result("m1", "0 failed tests, all passed");
            m.meta = Some(serde_json::json!({ "success": true }));
            m
        }];
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 0, &ret);
        let r = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("m1"))
            .unwrap();
        assert!(
            param_of(r).contains("successfully"),
            "meta.success=true 应判定为成功"
        );
    }

    /// 骨架化保留定位锚点：参数骨架化保留首个定位参数，
    /// 结果摘要前缀配对调用的锚点（真实会话实证：无主摘要"[行 585-602]"
    /// 让模型不知对应哪个文件，只能整目录重读）
    #[test]
    fn skeletonized_call_and_result_keep_anchor_param() {
        let messages = vec![
            tc_msg(
                "a1",
                "vdfs_read",
                r#"{"path":"gateway/server.rs","limit":50}"#,
            ),
            tool_result("a1", "[行 585-602，共 621 行]"),
        ];
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 0, &ret);

        let call = out.iter().find(|m| m.id == "a1").unwrap();
        let call_text = param_of(call);
        assert!(
            call_text.contains("(path=gateway/server.rs)"),
            "参数骨架化应保留 path 锚点: {call_text}"
        );
        assert!(
            !call_text.contains("limit"),
            "锚点之外的参数不应保留: {call_text}"
        );

        let result = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("a1"))
            .unwrap();
        let result_text = param_of(result);
        assert!(
            result_text.contains("(path=gateway/server.rs)"),
            "结果摘要应前缀配对调用的锚点: {result_text}"
        );
        assert!(
            result_text.contains("[行 585-602"),
            "首行摘要仍应保留: {result_text}"
        );
    }

    /// 无定位参数的调用（如 todo_write 的 todos）退回通用占位符，不强行编造锚点
    #[test]
    fn skeletonized_without_anchor_falls_back_to_generic() {
        let messages = vec![
            tc_msg("w1", "local/todo_write", r#"{"todos":"清单 v1 很长很长"}"#),
            tool_result("w1", "已更新"),
        ];
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 0, &ret);
        let call = out.iter().find(|m| m.id == "w1").unwrap();
        assert_eq!(
            param_of(call),
            "[System Info: Tool call input parameters skeletonized to save context.]",
            "无锚点参数应退回通用占位符"
        );
    }

    /// P2-1：单行 JSON 结果骨架化时应产出语义摘要（count + 首条目定位字段），
    /// 而非 96 字符裸切片（真实会话实证：`{"count":16,"entries":[{"modified":…`
    /// 对模型毫无信息量，迫使重跑工具）。
    #[test]
    fn json_result_gets_semantic_digest() {
        let json_body = format!(
            r#"{{"count":16,"entries":[{{"name":"main.rs","path":"src/main.rs","modified":1788948661}}],"next":[{}]}}"#,
            vec![r#""x""#; 400].join(",")
        );
        let messages = vec![
            tc_msg("j1", "local/glob", r#"{"pattern":"**/*.rs"}"#),
            tool_result("j1", &json_body),
        ];
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 0, &ret);
        let result = out
            .iter()
            .find(|m| m.parent_id.as_deref() == Some("j1"))
            .unwrap();
        let text = param_of(result);
        assert!(text.contains("count=16"), "应提取规模字段: {text}");
        assert!(
            text.contains("name=main.rs"),
            "应提取首条目定位字段: {text}"
        );
        assert!(!text.contains(r#""next""#), "不应残留大体积切片: {text}");
    }

    /// P2-1：无 count/条目定位字段的对象 → 键名列表兜底；数组 → 元素类型。
    #[test]
    fn json_digest_falls_back_to_structure() {
        // 顶层键名兜底
        let keys_only = r#"{"alpha":1,"beta":2,"gamma":3,"delta":4}"#;
        let d1 = first_line_digest(keys_only);
        assert!(d1.contains("keys=alpha,beta"), "对象应兜底键名列表: {d1}");
        // 纯数组元素类型兜底
        let d2 = first_line_digest(r#"["a","b","c"]"#);
        assert!(
            d2.contains("items=string/string/string"),
            "数组应兜底元素类型: {d2}"
        );
        // 非 JSON / 解析失败 → 原有首行切片行为不变（回归保护）
        let plain = "just a plain line of output";
        assert_eq!(first_line_digest(plain), plain);
    }
}
