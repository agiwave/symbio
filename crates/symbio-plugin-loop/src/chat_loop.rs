//! SESSION 聊天主循环
//!
//! 本文件只保留**主循环骨架**，其余按职责拆到子模块
//! （切分依据见 `crates/symbio-plugin-session/src/docs/104-module-layout.md` §0 五判据与 §1.1 编排层落点）：
//!
//! ```text
//! 前步骤 ① 请求快照 ② 开会话 ③ TurnState+容器 ④ resume
//! 步骤1 中止边界检查        ← 事实判据，不再是决策函数
//! 步骤2 turn_driver::drive    ← 外层环：选债 / 派发 / 收束判定（P2-6a）
//! 后步骤 无（直路只能由 return 离开，全部经 finish_turn 收尾）
//! ```
//!
//! 本文件**没有环**：`D-222` 裁决 5 把控制面收成**一个**环，它住在
//! [`super::turn_driver::drive`]。幕首、轮首四行、接缝、汇报、轮内各行与幕间接续
//! 都是那张绑定表上的行，由 `(phase, anchor)` 决出先后；本文件只做三件事——备好本轮状态、
//! 过闸门、把结局映射成 [`TurnExit`]。
//!
//! 落库是写投影，唤醒是交换点（`D-222` 裁决 1）。
//!
//! 子模块分工：
//! - [`state`]    会话上下文 / 请求快照 / 单轮状态 / 闸门结果 / 退出原因 / 编排器
//! - [`decide`]   轮首判决：经容器 `route` 调 `classify/decide`，把判决回读成枚举
//! - [`compose`]  判决的执行点：经容器 `route` 调 `compose/wording`，把措辞写进转写
//! - [`inputs`]   收口 ②③：提示词与工具的唯一收集点、压缩的唯一响应点
//! - [`turn`]     单幕收尾（产出处理）：推理并入 → 落库（`persist_turn`）
//! - [`io`]       副作用出口：落库 / 广播 / 流式占位 / 开会话 / 生命周期钩子
//!
//! 压缩流水线（自动语义压缩与用户重试共用内核）归
//! `super::context::pipeline`——上下文治理属**领域**，本文件只保留编排。
//!
//! 设计说明：
//! - 统一从会话服务获取消息历史，不区分有状态/无状态协议
//! - 具体协议实现层决定如何使用这些历史（有状态协议可能只使用部分或不使用）
//! - 请求中只包含当前要发送的单条消息（single_message）

pub(crate) mod compose;
pub(crate) mod delegate;
pub(crate) mod drive;
pub(crate) mod inputs;
mod io;
pub(crate) mod open_turn;
pub(crate) mod state;
mod turn;
mod wake;

pub use self::state::ChatOrchestrator;

// 模块内共享面：子模块经 `use super::*` 取用，测试亦同（`gate_tests` 等）。
use crate::session_port;
// 对话面节点构造器已上移 core `node_build`（[D-082](../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三），
// 消费方（`compose` / `rebuild` / `turn_runner`）各自直连 core——本模块不再续发短路径，
// 留守侧因此不必经驱动器拿构造器。
pub(crate) use self::drive::{land_dispatch, land_opening};
pub(crate) use self::io::{emit_streaming_start, finalize_turn_root, persist_messages};
pub(crate) use self::state::{
    request_actor_spec, request_source_principal, SessionContext, TurnExit, TurnRequest,
    TurnState,
};
pub(crate) use self::turn::{persist_turn, settle_reasoning};
// 一幕第三步（收好产出）要用它（镜像并入是落库的前一步）：`stage` 是兄弟模块，
// 拿不到 `mod wake` 的私有面，所以从这里再导一条。
pub(crate) use self::wake::merge_runner_messages;

/// 批次边界汇报的**能力口**：只产措辞，不判定、不落格、不数批次。
///
/// ## 为什么这里是这个形状（D-163）
///
/// 「这一段静默该不该说」已经全在行 `policy.report` 与 `report_due` 的 `due` 里
/// ——开关 / 静默时长 / 配额 / **工具轮数**四格都由本幕已落的事实折出。所以这里
/// 不再问 `owes`、也不再自己数批次边界（那个 `AtomicUsize` 与返回草稿的
/// `RoundHookFn` 因此退场）：编排层只剩一件本侧才能做的事——**问 compose 要一句话**
/// （编排器、请求句柄、轮首会话视图都在这一侧）。
///
/// ## 会话视图：只给**轮首**那一份
///
/// compose 的 `ComposeRequest` 带一段会话视图，而批次边界上 `context.messages` 里
/// **只有轮首那一批**（工具轮的消息由 `turn_exec` 在轮末才交回来）。所以这里传的
/// 快照与那一刻 `context.messages` 的内容**逐字相同**，不是近似。代价是它不含本轮
/// 工具节点——仓内的 `progress_text` 不读这一段（只用 `snapshot`），而要让仓外
/// compose 插件也拿到它就得再造一条把工具消息实时送回来的通路，为一个字段不值得。
/// 真要那一条时该一起做，别在这里打补丁。
pub(crate) fn report_port(
    orchestrator: Arc<ChatOrchestrator>,
    ctx: Arc<dyn symbio_kernel::plugin::PluginInvokeRequest>,
    conversation: Vec<ChatNode>,
) -> super::report_exec::ReportPort {
    let text: super::report_exec::ReportTextFn = Arc::new(move |snapshot: RunSnapshot| {
        let orchestrator = orchestrator.clone();
        let ctx = ctx.clone();
        let conversation = conversation.clone();
        Box::pin(async move {
            super::chat_loop::compose::report_text(&orchestrator, &ctx, &conversation, &snapshot)
                .await
        })
    });
    super::report_exec::ReportPort { text }
}

/// 把请求级注入段拼成一段前缀（v2 运行器的 prompt 入口）。
///
/// ## 为什么需要它
///
/// provider 贡献的各段与委派者真源段是**独立的消息**（`build_request_sections` 的
/// 产物，都置顶、`meta.kind` 可观测、不落库）。而 v2 运行器的 prompt 是**一条**
/// user 消息——`ProviderLlmAdapter::generate_turn` 只收 `TurnInput.text` 与渲染出的
/// 基线，那些段若不显式带进来就**静默**从模型眼前消失：事实照样入格、
/// `session/stats` 照样有数，只有模型看不见。
///
/// ## 取法：按**视图次序取全部**，不按名字查表（DE-025 D-215）
///
/// 端口化之前这里有一张 `ORDER` 数组按 `kind` 逐个 `find`。那个形状的前提是「段的名字
/// 有第二处拼写」——而端口化之后**段的名字只有一个出处**（provider 自己），视图里也
/// 只有段这一类节点，于是「按名字查」既没有要防的错，还会把「加一类上下文」变成改
/// 这张表。现在改成顺序遍历：**视图次序 = 装配次序 = 模型读到的次序**。
///
/// 旧注释说「装配次序与优先级不是同一条判据，故意分开」。端口化之后它们**合并**了，
/// 这是刻意的：段的先后现在由 provider 的 owner 序决定（DE-025 §3 第 4 条），
/// 那正是「按声明装配」的意思——分开两处只会在加 provider 时漂。
///
/// 全部为空 ⇒ `None`（不拼空壳：那会让每轮 prompt 都多一段空白，而各段皆空是常态）。
pub(crate) fn request_view_prefix(view: &[ChatNode]) -> Option<String> {
    // `Vec<String>` 而非 `Vec<&str>`：`to_text()` 按值产出一个 `String`，`trim()`
    // 借的是那个临时值——收 `&str` 会把返回值绑到函数参数上（E0515）。
    let mut parts: Vec<String> = Vec::new();
    let mut kinds: Vec<&str> = Vec::new();
    for node in view {
        let Some(kind) = node
            .meta
            .as_ref()
            .and_then(|v| v.get("kind"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let Some(text) = node
            .content
            .as_ref()
            .map(|c| c.to_text())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        kinds.push(kind);
        parts.push(text);
    }
    if parts.is_empty() {
        return None;
    }
    // **每轮都记一次「取到哪几段」**——各段皆空是常态（生产里绝大多数轮次），
    // 而「记忆段明明有东西却没进 prompt」这类失效**没有任何东西会变红**：事实照样
    // 入格、`session/stats` 照样有数。所以取值点自己出声：段在而模型看不见时，
    // 日志里至少有「取到了却没送出」的对照可查。
    symbio_plugin_api::plugin_info!(
        "session",
        "[request-prefix] 本轮取到 {} 段（{}），拼进 prompt 前缀",
        kinds.len(),
        kinds.join(",")
    );
    Some(parts.join("\n\n"))
}


use std::sync::Arc;
use symbio_kernel::dialog::RunSnapshot;
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::llm::ModelFinishReason;
use symbio_kernel::llm::ModelUsage;
use symbio_kernel::llm::TurnOutput;
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_plugin_api::plugin_info;
use symbio_plugin_api::plugin_warn;
use symbio_sdk::chat::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeRole, NodeStatus, ResumeAction,
};
use symbio_sdk::chat::chat_request;
use symbio_sdk::llm::turn::{llm_emit_message, llm_short_id};
use symbio_sdk::turn::dispatch::{run_ops, TurnReport, TurnStatus};

use super::context;
pub(crate) use super::message_build::TurnOutputExt;
use symbio_sdk::llm::llm_emit_state;
use symbio_sdk::turn::node_build::detach_if_flat;

pub async fn run_chat_loop(
    orchestrator: std::sync::Arc<ChatOrchestrator>,
    ctx: Arc<dyn PluginInvokeRequest>,
    sink: ExecEventSink,
    abort: ExecAbortSignal,
) -> TurnReport {
    // ── 前步骤 ①：请求解析与请求级配置快照 ─────────────────────────────────
    let mut req: chat_request::Request = match ctx.payload() {
        Ok(req) => req,
        Err(e) => {
            symbio_plugin_api::plugin_error!("session", "[Session] 请求体解析失败：{e}");
            return TurnReport {
                status: TurnStatus::Failed,
                error: Some(e.to_string()),
                partial: Vec::new(),
                last_message: None,
            };
        }
    };
    // 请求体未给出的字段一律回落到**本轮配置快照**（`TurnSettings`），本函数不再
    // 自带魔法数——默认值的唯一真源在配置层（审计 C2）。
    let turn_req = TurnRequest::new(&req, orchestrator.view.settings.auto_compress);

    // 编排器本体：下面多处按引用传（`&ChatOrchestrator`），而轮边界回调要
    // **自己持有一份**（回调是 `Fn + Send + Sync + 'static`，见
    // `round_report_hook` 的文档）。所以这里留一个借用、另clone 一份给回调。
    let orchestrator_arc = orchestrator.clone();
    let orchestrator = orchestrator.as_ref();

    // ── 前步骤 ②：会话句柄（随交付包进来，不在这里开会话）──────────────────
    let session = orchestrator.session.clone();

    // ── 前步骤 ③：轮次状态 + 上下文容器 ───────────────────────────────────
    let mut turn = TurnState {
        abort: abort.clone(),
        ..Default::default()
    };
    let mut single_message = req.single_message.take();
    let mut context = SessionContext {
        messages: Vec::new(),
        view: orchestrator.view.clone(),
        facts: orchestrator.facts.clone(),
        session,
        // 本轮压缩的寄存位：压缩内核压成之后写这里，由本轮的 `turn_exec::drive` 落格。
        consolidated: None,
    };

    // ── 会话恢复（resume）：动作在幕锚行里落成事实 ─────────────────────────
    //
    // `req.resume` 存在时，本调用是用户的恢复操作：
    // - `RetrySubtree`：按普通新轮重来，被重试那一支不删（成为另一支候选）；
    // - `RetryCompaction`：重跑一次压缩，不进对话轮次；
    // - `Retry`/`Approve`/`Reject`/`Supply`/`Answer`：在原轮上落一笔
    //   `request(port=tool)`（载荷带 `resume` 标记）。之后的授权 / 派发由行表
    //   自己的 `authorize-tool` / `dispatch-tool` / `deny-tools` 承担——恢复路径
    // 不再自己写协议事实、不再自己跑执行层（d）。
    //
    let resume = req.resume.take();
    if let Some(tr) = resume.as_ref() {
        if matches!(tr.action, ResumeAction::RetryCompaction) {
            if let Err(e) = super::context::retry_compaction(
                orchestrator,
                &ctx,
                &turn.abort,
                &context.session,
                &tr.target_id,
            )
            .await
            {
                plugin_warn!("session", "[Wake] RetryCompaction failed: {}", e);
                return finish_turn(&context, &sink, &turn, TurnExit::Failed(e)).await;
            }
            return finish_turn(&context, &sink, &turn, TurnExit::ResumeDone).await;
        }
    }

    // ── 步骤 1：退出条件（事实 / 租约判据，不再是决策函数）──────────────────
    // 不冒泡：上一轮 Turn 已定稿落库，冒泡会让消费循环的 `persist_failure` 把成功
    // 的 Turn 误回滚为 Failed。推理前后（在途 Turn 尚未定稿）的中止才走
    // [`TurnExit::Aborted`]。
    if turn.abort.is_aborted() {
        return finish_turn(&context, &sink, &turn, TurnExit::AbortedAtBoundary).await;
    }

    // ── 步骤 2：唯一一个环（`D-222` 裁决 5）─────────────────────────────
    //
    // 开幕、轮首四行、接缝、内环、收幕与幕间接续全在 [`super::turn_driver::drive`]
    // 里：它读事实选最小可还债，五道守卫各只有一份，幕与幕在同一次调用里接续。
    // 本文件不判「下一步是哪一步」，只把环的结局映射成 [`TurnExit`]。
    //
    // **不要在这里拦中止**（`D-052`）：拦截点就是环的第一道守卫（用户格已落、
    // 第一个请求未发）。在此处 `return TurnExit::Aborted(Vec::new())` 会让运行器
    // 从不落 `user.message`，中止这一轮**一条事实都没有**（e2e t7 钉的是这件事：
    // `v2-events.wal` 不存在、投影里看不到用户说过什么）。D-052 决策 1 要求
    // 「一轮的产出无论完成 / 失败 / 被中断 / 悬在等待用户，都入格」，决策 2 要求
    // 终止以**状态**入账。
    //
    // 结果先落成变量再 `match`：`Control` 里装的是 `&mut context` / `&mut turn`，
    // 作为 scrutinee 的临时值会活到整个 `match`——答话那一臂要再借它们一次。
    let driven = super::turn_driver::drive(
        super::turn_exec::Control {
            orchestrator,
            orchestrator_arc: &orchestrator_arc,
            ctx: &ctx,
            sink: &sink,
            context: &mut context,
            turn: &mut turn,
            req: &turn_req,
            single_message: single_message.take(),
            resume,
        },
        super::loop_rows::ACTORS,
    )
    .await;
    let exit = match driven {
        // 判决判成「就此答话」：本幕一次请求都没发，收束格由编排层就地落
        // （[`land_answered_turn`] / D-125）——它要的是编排器、会话上下文与本轮
        // 状态，环里没有它们。
        Ok(super::turn_exec::DriveEnd::Answered) => {
            let facts = context.facts_snapshot();
            if let Some(answer) =
                super::stage_events::answer_of(&facts, turn.turn_no, turn.drive_seq)
            {
                land_answered_turn(&ctx, &mut context, &mut turn, answer).await;
            }
            let landed = match context.facts() {
                Some(store) => {
                    super::turn_exec::land_inputs(store.as_ref(), ctx.as_ref(), &turn, "closed")
                }
                None => Err(super::turn_exec::ActorError::Read(
                    symbio_kernel::plugin::PluginError::InternalError(
                        "定稿答话轮需要持久会话（临时会话无事实源）".into(),
                    ),
                )),
            };
            if let Err(e) = landed {
                return finish_turn(
                    &context,
                    &sink,
                    &turn,
                    TurnExit::Failed(match e {
                        super::turn_exec::ActorError::Read(e) => e,
                        super::turn_exec::ActorError::Append(e) => {
                            symbio_kernel::plugin::PluginError::InternalError(format!(
                                "本幕落格失败：{e:?}"
                            ))
                        }
                    }),
                )
                .await;
            }
            TurnExit::Completed
        }
        // 幕间中止：上一幕已收束，下一幕没开（同一轮不落第二格收束）。
        Ok(super::turn_exec::DriveEnd::Boundary) => TurnExit::AbortedAtBoundary,
        // 控制面跑完了全部幕（收束格与派生事实都已落格）。
        Ok(super::turn_exec::DriveEnd::Completed) => TurnExit::Completed,
        // 夹具入口的结局只在测试里出现：控制面交下去的一定是 `Control`。
        Ok(super::turn_exec::DriveEnd::Bare(_) | super::turn_exec::DriveEnd::Running(_)) => {
            TurnExit::Failed(symbio_kernel::plugin::PluginError::InternalError(
                "控制面交下了夹具入口的结局（按构造不可达）".into(),
            ))
        }
        Err(super::turn_exec::TurnExecExit::Aborted(partial)) => TurnExit::Aborted(partial),
        Err(super::turn_exec::TurnExecExit::Failed(e)) => TurnExit::Failed(e),
    };
    finish_turn(&context, &sink, &turn, exit).await
}


/// 定稿答话轮的**收束**：两条收束格由**轮首准备**落（§5.1 / [D-125]）——它经
/// `settle::land_final` 的同一构造点写出去（`SP-033` `TC-4`），本函数只是**调用方**。
///
/// 这一轮一件都不欠：模板产线出话时零模型往返；措辞产线那一次由 `compose` 在轮首
/// 之前发起并已入账（`via=compose/wording`），它不是本轮的债。所以驱动循环不进——
/// 「谁有权写这一格」仍由 `loop_rows::ACTOR_CLOSE.writes` 回答，与进不进环无关。
///
/// ## 为什么仍须由 v2 运行器落
///
/// 不落，这一轮在事实网格里只有用户格、没有 `chat.assistant.final` ⇒ 下一轮 prompt
/// 里这次问答**整个消失**，`session/stats` 的轮次计数也缺收束——而 `messages.json`
/// 里有（答话随 `finish_turn` 落库）⇒ **两条真源**。完全静默：不报错、不告警、
/// 既有断言全绿。判据 = e2e `t22`。
///
/// ## 落格失败只告警
///
/// 答话已在 `context.messages` 里，用户照旧收得到。把它升级成整轮失败，等于用
/// 「记不下来」换「答不出来」——与中途汇报的落格同一条口径。
pub(crate) async fn land_answered_turn(
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    turn: &mut TurnState,
    answer: super::stage_events::AnswerFact,
) {
    // 先把寄存的压缩取走：后面按引用传上下文（落库读配置与事实）。
    let consolidation = context.consolidated.take();
    if let Err(e) = super::turn_exec::execute_final_reply_turn(
        context,
        ctx,
        turn.abort.clone(),
        turn.turn_no,
        turn.user_seq,
        turn.drive_seq,
        &answer.utterance,
        &answer.reply,
        answer.cost_ms,
        super::turn_runner::DialogFacts {
            id: answer.dialog_id,
            reason: answer.dialog_reason,
            exclude_from_context: answer.exclude_from_context,
        },
        answer.reasoning,
        // 答话轮同样要落「本轮之前那次压缩」这一格——它不进 `turn_exec::drive`，
        // 不带上就漏了（见该函数的入参文档）。
        consolidation.as_deref(),
    )
    .await
    {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Session] 定稿答话入格失败（本轮照常收尾）: {e}"
        );
    }
}


/// 主循环的**唯一收尾点**：增量落库 → 结局值。
///
/// 收口前，12 个出口各自重写「Stop 钩子 + 落库 + 返回」三件套。现在每个出口只负责
/// 判定 [`TurnExit`]，收尾动作只在本函数出现一次。
///
/// Stop 钩子**不在这里**（[D-086](../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §6）：
/// 它归会话生命周期，session 拿 [`TurnReport::last_message`] 统一触发——「恰好一次」
/// 因此仍由生命周期保证，而不是由两侧各记一次。
///
/// `persist_messages` 在锚点已对齐时是 no-op（切片为空即返回），因此所有出口都可以
/// 无条件调用——不会重复落库：`persist_turn` 与压缩流程每次落库后都会同步推进
/// [`TurnState::last_saved`]。
async fn finish_turn(
    context: &SessionContext,
    sink: &ExecEventSink,
    turn: &TurnState,
    exit: TurnExit,
) -> TurnReport {
    // 增量落库（锚点已对齐时为空切片，天然 no-op）。
    persist_messages(context, turn.last_saved, sink).await;
    // 分支标注的**实时面**（[D-173](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
    //
    // 标注由**重建**盖章，重建只在读的时候跑；流式帧里没有它 ⇒ 光靠本轮的帧，用户
    // 刷新页面 / 切回会话才看见 `‹ 1/2 ›`。所以本轮落库之后（新支的节点这才进得了投影）
    // 按已盖章的那一份把**组内每个成员**补发一次：切换器当场出现，两支谁在上谁在下也当场
    // 分明。落格那一刻做不了这件事——那时组里只有一条现存成员，重建一个字段都不盖。
    if let Some(into) = turn.branch_root() {
        republish_branch_group(context, sink, &into).await;
    }

    // Stop 钩子的 `last_message`：取本轮重建视图的末条正文（与旧 `StopSignal::fire`
    // 同一取法）。resume 出口发生在主循环之前、此时尚无消息列表 ⇒ `None`。
    let last_message = match exit {
        TurnExit::ResumeDone => None,
        _ => context
            .messages
            .last()
            .map(|m| m.content.as_ref().map(|c| c.to_text()).unwrap_or_default()),
    };

    // 结局方向：中止与失败各自分明（中止不是失败），由 session 侧重建在途 Turn 的
    // 终态；半截产出随 `partial` 交回（它们是事实，不因没说完就不记）。
    match exit {
        TurnExit::Aborted(partial) => TurnReport {
            status: TurnStatus::Aborted,
            error: None,
            partial,
            last_message,
        },
        TurnExit::Failed(e) => TurnReport {
            status: TurnStatus::Failed,
            error: Some(e.to_string()),
            partial: Vec::new(),
            last_message,
        },
        _ => TurnReport {
            status: TurnStatus::Completed,
            error: None,
            partial: Vec::new(),
            last_message,
        },
    }
}

/// 按**已盖章的投影**补发一个分支组里每个成员的帧（只读投影 + 发帧，不落任何事实）。
///
/// 读的是 `run_ops::MESSAGES`（呈现读侧：两支都在，带 `meta.branch`），不是
/// `CONTEXT_MESSAGES`（那条会先把未选中支藏掉——拿它补发等于把「藏」当成了「没发生」）。
/// 发不出去也没什么可修的：`sink` 缺席（内部请求）时本来就是静默出口。
async fn republish_branch_group(context: &SessionContext, sink: &ExecEventSink, member: &str) {
    let all = match session_port::call::<Vec<ChatNode>>(
        context.session.as_ref(),
        run_ops::MESSAGES,
        (),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            plugin_warn!("session", "[Branch] 补发标注帧前读投影失败：{e}");
            return;
        }
    };
    let Some(candidates) = all
        .iter()
        .find(|m| m.id == member)
        .and_then(symbio_sdk::projection::branch::branch_candidates)
    else {
        // 组里凑不齐两条现存成员（新支没产出节点就被中止 / 事实没落上）⇒ 没有标注可补，
        // 也没有切换器可画——不是错误，是这一轮确实没开出第二支。
        return;
    };
    for id in candidates {
        if let Some(n) = all.iter().find(|m| m.id == id) {
            sink.emit(symbio_sdk::llm::turn::llm_message_frame(n)).await;
        }
    }
}

#[cfg(test)]
#[path = "chat_loop.test.rs"]
mod tests;
