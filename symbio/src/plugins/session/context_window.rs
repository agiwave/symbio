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

/// 骨架化结果摘要的 token 预算（成功/失败摘要共用上限）。
///
/// 从 48 上调至 64：`truncate_tokens` 的截断口径是"token × 2 → 字符上限"，
/// 48 token 只装得下约 6 个条目名——而列表类输出（`vdfs_list` 等）的摘要若
/// 装不满一屏条目名，模型就无法据此建模，只能重跑工具（真实会话实证）。
/// 64 token（≈128 字符）可列出 8 个典型文件名，仍是"一句话摘要"的量级。
const SKELETON_DIGEST_TOKEN_CAP: usize = 64;

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

/// JSON 语义摘要的整体解析上限（字节）。
///
/// pretty-print 的多行 JSON 只有整体解析才能拿到结构（只喂首行 `{` 必然失败，
/// 摘要退化成 `"count": 16,` 这类中间行切片）。解析是热路径，故设上限：
/// 64 KiB 内 serde 解析是微秒级，超出则退回首行切片，不引入性能面风险。
const JSON_DIGEST_PARSE_LIMIT: usize = 64 * 1024;

/// JSON 摘要最多列出的条目名数量。
///
/// 只报 `count` + 首条目时，23 个条目只看得见 1 个——模型无法判断"这里有没有
/// 我接下来要找的东西"，只能逐轮重跑工具（真实会话实证：审查本仓库的 agent
/// 自述「我永远在看摘要，无法一次建模」，每次核对 1-3 轮往返）。
/// 列出前若干条定位名后，同一份摘要即可支撑一次完整建模，代价约为几十 token。
const JSON_DIGEST_MAX_NAMES: usize = 8;

/// 条目名列表的字符预算推导率：`text_split::truncate_tokens` 的截断口径是
/// **token × 2 → 字符上限**（保守代理，CJK 1 字 ≈ 1 token 时仍成立）。
/// 摘要自身的上限由该口径决定，故条目名预算必须按同一口径推导，
/// 否则 names 列表会被截在半个名字上（实测踩坑：8 个名字被砍成 `entry7.md,…`，
/// 余量标注与收尾方括号一起丢失，摘要反而变成噪声）。
const DIGEST_CHAR_PROXY_PER_TOKEN: usize = 2;

/// `count=` / `names=[` / `,…+NN]` 三段固定前缀与后缀的字符预留。
const DIGEST_AFFIX_CHAR_RESERVE: usize = 24;

/// 单个条目的定位名（对象取 name/path/file/title/id，字符串元素直接采用）。
/// 单名预算 24 token，与旧实现首条目字段口径一致。
fn entry_name(item: &serde_json::Value) -> Option<String> {
    match item {
        serde_json::Value::String(s) => Some(truncate_tokens(s, 24)),
        serde_json::Value::Object(obj) => ["name", "path", "file", "title", "id"]
            .iter()
            .find_map(|k| obj.get(*k).and_then(|v| v.as_str()))
            .map(|s| truncate_tokens(s, 24)),
        _ => None,
    }
}

/// 提取条目名列表：数量上限 [`JSON_DIGEST_MAX_NAMES`]，总长受 `char_budget` 约束
/// （由调用方按摘要 token 上限推导，见 [`DIGEST_CHAR_PROXY_PER_TOKEN`]）。
fn entry_names(items: &[serde_json::Value], char_budget: usize) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut used = 0usize;
    for item in items.iter().take(JSON_DIGEST_MAX_NAMES) {
        let Some(name) = entry_name(item) else {
            continue;
        };
        let cost = name.chars().count() + 1; // +1 = 分隔逗号
        if used + cost > char_budget {
            break;
        }
        used += cost;
        names.push(name);
    }
    names
}

/// 列表形状探测：数组本体，或对象中名为 entries / results / items / data / files
/// 的数组字段（工具输出里列表几乎总是这五个键之一）。
fn list_items(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    match value {
        serde_json::Value::Array(items) => Some(items),
        serde_json::Value::Object(obj) => ["entries", "results", "items", "data", "files"]
            .iter()
            .find_map(|k| obj.get(*k).and_then(|v| v.as_array())),
        _ => None,
    }
}

/// 顶层键名兜底（结构感）：无条目可比时，至少让模型知道这个对象长什么样。
fn top_level_keys(value: &serde_json::Value) -> Option<String> {
    let obj = value.as_object()?;
    let keys: Vec<&str> = obj.keys().take(8).map(|k| k.as_str()).collect();
    if keys.is_empty() {
        return None;
    }
    Some(format!("keys={}", keys.join(",")))
}

/// 生成成功结果的语义摘要（一行）。
///
/// 质量底线（真实会话实证驱动）：首个非空行若是碎片行（`}`、`]`、`150: }]`），
/// 取它作摘要毫无信息量；跳过碎片行找首个**有内容**的行。全碎片/全空白时
/// 返回空串——由调用方省略 Summary 子句（占位符仍保留 successfully 标记）。
///
/// 只取首个非空行（多为文件首行标题、命令输出首行），预算内截断——既给模型
/// "读过什么"的锚点，又不让摘要本身变成新的开销。
/// JSON 输入走 `json_digest` 语义摘要（P2-1）：裸切片 `{"count":16,"entries…`
/// 对模型毫无信息量，应提取 count + 条目名列表；单行 JSON 直接解析，
/// pretty-print 多行 JSON 在 [`JSON_DIGEST_PARSE_LIMIT`] 内整体解析。
fn success_digest(preview: &str) -> String {
    let first = preview
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !is_fragment_line(l))
        .unwrap_or("");
    let head = preview.trim_start();
    if head.starts_with('{') || head.starts_with('[') {
        let candidate = if preview.len() <= JSON_DIGEST_PARSE_LIMIT {
            preview
        } else {
            first
        };
        if let Some(digest) = json_digest(candidate) {
            return digest;
        }
    }
    truncate_tokens(first, SKELETON_DIGEST_TOKEN_CAP)
}

/// JSON 输出的语义摘要（P2-1）：提取对"下一步决策"最有用的关键字段。
///
/// 提取优先级：规模（对象 count/total/total_count/size，数组取自身长度）→
/// 条目名列表（≥2 条时列前 [`JSON_DIGEST_MAX_NAMES`] 个定位名，并标注余量）
/// → 顶层键名列表（结构感）。摘要预算仍受 `SKELETON_DIGEST_TOKEN_CAP` 约束；
/// 解析失败返回 None（退回首行切片，不因摘要逻辑引入新故障面）。
fn json_digest(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut parts: Vec<String> = Vec::new();

    // 规模字段：对象取 count/total/total_count/size；数组取自身长度。
    // （旧实现让数组不占 count，理由是 `items=string/string` 已表达规模——
    //  实测该推理不成立：元素类型不表达"有几条"，而"有几条"恰是列表类输出
    //  第一有用的信息。数组长度现直接作为 count。）
    let count = match &value {
        serde_json::Value::Object(obj) => ["count", "total", "total_count", "size"]
            .iter()
            .find_map(|k| obj.get(*k).and_then(|v| v.as_u64()))
            .map(|c| c as usize),
        serde_json::Value::Array(items) => Some(items.len()),
        _ => None,
    };
    if let Some(c) = count {
        parts.push(format!("count={c}"));
    }

    // 条目名列表：列表类输出（`vdfs_list` / glob / search 的结果）最需要的是
    // "这里面都有什么"。≥2 条时列前 N 个定位名并标注余量；恰好 1 条时退化为
    // `first=<name>`（少一层方括号歧义，也让摘要更短）。
    if let Some(items) = list_items(&value) {
        // 条目名预算：按摘要字符上限（token × 2）扣除固定前后缀后推导，
        // 保证 names 列表收尾的 `]` 不会被预算截掉。
        let names_budget = (SKELETON_DIGEST_TOKEN_CAP * DIGEST_CHAR_PROXY_PER_TOKEN)
            .saturating_sub(DIGEST_AFFIX_CHAR_RESERVE);
        let names = entry_names(items, names_budget);
        if names.len() > 1 {
            let more = items.len().saturating_sub(names.len());
            let suffix = if more > 0 {
                format!(",…+{more}")
            } else {
                String::new()
            };
            parts.push(format!("names=[{}{suffix}]", names.join(",")));
        } else if let Some(only) = names.first() {
            parts.push(format!("first={only}"));
        }
    }

    // 兜底结构感：无条目名可比时给出顶层键名（对象）。
    if parts.is_empty() {
        parts.extend(top_level_keys(&value));
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
/// 骨架化占位符**保留一行语义摘要**：失败结果保留错误类型与首行原因（失败是
/// 信息性的，错误详情决定模型换路径重试还是放弃）；成功结果保留语义摘要
/// ——JSON 列表类输出给 `count=N` + 前若干条条目名（只报首条目会让模型无法
/// 一次建模，只能逐轮重跑工具），文本输出给首个有内容的行。摘要判定结构化
/// 优先（`meta.success` / `meta.failure_kind`），文本启发式兜底。
///
/// 骨架化占位符同时**保留定位锚点**：参数骨架化保留首个定位参数
/// （path/command/url/pattern/query/name），结果摘要前缀配对调用的锚点——
/// 模型凭锚点把历史调用与结果对上号，避免"读过某文件第 N 行却不知是哪个
/// 文件"的链路断裂。
///
/// 结果占位符末尾再附**取回指引**（`Re-run <tool> to get the full output.`）：
/// 摘要永远是摘要，模型需要判断"够不够"。把"重跑一次就能拿到全文"写进占位符，
/// 核对成本从"猜"变成一次可预期的工具调用——核对成本过高时，模型会转而用
/// 自信语气包装未验证结论（真实会话实证：agent 自述"我中途放弃了核对"）。
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
                // 配对的 ToolCall：既给锚点，也给"重跑哪个工具"的工具名。
                let parent = msg
                    .parent_id
                    .as_deref()
                    .and_then(|pid| messages.iter().find(|m| m.id == *pid));
                // 结果摘要前拼接配对 ToolCall 的锚点（path/command 等），
                // 无主摘要（"读过某文件第 585 行"却不知是哪个文件）是链路断裂的根源
                let anchor = parent
                    .and_then(|tc| tc.content.as_ref().map(|c| c.to_text()))
                    .and_then(|args| anchor_of_args(&args))
                    .map(|a| format!(" ({a})"))
                    .unwrap_or_default();
                // 取回指引：骨架化占位符只留摘要，模型若无法判断摘要够不够，
                // 唯一的低成本核实动作就是重跑该工具。把这句话写进占位符，
                // 把"要不要赌一把摘要"变成一次可预期的工具调用——
                // 核对成本过高时，模型会转而用自信语气包装未验证结论（真实会话实证）。
                let retry_hint = parent
                    .and_then(|tc| tc.name.as_deref())
                    .map(|n| format!(" Re-run {n} to get the full output."))
                    .unwrap_or_default();
                let label = if msg.role == Some(MessageRole::Tool) {
                    // 失败结果保留错误摘要、成功结果保留语义摘要：
                    // 失败是信息性的（错误详情决定下一步动作），成功摘要避免
                    // "隔轮即忘"迫使模型整文件重读。摘要远短于原文，仍净省 token。
                    if is_failed_result(msg, &preview) {
                        let detail = error_digest(msg, &preview);
                        format!(
                            "[System Info: Tool result failed{anchor}: {}. Full output skeletonized.{retry_hint}]",
                            detail
                        )
                    } else {
                        let digest = success_digest(&preview);
                        if digest.is_empty() {
                            format!(
                                "[System Info: Tool result received successfully{anchor}. Output skeletonized.{retry_hint}]"
                            )
                        } else {
                            format!(
                                "[System Info: Tool result received successfully{anchor}. Output skeletonized. Summary: {digest}.{retry_hint}]"
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
#[path = "context_window.test.rs"]
mod tests;
