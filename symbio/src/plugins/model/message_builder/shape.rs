//! **消息形状换算**（三段之一）：
//! 给定「树在哪」（[`super::tree::RequestTree`]）与「缺的补什么」（[`super::degrade`]），
//! 把落库节点拼成协议侧的 `NativeMessage` 形状。
//!
//! 扁平化规则（与拆分之**逐字相同**，本段只做搬家）：
//! - `Turn` → 父 `assistant` native message（聚合 Reasoning / Text / tool_calls）
//! - `ToolCall` → 从其自身 `content` 读 args 拼入父 `assistant.tool_calls`
//! - 响应结果 `Text`(`Tool`) → 独立的 `role=tool` native message（tool_call_id = ToolCall 的 wire id）
//! - `User` / `System` 等 → 原样输出
//!
//! 分型树结构（请求/响应由 NodeRole 区分，组合节点可选）：
//! - 根级交替：`User`(请求, `Text`) 与 `Turn`(响应, `Assistant` 组合) 互为兄弟
//! - `Turn` 子节点：`Reasoning` / `Text`(回复) / `ToolCall`(`Assistant` 组合)
//! - `ToolCall`：`Assistant` 组合节点，自身 `content` 携带请求参数（JSON 文本）；
//!   子节点仅剩 `Text`(响应结果, `Tool`)
//!   （响应结果也可包在 `Turn`(`Tool`) 内，视复杂度而定；组合节点可选）

use crate::plugins::model::types::*;
use crate::symbio_core::chat_node::{ChatNode, NodeRole, NodeStatus};

use super::degrade;
use super::tree::RequestTree;

/// 一个根级 Turn 聚合出的请求包片段：一条 `assistant`，加上它带出的一串 `tool` 结果。
///
/// 分成两段返回而不是直接返回 `Vec<NativeMessage>`：调用方要先把 assistant 发出去、
/// 再按落库序发结果，这个顺序是协议侧的硬要求（`tool` 角色必须排在签发它的
/// `assistant.tool_calls` 之后）。
pub(super) struct RunOutput {
    pub(super) assistant: NativeMessage,
    pub(super) tool_results: Vec<NativeMessage>,
}

/// 根级 Turn → 一条 assistant（+ 它的工具结果序列）。
///
/// 失败 Turn（`status=Failed`）在这里交棒给 [`degrade::append_interrupt_note`]：
/// 半截输出照常聚合，只在正文尾部附加中断说明段落。
pub(super) fn aggregate_run(turn: &ChatNode, tree: &RequestTree<'_>) -> RunOutput {
    let mut native: NativeMessage = turn.clone().into();
    native.role = NodeRole::Assistant;
    native.content = None;
    native.reasoning_content = None;
    native.tool_calls = None;

    let mut tool_results: Vec<NativeMessage> = Vec::new();

    if let Some(kids) = tree.kids(turn.id.as_str()) {
        for child in kids {
            if child.is_reasoning()
                        // 仅最近 N 条思考进入请求视图
                        && tree.retains_reasoning(turn.id.as_str())
            {
                native.reasoning_content = child.content.as_ref().map(|c| c.to_text());
            }
            if child.is_text() {
                native.content = child.content.clone();
            }
            if child.is_tool_call() {
                // 请求参数直接来自 ToolCall 节点自身的 content（JSON 文本）。
                // 该文本在落库时（turn.rs:645）已由 `tc.arguments.to_string()`
                // 规范化——即 serde_json 输出，故 re-parse 再 re-serialize 幂等。
                // 因此**以字符串原样透传**：下游 types.rs 的 `is_string()` 快路径
                // 会直接发出这段文本，省掉一次 Value 物化 + re-serialize（评审 §3.2）。
                // 仅当文本非法时才回退 `{}`，与原 `from_str().unwrap_or({})` 行为完全一致
                // （落库时 broken 原文会带 parse_error，正常路径不会走到这里）。
                let args_text = child
                    .content
                    .as_ref()
                    .map(|c| c.to_text())
                    .unwrap_or_default();
                let args: serde_json::Value =
                    match serde_json::from_str::<serde_json::Value>(&args_text) {
                        Ok(_) => serde_json::Value::String(args_text),
                        Err(_) => serde_json::json!({}),
                    };
                let tc = ToolCall {
                    id: Some(
                        tree.wire_id(child.id.as_str())
                            .unwrap_or_else(|| child.id.clone()),
                    ),
                    kind: Some("function".to_string()),
                    name: child.tool_name().unwrap_or_default().to_string(),
                    arguments: args,
                };
                match &mut native.tool_calls {
                    Some(calls) => calls.push(tc),
                    None => native.tool_calls = Some(vec![tc]),
                }

                // 响应结果：ToolCall 下的 Text(Tool) 直接子节点，或 Turn(Tool) 内 Text
                match tree.find_tool_result(child.id.as_str()) {
                    Some(res) => {
                        let mut tool_native: NativeMessage = res.clone().into();
                        tool_native.role = NodeRole::Tool;
                        tool_native.tool_call_id = tree.wire_id(child.id.as_str());
                        degrade::mark_failed_result(&mut tool_native, res);
                        tool_results.push(tool_native);
                    }
                    None => {
                        // ToolCall 无对应结果（执行被打断，未产生结果）：合成占位
                        // tool 结果，避免请求包出现"有 tool_call 无 tool_result"触发
                        // provider 400。必须在此处进入 tool_results（先于尾部孤儿剔除），
                        // 否则会被当作孤儿 tool 结果误删。
                        tool_results.push(degrade::placeholder_tool_result(
                            child.tool_name().unwrap_or_default(),
                            tree.wire_id(child.id.as_str()),
                        ));
                    }
                }
            }
        }
    }

    // 注：本 Turn 的全部子孙 id 已在预处理遍（consumed 预扫描）中收集，
    // 主循环开头的 `if is_consumed { continue; }` 会跳过它们，
    // 因此此处无需再单独标记，避免重复代码与潜在误标记。
    if turn.status == Some(NodeStatus::Failed) {
        degrade::append_interrupt_note(&mut native, turn);
    }

    RunOutput {
        assistant: native,
        tool_results,
    }
}

/// 根级的内容节点（`User` / `System` 等）原样换算；`Tool` 角色的那条要回填
/// 父 ToolCall 的 wire id（历史数据回退节点 id）。
pub(super) fn plain_root(node: &ChatNode, tree: &RequestTree<'_>) -> NativeMessage {
    let mut native: NativeMessage = node.clone().into();
    if native.role == NodeRole::Tool {
        native.tool_call_id = node
            .parent_id
            .as_deref()
            .and_then(|pid| tree.wire_id(pid))
            .or_else(|| node.parent_id.clone());
    }
    native
}
