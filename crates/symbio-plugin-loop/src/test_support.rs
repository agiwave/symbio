//! loop 插件测试的共用假件：内存版会话动作面 + 交付包。
//!
//! 只在本插件内使用（`#[cfg(test)]`）：E-009 连测试一起判，测试路径里也不出现
//! `symbio_plugin_session`。假件模拟的是端口
//! 的**契约**，不是会话存储实现——消息列表在内存里，事实轨由用例自己持一份
//! [`EventStore`]交进来。D-094 后端口收窄成一个 `call`，假件同款：按 op
//! 分派是模拟服务端的份内事（生产侧的那次分派已经消失）。

use crate::control_decl::{CHAT_VOCAB, MESSAGE_VOCAB};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use symbio_kernel::actors::Scope;
use symbio_kernel::event::{Entity, Event, Seq, Verb};
use symbio_kernel::plugin::PluginError;
use symbio_kernel::store::{EventStore, Store};
use symbio_plugin_api::events::EVENT_SESSION_CONSOLIDATED;
use symbio_sdk::chat::chat_node::ChatNode;
use symbio_sdk::projection::turns::count_user_messages;
use symbio_sdk::turn::dispatch::{
    run_ops, AddNodesRequest, CompressionBeginReply, CompressionFinishRequest, ConsolidateRequest,
    EditNodesRequest, RemoveNodesRequest, ReplaceAllRequest, TurnSettings, TurnSpec,
    WORKER_FORCE_PREFIX, WORKER_KEYWORDS, WORKER_MIN_CHARS,
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
                    let turns =
                        count_user_messages(&facts.range(Seq::new(0)), &CHAT_VOCAB, &MESSAGE_VOCAB);
                    facts
                        .append(
                            Event::pending(
                                format!("v2sc-test-{turns}"),
                                EVENT_SESSION_CONSOLIDATED,
                                Entity::Message,
                                Verb::Progressed,
                                0,
                                symbio_sdk::authz::PRINCIPAL_MAIN,
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
            run_ops::WORKER_PROGRESS => {
                Ok(json!(Vec::<symbio_sdk::turn::facts::WorkerProgress>::new()))
            }
            run_ops::RECALL_SNAPSHOTS => {
                Ok(json!(Vec::<symbio_sdk::turn::facts::FleetSnapshot>::new()))
            }
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
        principal: symbio_sdk::authz::PRINCIPAL_USER.to_string(),
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
        presentation_tree: symbio_sdk::turn::node_build::PresentationTree::Grouped,
        skill_compile_enabled: false,
        skill_fast_path: false,
        worker_force_prefix: WORKER_FORCE_PREFIX.to_string(),
        worker_keywords: WORKER_KEYWORDS.join(","),
        worker_min_chars: WORKER_MIN_CHARS,
        task_cap: symbio_sdk::turn::dispatch::TASK_CAP,
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

// ==================== 能力包替身：`memory.live`（DE-026 D-268）====================
//
// ## 为什么需要它
//
// 巩固读的是**能力包交的声明视图** `memory.live`（记忆一个所有者，D-267）。本插件的
// 单测二进制**不链接**那个包（E-009：插件之间连测试也不许有兄弟边），于是：
//
// - `seal` 的 `OrphanFact` 会为 `memory.consolidated` / `memory.forgotten` 报红
//   （它们的形状住本插件，归宿行住能力包）；
// - `read_shared_view_all("memory.live")` 会返回 `Unknown`，巩固整条路径不可达。
//
// 所以本二进制交一份**替身声明**：名字、形状与求值与能力包逐字同构。这与
// `loop_rows.test.rs` 的 `task_capability_fixture` 同一个先例（子集二进制只需要
// 「这一格有居民」，不需要那个 crate 的任何符号）。
//
// ## 替身不是第二份实现
//
// 折叠的**判据**住能力包（`crates/symbio-plugin-memory/src/recall/live.rs`），由它的
// 用例钉；这里只保证「形状对、读数对得上」。两处若漂移，能力包的用例不会红——
// 但 e2e 会（真实装配里跑的是能力包那份）。所以本替身的价值是「让 loop 的巩固用例
// 可跑」，不是「证明折叠对」。

/// 替身视图的求值：与能力包 `recall::live` 同形的折叠（新近在前，排除式遗忘）。
fn memory_live_fixture(events: &[Event], now: i64) -> Value {
    use symbio_kernel::event::Entity;
    let mut live: Vec<(u64, i64, String, String)> = Vec::new();
    let mut forgotten: Vec<u64> = Vec::new();
    for e in events {
        if e.ts > now || e.entity != Entity::Memory {
            continue;
        }
        match e.kind.as_str() {
            symbio_plugin_api::events::EVENT_MEMORY_ENCODED
            | symbio_plugin_api::events::EVENT_MEMORY_CONSOLIDATED => {
                if e.actor != symbio_sdk::authz::PRINCIPAL_USER {
                    continue;
                }
                live.push((
                    e.seq.map(|s| s.value()).unwrap_or(u64::MAX),
                    e.ts,
                    e.payload
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    e.payload
                        .get("tag")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ));
            }
            symbio_plugin_api::events::EVENT_MEMORY_FORGOTTEN => {
                if let Some(target) = e.produced_by {
                    forgotten.push(target);
                }
            }
            _ => {}
        }
    }
    live.retain(|(seq, ..)| !forgotten.contains(seq));
    live.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
    Value::Array(
        live.into_iter()
            .map(|(seq, ts, content, tag)| {
                serde_json::json!({ "seq": seq, "ts": ts, "content": content, "tag": tag })
            })
            .collect(),
    )
}

/// 记忆族的三张归宿行（生产上住 `symbio-plugin-memory` 的 `episodic::decl::ROWS`）。
const MEMORY_SINK_ROWS: &[symbio_plugin_api::ring::Decl] = &[
    // 三行同形：有即闭合（`owes` 与 `until` 同一个源、空键、空 eq）。
    symbio_plugin_api::ring::Decl {
        name: symbio_plugin_api::events::EVENT_MEMORY_ENCODED,
        prio: 200,
        by: None,
        tier: symbio_plugin_api::LatencyTier::Reflex,
        expand: None,
        owes: symbio_plugin_api::ring::Pattern {
            source: symbio_plugin_api::events::EVENT_MEMORY_ENCODED,
            key: &[],
            eq: &[],
        },
        until: &[symbio_plugin_api::ring::Pattern {
            source: symbio_plugin_api::events::EVENT_MEMORY_ENCODED,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
    symbio_plugin_api::ring::Decl {
        name: symbio_plugin_api::events::EVENT_MEMORY_CONSOLIDATED,
        prio: 200,
        by: None,
        tier: symbio_plugin_api::LatencyTier::Reflex,
        expand: None,
        owes: symbio_plugin_api::ring::Pattern {
            source: symbio_plugin_api::events::EVENT_MEMORY_CONSOLIDATED,
            key: &[],
            eq: &[],
        },
        until: &[symbio_plugin_api::ring::Pattern {
            source: symbio_plugin_api::events::EVENT_MEMORY_CONSOLIDATED,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
    symbio_plugin_api::ring::Decl {
        name: symbio_plugin_api::events::EVENT_MEMORY_FORGOTTEN,
        prio: 200,
        by: None,
        tier: symbio_plugin_api::LatencyTier::Reflex,
        expand: None,
        owes: symbio_plugin_api::ring::Pattern {
            source: symbio_plugin_api::events::EVENT_MEMORY_FORGOTTEN,
            key: &[],
            eq: &[],
        },
        until: &[symbio_plugin_api::ring::Pattern {
            source: symbio_plugin_api::events::EVENT_MEMORY_FORGOTTEN,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
];

fn memory_capability_fixture() -> symbio_plugin_api::PluginDeclarations<'static> {
    use symbio_plugin_api::events::{
        EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN,
    };
    const FACTS: &[symbio_plugin_api::FactDecl] = &[
        symbio_plugin_api::FactDecl {
            name: EVENT_MEMORY_ENCODED,
            port: None,
            key: &[],
            expand: &[],
            fields: &[
                "content",
                "tag",
                "generation",
                "vis_scope",
                "skill_id",
                "trigger",
            ],
        },
        symbio_plugin_api::FactDecl {
            name: EVENT_MEMORY_CONSOLIDATED,
            port: None,
            key: &[],
            expand: &[],
            fields: &[
                "content",
                "tag",
                "generation",
                "fidelity",
                "sources",
                "vis_scope",
            ],
        },
        symbio_plugin_api::FactDecl {
            name: EVENT_MEMORY_FORGOTTEN,
            port: None,
            key: &[],
            expand: &[],
            fields: &["why", "into"],
        },
    ];
    const VIEWS: &[symbio_plugin_api::ViewDecl] = &[symbio_plugin_api::ViewDecl {
        name: "memory.live",
        ver: 1,
        reads: &[
            (EVENT_MEMORY_ENCODED, "content"),
            (EVENT_MEMORY_ENCODED, "tag"),
            (EVENT_MEMORY_CONSOLIDATED, "content"),
            (EVENT_MEMORY_CONSOLIDATED, "tag"),
        ],
        evidence: &[],
        cost: 4096,
        eval: memory_live_fixture,
        agg: None,
    }];
    symbio_plugin_api::PluginDeclarations {
        events: &[],
        facts: FACTS,
        views: VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: MEMORY_SINK_ROWS,
    }
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-loop-test-harness-memory",
    memory_capability_fixture
);
