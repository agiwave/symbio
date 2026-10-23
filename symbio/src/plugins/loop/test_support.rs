//! loop 插件测试的共用假件：内存版会话动作面 + 交付包。
//!
//! 只在本插件内使用（`#[cfg(test)]`）：E-009 连测试一起判，测试路径里也不出现
//! `crate::plugins::session`。假件模拟的是端口
//! 的**契约**，不是会话存储实现——消息列表在内存里，事实轨由用例自己持一份
//! [`EventStore`]交进来。D-094 后端口收窄成一个 `call`，假件同款：按 op
//! 分派是模拟服务端的份内事（生产侧的那次分派已经消失）。

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::{
    count_user_messages, run_ops, AddNodesRequest, CompressionBeginReply, CompressionFinishRequest,
    ConsolidateRequest, EditNodesRequest, Entity, Event, EventStore, PluginError,
    RemoveNodesRequest, ReplaceAllRequest, Scope, Seq, Store, TurnSettings, TurnSpec, Verb,
    EVENT_SESSION_CONSOLIDATED, WORKER_FORCE_PREFIX, WORKER_KEYWORDS, WORKER_MIN_CHARS,
};

use super::session_port::SessionPort;
use super::turn_session::LoopSession;

/// 内存版会话动作面：消息列表 + 动作记录 + 一份共享事实轨。
pub(crate) struct FakeSessionPort {
    pub(crate) messages: Mutex<Vec<ChatNode>>,
    /// 每次 `remove_nodes` 收到的 id 列表（顺序即调用顺序）。
    pub(crate) removed: Mutex<Vec<Vec<String>>>,
    /// 每次 `consolidate` 收到的整份消息列表。
    pub(crate) consolidated: Mutex<Vec<Vec<ChatNode>>>,
    /// 事实轨（与用例共享）：`consolidate` 像生产一样往它落快照格。
    pub(crate) facts: Option<Arc<EventStore>>,
}

impl FakeSessionPort {
    pub(crate) fn new(messages: Vec<ChatNode>) -> Self {
        Self {
            messages: Mutex::new(messages),
            removed: Mutex::new(Vec::new()),
            consolidated: Mutex::new(Vec::new()),
            facts: None,
        }
    }

    pub(crate) fn with_facts(mut self, facts: Arc<EventStore>) -> Self {
        self.facts = Some(facts);
        self
    }
}

#[async_trait]
impl SessionPort for FakeSessionPort {
    async fn call(&self, op: &str, payload: Value) -> Result<Value, PluginError> {
        /// 假件侧载荷解析：失败 = 调用点拼错了形状（生产侧同口径）。
        fn parse<T: serde::de::DeserializeOwned>(
            op: &str,
            payload: Value,
        ) -> Result<T, PluginError> {
            serde_json::from_value(payload)
                .map_err(|e| PluginError::InternalError(format!("假件载荷不合法（{op}）：{e}")))
        }

        match op {
            run_ops::MESSAGES | run_ops::CONTEXT_MESSAGES => {
                Ok(json!(self.messages.lock().unwrap().clone()))
            }
            run_ops::ADD_BY_RUNNER | run_ops::ADD_HERE => {
                let AddNodesRequest { messages } = parse(op, payload)?;
                self.messages.lock().unwrap().extend(messages.clone());
                Ok(json!(messages))
            }
            run_ops::EDIT_NODES => {
                let EditNodesRequest { nodes, .. } = parse(op, payload)?;
                let mut all = self.messages.lock().unwrap();
                for node in &nodes {
                    if let Some(slot) = all.iter_mut().find(|m| m.id == node.id) {
                        *slot = node.clone();
                    }
                }
                Ok(json!(nodes))
            }
            run_ops::REMOVE_NODES => {
                let RemoveNodesRequest { ids } = parse(op, payload)?;
                self.removed.lock().unwrap().push(ids.clone());
                self.messages
                    .lock()
                    .unwrap()
                    .retain(|m| !ids.contains(&m.id));
                Ok(Value::Null)
            }
            run_ops::REPLACE_ALL => {
                let ReplaceAllRequest { nodes, .. } = parse(op, payload)?;
                *self.messages.lock().unwrap() = nodes;
                Ok(Value::Null)
            }
            run_ops::DRAIN_PARKED => Ok(Value::Null),
            run_ops::CONSOLIDATE => {
                let ConsolidateRequest { messages } = parse(op, payload)?;
                self.consolidated.lock().unwrap().push(messages.clone());
                if let Some(facts) = &self.facts {
                    let turns = count_user_messages(&facts.range(Seq::new(0)));
                    facts
                        .append(
                            Event::pending(
                                format!("v2sc-test-{turns}"),
                                EVENT_SESSION_CONSOLIDATED,
                                Entity::Message,
                                Verb::Progressed,
                                0,
                                crate::symbio_core::PRINCIPAL_MAIN,
                            )
                            .with_payload(json!({
                                "messages": messages,
                                "turns": turns,
                            })),
                        )
                        .expect("假件的快照格入格");
                }
                Ok(Value::Null)
            }
            run_ops::TAKE_SUPPLEMENTS => Ok(Value::Null),
            run_ops::WORKER_PROGRESS => Ok(json!(Vec::<crate::symbio_core::WorkerProgress>::new())),
            run_ops::RECALL_SNAPSHOTS => Ok(json!(Vec::<crate::symbio_core::FleetSnapshot>::new())),
            run_ops::ELIMINATION_REVOKED => Ok(json!(false)),
            run_ops::COMPRESSION_BEGIN => Ok(json!(CompressionBeginReply { skipped: false })),
            run_ops::COMPRESSION_FINISH => {
                let req: CompressionFinishRequest = parse(op, payload)?;
                Ok(json!(self
                    .messages
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|m| m.id == req.node_id)
                    .cloned()
                    .unwrap_or_default()))
            }
            other => Err(PluginError::InternalError(format!(
                "假件未实现的内部动作：{other}"
            ))),
        }
    }
}

/// 交付包的出厂形状（路径取临时目录，测试不读写会话存储）。
pub(crate) fn turn_spec(
    session_id: &str,
    tool_archive_dir: std::path::PathBuf,
    transcript_archive_dir: std::path::PathBuf,
    settings: TurnSettings,
) -> TurnSpec {
    TurnSpec {
        session_id: session_id.to_string(),
        tool_archive_dir,
        transcript_archive_dir,
        principal: crate::symbio_core::PRINCIPAL_USER.to_string(),
        scope: Scope::Root,
        persistent: true,
        context_window: 6,
        settings,
    }
}

/// 与 `SessionConfig::default()` 同值的配置快照。
pub(crate) fn default_settings() -> TurnSettings {
    TurnSettings {
        auto_compress: true,
        classify_enabled: true,
        compose_enabled: true,
        opening_enabled: false,
        progress_enabled: false,
        progress_interval_ms: 60_000,
        progress_min_rounds: 2,
        progress_max_per_turn: 5,
        presentation_tree: crate::symbio_core::node_build::PresentationTree::Grouped,
        skill_compile_enabled: false,
        skill_fast_path: false,
        worker_force_prefix: WORKER_FORCE_PREFIX.to_string(),
        worker_keywords: WORKER_KEYWORDS.join(","),
        worker_min_chars: WORKER_MIN_CHARS,
    }
}

/// 会话句柄：事实轨与端口共持同一份内存事实。
pub(crate) fn loop_session(
    spec: TurnSpec,
    facts: Arc<EventStore>,
    port: Arc<FakeSessionPort>,
) -> LoopSession {
    LoopSession::new(spec, Some(facts as Arc<dyn Store<Event = Event>>), port)
}
