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

use symbio_core::TurnToolCallInfo;

use serde_json::{json, Value};
use std::sync::Arc;
use symbio_core::{
    chat_node::{ChatNode, MessageContent, NodeKind, NodeStatus},
    Event, PluginInvokeRequestExt, Store,
};
use symbio_core::{
    ExecAbortSignal, ExecEventSink, Plugin, PluginError, PluginInvokeRequest, PluginPayload,
};
use {symbio_core::plugin_error, symbio_core::plugin_info, symbio_core::plugin_warn};

mod breaker;
mod decls;
mod effects;

pub use self::breaker::BreakerInputs;
pub use self::decls::ToolDecls;
pub use self::effects::record_effect;
use symbio_plugin_api::effects::EffectInput;

// 工具结果提取

/// 同参重复调用的判据键：`(工具名, 参数文本)`。
///
/// 参数取 `Value::to_string()`（serde_json 对象按键序规范化），于是模型两次发出
/// 的同一调用——空白与键序不同也无妨——落到同一个键上。
pub(super) fn call_signature(name: &str, args: &str) -> String {
    format!("{name}\u{1}{args}")
}

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
        let end = symbio_core::text_floor_char_boundary(&s, max_chars);
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
/// 需要「本轮结束于等待用户动作」这类判断时，用 [`symbio_core::failure_kind`]
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

// 单工具执行（无阻塞）

/// 等待「用户中止」信号（工具执行期间）。
///
/// 只有一个来源：[`ExecAbortSignal`]。它由编排层创建、经 `ctx` 注入（键
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
async fn wait_tool_abort(abort: &ExecAbortSignal) {
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
    /// [`symbio_core::failure_kind`] 里的 pending 取值
    pub failure_kind: String,
}

/// 从工具返回的 `Data` 里读出「需要用户动作」的意图。
///
/// 判据是 `failure_kind` 这个**约定字段**（[`symbio_core::failure_kind::is_pending`]），
/// 不是靠猜 JSON 形状——[`extract_result`] 那种"从任意 JSON 里找 content/output"
/// 的启发式可以接受（**结果文本本就没有契约**，见其文档），但**控制流**不能建立
/// 在启发式上。
fn pending_prompt_from(data: &Value) -> Option<PendingPrompt> {
    let kind = data.get("failure_kind").and_then(|v| v.as_str())?;
    if !symbio_core::failure_kind::is_pending(kind) {
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
/// **本文件是工具结果节点的唯一写入者**：普通结果（`llm_build_tool_message`）与
/// 等待用户（本函数）都出自这里，两者共享同一套 id 约定
/// （`id = result_msg_id`、`parent_id = tool_call_id`）。
pub(super) fn build_user_prompt_message(
    result_msg_id: &str,
    tool_call_id: &str,
    pending: &PendingPrompt,
    tool_name: &str,
) -> ChatNode {
    ChatNode {
        id: result_msg_id.to_string(),
        parent_id: Some(tool_call_id.to_string()),
        // 等待用户的交互卡同样是**工具的结果**：族归工具自己。
        // 身份只写这一份——`role` 那格不再跟着写（D-147：主体在场时折叠读主体，
        // 第二份写了也不生效，只会在改写时留下「名实不符」的机会）。
        principal: Some(symbio_core::principal::Principal::tool_principal(tool_name)),
        msg_type: Some(NodeKind::Interaction),
        content: Some(MessageContent::Text(pending.text.clone())),
        status: Some(NodeStatus::WaitingUserAction),
        meta: Some(json!({
            "prompt": pending.prompt,
            "failure_kind": pending.failure_kind,
            "tool_name": tool_name,
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
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    result_msg_id: String,
    ctx: Arc<dyn PluginInvokeRequest>,
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
    // 投影函数（`tool_name::capability_to_wire`）在 core，因为**两个模块**依赖它：model 侧
    // 4 个协议要把名字发出去，本处要把它认回来。而「认回来」这一步（下面的
    // 解析）只有本模块需要，故留在本模块，不上 trait、不进 core。
    let tool_visitor = ctx.get(symbio_core::CAPABILITY_VISITOR);
    // 线上名解析的候选集 = 注册表给出的全名清单（收集管线的读侧助手，与 inputs /
    // context / decls 同一入口——收敛前这里自己写一遍 `list_capability()`）。
    let known_names: Vec<String> = symbio_core::tool_metas(&ctx)
        .await
        .into_iter()
        .map(|m| m.name)
        .collect();
    let resolved_name =
        symbio_core::capability_resolve(tool_name, known_names.iter().map(String::as_str))
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

    let session_id = ctx.get(symbio_core::SESSION_ID).unwrap_or_default();
    let agent_id = ctx.get(symbio_core::AGENT_ID).unwrap_or_default();
    let workdir = ctx.get(symbio_core::WORKDIR).unwrap_or_default();

    let tool_ctx = ctx.fork();
    tool_ctx.set(symbio_core::WORKDIR, workdir);
    tool_ctx.set(symbio_core::AGENT_ID, agent_id);
    tool_ctx.set(symbio_core::SESSION_ID, session_id);
    // `AGENT_PERMS`（D-153 的权限作用域）**不在这里显式设**：`fork()` 已把它从轮
    // ctx 复制过来（收集期由 agent 插件发布、`collect_capabilities` 挂回），而
    // 「缺席」与「空作用域」必须原样区分——前者是宿主态（无 manifest 可声明），
    // 后者是被发布过的空声明。上面三行显式重设是因为它们要把 `None` 归一成空串，
    // 这一行没有那种需求。
    tool_ctx.set(symbio_core::TOOL_CALL_ID, tool_call_id.to_string());
    // 流式工具（如 shell）据此 id 广播增量帧：与 result_msg_id 占位节点同 id，
    // 前端按 role=tool 全量替换合并；最终哨兵帧被捕获为工具结果。
    tool_ctx.set(symbio_core::RESULT_MSG_ID, result_msg_id.clone());
    // 执行期双原语注入 —— 工具「把事件写到哪」与「如何感知中止」的唯一来源。
    // 有了它，工具不必再用返回值里的 `PluginPayload::Session` 当事件流
    // （那是跨进程传输原语，见 `symbio_core::exec` 模块头）。
    tool_ctx.set(symbio_core::EVENT_SINK, sink.clone());
    tool_ctx.set(symbio_core::ABORT_SIGNAL, abort.clone());

    // 工具调用的发起（三条形态：ToolManager 命中 / 未命中回落 route /
    // 无 ToolManager 直接 route）。包成 `Box<dyn Future>` 是为了下面那个
    // 「轮询 + 空闲判定」的循环能重复借用同一份 future。
    //
    // 「命中」的判据是**解析成功**（名字在注册表里），不再是
    // `has_capability(线上名)`——后者按线上名查表，带非法字符的名字永远查不到，
    // 于是明明注册过也要走一遍 route 回落。
    let mut route_fut: std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<symbio_core::PluginPayload, PluginError>>
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
            // 执行不在表上（AR-025 §3）：按名取行 + capability_invoke；查不到即
            // 诚实的 NotFound（文案与机制化之前逐字一致）。
            Box::pin(async move {
                match visitor.get(&canonical).await.and_then(|r| r.cap().cloned()) {
                    Some(cap) => symbio_core::capability_invoke(cap.as_ref(), tool_ctx2).await,
                    None => Err(PluginError::NotFound(format!(
                        "Tool not found: {canonical}"
                    ))),
                }
            })
        } else {
            plugin_info!(
                "session",
                "[Tool] ToolManager does not have tool: {}, falling back to route",
                tool_name
            );
            tool_ctx.set(symbio_core::PATH, invoke_name.clone());
            let _ = tool_ctx.set_payload(args.clone());
            let p2 = p.clone();
            let tool_ctx2 = tool_ctx.clone();
            Box::pin(async move { p2.route(tool_ctx2).await })
        }
    } else {
        tool_ctx.set(symbio_core::PATH, invoke_name.clone());
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
    //      永远停在 `Streaming`（违反 `crates/symbio-plugin-session/src/docs/105-node-state-streaming.md` §8 #11）；
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
/// ## 四条不变量（本函数负责保证）
///
/// 1. **本批每一个工具调用都必须以终态收场**（`Completed` / `WaitingUserAction`）：
///    调用前广播 `Streaming`（`dispatch::batch::emit_tool_running`），调用后广播终态；未执行的
///    （阻塞 / 中止 / 交互中断）在函数末尾统一收口（`failure_kind = "not_executed"`）。
///    漏掉任何一条，前端就会有一个永远转下去的「运行中」。
/// 2. **每一个工具调用都必须有结果子节点**（`tool_messages` 里一条 role=Tool）——
///    「结果」在树里是子节点，前端按它渲染响应段。任何一条 return 分支
///    （参数解析失败 / 未执行 / 中止 / 被钩子拦下 / 闸门拒绝）都必须在给出父节点
///    终态的同时给出结果：只有父状态没有结果，用户看到的就是「调用凭空消失、
///    会话照旧往下走」。见 `dispatch::unfinished::not_executed_result`。
///
/// 3. **外部执行闸门在每个调用点各判一次**（S8 第 20 步）：判据批首读一次
///    （[`BreakerInputs`]），结论按调用点出——`Refuse` 零事件、`Break` 走出参
///    随收束入格、`Allow` 照旧开窗。
/// 4. **声明闸由能力包自己判**（DE-025 D-219）：工具在 `CapabilityMeta.behaviors`
///    上声明「我受哪项管辖」，对应实现从
///    `ToolBehavior::before` 回答能不能跑（淘汰 / 任务表写侧即此类）。宿主只交批首
///    读好的结论（主体 / 控制面撤销值 / 会话事实切片），本层不含任何「哪个工具名
///    受什么管辖」的分支。
#[allow(clippy::too_many_arguments)]
pub async fn process_tool_calls_async(
    tool_calls: Vec<TurnToolCallInfo>,
    parent: &Option<Arc<dyn Plugin>>,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    ctx: Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    // `tool_archive_dir` = 会话的 L0 存档目录（session 在派发前算好）。不从请求
    // 上下文反推插件目录——请求上下文不带 `PLUGIN_DIR`，子智能体下会指到父作用域。
    tool_archive_dir: &std::path::Path,
    // 本会话事实轨：闸门判据以**值**读（本会话完整史），本函数不拼路径、不开 WAL
    // （D-084 §1-A）。
    facts: &dyn Store<Event = Event>,
    // 淘汰闸的部署级判据（session 已折成一个值，见 D-086 §4）。
    elimination_revoked: bool,
    // 效果出参（批⑧ 的承诺与后续效果族共用一条通路，DE-025 D-207）：每次终态调用
    // 交出一条 `EffectInput`，轮末由写口问能力包的行为要草稿并落格。
    // out-param 而不是返回值：本函数的返回值是节点对（消费方三处都在用），
    // 再加一个只会逼着每个调用方都写 `let _ = `。
    effects: &mut Vec<EffectInput>,
    // 熔断出参（批⑩ 步 20）：外部执行闸门判 `Break` 的**理由**，同一条口径交回
    // 调用方随收束入格。**只记 `Break`**——`Refuse`（未授权）按 S09 §6 验收 1
    // 不得产生任何事件，两种拒绝在事件面上必须分得开。
    breaks: &mut Vec<&'static str>,
) -> (Vec<ChatNode>, Vec<ChatNode>) {
    // ① 批首闸门读数（阶段拆分见 `dispatch/{gates,batch,unfinished}`， / DE-021 §5 动作 B）。
    //    空批不读闸门、不发日志——与拆分前的早退位置逐字一致。
    if tool_calls.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut readings = super::dispatch::gates::read_batch_gates(
        &tool_calls,
        ctx.clone(),
        context_messages,
        facts,
        elimination_revoked,
    )
    .await;

    // ② 逐条执行：批内顺序流程（协议失败 / 闸门拒止 / 钩子拦下 / 正常执行四条出口）。
    // - `tool_messages`：工具结果子节点（用于追加到对话历史）
    // - `parent_updates`：ToolCall 父节点的**完整终态快照**（发射端从
    //   `context_messages` 取权威副本应用终态），供调用方做内存镜像整条替换并随
    //   `persist_messages` 落库。
    let (mut tool_messages, mut parent_updates) = super::dispatch::batch::run_batch(
        tool_calls,
        parent,
        sink,
        abort,
        ctx,
        context_messages,
        tool_archive_dir,
        &mut readings,
        effects,
        breaks,
    )
    .await;

    // ③ 未执行收口：本批没跑到的调用（中止 / 交互 break）统一补终态与结果子节点。
    super::dispatch::unfinished::close_unfinished(
        &readings,
        abort,
        sink,
        context_messages,
        &mut tool_messages,
        &mut parent_updates,
        effects,
    )
    .await;

    (tool_messages, parent_updates)
}

#[cfg(test)]
#[path = "tool_executor.test.rs"]
mod tests;
