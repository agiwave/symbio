//! 阶段② 开轮——幕首建草稿（· B2，DE-021 §5 动作 B2）。
//!
//! [`open_turn`] 把 [`Assembled`] 摊成 [`TurnInput`] 交给 `TurnRunner::begin`，产
//! [`OpenRun`]（草稿 + 驱动它要的端口）。它一格「要等轮首那几行」的事实都不落。
//!
//! **本模块不再驱动环**（`D-222` 裁决 5）：控制面只有**一个**环，它住在
//! [`super::drive`]；幕首开轮只是那个环的一步（幕与幕在同一次调用里接续）。
//! 轮首四行、接缝、内环是同一张绑定表上的行，先后由 `prio` 表达。

use super::*;

/// 收幕 ⇒ 归类 / 落库的交付束：调用结果与桥、发射任务、落库还要用的一切。
///
/// **全部 owned**（`D-222` 裁决 5）：它是由幕的运行态（[`super::drive`] 的
/// `StageRuntime`）拆出来的，拆完那一幕就没了——借来的引用没有出借方可以活到归类。
///
/// `bridge` 与 `emitter` 按值走：收幕之后 `bridge.close()` 是发射端退出的**唯一**
/// 信号，必须正好剩最后一个强引用——所以幕内所有 `bridge.clone()` 都随注入闭包与
/// 分发方在收幕时死去，最后一个原样搬进这里、由归类消费。
pub(crate) struct Running {
    pub(crate) session: Arc<LoopSession>,
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) bridge: Arc<UiBridge>,
    pub(crate) emitter: tokio::task::JoinHandle<()>,
    pub(crate) outcome: super::super::turn_runner::TurnOutcome,
    pub(crate) produced: Vec<ChatNode>,
    pub(crate) derived: super::super::tool_dispatch::DerivedFacts,
    pub(crate) collected: Arc<std::sync::Mutex<Vec<ChatNode>>>,
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) principal: String,
    pub(crate) to: String,
    pub(crate) user_text: String,
    pub(crate) root_id: String,
    pub(crate) presentation_tree: symbio_core::node_build::PresentationTree,
}

/// [`open_turn`] 的出口：草稿与驱动它要的端口。
///
/// `elimination_revoked` 在开轮前取一次（与接线前同一时刻）：淘汰闸的部署级判据由
/// session 折叠成**一个值**（`D-086 §4`：全量事件过墙是 P6「值注入」的反面）。
pub(crate) struct OpenRun {
    pub(crate) session: super::TurnSession,
    pub(crate) llm: std::sync::Arc<dyn LlmAdapter>,
    pub(crate) tok: FullModel,
    pub(crate) elimination_revoked: bool,
}

/// 幕首开轮（[`TurnRunner::begin`]）：只建草稿，不落任何「要等轮首那几行」的格。
///
/// 首请求前的中止由派发器的第一圈守卫接住（[`super::drive`] 的
/// `abort_before_first_request`，落中止收束格、交回 [`super::DriveEnd::Finished`]）
/// ——它排在任何一行轮首行**之前**，所以被中止的幕不再跑判决 / 压缩 / 召回
/// （`D-222` 订正 3 认下的行为变化）。
///
/// 开轮即收束那一支（[`super::TurnSessionBegin::Done`]）在本路**按构造不可达**：
/// 它只由定稿答话轮（`final_reply`）触发，而本函数恒传 `None`。所以这里不给它
/// 一条安静的出口——真到了那里说明有人改坏了 `TurnInput`，按内部错误报出来。
pub(crate) async fn open_turn(a: &Assembled) -> Result<OpenRun, TurnExecExit> {
    // 深度档：完整推理（含工具内环）。快路的**判定**已经不在这一层：它由环上的
    // `policy.reflex` 行选中，出口格（`stage:"reflex"`）与正文随本轮状态到接缝；
    // 本侧只把正文交给收束动作，不自己落 `final`（D-163）。
    let llm: std::sync::Arc<dyn LlmAdapter> = std::sync::Arc::new(ProviderLlmAdapter::for_turn(
        a.provider.clone(),
        &a.system_prompt,
        a.abort.clone(),
    ));
    let tok = TokenIssuer::issue_deep();
    let input = TurnInput {
        turn: a.turn_no,
        user_seq: a.user_seq,
        drive_seq: a.drive_seq,
        window_turns: Some(a.window_turns),
        actor: a.actor.clone(),
        // 轮内注入（缺口 3）：用户中途说的话折进**本轮**。
        //
        // core 只知道「加消息」——抽干、合并、**落格**全在这里（插件侧）。
        // core 不该知道「收件箱补充」这个概念（队列、`merge_supplements`、
        // `InboxItem` 都在 `plugins/session`），而落格也不需要 core 参与：
        // 本函数自己就开着 `store`。
        //
        // 汇报（缺口 4）**不在这里**：批次边界那件事成了 `policy.report` 行，
        // 由同一个环按 `prio` 选中、由它自己的执行体落格上线（D-163）。闭包只留
        // 「抽干」这一件事——它仍然是 core 唯一的轮边界挂点。
        inject: injector(a.supplements.clone()).map(|drain| {
            let store = a.store.clone();
            let actor = a.actor.clone();
            let turn_no = a.turn_no;
            let supplemental_no = a.supplemental_no.clone();
            // 折进的消息往收集器 push（末尾随 `ExecTurnResult` 回灌进落库镜像）。
            let sink_msgs = a.collected.clone();
            std::sync::Arc::new(
                move |anchor: Option<u64>| -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Vec<symbio_core::PromptMessage>> + Send>,
                > {
                    // `async move` 块按值捕获，所以这里先各 clone 一份——
                    // 否则第一次调用就把捕获物搬走，闭包退化成 `FnOnce`。
                    let drain = drain.clone();
                    let store = store.clone();
                    let actor = actor.clone();
                    let sink_msgs = sink_msgs.clone();
                    let supplemental_no = supplemental_no.clone();
                    Box::pin(async move {
                        // 抽干口本身就是异步的（会话动作面），直接 await——
                        // 搬家前那套 `blocking_read` + `block_in_place` 的桥接
                        // 随队列一起留在了 session，这里不再需要判 runtime flavor。
                        let batch = drain().await;
                        supplement_into_request(
                            batch,
                            anchor,
                            store.as_ref(),
                            &actor,
                            turn_no,
                            &sink_msgs,
                            &supplemental_no,
                        )
                    })
                },
            ) as super::super::turn_runner::RoundInjector
        }),
        // 本路**恒**走生成：定稿答话轮（缺口 5）不经这里——它一个模型请求都不发，
        // 由 `execute_final_reply_turn` 单独入口落格（见该函数）。
        final_reply: None,
        final_cost_ms: 0,
        // 本路恒走生成 ⇒ 没有对话面节点（定稿答话轮不走这里）。
        dialog: None,
        // 落格身份：本轮 Turn 根 id（载荷 `turn_id`）+ 流式节点取数口。用户格
        // 已由编排层先落，运行器只收它的 seq。取数口是**快照闭包**：
        // 运行器每轮收束 / 进展 / 兜底时问一次「此刻桥上攒着哪两个节点」，
        // 中途正文与思考因此逐轮入格（半截中止时它们是唯一幸存者）。
        turn_root_id: a.root_id.clone(),
        nodes: Some(std::sync::Arc::new({
            let bridge = a.bridge.clone();
            move || {
                let (text_id, text) = bridge
                    .current_node()
                    .map(|(id, acc)| (Some(id), Some(acc)))
                    .unwrap_or((None, None));
                let (reasoning_id, reasoning) = bridge
                    .current_reasoning()
                    .map(|(id, acc)| (Some(id), Some(acc)))
                    .unwrap_or((None, None));
                super::super::turn_runner::StreamNodeFacts {
                    text_id,
                    text,
                    reasoning_id,
                    reasoning,
                }
            }
        }) as super::super::turn_runner::NodeFactsFn),
        // 中途中止信号（D-052 决策 1）：闸门与首请求之间的空隙由运行器
        // 接管——落完用户格、发第一个请求前检查，已中止则走中止收束格。
        abort: Some(a.abort.clone()),
    };
    // 淘汰闸的部署级判据：取失败按「未收回」继续并出声——与「读不出的 WAL 跳过」的
    // 既有口径一致。取值点与接线前一致：开轮之前（轮首那几行不参加这个判断）。
    let elimination_revoked = a
        .session
        .run_op::<bool>(run_ops::ELIMINATION_REVOKED, ())
        .await
        .unwrap_or_else(|e| {
            symbio_core::plugin_warn!(
                "session",
                "[Turn] 淘汰闸判据读取失败（按未收回处理，本轮继续）: {e}"
            );
            false
        });
    match super::TurnRunner::begin(a.store.as_ref(), input).await {
        // 落格失败与动作失败走**同一个出口**（`ActorError` 的语义，这里折成
        // `TurnExecExit::Failed`）：调用方只认这一条通道，不为「开轮」单开一条。
        Err(e) => Err(TurnExecExit::Failed(PluginError::InternalError(format!(
            "v2 运行器落格失败：{e:?}"
        )))),
        Ok(super::TurnSessionBegin::Done(_)) => {
            Err(TurnExecExit::Failed(PluginError::InternalError(
                "生成路径的开轮被判成「已收束」（定稿答话轮才走那一条）".into(),
            )))
        }
        Ok(super::TurnSessionBegin::Run(session)) => Ok(OpenRun {
            session,
            llm,
            tok,
            elimination_revoked,
        }),
    }
}

/// 轮边界注入的唯一挂点：抽干口（`None` = 本会话没装收件箱）。
///
/// 汇报不挂在这里：它是 `policy.report` 行（D-163），判定归行表、落格与上线归行
/// 自己的执行体。这里只剩「用户有没有中途插话」这一件 core 认得的事。
fn injector(supplements: Option<SupplementFn>) -> Option<SupplementFn> {
    supplements
}

/// 抽干结果 → **落格 + 构造注入消息**（缺口 3 的全部逻辑，闭包之外）。
///
/// 为什么抽出来：闭包在 P3c 里已经 90 行，且一半是「抽干 / 落格 / 造节点」的直线代码。
/// 缺口 4 要往同一个闭包里再加一段汇报逻辑，不抽出来的话那个闭包会奔到 200 行——
/// 而它的可读性靠的正是「一眼看到这一轮边界上依次发生了什么」。
///
/// 返回**空 vec** = 本轮无可注入（没有补充 / 正文为空 / 拿不到溯源锚点）。三种情形
/// 都**不消耗任何配额**、不留半截状态，与 `apply_verdict` 拿不到措辞时同一条方向。
#[allow(clippy::too_many_arguments)] // 抽出来之后参数就是它的全部依赖，多一个都该重新想
pub(crate) fn supplement_into_request(
    batch: Option<symbio_core::DrainedSupplement>,
    anchor: Option<u64>,
    store: &dyn Store<Event = Event>,
    actor: &ActorSpec,
    turn_no: u64,
    sink_msgs: &Arc<std::sync::Mutex<Vec<ChatNode>>>,
    no_counter: &std::sync::atomic::AtomicU64,
) -> Vec<symbio_core::PromptMessage> {
    let Some(drained) = batch else {
        return Vec::new();
    };
    let (text, count, ids) = (drained.text, drained.count, drained.ids);
    if text.trim().is_empty() {
        return Vec::new();
    }
    // **落格**：成为一格事实（`turn × asserted`）。不落格的话它只活在这一次请求里，
    // 下一轮又消失了——而「用户说过的话没有变成事实」正是缺口 3 本身。
    //
    // 事件 id 带序号（`s-{turn}-{no}`）：一次工具循环里可能折进多条，不带序号会撞
    // 幂等键（`Duplicate`）。
    //
    // 拿不到溯源锚点 ⇒ **不落也不注入**：I2 要求断言类事件带溯源，而无溯源的断言
    // 事件比不落更难查。
    let Some(anchor) = anchor else {
        symbio_core::plugin_warn!(
            "session",
            "[Turn] 补充无溯源锚点，本轮丢弃（见本函数「拿不到溯源锚点」处）"
        );
        return Vec::new();
    };
    let no = no_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Err(e) = store.append(
        symbio_core::Event::pending(
            format!("s-{turn_no}-{no}"),
            symbio_plugin_api::events::EVENT_TURN_SUPPLEMENTED,
            symbio_core::Entity::Turn,
            symbio_core::Verb::Asserted,
            turn_no,
            &actor.principal,
        )
        .with_produced_by(anchor)
        // 载荷带上**节点身份**（原则：节点身份是事实的一部分）：重建按 `id`
        // 复刻这条用户消息的身份、按 `ids` 复刻 `meta.supplement_ids`。缺了它们，
        // 重建只能合成 id（每次重开都换一批），且合并标记里的原始条目 id 无处可寻
        // ——而「合并后原始条目只此一处留痕」（见 `transcript::supplements` 的模块头）。
        .with_payload(serde_json::json!({
            "text": text,
            "count": count,
            "id": drained.id.clone(),
            "ids": ids.clone(),
        })),
    ) {
        // 落格失败**不静默**：消息仍会进这一次请求，但事实网格里没有它 ⇒ 下一轮起
        // 不可见。所以出声。
        //
        // `{:?}` 不是 `{e}`：`AppendError` 只 derive 了 `Debug`。
        symbio_core::plugin_warn!(
            "session",
            "[Turn] 补充落格失败（仍注入本轮，下一轮起不可见）: {e:?}"
        );
    }
    let msg = ChatNode {
        // 沿用第一条补充的 id（见 `DrainedSupplement::id` 的说明）
        id: drained.id.clone(),
        principal: Some(symbio_core::principal::Principal::from_legacy(
            symbio_core::PRINCIPAL_USER,
        )),
        content: Some(MessageContent::Text(text.clone())),
        meta: Some(serde_json::json!({
            symbio_core::META_SUPPLEMENT: true,
            symbio_core::META_SUPPLEMENT_COUNT: count,
            symbio_core::META_SUPPLEMENT_IDS: ids,
        })),
        ..Default::default()
    };
    // 锁拿不到就**跳过回灌**，不 panic：这条补充**已经落格、已经进了本轮请求**，
    // 用户看得见——丢的只是落库镜像里的一条（重启后少它）。为这个 panic 不值。
    if let Ok(mut sink) = sink_msgs.lock() {
        sink.push(msg);
    }
    vec![symbio_core::PromptMessage {
        role: "user".into(),
        text,
        tool_call_id: None,
        tool: None,
        tool_calls: None,
    }]
}
