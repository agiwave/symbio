//! Tool Call Accumulator
//!
//! 用于正确累积流式 tool_calls

use serde_json::Value;
use std::collections::HashMap;
use tracing::warn;

use super::message_builder::short_id;

#[derive(Debug, Default, Clone)]
struct AccumulatedToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// Tool call information
#[derive(Debug, Clone)]
pub struct ToolCallInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Value,
}

/// Accumulates incremental tool call deltas.
///
/// LLM APIs stream tool calls incrementally. This struct handles the accumulation
/// so plugin authors don't need to manage index-based HashMaps.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: HashMap<usize, AccumulatedToolCall>,
}

impl ToolCallAccumulator {
    /// Process a tool call delta from the API and return (tool_call_id, accumulated_args, name).
    pub fn process_delta(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        args_delta: Option<&str>,
    ) -> (String, String, Option<String>) {
        let entry = self.calls.entry(index).or_default();

        // 仅接受非空 id/name：
        // 部分 OpenAI 兼容网关（如实测 apinex qwen-3.8-max）只在首个增量携带合法 id，
        // 后续增量重复发送 `id:""`。若用空值覆盖，会把首个增量的合法 id 冲掉，
        // 最终得到 Some("") → 工具调用被误判为 id 缺失而被跳过（历史 Bug）。
        if let Some(id) = id.filter(|s| !s.trim().is_empty()) {
            entry.id = Some(id.to_string());
        }
        if let Some(name) = name.filter(|s| !s.trim().is_empty()) {
            entry.name = Some(name.to_string());
        }

        // 供应商始终未返回 id（缺失或全为空串）时，主动分配一个短 GUID 作为工具调用 id。
        // 该 id 在首个增量即确定并写入 entry，保证流式广播、落库（build_assistant_messages）
        // 与执行（process_tool_calls_async）三处使用同一 id。
        if entry.id.is_none() {
            entry.id = Some(short_id());
        }

        if let Some(delta) = args_delta {
            entry.arguments.push_str(delta);
        }

        (
            entry.id.clone().unwrap_or_default(),
            entry.arguments.clone(),
            entry.name.clone(),
        )
    }

    /// Get the list of completed tool calls.
    ///
    /// 保证返回的每个 ToolCallInfo.id 均为非空：正常情况下 process_delta 已在首个增量
    /// 确定 id，此处为幂等兜底——重复调用返回相同 id，**绝不**重新随机生成
    /// （chat_loop 与 into_messages 会各取一次，两次结果不一致会使工具结果子节点变孤儿）。
    pub fn get_completed(&mut self) -> Vec<ToolCallInfo> {
        self.calls
            .values_mut()
            .map(|call| {
                if call.id.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true) {
                    call.id = Some(short_id());
                }
                let args: Value = match serde_json::from_str(&call.arguments) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(
                            error = %e,
                            raw_arguments = %call.arguments,
                            "tool call parse error"
                        );
                        serde_json::json!({})
                    }
                };
                ToolCallInfo {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: args,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归（apinex qwen-3.8-max 网关真实行为）：
    /// 首个增量携带合法 id，后续增量重复发送 `id:""`。
    /// 空串不得覆盖合法 id——否则最终得到 Some("")，工具调用被误判为
    /// id 缺失而跳过，落库的 ToolCall 节点 id 为空串且无结果子节点。
    #[test]
    fn empty_id_delta_does_not_overwrite_real_id() {
        let mut acc = ToolCallAccumulator::default();
        let (id1, _, _) =
            acc.process_delta(0, Some("call_8f3a59f5f8e14258a427e432"), Some("get_weather"), Some(""));
        assert_eq!(id1, "call_8f3a59f5f8e14258a427e432");

        // 后续增量：id:""（该网关的真实行为）
        let (id2, args, _) =
            acc.process_delta(0, Some(""), None, Some("{\"city\": \"Paris\"}"));
        assert_eq!(id2, "call_8f3a59f5f8e14258a427e432");
        assert_eq!(args, "{\"city\": \"Paris\"}");

        let done = acc.get_completed();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].id.as_deref(), Some("call_8f3a59f5f8e14258a427e432"));
        assert_eq!(done[0].name.as_deref(), Some("get_weather"));
    }

    /// 需求 1：供应商始终未返回 id 时，必须主动分配短 GUID 作为工具调用 id。
    /// 流式返回值与 get_completed 结果必须一致，且重复取值幂等
    /// （chat_loop 与 into_messages 各取一次，两次结果不一致会使结果子节点变孤儿）。
    #[test]
    fn missing_id_gets_stable_generated_guid() {
        let mut acc = ToolCallAccumulator::default();
        let (stream_id, _, _) =
            acc.process_delta(0, None, Some("dir_list"), Some("{\"path\": \".\"}"));
        assert!(!stream_id.is_empty(), "流式期间即应有非空 id");
        assert_ne!(stream_id, "tc-0", "不得再使用 index 占位符");

        let done1 = acc.get_completed();
        let done2 = acc.get_completed();
        assert_eq!(done1[0].id.as_deref(), Some(stream_id.as_str()));
        assert_eq!(
            done2[0].id.as_deref(),
            Some(stream_id.as_str()),
            "重复调用 get_completed 必须返回同一 id"
        );
    }

    /// 纯空白 id 视为"不合法"，与缺失同等对待。
    #[test]
    fn whitespace_id_treated_as_missing() {
        let mut acc = ToolCallAccumulator::default();
        let (id, _, _) = acc.process_delta(0, Some("   "), None, Some("{}"));
        assert!(!id.trim().is_empty());
    }

    /// 空串 name 不得覆盖首个增量的合法 name（与 id 同理）。
    #[test]
    fn empty_name_delta_does_not_overwrite_real_name() {
        let mut acc = ToolCallAccumulator::default();
        acc.process_delta(0, Some("call_x"), Some("cmd.exe"), Some(""));
        acc.process_delta(0, Some(""), Some(""), Some("{}"));

        let done = acc.get_completed();
        assert_eq!(done[0].id.as_deref(), Some("call_x"));
        assert_eq!(done[0].name.as_deref(), Some("cmd.exe"));
    }

    /// 多个并行工具调用（不同 index）互不干扰，各自持有独立 id。
    #[test]
    fn parallel_tool_calls_keep_separate_ids() {
        let mut acc = ToolCallAccumulator::default();
        acc.process_delta(0, Some("call_a"), Some("f1"), Some("{}"));
        acc.process_delta(1, Some("call_b"), Some("f2"), Some("{}"));

        let mut done = acc.get_completed();
        done.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(done[0].id.as_deref(), Some("call_a"));
        assert_eq!(done[1].id.as_deref(), Some("call_b"));
    }
}
