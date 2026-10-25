//! SESSION 聊天主循环
//!
//! 本文件只保留**主循环骨架**，其余按职责拆到子模块
//! （切分依据见 `../session/docs/104-module-layout.md` §0 五判据与 §1.1 编排层落点）：
//!
//! ```text
//! 前步骤 ① 请求快照 ② 开会话 ③ TurnState+容器 ④ resume
//! loop
//!   步骤1 加载上下文
//!   步骤2 gate_turn            ← 退出条件（唯一判定点）
//!   步骤2b decide_turn         ← 轮首判决（唯一判决点）
//!   步骤2c apply_verdict       ← 判决的执行点：措辞 + 落点（`Answered` 在此收尾）
//!   步骤3 prepare_turn_inputs  ← 提示词 + 工具 + 压缩 + 请求视图（唯一收集点）
//!   步骤4 execute_turn         ← LLM 调用 + 工具轮 + **轮边界汇报回调**（`on_round`）
//!   步骤5 settle_reasoning     ← 推理产物并入上下文（定稿内容子节点）
//!   步骤6 close_turn           ← 落库 + 流向判定（子树在此收敛）
//!   步骤7 finalize_turn_root   ← 封根 Turn（组合节点终态跟随子树）
//! end loop
//! 后步骤 无（循环只能由 return 离开，全部经 finish_turn 收尾）
//! ```
//!
//! 子模块分工：
//! - [`state`]    会话上下文 / 请求快照 / 单轮状态 / 闸门结果 / 退出原因 / 编排器
//! - [`decide`]   轮首判决：经容器 `route` 调 `classify/decide`，把判决回读成枚举
//! - [`compose`]  判决的执行点：经容器 `route` 调 `compose/wording`，把措辞写进转写
//! - [`inputs`]   收口 ②③：提示词与工具的唯一收集点、压缩的唯一响应点
//! - [`turn`]     单轮收尾：推理并入 → 落库 → 流向判定
//! - [`io`]       副作用出口：落库 / 广播 / 流式占位 / 开会话 / 生命周期钩子
//!
//! 压缩流水线（自动语义压缩与用户重试共用内核）归
//! `super::context::pipeline`——上下文治理属**领域**，本文件只保留编排。
//!
//! 设计说明：
//! - 统一从会话服务获取消息历史，不区分有状态/无状态协议
//! - 具体协议实现层决定如何使用这些历史（有状态协议可能只使用部分或不使用）
//! - 请求中只包含当前要发送的单条消息（single_message）

mod compose;
mod decide;
pub(crate) mod delegate;
pub(crate) mod drive;
mod inputs;
mod io;
pub(crate) mod state;
mod turn;
mod wake;

pub use self::state::ChatOrchestrator;

// 模块内共享面：子模块经 `use super::*;` 取用，测试亦同（`gate_tests` 等）。
pub(crate) use self::compose::{apply_verdict, VerdictEffect};
// 对话面节点构造器已上移 core `node_build`（[D-082](../../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三），
// 消费方（`compose` / `rebuild` / `turn_runner`）各自直连 core——本模块不再续发短路径，
// 留守侧因此不必经驱动器拿构造器。
pub(crate) use self::decide::decide_turn;
pub(crate) use self::drive::begin_drive;
pub(crate) use self::inputs::{prepare_turn_inputs, run_recall};
pub(crate) use self::io::{emit_streaming_start, finalize_turn_root, persist_messages};
pub(crate) use self::state::{
    request_actor_spec, request_source_principal, Gate, SessionContext, TurnExit, TurnRequest,
    TurnState,
};
pub(crate) use self::turn::{close_turn, settle_reasoning};

/// 轮边界回调（缺口 4）：判定与措辞都在本函数里，落格在 `turn_exec`。
///
/// ## 为什么返回**草稿**而不是 `()`
///
/// 落格要 `EventWalStore`，而开着它的是 `turn_exec`。交回一个 `()` 就等于让本函数
/// 「说完了就算记下了」，而这两件事不是同一件事（汇报的对话面节点落进
/// `context.messages`，由 `persist_messages` 落库，**不进事实网格**）。交回草稿
/// 让「谁落格」这件事留在有 store 的那一侧。
///
/// ## 判定只读日志：静默与配额都从事实来
///
/// 旧 `RoundProgress` 自己持静默时钟与配额计数，是同一份事实的第二来源。
/// 起两者都由 `report_due` 视图从**本幕已落的事实**算：
///
/// - 静默锚 = 最近一次用户看得见的动静（用户格 / 补充格 / 任何助手文本）；
/// - 配额 = 本幕锚之后的汇报格数。
///
/// `now_ms` 由开幕交下来（本幕唤醒事实的 `ts`，判据 12）：静默长度是「本幕的现在」
/// 减那条事实，两个读数都在日志里。回调因此没有入参——抽干时落的补充格已经把
/// 静默锚前移，再递一个「抽到的时刻」只是在判决路径上多留一个墙钟读数。
///
/// ## 轮次前提留在触发点
///
/// 日志里没有「当前工具轮数」这一格（`tool_rounds` 只在说出口之后才有），所以
/// 「轮数 > 0 ∧ 轮数 ≥ progress_min_rounds」由本函数自己数批次边界；`min_rounds`
/// 仍由视图交出（不在这里读配置）。
///
/// ## 会话视图：只给**轮首**那一份
///
/// compose 的 `ComposeRequest` 带一段会话视图，而轮边界上
/// `context.messages` 里**只有轮首那一批**（工具轮的消息由 `turn_exec` 在轮末才
/// 交回来）。所以本函数传的快照与那一刻 `context.messages` 的内容**逐字相同**，
/// 不是近似。代价是它不含本轮工具节点——仓内的 `progress_text` 不读这一段
/// （只用 `snapshot`），而要让仓外 compose 插件也拿到它就得再造一条把工具消息
/// 实时送回来的通路，为一个字段不值得。真要那一条时该一起做，别在这里打补丁。
pub(crate) fn round_report_hook(
    orchestrator: Arc<super::chat_loop::state::ChatOrchestrator>,
    ctx: Arc<dyn crate::symbio_core::PluginInvokeRequest>,
    conversation: Vec<ChatNode>,
    store: Arc<dyn crate::symbio_core::Store<Event = crate::symbio_core::Event>>,
    turn_no: u64,
    now_ms: i64,
) -> super::turn_exec::RoundHookFn {
    let rounds = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Arc::new(move || {
        let orchestrator = orchestrator.clone();
        let ctx = ctx.clone();
        let conversation = conversation.clone();
        let store = store.clone();
        let rounds = rounds.clone();
        Box::pin(async move {
            // 一个批次边界 = 一次「已完成一轮」。
            let tool_rounds = rounds.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let events = store.range(crate::symbio_core::Seq::new(0));
            let view = super::loop_rows::view("report_due", &events, turn_no, now_ms)?;
            // 轮次前提（见 `loop_rows` 模块头）：日志里没有这一格，归触发点。
            let min_rounds = view.get("min_rounds").and_then(|v| v.as_u64()).unwrap_or(0);
            if (tool_rounds as u64) < min_rounds.max(1) {
                return None;
            }
            if !super::loop_rows::owes("policy.report", &events, turn_no, now_ms) {
                return None;
            }
            let quiet_ms = view.get("quiet_ms").and_then(|v| v.as_i64()).unwrap_or(0);
            let snapshot = RunSnapshot {
                tool_rounds,
                quiet_ms,
            };
            let Some(text) = super::chat_loop::compose::report_text(
                &orchestrator,
                &ctx,
                &conversation,
                &snapshot,
            )
            .await
            else {
                // 拿不到措辞 ⇒ 没说话，也**不消耗配额**（下一个轮边界还会再试）——
                // 与 `apply_verdict` 降级而不失效同一条方向。
                return None;
            };
            Some(super::round_hook::ReportDraft {
                text,
                tool_rounds,
                quiet_ms,
            })
        })
    })
}

/// 把请求级注入段**三段**拼成一段前缀（v2 运行器的 prompt 入口）。
///
/// ## 为什么需要它
///
/// 长期记忆召回段 / 就绪任务集段 / 委派者真源段是**独立的消息**
/// （`build_request_sections` 的产物，三段都置顶、`meta.kind` 可观测、不落库）。
/// 而 v2 运行器的 prompt 是**一条** user 消息——`ProviderLlmAdapter::generate_turn`
/// 只收 `TurnInput.text` 与渲染出的基线，那三段若不显式带进来就**静默**从模型眼前
/// 消失：事实照样入格、`session/stats` 照样有数，只有模型看不见。
///
/// ## 取法：按 `meta.kind` 取，不按位置
///
/// 各段的 `kind` 由 [`context::SectionKind::name`] 单点给出。按 `kind` 取而不是按下标
/// （0 / 1 / 2）——下标是「各段都在场时」的巧合，缺一段时下标全部左移，按下标取会
/// 取错段或漏段。
///
/// ## 顺序：按本函数声明的优先级，不按它们在视图里的位置
///
/// 请求级前缀是一段文本，模型读到的是先后顺序；这里是「背景 → 调度候选 → 身份目录」
/// 的**语义优先级**。视图里的位置由装配调用点声明，两者今天恰好一致，但**不是同一条
/// 判据**：把「谁先谁后」绑在装配次序上，下次调装配次序就会连带改掉模型看到的优先级。
///
/// 全部为空 ⇒ `None`（不拼空壳：那会让每轮 prompt 都多一段空白，而各段皆空是常态）。
fn request_view_prefix(view: &[ChatNode]) -> Option<String> {
    const ORDER: [context::SectionKind; 3] = [
        context::SectionKind::Recall,
        context::SectionKind::ReadySet,
        context::SectionKind::Delegate,
    ];
    // `Vec<String>` 而非 `Vec<&str>`：`to_text()` 按值产出一个 `String`，`trim()`
    // 借的是那个临时值——收 `&str` 会把返回值绑到函数参数上（E0515）。
    let mut parts: Vec<String> = Vec::new();
    let mut kinds: Vec<&str> = Vec::new();
    for section_kind in ORDER {
        let kind = section_kind.name();
        if let Some(text) = view
            .iter()
            .find(|m| {
                m.meta
                    .as_ref()
                    .and_then(|v| v.get("kind"))
                    .and_then(|v| v.as_str())
                    == Some(kind)
            })
            .and_then(|m| m.content.as_ref().map(|c| c.to_text()))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            kinds.push(kind);
            parts.push(text);
        }
    }
    if parts.is_empty() {
        return None;
    }
    // **每轮都记一次「取到哪几段」**——三段皆空是常态（生产里绝大多数轮次），
    // 而「记忆段明明有东西却没进 prompt」这类失效**没有任何东西会变红**：事实照样
    // 入格、`session/stats` 照样有数。所以取值点自己出声：段在而模型看不见时，
    // 日志里至少有「取到了却没送出」的对照可查。
    crate::plugin_info!(
        "session",
        "[request-prefix] 本轮取到 {} 段（{}），拼进 prompt 前缀",
        kinds.len(),
        kinds.join(",")
    );
    Some(parts.join("\n\n"))
}

pub(crate) use super::context::auto_compress_process;

use crate::plugin_info;
use crate::plugin_warn;
use crate::symbio_core::chat_request;
use crate::symbio_core::ModelFinishReason;
use crate::symbio_core::{
    chat_node::{ChatNode, MessageContent, NodeKind, NodeRole, NodeStatus},
    llm_emit_message, llm_short_id, run_ops, RunSnapshot, TurnOutput, TurnReport, TurnStatus,
};
use crate::symbio_core::{
    ExecAbortSignal, ExecEventSink, ModelProvider, ModelUsage, Plugin, PluginInvokeRequest,
    PluginInvokeRequestExt,
};
use std::sync::Arc;

use super::context;
use crate::symbio_core::llm_emit_state;
use crate::symbio_core::node_build::detach_if_flat;

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
            crate::plugin_error!("session", "[Session] 请求体解析失败：{e}");
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
    let turn_req = TurnRequest::new(&req, orchestrator.session.settings().auto_compress);

    // 编排器本体：下面多处按引用传（`&ChatOrchestrator`），而轮边界回调要
    // **自己持有一份**（回调是 `Fn + Send + Sync + 'static`，见
    // `round_report_hook` 的文档）。所以这里留一个借用、另clone 一份给回调。
    let orchestrator_arc = orchestrator.clone();
    let orchestrator = orchestrator.as_ref();

    plugin_info!(
        "session",
        "[Session] 新会话开始（协议 {:?}）",
        orchestrator.provider.api_protocol()
    );

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
        session,
        // 本轮压缩的寄存位：压缩内核压成之后写这里，由本轮的 `execute_turn` 落格。
        consolidated: None,
    };

    // 本轮输入（消息 id + 正文）：锚定在 `single_message` 被消费**之前**。
    //
    // 这是「本请求带了什么输入」这一事实，而不是「第几轮」的函数；两处消费它——
    // 下面判决的 `first_utterance`，以及委派段 Q1 的输入（`inputs.rs` 取
    // `turn.input_utterance`）。锚定的那一份不能改从 `context.messages` 现取：
    // `load_history = true` 时那份列表装着整段历史，
    // 「第一条用户发言」逐轮都指回首轮那句（e2e t29 钉住这条）。
    turn.input_utterance = single_message.as_ref().and_then(|m| {
        let text = m.content.as_ref()?.to_text();
        Some((m.id.clone(), text))
    });
    // 同一个锚点取本轮输入的 `meta`（折进的合并补充带 `meta.supplement*`）：它与 id /
    // 正文同属一条用户消息的形状，由 `begin_drive` 落进用户格载荷。
    //
    // 本轮用户发言（**判决的输入**）。
    //
    // 取值点在循环**之前**、`single_message` 被消费之前，因为它是「本请求带了什么
    // 输入」这一事实，而不是「第几轮」的函数。三件事据此判定：
    //
    // - `resume` 请求（重试 / 审批 / 回填答案）**没有** `single_message` ⇒ 不是用户
    //   的新发言，判决无从谈起，也不该被判（那些请求的语义由 `resume.rs` 决定）；
    // - 心跳等无上下文轮次同理（它们由 `single_message` 携带触发文案，见 `heartbeat`）；
    // - `None` 与 `Some("")` 是两件事：前者「没有发言」，后者「发言是空的」——
    //   后者由规则表判成 `Answered{empty}`，前者根本不进判决。
    let first_utterance: Option<String> = turn.input_utterance.as_ref().map(|(_, t)| t.clone());

    // ── 会话恢复（resume）：在 turn 循环前把用户动作落成唤醒请求 ─────────────
    //
    // `req.resume` 存在时，本调用是用户的恢复操作：
    // - `RetrySubtree`：删掉失败轮的消息，按普通新轮重来；
    // - `RetryCompaction`：重跑一次压缩，不进对话轮次；
    // - `Retry`/`Approve`/`Reject`/`Supply`/`Answer`：在原轮上落一笔
    //   `request(port=tool)`（载荷带 `resume` 标记）。之后的授权 / 派发由行表
    //   自己的 `authorize-tool` / `dispatch-tools` / `deny-tools` 承担——恢复路径
    // 不再自己写协议事实、不再自己跑执行层（d）。
    //
    // 唤醒请求落在幕锚之前，但 `turn_head` 的请求面按 seq 判终局，照样看得见它；
    // 本幕因此从「工具批没关」开始，执行行让位给授权 / 派发行。
    let mut next_drive_resume: Option<&'static str> = None;
    if let Some(tr) = req.resume.take() {
        match super::chat_loop::wake::process_wake(
            orchestrator,
            &ctx,
            &turn.abort,
            &context.session,
            tr,
        )
        .await
        {
            Ok(super::chat_loop::wake::WakeOutcome::Woken {
                kind,
                root_id,
                old_child,
            }) => {
                turn.resume_root = Some(root_id);
                turn.resume_old_child = old_child;
                next_drive_resume = Some(kind);
            }
            Ok(super::chat_loop::wake::WakeOutcome::NewTurn) => {}
            Ok(super::chat_loop::wake::WakeOutcome::Done) => {
                return finish_turn(&context, &sink, &turn, TurnExit::ResumeDone).await;
            }
            Err(e) => {
                plugin_warn!("session", "[Wake] process_wake failed: {}", e);
                return finish_turn(&context, &sink, &turn, TurnExit::Failed(e)).await;
            }
        }
    }

    loop {
        // ── 步骤 1：加载本轮上下文 ───────────────────────────────────────────
        // 每轮开始时从 ChatSession 获取最新上下文（轮次窗口生效；骨架化/淡化为请求
        // 视图层职责）。心跳任务等场景可设置 `load_history = false`：仅用本次
        // single_message，完全不加载历史，也不保留上一轮内存累积（随本轮重置）。
        context.messages = if turn_req.load_history {
            context
                .session
                .run_op::<Vec<ChatNode>>(run_ops::CONTEXT_MESSAGES, ())
                .await
                .unwrap_or_default()
        } else {
            // 心跳等无上下文场景：从空上下文开始，仅携带本轮新消息。
            Vec::new()
        };

        // 首轮追加当前用户消息（去重：避免与存储中已持久化的消息重复）。
        // `UserPromptSubmit` 钩子**不在这里**：它由 session 在派发本轮之前触发
        // （[D-086](../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §6——钩子与会话生命周期同侧，
        // 驱动器只负责执行）。
        if turn.request_round == 0 {
            if let Some(msg) = single_message.take() {
                if !context.messages.iter().any(|m| m.id == msg.id) {
                    context.messages.push(msg);
                }
            }
        }

        turn.last_saved = context.messages.len();

        // ── 步骤 2：闸门（退出条件，唯一判定点）───────────────────────────
        // 收口前，这里散落 4 处 `if abort_flag { ... return }`（其中两处相邻重复），
        // 各自重写收尾三件套。
        if let Gate::Exit(exit) = gate_turn(&turn) {
            return finish_turn(&context, &sink, &turn, exit).await;
        }

        // ── 步骤 2a：开一幕────────────────────────────────────────
        // 用户格（或恢复格）+ 当轮配置 + 轮次事实先于任何判决入格。裁决只读
        // 事实：配置、轮号、幕锚都在这一处定下，后续阶段不再各自数一遍。
        let drive = match begin_drive(&context, &mut turn, &turn_req, next_drive_resume.take()) {
            Ok(d) => d,
            Err(exit) => return finish_turn(&context, &sink, &turn, exit).await,
        };

        // ── 步骤 2b：轮首判决（唯一判决点）───────────────────────────────────
        //
        // ## 它判什么
        //
        // 「这一轮该直接回答、还是派给工具循环」。`classify` 只输出**枚举**
        // （`Verdict`），本循环负责**执行**它——判决与措辞分开，执行权归编排层。
        //
        // ## 判据在哪
        //
        // 不在下面这个 `if` 里，在 [`super::loop_rows`] 的 `policy.classify` 行：
        // 「`request_round == 0` ∧ 有发言 ∧ 开关开着 ∧ 没判过」四条写在数据里，
        // 可单测、可增删。本处只问「这一格现在欠不欠」，欠才走这段。
        // 加一条新策略 = 一条声明，**本文件不改**（DE-014 §6 判据 1）。
        //
        // ## 为什么在 `gate_turn` 之后
        //
        // 闸门管的是「本轮要不要发请求」（中止边界）。判决是
        // 「发请求之前先问一句该不该派活」，顺序反了会为一次注定退出的轮次白付
        // 一次判决。
        //
        // ## 为什么只在轮首、且有用户发言时
        //
        // - `request_round == 0`：判决的对象是**用户这一轮说了什么**，只在轮首成立。
        //   工具轮次里循环回顶部时用户没再说话，再判一次就是拿同一句话重判。
        // - `first_utterance.is_some()`：`resume` / 无发言轮次不进判决（取值点的
        //   说明见前步骤 ③）。
        //
        // ## 判决与执行分成两步（`decide` → `compose`）
        //
        // 步骤 2b 只拿**枚举**（`classify` 不说人话），步骤 2c 才把枚举翻成文本
        // （`compose` 不做判决）。分开的理由见 `dialog/mod.rs` 的模块文档：
        // 编排层要能**执行**判决，而措辞只被展示——把两者揉进一次调用，等于让
        // "这轮走哪条路"取决于一段文本长什么样。
        //
        // ## `Answered` 什么时候收尾
        //
        // 只有 `apply_verdict` 返回 `true` 才收尾——即"判决说能直接答，**且**答话
        // 真的写出来了"。措辞拿不到时（未挂载 `compose` / `compose_enabled = false`）
        // 它返回 `false`，本轮**照旧进工具循环**：沉默是最坏的失败形态（用户什么
        // 都收不到，且没有任何错误信号），降级进工具循环则退化成"引入判决之前的
        // 行为"——慢一点，但有答案。判据与兜底方向见 `chat_loop/compose.rs`。
        //
        // `Escalate` / `Report` 恒不收尾：前者本来就要干活（首响只是开场白），
        // 后者只是"中途说一句"（它的触发点在轮边界回调，不在轮首）。
        //
        // 卸载 `classify`（或 `classify_enabled = false`）时这段整体不发生，
        // 行为与引入判决之前逐字一致。
        // `done` 也由这一问问出来：判过的格在 `turn.stage` 里，`turn_head` 视图
        // 按幕锚折叠它（D-107 判据 2）。前三条在这一轮内恒真，没有这一条，
        // 任何以「再问一次表」为形状的驱动都会重判同一句话。
        if super::loop_rows::owes("policy.classify", &turn.facts, turn.turn_no, turn.now_ms) {
            if let Some(exit) = run_turn_head(
                orchestrator,
                &ctx,
                &mut context,
                &mut turn,
                first_utterance.as_deref(),
            )
            .await
            {
                return finish_turn(&context, &sink, &turn, exit).await;
            }
        }

        // 补充的抽干点**不在这里**：v2 在每个批次边界抽，位置由运行器给出——
        // `turn_runner.rs` 的 2f 紧接 2e（本批结果已接进 `exchange`），于是「只有确定
        // 要发下一次请求才抽」与「绝不落在 `assistant(tool_calls)` 与其 `tool` 结果
        // 之间」都由**结构**保证，不再需要一条判据守着（旧判据读的是恒 0 的轮次计数，
        // v2 下永不成立；裁决见 [D-074](../../../../docs/003-design/014-单环一表.md#d-074-恒-0-的轮内计数退出主循环工具轮软上限整条退役抽干点搬到批次边界 "DE-014-单环一表")）。
        // 同一处还负责「抽到补充 ⇒ 静默时钟归零」：抽干落的补充格就是那条静默锚，
        // 视图从日志里读它（回调不再有时刻入参，见 `round_hook::RoundHookFn`）。
        plugin_info!(
            "session",
            "[Turn] 第 {} 次请求开始 (msgs={})",
            turn.request_round + 1,
            context.messages.len()
        );

        // ── 步骤 3：本轮输入准备（收口 ②，唯一准备点）────────────────────────
        // 系统提示词与工具在同一函数内、同一时刻收集（同一个 Registry），
        // 并由此派生请求级开销与请求视图；压缩的水位响应（`apply_compaction`）也收口在该函数内。
        let inputs = match prepare_turn_inputs(
            orchestrator,
            &ctx,
            &mut context,
            &sink,
            &mut turn,
            &turn_req,
        )
        .await
        {
            Ok(inputs) => inputs,
            Err(exit) => {
                return finish_turn(&context, &sink, &turn, exit).await;
            }
        };

        // **不要在这里拦中止**（/ D-052）：在此处 `return
        // TurnExit::Aborted(Vec::new())` 会让运行器从不落 `user.message`，中止
        // 这一轮**一条事实都没有**（e2e t7 钉的是这件事：`v2-events.wal` 不存在、
        // 投影里看不到用户说过什么）。D-052 决策 1 要求「一轮的产出无论完成 / 失败 /
        // 被中断 / 悬在等待用户，都入格」，决策 2 要求终止以**状态**入账。
        //
        // 检查点已**下沉**到 `turn_exec::drive_steps` 的 1c 步：用户格已落、
        // 第一个 LLM 请求未发，已中止则落**中止收束格**（`chat.assistant.final` +
        // `status: "aborted"` + `terminated`）。因此：
        //   - 压缩阶段置位的中止：`execute_turn` 进运行器后被 1c 接住，**不会**多发
        //     一次 LLM 请求（与原意图一致）；
        //   - 用户格与中止收束格成对入格（N3/C4 的不变量在运行器内守住）。
        // 出口形态不变：运行器回 `aborted`，`turn_exec` 上抛
        // `TurnExecExit::Aborted`，走下方 812 分支 → `finish_turn(TurnExit::Aborted(..))`。

        // ── 步骤 4：LLM 调用（唯一发起处）────────────────────────────────────
        // 轮次经 v2 运行器（[`super::turn_exec`]）执行——事实原生入格、prompt 从转写出、
        // 流式经 UiBridge 回同一出口、工具轮经三动作契约分发（工具节点与
        // 结果仍写进同一份对话图）；收口（步骤 5-7）与之共享。
        //
        // 轮次事实**只有这一个写方**：成功原生入格，失败也已尽力入格
        // （I3 兜底格 + Failed 出口），因此同轮两份记账不可能发生。
        //
        // ## 外面多了一层：端口 `turn`
        //
        // 「这一轮欠一次 `turn` 调用」从函数调用变成**事实**：
        //
        // 1. 决策者写 `request{port:turn}`（[`super::port::announce`]）；
        // 2. 问表「`dispatch-turn` 欠不欠」——`request` 没落成或行表装载失败在这里
        //    显形（而不是让一次没人承认的调用静默跑完）；
        // 3. 引擎派发：跑执行体**之前**落 `sent`（带租约与尝试上限），`Ok` 落
        //    `response`、`Err` 落 `failed`。运行器只做活，不写协议事实——它可能
        //    不产任何事实就返回，而「派出去了」不能依赖它自觉。
        //
        // 逻辑 id 含**请求轮次**：一幕可以有多次往返（auto 续写），每次是一次调用。
        // 压缩的寄存位**取走**（S2.7-b2）：`take` 而不是 `clone`——同一份快照只该落一次，
        // 而本轮的压缩已在此刻之前发生完（步骤 3）或已经过去（上一轮步骤 6）。落格与归档
        // 由 `execute_turn` 做（唯一持写者令牌之处），这里只负责把「压成了什么」递过去。
        let consolidation = context.consolidated.take();
        let store = match context.session.facts() {
            Some(store) => store,
            // `begin_drive` 已经拦过「临时会话无事实源」，走到这里就是内部错位。
            None => {
                return finish_turn(
                    &context,
                    &sink,
                    &turn,
                    TurnExit::Failed(crate::symbio_core::PluginError::InternalError(
                        "端口 `turn` 需要持久会话（临时会话无事实源）".into(),
                    )),
                )
                .await;
            }
        };
        // 逻辑 id 必须带**幕锚**：续写幕（审批 / 问答恢复）复用同一轮号且
        // `request_round` 从 0 重来——只带轮号与往返号会与首幕撞成同一个
        // `id`，而首幕已经有过 `sent`；幂等键命中之后 `dispatch-turn` 行不欠，
        // 整轮会在派发处被拒。幕锚是「这一幕」的唯一标识（与 `turn.stage` 同源）。
        let call = super::port::Call::new(
            "turn",
            format!(
                "turn-{}-d{}-r{}",
                turn.turn_no, drive.drive_seq, turn.request_round
            ),
            1,
        );
        if let Err(e) = super::port::announce(store.as_ref(), &call, turn.turn_no, turn.user_seq) {
            return finish_turn(
                &context,
                &sink,
                &turn,
                TurnExit::Failed(crate::symbio_core::PluginError::InternalError(format!(
                    "端口 `turn` 的 request 落格失败：{e:?}"
                ))),
            )
            .await;
        }
        // 派发前的事实切片必须是**刚写完 `request`** 的那一份：行欠不欠由引擎现读
        // （`Dispatch::row_owed`），这里只声明这一笔站在 `dispatch-turn` 上。
        let out = match super::port::dispatch(
            store.as_ref(),
            super::loop_rows::PORTS,
            super::port::Dispatch {
                call: &call,
                turn: turn.turn_no,
                produced_by: turn.user_seq,
                row: "dispatch-turn",
                now: turn.now_ms,
                // 整轮往返等在当前 future 里；崩溃后恢复走新的一幕（新 id），
                // 重试旧 id 只会留下一条没人接的 `request`。
                awaited: false,
            },
            || {
                super::turn_exec::execute_turn(super::turn_exec::ExecTurn {
                    session: &context.session,
                    provider: orchestrator.provider.clone(),
                    parent: orchestrator.parent.clone(),
                    ctx: ctx.clone(),
                    system_prompt: &inputs.system_prompt,
                    abort: turn.abort.clone(),
                    root_id: &inputs.root_id,
                    sink: &sink,
                    user_text: drive.user_text.as_str(),
                    turn_no: drive.turn_no,
                    user_seq: drive.user_seq,
                    drive_seq: drive.drive_seq,
                    window_turns: orchestrator.session.context_window(),
                    tools: &inputs.tools,
                    // 记忆与学习的入参：轮次事实由 v2 原生记账，这两项**不在其中**
                    // （它们的写方是 `derived_facts::record_learning`，见其文档）。
                    recalled: turn.recall_view.as_ref(),
                    skill_obs: &turn.skill_route,
                    // 快路候选集（S11 执行半边）：命中一条 ⇒ 本轮走反射档、不调模型。
                    // 空表（默认）⇒ 与接线前逐字同路（`run_with_tools`）。
                    skill_hits: &turn.skill_hits,
                    // 请求级前缀 = 请求级注入段的三段（见 request_view_prefix）。
                    prefix: request_view_prefix(&inputs.request_sections),
                    // 压缩的寄存位（S2.7-b2）：`None` = 本轮没压过（常态）。
                    consolidation: consolidation.as_deref(),
                    // 本轮已执行的一次性阶段（判决 / 召回 / 压缩）：落格在运行器（用户格之后）。
                    stages: std::mem::take(&mut turn.stages_pending),
                    // 轮内注入（缺口 3）：把本会话的收件箱抽干口交给 v2 的工具循环，
                    // 让「用户中途插话」**折进本轮**。抽干住在 session（队列与配置都在那边），
                    // 驱动器经端口取——异步，因为动作面是异步的。
                    supplements: Some({
                        let session = context.session.clone();
                        Arc::new(move || {
                            let session = session.clone();
                            let fut: std::pin::Pin<
                                Box<
                                    dyn std::future::Future<
                                            Output = Option<crate::symbio_core::DrainedSupplement>,
                                        > + Send,
                                >,
                            > = Box::pin(async move {
                                let supplements: Option<crate::symbio_core::DrainedSupplement> =
                                    session
                                        .run_op(run_ops::TAKE_SUPPLEMENTS, ())
                                        .await
                                        .unwrap_or_else(|e| {
                                            plugin_warn!(
                                                "session",
                                                "[Loop] 取出用户补充失败（本轮继续）: {e}"
                                            );
                                            None
                                        });
                                supplements
                            });
                            fut
                        }) as super::turn_exec::SupplementFn
                    }),
                    supplemental_no: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                    // 轮边界汇报（缺口 4）：判定 + 措辞在这里（编排器 + compose 都在
                    // 本函数手里），**落格**在 `turn_exec`（store 在那边）。判定读日志，
                    // 所以回调也拿一份事实源句柄（同一实例，按引用分发）。
                    on_round: context.session.facts().map(|store| {
                        round_report_hook(
                            orchestrator_arc.clone(),
                            ctx.clone(),
                            context.messages.clone(),
                            store,
                            turn.turn_no,
                            turn.now_ms,
                        )
                    }),
                    // 对话面开场白：步骤 2b 的 `Escalate` 首响。落格在 `turn_exec`
                    // （唯一持写者令牌处），本函数只递「说了什么 / 节点身份」。
                    opening_text: turn.opening_text.as_deref(),
                    opening_dialog: turn.opening_dialog.take(),
                    presentation_tree: orchestrator.presentation_tree,
                })
            },
            // `failed` 的 `why`：中止与失败是两种停法，闭集（`ClosureReason`）
            // 管的是轮子，这里管的是这一次调用为什么没成功。
            |e: &super::turn_exec::TurnExecExit| match e {
                super::turn_exec::TurnExecExit::Aborted(_) => "aborted".to_string(),
                super::turn_exec::TurnExecExit::Failed(err) => format!("failed: {err}"),
            },
        )
        .await
        {
            Ok(res) => {
                // v2 原生产生的消息（工具轮：助手中途正文 + ToolCall + 工具结果）
                // 并入落库权威镜像——`persist_messages` 只写这份的尾部增量，
                // 不并入就只看得见、不落库（重启即丢）。
                //
                // 按 id 并入而不是整段 extend：续写幕的调用卡已经在历史里，新结果
                // 还要替换旧结果子节点（见 `wake::merge_runner_messages`）。
                let session = context.session.clone();
                if let Err(e) = super::chat_loop::wake::merge_runner_messages(
                    &session,
                    &mut context,
                    &mut turn,
                    res.messages,
                )
                .await
                {
                    return finish_turn(&context, &sink, &turn, TurnExit::Failed(e)).await;
                }
                res.output
            }
            Err(super::port::DispatchError::Actor(super::turn_exec::TurnExecExit::Aborted(
                partial,
            ))) => {
                // 中止：在途 Turn 交消费循环落库为 `NodeStatus::Aborted` +
                // 会话结局 `aborted`（**不是** `failed`——中止不是错误）。半截产出
                // （正文 / 思考 / 工具节点）随出口一路带到 `persist_failure`
                // 并入格——它们是事实，不因没说完就不记。
                return finish_turn(&context, &sink, &turn, TurnExit::Aborted(partial)).await;
            }
            Err(super::port::DispatchError::Actor(super::turn_exec::TurnExecExit::Failed(e))) => {
                plugin_warn!("session", "[Session] v2 执行失败，走 Failed 出口: {e}");
                return finish_turn(&context, &sink, &turn, TurnExit::Failed(e)).await;
            }
            // 记账面的失败（行不欠 / 事实落不进去 / 端口没声明）：不给它一个
            // 「安静地不跑」的出口——那正是协议事实要堵的失败形态。
            Err(other) => {
                plugin_warn!("session", "[Session] 端口 `turn` 派发失败: {other:?}");
                return finish_turn(
                    &context,
                    &sink,
                    &turn,
                    TurnExit::Failed(crate::symbio_core::PluginError::InternalError(format!(
                        "端口 `turn` 派发失败：{other:?}"
                    ))),
                )
                .await;
            }
        };

        if turn.abort.is_aborted() {
            // 与 `TurnExecExit::Aborted` 分支同理：请求结束后才置位的 abort 标志
            // 同样向上冒泡，由消费循环统一收尾（在途 Turn → `Aborted` + error +
            // 可重试，会话结局 `aborted`）。仅 send_request 之后的 abort 冒泡；
            // turn 循环顶部的边界检查点不冒泡——上一轮已定稿落库，冒泡会把成功的
            // Turn 误回滚为失败。本轮已完整产出（`res.messages` 已并入镜像并随
            // `persist_messages` 落库），没有半截要补 ⇒ `partial` 空。
            return finish_turn(&context, &sink, &turn, TurnExit::Aborted(Vec::new())).await;
        }

        // ── 步骤 5：推理收尾（定稿内容子节点 → 校准估算 → 并入上下文）────────
        let finish =
            settle_reasoning(orchestrator, &mut context, &sink, &inputs.root_id, out).await;

        // ── 步骤 6：本轮结算 + 下一步判定（子树在此收敛）────────────────────
        let flow = close_turn(&sink, &mut context, &mut turn, finish).await;

        // ── 步骤 7：封根 Turn（子树收敛之后，全流程唯一一处）──────────────────
        // Turn 是组合节点，终态必须跟随子树；`close_turn` 归来即子树收敛
        // （ToolCall 已由执行方定格、结果子节点已就位）。发出的就是**即将落库的
        // 那条节点**，因此实时帧与存储按同一取值收敛。详见 [`finalize_turn_root`]。
        finalize_turn_root(&sink, &context, &inputs.root_id).await;

        match flow {
            // 续写：本轮还要再发一次请求，计数在这里加（**唯一**写方）。
            TurnFlow::NextTurn => turn.request_round += 1,
            TurnFlow::Finish(exit) => return finish_turn(&context, &sink, &turn, exit).await,
        }
    }
}

/// 轮首判决格的执行体（`policy.classify`）：问 `classify` → 执行判决 → 收尾。
///
/// 调用点只问「这一格欠不欠」；执行体的入参是 `SessionContext` / `TurnState`
/// 这类本层私有上下文，不是行本身（D-107：执行体留在格所在层）。
///
/// 返回 `Some(exit)` = 本轮就此收尾（`Answered` 等 `Finish` 效应）；`None` = 继续进
/// 工具循环。
async fn run_turn_head(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    turn: &mut TurnState,
    first_utterance: Option<&str>,
) -> Option<TurnExit> {
    if let Some(utterance) = first_utterance {
        // 先落成变量再进 `if let`：`decide_turn` 借的是 `&context`，而
        // `apply_verdict` 要 `&mut context`——把两次借用放进同一个表达式
        // 会让不可变借用活到整个 `if let` 体（Rust 2021 的临时值规则）。
        let decision = decide_turn(orchestrator, ctx, context, turn, utterance).await;
        // 判决事实 + 「已做」同一格：`reason` / `source` 是这次走了哪条路的
        // 唯一事实面记录，`done` 是下一问问表时不再重判的依据。落格在
        // `turn_exec`（写者令牌在那边），这里只产草稿。
        //
        // `model_calls` 是这次判决打了几次模型（D-122 决策 4）：它是那次往返
        // 在事实层唯一的**数**——判据侧拿它对账「观测 ⇔ 三格」（`source` 读不出
        // 「有没有打过模型」：模型调了而应答不可解析时 `source` 落 `fallback`）。
        turn.mark_stage(super::stage_events::StageFact::classify(
            turn.request_round,
            decision.verdict.as_ref(),
            decision.model_calls,
        ));
        if let Some(verdict) = decision.verdict {
            // 轮首判决不带运行现状：`Answered` / `Escalate` 说的是"这一轮怎么办"，
            // 而 `RunSnapshot` 说的是"干到哪一步"——只有 `Report` 读它，
            // 而 `Report` 不在轮首产生（见轮边界汇报回调）。
            let (effect, composed) = apply_verdict(
                orchestrator,
                ctx,
                context,
                turn,
                verdict.clone(),
                &RunSnapshot::default(),
            )
            .await;
            // 说了一句话（首响 / 答话）⇒ 静默锚随事实前移（`chat.assistant.opening`
            // / 收束格落格后，`report_due` 视图自会从那一刻起算），本层不再持时钟。
            if effect == VerdictEffect::Finish {
                plugin_info!(
                    "session",
                    "[Classify] 本轮不进工具循环（verdict={verdict:?}）"
                );
                // ── 缺口 5：这一轮的**轮次事实仍须入格** ──────────────────
                //
                // `Answered` 在这一步就收尾，收束格必须由 v2 运行器补落（用户格
                // 已在 `drive` 里先落）⇒ 不落收束格，下一轮 prompt 里这次问答
                // **整个消失**，`session/stats` 的轮次计数也缺收束——而
                // `messages.json` 里有（`apply_verdict` 刚 push 的答话会随
                // `finish_turn` 落库）⇒ **两条真源**。修法 = 把答话交给运行器落格
                // （见 `turn_exec::execute_final_reply_turn` 的文档）。
                // 答话 = `apply_verdict` 刚 push 的那条对话面文本（末尾一条
                // assistant 消息）。取它而不是再算一遍：措辞归 `compose` 所有，
                // 这里只搬运。
                let reply = context
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.node_role() == Some(NodeRole::Assistant))
                    .and_then(|m| match &m.content {
                        Some(MessageContent::Text(t)) => Some(t.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                // 答话**对话面节点**的身份与标记：`apply_verdict` 刚 push 的那条
                // （末尾一条 assistant）。取它而不是再算一遍：措辞归 `compose`
                // 所有，这里只搬运（同 `reply` 的取法）。`reason` /
                // `exclude_from_context` 从它的 `meta` 读——那条节点带
                // `meta.surface = "reply"`，重建据这三样复刻。
                let dialog = context
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.node_role() == Some(NodeRole::Assistant))
                    .map(|m| {
                        let meta = m.meta.as_ref();
                        super::turn_runner::DialogFacts {
                            id: m.id.clone(),
                            reason: meta
                                .and_then(|v| v.get("reason"))
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            exclude_from_context: meta
                                .and_then(|v| v.get("exclude_from_context"))
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        }
                    })
                    .unwrap_or_default();
                if let Err(e) = super::turn_exec::execute_final_reply_turn(
                    &context.session,
                    ctx,
                    orchestrator.provider.clone(),
                    turn.abort.clone(),
                    turn.turn_no,
                    turn.user_seq,
                    turn.drive_seq,
                    utterance,
                    &reply,
                    // 这次措辞往返的实测耗时：答话轮唯一的模型调用在这条产线里，
                    // 不带给运行器，收束格就恒记 0（成本台账整类漏记）。
                    composed.as_ref().map(|r| r.cost_ms).unwrap_or(0),
                    dialog,
                    // 生成这次答话的思考事实：措辞产线是这条轮次里唯一一次
                    // 真实模型推理，不入格则 `Reason` 节点从未存在。
                    composed.as_ref().and_then(|r| {
                        let text = r.reasoning.clone()?;
                        let id = r
                            .reasoning_id
                            .clone()
                            .unwrap_or_else(crate::symbio_core::llm_short_id);
                        Some((id, text))
                    }),
                    // 答话轮同样要落「本轮之前那次压缩」这一格——它不进
                    // `execute_turn`，不带上就漏了（见该函数的入参文档）。
                    context.consolidated.take().as_deref(),
                    // 判过的阶段（此刻是 classify）随答话轮一起落格：这一轮
                    // 不进 `execute_turn`，不递过去判决事实就丢了。
                    std::mem::take(&mut turn.stages_pending),
                )
                .await
                {
                    // 落格失败**只告警**：答话已在 `context.messages` 里，用户
                    // 照旧收得到。把它升级成整轮失败，等于用「记不下来」换
                    // 「答不出来」——与缺口 4 的汇报落格同一条口径。
                    plugin_warn!(
                        "session",
                        "[Session] v2 定稿答话入格失败（本轮照常收尾）: {e}"
                    );
                }
                return Some(TurnExit::Completed);
            }
            // ── ：`Escalate` 首响必须入格（本批新增）────────────────
            //
            // `Escalate` 的效应是 `Spoke`（不是 `Silent` / `Finish`）：本轮
            // **继续**进工具循环，开场白只是干活前说的一句话。轮首这里唯一
            // 能产出 `Spoke` 的判决就是 `Escalate`（`Report` 不在轮首产生，
            // 见上方注释与轮边界汇报回调）。
            //
            // 这句开场白是事实：随收束格入格（WAL 为唯一事实源，
            // D-049），不入格则重建折不出这条根级对话面节点 ⇒ t20 / t22 的
            // 「恰有一条对话面节点」全部落空。落格在 `execute_turn`（唯一持
            // 写者令牌处）——这里只把「说了什么 / 节点身份」递过去，与缺口 5
            // 的答话同一条分割（判定与措辞在 chat_loop，落格在 turn_exec）。
            if effect == VerdictEffect::Spoke {
                let node = context
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.meta.as_ref().is_some_and(|v| v.get("surface").is_some()));
                if let Some(m) = node {
                    let meta = m.meta.as_ref();
                    turn.opening_dialog = Some(super::turn_runner::DialogFacts {
                        id: m.id.clone(),
                        reason: meta
                            .and_then(|v| v.get("reason"))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        exclude_from_context: meta
                            .and_then(|v| v.get("exclude_from_context"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true),
                    });
                    turn.opening_text = Some(match &m.content {
                        Some(MessageContent::Text(t)) => t.clone(),
                        _ => String::new(),
                    });
                }
            }
        }
    }
    None
}

/// `run_chat_loop` 单轮的流向。
///
/// - `Finish`：本轮即请求终态，携带退出原因（收尾由 [`finish_turn`] 统一执行）
/// - `NextTurn`：工具轮结束，进入下一轮 LLM 请求
pub(crate) enum TurnFlow {
    Finish(TurnExit),
    NextTurn,
}

/// 主循环顶部的**唯一闸门**：退出条件。
///
/// 收口前，循环顶部与推理前后散落 4 处 `if abort_flag { ... return }`（其中两处
/// 相邻重复，中间只隔一行日志），每处各自重写收尾三件套。
/// 现在：**退出条件只在本函数判定一次**，收尾动作只在 [`finish_turn`] 出现一次。
///
/// ## 「启动条件」这一态为什么没了
///
/// 形状上闸门原本三态：放行 / **等工具齐** / 退出。「等工具齐」判的是本轮
/// 「已派发、尚未产出结果」的工具调用 id 集合，语义是「多个工具调用全部结束后才唤醒
/// 主循环，不完整不唤醒」。**那个集合（连同判它的分支）已随 v2 移除**：
/// 工具在 `turn_exec::execute_turn` **内部**跑完（`SessionDispatchPort`
/// 同步分发、批批接上），没有任何代码路径会往它里面登记 id——它是 [DE-014 步骤 5](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")
/// 说的「簿记字段」，一个恒空的集合加一条恒不命中的分支。
/// 「不完整不唤醒」这条不变量并没有被放弃，它换了承担者：**运行器的环体只在把本批
/// 结果接进 `exchange` 之后才发下一次请求**（`turn_runner.rs` 2e），
/// 结构上不可能带着半个批次出门。
///
/// ## 工具轮软上限为什么也不在这里判
///
/// 闸门能读的只有本轮的请求数，而**工具轮数在运行器的环体里**（`turn_runner.rs` 的
/// `tool_round`）。挂在这里判的是一个恒 0 的字段，v2 下永不命中——那条配置项与它的
/// 退役裁决见 [D-074](../../../../docs/003-design/014-单环一表.md#d-074-恒-0-的轮内计数退出主循环工具轮软上限整条退役抽干点搬到批次边界 "DE-014-单环一表")；将来真要这道上限，位置在 2e（结果接进 `exchange`）与
/// 2f（轮内注入）之间。
///
/// ⚠️ 顺带记下：**「历史中所有 ToolCall 都有结果子节点」从来不是判据**——
/// 「ToolCall 无结果」是**合法状态**（交互模式下工具批被用户审批中断时，本批剩余
/// ToolCall 会被有意留空，由请求视图层 `flatten_chat_messages`
/// （`plugins/model/message_builder.rs`）合成占位 tool 结果喂回模型）。
/// 按历史判定会让 approve/reject 恢复后的续写被永久挡住。
fn gate_turn(turn: &TurnState) -> Gate {
    // ── 退出条件：用户中止（边界检查点）─────────────────────────────────
    // 循环顶部的中止检查点**不冒泡**：上一轮 Turn 已定稿落库，
    // 冒泡会让消费循环的 `persist_failure` 把成功的 Turn 误回滚为 Failed。
    // 推理前后（在途 Turn 尚未定稿）的中止才走 [`TurnExit::Aborted`]。
    if turn.abort.is_aborted() {
        return Gate::Exit(TurnExit::AbortedAtBoundary);
    }

    Gate::Proceed
}

/// 主循环的**唯一收尾点**：增量落库 → 结局值。
///
/// 收口前，12 个出口各自重写「Stop 钩子 + 落库 + 返回」三件套。现在每个出口只负责
/// 判定 [`TurnExit`]，收尾动作只在本函数出现一次。
///
/// Stop 钩子**不在这里**（[D-086](../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §6）：
/// 它归会话生命周期，session 拿 [`TurnReport::last_message`] 统一触发——「恰好一次」
/// 因此仍由生命周期保证，而不是由两侧各记一次。
///
/// `persist_messages` 在锚点已对齐时是 no-op（切片为空即返回），因此所有出口都可以
/// 无条件调用——不会重复落库：`close_turn` 与压缩流程每次落库后都会同步推进
/// [`TurnState::last_saved`]。
async fn finish_turn(
    context: &SessionContext,
    sink: &ExecEventSink,
    turn: &TurnState,
    exit: TurnExit,
) -> TurnReport {
    // 增量落库（锚点已对齐时为空切片，天然 no-op）。
    persist_messages(context, turn.last_saved, sink).await;

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

#[cfg(test)]
#[path = "chat_loop.test.rs"]
mod tests;
