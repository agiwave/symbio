//! 阶段③「未执行收口」——`process_tool_calls_async` 三段拆分的第三段
//! （T-179 / [PL-015 §5 动作 B](../../../../../../docs/004-plan/015-可扩充性与插件切分.md "PL-015-可扩充性与插件切分")）。
//!
//! 两条路径会让批内留下没跑到的调用（用户中止 / 交互模式前序待用户）；本段给它们
//! 统一补「结果子节点 + 父节点终态」，并如实记未履约的承诺。收口依据是阶段①的
//! 本批留底（[`GateReadings::batch_meta`]），判据是阶段②已收口的父节点集合。
//!
//! 判据（「这段错了谁会红」）：未执行收口用例红——`not_executed` / `aborted` 的
//! 结果子节点与父终态断言（不变量 1「每个调用以终态收场」与不变量 2「有调用必有结果」）。

use super::*;

/// 「执行层没跑到」的两个成因——**结果节点的 `meta.failure_kind` 与端口的
/// `failed{why}` 用同一份词表**（写方只有 [`close_unfinished`] 一处）。
pub const ABORTED: &str = "aborted";
pub const NOT_EXECUTED: &str = "not_executed";

/// 这条 `failure_kind` 是不是「没跑到」的占位（端口关账读它）。
///
/// ## 为什么端口要看节点上的成因
///
/// 「这次调用到底有没有执行」只在收口那一刻显形——[`close_unfinished`] 数的就是
/// 「本批没定稿父节点的那几笔」，而它把成因写在结果节点的 `meta.failure_kind` 上。
/// 占位结果是**框架的交代**（给模型一句话、给前端一张卡），不是工具的应答：
/// 端口若按「`outcomes` 里有这个 id 就算应答」关账，中止的调用会落成 `response`——
/// 日志说「回来了」，而它从未执行，`await-tools` 的 `failed` 出口于是只有 resume
/// 一条路径走过（PL-017 判据 6 的「中止唤醒」在事实面上等于没接）。
///
/// 反过来，工具**跑完而失败**（`error` / `overflow_dropped` / 闸门拒绝的
/// `refusal.kind`）是应答：工具确实回了话，关账仍落 `response`。
///
/// 可见性 `pub`：跨域取用经 `tools` 的门面（`tool_dispatch` 是编排层，不深潜本子模块）。
pub fn is_unanswered(failure_kind: &str) -> bool {
    failure_kind == ABORTED || failure_kind == NOT_EXECUTED
}

/// 「工具调用未执行」的**结果子节点**——与父节点终态补丁成对使用（补丁在
/// [`close_unfinished`] 里落：`success: false` + `failure_kind`）。
///
/// ## 为什么父节点补丁不够（本函数存在的全部理由）
///
/// 「这次调用的结果」在树里是一条**子节点**：前端 `ToolCallNode` 的「结果」段按子节点
/// 渲染（`v-if="resultChildren.length"`）。只补父节点 ⇒ 卡片有请求、**没有响应**——
/// 用户看到一次调用凭空消失，而会话照常往下走（下一轮照常发请求）。
///
/// 更隐蔽的是「模型看得见、用户看不见」：请求视图里有 `flatten_chat_messages` 的占位
/// 兜底（否则 provider 直接 400），因此**请求包始终合法**，问题只在存储与 UI 上——
/// 这正是它能长期潜伏而不被任何测试拦住的原因。
///
/// 因此「**有调用必有结果**」这条不变量必须在**存储**里成立（本函数写入 `tool_messages`
/// ⇒ 落库 + 广播 + 进下一轮上下文），而不是只在请求视图里成立。
///
/// `reason` 与端口侧 `failed{why}` 用同一份词表（[`ABORTED`] / [`NOT_EXECUTED`]，
/// 成因可区分，事后排查不必靠猜）：中止 / 批次跳过。
///
/// ## 为什么是 `Completed`
///
/// 与其它工具结果同口径：「没跑」不是「跑失败」。`Failed` 结果会被
/// `get_context_messages` 当失败处理，且前端会在结果节点上再渲染一条 ⚠ 错误条，
/// 而父节点的交代由 `meta.failure_kind` 承载。
pub(super) fn not_executed_result(tool_call_id: &str, reason: &str, tool_name: &str) -> ChatNode {
    let text = if reason == ABORTED {
        "本次调用已中止（用户终止），未执行，未产生结果。".to_string()
    } else {
        "本次调用未执行：同一批中排在它之前的工具未正常结束（失败或需要用户处理），\
              同批剩余调用被一并跳过。需要时请重新发起这一次调用。"
            .to_string()
    };
    let mut msg = llm_build_tool_message(tool_call_id, &text, Some(false), None, Some(tool_name));
    // 与 `record_protocol_failure` 同口径：失败属**信息性**，结果以 Completed
    // 留在上下文（标 Failed 会被上下文过滤，并让前端多渲染一条 ⚠）。
    msg.status = Some(NodeStatus::Completed);
    // **并入**而非整格替换：`meta.tool_name` 是结果节点身份的一部分（[T-147d-iii]），
    // 整格覆盖会连它一起丢掉。
    if let Some(obj) = msg.meta.as_mut().and_then(|m| m.as_object_mut()) {
        obj.insert("failure_kind".into(), json!(reason));
    }
    msg
}

/// 未执行收口（阶段③）：以下收口循环逐字来自拆分前 `process_tool_calls_async` 的末尾段。
// 同 `batch::run_batch`：`&decls` 逐字保留自拆分前代码，needless_borrow 是对纯结构拆分的误报。
#[allow(clippy::needless_borrow)]
pub(crate) async fn close_unfinished(
    readings: &GateReadings,
    abort: &ExecAbortSignal,
    sink: &ExecEventSink,
    context_messages: &[ChatNode],
    tool_messages: &mut Vec<ChatNode>,
    parent_updates: &mut Vec<ChatNode>,
    delegations: &mut Vec<Delegation>,
) {
    let decls = &readings.decls;

    // ── 收口：本批未执行的工具调用不得停在 `Streaming` ────────────────────
    // 两条路径会走到这里：① 用户中止；② 交互模式下前一个工具待用户恢复，
    // 本批剩余被 break 掉。它们从未经过 `emit_tool_running`，但参数流式阶段
    // 已经把它们广播成 `Streaming`——不定格就是前端一个永远转下去的「运行中」。
    for (id, name, args) in &readings.batch_meta {
        if parent_updates.iter().any(|p| p.id == *id) {
            continue;
        }
        // **结果子节点 + 父节点补丁，一个都不能少**（顺序与正常路径一致：
        // 先结果、后父状态）。只发父节点补丁会让卡片有请求、无响应——
        // 那正是「工具没有响应节点，会话却继续往后」的成因。
        //
        // reason 必须分辨中止：`6cf9520d` 号会话的 56 条占位全写成了
        // `not_executed`（批被用户中止，收口却报「同批前序未结束」）——
        // 成因被谎报，事后排查就得靠猜。中止即 `aborted`，其余才是批跳过。
        let reason = if abort.is_aborted() {
            ABORTED
        } else {
            NOT_EXECUTED
        };
        let result_msg = not_executed_result(id, reason, name);
        llm_emit_message(sink, result_msg.clone()).await;
        if let Some(parent_update) = emit_parent_finalized(
            sink,
            context_messages,
            id,
            NodeStatus::Completed,
            json!({ "success": false, "failure_kind": reason }),
            None,
        )
        .await
        {
            parent_updates.push(parent_update);
        }
        tool_messages.push(result_msg);
        // 没跑到同样是没履约（`agent_run` 的约不能因为"没执行"就查无此约）。
        note_delegation(
            delegations,
            &decls,
            id,
            name,
            args,
            false,
            "未执行（中止或同批前序未结束）",
        );
    }
}
