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
use symbio_plugin_api::events::{
    EVENT_COMMITMENT_BROKEN, EVENT_COMMITMENT_OPENED, EVENT_COMMITMENT_RELEASED,
    EVENT_SESSION_CONSOLIDATED,
};
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
        phase: 8,
        anchor: 200,
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
        phase: 8,
        anchor: 200,
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
        phase: 8,
        anchor: 200,
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

// ==================== 能力包替身：`cost.ledger`（名册 #18）====================
//
// 熔断的 `spent_ms` 读的是**成本能力包交的声明视图**（折叠只有一份，住在 owner 那一侧）。
// 与上面的记忆替身同一条理由：本插件的单测二进制不链接那个包（`E-009`），于是
// `read_shared_view_all("cost.ledger")` 会返回 `Unknown`，「值里的账必须被读出来」那条
// 判据（DE-039 §6.4 的反向用例）整条不可达。
//
// 替身只保证**形状对、读数对得上**，让闸门的读路径可跑；折叠本身的数字由能力包自己的
// 用例钉（`crates/symbio-plugin-cost/src/decl.test.rs`），两处若漂移 e2e 会红。

/// 替身视图的求值：按主体累计信封 `cost_ms`（as-of 之后不算、`cost_ms == 0` 不进账）。
fn cost_ledger_fixture(events: &[Event], now: i64) -> Value {
    let mut rows: std::collections::BTreeMap<String, (u64, u64)> =
        std::collections::BTreeMap::new();
    let mut total_ms = 0u64;
    for e in events {
        if e.ts > now || e.cost_ms == 0 {
            continue;
        }
        let row = rows.entry(e.actor.clone()).or_default();
        row.0 += e.cost_ms;
        row.1 += 1;
        total_ms += e.cost_ms;
    }
    let by_principal: serde_json::Map<String, Value> = rows
        .into_iter()
        .map(|(principal, (spent_ms, entries))| {
            (
                principal.clone(),
                json!({ "principal": principal, "spent_ms": spent_ms, "entries": entries }),
            )
        })
        .collect();
    json!({ "by_principal": Value::Object(by_principal), "total_ms": total_ms })
}

fn cost_capability_fixture() -> symbio_plugin_api::PluginDeclarations<'static> {
    const VIEWS: &[symbio_plugin_api::ViewDecl] = &[symbio_plugin_api::ViewDecl {
        name: "cost.ledger",
        ver: 1,
        reads: &[],
        evidence: &[],
        cost: 4096,
        eval: cost_ledger_fixture,
        agg: None,
    }];
    symbio_plugin_api::PluginDeclarations {
        events: &[],
        facts: &[],
        views: VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: &[],
    }
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-loop-test-harness-cost",
    cost_capability_fixture
);

// ==================== 能力包替身：`reputation.ledger`（名册 #22/#28）====================
//
// 淘汰回路读的是**声誉能力包交的声明视图**（折叠只有一份，住在 owner 那一侧）。
// 与上面两个替身同一条理由：本插件的单测二进制不链接那个包（`E-009`），于是
// `read_shared_view_all("reputation.ledger")` 会返回 `Unknown`，而「分数低于阈值 ⇒ 收回
// 派活权」那四条判据整条不可达——缺席时的处置（不淘汰 + 出声）反而成了唯一可达的路径，
// 那是把兜底当成主路来测。
//
// 替身只保证**形状对、读数对得上**，让回路的读路径可跑；折叠本身的数字由能力包自己的
// 用例钉（`crates/symbio-plugin-reputation/src/decl.test.rs`），两处若漂移 e2e 会红。

/// 替身视图的求值：立约按 `from` 计 `offered`，收束经载荷 `id` 找回立约方计
/// `kept` / `broken`，`score = 守约 − 违约`（as-of 之后不算）。
fn reputation_ledger_fixture(events: &[Event], now: i64) -> Value {
    let mut offered_of: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut open: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut kept_of: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut broken_of: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for e in events {
        if e.ts > now {
            continue;
        }
        let id = e
            .payload
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if e.kind == EVENT_COMMITMENT_OPENED {
            let from = e
                .payload
                .get("from")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            *offered_of.entry(from.clone()).or_insert(0) += 1;
            open.insert(id, from);
        } else if e.kind == EVENT_COMMITMENT_RELEASED || e.kind == EVENT_COMMITMENT_BROKEN {
            let Some(from) = open.get(&id) else { continue };
            *if e.kind == EVENT_COMMITMENT_BROKEN {
                &mut broken_of
            } else {
                &mut kept_of
            }
            .entry(from.clone())
            .or_insert(0) += 1;
        }
    }
    let by_principal: serde_json::Map<String, Value> = offered_of
        .into_iter()
        .map(|(principal, offered)| {
            let kept = *kept_of.get(&principal).unwrap_or(&0);
            let broken = *broken_of.get(&principal).unwrap_or(&0);
            (
                principal.clone(),
                json!({
                    "principal": principal,
                    "offered": offered,
                    "kept": kept,
                    "broken": broken,
                    "score": kept as i64 - broken as i64,
                }),
            )
        })
        .collect();
    json!({ "by_principal": Value::Object(by_principal) })
}

fn reputation_capability_fixture() -> symbio_plugin_api::PluginDeclarations<'static> {
    const VIEWS: &[symbio_plugin_api::ViewDecl] = &[symbio_plugin_api::ViewDecl {
        name: "reputation.ledger",
        ver: 1,
        reads: &[
            (EVENT_COMMITMENT_OPENED, "id"),
            (EVENT_COMMITMENT_OPENED, "from"),
            (EVENT_COMMITMENT_RELEASED, "id"),
            (EVENT_COMMITMENT_BROKEN, "id"),
        ],
        evidence: &[],
        cost: 4096,
        eval: reputation_ledger_fixture,
        agg: None,
    }];
    symbio_plugin_api::PluginDeclarations {
        events: &[],
        facts: &[],
        views: VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: &[],
    }
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-loop-test-harness-reputation",
    reputation_capability_fixture
);

// ==================== 工具端口替身 ====================
//
// `execute_tool_async` 薄壳化之后，真实执行住 session 的 `ToolPort`（工具端口
// 提供者）。loop 的单测二进制不链接兄弟插件（E-009，与上面三个替身同一条理由），
// 于是走真执行的批处理用例（并行 / 中止传播 / 部分失败……）需要一个**同形**的
// 端口行，否则薄壳拿到缺席读数、整批响亮失败，批语义用例全部不可达。
//
// 替身只保证「字面名 + 表取 + `capability_invoke`、中止经 select、错误与结果的
// 文本形状与生产一致」；线上名解析 / 空闲超时 / 结果契约的行为判据由 session 一侧
// 的测试电池钉（`crates/symbio-plugin-session/src/port.test.rs`），两处若漂移 e2e 会红。

use symbio_kernel::capability::Registry;
use symbio_kernel::plugin::PluginInvokeRequestExt;
use symbio_plugin_api::port::{
    port_row, PortProvider, ToolCallExecutor, ToolCallOutcome, ToolCallRequest,
};
use symbio_plugin_api::ports;
use symbio_plugin_api::CAPABILITY_VISITOR;

/// 工具端口替身：执行体与生产 `ToolPort` 同形的最小子集（见上方注释）。
pub(crate) struct FixtureToolPort;

impl PortProvider for FixtureToolPort {
    fn port(&self) -> &'static str {
        ports::TOOL
    }

    fn provider_id(&self) -> &str {
        "tests/tool-port"
    }

    fn tool_execute(&self) -> Option<Arc<dyn ToolCallExecutor>> {
        Some(Arc::new(FixtureToolPort))
    }
}

#[async_trait]
impl ToolCallExecutor for FixtureToolPort {
    async fn execute(&self, req: ToolCallRequest) -> ToolCallOutcome {
        let name = req.tool_name.clone();
        // 宿主检查在最前（与生产逐字同形）：无 parent 的调用诚实失败，
        // 不查表、不回落——「无宿主 ⇒ No parent plugin」是既有用例钉住的形状。
        let Some(parent) = req.parent.clone() else {
            return ToolCallOutcome {
                result_text: "Error: No parent plugin".into(),
                success: false,
                pending: None,
            };
        };
        // 「等待用户」意图：与生产同判据（`failure_kind` 闭集），节点仍由环构造。
        let pending_of = |data: &Value| {
            let kind = data.get("failure_kind").and_then(Value::as_str)?;
            if !symbio_kernel::capability::failure_kind::is_pending(kind) {
                return None;
            }
            Some(symbio_plugin_api::port::PendingPrompt {
                text: data
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("需要用户操作")
                    .to_string(),
                prompt: data.get("prompt").cloned().unwrap_or(Value::Null),
                failure_kind: kind.to_string(),
            })
        };
        // 结果文本：content/output 字符串优先，兜底整包 JSON（生产 `extract_result`
        // 的前两条判据——批用例只消费这两种形状）。
        let text_of = |data: &Value| {
            data.get("content")
                .and_then(Value::as_str)
                .or_else(|| data.get("output").and_then(Value::as_str))
                .map(str::to_string)
                .unwrap_or_else(|| data.to_string())
        };

        let visitor: Option<Arc<dyn Registry>> = req.ctx.get(CAPABILITY_VISITOR);
        let tool_ctx = req.ctx.fork();
        let _ = tool_ctx.set_payload(req.args.clone());
        tool_ctx.set(symbio_sdk::keys::TOOL_CALL_ID, req.tool_call_id.clone());
        tool_ctx.set(symbio_sdk::keys::RESULT_MSG_ID, req.result_msg_id.clone());
        tool_ctx.set(symbio_kernel::exec::EVENT_SINK, req.sink.clone());
        tool_ctx.set(symbio_kernel::exec::ABORT_SIGNAL, req.abort.clone());

        // 与生产同序的三条形态：表命中（字面名——夹具工具都是 canonical 名）/
        // 未命中回落 route / 无表直接 route。
        let fut: std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<symbio_kernel::plugin::PluginPayload, PluginError>,
                    > + Send,
            >,
        > = if let Some(visitor) = visitor {
            if let Some(row) = visitor.get(&name).await {
                match row.cap().cloned() {
                    Some(cap) => {
                        let ctx2 = tool_ctx.clone();
                        Box::pin(async move {
                            symbio_kernel::capability::capability_invoke(cap.as_ref(), ctx2).await
                        })
                    }
                    None => {
                        return ToolCallOutcome {
                            result_text: format!("Error: Tool not found: {name}"),
                            success: false,
                            pending: None,
                        }
                    }
                }
            } else {
                tool_ctx.set(symbio_sdk::keys::PATH, name.clone());
                let ctx2 = tool_ctx.clone();
                Box::pin(async move { parent.route(ctx2).await })
            }
        } else {
            tool_ctx.set(symbio_sdk::keys::PATH, name.clone());
            let ctx2 = tool_ctx.clone();
            Box::pin(async move { parent.route(ctx2).await })
        };

        // 中止是第二出口（与生产同形）：用户按下停止 ⇒ 放弃工具、如实报「被中止」。
        let result = tokio::select! {
            biased;
            r = fut => r,
            _ = req.abort.cancelled() => {
                return ToolCallOutcome {
                    result_text: format!("Error: 工具 {name} 被用户中止，未返回结果"),
                    success: false,
                    pending: None,
                };
            }
        };
        match result {
            Ok(resp) => {
                let Ok(data) = resp.get::<Value>() else {
                    return ToolCallOutcome {
                        result_text: "Error: Failed to deserialize payload".into(),
                        success: false,
                        pending: None,
                    };
                };
                ToolCallOutcome {
                    result_text: text_of(&data),
                    success: true,
                    pending: pending_of(&data),
                }
            }
            Err(e) => ToolCallOutcome {
                result_text: format!("Error: {e}"),
                success: false,
                pending: None,
            },
        }
    }
}

/// 给夹具的能力表装上工具端口行（`port.tool` 就绪，替身执行器见上）。
pub(crate) async fn put_tool_port_row<T: Registry + ?Sized>(visitor: &Arc<T>) {
    let port = Arc::new(FixtureToolPort) as Arc<dyn PortProvider>;
    let row = port_row(&port).expect("夹具端口名在闭集内");
    visitor.put(row).await;
}
