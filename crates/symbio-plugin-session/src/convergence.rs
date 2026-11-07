//! 广播出口：把「状态收敛 + 变更投递」集中到一处。
//!
//! 只有一个出口，只做**投递**，不含业务判定：
//! [`Convergence::emit_session_state`]：**会话运行态的唯一出口**——写运行态 → 发一条
//! VDFS 变更（`<sid>` 上 `data` = 全量节点视图，与 `stat` 同源构造 → 消费端零回读）。
//!
//! 它住在 [`Convergence`]（收敛手柄）上而不是 `SessionPlugin` 上：调用方要的是收敛能力，
//! 不是整个插件。
//!
//! ## 为什么运行态要经 VDFS 变更而不是事件
//!
//! 见 `docs/105-node-state-streaming.md`：状态是**节点的属性**，因此幂等、
//! 可交换、丢一次不影响正确性——消费端回读 `stat` 即得当前值；而事件是增量的、
//! 有顺序的（`busy` 与 `idle` 谁先到决定 UI 对错）。随载荷下发的视图是**调用
//! 那一刻**从权威源构造的（与 `stat` 同一构造点），不是缓存副本——幂等不受影响。
//!
//! ## `kind = "session"` 事件频道已废除
//!
//! 会话域原先还往事件总线发一整套 `StreamEvent` 帧（`Status` / `Error` / `Abort`
//! / 消息 `Update`），供**进程内**消费者（子智能体转播、CLI）使用。那两个消费者
//! 现已改订阅 VDFS 变更（`vdfs/watch` + `kind = "vdfs"`），端口随之关闭：
//! 后端只发 VDFS 一条频道，`event_bus` 退化为「VDFS 变更的传输层」。
//! 漏掉这条的人会重新长出一条与 VDFS 并行的第二真相——那正是本次要废除的东西。
//!
//! 可见性：两个出口都在 [`Convergence`] 上，`pub(crate)`（宿主与执行期出口都是同一插件内
//! 的持有者）。

use super::active::ActiveSessionState;
use super::chat_session::Carrier;
use super::declarations::{CHAT_VOCAB, MESSAGE_VOCAB};
use super::plugin::{session_node, OUTCOME_ABORTED, OUTCOME_COMPLETED};
use super::transcript::publish::append_and_publish;
use super::types::SessionSummary;
use serde_json::Value;
use std::sync::Arc;
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_kernel::plugin::PluginError;
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL,
    EVENT_USER_MESSAGE,
};
use symbio_plugin_api::fact_source::FactSource;
use symbio_plugin_api::FactDraft;
use symbio_sdk::authz::PRINCIPAL_MAIN;
use symbio_sdk::chat::chat_node as cm;

/// 会话运行态的一次变化。
///
/// 用枚举而不是 `(status: &str, error: Option<String>)` 两个参数：后者允许
/// "`busy` 还带着上次的错误"这种非法组合存在，而它的表现是**静默的**——
/// 前端会同时显示"处理中"和"上次失败"。
pub enum SessionStateChange {
    /// 新一轮开始（上一轮的结局随之作废）
    Working,
    /// 一轮结束：正常 / 用户中止 / 以错误结束
    Finished {
        outcome: &'static str,
        error: Option<String>,
    },
}

/// 会话**收敛**手柄：把一个会话的运行态与终态落到它身上所需的那四样。
///
/// ## 为什么是手柄而不是插件（[D-075](../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") 前置 P2）
///
/// 收敛这件事的调用方是会话动作（开轮 / 收轮 / 中止收口）——它们要的就是
/// 「打开会话（存储 + 配置）+ 取会话摘要（存储）+ 取运行态（名册）+ 写进这一份
/// 会话状态并发一帧」，把整个插件背进来既多于所需，又把 provider 注册、路由与
/// 订阅表一起钉住。
///
/// `store` 是**同一个** `OnceCell`（不是解析好的 store）：存储的懒初始化只有一条路，
/// 由 [`super::store::resolve_store`] 承载，插件与手柄共用这一个格子。
#[derive(Clone)]
pub(crate) struct Convergence {
    store: Arc<tokio::sync::OnceCell<Arc<super::store::SessionStore>>>,
    dir: std::path::PathBuf,
    config: Arc<tokio::sync::RwLock<super::config::SessionConfig>>,
    active: Arc<super::active::ActiveSessionManager>,
}

impl Convergence {
    pub(crate) fn new(
        store: Arc<tokio::sync::OnceCell<Arc<super::store::SessionStore>>>,
        dir: std::path::PathBuf,
        config: Arc<tokio::sync::RwLock<super::config::SessionConfig>>,
        active: Arc<super::active::ActiveSessionManager>,
    ) -> Self {
        Self {
            store,
            dir,
            config,
            active,
        }
    }

    pub(crate) async fn get_store(&self) -> Result<Arc<super::store::SessionStore>, PluginError> {
        super::store::resolve_store(&self.store, &self.dir).await
    }

    /// 打开会话引擎（存储 + 配置 + id）——`SessionPlugin::open_chat_session` 的实现就在这里，
    /// 那里只是门面。
    pub(crate) async fn open_chat_session(
        &self,
        session_id: &str,
    ) -> Result<Arc<super::chat_session::PersistentChatSession>, PluginError> {
        let store = self.get_store().await?;
        Ok(Arc::new(super::chat_session::PersistentChatSession::new(
            session_id.to_string(),
            self.config.clone(),
            store,
        )))
    }

    /// 会话运行态变化的**唯一出口**。
    ///
    /// 两件事，顺序写死：
    /// 1. **写运行态**——`list` / `stat` 的数据源（与本函数因此同源）；
    /// 2. **发一条 VDFS 变更**（`<sid>` 上 `data` = 全量节点视图）——消费端
    ///    零回读就地收敛；视图与 `stat` 同源（`session_node`），口径必然一致。
    ///
    /// 漏掉第 2 步，UI 会永久停在旧状态（角标不转、停止按钮不出现/不消失），
    /// 且没有任何机制会纠正它。
    ///
    /// ## 视图从哪来
    ///
    /// 调用本函数前运行态刚写完，此时经 `session_node`（与 `list` / `stat` 同一
    /// 构造点）取视图——它读的是**同一份** `ActiveSessionStateInner`，因此发出去
    /// 的就是「此刻的状态」，不是过期副本。会话已被删时取不到视图，退化为
    /// 无载荷变更（回读 `NotFound` 即收敛）。
    ///
    /// ## 与消息变更的关系：同一张表，不靠顺序
    ///
    /// 两者都投到 session provider 的那张 `VdfsChangeSubscriptions`（消息投
    /// `<sid>/message/<mid>`、运行态投 `<sid>`）。到达顺序**没有**机制保证，也
    /// **不需要**：`ChatNode.seq` 决定显示顺序（节点属性），而「本轮是否结束」
    /// 由会话节点自己的 `status` 决定——消费端回读即得，不依赖谁先到。
    ///
    /// 调用时机：本轮**最后一条**消息帧之后（收尾 / 中止的落点由动作层决定，
    /// 本函数只负责写运行态并投递）。
    pub(crate) async fn emit_session_state(
        &self,
        state: &Arc<ActiveSessionState>,
        change: SessionStateChange,
    ) {
        // 运行态写路径各只有一个方法（单写者）——见 `active.rs` 的
        // 「运行态的写路径」：写点散回调用方就会重新长出双写面。
        match &change {
            SessionStateChange::Working => state.begin_turn().await,
            SessionStateChange::Finished { outcome, error } => {
                state.finish_turn(outcome, error.clone()).await
            }
        }

        // 运行态变更挂在会话叶子 `<sid>` 上，`data` = 全量节点视图（与 `stat`
        // 同一构造点 `session_node`）。会话已被删时取不到视图——退化为无载荷
        // 变更，消费端回读 `stat` 得到 `NotFound` 即自然收敛。
        let node = match self.get_store().await {
            Ok(store) => match store.load_session_checked(&state.session_id).await {
                Ok(Some(session)) => {
                    let facts = store
                        .fact_source(&state.session_id)
                        .ok()
                        .flatten()
                        .map(|source| crate::runtime::project(&source.range(0)));
                    let rt = self
                        .active
                        .runtime_of(&state.session_id, facts.as_ref())
                        .await;
                    Some(session_node(&SessionSummary::of(&session), &rt))
                }
                _ => None,
            },
            Err(_) => None,
        };
        state.transcript.lock().await.emit_session_state(node);
    }

    /// 该开轮事实的终答事实 → 转写节点，并收束本轮；没有终答 ⇒ `false`（不投影）。
    ///
    /// 配对靠**事实身份**：由 `user.message` 的 `seq` 取回它的 `event_id`，
    /// 再找载荷 `root_id` 指向它的那条终答——不再拿轮号当键。
    /// 节点由 `rebuild` 从**同一条事实**折出（读路径与写路径同一个折叠）。幂等：
    /// 同一条事实再投影一次时，`apply` 见到节点已存在会跳过（不重复追加）。
    ///
    /// `drive` 是**本部署根**的驱动实例（会话插件经 `drive_entry` 取，收轮任务用
    /// 开轮时绑好的那一份）。这里刻意不读进程级 `declared_drive_entry()`：那是
    /// 主根的一份，子智能体容器的会话等它会等到别人的空闲上（t29 / D-365）。
    pub(crate) async fn project_final_turn(
        &self,
        session_id: &str,
        user_seq: u64,
        source: &dyn FactSource,
        drive: Option<&Arc<dyn symbio_plugin_api::DriveEntry>>,
    ) -> Result<bool, PluginError> {
        let events = source.range(0);
        let user = events
            .iter()
            .find(|e| {
                e.kind == EVENT_USER_MESSAGE && e.seq.map(|seq| seq.value()) == Some(user_seq)
            })
            .cloned();
        // 编码先于巩固（D-268）：终答一落格就踢起一轮驱动，`session.close-turn` 债在
        // 那一轮里才写 `memory.encoded`。不等到「没有在飞的债」就读快照，巩固看到的
        // 是半轮——t29 的失败形态正是 4 条经验只到 3 条、合并被跳过，而这一轮之后
        // 再没有调用方会回来巩固。等到的空闲 = 应答与编码都已落格（t29 / D-359）。
        //
        // 两格仍按活集兜底（条数不够就不合并）：轮停在等外部事实（`NeedsHuman`）时
        // 编码本来就还没发生；安全阀出口（`Exhausted` / `Stalled`）意味着本轮没跑完。
        // 事件键按 `(user_seq, 源 seq)` 定死，CLI 投影与后台收轮的重复调用按
        // `Duplicate` 收敛，不会多出一条合并产物。
        if let Some(entry) = drive {
            entry.settled(session_id).await;
        }
        let events = source.range(0);
        let now = events.iter().map(|e| e.ts).max().unwrap_or(0);
        if let Err(why) =
            crate::memory_consolidate::consolidate_turn(source, &events, user_seq, now)
        {
            symbio_plugin_api::plugin_warn!("session", "记忆巩固落格失败（{session_id}）：{why}");
        }
        let Some(user) = user else {
            return Ok(false);
        };
        let Some(final_event) = events
            .iter()
            .rev()
            .find(|e| {
                e.kind == EVENT_CHAT_ASSISTANT_FINAL
                    && e.payload.get("root_id").and_then(Value::as_str) == Some(&user.event_id)
            })
            .cloned()
        else {
            // 没有终答：若本轮落了「等用户动作」的产物，本轮就停在卡片上。
            // 这是 interactive 档的正常收束形态（`ask_user` / 审批卡）——运行器
            // 不再推进工具循环，CLI 也据此以退出码 0 结束。归本轮靠 seq：只看
            // 开轮事实之后的这一段，前一轮没答复的卡不会误判。
            let waiting_events: Vec<_> = events
                .iter()
                .filter(|e| e.seq.is_some_and(|seq| seq.value() > user_seq))
                .cloned()
                .collect();
            let waiting_on_user = waiting_events.iter().any(|e| {
                e.kind == EVENT_ARTIFACT_ADDED
                    && e.payload.get("needs_user_action").and_then(Value::as_bool) == Some(true)
            });
            // 发言闸拒也是**正常收束**（D-067）：终答格被拒 ⇒ 不落 `final`，但轮
            // 照样了结——拒绝是记录层的结论，不是静默中断。判据按**事实身份**认：
            // `authz.denied{key = chat.assistant.final}` 且 `root_id` 归属本轮。
            // 没有这一支，会话停在 `working`、CLI 等不到收束（t73 的 B 侧）。
            let speech_denied = waiting_events.iter().any(|e| {
                e.kind == CHAT_VOCAB.denied
                    && e.payload.get("key").and_then(Value::as_str) == Some(CHAT_VOCAB.final_reply)
                    && e.payload.get("root_id").and_then(Value::as_str)
                        == Some(user.event_id.as_str())
            });
            if !waiting_on_user && !speech_denied {
                // 失败也要收束：本轮落了 `failed`（模型 / 工具失败）却没有终答事实，
                // 就没有任何节点承载「这一轮怎么结束的」——会话停在 working、CLI
                // 等不到收束。这里补一条**失败终态**：正文取 `failed` 的可读原因，
                // 与卡片 / 终答走同一条折叠 + 发布（同一个 closer、同一份事实源，
                // 不新增契约、不新增写点）。
                let failed = waiting_events
                    .iter()
                    .rev()
                    .find(|e| e.kind == symbio_plugin_api::EVENT_FAILED)
                    .cloned();
                let Some(failed) = failed else {
                    return Ok(false);
                };
                // 环侧失败格的键是 `reason`（`ring/engine.rs` 的 `failed` 载荷），
                // 执行体自己落的那条用 `why`——两个写法都读，否则环收口那条会
                // 掉回通用文案，把「长度截断」说成「模型调用失败」（t77）。
                let why = failed
                    .payload
                    .get("reason")
                    .or_else(|| failed.payload.get("why"))
                    .and_then(Value::as_str)
                    .unwrap_or("模型调用失败")
                    .to_string();
                // 中止不是失败（D-044 / D-165：中止与失败只在 `status` 上区分）。
                // 中止只落终答格（`status: "aborted"`），**不落兜底、不推失败节点**
                // ——那两条会把「用户按下停止」说成错误，也会把中止样本记进兜底率。
                let aborted = failed.payload.get("status").and_then(Value::as_str)
                    == Some("aborted")
                    || why == "aborted";
                if aborted {
                    let root_id = user.event_id.as_str();
                    // 正文取 `failed` 折出的半截（`output_payload` 的 `answer`），与
                    // 正文节点同源；没有半截（派发前就中止）则为空串。
                    let text = failed
                        .payload
                        .get("answer")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let has_final = events.iter().any(|e| {
                        e.kind == EVENT_CHAT_ASSISTANT_FINAL
                            && e.payload.get("root_id").and_then(Value::as_str) == Some(root_id)
                    });
                    if !has_final {
                        // 终答格是这一轮的**持久收束证据**：重启后 `runtime` 按
                        // `final.status` 判 `outcome=aborted`（映射已有）。
                        source
                            .append(
                                PRINCIPAL_MAIN.to_string(),
                                FactDraft {
                                    key: format!("{EVENT_CHAT_ASSISTANT_FINAL}:{root_id}"),
                                    kind: EVENT_CHAT_ASSISTANT_FINAL.to_string(),
                                    entity: Entity::Scope,
                                    verb: Verb::Closed,
                                    payload: serde_json::json!({
                                        "root_id": user.event_id,
                                        "status": "aborted",
                                        "reason": "interrupted",
                                        "text": &text,
                                        "principal": PRINCIPAL_MAIN,
                                        // 对话面节点：与正常终答同一形状（`rebuild`
                                        // 的 `dialog` 分支折出一条正文节点），否则
                                        // 只剩「轮骨架」——D-335 后骨架不是发布身份，
                                        // 这一轮在对话面就凭空消失。id 与模型端口
                                        // 交回的正文身份同源（`{root}-text`）：中止前
                                        // 已流过正文时，`failed` 折出的节点与它同 id，
                                        // 重建按 id 合成一条，不会多出第二个 aborted。
                                        "dialog": {
                                            "id": format!("{root_id}-text"),
                                            "reason": "interrupted",
                                            "text": &text,
                                            "exclude_from_context": false,
                                        },
                                    }),
                                    produced_by: Some(user_seq),
                                },
                            )
                            .map_err(|e| {
                                PluginError::InternalError(format!("中止收束落格失败: {e}"))
                            })?;
                    }
                    // 终答格刚落格：重建要**看得见它**。只折写前那段快照，首读为空、
                    // 重启后又出现——同一轮两条命。重读本段事实再折，与落库面同源。
                    let fresh = source.range(0);
                    let fresh_waiting: Vec<_> = fresh
                        .iter()
                        .filter(|e| e.seq.is_some_and(|seq| seq.value() > user_seq))
                        .cloned()
                        .collect();
                    // 正文节点全部由 `rebuild` 从事实折出：中止前已流过正文时来自
                    // `failed` 的半截，否则来自刚落格的终答 `dialog`（同 id，合一条）。
                    // 中止不推 `failed:` 节点（那是失败路径，D-044 / D-165）。
                    let nodes: Vec<cm::ChatNode> = symbio_sdk::projection::rebuild::rebuild(
                        &fresh_waiting,
                        &CHAT_VOCAB,
                        &MESSAGE_VOCAB,
                    )
                    .into_iter()
                    .map(|mut node| {
                        node.seq = None;
                        node
                    })
                    .collect();
                    if !nodes.is_empty() {
                        let session = self.open_chat_session(session_id).await?;
                        append_and_publish(
                            &session,
                            nodes,
                            Carrier::Runner,
                            &self.active,
                            session_id,
                            PRINCIPAL_MAIN,
                        )
                        .await?;
                    }
                    let state = self.active.get_or_create(session_id).await;
                    self.emit_session_state(
                        &state,
                        SessionStateChange::Finished {
                            outcome: super::plugin::OUTCOME_ABORTED,
                            error: None,
                        },
                    )
                    .await;
                    return Ok(true);
                }
                // 兜底格（`chat.assistant.fallback`）：本轮没有终答、以失败收束时，
                // 除「哪一步坏了」之外还必须有一条**轮级收束事实**——`slo.fallback`
                // 按它算兜底率，`runtime` 按它判结局（两处声明都在，缺的只是写方，t47）。
                //
                // **与环的 `failed` 的关系**：环的 `failed` 是执行事实（哪个端口、
                // 为什么），先落；这一格是会话对**这一轮**的收束，后落，两者不重复
                // ——前者回答「哪一步坏了」，后者回答「这一轮以什么结束」。
                // 键按轮固定并在写前查重：重入（CLI 投影 / 后台收轮重复调用）每轮
                // 至多写一条。
                let root_id = user.event_id.as_str();
                let has_fallback = events.iter().any(|e| {
                    e.kind == EVENT_CHAT_ASSISTANT_FALLBACK
                        && e.payload.get("root_id").and_then(Value::as_str) == Some(root_id)
                });
                if !has_fallback {
                    // 两个收轮路径并发（CLI 的 `session/project` 与后台收轮）：
                    // 两者都用同一个按轮固定的幂等键，后到的拿到 `Duplicate`。
                    // 「已经有兜底格」不是失败——那一轮要的事实已在，重入直接继续。
                    match source.append(
                        PRINCIPAL_MAIN.to_string(),
                        FactDraft {
                            key: format!("{EVENT_CHAT_ASSISTANT_FALLBACK}:{root_id}"),
                            kind: EVENT_CHAT_ASSISTANT_FALLBACK.to_string(),
                            entity: Entity::Scope,
                            verb: Verb::Closed,
                            payload: serde_json::json!({
                                "root_id": user.event_id,
                                "status": "failed",
                                "reason": "fallback",
                                "detail": &why,
                                // 正文由失败节点承载：这一格若再带正文，读面会
                                // 多长出一条同轮正文节点（t77 的空正文判据）。
                                "text": "",
                                "principal": PRINCIPAL_MAIN,
                            }),
                            produced_by: Some(user_seq),
                        },
                    ) {
                        Ok(_) => {}
                        Err(e) if e.contains("Duplicate") => {}
                        Err(e) => {
                            return Err(PluginError::InternalError(format!(
                                "兜底收束落格失败: {e}"
                            )));
                        }
                    }
                }
                let mut nodes: Vec<cm::ChatNode> = symbio_sdk::projection::rebuild::rebuild(
                    &waiting_events,
                    &CHAT_VOCAB,
                    &MESSAGE_VOCAB,
                )
                .into_iter()
                .map(|mut node| {
                    node.seq = None;
                    node
                })
                .collect();
                nodes.push(cm::ChatNode {
                    id: format!("failed:{}", failed.event_id),
                    msg_type: Some(cm::NodeKind::Body),
                    content: Some(cm::MessageContent::Text(why.clone())),
                    status: Some(cm::NodeStatus::Failed),
                    error: Some(why.clone()),
                    ..Default::default()
                });
                if !nodes.is_empty() {
                    let session = self.open_chat_session(session_id).await?;
                    append_and_publish(
                        &session,
                        nodes,
                        Carrier::Runner,
                        &self.active,
                        session_id,
                        PRINCIPAL_MAIN,
                    )
                    .await?;
                }
                let state = self.active.get_or_create(session_id).await;
                self.emit_session_state(
                    &state,
                    SessionStateChange::Finished {
                        outcome: super::plugin::OUTCOME_FAILED,
                        error: Some(why),
                    },
                )
                .await;
                return Ok(true);
            }
            // 卡片节点与终答走同一条折叠 + 发布（读路径与写路径同一份规则）：
            // 本轮所有节点（正文 / 工具调用 / 卡片）一并投影，`apply` 对已存在的
            // 节点幂等跳过。否则实时面只看得到环的协议事实、看不到卡片。
            let nodes: Vec<cm::ChatNode> = symbio_sdk::projection::rebuild::rebuild(
                &waiting_events,
                &CHAT_VOCAB,
                &MESSAGE_VOCAB,
            )
            .into_iter()
            .map(|mut node| {
                node.seq = None;
                node
            })
            .collect();
            let projected: Vec<String> = nodes.iter().map(|node| node.id.clone()).collect();
            if !nodes.is_empty() {
                let session = self.open_chat_session(session_id).await?;
                append_and_publish(
                    &session,
                    nodes,
                    Carrier::Runner,
                    &self.active,
                    session_id,
                    PRINCIPAL_MAIN,
                )
                .await?;
            }
            // 发言闸拒：模型在流式期已经把正文推上实时面（`llm_emit_message`），但这一轮
            // 没有终答格来接替它（`chat.assistant.final` 被写侧闸拒）——不撤回落空的那条，
            // 转写里就留下一条永远 `streaming` 的助手正文，与「拒了就不入格」相矛盾
            // （t71 ⑧）。撤销复用既有的 `removed` 帧，不新造状态（同 D-347 的口径）。
            if speech_denied {
                let ghosts: Vec<cm::ChatNode> = {
                    let state = self.active.get_or_create(session_id).await;
                    let live = state.transcript.lock().await.snapshot();
                    live.into_iter()
                        .filter(|node| {
                            node.status == Some(cm::NodeStatus::Streaming)
                                && node.msg_type == Some(cm::NodeKind::Body)
                                && node.tool_call_id.is_none()
                                && !projected.iter().any(|id| id == &node.id)
                        })
                        .map(|node| cm::ChatNode {
                            id: node.id,
                            status: Some(cm::NodeStatus::Removed),
                            ..Default::default()
                        })
                        .collect()
                };
                if !ghosts.is_empty() {
                    self.active.transcript_apply_all(session_id, ghosts).await;
                }
            }
            let state = self.active.get_or_create(session_id).await;
            self.emit_session_state(
                &state,
                SessionStateChange::Finished {
                    outcome: OUTCOME_COMPLETED,
                    error: None,
                },
            )
            .await;
            return Ok(true);
        };

        // 本轮的全部事实（开轮之后、终答为止）：推理 / 正文 / 工具调用都从这一
        // 段折出。只喂终答一格会把工具轮（没有 `chat.assistant.final` 的那一半）
        // 的节点整段丢掉——实时面只剩 user.message 与 final。
        let final_seq = final_event.seq.map(|seq| seq.value()).unwrap_or(u64::MAX);
        let turn_events: Vec<_> = events
            .iter()
            .filter(|e| {
                e.seq
                    .is_some_and(|seq| seq.value() > user_seq && seq.value() <= final_seq)
            })
            .cloned()
            .collect();
        let nodes: Vec<cm::ChatNode> =
            symbio_sdk::projection::rebuild::rebuild(&turn_events, &CHAT_VOCAB, &MESSAGE_VOCAB)
                .into_iter()
                .map(|mut node| {
                    // seq 由存储在临界区内权威分配；折叠给的是「从这份切片起」的局部号。
                    node.seq = None;
                    node
                })
                .collect();
        if !nodes.is_empty() {
            let session = self.open_chat_session(session_id).await?;
            append_and_publish(
                &session,
                nodes,
                Carrier::Runner,
                &self.active,
                session_id,
                PRINCIPAL_MAIN,
            )
            .await?;
        }

        let state = self.active.get_or_create(session_id).await;
        // 终答格的 `status` 是这一轮的持久结局：中止轮（`aborted`）与正常轮
        // （`completed`）走同一条收束，但结局不同——重入投影不能把中止覆盖成完成。
        let aborted = final_event.payload.get("status").and_then(Value::as_str) == Some("aborted");
        self.emit_session_state(
            &state,
            SessionStateChange::Finished {
                outcome: if aborted {
                    OUTCOME_ABORTED
                } else {
                    OUTCOME_COMPLETED
                },
                error: None,
            },
        )
        .await;
        Ok(true)
    }
}

/// 对话线：由既有折叠（与实时面同一份规则）折出用户 / 助手文本节点。
///
/// **不要在这里重拼「以下是历史」的散文**：那会把同一份事实变成两种形状，且给
/// 「谁说了什么」再错一次的机会。只留 `user | assistant` 两类——工具卡不属于对话线。
pub(crate) fn dialog_context(events: &[Event]) -> Vec<cm::ChatNode> {
    symbio_sdk::projection::rebuild::rebuild(events, &CHAT_VOCAB, &MESSAGE_VOCAB)
        .into_iter()
        .filter(|node| {
            matches!(
                node.node_role(),
                Some(cm::NodeRole::User) | Some(cm::NodeRole::Assistant)
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "convergence.test.rs"]
mod tests;
