//! 工具调用增量累积 —— **流式状态机**（只有本插件驱动它）。
//!
//! ## 为什么住在这里，而不是 `symbio_core`
//!
//! 唯一的驱动方是本插件的 `stream.rs`：它按协议增量逐块喂 [`process_delta`]。
//! session 只读**结果**（`TurnOutput::tool_calls`），从不驱动累积过程——即
//! 「怎么把分片攒成一次调用」是 model 的实现细节，不是两侧共用的契约。
//!
//! 历史上它住在 `symbio_core::llm::turn`，理由是「`TurnOutput::into_messages`
//! 依赖它」。那是**循环论证**（ADR-023 开头点名的那种推理）：`TurnOutput` 之所以
//! 依赖它，只因为它把累积器当字段带着；把字段换成**结果形态**
//! （`tool_calls: Vec<TurnToolCallInfo>`）之后，这条依赖自己就消失了。
//!
//! ## 契约 / 实现分工
//!
//! | 符号 | 住哪 | 理由 |
//! |---|---|---|
//! | [`TurnToolCallInfo`] | `symbio_core` | `llm_build_assistant_messages` 的**形参类型**，两侧共用 |
//! | `llm_short_id` | `symbio_core` | 消息构造家族共用的 id 原语 |
//! | [`TurnToolCallAccumulator`] | 本模块 | 只有 `stream.rs` 驱动 |
//!
//! ## 三条不变量
//!
//! 1. **节点 id 在首个增量确定，此后只读**：流式帧、落库（`llm_build_assistant_messages`）、
//!    工具执行（`process_tool_calls_async`）三处用的是同一个 id。见
//!    [`process_delta`] 返回的 `node_id` 与 [`finish`] 产出的 `TurnToolCallInfo::id`。
//! 2. **空串 / 纯空白的 `id` / `name` 增量不覆盖已定值**：实测 apinex qwen-3.8-max
//!    网关只在首个增量携带合法 id，后续增量重复发 `id:""`；若用空值覆盖，最终得到
//!    `Some("")`，工具调用被误判为 id 缺失而跳过。
//! 3. **参数 JSON 非空且解析失败时保留残破原文**（`parse_error`），绝不伪装成空参数：
//!    静默 `{}` 会让工具报「缺少必填参数」，模型看不懂原因便原样重试，卡死在思考循环里。
//!
//! [`process_delta`]: TurnToolCallAccumulator::process_delta
//! [`finish`]: TurnToolCallAccumulator::finish

use crate::plugin_warn;
use crate::symbio_core::{llm_short_id, TurnToolCallInfo};
use serde_json::Value;
use std::collections::HashMap;

/// 单次工具调用的累积态（**过程形态**，只在本模块内可见）。
#[derive(Debug, Default)]
struct AccumulatedToolCall {
    /// provider 原始 `tool_call_id`（wire id）。供应商未返回（或全空白）时为 `None`，
    /// 此时请求构建回退用节点 id。
    id: Option<String>,
    /// 消息节点 id：**首个增量到达时分配**，会话内唯一。
    ///
    /// 许多 OpenAI 兼容网关**跨轮复用** `call_0` / `call_xxx` 这类短 id；若直接把
    /// wire id 当节点 id，第二轮的同 id 工具调用会更新到第一轮的老节点（后端
    /// `Vec` 存储不撞、前端按 id 的 map 撞——"后端正常、前端显示混乱"的根源）。
    node_id: String,
    name: Option<String>,
    arguments: String,
}

/// 把增量工具调用分片累积成完整的调用（**过程形态**）。
///
/// LLM API 以增量方式流式输出工具调用。本结构承担这段累积，使上层不必自己维护
/// 一个按 `index` 索引的 map。
///
/// **生命周期**：`process_delta`（多次）→ `finish`（恰好一次，按值消费）。
/// `finish` 取 `self` 而非 `&mut self`，是为了让「收口只有一次」成为类型层面的事实
/// ——旧版 `get_completed(&mut self)` 可以被反复调用，于是不得不写一条「重复调用返回
/// 相同 id」的兜底；那条兜底实际不可达（见 [`TurnToolCallAccumulator::finish`] 的注释）。
#[derive(Debug, Default)]
pub(crate) struct TurnToolCallAccumulator {
    calls: HashMap<usize, AccumulatedToolCall>,
}

impl TurnToolCallAccumulator {
    /// 处理一片工具调用增量。
    ///
    /// 返回 `(node_id, wire_id, accumulated_args, name, snapshot_required)`：
    /// - `node_id`：消息节点 id（**首个增量分配，会话内唯一**）——流式帧与落库都用它；
    /// - `wire_id`：provider 的原始 tool_call_id（未提供时等于 `node_id`）——
    ///   仅在构建 LLM 请求包时使用；
    /// - `accumulated_args`：**迄今累积**的参数 JSON；
    /// - `name`：工具名（空串增量不覆盖已定名）；
    /// - `snapshot_required`：本次增量是否改动了节点的**身份字段**（新建节点 / 首次定名）。
    ///   是 → 调用方必须发**完整快照**（`Upsert`，身份与内容一次给全）；
    ///   否 → 只是正文增长，调用方发**窄追加**（`Append`，O(delta)）。
    ///
    /// 这条划分与 Text / Reasoning 子节点**同构**：帧面只有两种语义——「整条替换」与
    /// 「尾部追加」——由覆盖方式决定，而不是由接收端去猜。
    pub(crate) fn process_delta(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        args_delta: Option<&str>,
    ) -> (String, String, String, Option<String>, bool) {
        let entry = self.calls.entry(index).or_default();

        // 节点 id 在诞生时确定并写入 entry：流式广播、落库（llm_build_assistant_messages）、
        // 执行（process_tool_calls_async）三处使用同一节点 id。
        let is_new_node = entry.node_id.is_empty();
        if is_new_node {
            entry.node_id = llm_short_id();
        }
        // 新建节点必须发快照（接收端尚无此节点）；首次定名亦然——身份字段
        // （name / tool_call_id / 父子）只随快照下发，之后的增量只带参数片段。
        let mut snapshot_required = is_new_node;

        // 仅接受非空 id/name：
        // 部分 OpenAI 兼容网关（如实测 apinex qwen-3.8-max）只在首个增量携带合法 id，
        // 后续增量重复发送 `id:""`。若用空值覆盖，会把首个增量的合法 id 冲掉，
        // 最终得到 Some("") → 工具调用被误判为 id 缺失而被跳过。
        if let Some(id) = id.filter(|s| !s.trim().is_empty()) {
            entry.id = Some(id.to_string());
        }
        if let Some(name) = name.filter(|s| !s.trim().is_empty()) {
            if entry.name.is_none() {
                snapshot_required = true;
            }
            entry.name = Some(name.to_string());
        }

        let node_id = entry.node_id.clone();
        // 供应商始终未返回 id（缺失或全为空串）时，wire id 回退为节点 id——
        // 请求包里的 tool_call 与 tool 结果引用同一节点 id，依然自洽。
        let wire_id = entry
            .id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| node_id.clone());

        if let Some(delta) = args_delta {
            entry.arguments.push_str(delta);
        }

        (
            node_id,
            wire_id,
            entry.arguments.clone(),
            entry.name.clone(),
            snapshot_required,
        )
    }

    /// 收口：把累积结果转成**结果形态**的 [`TurnToolCallInfo`]，按值消费。
    ///
    /// **按 wire `index` 升序返回**：`HashMap` 的迭代顺序随进程随机，不排序则同一批
    /// 工具调用的**执行顺序**会在两次运行间漂移（`process_tool_calls_async` 按此顺序
    /// 逐个执行并广播，前端帧序随之漂移）。`index` 就是模型给出的调用序号，排序即还原
    /// 模型的本意顺序，代价是 n≤个位数的排序。
    ///
    /// **节点 id 不在此处生成**：`process_delta` 已在首个增量确定并写回 entry，而 entry
    /// 只可能由 `process_delta` 创建——故「entry 的 `node_id` 为空」不可达。旧版在这里
    /// 写了一条 `if call.node_id.is_empty() { … }` 的「幂等兜底」，正是那条不可达分支。
    ///
    /// 参数解析区分三种情况，**绝不**把解析失败伪装成空参数：
    /// - 空串 / 纯空白：无参工具的合法形态（`from_str("")` 必失败），视为 `{}`；
    /// - 合法 JSON：照常使用；
    /// - 非空且非法：参数已残破（典型为 `max_tokens` 截断），保留原文交给
    ///   `parse_error`，由执行侧拒绝执行并回报明确错误。
    pub(crate) fn finish(self) -> Vec<TurnToolCallInfo> {
        let mut entries: Vec<(usize, AccumulatedToolCall)> = self.calls.into_iter().collect();
        entries.sort_by_key(|(index, _)| *index);

        entries
            .into_iter()
            .map(|(_, call)| {
                // wire id：供应商提供了合法 id 才携带；否则 None（请求构建回退节点 id）
                let wire_id = call.id.filter(|s| !s.trim().is_empty());
                let raw = call.arguments;
                let (arguments, parse_error) = if raw.trim().is_empty() {
                    (serde_json::json!({}), None)
                } else {
                    match serde_json::from_str::<Value>(&raw) {
                        Ok(v) => (v, None),
                        Err(e) => {
                            plugin_warn!(
                                "model",
                                "[LLM] 工具调用参数 JSON 非法（拒绝执行）：error={}, raw_arguments={}",
                                e,
                                raw
                            );
                            (serde_json::json!({}), Some(raw))
                        }
                    }
                };
                TurnToolCallInfo {
                    id: Some(call.node_id),
                    wire_id,
                    name: call.name,
                    arguments,
                    parse_error,
                }
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "tool_accumulator.test.rs"]
mod tests;
