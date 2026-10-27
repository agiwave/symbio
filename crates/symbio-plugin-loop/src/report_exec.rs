//! **批次边界 host** 的执行体与派发点（DE-015 的 D-159 分片 + D-163 的口径）。
//!
//! ## 这一台 host 管的是哪一步
//!
//! 轮中汇报（`policy.report`）：一段静默到期、配额未满、且本幕已经跑完至少
//! `progress_min_rounds` 轮工具调用——三条判据全在 `report_due` v4 的 `due` 里。
//! 派发点是**轮内注入口**（`turn_exec` 的 2f：每跑完一批工具问一次），因为
//! 「一段静默」在时间上就只在批次边界上可观测。
//!
//! ## 为什么它不是 `TurnStep`
//!
//! 汇报要在模型往返**进行中**落地（那句话的全部意义就是「本轮还要跑很久，先报一句」），
//! 而那一刻本轮草稿正被 `session.model(...)` 借走（`&mut TurnSession`）——同一块可变
//! 状态不能同时借给两个执行体动作。所以这三行归的是**另一块**：落格句柄 + 上线面
//! （实时桥 / 落库镜像）+ 措辞能力口。D-159 裁决 1 的 host 定义照用——差别只在借什么。
//!
//! ## 出口那一格由执行体自己落
//!
//! 执行体不交回产出（D-123 决策 5）：汇报那条事实由它自己 append，对话面节点由它自己
//! 回灌落库镜像与实时桥。派发器只读账本判「这一步做了没有」（写封闭 + 空账本两条，
//! 与 [`super::head_exec::drive_head`] 逐字同形）。
//!
//! ## 一次边界只派发一次，不循环
//!
//! 一台 host 一个派发点不等于一个 `loop`：这一台只有一行，而「这段静默的一笔账」在
//! 落地之后必然关掉（汇报自己成为下一段静默的起点）。落地与关账若是不同锚，循环会
//! 在批次边界上原地空转——停一次 + 出声（下一批次边界重试）比空转好。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use symbio_core::chat_node::ChatNode;
use symbio_core::{DeltaSink, Event, Seq, Store};

use super::turn_exec::{ActorError, StepFuture};

/// 汇报正文的**能力口**：给一份运行现状，交回一句要说的话（`None` = 不说）。
///
/// 与 [`super::turn_exec::SupplementFn`] 同族：编排层手里有 compose 的能力（编排器 +
/// 请求句柄 + 轮首的会话视图），而落格与上线在这一侧。它**只产文本**——判定（该不该
/// 说）在行表，配额与静默在视图，这里都不掺。这正是旧 `RoundHookFn` 退场的那一半：
/// 回调不再回答「要不要说」，也不再自己数批次。
pub(crate) type ReportTextFn = Arc<
    dyn Fn(
            symbio_core::RunSnapshot,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send>>
        + Send
        + Sync,
>;

/// 汇报的**另一半出口**：这句话得同时出现在两个面上。
///
/// - `collected`：本轮落库镜像的收集器（`persist_messages` 只写它的尾部增量），
///   不推就等于「实时面看见了、重启后没了」；
/// - `live`：实时桥（`UiBridge`）。**产生即上线**是这条产线的目的：攒到轮末再送，
///   一次长工具轮就全程无反馈；
/// - `no`：同一幕里第几句汇报的序号（事件 id `rp-{turn}-{no}`；与轮内补充共用这一个
///   计数器，两者用的是同一个 `<turn>-<no>` 名字空间）。
///
/// `Clone` 是因为批次边界的闭包是 `Fn`：每次调用都要一份自己的落点（`Arc` 各自加一，
/// 不是第二个真源）。
#[derive(Clone)]
pub(crate) struct ReportLanding {
    pub(crate) collected: Arc<Mutex<Vec<ChatNode>>>,
    pub(crate) live: Arc<super::turn_exec::UiBridge>,
    pub(crate) no: Arc<AtomicU64>,
}

/// 编排层交给轮内的一次性能力包：这一轮的汇报开没开（`None` = compose 产线关着），
/// 以及它要的两样东西。
///
/// 为什么能力口随 `ExecTurn` 下去、而不是在这里自己取：编排器、请求句柄与轮首的
/// 会话视图都在 `chat_loop` 那一侧，而落格与上线在这一侧。两边各管一半，所以交下来
/// 的只能是**能力**（产一句什么话），不是**决定**（要不要说）——后者已经在行表里。
#[derive(Clone)]
pub(crate) struct ReportPort {
    /// 本幕的「现在」（唤醒事实的 `ts`）：静默长度的右端读数，与视图同一个值。
    pub(crate) now_ms: i64,
    /// 措辞能力口（compose 产线）。
    pub(crate) text: ReportTextFn,
}

/// 一次汇报执行体动作需要的**全部**环境：事实源 + 本轮身份 + 措辞能力口 + 上线面。
///
/// 与另外两台 host 的形状逐字相同（`fn(&mut Step) -> StepFuture`、同一个
/// [`ActorError`]、同一份对账名单）：执行体拿得到这些，拿不到「下一笔债是什么」的
/// 选择权（D-118 决策 4）。
pub(crate) struct ReportStep<'a> {
    /// 本轮的**事实源**：执行体落自己那一行出口的地方，派发器读账本的地方。
    pub(crate) store: &'a dyn Store<Event = Event>,
    pub(crate) turn_no: u64,
    /// 本幕的「现在」（唤醒事实的 `ts`）：静默长度的右端读数，与视图同一个值。
    pub(crate) now_ms: i64,
    /// 汇报事实的溯源锚（本轮用户格的 seq，I2：断言类事件必须带溯源）。
    pub(crate) anchor: u64,
    /// 说这句话的主体（落格身份，与轮内其它事实同一个来源）。
    pub(crate) principal: &'a str,
    /// 措辞能力口：`Some` = 这一轮的 compose 产线开着。
    pub(crate) text: &'a ReportTextFn,
    pub(crate) landing: &'a ReportLanding,
}

/// 批次边界绑定表里的动作类型（与另外两台同形状，借的面不同）。
pub(crate) type ReportAction = for<'a> fn(&'a mut ReportStep<'a>) -> StepFuture<'a>;

/// `report` 执行体：`policy.report` 行选中的动作——取运行现状 → 要一句措辞 →
/// 落格 + 上线。
///
/// ## 读数从视图来，不从触发点传
///
/// `quiet_ms` / `tool_rounds` 都是 `report_due` 的输出：同一份日志、同一个 `now`
/// 在两个读点（派发器选债、执行体取数）折出同一份值。让派发点把值递进来会造出
/// 第二个读数点——而两处一旦漂移，汇报句里的「已经完成 N 轮」就成了假话，且没有
/// 任何东西会红。
///
/// ## 拿不到措辞 ⇒ 什么都不写
///
/// `compose` 未挂载 / 模板产线失败时 `report_text` 交回 `None`：这一笔账照旧开着，
/// 下一个批次边界再试——与 `apply_verdict` 降级而不失效同一条方向。派发器会把
/// 「落了零格」判出声（`no_progress` 的轮内那一档），因此这不是一个静默的洞。
pub(crate) fn run_report<'a>(step: &'a mut ReportStep<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let ReportStep {
            store,
            turn_no,
            now_ms,
            anchor,
            principal,
            text,
            landing,
        } = step;
        let facts = store.range(Seq::new(0));
        let Some(view) = super::loop_rows::view("report_due", &facts, *turn_no, *now_ms) else {
            // 视图读不出来（未声明 / 扫描超界）⇒ 不猜运行现状。交回读失败，
            // 派发点据此停驱并出声（这一笔账仍在日志里开着）。
            return Err(ActorError::Read(symbio_core::PluginError::InternalError(
                "轮中汇报要读 `report_due`，而声明层没交出这个视图".into(),
            )));
        };
        let quiet_ms = view.get("quiet_ms").and_then(|v| v.as_i64()).unwrap_or(0);
        let tool_rounds = view
            .get("tool_rounds")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let Some(body) = text(symbio_core::RunSnapshot {
            tool_rounds,
            quiet_ms,
        })
        .await
        else {
            return Ok(());
        };
        let draft = symbio_core::facts::ReportDraft {
            text: body,
            tool_rounds,
            quiet_ms,
        };
        let node = draft.to_message(&symbio_core::principal::Principal::from_legacy(principal));
        // 落格：这条事实是「这段静默已经说过了」的唯一凭据（`reported_anchors`
        // 视图按它关账），也是重建侧折出根级对话面节点的唯一来源。
        store
            .append(
                symbio_core::Event::pending(
                    format!(
                        "rp-{turn_no}-{}",
                        landing.no.fetch_add(1, Ordering::Relaxed)
                    ),
                    symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_REPORTED,
                    symbio_core::Entity::Turn,
                    symbio_core::Verb::Asserted,
                    *turn_no,
                    *principal,
                )
                .with_produced_by(*anchor)
                .with_payload(serde_json::json!({
                    "text": draft.text,
                    "tool_rounds": draft.tool_rounds,
                    "quiet_ms": draft.quiet_ms,
                    // 节点身份是事实的一部分：重建侧（`reported_node` 走
                    // `event_payload_node_id`）按它复刻同一张卡，缺了它实时面与
                    // 重建面就是两个身份。
                    "id": node.id.clone(),
                })),
            )
            .map_err(ActorError::Append)?;
        // 上线两面：落库镜像（重启后还在）+ 实时桥（现在就看得见）。
        if let Ok(mut sink) = landing.collected.lock() {
            sink.push(node.clone());
        }
        landing.live.on_tool_frame(&node);
        Ok(())
    })
}

/// 批次边界的派发点：问一次本 host 的行子集，选中就把这一步跑掉。
///
/// 与 [`super::head_exec::drive_head`] 共享的全部纪律：选债只有一个求解器
/// （[`super::loop_rows::next_report`]）、动作前后各读一次日志（尾段 = 这一次动作的
/// **实际写入**，不由执行体自报）、名单外领域格与空账本都出声。
///
/// 两处按 host 不同，都不是放宽：
///
/// - **一次边界只派发一次**（见模块头）：这一台只有一行，且它落地即关账；不循环，
///   免得「落地与关账的锚不一致」在批次边界上原地空转。
/// - 停驱只是**这一步不做了**：账照旧开着，下一个批次边界（或下一幕）会再问。
pub(crate) async fn drive_report(
    store: &dyn Store<Event = Event>,
    turn_no: u64,
    port: &ReportPort,
    anchor: u64,
    principal: &str,
    landing: &ReportLanding,
) -> Result<(), ActorError> {
    let facts = store.range(Seq::new(0));
    let Some(next) = super::loop_rows::next_report(
        &facts,
        turn_no,
        port.now_ms,
        super::loop_rows::REPORT_ACTORS,
    ) else {
        return Ok(());
    };
    symbio_core::plugin_debug!(
        "session",
        "[Loop] 批次边界派发 {}（执行体 {}；债 {}）",
        next.row,
        next.by,
        next.key
    );
    // 账本的两端都在同一块借用里取：执行体借走的是本函数栈上这份 `ReportStep`，
    // 与另外两台派发点同一个构造位置（&mut 的不变性逼出来的，不是选择）。
    let head = store.head();
    {
        let mut step = ReportStep {
            store,
            turn_no,
            now_ms: port.now_ms,
            anchor,
            principal,
            text: &port.text,
            landing,
        };
        (next.action)(&mut step).await?;
    }
    let ledger = store.range(head);
    if let Some(offender) = ledger
        .iter()
        .map(|e| e.kind.as_str())
        .find(|k| !next.writes_closed(k))
    {
        symbio_core::plugin_warn!(
            "session",
            "[Loop] 批次边界执行体 {} 写了未登记的事实 `{}`（行 {}，turn={}）——本步不派发，账仍开着",
            next.by,
            offender,
            next.row,
            turn_no
        );
        return Ok(());
    }
    if ledger.is_empty() {
        symbio_core::plugin_warn!(
            "session",
            "[Loop] 批次边界执行体 {} 落了零格（行 {}，turn={}）——出口没进日志，下一个边界重试",
            next.by,
            next.row,
            turn_no
        );
    }
    Ok(())
}
