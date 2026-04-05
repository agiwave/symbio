//! Tool Call Accumulator
//!
//! 从 agierFlow 项目引入，用于正确累积流式 tool_calls

use serde_json::Value;
use std::collections::HashMap;

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
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are no accumulated tool calls.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// Process a tool call delta from the API.
    pub fn process_delta(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        args_delta: Option<&str>,
    ) {
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
    }

    /// Get the list of completed tool calls.
    pub fn get_completed(&self) -> Vec<(String, String, Value)> {
        self.calls
            .values()
            .filter_map(|call| {
                let id = call.id.as_ref()?;
                let name = call.name.as_ref()?;
                let args: Value =
                    serde_json::from_str(&call.arguments).unwrap_or(serde_json::json!({}));
                Some((id.clone(), name.clone(), args))
            })
            .collect()
    }

    /// Clear all accumulated data.
    pub fn clear(&mut self) {
        self.calls.clear();
    }
}
