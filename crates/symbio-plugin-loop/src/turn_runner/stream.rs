//! 阶段② 请求 / 流式（· B3，DE-021 §5 动作 B3）。
//!
//! 一次工具循环迭代：拼请求消息 → 调模型 → 按 [`classify_round`] 分流。
//! 收束（`Halt`）与配置缺口（`ConfigGap`）在这里就地落格；派发（`Act`）把
//! 工具批交给 [`super::batch`]，等待用户的落进展格，其余继续下一轮。
//!
//! 状态只经 [`Prepared`] 在阶段间传递：每个动作写自己的那一半事实，派发器按行表
//! 决定下一个动作（`D-222` 裁决 5）。

use super::*;

/// 一次模型往返的失败：三类，各有各的收口。
///
/// 为什么要分类而不是直接抛 `AdapterError`：中止与失败是**两种停法**
/// （收束格 vs 兜底格），而控制面不一致（行不欠 / 端口没声明）**根本不是适配器的
/// 错**——把它们揉进一个错误类型，就只能给它们同一个形状。
pub(crate) enum ModelCallError {
    /// 适配器中止（`AdapterError::Aborted`）——中止收束格。
    Aborted,
    /// 适配器失败——兜底格（`detail` 是适配器的人话，只给人读）。
    Failed(String),
    /// 协议事实落不进去——往上传：事件都记不下来，后面的一切都不可信。
    Store(symbio_core::AppendError),
    /// 控制面自身不一致（行不欠 / 端口没声明）——模型没跑成，如实当失败报。
    Control(String),
}

/// 一次模型往返 = 端口 `model` 的一次调用。
///
/// 协议事实由 `port` 模块写（**唯一写方**）：
/// `request`（决策者：这一步欠一次模型往返）→ `sent`（引擎，跑**之前**）→
/// `response` / `failed`。
///
/// ## 为什么模型调用要落事实
///
/// `turn` 那一笔是**一整幕**：幕里发生过什么，只能从更里面的协议事实读出来。模型调用
/// 就是那层最小单位——落成事实之后，「上次发到第几次模型请求」有答案，而那一轮里的
/// 工具批次也能按它对齐。崩溃之后按 `(port, id, attempt)` 逐笔判（见 `port::reap`），
/// 已经回来的那几笔既不重开账也不重判。
///
/// ## 为什么逻辑 id 带**幕锚**与**步号**
///
/// 续写幕复用轮号且步号从 0 重来，只带轮号会与首幕撞成同一笔——首幕已经有过
/// `sent`，幂等键命中之后派发会被拒（与 的 `turn` 端口同一条理由）。
///
/// ## 为什么 id 以**端口名**开头
///
/// 视图 `in_flight` / `stale_sent` 的关账口径按 **id** 匹配，不带端口
/// （`open_calls`：`response` / `retry` 关同 id + attempt，`failed` 关同逻辑 id）
/// ——也就是说，**id 必须跨端口唯一**，否则一个端口的失败会关掉另一个端口的账。
/// 以端口名开头是最省心的做法：`turn-…` 与 `model-…` 天然不撞。
// 八个入参是「七个端口 + 这一笔的身份」：前七个各是一个不同的端口（草稿 / 事实源 /
// 适配器 / 令牌 / 消息 / 工具清单 / 流式出口），第八个是**这一笔往返的键**——它在
// 宣布那一步就定了（D-166），所以不能在这里重新生成。
#[allow(clippy::too_many_arguments)]
pub(crate) fn begin_model(
    run: &mut Prepared,
    store: std::sync::Arc<dyn Store<Event = Event>>,
    llm: std::sync::Arc<dyn LlmAdapter>,
    tok: FullModel,
    sink: std::sync::Arc<dyn symbio_core::DeltaSink>,
    tools: std::sync::Arc<[symbio_core::CapabilityMeta]>,
) -> Result<ModelBegin, ModelCallError> {
    // 没有交接件 ⇒ 这一笔不是本幕宣布的（上一世的遗留）。判死由调用方做
    // （[`kill_dangling_round`]）：那里看得见日志，这里只看得到草稿。
    let Some(call) = run.pending_round.take() else {
        return Ok(ModelBegin::NoPending);
    };
    let turn = run.turn;
    let drive = run.drive_seq;
    let produced_by = run.user_seq;
    // 基线 + 本轮交换。**拼接消息数组**：拼字符串会把角色拍平、丢掉 `tool_call_id`。
    let mut messages = run.base_messages.clone();
    messages.extend(run.exchange.iter().cloned());

    // 派出这一步的**唯一出口**是 `sent`（`D-222` 订正 2）：落完它，这一行的账就还清了，
    // 「在等」从下一圈起由 `await-model` 认领——于是两次选债之间「已派出、未收清」
    // 在事实面上读得到（`waiting` 视图 + `stale_sent`），而不是只在崩溃恢复的输入上才有值。
    super::super::port::begin(
        store.as_ref(),
        super::super::loop_rows::PORTS,
        super::super::port::Dispatch {
            call: &call,
            turn,
            produced_by,
            row: "dispatch-model",
            now: run.now_ms(),
            // 这次往返等在被寄存的 future 里：进程一死等待方就没了（判死归 `reap`）。
            awaited: false,
        },
    )
    .map_err(|e| ModelCallError::Control(format!("{e:?}")))?;

    let nodes_ok = run.nodes.clone();
    let nodes_failed = run.nodes.clone();
    let principal = run.actor.principal.clone();
    let turn_root_id = run.turn_root_id.clone();
    // 产出事实的**寄存槽**：桥的一次读只换一份读数，而这条事实要写进
    // `response` / `failed` 载荷，又要回到草稿的 `last_output_facts` /
    // `last_answer_id`。两处各取一次就是两次读数，取数口的消费方（测试的计数式
    // 假桥）会因此看到两倍的消耗——槽位让「读一次、存一次、两处共用」。
    let output_slot: std::sync::Arc<std::sync::Mutex<Option<(StreamNodeFacts, String)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let slot_ok = output_slot.clone();
    let slot_failed = output_slot.clone();
    let call_id = call.id.clone();
    let round_id = call.id.clone();

    // future 捕获的每一样都是 owned：槽不带生命周期参数（见 [`ModelInFlight`]）。
    let fut = async move {
        let answer = llm
            .generate_turn(&tok, &messages, tools.as_ref(), sink)
            .await;
        let result = match answer {
            Ok(rt) => {
                let facts = node_facts(&nodes_ok);
                let mut fields =
                    output_fields(&call_id, &turn_root_id, &principal, &facts, Some(&rt));
                let answer_id = fields["answer_id"].as_str().unwrap_or_default().to_string();
                *slot_ok.lock().unwrap_or_else(|e| e.into_inner()) = Some((facts, answer_id));
                // 这一轮交回的工具调用（集合展开读 `calls` 的 `id` / `name` / `arguments`）。
                let calls: Vec<serde_json::Value> = rt
                    .tool_calls
                    .iter()
                    .enumerate()
                    .map(|(idx, tc)| {
                        serde_json::json!({
                            "port": "tool",
                            "id": super::super::tool_dispatch::batch_call_id(
                                turn,
                                drive,
                                idx,
                                tc.id.as_deref(),
                                &round_id,
                            ),
                            "name": tc.name.clone().unwrap_or_default(),
                            "arguments": tc.arguments.clone(),
                        })
                    })
                    .collect();
                fields["calls"] = serde_json::Value::Array(calls);
                match super::super::port::succeed_with(
                    store.as_ref(),
                    &call,
                    turn,
                    produced_by,
                    fields,
                ) {
                    Ok(()) => Ok(rt),
                    Err(e) => Err(ModelCallError::Store(e)),
                }
            }
            Err(e) => {
                let facts = node_facts(&nodes_failed);
                let mut fields = output_fields(&call_id, &turn_root_id, &principal, &facts, None);
                let answer_id = fields["answer_id"].as_str().unwrap_or_default().to_string();
                *slot_failed.lock().unwrap_or_else(|e| e.into_inner()) = Some((facts, answer_id));
                // 停法进载荷：重建按它定节点状态，而 `failed` 本身不区分「用户按停」
                // 与「适配器报错」——那条区分只在这一格。
                fields["status"] = serde_json::Value::String(
                    if matches!(e, AdapterError::Aborted) {
                        "aborted"
                    } else {
                        "failed"
                    }
                    .to_string(),
                );
                // 失败也要留痕：写不进去不掩盖适配器自己的错误（它更有信息量）。
                let _ = super::super::port::failed_with_extra(
                    store.as_ref(),
                    &call,
                    turn,
                    produced_by,
                    &e.to_string(),
                    fields,
                );
                Err(match e {
                    AdapterError::Aborted => ModelCallError::Aborted,
                    other => ModelCallError::Failed(other.to_string()),
                })
            }
        };
        let output = output_slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        ModelFlight {
            id: call_id,
            result,
            output,
        }
    };
    Ok(ModelBegin::InFlight(Box::pin(fut)))
}

/// 一次**已派出、还没收清**的模型往返（`DE-015` 的 `D-222` 订正 2）。
///
/// ## 为什么是 `'static`
///
/// 它寄存进本轮草稿（[`super::TurnSession::inflight`]），而草稿**没有生命周期参数**：
/// 草稿是 `Ctx<'a>` 里被 `&'a mut` 借走的那一片，一旦它自己也带生命周期，
/// `Action = for<'a> fn(&'a mut Ctx<'a>)` 就得对**两个**生命周期成立——而绑定表
/// 要求这些函数指针 `'static`，全量化的只有 `'a` 一个。所以路只有一条：future 不借
/// 任何只在派出那一层栈帧里成立的引用。[`begin_model`] 因此把端口与消息全部搬成
/// owned（`Arc` 句柄 + `Vec<PromptMessage>`），槽里剩下的就是一个 `'static` 的框。
///
/// ## 它交出什么
///
/// [`ModelFlight`]：结论 + 这次往返的桥读数。**写回草稿的活不在这里**——future 跑完时
/// 已经回到 [`super::TurnSession`] 手里，`responses` / `last_output_facts` /
/// `last_answer_id` 由那一侧写（草稿的唯一写方仍是持有它的人）。
/// 控制面失败的兜底：模型**压根没派发**（行不欠 / 端口没声明）⇒ 日志上没有模型的终局
/// 事实，收束行开不出账，兜底格只能由手上拿着这一笔往返的那一方就地落（D-166）。
///
/// 从 [`begin_model`] 里分出来是因为它要 `await`：派出那一步是同步的（落 `sent` +
/// 寄存一个 future），而兜底要写一条事实。
pub(crate) async fn settle_control<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    why: &str,
) -> Result<TurnOutcome, symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let (cost_ms, produced) = failed_payload(run, llm);
    settle::append_fallback(
        store,
        &run.actor,
        run.turn,
        run.user_seq,
        why,
        cost_ms,
        produced,
        run.artifacts_landed,
    )
    .await
}

pub(crate) type ModelInFlight =
    std::pin::Pin<Box<dyn std::future::Future<Output = ModelFlight> + Send + 'static>>;

/// 一次往返落地时交回的两样。
pub(crate) struct ModelFlight {
    /// 这一笔的逻辑 id（写回 `responses` 用）。
    pub(crate) id: String,
    /// 适配器的结论；`Err` 三档与 [`ModelCallError`] 同义。
    pub(crate) result: Result<symbio_core::LlmTurn, ModelCallError>,
    /// 这一次往返的桥读数（`last_output_facts` / `last_answer_id` 的来源）。
    ///
    /// `None` = 没有落输出事实（控制面失败 / 存储失败），此时**不能**把 id 记进
    /// `responses`——读侧会去查一条不存在的正文。
    pub(crate) output: Option<(StreamNodeFacts, String)>,
}

/// 在飞的一批工具调用（`D-124` 订正 1）：派出与收尾两半之间的**唯一**交接件。
///
/// 与 [`ModelInFlight`] 同一条理由活着：两次选债之间它在草稿里挂着，`sent` 与终局
/// 之间那一圈因此可以横跨两个环步。`rt` 与 `calls` 一并寄存——收尾半要用前者算
/// 正文载荷、用后者按 `call_id` 关账，而派发器不替它记这两样。
pub(crate) struct ToolFlight {
    /// 这一批对应的那一次模型往返（唤醒幕是合成的那一份）。
    pub(crate) rt: symbio_core::LlmTurn,
    /// 开账过的调用（`sent` 标记在里面，关账按它逐笔判）。
    pub(crate) calls: Vec<super::super::tool_dispatch::ToolPortCall>,
    /// 执行层那一段：`'static` 的框，收尾半 poll 它。
    pub(crate) fut: super::super::tool_dispatch::ToolInFlight,
}

/// 派出的结果：三态。
pub(crate) enum ModelBegin {
    /// 已派出（`sent` 落了），future 在这里。
    InFlight(ModelInFlight),
    /// 草稿里没有交接件：这一笔不是本幕宣布的（上一段生命在宣布与派出之间结束）。
    NoPending,
}

/// 一次模型往返的推进结果。
pub(crate) enum ModelRound {
    /// 本轮已收口（兜底 / 中止 / 控制面失败——都没有可收束的应答）。
    Done(TurnOutcome),
    /// 模型请求了工具；应答与节点事实已寄存在 `Prepared`，等 [`tool_round`]。
    Tools,
    /// 本轮该收束了；**结局**寄存在 [`Prepared::closing`]，落格归 `close-turn`
    /// 行的执行体（/ 步 5c）。
    ///
    /// 三次收法共用这一个出口：答完了、模型没答成、中止——它们落的格子不同，
    /// 但**都是「这一轮该收了」**，而收束格只由 `close` 执行体写。
    Close,
    /// 这一笔不是本幕宣布的（草稿里没有交接件）：上一段生命在宣布与派出之间结束，
    /// 请求留在日志上没人接。调用方据此把它判死（[`kill_dangling_round`]）。
    NoPending,
}

/// 阶段②a-0：为**下一次**模型往返开账（`request(port=model)`），把这一笔留在草稿里。
///
/// 派发不在这里（D-166）：「宣布」与「派出」是两个环步。宣布之后那笔账在日志上是
/// **开着的**（`dispatch-model` 行因它而欠），派出由 [`begin_model`] 在另一个动作里
/// 做——这样「这一笔该谁跑」在事实面上可判，而不由某段代码顺手调一次往返决定。
///
/// 逻辑 id 的取法与 [`begin_model`] 的头注同源（端口名开头 + 幕锚 + 步号）：步号在
/// 派出之后才 +1，所以一次往返只有一条 `request`，第二次不会撞上第一次的幂等键。
pub(crate) async fn announce_round<S>(
    run: &mut Prepared,
    store: &S,
) -> Result<(), symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let call = super::super::port::Call::new(
        "model",
        format!("model-{}-d{}-s{}", run.turn, run.drive_seq, run.step),
        1,
    );
    super::super::port::announce(store, &call, run.turn, run.user_seq)?;
    run.pending_round = Some(call);
    Ok(())
}

/// 把一笔**没有等待方**的模型请求判死（`failed`，`why = "no_pending_round"`）。
///
/// 判据与取法全在 [`super::super::port::kill_dangling`]：这里只把端口名与理由定死。
pub fn kill_dangling_round<S>(store: &S) -> Result<bool, symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    super::super::port::kill_dangling(store, "model", "no_pending_round")
}

/// 收**已派出**的那一次模型往返：**poll 一次**在飞的 future。
///
/// 与 [`begin_model`] 成对（`D-222` 订正 2）：派出与收应答分处两个环步，中间那一圈
/// 「`sent` 有、`response` 无」就是**在等**——`waiting` 视图与 `stale_sent` 读的正是它。
/// 拆之前两者在同一个动作窗口里，`waiting` 只在崩溃恢复的输入上才有值。
///
/// ## 为什么是 poll 一次，不是 `await` 到底
///
/// 「等」这个动作返回的那一刻，环要能回到**选债口**：往返在飞的时候，别的行（中途汇报
/// 就是一条）才轮得到。`await` 到底等于把这一段重新塞回一个动作窗口里，只是把窗口挪了
/// 个位置——`D-163` 要的「汇报在往返**进行中**落地」反过来又做不到了。
///
/// 于是这里只推进一次：`Ready` ⇒ 落账并分类（[`settle_flight`]）；`Pending` ⇒ 原样放回
/// 槽里、让出执行权。**「让出」不是客套**：poll 一次之后那一圈在同一个任务里同步转完，
/// 不 `yield` 就永远不会把控制权还给执行器，在飞的 I/O 因此永远等不到被驱动——
/// 那不是「在等」，是自旋。
///
/// ## 槽空 ⇒ 判死，不猜
///
/// 进程在 `sent` 与 `response` 之间死过：future 随进程没了，留下来的只有请求格。
/// 重放没有对象（等待方是那一段栈帧），所以按与「上一世遗留的请求」同一条处置：
/// [`kill_dangling_round`] 落 `failed`，那一笔账关上。
pub(crate) async fn poll_flight<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    inflight: Option<ModelInFlight>,
    has_dispatch: bool,
) -> Result<PolledRound, symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let Some(mut fut) = inflight else {
        kill_dangling_round(store)?;
        return Ok(PolledRound::Settled(ModelRound::NoPending));
    };
    let ready = std::future::poll_fn(|cx| std::task::Poll::Ready(fut.as_mut().poll(cx))).await;
    match ready {
        std::task::Poll::Ready(flight) => Ok(PolledRound::Settled(
            settle_flight(run, store, llm, flight, has_dispatch).await?,
        )),
        // 还没好：原样放回槽里，并且让出执行权（理由见上）。
        std::task::Poll::Pending => {
            tokio::task::yield_now().await;
            Ok(PolledRound::Parked(fut))
        }
    }
}

/// 一次 poll 的结果：落地了，还是「还在等」。
pub(crate) enum PolledRound {
    /// 这一笔已经有终局，附分类结果。
    Settled(ModelRound),
    /// 还在路上：future 原样交回，下一圈接着 poll。
    Parked(ModelInFlight),
}

/// 一次往返的 future 已经 `Ready`：写回草稿 → 分类。
///
/// ## 写回草稿的活在这里
///
/// future 里只跑适配器与落协议格；`responses` / `last_output_facts` /
/// `last_answer_id` 由**持有草稿的这一方**写——草稿的写方仍只有一个人。
pub(crate) async fn settle_flight<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    flight: ModelFlight,
    has_dispatch: bool,
) -> Result<ModelRound, symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    // 落了输出事实的两种情形才把 id 记进 `responses`（读侧按它查正文）：
    // 应答落 `response`，中止 / 适配器失败落 `failed`——两条都带产出载荷。
    // 存储失败与控制面失败**没有**落任何输出事实，记进去就会查到一条不存在的正文。
    let has_output = flight.result.is_ok()
        || matches!(
            &flight.result,
            Err(ModelCallError::Aborted) | Err(ModelCallError::Failed(_))
        );
    if has_output {
        run.responses.push(flight.id);
        if let Some((facts, answer_id)) = flight.output {
            run.last_output_facts = facts;
            run.last_answer_id = answer_id;
        }
    }
    let rt = match flight.result {
        Ok(rt) => rt,
        // 中止：收束格（`status: "aborted"`）由收束动作落，这里只寄存结局。
        Err(ModelCallError::Aborted) => {
            let (cost_ms, produced, text) = aborted_payload(run, llm);
            run.closing = Some(PendingClose::Aborted {
                cost_ms,
                produced,
                text,
            });
            return Ok(ModelRound::Close);
        }
        // 适配器失败：兜底格由收束动作落，这里只寄存结局。
        Err(ModelCallError::Failed(why)) => {
            let (cost_ms, produced) = failed_payload(run, llm);
            run.closing = Some(PendingClose::Failed {
                detail: why,
                cost_ms,
                produced,
            });
            return Ok(ModelRound::Close);
        }
        Err(ModelCallError::Store(e)) => return Err(e),
        // 控制面自身不一致只可能在**派出**那一步（[`begin_model`] 的 `port::begin`），
        // 那一路由它就地落兜底格。这里接住它是为了让「加变体必须在这里回答一次」
        // 这件事留在派发器上，而不是等到编译不过才发现。
        Err(ModelCallError::Control(why)) => {
            let (cost_ms, produced) = failed_payload(run, llm);
            return settle::append_fallback(
                store,
                &run.actor,
                run.turn,
                run.user_seq,
                &why,
                cost_ms,
                produced,
                run.artifacts_landed,
            )
            .await
            .map(ModelRound::Done);
        }
    };
    run.cost_ms += rt.cost_ms;
    // 步号 +1：下一次模型往返换一个逻辑 id（否则第二次的 `sent` 撞第一次的幂等键）。
    run.step += 1;
    // 每轮的流式节点事实在**分发之前**取定：工具分发的定稿会重置桥，
    // 之后再取只拿得到下一轮的节点——本轮的中途正文 / 思考就静默丢了。
    //
    // 取数已经发生在应答那一侧（[`begin_model`] 的寄存槽）：桥只读一次。
    let round_facts = run.last_output_facts.clone();

    // 本轮不再请求工具 ⇒ 该收束了。**不在这里落收束格**：
    //     `close-turn` 行的前件（`turn.closeable`）此刻已经在日志上，派发器下一步
    //     就会选中它，把收束交给 `closer` 执行体。
    if classify_round(&rt, has_dispatch) == RoundDecision::Halt {
        run.closing = Some(PendingClose::Answer {
            rt,
            facts: round_facts,
        });
        return Ok(ModelRound::Close);
    }

    // 模型请求了工具 ⇒ 寄存。没有分发方却收到工具调用是**配置缺口**
    //     （无工具轮不该出现工具调用），按失败诚实回报。
    //
    //     这一格**就地落**（不走收束动作）：工具批开不出来这件事在日志上没有
    //     任何事实（`failed(port=model)` 落不下来——模型应答是好的，缺的是分发
    //     通道），收束行因此判不出「该收束了」；兜底格写进 `waiter` 的
    //     `decl.writes`（`ACTOR_WAITER`）。生产里这一路不可达（模型调用点恒带分发通道）。
    if !has_dispatch {
        let cost = run.started.elapsed().as_millis() as u64;
        // 这一轮没有可派的工具批：正文归刚才那条 `response` 事实，这里只记结局。
        let produced = produced_payload(
            "",
            llm.model_id(),
            &run.turn_root_id,
            None,
            &run.last_answer_id,
            &run.responses,
        );
        return settle::append_fallback(
            store,
            &run.actor,
            run.turn,
            run.user_seq,
            "模型请求了工具，但本轮没有工具分发通道",
            cost,
            produced,
            run.artifacts_landed,
        )
        .await
        .map(ModelRound::Done);
    }

    run.pending_model = Some(rt);
    Ok(ModelRound::Tools)
}

/// 阶段②c：收束寄存的**结局**（/ 步 5c，`close-turn` 行的动作）。
///
/// 前件由日志说（本幕要么「答完了」、要么「模型没答成」），落格在这里做。
/// 没有寄存 ⇒ 引擎收错了账：不落任何格、返回 `None`，让派发器的「无进展」判据把
/// 它显形——不拿一份来历不明的草稿硬造一条收束。
pub(crate) async fn close_round<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    sink: std::sync::Arc<dyn symbio_core::DeltaSink>,
) -> Result<Option<TurnOutcome>, symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let Some(pending) = run.closing.take() else {
        return Ok(None);
    };
    match pending {
        PendingClose::Answer { rt, facts } => halt(run, store, llm, rt, &facts).await.map(Some),
        PendingClose::Failed {
            detail,
            cost_ms,
            produced,
        } => settle::append_fallback(
            store,
            &run.actor,
            run.turn,
            run.user_seq,
            &detail,
            cost_ms,
            produced,
            run.artifacts_landed,
        )
        .await
        .map(Some),
        PendingClose::Aborted {
            cost_ms,
            produced,
            text,
        } => land_aborted(run, store, cost_ms, produced, &text)
            .await
            .map(Some),
        PendingClose::Reflex { text } => land_reflex(run, store, &text, &sink).await.map(Some),
    }
}

/// 快路的收束（D-163）：技能正文**直落** `chat.assistant.final`，并上桥。
///
/// 与模型应答的收束同一份纪律：正文先经 `sink` 上线（否则前端建不出正文节点），
/// 写侧闸不允许就说「不入格」并落否定事实，返回 `closure_denied`。载荷的
/// `model` 记 `reflex`——这一轮没有模型，但「这段话是谁产的」仍要可观测。
async fn land_reflex<S>(
    run: &Prepared,
    store: &S,
    text: &str,
    sink: &std::sync::Arc<dyn symbio_core::DeltaSink>,
) -> Result<TurnOutcome, symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    sink.on_delta(text);
    let cost_ms = run.started.elapsed().as_millis() as u64;
    if let Err(why) = closure_gate(&run.actor, ClosureKind::Speech) {
        deny(
            store,
            &run.actor,
            run.turn,
            run.user_seq,
            ClosureKind::Speech,
            EVENT_CHAT_ASSISTANT_FINAL,
            &why,
        );
        symbio_core::plugin_warn!(
            "actors",
            "[v2] 快路收束被授权拒绝（{why}），本轮不入格（turn={}）",
            run.turn
        );
        return Ok(TurnOutcome {
            turn: run.turn,
            text: text.to_string(),
            cost_ms,
            usage: None,
            fell_back: false,
            aborted: false,
            awaits_user: false,
            closure_denied: true,
            finish: symbio_core::ModelFinishReason::Stop,
        });
    }
    settle::land_final(
        store,
        &run.actor,
        run.turn,
        run.user_seq,
        cost_ms,
        // 与深度档同一个产出形状（`produced_payload`）+ 状态字段：快路无模型往返、
        // 无流式节点，正文只能随收束格入格；`turn_id` 记空串（重建侧按事件 id 稳定合成）。
        with_reason(
            with_status(
                produced_payload(
                    text,
                    LatencyTier::Reflex.name(),
                    "",
                    None,
                    &answer_id_of(text, "", &StreamNodeFacts::default(), ""),
                    &[],
                ),
                "completed",
            ),
            symbio_core::ClosureReason::Completed,
        ),
    )?;
    Ok(TurnOutcome {
        turn: run.turn,
        text: text.to_string(),
        cost_ms,
        usage: None,
        fell_back: false,
        aborted: false,
        awaits_user: false,
        closure_denied: false,
        // 快路没有模型轮，长度戳判据对它无意义（技能正文一次写完）。
        finish: symbio_core::ModelFinishReason::Stop,
    })
}

/// 阶段②b-1：创建（`open-tools` 行的动作， 步 5b）。
///
/// 把模型交回的调用逐条开成 `request(port=tool)`，开出的账寄存在草稿里交给授权步。
/// 没有寄存的模型应答 ⇒ 引擎选错了行：不写任何格，派发器的「无进展」判据会显形。
pub(crate) async fn open_round(
    run: &mut Prepared,
    store: &dyn Store<Event = Event>,
    dispatch: &dyn super::super::tool_dispatch::ToolDispatch,
) {
    let Some(rt) = run.pending_model.as_ref() else {
        return;
    };
    let calls = dispatch.open(store, rt).await;
    run.pending_calls = Some(calls);
}

/// 阶段②b-2：授权（`authorize-tool` 行的动作）：为已开账的调用落放行 / 拒绝。
pub(crate) async fn authorize_round(
    run: &mut Prepared,
    store: &dyn Store<Event = Event>,
    dispatch: &dyn super::super::tool_dispatch::ToolDispatch,
) {
    match run.pending_calls.as_mut() {
        Some(calls) => dispatch.authorize(store, calls).await,
        None => {
            // 恢复幕：草稿里没有模型批次，动作自己去读开幕前列队的唤醒请求。
            let mut none: Vec<super::super::tool_dispatch::ToolPortCall> = Vec::new();
            dispatch.authorize(store, &mut none).await;
        }
    }
}

/// 阶段②b-3：派发（`dispatch-tools` / `deny-tools` 两行的动作）：落 `sent`、
/// 跑执行层、关账，落产物 / 等待 / 交换。
///
/// `Ok(None)` = 继续下一轮；`Ok(Some)` = 本轮收束（等待用户）。
///
/// 两行共用这一个动作：混合批次里 `dispatch-tools` 先被选中，它顺手把被拒的那几笔
/// 也收尾（`close_batch`）；一批全被拒时 `dispatch-tools` 不欠，`deny-tools` 选中
/// 本动作，仍然要走执行层（审批卡 / 拒绝反馈由它唯一构造）。
/// 阶段②b-3 的**派出半**（`dispatch-tools` 行的动作，`D-124` 订正 1）：落 `sent`、
/// 把「跑执行层 + 收结果」整段寄存进草稿的 in-flight 槽。
///
/// 与 [`land_round`] 成对（与 `model` 端口的 [`begin_model`] / [`poll_flight`] 逐字同形）：
/// 派出与收应答分处两个环步，中间那一圈「`sent` 有、终局无」在**正常跑**里可见——
/// `waiting` 视图、`stale_sent` 与 `reap` 读的都是它。
///
/// 这一半只做「交出去」：定格正文、广播工具节点、落 `sent`。批次真正落地的东西
/// （产物 / 等待进展 / 轮内注入 / 批次边界汇报）全在 [`land_batch`]。
pub(crate) async fn begin_round(
    run: &mut Prepared,
    store: &std::sync::Arc<dyn Store<Event = Event>>,
    dispatch: std::sync::Arc<dyn super::super::tool_dispatch::ToolDispatch>,
) -> Option<ToolFlight> {
    // 两格齐 ⇒ 正常批次；两格皆空 ⇒ 可能是一笔唤醒（恢复幕没有模型应答）。
    // 只缺一格是草稿错位，不拿唤醒兜底掩盖它。
    let batch = match (run.pending_model.take(), run.pending_calls.take()) {
        (Some(rt), Some(calls)) => Some((rt, calls)),
        _ => dispatch
            .wake(store.as_ref())
            .await
            .map(|batch| (batch.turn, batch.calls)),
    };
    // 没有寄存、也没有唤醒请求 = 引擎没有可派的工具批；不空转。
    let (rt, mut calls) = batch?;
    match dispatch.clone().begin(store.clone(), &rt, &mut calls).await {
        // 草稿错位（`begin` 没找到可派的批次）：一格没落，派发器的空账本守卫会显形。
        super::super::tool_dispatch::ToolBegin::None => None,
        super::super::tool_dispatch::ToolBegin::InFlight(fut) => {
            Some(ToolFlight { rt, calls, fut })
        }
    }
}

/// 阶段②b-3 的**收尾半**（`await-tools` / `deny-tools` 两行的动作）：poll 一次在飞的
/// 那一批，落地了就关账并落产物 / 等待格。
///
/// ## 为什么只 poll 一次
///
/// 与 `model` 端口同一个理由（`D-222` 订正 2）：返回的那一刻环要能回到选债口，
/// 往返在飞的时候别的行才轮得到。`await` 到底等于把派出与收应答又合回一个动作窗口。
///
/// 还在跑就把 future 原样放回槽里、**让出执行权**（不 yield 就是自旋：那一圈在同一个
/// 任务里同步转完，执行层的 I/O 永远等不到驱动）。
///
/// ## 槽空的两种处境
///
/// 1. **这一批没有跨边界的往返**（一批全被拒 / 唤醒的用户结果）：结论本地可算，
///    起批之后跑到完——那一条路上没有「在等」这一圈可言；
/// 2. **等待方随进程没了**（日志上有 `sent`、草稿里没有批次）：按与
///    [`kill_dangling_round`] 同一条处置判死，不猜、不重放。
pub(crate) async fn land_round(
    run: &mut Prepared,
    flight: Option<ToolFlight>,
    store: &std::sync::Arc<dyn Store<Event = Event>>,
    llm: &dyn LlmAdapter,
    dispatch: std::sync::Arc<dyn super::super::tool_dispatch::ToolDispatch>,
) -> Result<Option<ToolFlight>, symbio_core::AppendError> {
    let Some(flight) = flight else {
        let batch = match (run.pending_model.take(), run.pending_calls.take()) {
            (Some(rt), Some(calls)) => Some((rt, calls)),
            _ => dispatch
                .wake(store.as_ref())
                .await
                .map(|batch| (batch.turn, batch.calls)),
        };
        let Some((rt, mut calls)) = batch else {
            // 没有批次可起、也没有在飞的 future：等待方随进程没了。
            super::super::port::kill_dangling_sent(store.as_ref(), "tool", "no_pending_batch")?;
            return Ok(None);
        };
        let super::super::tool_dispatch::ToolBegin::InFlight(fut) =
            dispatch.clone().begin(store.clone(), &rt, &mut calls).await
        else {
            return Ok(None);
        };
        // 这一条路没有真正的等待：起批之后直接跑到完（结论本地可算）。
        let outcomes = fut.await;
        dispatch.finish(store.as_ref(), &calls, &outcomes);
        land_batch(run, store.as_ref(), llm, rt, outcomes).await?;
        return Ok(None);
    };

    let ToolFlight { rt, calls, mut fut } = flight;
    let ready = std::future::poll_fn(|cx| std::task::Poll::Ready(fut.as_mut().poll(cx))).await;
    match ready {
        std::task::Poll::Pending => {
            // 让出执行权：这一圈没写任何格，派发器的空账本守卫据 `sent` 无所终局
            // 认出这是**合法地不前进**，不判 `no_progress`。
            tokio::task::yield_now().await;
            Ok(Some(ToolFlight { rt, calls, fut }))
        }
        std::task::Poll::Ready(outcomes) => {
            // 关账先于落格：`response` / `failed` 是端口账的终局，产物与等待格是业务
            // 事实——两者在同一圈里落，次序按「账先关，事再写」。
            dispatch.finish(store.as_ref(), &calls, &outcomes);
            land_batch(run, store.as_ref(), llm, rt, outcomes).await?;
            Ok(None)
        }
    }
}

/// 一批工具的结果**落地**：产物格、等待进展、交换累加、轮内注入。
///
/// 四段的写点都在「结果已经在手上」之后，与「派出去」那一刻无关，所以归收尾半
/// （`await-tools` 那一行的动作，§4.1）。
async fn land_batch(
    run: &mut Prepared,
    store: &dyn Store<Event = Event>,
    llm: &dyn LlmAdapter,
    rt: LlmTurn,
    outcomes: Vec<super::super::tool_dispatch::DispatchOutcome>,
) -> Result<(), symbio_core::AppendError> {
    // 产物落格：每条工具结果一格，溯源指向本轮用户格。
    batch::land_artifacts(run, store, &outcomes).await?;

    // 收束于等待用户 ⇒ 落**进展格**（`status: "pending"`：模型已
    //     说过的话照样是事实，等待是状态），不收束轮次。
    if outcomes.iter().any(|o| o.needs_user_action) {
        // 模型轮：正文归那一轮的 `response` 事实（`responses` 回溯得到），这里只
        // 记结局。唤醒幕（草稿里没有模型往返）没有可回溯的事实，正文取唤醒批自带的。
        let produced = if run.responses.is_empty() {
            let facts = StreamNodeFacts::default();
            let answer_id = answer_id_of(&rt.text, "", &facts, &run.turn_root_id);
            produced_payload(
                &rt.text,
                llm.model_id(),
                &run.turn_root_id,
                None,
                &answer_id,
                &run.responses,
            )
        } else {
            produced_payload(
                "",
                llm.model_id(),
                &run.turn_root_id,
                None,
                &run.last_answer_id,
                &run.responses,
            )
        };
        // 进展格的事件 id 按**本轮已有进展格数**编号（同产物格的幂等
        // 模式）：等待轮可能已落过进展格，从 0 起会撞幂等键。
        let progress_no = run
            .full_snapshot
            .iter()
            .filter(|e| e.turn == run.turn && e.kind == EVENT_CHAT_ASSISTANT_PROGRESS)
            .count() as u64;
        store.append(
            Event::pending(
                format!("p-{}-{progress_no}", run.turn),
                EVENT_CHAT_ASSISTANT_PROGRESS,
                Entity::Turn,
                Verb::Progressed,
                run.turn,
                &run.actor.principal,
            )
            .with_produced_by(run.user_seq)
            .with_cost_ms(run.cost_ms)
            .with_payload(with_status(produced, "pending")),
        )?;
        run.settled = Some(TurnOutcome {
            turn: run.turn,
            text: rt.text,
            cost_ms: run.cost_ms,
            usage: rt.usage,
            fell_back: false,
            aborted: false,
            awaits_user: true,
            closure_denied: false,
            // 等待轮的收束留给恢复幕，但**本轮已产出的正文**照样带真原因。
            finish: rt.finish.clone(),
        });
        return Ok(());
    }

    // 把这一轮交换**累加**进 prompt，继续下一轮。
    run.exchange
        .extend(tool_exchange_messages(&rt, &outcomes, run.tool_round));
    run.tool_round += 1;

    // 轮内注入（生产里是「用户中途补充」）。挂在交换之后：工具交换
    // 先接上，注入的内容排在它之后——时间序。
    if let Some(inject) = &run.inject {
        let injected = inject(Some(run.user_seq)).await;
        run.exchange.extend(injected);
    }
    Ok(())
}

/// 收束：答话 / 空答兜底 / 写侧闸拒。
async fn halt<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    rt: LlmTurn,
    round_facts: &StreamNodeFacts,
) -> Result<TurnOutcome, symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    // 答话取值（正文 / 截断 / 端口思考 / 桥上思考）见 [`final_answer`]。
    // `truncated` 另有用处：兜底话术要说清「是截断」还是「空答」。
    let truncated = rt.finish.is_length();
    let answer = final_answer(&rt, round_facts);
    if answer.trim().is_empty() {
        // 正文与思考**都**空 + 无工具调用 = 模型什么也没答。这是**失败**：
        // 落成 final 会把「没答」记成「答了空话」。
        let cost = run.started.elapsed().as_millis() as u64;
        // 空答的正文 / 思考都归这一轮的 `response` 事实，收束格只记结局。
        let produced = produced_payload(
            "",
            llm.model_id(),
            &run.turn_root_id,
            None,
            &run.last_answer_id,
            &run.responses,
        );
        return settle::append_fallback(
            store,
            &run.actor,
            run.turn,
            run.user_seq,
            if truncated {
                "模型输出被长度上限截断，本轮未产出答话"
            } else {
                "model returned empty text"
            },
            cost,
            produced,
            run.artifacts_landed,
        )
        .await;
    }
    if let Err(why) = closure_gate(&run.actor, ClosureKind::Speech) {
        // 写侧闸拒：**不落收束格**——该轮留在未收束态，`check_all` 会把它报出来
        // （C4）。**例外**：本轮落过产物格 ⇒ 已由产物收束（D-067）。
        // 拒绝本身落一条否定事实（`authz.denied`），于是「被拒」不再与「没跑完」同形。
        deny(
            store,
            &run.actor,
            run.turn,
            run.user_seq,
            ClosureKind::Speech,
            EVENT_CHAT_ASSISTANT_FINAL,
            &why,
        );
        symbio_core::plugin_warn!(
            "actors",
            "[v2] 收束被授权拒绝（{why}），本轮不入格（turn={}）",
            run.turn
        );
        return Ok(TurnOutcome {
            turn: run.turn,
            text: answer,
            cost_ms: run.cost_ms,
            usage: rt.usage,
            fell_back: false,
            aborted: false,
            awaits_user: false,
            closure_denied: run.artifacts_landed == 0,
            // 末轮原因随本轮的收束一起带出去。
            finish: rt.finish.clone(),
        });
    }
    let produced = produced_payload(
        "",
        llm.model_id(),
        &run.turn_root_id,
        None,
        &run.last_answer_id,
        &run.responses,
    );
    settle::land_final(
        store,
        &run.actor,
        run.turn,
        run.user_seq,
        run.cost_ms,
        // `reason` 由 `finish` 折出：被 max_tokens 掐断的一轮**答出来了但没答完**，
        // 它与自然说完在 `status` 上同形（都是 completed）、在 `reason` 上不同形。
        with_reason(
            with_status(produced, "completed"),
            reason_of_finish(&rt.finish),
        ),
    )?;
    Ok(TurnOutcome {
        turn: run.turn,
        text: answer,
        cost_ms: run.cost_ms,
        usage: rt.usage,
        fell_back: false,
        aborted: false,
        awaits_user: false,
        closure_denied: false,
        finish: rt.finish.clone(),
    })
}

/// 3a 中止的**载荷**（c）：半截产出与实测耗时。
///
/// 载荷与落格分开：收束格只由 `close` 执行体写，中止那一步只负责把结局寄存下来。
fn aborted_payload(run: &Prepared, llm: &dyn LlmAdapter) -> (u64, serde_json::Value, String) {
    let cost_ms = run.started.elapsed().as_millis() as u64;
    // 半截正文取自那一轮的寄存读数——桥已在 `failed` 载荷那一侧读过一次，
    // 这里不再重读（重读会拿到同一份，只让取数口多算一次）。
    let text = run.last_output_facts.text.clone().unwrap_or_default();
    // 半截正文 / 思考已随 `failed(port=model)` 入格（`output_fields`），收束格只记结局。
    let produced = produced_payload(
        "",
        llm.model_id(),
        &run.turn_root_id,
        None,
        &run.last_answer_id,
        &run.responses,
    );
    (cost_ms, produced, text)
}

/// 3a 中止：照落收束格（`status: "aborted"` + `terminated`）——思考 / 正文是事实，
/// 终止是状态；收束 kind 绝不复用兜底格。
async fn land_aborted<S>(
    run: &mut Prepared,
    store: &S,
    cost_ms: u64,
    produced: serde_json::Value,
    partial_text: &str,
) -> Result<TurnOutcome, symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    if let Err(why) = closure_gate(&run.actor, ClosureKind::Speech) {
        // 中止的收束格同过写侧闸（收束格是同一个事实，不论哪条路径写出）。
        deny(
            store,
            &run.actor,
            run.turn,
            run.user_seq,
            ClosureKind::Speech,
            EVENT_CHAT_ASSISTANT_FINAL,
            &why,
        );
        symbio_core::plugin_warn!(
            "actors",
            "[v2] 中止收束被授权拒绝（{why}），本轮不入格（turn={}）",
            run.turn
        );
        return Ok(TurnOutcome {
            turn: run.turn,
            text: partial_text.to_string(),
            cost_ms,
            usage: None,
            fell_back: false,
            aborted: true,
            awaits_user: false,
            closure_denied: run.artifacts_landed == 0,
            // 中止是「用户按了停止」，不是「模型被截断」：长度戳不得误触发。
            finish: symbio_core::ModelFinishReason::Stop,
        });
    }
    settle::land_final(
        store,
        &run.actor,
        run.turn,
        run.user_seq,
        cost_ms,
        with_terminated(with_reason(
            with_status(produced, "aborted"),
            symbio_core::ClosureReason::Interrupted,
        )),
    )?;
    Ok(TurnOutcome {
        turn: run.turn,
        text: partial_text.to_string(),
        cost_ms,
        usage: None,
        fell_back: false,
        aborted: true,
        awaits_user: false,
        closure_denied: false,
        // 中止轮不入续写判据（用户按的停止不是被 max_tokens 掐断）。
        finish: symbio_core::ModelFinishReason::Stop,
    })
}

/// 3b 兜底的**载荷**（c）：失败前已产出的半截产出与实测耗时。
///
/// 失败前已产出的半截正文 / 思考已随 `failed(port=model)` 入格（`output_fields`）；
/// 兜底格的正文是 `detail`（人话），这里只记结局。
fn failed_payload(run: &Prepared, llm: &dyn LlmAdapter) -> (u64, serde_json::Value) {
    let cost_ms = run.started.elapsed().as_millis() as u64;
    let produced = produced_payload(
        "",
        llm.model_id(),
        &run.turn_root_id,
        None,
        &run.last_answer_id,
        &run.responses,
    );
    (cost_ms, produced)
}
