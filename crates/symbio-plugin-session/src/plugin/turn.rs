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
use crate::convergence::{ComposeCall, SessionStateChange};
use crate::transcript::publish::append_and_publish;
use crate::transcript::supplements::merge_supplements;
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_kernel::exec::ExecAbortSignal;
use symbio_kernel::LatencyTier;
use symbio_plugin_api::capability_take_errors;
use symbio_plugin_api::debt::{session_config_id, session_recall_id};
use symbio_plugin_api::events::{EVENT_SESSION_CONFIG, EVENT_TURN_DISPATCH, EVENT_USER_MESSAGE};
use symbio_plugin_api::fact_source::FactSource;
use symbio_plugin_api::retrieval::{retrieve_all, RetrievalRequest};
use symbio_plugin_api::DriveEntry;
use symbio_plugin_api::FactDraft;
use symbio_sdk::authz::{PRINCIPAL_MAIN, PRINCIPAL_USER};
use symbio_sdk::turn::facts::FleetSnapshot;

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
        source: &dyn FactSource,
    ) -> Result<Option<u64>, PluginError> {
        let state = self.active_mgr.get_or_create(session_id).await;
        // 忙则留队：出队与开轮是同一件事的两半，忙窗里出队会开出第二条并发轮。
        let Some(item) = self.take_inbox_front(&state).await else {
            return Ok(None);
        };
        self.consume_item_turn(ctx, session_id, source, item).await
    }

    /// 把**一条已出队**的条目落成本轮（写臂与轮收尾抽干共用同一份落格）。
    ///
    /// 入队形态（`InboxItem`）是两条路径的共同语言：写臂按一条开轮，轮收尾把
    /// 一批补充合并成一条（`merge_supplements`）后同样以一条开下一轮。
    pub(crate) async fn consume_item_turn(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
        source: &dyn FactSource,
        item: InboxItem,
    ) -> Result<Option<u64>, PluginError> {
        let state = self.active_mgr.get_or_create(session_id).await;
        {
            let inner = state.inner.read().await;
            if inner.is_working() {
                return Ok(None);
            }
        }

        // 入站来源主体（D-067）：条目在**入队时**把它寄存进 `params`（写臂从请求
        // `metadata.source_principal` 取），这里在开轮前发布到会话槽——随后的
        // `append` 会把它装进本轮驱动目标，工具侧 `commitment.*` 的 `to` 据此取值。
        // 每轮都发布（含 `None`）：上一轮的来源不能漏进下一轮。
        self.get_store()
            .await?
            .set_source_principal(session_id, item.params.source_principal.clone());

        // 本轮的中止信号（`DE-047` 驱动级信号）：**登记必须早于第一次 `append`**——
        // 那次 append 就会踢驱动，而 `DriveTarget` 只带当时的当前值。信号对象由
        // 会话状态唯一持有，事实源只引用同一个 `Arc`；中止动作只置位、不取走，
        // 所以本轮后续派发（如工具行）读到的仍是**这个已取消的信号**，而不是新造的死信号。
        state.register_abort_signal(ExecAbortSignal::new());

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
            .map_err(|e| PluginError::InternalError(format!("user.message 落格失败: {e}")))?;

        // 幕级派发格（`D-223` / `D-282`）：`tier` 是档位的**唯一来源**，写侧在开轮处。
        // 快路（技能命中直接作答）随 `D-318 B2` 整条退场后，本幕没有第二条执行路径——
        // 档位折自「还能落到别处吗」的答案恒为 `Deep`（`D-262` 的「否则 Deep」一支）。
        // `root_id` 与收束格的同名字段取同一个值（开轮事实的 event id），SLO 的
        // `declared_tier` 按它把收束格归回本幕；`budget_ms` 由档位数据面派生，两者
        // 不许各写一个数（`C5` 读的是这一格）。
        let tier = LatencyTier::Deep;
        source
            .append(
                PRINCIPAL_MAIN.to_string(),
                FactDraft {
                    key: format!("{EVENT_TURN_DISPATCH}:{}", item.id),
                    kind: EVENT_TURN_DISPATCH.to_string(),
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
            .map_err(|e| PluginError::InternalError(format!("turn.dispatch 落格失败: {e}")))?;

        // 轮首装配：能力收集 → `session.config`（系统提示词 + 工具表 + 本轮生效设置）。
        //
        // 写方是本插件（会话编排权归 session）：模型端口只读「最近一条 session.config」，
        // 读不到就退化为空提示词。提示词里的 VDFS 地址由收集期的 `descend_addr` 归一
        // （根名取自 vdfs 插件的装配期声明，不是本插件里的字面量），**不是**在这里把
        // 绝对路径再相对化。
        //
        // 作用域按**会话自己的绑定**展开（metadata.agent_id）：收集器只把它当通用的
        // agent 作用域键递下去，agent 插件自行把子树 prompt rows / 工具加前缀——
        // 「人格 / 记忆」这些段名谁都不需要认识。
        let agent_scope = self.get_store().await?.session_agent_scope(session_id);
        let manager = crate::capabilities::collect_capabilities(
            self.get_parent().as_ref(),
            ctx,
            agent_scope.as_deref(),
        )
        .await;
        // 能力视图只在本轮**装配**（系统提示词 / 工具表 / 分类开关）里读；收尾面
        // 不再需要它：答话的生成往返由 compose 的 `model.assist` 行债驱动，回执从
        // 事实读面取，模板路径也不读能力视图。
        //
        // 作用域要**随轮进执行 ctx**（D-153）：收集器把它挂在**本轮** ctx 上，而工具
        // 执行跑在驱动新建的 ctx 里。由事实源随驱动目标带过去（`set_agent_scope`），
        // 工具执行体只读 `AGENT_PERMS`，不重新解析 manifest。
        self.get_store()
            .await?
            .set_agent_scope(session_id, symbio_sdk::agent::agent_perms_of(ctx.as_ref()));
        let system_prompt = symbio_sdk::capability::capability_prompts(&*manager)
            .await
            .into_iter()
            .map(|(_, prompt)| prompt)
            .filter(|prompt| !prompt.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let tools = manager
            .list(symbio_kernel::capability::family::TOOL)
            .await
            .into_iter()
            .map(|row| row.meta())
            .collect::<Vec<_>>();
        // 收集过程的失败信号（traverse 侧记在 ctx 的桶里）：能力表为空/有错都要落 config，
        // 只是把原因写进载荷。**不造新事件类型**——模型行依赖的是 `session.config` 这条
        // 输入本身，它必须在每一轮都出现，否则模型行会永远等不到输入。
        let capability_errors = capability_take_errors(ctx).await;
        let tools_empty_reason = if tools.is_empty() {
            Some(if capability_errors.is_empty() {
                "no_tool_rows"
            } else {
                "capability_error"
            })
        } else {
            None
        };
        let cfg = self.config.read().await.clone();
        // 答话措辞的**有效开关**：调用方开关 ∧ compose 本轮的债表在场（`debt` 族报到）。
        // 关掉 / 未挂载时，本插件不欠 `model.assist` 往返；模型行也据此知道「判决说
        // 能直接答，但没人能说话」——那一轮降级进工具循环，而不是沉默。
        let compose_enabled = cfg.compose_enabled
            && manager
                .list(symbio_plugin_api::debt::DEBT_FAMILY)
                .await
                .iter()
                .any(|row| row.name() == format!("<debt>{}", symbio_plugin_api::PLUGIN_ID_COMPOSE));
        // 分类开关的**有效值**：调用方开关 ∧ 分类插件本轮的债表在场（`debt` 族报到）。
        // 卸载（t21-E）时写 false——模型债行的 `after` 等的是判决行的债；一个不会出现
        // 的前置债会让模型行永远停在「等」，本轮一个请求都不发。
        let classify_enabled = cfg.classify_enabled
            && manager
                .list(symbio_plugin_api::debt::DEBT_FAMILY)
                .await
                .iter()
                .any(|row| {
                    row.name() == format!("<debt>{}", symbio_plugin_api::PLUGIN_ID_CLASSIFY)
                });
        let config_draft = FactDraft {
            key: session_config_id(&item.id),
            kind: EVENT_SESSION_CONFIG.to_string(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: json!({
                "system_prompt": system_prompt,
                "tools": tools,
                "classify_enabled": classify_enabled,
                // 首响开关（调用方开关；没有 compose 时行会因路由取不到措辞而失败、
                // 收敛为「不说话」，模型行照常放行）。模型债行按它决定是否等首响行。
                "opening_enabled": cfg.opening_enabled,
                // 答话措辞的**有效**开关：compose 的 `model.assist` 口债按它决定欠不欠
                // 一次生成往返；模型行也按它决定「answered 是否提前收手」。
                "compose_enabled": compose_enabled,
                "progress_enabled": cfg.progress_enabled,
                "progress_interval_ms": cfg.progress_interval_ms,
                "progress_max_per_turn": cfg.progress_max_per_turn,
                "progress_min_rounds": cfg.progress_min_rounds,
                // 债行的租约与尝试上界：策略数随 config 走，债行不写字面量。
                "progress_lease_ms": cfg.progress_lease_ms,
                "progress_max_attempts": cfg.progress_max_attempts,
                "readyset_cap": cfg.task_cap,
                "skill_compile_enabled": cfg.skill_compile_enabled,
                // 兜底表达：空表原因 / 收集错误明细（均为既有 config 载荷的字段，
                // 不是新事件类型）。非空表时前者为 null，后者为空数组。
                "tools_empty_reason": tools_empty_reason,
                "capability_errors": capability_errors
                    .iter()
                    .map(|e| json!({ "plugin": e.plugin, "message": e.message }))
                    .collect::<Vec<_>>(),
            }),
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

        self.convergence
            .emit_session_state(&state, SessionStateChange::Working)
            .await;
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
    pub(crate) fn spawn_turn_closer(
        &self,
        session_id: String,
        user_seq: u64,
        source: Arc<dyn FactSource>,
        drive: Option<Arc<dyn DriveEntry>>,
        compose: Option<ComposeCall>,
    ) {
        // 后台任务需要一个 `'static` 的自持句柄；`build` 期登记的弱自引用就是它。
        // 直接 `new` 出来的实例没有它（收不到自引用）——保留旧行为：只投影、不抽干。
        let Some(plugin) = self.self_ref.get().and_then(|weak| weak.upgrade()) else {
            let this = self.convergence.clone();
            tokio::spawn(async move {
                if let Some(entry) = &drive {
                    entry.settled(&session_id).await;
                }
                if let Err(e) = this
                    .project_final_turn(&session_id, user_seq, &*source, compose.as_ref())
                    .await
                {
                    symbio_plugin_api::plugin_warn!("session", "收轮投影失败（{session_id}）：{e}");
                }
            });
            return;
        };
        tokio::spawn(async move {
            let mut seq = user_seq;
            loop {
                if let Some(entry) = &drive {
                    entry.settled(&session_id).await;
                }
                if let Err(e) = plugin
                    .convergence
                    .project_final_turn(&session_id, seq, &*source, compose.as_ref())
                    .await
                {
                    symbio_plugin_api::plugin_warn!("session", "收轮投影失败（{session_id}）：{e}");
                    // 终态未定就不接着开下一轮：队列留给下一次写入按空闲路径处理。
                    return;
                }
                let state = plugin.active_mgr.get_or_create(&session_id).await;
                let batch = plugin.take_inbox_batch_when_idle(&state).await;
                let Some(message) = merge_supplements(&batch) else {
                    return;
                };
                // 没有能力视图（`session/project` 路径）就开不了下一轮：条目仍在队列里，
                // 下一次写入会按空闲路径开轮。
                let Some(call) = compose.as_ref() else {
                    return;
                };
                let item = InboxItem {
                    id: message.id.clone(),
                    message,
                    params: batch[0].params.clone(),
                    workdir: batch[0].workdir.clone(),
                };
                match plugin
                    .consume_item_turn(&call.ctx, &session_id, &*source, item)
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
        let sid = ctx.get(SESSION_ID).unwrap_or_default();
        if sid.is_empty() {
            return Err(PluginError::ValidationError(
                "session/consume 需要 metadata.session_id".into(),
            ));
        }
        let source = self.fact_source_for(&ctx, &sid).await?;
        let seq = self.consume_inbox_turn(&ctx, &sid, &*source).await?;
        Ok(PluginPayload::new(&json!({ "seq": seq })))
    }

    /// 路由 `session/project`：把该开轮事实的终答投影回转写并收束。
    pub(crate) async fn handle_project(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sid = ctx.get(SESSION_ID).unwrap_or_default();
        if sid.is_empty() {
            return Err(PluginError::ValidationError(
                "session/project 需要 metadata.session_id".into(),
            ));
        }
        let payload = ctx.payload::<Value>()?;
        let seq = payload
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| PluginError::ValidationError("session/project 需要 seq".into()))?;
        let source = self.fact_source_for(&ctx, &sid).await?;
        // 本路由不由开轮单元发起（拿不到本轮的能力视图），因此不在这里问 compose：
        // 带措辞的收口由开轮臂的 `spawn_turn_closer` 完成；这里只是幂等投影。
        let projected = self
            .convergence
            .project_final_turn(&sid, seq, &*source, None)
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
    use symbio_sdk::governance::VisScope;
    match VisScope::of_event(e) {
        VisScope::ThreadPrivate => own,
        s => own || symbio_sdk::authz::content_visible(s, viewer),
    }
}
