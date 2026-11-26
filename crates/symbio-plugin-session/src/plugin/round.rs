//! 一轮的两半——**会话侧**的写方与投影。
//!
//! - 收件箱队首 → 一条 `user.message` 事实（开轮）。收件箱归本插件，会话身份也归
//!   本插件，因此这条事实的写方在这里，不在宿主。
//! - 终答事实 → 转写（收束）。`chat.assistant.final` 是本轮已经落下的事实，
//!   本插件把它投影成消息节点；**宿主不写任何业务事实**。
//!
//! 本插件**不调** `drive`（那会变成插件调插件）：它只写事实 / 读事实，宿主看到
//! 新事实再单次驱动那一轮。两半之间的唯一接口是**事实源 + 开轮事实的身份**：
//! 事实源在打开时就绑定会话，`user.message` 的 `seq` / `event_id` 就是这一轮的名字。

use super::*;
use crate::chat_session::Carrier;
use crate::convergence::SessionStateChange;
use crate::projections::facts::FleetSnapshot;
use crate::route_guard::require_sid;
use crate::transcript::publish::append_and_publish;
use crate::transcript::supplements::merge_supplements;
use symbio_platform::kernel::capability::Registry;
use symbio_platform::kernel::event::{Entity, Event, Verb};
use symbio_platform::kernel::LatencyTier;
use symbio_platform::sdk::authz::PRINCIPAL_MAIN;
use symbio_platform::sdk::drive::DriveEntry;
use symbio_platform::sdk::plugin::fact_source::FactSource;
use symbio_plugin_api::capability_take_errors;
use symbio_plugin_api::debt::{session_config_id, session_recall_id};
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_SCOPE_DISPATCH, EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::retrieval::{retrieve_all, RetrievalRequest};
use symbio_plugin_api::FactDraft;
use symbio_plugin_api::PRINCIPAL_USER;
use symbio_plugin_api::SESSION_ID;

pub(crate) struct PreparedExecutionContext {
    pub(crate) manager: Arc<dyn Registry>,
    pub(crate) ctx: Arc<dyn PluginInvokeRequest>,
    agent_perms: Option<symbio_plugin_api::AgentScope>,
}

impl PreparedExecutionContext {
    pub(crate) fn publish(
        &self,
        store: &super::super::store::SessionStore,
        session_id: &str,
        source_principal: Option<String>,
    ) {
        store.set_source_principal(session_id, source_principal);
        store.set_agent_scope(session_id, self.agent_perms.clone());
    }
}

pub(crate) struct RoundOpenError {
    pub(crate) error: PluginError,
    pub(crate) user_seq: Option<u64>,
}

fn execution_context_snapshot(
    store: &super::super::store::SessionStore,
    session_id: &str,
) -> (Option<String>, Option<symbio_plugin_api::AgentScope>) {
    let source = store
        .source_principal_cell(session_id)
        .read()
        .ok()
        .and_then(|source| source.clone());
    let scope = store
        .agent_scope_cell(session_id)
        .read()
        .ok()
        .and_then(|scope| scope.clone());
    (source, scope)
}

fn restore_execution_context(
    store: &super::super::store::SessionStore,
    session_id: &str,
    snapshot: &(Option<String>, Option<symbio_plugin_api::AgentScope>),
) {
    store.set_source_principal(session_id, snapshot.0.clone());
    store.set_agent_scope(session_id, snapshot.1.clone());
}

/// 工具表的**去重**：子 Agent 作用域会把同一能力再注册一遍，而 `PrefixedCapability`
/// 只改名、执行原样透传（owner 插件的作用域访问器），所以对模型
/// 而言两份是同一个能力。
///
/// 哪两个名字是同一能力的两份，由**声明面**回答（`SCOPE_TWIN` 的 `(前缀名, 本名)` 对，
/// ）；本函数只做一次集合判定：前缀名与本名都在本轮工具表里 ⇒ 前缀那份是副本，
/// 去掉。本名缺席（能力被卸载 / 子树独有工具）⇒ 前缀那份必须保留，否则能力会凭空消失。
///
/// 控制面不解析任何名字形状，也不认识前缀规则；判定与写入顺序无关（集合，不是遍历序）。
fn dedup_declared_twins(
    metas: Vec<symbio_platform::kernel::capability::CapabilityMeta>,
    twins: &[(String, String)],
) -> Vec<symbio_platform::kernel::capability::CapabilityMeta> {
    let names: std::collections::HashSet<&str> =
        metas.iter().map(|meta| meta.name.as_str()).collect();
    let dropped: std::collections::HashSet<&str> = twins
        .iter()
        .filter(|(scoped, base)| names.contains(scoped.as_str()) && names.contains(base.as_str()))
        .map(|(scoped, _)| scoped.as_str())
        .collect();
    metas
        .into_iter()
        .filter(|meta| !dropped.contains(meta.name.as_str()))
        .collect()
}

/// 本轮**暴露视图**：显式暴露 ∩ 作用域允许 ∩ 有效绑定 + wire 别名。
///
/// - 有效绑定取装配期收集容器 `PortBindings` 的 `bound()`只有描述行、
///   没有执行体的孤儿行不进清单；只有绑定、没有描述行的内部端口也不进清单。
///   作用域允许由 `collect_capabilities` 的 `agent_scope` 在同一次收集里保证。
/// - **本插件不算别名**：候选集合交给 `model/project_tools`，取回
///   `{ exposed, aliases, collisions }` 原样落 `session.config`。协议字符集是模型
///   侧的知识，会话不认识 `_` 替换，也不替碰撞做「优先匹配」——碰撞整组退出由投影
///   回答，会话只按回答过滤。
fn candidate_tools(
    metas: Vec<symbio_platform::kernel::capability::CapabilityMeta>,
    bound: &std::collections::HashSet<String>,
) -> Vec<symbio_platform::kernel::capability::CapabilityMeta> {
    metas
        .into_iter()
        .filter(|meta| bound.contains(&meta.name))
        .collect()
}

/// 本轮工具投影的读视图（`model/project_tools` 回执的形状）。
///
/// `collisions` 原样透传（形状归投影方解释），`session.config` 只负责搬运。
#[derive(Debug, Default, Clone, serde::Deserialize, serde::Serialize)]
struct ToolProjectionView {
    #[serde(default)]
    exposed: Vec<String>,
    #[serde(default)]
    aliases: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    collisions: Vec<serde_json::Value>,
}

/// 经 route 问模型插件要一次投影；取不到 ⇒ `Err`，由调用方落「本轮无工具」。
///
/// 跨插件只走 route（与 `compose/wording` 同一形态）：子上下文是同一份 ctx 的
/// fork；父句柄取**本插件的挂载父**（`get_parent`，与能力收集同一条路），不依赖
/// 调用方 ctx 上有没有父指针——驱动重建的 ctx 不保证带它。
async fn project_tools(
    parent: &Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    names: &[String],
) -> Result<ToolProjectionView, PluginError> {
    let child = ctx.fork();
    child.set(
        symbio_plugin_api::PATH,
        symbio_plugin_api::routes::ROUTE_MODEL_PROJECT_TOOLS.to_string(),
    );
    child.set_payload(json!({ "names": names }))?;
    let response = parent.clone().route(child).await?;
    response
        .get::<ToolProjectionView>()
        .map_err(|e| PluginError::ParseError(format!("模型工具投影回执无法解析: {e}")))
}

impl SessionPlugin {
    /// 收件箱队首 → `user.message` 事实；返回这条事实的 `seq`（无消息 ⇒ `None`）。
    ///
    /// `seq` 由事实源在入库时分配，它就是**开轮事实的身份**：`session.config` 的
    /// 溯源指向它；`session.config` 的幂等键则由这条 `user.message` 的入库 id 派生
    /// （`session_config_id`），让模型债行的 `when_id` 能只认**这一轮**的 config。
    pub(crate) async fn consume_inbox_turn(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        source: Arc<dyn FactSource>,
    ) -> Result<Option<u64>, PluginError> {
        let state = self.active_mgr.get_or_create(session_id).await;
        // 忙则留队：出队与开轮是同一件事的两半，忙窗里出队会开出第二条并发轮。
        let Some(item) = self.take_inbox_front(&state).await else {
            return Ok(None);
        };
        let rollback = vec![item.clone()];
        self.consume_item_turn(ctx, session_id, source, item, rollback)
            .await
    }

    /// 开轮与恢复动作共用的执行上下文准备；返回值供开轮装配系统提示词与工具表。
    ///
    /// 来源主体和 Agent 权限都是进程内状态，worker 重启后必须从本次可信请求与
    /// 会话绑定重新发布，且必须早于第一条会踢驱动的事实。恢复动作只复用这段准备，
    /// 不新建中止信号——中止仍归正在运行的那一轮所有。
    pub(crate) async fn prepare_execution_context(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
    ) -> Result<PreparedExecutionContext, PluginError> {
        let store = self.get_store().await?;
        let round_ctx = ctx.fork();
        // 收集槽归本轮所有；不得从调用方继承上一轮的 OnceLock 或端口容器。
        round_ctx.set(
            symbio_plugin_api::AGENT_PERMS,
            Arc::new(std::sync::OnceLock::new()),
        );
        round_ctx.set(
            symbio_plugin_api::SANDBOX,
            Arc::new(std::sync::OnceLock::new()),
        );
        // 宿主预置绑定是请求级能力；收集器在其副本上追加插件绑定。
        // 这里显式换容器，父插件缺席时也不会把调用方的旧容器直接带回本轮。
        let host_bindings = ctx
            .get(symbio_plugin_api::port::PORT_BINDINGS)
            .map(|bindings| bindings.fresh())
            .unwrap_or_default();
        round_ctx.set(
            symbio_plugin_api::port::PORT_BINDINGS,
            Arc::new(host_bindings),
        );

        // 作用域按**会话自己的绑定**展开（metadata.agent_id）：收集器只把它当通用的
        // agent 作用域键递下去，子智能体插件自行把子树 prompt rows / 工具加前缀。
        let agent_scope = store.session_agent_scope(session_id);
        // 会话级数据目录：会话存储布局的唯一 owner 是本插件，收集期把目录
        // 按**值**递上请求 ctx（memory 的会话记忆据此落位）。
        let session_dir = store.session_dir(session_id);
        if let Some(dir) = &session_dir {
            round_ctx.set(
                symbio_plugin_api::SESSION_DIR,
                dir.to_string_lossy().into_owned(),
            );
        }
        // 作用域 VDFS provider：在真实能力收集前挂上工作区 / 会话两层。
        // `AGENT_VDFS` 不由本处决定——保留 ctx 的既有绑定。
        let work_dir = ctx
            .get(symbio_plugin_api::WORKDIR)
            .map(std::path::PathBuf::from);
        // 子空间开轮时 ctx 可能已经被子智能体插件重挂 `AGENT_VDFS` 到该子智能体目录，
        // 这里不能覆盖，否则子会话会注入顶层人格而不是自己的（T15）。
        symbio_providers::attach_round_providers(
            round_ctx.as_ref(),
            work_dir.as_deref(),
            session_dir.as_deref(),
        );
        let manager = crate::capabilities::collect_capabilities(
            self.get_parent().as_ref(),
            &round_ctx,
            agent_scope.as_deref(),
        )
        .await;
        // 选中了智能体 ⇒ 收集期必须发布权限作用域。发布不到就拒绝，不能当作
        // 「没有选智能体」的宿主态放行——那正是 t84 的漏过窗口。
        let agent_perms = crate::capabilities::require_agent_scope(
            agent_scope.as_deref(),
            symbio_plugin_api::agent_perms_of(round_ctx.as_ref()),
        )?;
        Ok(PreparedExecutionContext {
            manager,
            ctx: round_ctx,
            agent_perms,
        })
    }

    /// 把**一条已出队**的条目落成本轮（写臂与轮收尾抽干共用同一份落格）。
    ///
    /// 入队形态（`InboxItem`）是两条路径的共同语言：写臂按一条开轮，轮收尾把
    /// 一批补充合并成一条（`merge_supplements`）后同样以一条开下一轮。
    pub(crate) async fn consume_item_turn(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        source: Arc<dyn FactSource>,
        item: InboxItem,
        rollback: Vec<InboxItem>,
    ) -> Result<Option<u64>, PluginError> {
        let state = self.active_mgr.get_or_create(session_id).await;
        let snapshot = match self.get_store().await {
            Ok(store) => Some(store),
            Err(error) => {
                self.restore_inbox_front(&state, &rollback).await;
                self.convergence
                    .emit_session_state(
                        &state,
                        SessionStateChange::Finished {
                            outcome: super::OUTCOME_FAILED,
                            error: Some(error.to_string()),
                        },
                    )
                    .await;
                return Err(error);
            }
        };
        let previous = snapshot
            .as_ref()
            .map(|store| execution_context_snapshot(store, session_id));
        let source_principal = item.params.source_principal.clone();

        let prepared = match self.prepare_execution_context(ctx, session_id).await {
            Ok(prepared) => prepared,
            Err(error) => {
                self.restore_inbox_front(&state, &rollback).await;
                self.convergence
                    .emit_session_state(
                        &state,
                        SessionStateChange::Finished {
                            outcome: super::OUTCOME_FAILED,
                            error: Some(error.to_string()),
                        },
                    )
                    .await;
                return Err(error);
            }
        };
        if let Some(store) = &snapshot {
            prepared.publish(store, session_id, source_principal);
        }

        let result = self
            .open_reserved_round(
                &prepared.ctx,
                session_id,
                source.as_ref(),
                item,
                prepared.manager,
            )
            .await;
        match result {
            Ok(seq) => {
                self.convergence
                    .emit_session_state(&state, SessionStateChange::Working)
                    .await;
                Ok(seq)
            }
            Err(open_error) => {
                if let Some(user_seq) = open_error.user_seq {
                    self.convergence
                        .emit_session_state(&state, SessionStateChange::Working)
                        .await;
                    self.spawn_round_closer(
                        session_id.to_string(),
                        user_seq,
                        source,
                        self.drive_entry(),
                        self.get_parent().map(|_| Arc::clone(ctx)),
                        Some(open_error.error.to_string()),
                    );
                } else {
                    if let (Some(store), Some(previous)) = (&snapshot, &previous) {
                        restore_execution_context(store, session_id, previous);
                    }
                    self.restore_inbox_front(&state, &rollback).await;
                    self.convergence
                        .emit_session_state(
                            &state,
                            SessionStateChange::Finished {
                                outcome: super::OUTCOME_FAILED,
                                error: Some(open_error.error.to_string()),
                            },
                        )
                        .await;
                }
                Err(open_error.error)
            }
        }
    }

    /// 恢复动作落在原轮根上：等原轮收束后重新占轮，首条答复事实落格前发布可信上下文。
    pub(crate) async fn append_resume_fact(
        &self,
        host: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        call_id: &str,
        source_principal: Option<String>,
        mut draft: FactDraft,
    ) -> Result<(), PluginError> {
        let store = self.get_store().await?;
        let events = store
            .read_events(session_id)
            .map_err(PluginError::InternalError)?;
        let card_seq = events
            .iter()
            .rev()
            .find(|event| {
                event.kind == EVENT_ARTIFACT_ADDED
                    && event.payload.get("tool_call_id").and_then(Value::as_str) == Some(call_id)
            })
            .and_then(|event| event.seq.map(|seq| seq.value()))
            .ok_or_else(|| {
                PluginError::NotFound(format!("找不到调用 {call_id} 的待答复工具产物"))
            })?;
        let user_seq = events
            .iter()
            .rev()
            .find(|event| {
                event.kind == EVENT_USER_MESSAGE
                    && event.seq.is_some_and(|seq| seq.value() < card_seq)
            })
            .and_then(|event| event.seq.map(|seq| seq.value()))
            .ok_or_else(|| {
                PluginError::InternalError(format!("调用 {call_id} 没有可恢复的轮次根"))
            })?;

        let state = self.active_mgr.get_or_create(session_id).await;
        state.begin_when_idle().await;
        let previous = execution_context_snapshot(&store, session_id);
        let prepared = match self.prepare_execution_context(host, session_id).await {
            Ok(prepared) => prepared,
            Err(error) => return Err(self.fail_reserved_round(&state, error).await),
        };
        let source = match self
            .fact_source_with_bindings(session_id, host.get(symbio_plugin_api::port::PORT_BINDINGS))
            .await
        {
            Ok(source) => source,
            Err(error) => return Err(self.fail_reserved_round(&state, error).await),
        };
        let Some(anchor) = source
            .range(0)
            .last()
            .and_then(|event| event.seq.map(|seq| seq.value()))
        else {
            return Err(self
                .fail_reserved_round(
                    &state,
                    PluginError::InternalError(
                        "答复未能入格：事实源为空，没有可用的溯源锚".to_string(),
                    ),
                )
                .await);
        };
        draft.produced_by = Some(anchor);
        prepared.publish(&store, session_id, source_principal);
        if let Err(error) = source.append(symbio_plugin_api::PRINCIPAL_USER.to_string(), draft) {
            restore_execution_context(&store, session_id, &previous);
            return Err(self
                .fail_reserved_round(
                    &state,
                    PluginError::InternalError(format!("user.answer 落格失败：{error}")),
                )
                .await);
        }

        self.convergence
            .emit_session_state(&state, SessionStateChange::Working)
            .await;
        let next_ctx = self.get_parent().map(|_| {
            let ctx = host.fork();
            ctx.set(SESSION_ID, session_id.to_string());
            ctx.set(FACT_SOURCE, Arc::clone(&source));
            ctx
        });
        self.spawn_round_closer(
            session_id.to_string(),
            user_seq,
            source,
            self.drive_entry(),
            next_ctx,
            None,
        );
        Ok(())
    }

    async fn fail_reserved_round(
        &self,
        state: &Arc<crate::active::ActiveSessionState>,
        error: PluginError,
    ) -> PluginError {
        self.convergence
            .emit_session_state(
                state,
                SessionStateChange::Finished {
                    outcome: super::OUTCOME_FAILED,
                    error: Some(error.to_string()),
                },
            )
            .await;
        error
    }

    async fn open_reserved_round(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        source: &dyn FactSource,
        item: InboxItem,
        manager: Arc<dyn Registry>,
    ) -> Result<Option<u64>, RoundOpenError> {
        let text = item
            .message
            .content
            .as_ref()
            .map(|c| c.to_text())
            .unwrap_or_default();
        // `meta` 必须随用户格事实入格：读路径由 `rebuild::user_node` 从载荷复刻，
        // 落在节点上而不在载荷里就等于只有实时面看得到——重开会话后合并标记消失，
        // 同一条发言在两面成了两个身份（t19 的 A 线读的就是重建面）。
        let payload = match item.message.meta.as_ref() {
            Some(meta) => json!({ "text": text, "id": item.id, "meta": meta }),
            None => json!({ "text": text, "id": item.id }),
        };
        let draft = FactDraft {
            key: item.id.clone(),
            kind: EVENT_USER_MESSAGE.to_string(),
            entity: Entity::Scope,
            verb: Verb::Opened,
            payload,
            produced_by: None,
        };
        let seq = source
            .append(PRINCIPAL_USER.to_string(), draft)
            .map_err(|error| RoundOpenError {
                error: PluginError::InternalError(format!("user.message 落格失败: {error}")),
                user_seq: None,
            })?;

        let result: Result<(), PluginError> = async {
            // 幕级派发格（ / ）：`tier` 是档位的**唯一来源**，写侧在开轮处。
            // 快路（技能命中直接作答）随 ` B2` 整条退场后，本幕没有第二条执行路径——
            // 档位折自「还能落到别处吗」的答案恒为 `Deep`（「否则 Deep」一支）。
            // `root_id` 与收束格的同名字段取同一个值（开轮事实的 event id），SLO 的
            // `declared_tier` 按它把收束格归回本幕；`budget_ms` 由档位数据面派生，两者
            // 不许各写一个数（`C5` 读的是这一格）。
            let tier = LatencyTier::Deep;
            source
                .append(
                    PRINCIPAL_MAIN.to_string(),
                    FactDraft {
                        key: format!("{EVENT_SCOPE_DISPATCH}:{}", item.id),
                        kind: EVENT_SCOPE_DISPATCH.to_string(),
                        entity: Entity::Scope,
                        verb: Verb::Asserted,
                        payload: json!({
                            "root_id": item.id,
                            "tier": tier.name(),
                            "budget_ms": tier.budget_ms(),
                            "principal": PRINCIPAL_MAIN,
                        }),
                        // 溯源指同一轮刚落的 `user.message`（seq 就是它的号）。
                        produced_by: Some(seq),
                    },
                )
                .map_err(|e| PluginError::InternalError(format!("scope.dispatch 落格失败: {e}")))?;

            // 轮首装配的收尾：`session.config`（系统提示词 + 工具表 + 本轮生效设置）。
            //
            // 写方是本插件（会话编排权归 session）：模型端口只读「最近一条 session.config」，
            // 读不到就退化为空提示词。提示词里的 VDFS 地址由收集期的 `descend_addr` 归一
            // （根名取自 vdfs 插件的装配期声明，不是本插件里的字面量），**不是**在这里把
            // 绝对路径再相对化。
            //
            // 能力视图只在本轮**装配**（系统提示词 / 工具表 / 分类开关）里读；收尾面
            // 只投影终答，不再补写它——`session.answer` 行在驱动内落格。
            let system_prompt = symbio_plugin_api::capability_prompts(&*manager)
                .await
                .into_iter()
                .map(|(_, prompt)| prompt)
                .filter(|prompt| !prompt.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            let twins = symbio_platform::sdk::capability::capability_scope_twins(&*manager).await;
            // 有效绑定 = 装配期收集到的端口名（ / ）：描述与执行两视图由
            // **同一次收集**得出，不维护第二份注册事实。冲突键不在 `bound()` 里——
            // 它们已在收集期落进错误桶（上面那句 `capability_errors` 会读到）。
            let bound_ports: std::collections::HashSet<String> = ctx
                .get(symbio_plugin_api::port::PORT_BINDINGS)
                .map(|bindings| bindings.bound().into_iter().collect())
                .unwrap_or_default();
            // 候选 = 显式暴露 ∩ 作用域 ∩ 有效绑定；协议命名（别名与碰撞）由模型插件的
            // 投影回答。投影缺席 ⇒ 本轮无工具：字符集规则不可用时不能把内部
            // canonical 名直接当线上名送出去。
            let candidates = candidate_tools(
                dedup_declared_twins(
                    manager
                        .list(symbio_platform::kernel::capability::family::TOOL)
                        .await
                        .into_iter()
                        .map(|row| row.meta())
                        .collect(),
                    &twins,
                ),
                &bound_ports,
            );
            let names: Vec<String> = candidates.iter().map(|meta| meta.name.clone()).collect();
            let projected = if names.is_empty() {
                Some(ToolProjectionView::default())
            } else if let Some(parent) = self.get_parent() {
                match project_tools(&parent, ctx, &names).await {
                    Ok(view) => Some(view),
                    Err(e) => {
                        symbio_plugin_api::plugin_warn!(
                            "session",
                            "model/project_tools 不可用：{e}"
                        );
                        None
                    }
                }
            } else {
                symbio_plugin_api::plugin_warn!(
                    "session",
                    "model/project_tools 不可用：没有父容器句柄"
                );
                None
            };
            let projection_unavailable = projected.is_none();
            let (tools, tool_aliases, alias_collisions) = match projected {
                Some(view) => {
                    let exposed: std::collections::HashSet<&str> =
                        view.exposed.iter().map(String::as_str).collect();
                    let tools = candidates
                        .into_iter()
                        .filter(|meta| exposed.contains(meta.name.as_str()))
                        .collect();
                    (tools, view.aliases, view.collisions)
                }
                None => (Vec::new(), Default::default(), Vec::new()),
            };
            // 收集过程的失败信号（traverse 侧记在 ctx 的桶里）：能力表为空/有错都要落 config，
            // 只是把原因写进载荷。**不造新事件类型**——模型行依赖的是 `session.config` 这条
            // 输入本身，它必须在每一轮都出现，否则模型行会永远等不到输入。
            let capability_errors = capability_take_errors(ctx).await;
            let tools_empty_reason = if tools.is_empty() {
                Some(if projection_unavailable {
                    "projection_unavailable"
                } else if !alias_collisions.is_empty() {
                    "wire_alias_collision"
                } else if capability_errors.is_empty() {
                    "no_tool_rows"
                } else {
                    "capability_error"
                })
            } else {
                None
            };
            // 当轮配置快照 = 会话基底格 + 能力包在开轮 traverse 里各自贡献的格。
            //
            // 两半的分工就是「谁的真源谁写」：`opening_*` / `progress_*` 的真源在本插件
            // （实现体也仍在本插件，归属裁决见 DE-055 ），
            // 故**在这里直接投影**——不绕一次广播，也就不依赖本插件 `traverse` 与这次
            // 写格的先后。能力开关的真源在能力包，本插件不认识它们的键名，只按 owner
            // 合并收集口给出的那一张表；不按插件名去债表查「谁在场」——卸载一个能力包，
            // 它的格子自然缺席，读方按自己的出厂值收敛（T3）。决策仍只读这一条开轮事实。
            let mut config_payload = serde_json::Map::new();
            config_payload.insert("system_prompt".to_string(), json!(system_prompt));
            config_payload.insert("tools".to_string(), json!(tools));
            // 本轮暴露快照的别名表（内部身份 → wire）：由模型插件的投影交出，
            // 清单、协议序列化、返回调用解析三处读同一份数据；碰撞组已在
            // 投影里整组退出清单，这里只记原因。
            config_payload.insert("tool_aliases".to_string(), json!(tool_aliases));
            config_payload.insert("tool_alias_collisions".to_string(), json!(alias_collisions));
            // 兜底表达：空表原因 / 收集错误明细（均为既有 config 载荷的字段，
            // 不是新事件类型）。非空表时前者为 null，后者为空数组。
            config_payload.insert("tools_empty_reason".to_string(), json!(tools_empty_reason));
            config_payload.insert(
                "capability_errors".to_string(),
                json!(capability_errors
                    .iter()
                    .map(|e| json!({ "plugin": e.plugin, "message": e.message }))
                    .collect::<Vec<_>>()),
            );
            {
                let cfg = self.config.read().await;
                config_payload.insert("opening_enabled".to_string(), json!(cfg.opening_enabled));
                config_payload.insert("progress_enabled".to_string(), json!(cfg.progress_enabled));
                config_payload.insert(
                    "progress_interval_ms".to_string(),
                    json!(cfg.progress_interval_ms),
                );
                config_payload.insert(
                    "progress_max_per_turn".to_string(),
                    json!(cfg.progress_max_per_turn),
                );
                config_payload.insert(
                    "progress_min_rounds".to_string(),
                    json!(cfg.progress_min_rounds),
                );
                config_payload.insert(
                    "progress_lease_ms".to_string(),
                    json!(cfg.progress_lease_ms),
                );
                config_payload.insert(
                    "progress_max_attempts".to_string(),
                    json!(cfg.progress_max_attempts),
                );
            }
            config_payload.extend(
                symbio_platform::sdk::capability::capability_config_snapshot(&*manager).await,
            );
            let config_draft = FactDraft {
                key: session_config_id(&item.id),
                kind: EVENT_SESSION_CONFIG.to_string(),
                entity: Entity::Scope,
                verb: Verb::Asserted,
                payload: serde_json::Value::Object(config_payload),
                // 溯源指同一轮刚落的 `user.message`（seq 就是它的号）。
                produced_by: Some(seq),
            };
            // 轮首召回：跨会话枚举 + 可见域收放（宿主侧隔离面）→ 检索面 → 一格读数。
            //
            // 段 → provider → 落点的实际收集路径（以后加段照抄这一条）：
            // 1. **本插件**在轮首取快照、按可见域收放（`recall_snapshots` 只在这里读，`pub(crate)`）；
            // 2. 把收放后的**切片值**交给 `retrieve_all`（能力包注册的 `Retriever` 折成读数）；
            // 3. 读数落一格事实（`session.recall`），不把切片写进 `session.config`；
            // 4. **能力包的 `ContextProvider`** 在 `model::turn_input` 收集时从 `req.facts`
            //    按名字读这一格并渲染段 → 进请求前缀。
            // `plugin-model` 不认识任何段名：加一段 = 加一个 provider + 它在轮首的那一格读数。
            //
            // **先于 `session.config` 落格**：config 是这一轮模型债的 `when`，而模型执行体
            // 只读事实。若 config 先落，驱动可能在读数落格之前就发出本轮请求——新会话首轮
            // 没有上一轮读数可借，请求包里就整段缺席。
            self.append_recall(session_id, source, &item.id, seq, &text)
                .await?;

            source
                .append(PRINCIPAL_MAIN.to_string(), config_draft)
                .map_err(|e| PluginError::InternalError(format!("session.config 落格失败: {e}")))?;

            let mut message = item.message;
            message.seq = None;
            let session = self.open_chat_session(session_id).await?;
            append_and_publish(
                &session,
                vec![message],
                Carrier::Runner,
                &self.active_mgr,
                session_id,
                PRINCIPAL_MAIN,
            )
            .await?;

            Ok(())
        }
        .await;
        result.map_err(|error| RoundOpenError {
            error,
            user_seq: Some(seq),
        })?;
        Ok(Some(seq))
    }

    /// 开轮之后的**收尾**：等写口踢出的驱动空闲 → 投影终答 → 把排队的补充
    /// 抽干成**下一轮**。
    ///
    /// 起后台任务而不是在写臂里等：写入的回执语义是「已经开轮」，不是「这一轮
    /// 已经跑完」。收尾也不靠时间——`settled` 等的是驱动自己的空闲通知
    /// （`DriveEntry::settled`），驱动跑到 `NoDebt` 才可能返回。
    ///
    /// ## 为什么抽干挂在这里（t19 的 A 线）
    ///
    /// 写臂在忙窗只入队、不开轮（`take_inbox_front` 的忙门）。没有批次边界的轮
    /// （无工具调用）结束后，排队条目只能由**后台消费者**接手；本任务就是那个
    /// 消费者：本轮收尾后抽一批、合并成一条、开下一轮，直到队列排空。
    ///
    /// 合并沿用 [`merge_supplements`]（`supplements_enabled = false` 时它退化取一条，
    /// 叠加后的行为与「一条消息 = 一轮」逐字一致）。
    pub(crate) fn spawn_round_closer(
        &self,
        session_id: String,
        user_seq: u64,
        source: Arc<dyn FactSource>,
        drive: Option<Arc<dyn DriveEntry>>,
        next_ctx: Option<Arc<dyn PluginInvokeRequest>>,
        opening_error: Option<String>,
    ) {
        let had_opening_error = opening_error.is_some();
        // 后台任务需要一个 `'static` 的自持句柄；`build` 期登记的弱自引用就是它。
        // 直接 `new` 出来的实例没有它（收不到自引用）——保留旧行为：只投影、不抽干。
        let Some(plugin) = self.self_ref.get().and_then(|weak| weak.upgrade()) else {
            let this = self.convergence.clone();
            tokio::spawn(async move {
                match this
                    .settle_round(
                        &session_id,
                        user_seq,
                        &*source,
                        drive.as_ref(),
                        opening_error,
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) if drive.is_none() => {
                        this.finish_session_state(
                            &session_id,
                            if had_opening_error {
                                super::OUTCOME_FAILED
                            } else {
                                super::OUTCOME_COMPLETED
                            },
                            None,
                        )
                        .await;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        symbio_plugin_api::plugin_warn!(
                            "session",
                            "收轮投影失败（{session_id}）：{error}"
                        );
                        if had_opening_error || drive.is_none() {
                            this.finish_session_state(
                                &session_id,
                                super::OUTCOME_FAILED,
                                Some(error.to_string()),
                            )
                            .await;
                        }
                    }
                }
            });
            return;
        };
        tokio::spawn(async move {
            let mut seq = user_seq;
            let mut opening_error = opening_error;
            loop {
                match plugin
                    .convergence
                    .settle_round(
                        &session_id,
                        seq,
                        &*source,
                        drive.as_ref(),
                        opening_error.take(),
                    )
                    .await
                {
                    Ok(true) => {}
                    Ok(false) if drive.is_none() => {
                        let state = plugin.active_mgr.get_or_create(&session_id).await;
                        plugin
                            .convergence
                            .emit_session_state(
                                &state,
                                SessionStateChange::Finished {
                                    outcome: if had_opening_error {
                                        super::OUTCOME_FAILED
                                    } else {
                                        super::OUTCOME_COMPLETED
                                    },
                                    error: None,
                                },
                            )
                            .await;
                        return;
                    }
                    Ok(false) => return,
                    Err(error) => {
                        symbio_plugin_api::plugin_warn!(
                            "session",
                            "收轮投影失败（{session_id}）：{error}"
                        );
                        if had_opening_error || drive.is_none() {
                            let state = plugin.active_mgr.get_or_create(&session_id).await;
                            plugin
                                .convergence
                                .emit_session_state(
                                    &state,
                                    SessionStateChange::Finished {
                                        outcome: super::OUTCOME_FAILED,
                                        error: Some(error.to_string()),
                                    },
                                )
                                .await;
                        }
                        return;
                    }
                }
                // 没有可信基底 ctx（`session/project` 路径）就开不了下一轮：不抽队，
                // 条目留给下一次写入按空闲路径处理。能力收集始终从它 fork 出 round_ctx。
                let Some(ctx) = next_ctx.as_ref() else {
                    return;
                };
                let state = plugin.active_mgr.get_or_create(&session_id).await;
                let batch = plugin.take_inbox_batch_when_idle(&state).await;
                let Some(message) = merge_supplements(&batch) else {
                    return;
                };
                let item = InboxItem {
                    id: message.id.clone(),
                    message,
                    params: batch[0].params.clone(),
                    workdir: batch[0].workdir.clone(),
                };
                match plugin
                    .consume_item_turn(ctx, &session_id, Arc::clone(&source), item, batch.clone())
                    .await
                {
                    Ok(Some(next)) => seq = next,
                    Ok(None) => return,
                    Err(e) => {
                        symbio_plugin_api::plugin_warn!(
                            "session",
                            "轮收尾抽干开轮失败（{session_id}）：{e}"
                        );
                        return;
                    }
                }
            }
        });
    }

    /// 轮首召回读数：可见切片 → 检索面 → 一格 `session.recall` 事实。
    ///
    /// 读数（而不是切片）落格：条目带着来源 `seq`、正文与认知标签，能力包的 provider
    /// 据此渲染段；别的会话的原始事件集不进本会话 WAL。
    async fn append_recall(
        &self,
        session_id: &str,
        source: &dyn FactSource,
        user_fact_id: &str,
        user_seq: u64,
        query: &str,
    ) -> Result<(), PluginError> {
        let store = self.get_store().await?;
        let viewer = store.session_principal(session_id);
        let snapshots = store.recall_snapshots(session_id);
        let visible = recall_visible_slice(snapshots, &viewer);
        let query = query.trim();
        let query = (!query.is_empty()).then_some(query);
        let Some(retrieval) = retrieve_all(&RetrievalRequest {
            events: &visible,
            query,
        }) else {
            return Ok(());
        };
        let draft = FactDraft {
            key: session_recall_id(user_fact_id),
            kind: crate::declarations::EVENT_SESSION_RECALL.to_string(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: json!({
                "degraded": retrieval.degraded,
                "entries": retrieval
                    .entries
                    .iter()
                    .map(|entry| json!({
                        "seq": entry.seq,
                        "content": entry.content,
                        "tag": entry.tag,
                    }))
                    .collect::<Vec<_>>(),
            }),
            produced_by: Some(user_seq),
        };
        source
            .append(PRINCIPAL_MAIN.to_string(), draft)
            .map_err(|e| PluginError::InternalError(format!("session.recall 落格失败: {e}")))?;
        Ok(())
    }

    /// 路由 `session/consume`：把队首落成事实，返回开轮事实的 `seq`。
    pub(crate) async fn handle_consume(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sid = require_sid(&ctx, "session/consume")?;
        let source = self.fact_source_for(&ctx, &sid).await?;
        let seq = self
            .consume_inbox_turn(&ctx, &sid, Arc::clone(&source))
            .await?;
        Ok(PluginPayload::new(&json!({ "seq": seq })))
    }

    /// 路由 `session/project`：把该开轮事实的终答投影回转写并收束。
    pub(crate) async fn handle_project(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sid = require_sid(&ctx, "session/project")?;
        let payload = ctx.payload::<Value>()?;
        let seq = payload
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| PluginError::ValidationError("session/project 需要 seq".into()))?;
        let source = self.fact_source_for(&ctx, &sid).await?;
        // 等的是**本根**那一份驱动（不是进程级主根入口）：CLI 与子智能体容器的
        // `session/project` 都必须等到自己这一根空闲。取不到（单测未装配）就不等。
        let drive = self.drive_entry();
        // 终答已在驱动内由 `session.answer` 行落格；这里只做幂等投影。
        let projected = self
            .convergence
            .project_final_turn(&sid, seq, &*source, drive.as_ref())
            .await?;
        Ok(PluginPayload::new(&json!({ "projected": projected })))
    }
}

/// 可见切片：跨会话快照 → **收放之后**的事件集（新近在前）。
///
/// 快照是存储层交回的值（枚举平级会话、mtime 新近优先、文件数与字节双预算都在
/// `SessionStore::recall_snapshots`）；这里只做隔离面那一步——逐条按读方主体收放，
/// 交出去的是**收放之后**的切片，检索面不需要、也拿不到「哪个会话是谁的」。
fn recall_visible_slice(snapshots: Vec<FleetSnapshot>, viewer: &str) -> Vec<Event> {
    let mut events: Vec<Event> = Vec::new();
    for snap in snapshots {
        events.extend(snap.events.into_iter().filter(|e| {
            let is_memory = e.entity == Entity::Memory
                && (e.kind == symbio_plugin_api::events::EVENT_MEMORY_ENCODED
                    || e.kind == symbio_plugin_api::events::EVENT_MEMORY_CONSOLIDATED);
            !is_memory || memory_visible(e, snap.own, viewer)
        }));
    }
    events.sort_by(|a, b| b.ts.cmp(&a.ts));
    events
}

/// 一条记忆事件对 `viewer` 可见吗——读侧预过滤的**唯一判定点**。
///
/// 本会话自己的内容恒可见；别的会话的 `shared` / `public` 走 core 的读侧闸
/// （`content_visible`，与 `stats` 同一份判据）。缺省（无 `vis_scope`）= 私有。
fn memory_visible(e: &Event, own: bool, viewer: &str) -> bool {
    use symbio_platform::sdk::governance::VisScope;
    match VisScope::of_event(e) {
        VisScope::ThreadPrivate => own,
        s => own || symbio_platform::sdk::authz::content_visible(s, viewer),
    }
}

#[cfg(test)]
#[path = "round.test.rs"]
mod tests;
