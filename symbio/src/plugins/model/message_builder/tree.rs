//! **树形路径**（[T-209](../../../../../docs/009-task/209-消息树换算下沉批.md "T-209-消息树换算下沉批") 三段之一）：
//! 落库树在请求视图里的**位置与配对**——谁是谁的孩子、wire id 配给谁、哪些节点已被
//! 聚合吃掉、哪几条思考还留着。
//!
//! 这一段与 [`super::shape`]（形状换算）、[`super::degrade`]（缺东西时补什么）的分界是
//! 一句话：**这里只回答「在哪、归谁」，不回答「长成什么」**。换算出错可以拆成两类，
//! 一类是路径找错（把 A 的结果配给 B 的调用），一类是形状拼错（配对了但发成错角色）——
//! 分家之后「哪一段错了谁会红」有答案：找错红在这里，拼错红在 shape。
//!
//! 借来的生命周期 `'a` 指向传入的节点切片：这一段**不 clone 节点**，只存引用与 id。

use std::collections::{HashMap, HashSet};

use crate::symbio_core::chat_node::{ChatNode, NodeRole};

/// LLM 请求包中保留的最近思考条数。
///
/// 思考链的价值随距离衰减：模型只需最近几条推理来"接上思路"，
/// 全量回传历史思考既浪费 token，也可能诱导模型复述旧结论。
/// 存储层 Reasoning 节点完整保留，此处只裁剪请求视图
/// （Anthropic 协议本就不回传历史思考——缺官方签名，此裁剪对它无副作用）。
pub(super) const RETAINED_RECENT_REASONING: usize = 2;

/// 一棵落库树在请求视图里的位置索引。
pub(super) struct RequestTree<'a> {
    by_id: HashMap<&'a str, &'a ChatNode>,
    children: HashMap<&'a str, Vec<&'a ChatNode>>,
    /// 节点 id → wire id（provider 原始 tool_call_id）
    wire_ids: HashMap<&'a str, String>,
    /// 已被某个根级 Turn 聚合吃掉的**子孙** id（不含 Turn 自身）
    consumed: HashSet<&'a str>,
    /// 允许回传思考的那几个根级 Turn 的 id
    retained_reasoning: HashSet<&'a str>,
}

impl<'a> RequestTree<'a> {
    /// 扫一遍节点列表建索引（三次遍历：孩子表 / consumed / 思考保留集）
    pub(super) fn of(messages: &'a [ChatNode]) -> Self {
        let by_id: HashMap<&str, &ChatNode> = messages.iter().map(|m| (m.id.as_str(), m)).collect();
        let mut children: HashMap<&str, Vec<&ChatNode>> = HashMap::new();
        for m in messages {
            if let Some(pid) = m.parent_id.as_deref() {
                children.entry(pid).or_default().push(m);
            }
        }

        // 节点 id → wire id（provider 原始 tool_call_id）映射。
        //
        // ToolCall 节点 id 与 wire id 分离后（节点 id 会话内唯一，wire id 可能被
        // 网关跨轮复用），请求包必须回传 provider 认识的 wire id——部分协议要求
        // `tool_call_id` 与其签发值逐字一致（如 OpenAI Responses 的 call_id 链）。
        // 历史数据无 `tool_call_id` 字段 → 回退节点 id（旧数据里两者本就是同一个值）。
        let wire_id_of = |m: &ChatNode| -> String {
            m.tool_call_id
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| m.id.clone())
        };
        let wire_ids: HashMap<&str, String> = messages
            .iter()
            .filter(|m| m.is_tool_call())
            .map(|m| (m.id.as_str(), wire_id_of(m)))
            .collect();

        // 已聚合消费的节点 id 集合：每个根级 Turn 在聚合时会把自身及全部子孙合并进
        // 一条 assistant 消息。此处仅把「子孙」id 收集进来（不含 Turn 自身），这样：
        //  - 若同一节点在列表中以「裸根」形式（parent_id 为 None 或指向缺失节点）重复出现，
        //    会被跳过，避免 reasoning / tool_call 参数 / tool_result 在 LLM 请求中重复；
        //  - Turn 自身不会被误标记，保证它能被主循环正常聚合（把 Turn 自身 id 也收进
        //    consumed 会让根级 Turn 被跳过、永不聚合）。
        let mut consumed: HashSet<&str> = HashSet::new();
        for m in messages {
            if is_root(m, &by_id) && m.is_run() {
                collect_descendants(&children, &mut consumed, m.id.as_str());
            }
        }

        // ── 请求视图思考裁剪 ─────────────────────────────────────────────────
        // 按列表序（时间序）收集携带 Reasoning 子节点的根级 Turn，
        // 仅最近 RETAINED_RECENT_REASONING 条允许回传 reasoning_content：
        // 完全清空思考会让模型"失忆"后重复思考；全量回传历史思考既浪费
        // token，也可能诱导模型复述旧结论。只裁请求视图，存储层完整保留。
        let turns_with_reasoning: Vec<&str> = messages
            .iter()
            .filter(|m| {
                is_root(m, &by_id)
                    && m.is_run()
                    && children
                        .get(m.id.as_str())
                        .map(|kids| kids.iter().any(|c| c.is_reasoning()))
                        .unwrap_or(false)
            })
            .map(|m| m.id.as_str())
            .collect();
        let retained_reasoning: HashSet<&str> = turns_with_reasoning
            .iter()
            .rev()
            .take(RETAINED_RECENT_REASONING)
            .copied()
            .collect();

        Self {
            by_id,
            children,
            wire_ids,
            consumed,
            retained_reasoning,
        }
    }

    /// 这条节点是不是**请求视图的根**：`parent_id` 缺失，或指向一个不在本批里的节点
    /// （后者就是注释里说的「裸根」形态）。
    pub(super) fn is_root(&self, m: &ChatNode) -> bool {
        is_root(m, &self.by_id)
    }

    /// 已被某个根级 Turn 聚合吃掉（它的内容会出现在那条 assistant 里，不该再单独发出）
    pub(super) fn is_consumed(&self, id: &str) -> bool {
        self.consumed.contains(id)
    }

    /// 某个根级 Turn 的直接子节点（**保持落库序**——聚合取值按这个顺序，
    /// 文本取后写的、思考取后写的，顺序变了形状就变）
    pub(super) fn kids(&self, id: &str) -> Option<&[&'a ChatNode]> {
        self.children.get(id).map(|v| v.as_slice())
    }

    /// 这个 ToolCall 该用哪个 wire id 回传给 provider（历史数据回退节点 id）
    pub(super) fn wire_id(&self, id: &str) -> Option<String> {
        self.wire_ids.get(id).cloned()
    }

    /// 本轮的思考还留着吗（只有最近 N 条 Turn 的思考进请求视图）
    pub(super) fn retains_reasoning(&self, turn_id: &str) -> bool {
        self.retained_reasoning.contains(turn_id)
    }

    /// 在 ToolCall 下查找响应结果节点：
    /// 1. 直接的 `Text`(role=Tool) 子节点（非空内容优先）
    /// 2. 或被 `Turn`(role=Tool) 包装的 `Text` 子节点（组合节点可选时的兼容形式，
    ///    子 agent 会话即以此种方式返回完整会话）
    /// 3. 兜底：任意直接的 `Text`(role=Tool) 子节点
    pub(super) fn find_tool_result(&self, tool_call_id: &str) -> Option<&'a ChatNode> {
        let kids = self.children.get(tool_call_id)?;
        // 1. 优先直接 Text(Tool) 且内容非空
        if let Some(direct) = kids.iter().find(|c| {
            c.node_role() == Some(NodeRole::Tool)
                && c.is_text()
                && c.content.as_ref().map(|x| !x.is_empty()).unwrap_or(false)
        }) {
            return Some(direct);
        }
        // 2. 被 Turn(Tool) 包装（子 agent 会话）
        if let Some(turn) = kids
            .iter()
            .find(|c| c.is_run() && c.node_role() == Some(NodeRole::Tool))
        {
            if let Some(tkids) = self.children.get(turn.id.as_str()) {
                return tkids.iter().find(|c| c.is_text()).copied();
            }
        }
        // 3. 兜底：任意直接 Text(Tool)
        kids.iter()
            .find(|c| c.node_role() == Some(NodeRole::Tool) && c.is_text())
            .copied()
    }
}

fn is_root(m: &ChatNode, by_id: &HashMap<&str, &ChatNode>) -> bool {
    m.parent_id
        .as_deref()
        .map(|p| by_id.contains_key(p))
        .unwrap_or(true)
}

/// 递归收集某 Turn 的全部子孙 id（不含 Turn 自身）到 `acc`
fn collect_descendants<'a>(
    children: &HashMap<&'a str, Vec<&'a ChatNode>>,
    acc: &mut HashSet<&'a str>,
    id: &'a str,
) {
    // 从 Turn 的直接子节点开始递归收集全部子孙 id（不含 Turn 自身）
    let mut stack: Vec<&'a str> = Vec::new();
    if let Some(kids) = children.get(id) {
        for kid in kids {
            stack.push(kid.id.as_str());
        }
    }
    while let Some(cur) = stack.pop() {
        acc.insert(cur);
        if let Some(kids) = children.get(cur) {
            for kid in kids {
                stack.push(kid.id.as_str());
            }
        }
    }
}
