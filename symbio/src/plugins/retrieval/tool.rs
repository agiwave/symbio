//! `memory_recall` —— 检索者的 **LLM 消费口**（B5：把 B1–B4 的地基接进聊天）。
//!
//! ## 它补的是哪一环
//!
//! B1–B4 交付了「事实 → 投影 → Actor 行」的**生产侧**，但消费侧只有 `retrieval/list`
//! 这条**诊断路由**——聊天里的模型看不到也调不到，能力"可查而不可用"（BR2 风险的
//! 实形态）。本工具是第一个进入 LLM 工具清单的 v2 桥接能力：模型在会话里按需召回
//! **跨会话**钉住的记忆（`MEMORY.md`），回答"之前记过什么 / 上次说过"。
//!
//! ## 边界（承 B4 的三条 + 工具分级规则）
//!
//! | 边界 | 为什么 |
//! |---|---|
//! | **轻查询**：单发、毫秒级、只读 | 主会话只允许这一类工具；多轮 / 写盘 / 重度思考一律进 worker 会话（`docs/plan/06` §10 的 R1） |
//! | **正文来自 `MEMORY.md`**，事实只作投影输入 | 事实是索引不是副本（B1 取舍）；本工具是把正文交给模型的唯一出口，故必须只读 |
//! | **可选**：`enabled=false` ⇒ `traverse` 不注册 | 模型的工具清单里直接没有它（J2 平凡值可区分，e2e T23 断言） |
//! | **降级可区分**：`memory.recall` 未登记 ⇒ `degraded: true` | 与 `retrieval/list` 同一判据；能力未接入 ≠ 出错 |
//!
//! ## 输出契约
//!
//! ```json
//! { "entries": [{"session": "…", "line": 0, "text": "…"}],
//!   "returned": 3, "matched": 3, "limit": 20,
//!   "trivial": false, "degraded": false }
//! ```
//!
//! - `entries` 按**会话 id 字典序、行序**排列（确定性，A4 双跑一致）；
//! - `trivial` = 全盘无 `memory.*` ⇒ 本次召回无长期记忆成分（与投影同判据）；
//! - `degraded` = 投影未登记 ⇒ 只回内容、不回视图判据（两态可区分）。

use super::derive::{read_memory_lines, session_ids};
use super::plugin::{RetrievalPlugin, PROJECTION_RECALL};
use crate::symbio_core::{
    projection_has, projection_run, Capability, CapabilityCategory, CapabilityMeta, ExecEnv,
    PluginError, PluginInvokeRequest, ProjectionInput,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// 缺省返回条数（不给 `limit` 时）。
const DEFAULT_LIMIT: usize = 20;
/// `limit` 上限——工具结果要进模型上下文，必须有界。
const MAX_LIMIT: usize = 100;

/// 跨会话记忆召回工具（只读轻查询）。
pub struct MemoryRecallTool {
    plugin: Arc<RetrievalPlugin>,
}

impl MemoryRecallTool {
    pub(super) fn new(plugin: Arc<RetrievalPlugin>) -> Self {
        Self { plugin }
    }

    /// 召回核心（同步纯逻辑，单测直接打这里；`execute` 只做参数解析）。
    ///
    /// 与 `retrieval/list` **共用同一条链**：派生事实（B1）→ 跑 `memory.recall`
    /// 投影（B2）取平凡 / 降级两态；正文则从 `MEMORY.md` 本身读（事实不存正文）。
    pub(super) fn recall(&self, query: Option<&str>, limit: usize) -> Value {
        let limit = limit.clamp(1, MAX_LIMIT);
        let needle = query.map(str::to_lowercase);

        // ① 内容：跨会话读 MEMORY.md —— 会话 id 排序后逐行扫（确定性 A4）。
        let root = self.plugin.session_root();
        let mut ids = session_ids(&root);
        ids.sort();
        let mut entries: Vec<Value> = Vec::new();
        let mut matched = 0usize;
        let mut has_any_memory = false;
        for id in &ids {
            let lines = read_memory_lines(&root, id);
            if !lines.is_empty() {
                has_any_memory = true;
            }
            for (i, line) in lines.iter().enumerate() {
                let hit = needle
                    .as_deref()
                    .map(|q| line.to_lowercase().contains(q))
                    .unwrap_or(true);
                if !hit {
                    continue;
                }
                matched += 1;
                if entries.len() < limit {
                    entries.push(json!({ "session": id, "line": i, "text": line }));
                }
            }
        }

        // ② 判据：跑投影取平凡值（与 retrieval/list 同判据）；
        //    未登记 ⇒ degraded（能力未接入，不是错误）。
        let facts = self.plugin.derive_all();
        let (trivial, degraded) = if projection_has(PROJECTION_RECALL) {
            let input = ProjectionInput::new(&facts, 0);
            match projection_run(PROJECTION_RECALL, &input) {
                Ok(view) => (view.trivial, false),
                // 登记了却跑不出来 —— 如实报错，不假装平凡
                Err(e) => {
                    return json!({
                        "error": format!("投影运行失败: {e}"),
                        "entries": [], "returned": 0, "matched": 0,
                        "limit": limit, "trivial": true, "degraded": true,
                    })
                }
            }
        } else {
            (!has_any_memory, true)
        };

        json!({
            "entries": entries,
            "returned": entries.len(),
            "matched": matched,
            "limit": limit,
            "trivial": trivial,
            "degraded": degraded,
        })
    }
}

#[async_trait]
impl Capability for MemoryRecallTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "memory_recall".to_string(),
            description: "跨会话召回长期记忆：列出各会话 MEMORY.md 中钉住的结论与约定。\
                 当用户问『之前记过什么 / 上次说过 / 我们约定过什么』时调用；\
                 可给 query 关键词过滤。只读、单发、毫秒级。"
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "关键词（不区分大小写的子串过滤；省略 = 返回全部）"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回条数（缺省 20，上限 100）"
                    }
                },
                "required": []
            }),
            category: Some(CapabilityCategory::Other),
            examples: Some(vec!["query='部署'".to_string()]),
            ..Default::default()
        }
    }

    async fn execute(
        &self,
        args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_LIMIT);
        Ok(self.recall(query, limit))
    }
}
