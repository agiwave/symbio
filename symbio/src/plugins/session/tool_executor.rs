//! 工具执行层
//!
//! 负责：
//! - 单个工具调用的路由与执行（`execute_tool_async`）
//! - 批量工具调用处理与结果广播（`process_tool_calls_async`）
//!
//! 设计说明（会话激活/恢复状态机）：
//! - 本层**不阻塞**等待任何用户输入。需要用户确认（confirm）或主动询问（ask_user）
//!   的工具会自行产出 `user_prompt` 消息节点并标记 `WaitingUserAction`，由编排层
//!   （chat_loop）在本轮结束时将会话置于 `AwaitingInput(user)`；用户答案以一条普通
//!   `user` 消息回填后，新一轮会重跑该工具。详见 USER_INPUT_MECHANISM 设计文档。

use crate::symbio_core::llm::turn::{
    build_tool_message, emit_message, emit_state, short_id, ToolCallInfo,
};
use crate::symbio_core::{dir_from_ctx, PLUGIN_SESSION};
use crate::symbio_core::{
    schemas::{
        hook::{HookEvent, HookOutput},
        session::chat_message::{
            ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
        },
    },
    InvokeRequestExt,
};
use crate::symbio_core::{
    AbortSignal, EventSink, InvokeRequest, Plugin, PluginError, PluginPayload, HOOK_FIRE,
};
use crate::{plugin_error, plugin_info, plugin_warn};
use serde_json::{json, Value};
use std::sync::Arc;

use super::tool_result_guard::{guard_tool_result, DEFAULT_TOOL_RESULT_TOKEN_CAP};

// 工具结果提取

/// 参数摘要：截断到 max_chars，用于日志打印（避免超长参数刷屏）。
///
/// 安全截断：`max_chars` 按**字节**解释但必须落在字符边界上。
/// 历史事故：中文参数（如 `write_file` 的正文）使 `&s[..200]` 落在
/// 多字节字符内部 → `tokio-rt-worker` panic → 整轮 ChatLoop 异常终止。
fn args_summary(args: &Value, max_chars: usize) -> String {
    let s = args.to_string();
    if s.len() <= max_chars {
        s
    } else {
        let end = crate::symbio_core::floor_char_boundary(&s, max_chars);
        format!("{}…(len={})", &s[..end], s.len())
    }
}

/// 工具结果里「给模型看的正文」字段名（[`extract_result`] 的第 1 条判据）。
pub const RESULT_CONTENT: &str = "content";
/// 命令类工具的结果正文字段名（`shell` 等；[`extract_result`] 的第 2 条判据）。
pub const RESULT_OUTPUT: &str = "output";

/// 从工具返回的 JSON 数据中提取可读的文本结果。
///
/// ## 判定顺序（**先命中者胜**，顺序即约定）
///
/// | 序号 | 判据 | 结果 |
/// |---|---|---|
/// | 1 | `content` 是字符串 | 它本身 |
/// | 2 | `output` 是字符串 | 它本身 |
/// | 3 | `success` 是布尔 | `true` → 整包 JSON；`false` → `Error: {error}` |
/// | 4 | 顶层就是字符串 | 它本身 |
/// | 5 | 以上都不是 | 整包 JSON |
///
/// 「先命中者胜」而不是「取第一个存在的键」：`{"content": [1,2], "output": "x"}`
/// 里 `content` 存在但不是字符串 ⇒ 第 1 条不成立，继续走到 `output`。
/// 若按「存在即取」，数组会被原样丢给模型。
///
/// ## 为什么是「猜字段」而不是「强类型结果」
///
/// 这里必须说清楚，否则下一轮很容易把它当遗留脏东西顺手「修」掉：
///
/// **结果文本本就没有契约**——工具可以返回任意 JSON（`shell` 给 `output`、
/// `vdfs_read` 给 `VdfsContent`、`ask_user` 给 `content` + `prompt`），而模型只
/// 消费**一段文本**。这层「把任意形状压成一段文本」的适配是**必要**的，不是失误。
///
/// 失误在于它曾是**隐式**的：判定顺序没写下来，加一个工具就得读源码才知道自己
/// 该返回哪个字段名。现在顺序即约定、字段名有常量、顺序有测试钉住。
///
/// ## 为什么不放 `symbio_core`
///
/// 本函数的**唯一**消费方就是本文件（生产方是各插件里的工具，它们只需要知道
/// 「写 `output` / `content`」这一个事实，不需要这段代码）。按 core 的准入规则
/// ——「只被一个模块依赖的内容不进 core」——它就该留在本模块。
///
/// 因此「工具结果字段名」这条跨插件约定**只能以文档与常量形式存在**：
/// 生产方（`local/shell.rs` 等）在自己的文档注释里声明自己写哪个字段，
/// 消费方在这里声明自己读哪个字段。这条张力是**自觉保留**的，不是疏漏——
/// 把它升成 core 里的共享类型，是用架构纯度换掉一个不存在的问题。
///
/// ## 控制流不得建立在这里
///
/// 需要「本轮结束于等待用户动作」这类判断时，用 [`crate::symbio_core::failure_kind`]
/// 这个**约定字段**（见 [`pending_prompt_from`]），不要靠猜 JSON 形状。
pub fn extract_result(data: &Value) -> String {
    if let Some(content) = data.get(RESULT_CONTENT).and_then(|v| v.as_str()) {
        return content.to_string();
    }
    if let Some(output) = data.get(RESULT_OUTPUT).and_then(|v| v.as_str()) {
        return output.to_string();
    }
    if let Some(success) = data.get("success").and_then(|s| s.as_bool()) {
        if success {
            data.to_string()
        } else {
            format!(
                "Error: {}",
                data.get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown error")
            )
        }
    } else {
        data.as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| data.to_string())
    }
}

/// Hook 事件触发工具函数
/// 触发一次生命周期钩子，返回钩子插件的输出。
///
/// **返回 `HookOutput` 而非 `Result`，是刻意的**：钩子是**旁路观察者**——它的职责是
/// 「被通知到」，不是「否决主流程」。路由失败（没有钩子插件、插件未挂载、插件自己
/// 报错）在函数内部就吞成 `HookOutput::default()`，因此调用点的 `let _ = fire_hook(...)`
/// **丢的不是错误**（没有错误可丢），只是不需要那份输出。`grep-audit` 的 S-002-bonus
/// 按 `let _ = ...await` 的**形状**判「吞错」，在这里是假阳性，故调用点带豁免留痕。
pub async fn fire_hook(
    parent: &Option<Arc<dyn Plugin>>,
    event: HookEvent,
    ctx: Arc<dyn InvokeRequest>,
) -> HookOutput {
    let p = match parent {
        Some(p) => p,
        None => return HookOutput::default(),
    };

    let session_id = ctx.get(crate::symbio_core::SESSION_ID).unwrap_or_default();

    let hook_ctx = ctx.fork();
    hook_ctx.set(crate::symbio_core::PATH, HOOK_FIRE.to_string());
    let _ = hook_ctx.set_payload(json!({
        "session_id": session_id,
        "event": serde_json::to_value(&event).unwrap_or(json!({})),
    }));

    match p.clone().route(hook_ctx).await {
        Ok(resp) => resp
            .get::<HookOutput>()
            .unwrap_or_else(|_| HookOutput::default()),
        Err(_) => HookOutput::default(),
    }
}

// 单工具执行（无阻塞）

/// 等待「用户中止」信号（工具执行期间）。
///
/// 只有一个来源：[`AbortSignal`]。它由编排层创建、经 `ctx` 注入（键
/// `symbio_core::ABORT_SIGNAL`），因此本函数不必再分辨「abort 帧 / 取消令牌 /
/// 已置位标志」——`abort()` 一次调用同时置位与唤醒。
///
/// **「对端消失」不是中止**：历史上主通道关闭曾被当作中止，于是**每一次正常收尾**
/// 都变成"用户中止"，正常完成的会话被报成 `aborted`、提示音选错音色（该 bug 曾
/// 真实发生过）。现在这条无从误读：没有通道可关闭，「没人会中止我」与「用户中止了」
/// 在类型层面就是两件事。
///
/// **返回即意味着放弃工具**：调用方用 `select!` 包着它，因此工具 future 会被
/// drop。这与既有的硬超时分支语义一致（那条路同样是 drop），不引入新的副作用
/// 类别；代价是工具可能留下半完成的副作用——但「用户按下停止」本就要求尽快放手，
/// 而让它继续跑完 600s 才是更坏的选择。
async fn wait_tool_abort(abort: &AbortSignal) {
    abort.cancelled().await;
}

/// 一次工具调用以「等待用户动作」结束时的载荷。
///
/// 工具只声明**意图与内容**（`failure_kind` + `prompt`），**不构造节点**：
/// user_prompt 节点的身份是 `result_msg_id`，而那个 id 由编排层持有
/// （占位节点也是它建的）。让工具去猜 id 正是历史上「同一逻辑节点两个 id」
/// 的成因——前端出现重复审批卡，resume 只删得掉一个，另一个永远不消失。
#[derive(Debug, Clone)]
pub struct PendingPrompt {
    /// 节点正文（审批卡 / 提问卡显示的那句话）
    pub text: String,
    /// 写进 `meta.prompt` 的载荷——卡片形状由**工具**决定（审批卡 / 提问卡不同）
    pub prompt: Value,
    /// [`crate::symbio_core::failure_kind`] 里的 pending 取值
    pub failure_kind: String,
}

/// 从工具返回的 `Data` 里读出「需要用户动作」的意图。
///
/// 判据是 `failure_kind` 这个**约定字段**（[`failure_kind::is_pending`]），
/// 不是靠猜 JSON 形状——[`extract_result`] 那种"从任意 JSON 里找 content/output"
/// 的启发式可以接受（**结果文本本就没有契约**，见其文档），但**控制流**不能建立
/// 在启发式上。
fn pending_prompt_from(data: &Value) -> Option<PendingPrompt> {
    let kind = data.get("failure_kind").and_then(|v| v.as_str())?;
    if !crate::symbio_core::failure_kind::is_pending(kind) {
        return None;
    }
    Some(PendingPrompt {
        text: data
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("需要用户操作")
            .to_string(),
        prompt: data.get("prompt").cloned().unwrap_or(Value::Null),
        failure_kind: kind.to_string(),
    })
}

/// 由载荷构造 user_prompt 节点（`WaitingUserAction`）。
///
/// **本文件是工具结果节点的唯一写入者**：普通结果（`build_tool_message`）与
/// 等待用户（本函数）都出自这里，两者共享同一套 id 约定
/// （`id = result_msg_id`、`parent_id = tool_call_id`）。
fn build_user_prompt_message(
    result_msg_id: &str,
    tool_call_id: &str,
    pending: &PendingPrompt,
) -> ChatMessage {
    ChatMessage {
        id: result_msg_id.to_string(),
        parent_id: Some(tool_call_id.to_string()),
        role: Some(MessageRole::Tool),
        msg_type: Some(MessageType::UserPrompt),
        content: Some(MessageContent::Text(pending.text.clone())),
        status: Some(MessageStatus::WaitingUserAction),
        meta: Some(json!({
            "prompt": pending.prompt,
            "failure_kind": pending.failure_kind,
        })),
        ..Default::default()
    }
}

/// 执行单个工具调用（不阻塞等待用户）。
///
/// 返回 `(result_text, success, pending)`：
/// - `result_text` / `success`：回传给 LLM 的结果（失败属信息性，照样回传）；
/// - `pending`：`Some` ⇒ 本轮结束于「等待用户动作」（审批 / 提问）。工具只给
///   **载荷**，节点由调用方构造（见 [`PendingPrompt`]）。
///
/// 事件出口（流式增量）经 `ctx` 的 `symbio_core::EVENT_SINK` 注入：工具把增量
/// 直接写到出口，本函数**不再有流式分支**——历史上工具可以返回
/// `PluginPayload::Session` 当事件流，于是本函数要解包帧、做子会话转播的二次
/// 分派、捕获冒泡的审批节点。那是跨进程传输原语被当作进程内事件流用（见
/// `symbio_core::exec` 模块头），现在返回值只有「工具结果」一种含义。
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool_async(
    parent: &Option<Arc<dyn Plugin>>,
    tool_name: &str,
    args: Value,
    tool_call_id: &str,
    sink: &EventSink,
    abort: &AbortSignal,
    result_msg_id: String,
    ctx: Arc<dyn InvokeRequest>,
) -> (String, bool, Option<PendingPrompt>) {
    let started_at = std::time::Instant::now();
    plugin_info!(
        "session",
        "[Tool] 请求发起: {} args={}",
        tool_name,
        args_summary(&args, 200)
    );

    let p = match parent {
        Some(p) => p,
        None => return ("Error: No parent plugin".into(), false, None),
    };

    // 入方向：模型给的**线上名** → 能力名。
    //
    // 判据是「注册表里谁映射到这个名字」，**不是**「把名字反演回去」：
    // `mcp__fs__read` 既可能是 `mcp.fs.read` 的线上形态，也可能本身就是
    // `mcp__fs__read`——字符串分不出这两种，集合可以。
    //
    // 收口前这里是 `tool_name.replace("__", "/")`：那条反演只在「名字里的 `__`
    // 一定是非法字符变的」时才成立，而它错得没有声音——解析到一个不存在的工具，
    // 报错信息还指着另一个名字。
    //
    // 投影函数（`tool_name::to_wire`）在 core，因为**两个模块**依赖它：model 侧
    // 4 个协议要把名字发出去，本处要把它认回来。而「认回来」这一步（下面的
    // 解析）只有本模块需要，故留在本模块，不上 trait、不进 core。
    let tool_visitor = ctx.get(crate::symbio_core::CAPABILITY_VISITOR);
    let known_names: Vec<String> = match tool_visitor.as_ref() {
        Some(v) => v
            .list_capability()
            .await
            .into_iter()
            .map(|m| m.name)
            .collect(),
        None => Vec::new(),
    };
    let resolved_name =
        crate::symbio_core::tool_name::resolve(tool_name, known_names.iter().map(String::as_str))
            .map(str::to_string);
    // 解析失败**不猜**：按原样交给路由，由它给出诚实的 NotFound。
    // 反演猜错会调起**另一个工具**，那比报错坏得多。
    let invoke_name = resolved_name
        .clone()
        .unwrap_or_else(|| tool_name.to_string());
    if resolved_name.is_none() && tool_visitor.is_some() {
        plugin_warn!(
            "session",
            "[Tool] 名字 {} 不在能力注册表里（线上名解析失败），按原样路由",
            tool_name
        );
    }

    let session_id = ctx.get(crate::symbio_core::SESSION_ID).unwrap_or_default();
    let agent_id = ctx.get(crate::symbio_core::AGENT_ID).unwrap_or_default();
    let workdir = ctx.get(crate::symbio_core::WORKDIR).unwrap_or_default();

    let tool_ctx = ctx.fork();
    tool_ctx.set(crate::symbio_core::WORKDIR, workdir);
    tool_ctx.set(crate::symbio_core::AGENT_ID, agent_id);
    tool_ctx.set(crate::symbio_core::SESSION_ID, session_id);
    tool_ctx.set(crate::symbio_core::TOOL_CALL_ID, tool_call_id.to_string());
    // 流式工具（如 shell）据此 id 广播增量帧：与 result_msg_id 占位节点同 id，
    // 前端按 role=tool 全量替换合并；最终哨兵帧被捕获为工具结果。
    tool_ctx.set(crate::symbio_core::RESULT_MSG_ID, result_msg_id.clone());
    // 执行期双原语注入 —— 工具「把事件写到哪」与「如何感知中止」的唯一来源。
    // 有了它，工具不必再用返回值里的 `PluginPayload::Session` 当事件流
    // （那是跨进程传输原语，见 `symbio_core::exec` 模块头）。
    tool_ctx.set(crate::symbio_core::EVENT_SINK, sink.clone());
    tool_ctx.set(crate::symbio_core::ABORT_SIGNAL, abort.clone());

    // 工具调用的发起（三条形态：ToolManager 命中 / 未命中回落 route /
    // 无 ToolManager 直接 route）。包成 `Box<dyn Future>` 是为了下面那个
    // 「轮询 + 空闲判定」的循环能重复借用同一份 future。
    //
    // 「命中」的判据是**解析成功**（名字在注册表里），不再是
    // `has_capability(线上名)`——后者按线上名查表，带非法字符的名字永远查不到，
    // 于是明明注册过也要走一遍 route 回落。
    let mut route_fut: std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::symbio_core::PluginPayload, PluginError>>
                + Send,
        >,
    > = if let Some(tool_visitor) = tool_visitor {
        if resolved_name.is_some() {
            plugin_info!(
                "session",
                "[Tool] Using ToolManager for: {} (能力名 {})",
                tool_name,
                invoke_name
            );
            let _ = tool_ctx.set_payload(args.clone());
            let visitor = tool_visitor.clone();
            let tool_ctx2 = tool_ctx.clone();
            let canonical = invoke_name.clone();
            Box::pin(async move { visitor.invoke(&canonical, tool_ctx2.clone()).await })
        } else {
            plugin_info!(
                "session",
                "[Tool] ToolManager does not have tool: {}, falling back to route",
                tool_name
            );
            tool_ctx.set(crate::symbio_core::PATH, invoke_name.clone());
            let _ = tool_ctx.set_payload(args.clone());
            let p2 = p.clone();
            let tool_ctx2 = tool_ctx.clone();
            Box::pin(async move { p2.route(tool_ctx2).await })
        }
    } else {
        tool_ctx.set(crate::symbio_core::PATH, invoke_name.clone());
        let _ = tool_ctx.set_payload(args.clone());
        let p2 = p.clone();
        let tool_ctx2 = tool_ctx.clone();
        Box::pin(async move { p2.route(tool_ctx2).await })
    };

    // 工具执行兜底超时：**空闲**上限（不是总时长上限）——「有进展就不算挂死」。
    //
    // 进展的判据是**出口的发射计数**：工具唯一的出方向动作就是 `emit`，因此
    // 「计数在涨」等价于「它还在干活」（流式命令的逐行快照、子智能体的过程节点…）。
    // 单次 await、从不发事件的工具（`web_search` / MCP / `codebase_search`…）
    // 读数恒为 0，于是退化为「总时长上限」——与历史口径一致。
    //
    // 为什么必须是空闲而不是总时长：`agent_run` 委托一轮可以跑十几分钟，而它
    // **一直在发事件**。总时长上限会把它与「真的挂死的工具」一并误杀——两者在
    // 「总时长」这一维上完全同形，只有「是否仍在发射」能把它们分开。
    const TOOL_EXEC_IDLE_TIMEOUT_SECS: u64 = 600; // 10 分钟无任何进展
    let progress = sink.progress();
    let mut seen_emits = progress.emitted();

    // 中止感知：工具在这里是一次 await，期间执行层看不到任何中间状态。若不与
    // 中止信号 select，用户按下停止后会连锁发生三件事：
    //   1. 工具照跑（最长到空闲超时），整个 Turn 继续推进——用户以为停了，其实没停；
    //   2. `handle_abort` 的 3s 兜底把 `is_working` 复位，消费循环在下一帧醒来时
    //      因 `!is_working` 直接 break，**跳过 `persist_failure`**，于是工具节点
    //      永远停在 `Streaming`（违反 `docs/node-state-streaming.md` §8 #11）；
    //   3. 工具返回后的 `Completed` 补丁被那个已 break 的消费循环丢弃，前端收不到。
    // 因此「中止」必须是与「工具返回」「空闲超时」并列的第三个出口。
    //
    // 优先级用 `biased` 固定：工具已返回 ⇒ 立刻采纳（哪怕同时到点）；其次是中止；
    // 最后才是空闲判定——否则一个刚好在到点瞬间返回的工具会被误判成挂死。
    let route_result = loop {
        tokio::select! {
            biased;
            r = &mut route_fut => break r,
            _ = wait_tool_abort(abort) => {
                plugin_warn!(
                    "session",
                    "[Tool] 执行期间被用户中止: {} (耗时 {}ms, call_id={})",
                    tool_name,
                    started_at.elapsed().as_millis(),
                    tool_call_id
                );
                return (
                    format!("Error: 工具 {} 被用户中止，未返回结果", tool_name),
                    false,
                    None,
                );
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(TOOL_EXEC_IDLE_TIMEOUT_SECS)) => {
                let now = progress.emitted();
                if now != seen_emits {
                    // 有进展（工具在发事件）→ 重新计时，不算挂死
                    seen_emits = now;
                    continue;
                }
                plugin_error!(
                    "session",
                    format!(
                        "[Tool] 执行空闲超时 ({}s 无任何进展): {}，已中断。call_id={}",
                        TOOL_EXEC_IDLE_TIMEOUT_SECS, tool_name, tool_call_id
                    )
                );
                return (
                    format!(
                        "Error: 工具 {} 已 {} 秒无任何进展，已强制中断（疑似挂死）",
                        tool_name, TOOL_EXEC_IDLE_TIMEOUT_SECS
                    ),
                    false,
                    None,
                );
            }
        }
    };

    match route_result {
        Ok(resp) => match resp {
            // ── 即时响应 ──────────────────────────────────────────────────────
            PluginPayload::Data(_) => {
                let data = match resp.get::<serde_json::Value>() {
                    Ok(d) => d,
                    Err(_) => {
                        return ("Error: Failed to deserialize payload".into(), false, None);
                    }
                };
                // plugin_debug!(
                //     "session",
                //     "Tool immediate response for {}: {}",
                //     tool_name,
                //     data
                // );

                // 工具只声明意图：`failure_kind` 落在 pending 闭集里 ⇒ 本轮结束于
                // 等待用户动作，节点由调用方构造（工具不碰 id）。
                let pending = pending_prompt_from(&data);
                let res = extract_result(&data);
                plugin_info!(
                    "session",
                    "[Tool] 正常结束: {} (耗时 {}ms, 结果长度 {}, pending={})",
                    tool_name,
                    started_at.elapsed().as_millis(),
                    res.len(),
                    pending
                        .as_ref()
                        .map(|p| p.failure_kind.as_str())
                        .unwrap_or("-")
                );
                (res, true, pending)
            }

            _ => ("Error: Unexpected payload type".into(), false, None),
        },
        Err(e) => {
            plugin_error!(
                "session",
                format!(
                    "[Tool] ROUTE Error: {} (耗时 {}ms)",
                    e,
                    started_at.elapsed().as_millis()
                )
            );
            (format!("Error: {e}"), false, None)
        }
    }
}

// 批量工具调用处理

/// 记录协议级工具调用失败（工具调用 id/name 缺失或非法）。
///
/// 不再简单跳过：跳过会让已落库的 ToolCall 节点没有结果子节点，
/// 下一轮请求携带"无结果的 tool_call"触发 provider 400（Bug 2 同类问题）。
/// 处理口径与普通工具执行失败一致（失败属信息性）：
/// - 生成一条 `role=Tool` 的错误结果子节点（Completed + success=false），错误内容喂回 LLM；
/// - 生成父 ToolCall 节点的失败补丁（failure_kind=error）并广播。
///
/// 两条消息分别加入 tool_messages / parent_updates，由调用方统一持久化。
async fn record_protocol_failure(
    sink: &EventSink,
    tool_call_id: &str,
    error_text: &str,
    tool_messages: &mut Vec<ChatMessage>,
    parent_updates: &mut Vec<ChatMessage>,
    context_messages: &[ChatMessage],
) {
    let result_msg_id = uuid::Uuid::new_v4().to_string();
    let mut tool_msg = build_tool_message(
        tool_call_id,
        &format!("Error: {error_text}"),
        Some(false),
        Some(result_msg_id),
    );
    // 失败属信息性：结果以 Completed 留在上下文（Failed 会被 get_context_messages
    // 过滤，导致"孤儿 tool 结果"使下一轮 LLM 请求非法）。
    tool_msg.status = Some(MessageStatus::Completed);

    emit_message(sink, tool_msg.clone()).await;

    // 父 ToolCall 终态——**完整快照**，两种情形：
    // - id 合法但 name/参数非法：ToolCallDelta 已广播过完整节点（在权威转写里），
    //   取副本应用终态；
    // - id 本身缺失（兜底 short_id）：不存在任何父节点——构造**完整**的 ToolCall
    //   终态节点（name/参数为 None 是诚实表达），`close_turn` 会把它补进转写并
    //   落库，结果子节点因此有真实的父节点，不再悬空。
    let parent_update = match context_messages.iter().find(|m| m.id == tool_call_id) {
        Some(p) => {
            let mut full = p.clone();
            full.status = Some(MessageStatus::Completed);
            let mut meta = full.meta.clone().unwrap_or_else(|| json!({}));
            if let Some(obj) = meta.as_object_mut() {
                obj.insert("success".into(), json!(false));
                obj.insert("failure_kind".into(), json!("error"));
            }
            full.meta = Some(meta);
            full.error = Some(error_text.to_string());
            full
        }
        None => ChatMessage {
            id: tool_call_id.to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            status: Some(MessageStatus::Completed),
            error: Some(error_text.to_string()),
            meta: Some(json!({
                "success": false,
                "failure_kind": "error",
            })),
            ..Default::default()
        },
    };
    emit_state(sink, parent_update.clone()).await;

    plugin_info!(
        "session",
        "[Tool] Protocol failure recorded as failed tool call: {}",
        error_text
    );

    tool_messages.push(tool_msg);
    parent_updates.push(parent_update);
}

/// 广播父 ToolCall 的「执行中」状态（`Streaming`）。
///
/// ## 为什么需要它
///
/// ToolCall 节点的生命周期**不在模型停止输出参数时结束**——那只是"参数齐了"。
/// 它此后还要经历一段**执行窗口**：一次编译、一次网络请求、一个子智能体跑完，
/// 往往比参数流式本身长得多。若此处不下发状态，前端在整段窗口里没有任何
/// 「运行中」迹象：参数流完画面静止，直到结果突然出现——用户无法判断
/// 是"还在跑"还是"卡死了"。
///
/// 状态机（`docs/node-state-streaming.md` §2.3）：
/// `pending → streaming（参数流式 + 执行）→ waiting_user_action → completed / failed`。
/// 因此 `Streaming` 在这里**不是**"正在接收流"，而是"这次调用正在跑"。
///
/// 同时写入 `meta.started_at`（毫秒）：前端据此显示"已运行 47s"——「运行中」是断言，
/// 时长才是**判据**，用户靠它区分"还在跑"与"卡住了"。用节点属性承载而不是前端
/// 自己计时，切会话/重连后时长仍然连续（前端计时会从零重来）。
async fn emit_tool_running(sink: &EventSink, context_messages: &[ChatMessage], tool_call_id: &str) {
    // 完整快照：从权威转写取父 ToolCall 副本（id / 父子关系 / name / 参数都在），
    // 应用「运行中」状态与 meta.started_at 后整条广播。找不到 = 协议违例
    // （ToolCallDelta 必然先广播过完整节点），报错并跳过——不造半截节点。
    let Some(parent) = context_messages.iter().find(|m| m.id == tool_call_id) else {
        plugin_error!(
            "session",
            "[Tool] emit_tool_running: 转写中不存在工具调用 {}（协议违例），跳过运行态广播",
            tool_call_id
        );
        return;
    };
    let mut running = parent.clone();
    running.status = Some(MessageStatus::Streaming);
    let mut meta = running.meta.clone().unwrap_or_else(|| json!({}));
    if let Some(obj) = meta.as_object_mut() {
        obj.insert("started_at".into(), json!(crate::symbio_core::now_ms()));
    }
    running.meta = Some(meta);
    emit_state(sink, running).await;
}

/// 把父 ToolCall 的**完整副本**定稿为终态并广播。
///
/// 完整快照纪律：从权威转写取副本（id / 父子关系 / name / 参数都在）→
/// 应用状态、合并 meta、设置 error → 整条广播。找不到 = 协议违例
/// （ToolCallDelta 必然先广播过完整节点），报错并返回 `None`——调用方跳过
/// 父状态帧，结果子节点照常给出。返回值是已广播的完整消息，调用方把它
/// 收进 `parent_updates` 供内存镜像同步与落库（整条替换，非补丁合并）。
async fn emit_parent_finalized(
    sink: &EventSink,
    context_messages: &[ChatMessage],
    tool_call_id: &str,
    status: MessageStatus,
    meta_extra: Value,
    error: Option<String>,
) -> Option<ChatMessage> {
    let Some(parent) = context_messages.iter().find(|m| m.id == tool_call_id) else {
        plugin_error!(
            "session",
            "[Tool] 终态广播: 转写中不存在工具调用 {}（协议违例），跳过父状态帧",
            tool_call_id
        );
        return None;
    };
    let mut full = parent.clone();
    full.status = Some(status);
    // meta 在**发射端**并入权威副本（读-改-写发生在状态所有者处），
    // 广出去的 meta 因此是完整对象，接收端只做整条替换。
    let mut meta = full.meta.clone().unwrap_or_else(|| json!({}));
    if let (Some(obj), Some(new)) = (meta.as_object_mut(), meta_extra.as_object()) {
        for (k, v) in new {
            obj.insert(k.clone(), v.clone());
        }
    } else {
        meta = meta_extra;
    }
    full.meta = Some(meta);
    full.error = error;
    // 终态帧只带状态 / 元数据 / 错误，**不带正文**：ToolCall 的参数正文已在
    // 流式阶段以 delta 逐帧上线。
    emit_state(sink, full.clone()).await;
    Some(full)
}

/// 本批**未执行**的工具调用的终态。`reason` 区分成因，写进 `meta.failure_kind`：
///
/// - `"not_executed"`：批次没轮到它（用户中止 / 交互模式下前一个工具待用户恢复）；
/// - `"blocked"`：被 `PreToolUse` 钩子拦下（已有 `Blocked:` 结果子节点）。
///
/// 两条路径都必须给出终态，否则节点会永远停在 `Streaming`——前端一直转"运行中"。
/// （`Streaming` 是瞬态状态：只经广播通道下发，持久层拒绝落盘——见
/// `chat_session.rs::ensure_durable_states`。停在 Streaming 的节点是**广播层**的
/// 收口缺口，`converge_inflight` 负责兜住。）
///
/// ## 为什么是 `Completed` 而不是 `Failed`
///
/// `Failed` 的语义是"以错误结束"，而"没轮到执行"与"被策略拦下"都不是错误。
/// 标 `Failed` 还会连带两个后果：前端渲染 ⚠ 错误条（用户以为工具真的失败了），以及
/// `get_context_messages` 过滤掉父节点后留下孤儿结果子节点（下一轮请求非法）。
/// 与既有口径一致——工具失败属**信息性**，父节点一律 `Completed`，
/// 差异由 `meta.failure_kind` 承载。
///
/// 可见性 `pub(super)`：`resume.rs` 要用同一份定义，否则同一语义会长出第二个写法。
/// 就地应用于**完整消息副本**（发射端自己组装好终态快照再广播），不再返回
/// 「只有 id + status 的补丁」——那需要接收端猜意图，正是已删除的补丁语义。
pub(super) fn apply_not_executed(parent: &mut ChatMessage, reason: &str) {
    parent.status = Some(MessageStatus::Completed);
    let mut meta = parent.meta.clone().unwrap_or_else(|| json!({}));
    if let Some(obj) = meta.as_object_mut() {
        obj.insert("success".into(), json!(false));
        obj.insert("failure_kind".into(), json!(reason));
    }
    parent.meta = Some(meta);
}

/// 「工具调用未执行」的**结果子节点**——与 [`apply_not_executed`] 成对使用。
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
/// `reason` 与 [`not_executed_patch`] 同源（成因可区分，事后排查不必靠猜）：
/// 中止 / 批次跳过。
///
/// ## 为什么是 `Completed`
///
/// 与其它工具结果同口径：「没跑」不是「跑失败」。`Failed` 结果会被
/// `get_context_messages` 当失败处理，且前端会在结果节点上再渲染一条 ⚠ 错误条，
/// 而父节点的交代由 `meta.failure_kind` 承载。
pub(super) fn not_executed_result(tool_call_id: &str, reason: &str) -> ChatMessage {
    let text = match reason {
        "aborted" => "本次调用已中止（用户终止），未执行，未产生结果。".to_string(),
        _ => "本次调用未执行：同一批中排在它之前的工具未正常结束（失败或需要用户处理），\
              同批剩余调用被一并跳过。需要时请重新发起这一次调用。"
            .to_string(),
    };
    let mut msg = build_tool_message(tool_call_id, &text, Some(false), None);
    // 与 `record_protocol_failure` 同口径：失败属**信息性**，结果以 Completed
    // 留在上下文（标 Failed 会被上下文过滤，并让前端多渲染一条 ⚠）。
    msg.status = Some(MessageStatus::Completed);
    msg.meta = Some(json!({
        "success": false,
        "failure_kind": reason,
    }));
    msg
}

/// 顺序处理一批工具调用，向 channel 广播每个工具的结果，
/// 并返回 `(tool_messages, parent_updates)`：
/// - `tool_messages`：工具结果子节点（用于追加到对话历史）
/// - `parent_updates`：ToolCall 父节点的**完整终态快照**（发射端从
///   `context_messages` 取权威副本应用终态，整条广播），供调用方做内存镜像
///   整条替换并随 `persist_messages` 落库。
///
/// 交互模式（interactive）下，若前一个工具产出 user_prompt（待审批/询问）或失败，
/// 则中止本批剩余工具（用户需逐个处理）；auto 模式不中止，失败结果传 LLM 继续。
///
/// ## 两条不变量（本函数负责保证）
///
/// 1. **本批每一个工具调用都必须以终态收场**（`Completed` / `WaitingUserAction`）：
///    调用前广播 `Streaming`（[`emit_tool_running`]），调用后广播终态；未执行的
///    （阻塞 / 中止 / 交互中断）在函数末尾统一收口（`failure_kind = "not_executed"`）。
///    漏掉任何一条，前端就会有一个永远转下去的「运行中」。
/// 2. **每一个工具调用都必须有结果子节点**（`tool_messages` 里一条 role=Tool）——
///    「结果」在树里是子节点，前端按它渲染响应段。任何一条 return 分支
///    （参数解析失败 / 未执行 / 中止 / 被钩子拦下）都必须在给出父节点终态的同时
///    给出结果：只有父状态没有结果，用户看到的就是「调用凭空消失、会话照旧往下走」。
///    见 [`not_executed_result`]。
#[allow(clippy::too_many_arguments)]
pub async fn process_tool_calls_async(
    tool_calls: Vec<ToolCallInfo>,
    parent: &Option<Arc<dyn Plugin>>,
    sink: &EventSink,
    abort: &AbortSignal,
    ctx: Arc<dyn InvokeRequest>,
    context_messages: &[ChatMessage],
) -> (Vec<ChatMessage>, Vec<ChatMessage>) {
    let mut tool_messages = Vec::new();
    // 父 ToolCall 终态——**完整消息**（发射端从权威转写取副本应用终态）。
    // 返回给调用方做内存镜像整条替换 + 随 persist_messages 落库。
    let mut parent_updates: Vec<ChatMessage> = Vec::new();
    if tool_calls.is_empty() {
        return (tool_messages, parent_updates);
    }

    let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();

    plugin_info!(
        "session",
        "[Tool] 处理 {} 个工具调用（mode={}）",
        tool_calls.len(),
        mode
    );

    // 本批全部工具调用 id。循环按值消费 `tool_calls`，而"哪些没被执行"要在循环
    // **之后**才知道（break 出口），故先留一份 id 清单供末尾收口。
    let batch_ids: Vec<String> = tool_calls
        .iter()
        .filter_map(|tc| tc.id.as_ref())
        .filter(|id| !id.trim().is_empty())
        .cloned()
        .collect();

    for tc in tool_calls {
        if abort.is_aborted() {
            break;
        }
        // 交互模式下，前一个工具待审批/失败 → 中止本批剩余（用户需逐个处理）
        if mode == "interactive" && !parent_updates.is_empty() {
            let last_blocked = parent_updates
                .last()
                .map(|p| {
                    p.status == Some(MessageStatus::WaitingUserAction)
                        || p.meta
                            .as_ref()
                            .and_then(|m| m.get("failure_kind"))
                            .and_then(|v| v.as_str())
                            .map(|k| {
                                // pending 两种 + error 都算「本批剩余不要继续跑」
                                crate::symbio_core::failure_kind::is_pending(k)
                                    || k == crate::symbio_core::failure_kind::ERROR
                            })
                            .unwrap_or(false)
                })
                .unwrap_or(false);
            if last_blocked {
                plugin_info!(
                    "session",
                    "[Tool] 交互模式下前一个工具待用户恢复，中止本批剩余工具"
                );
                break;
            }
        }

        // 工具调用 id 缺失/非法 → 作为工具调用失败处理（不跳过）。
        // 正常情况下 ToolCallAccumulator 已保证 id 非空；此分支为兜底防御。
        // 注意：兜底 id 仅用于挂载失败结果与父节点补丁（保持结构完整）。
        let id = match tc.id.as_ref() {
            Some(id) if !id.trim().is_empty() => id.clone(),
            _ => {
                plugin_error!(
                    "session",
                    "[Tool] 协议错误：工具调用 ID 缺失/非法，记为失败调用"
                );
                record_protocol_failure(
                    sink,
                    &short_id(),
                    "模型未返回有效的工具调用 ID（协议错误）",
                    &mut tool_messages,
                    &mut parent_updates,
                    context_messages,
                )
                .await;
                continue;
            }
        };
        // 工具名缺失/非法 → 同样作为工具调用失败处理（不跳过），
        // 避免已落库的 ToolCall 节点没有结果子节点。
        let name = match tc.name.as_ref() {
            Some(name) if !name.trim().is_empty() => name.clone(),
            _ => {
                plugin_error!("session",
                    format!(
                        "Protocol Error: Tool call name missing/invalid, recording as failed tool call. ID: {id}"
                    )
                );
                record_protocol_failure(
                    sink,
                    &id,
                    "模型未返回有效的工具名称（协议错误）",
                    &mut tool_messages,
                    &mut parent_updates,
                    context_messages,
                )
                .await;
                continue;
            }
        };

        // 参数 JSON 非空却解析失败（典型：被 max_tokens 截断）→ **拒绝执行**。
        // 旧行为是带着占位 `{}` 继续执行，工具必然报「缺少必填参数」，而这条错误
        // 对模型毫无信息量（它认为自己发了完整参数），于是原样重试 → 卡思考死循环。
        // 这里以协议错误形态回报，明确告诉模型「参数残破，需重发完整调用」。
        if let Some(raw) = tc.parse_error.as_ref() {
            let preview: String = raw.chars().take(400).collect();
            let truncated = raw.chars().count() > 400;
            plugin_error!(
                "session",
                "[Tool] 协议错误：工具调用参数 JSON 解析失败，拒绝执行。ID: {id}, name: {name}"
            );
            record_protocol_failure(
                sink,
                &id,
                &format!(
                    "工具参数 JSON 解析失败（可能被长度上限截断），本次调用未执行。\
                     只有完整且可解析的 JSON 参数才会被执行，请勿以相同内容重试。\n\
                     参数原文（{} 字符{}）：{preview}",
                    raw.chars().count(),
                    if truncated { "，已截断展示" } else { "" }
                ),
                &mut tool_messages,
                &mut parent_updates,
                context_messages,
            )
            .await;
            continue;
        }

        let result_msg_id = uuid::Uuid::new_v4().to_string();

        let pre_output = fire_hook(
            parent,
            HookEvent::PreToolUse {
                tool_name: name.clone(),
                tool_input: tc.arguments.clone(),
            },
            ctx.clone(),
        )
        .await;
        if !pre_output.should_proceed {
            let block_msg = pre_output
                .block_reason
                .unwrap_or_else(|| "Blocked by pre hook".to_string());
            plugin_warn!(
                "session",
                "[Tool] BLOCKED by PreToolUse hook: {}",
                block_msg
            );
            let tool_msg = build_tool_message(
                &id,
                &format!("Blocked: {block_msg}"),
                Some(false),
                Some(result_msg_id.clone()),
            );
            tool_messages.push(tool_msg);
            // 被钩子拦下 → 父节点同样必须收敛（此前只推了结果子节点，父节点
            // 靠 `finalize_assistant_turn` 的 Completed 兜着；那条兜底已移除）。
            if let Some(parent_update) = emit_parent_finalized(
                sink,
                context_messages,
                &id,
                MessageStatus::Completed,
                json!({ "success": false, "failure_kind": "blocked" }),
                None,
            )
            .await
            {
                parent_updates.push(parent_update);
            }
            continue;
        }

        // 工具即将真正执行 → 父 ToolCall 置「运行中」。这是整段执行窗口的**唯一**
        // 运行中信号来源，必须紧贴 `execute_tool_async` 之前（`finalize_assistant_turn`
        // 已不再在参数流结束时提前定格）。
        emit_tool_running(sink, context_messages, &id).await;

        let (res, success, mut pending_user_prompt) = execute_tool_async(
            parent,
            &name,
            tc.arguments.clone(),
            &id,
            sink,
            abort,
            result_msg_id.clone(),
            ctx.clone(),
        )
        .await;

        let final_res = res;

        let tool_output = if success {
            serde_json::json!({ "content": final_res.clone() })
        } else {
            serde_json::json!({ "error": final_res.clone() })
        };
        let _post_output = fire_hook(
            parent,
            HookEvent::PostToolUse {
                tool_name: name.clone(),
                tool_input: tc.arguments.clone(),
                tool_output,
            },
            ctx.clone(),
        )
        .await;

        // 工具以「等待用户动作」结束时，**节点在这里构造**：本函数是工具结果节点的
        // 唯一写入者（它持有 `result_msg_id` 与父 ToolCall 的终态）。工具只给载荷
        // （`PendingPrompt`），因此不存在"工具造一个节点、这里再造一个"的双 id 问题。
        let mut tool_msg = if let Some(pending) = pending_user_prompt.take() {
            build_user_prompt_message(&result_msg_id, &id, &pending)
        } else {
            // L0 守卫：超长工具结果存档 + head/tail 摘要，避免单条撑爆上下文窗口
            // （对应"单次工具调用内容太长"的压缩诉求；物理字节上限不再是唯一防线）。
            // 传入 session_id：存档跟随会话目录（tool_archives/），历史可取回不被 OS 清理。
            let guard_session = ctx.get(crate::symbio_core::SESSION_ID).unwrap_or_default();
            // 会话存储根 = 本插件自己的目录（装配态由父插件经 `PLUGIN_DIR` 告知）
            let storage_root = dir_from_ctx(&*ctx, PLUGIN_SESSION);
            let guarded = guard_tool_result(
                &final_res,
                DEFAULT_TOOL_RESULT_TOKEN_CAP,
                Some(&guard_session),
                storage_root.dir(),
            );
            let mut tool_msg =
                build_tool_message(&id, &guarded.text, Some(success), Some(result_msg_id));
            if guarded.truncated {
                let mut meta = tool_msg.meta.clone().unwrap_or_else(|| json!({}));
                meta["tool_result_truncated"] = json!(true);
                if let Some(p) = guarded.archive_path {
                    meta["archive_path"] = json!(p);
                }
                meta["origin_tokens"] = json!(guarded.original_tokens);
                tool_msg.meta = Some(meta);
            }
            tool_msg
        };
        // 任何模式：工具失败属"信息性"，结果仍以合法 tool 结果（Completed）留在上下文，
        // 让 LLM 看到错误并继续；其父节点在下方也标 Completed（不暂停会话）。
        // 若此处仍标 Failed，则会被 get_context_messages 过滤，导致"孤儿 tool 结果"
        // （父 tool_call 被过滤、结果残留）使下一轮 LLM 请求非法（Bug 2 同类问题）。
        // 仅对普通工具结果生效（user_prompt 走 WaitingUserAction 分支，不受此覆盖）。
        if !success && tool_msg.msg_type != Some(MessageType::UserPrompt) {
            tool_msg.status = Some(MessageStatus::Completed);
        }

        if tool_msg.msg_type == Some(MessageType::UserPrompt) {
            // user_prompt 节点本身即是工具"结果"（待用户审批/回答）：
            // 广播该节点（WaitingUserAction），并把父节点 ToolCall 标为
            // WaitingUserAction（持久化 failure_kind 供 resume 提取）。
            let failure_kind = tool_msg
                .meta
                .as_ref()
                .and_then(|m| m.get("failure_kind"))
                .and_then(|v| v.as_str())
                .unwrap_or(crate::symbio_core::failure_kind::NEEDS_APPROVAL)
                .to_string();

            emit_message(sink, tool_msg.clone()).await;

            // 父 ToolCall 置 WaitingUserAction（完整快照；meta.failure_kind 供 resume 提取）
            if let Some(parent_update) = emit_parent_finalized(
                sink,
                context_messages,
                &id,
                MessageStatus::WaitingUserAction,
                json!({
                    "success": false,
                    "failure_kind": failure_kind,
                }),
                None,
            )
            .await
            {
                parent_updates.push(parent_update);
            }
        } else {
            // 广播工具结果子节点：**一次性节点单帧完成**——内容以 delta 首次传输，
            // 与状态、meta 同帧（meta 一并带全：截断标记 / 存档路径不再只活在存储里）。
            emit_message(sink, tool_msg.clone()).await;

            // 标记父节点最终状态（完整快照）：
            // - 成功 => Completed
            // - 普通工具失败 => Completed（失败属信息性，错误结果作为合法 tool 结果留在
            //   上下文，父节点不再标 Failed，不暂停会话、不触发重试/补参渲染）。
            //   仅真正需要用户输入的 UserPrompt 场景在上方以 WaitingUserAction 处理。
            let (status, meta_extra, error) = if success {
                (MessageStatus::Completed, json!({ "success": true }), None)
            } else {
                (
                    MessageStatus::Completed,
                    json!({
                        "success": false,
                        "failure_kind": "error",
                        "tool_name": name,
                        "args": tc.arguments,
                    }),
                    Some(final_res.clone()),
                )
            };
            if let Some(parent_update) =
                emit_parent_finalized(sink, context_messages, &id, status, meta_extra, error).await
            {
                parent_updates.push(parent_update);
            }
        }

        tool_messages.push(tool_msg);
    }

    // ── 收口：本批未执行的工具调用不得停在 `Streaming` ────────────────────
    // 两条路径会走到这里：① 用户中止；② 交互模式下前一个工具待用户恢复，
    // 本批剩余被 break 掉。它们从未经过 `emit_tool_running`，但参数流式阶段
    // 已经把它们广播成 `Streaming`——不定格就是前端一个永远转下去的「运行中」。
    for id in &batch_ids {
        if parent_updates.iter().any(|p| p.id == *id) {
            continue;
        }
        // **结果子节点 + 父节点补丁，一个都不能少**（顺序与正常路径一致：
        // 先结果、后父状态）。只发父节点补丁会让卡片有请求、无响应——
        // 那正是「工具没有响应节点，会话却继续往后」的成因。
        let result_msg = not_executed_result(id, "not_executed");
        emit_message(sink, result_msg.clone()).await;
        if let Some(parent_update) = emit_parent_finalized(
            sink,
            context_messages,
            id,
            MessageStatus::Completed,
            json!({ "success": false, "failure_kind": "not_executed" }),
            None,
        )
        .await
        {
            parent_updates.push(parent_update);
        }
        tool_messages.push(result_msg);
    }

    (tool_messages, parent_updates)
}

#[cfg(test)]
#[path = "tool_executor.test.rs"]
mod tests;
