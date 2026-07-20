//! Tool Call Accumulator
//!
//! 用于正确累积流式 tool_calls

use serde_json::Value;
use std::collections::HashMap;
use tracing::warn;

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

        if let Some(id) = id {
            entry.id = Some(id.to_string());
        }

        if let Some(name) = name {
            entry.name = Some(name.to_string());
        }

        if let Some(delta) = args_delta {
            entry.arguments.push_str(delta);
        }

        // Return current ID or a placeholder based on index
        let id = entry.id.clone().unwrap_or_else(|| format!("tc-{index}"));
        (id, entry.arguments.clone(), entry.name.clone())
    }

    /// Get the list of completed tool calls.
    pub fn get_completed(&self) -> Vec<ToolCallInfo> {
        self.calls
            .values()
            .map(|call| {
                let args: Value = match serde_json::from_str(&call.arguments) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(
                            error = %e,
                            raw_arguments = %call.arguments,
                            "tool call parse error"
                        );
                        serde_json::json!({})
                    },
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
