//! `port.tool` 的世界提供者（D-298）：工具调用的执行面收在 session 一侧。
//!
//! 环（loop）只按端口名取——线上名怎么解析（`mcp__fs__read` → 能力名）、挂死怎么判
//! （空闲而不是总时长）、结果怎么压成一段文本，都是这里的私事。session 是**收集宿主**
//! （`capabilities.rs` 的 `collect_capabilities`），能力表就是工具世界：`ToolPort`
//! 内部经 `tool_metas` + `capability_invoke` 调各工具插件的 `Capability`——执行提供者
//! 是它们，本类型只是端口面上的前端，不新建工具代理插件。
//!
//! 装行条件：**表里有工具行才有提供者**（`collect_capabilities` 在广播之后装）——镜像
//! model 端口的「生效者缺席 ⇒ 不注册」：零工具的装配没有工具世界的提供者，环在调用点
//! 拿到缺席读数，响亮失败并落事实，不在这里猜一个别的。`ToolPort` 因此**无状态**：
//! 执行期从 `req.ctx` 读能力表（不在装行时持有 `Arc<dyn Registry>`——那会与表里的
//! 提供者行互指成环，整个装配图泄不掉）。

use std::sync::Arc;

use symbio_kernel::exec::ExecAbortSignal;
use symbio_kernel::plugin::{PluginError, PluginInvokeRequestExt, PluginPayload};
use symbio_plugin_api::port::{
    PendingPrompt, PortProvider, ToolCallExecutor, ToolCallOutcome, ToolCallRequest,
};
use symbio_plugin_api::{plugin_error, plugin_info, plugin_warn, ports};
use symbio_sdk::capability::{capability_resolve, tool_metas, CAPABILITY_VISITOR};

/// `port.tool` 的提供者：无状态前端（见模块文档）。
pub(crate) struct ToolPort;

impl PortProvider for ToolPort {
    fn ports(&self) -> &'static [&'static str] {
        &[ports::TOOL]
    }

    fn provider_id(&self) -> &str {
        symbio_deploy::PLUGIN_ID_SESSION
    }

    fn tool_execute(&self) -> Option<Arc<dyn ToolCallExecutor>> {
        Some(Arc::new(ToolPort))
    }
}

#[async_trait::async_trait]
impl ToolCallExecutor for ToolPort {
    /// 执行一次工具调用（不阻塞等待用户）：解析线上名、经能力表调用、空闲超时看护、
    /// 按结果契约解码。三段语义（B1–B3）逐字来自 loop 的 `execute_tool_async`，
    /// 行为等价由 e2e 全量守着。
    async fn execute(&self, req: ToolCallRequest) -> ToolCallOutcome {
        let ToolCallRequest {
            parent,
            tool_name,
            args,
            tool_call_id,
            sink,
            abort,
            result_msg_id,
            ctx,
        } = req;
        let started_at = std::time::Instant::now();
        plugin_info!(
            "session",
            "[Tool] 请求发起: {} args={}",
            tool_name,
            args_summary(&args, 200)
        );

        let p = match parent {
            Some(p) => p,
            None => return outcome(("Error: No parent plugin".into(), false, None)),
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
        let tool_visitor = ctx.get(CAPABILITY_VISITOR);
        // 线上名解析的候选集 = 注册表给出的全名清单（收集管线的读侧助手，与 inputs /
        // context / decls 同一入口——收敛前这里自己写一遍 `list_capability()`）。
        let known_names: Vec<String> = tool_metas(&ctx).await.into_iter().map(|m| m.name).collect();
        let resolved_name = capability_resolve(&tool_name, known_names.iter().map(String::as_str))
            .map(str::to_string);
        // 解析失败**不猜**：按原样交给路由，由它给出诚实的 NotFound。
        // 反演猜错会调起**另一个工具**，那比报错坏得多。
        let invoke_name = resolved_name.clone().unwrap_or_else(|| tool_name.clone());
        if resolved_name.is_none() && tool_visitor.is_some() {
            plugin_warn!(
                "session",
                "[Tool] 名字 {} 不在能力注册表里（线上名解析失败），按原样路由",
                tool_name
            );
        }

        let session_id = ctx.get(symbio_sdk::keys::SESSION_ID).unwrap_or_default();
        let agent_id = ctx.get(symbio_sdk::keys::AGENT_ID).unwrap_or_default();
        let workdir = ctx.get(symbio_sdk::keys::WORKDIR).unwrap_or_default();

        let tool_ctx = ctx.fork();
        tool_ctx.set(symbio_sdk::keys::WORKDIR, workdir);
        tool_ctx.set(symbio_sdk::keys::AGENT_ID, agent_id);
        tool_ctx.set(symbio_sdk::keys::SESSION_ID, session_id);
        // `AGENT_PERMS`（D-153 的权限作用域）**不在这里显式设**：`fork()` 已把它从轮
        // ctx 复制过来（收集期由 agent 插件发布、`collect_capabilities` 挂回），而
        // 「缺席」与「空作用域」必须原样区分——前者是宿主态（无 manifest 可声明），
        // 后者是被发布过的空声明。上面三行显式重设是因为它们要把 `None` 归一成空串，
        // 这一行没有那种需求。
        tool_ctx.set(symbio_sdk::keys::TOOL_CALL_ID, tool_call_id.clone());
        // 流式工具（如 shell）据此 id 广播增量帧：与 result_msg_id 占位节点同 id，
        // 前端按 role=tool 全量替换合并；最终哨兵帧被捕获为工具结果。
        tool_ctx.set(symbio_sdk::keys::RESULT_MSG_ID, result_msg_id.clone());
        // 执行期双原语注入 —— 工具「把事件写到哪」与「如何感知中止」的唯一来源。
        // 有了它，工具不必再用返回值里的 `PluginPayload::Session` 当事件流
        // （那是跨进程传输原语，见 `symbio_kernel::exec` 模块头）。
        tool_ctx.set(symbio_kernel::exec::EVENT_SINK, sink.clone());
        tool_ctx.set(symbio_kernel::exec::ABORT_SIGNAL, abort.clone());

        // 工具调用的发起（三条形态：ToolManager 命中 / 未命中回落 route /
        // 无 ToolManager 直接 route）。包成 `Box<dyn Future>` 是为了下面那个
        // 「轮询 + 空闲判定」的循环能重复借用同一份 future。
        //
        // 「命中」的判据是**解析成功**（名字在注册表里），不再是
        // `has_capability(线上名)`——后者按线上名查表，带非法字符的名字永远查不到，
        // 于是明明注册过也要走一遍 route 回落。
        let mut route_fut: std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<symbio_kernel::plugin::PluginPayload, PluginError>,
                    > + Send,
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
                // 执行不在表上（DE-025 §3）：按名取行 + capability_invoke；查不到即
                // 诚实的 NotFound（文案与机制化之前逐字一致）。
                Box::pin(async move {
                    match visitor.get(&canonical).await.and_then(|r| r.cap().cloned()) {
                        Some(cap) => {
                            symbio_kernel::capability::capability_invoke(cap.as_ref(), tool_ctx2)
                                .await
                        }
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
                tool_ctx.set(symbio_sdk::keys::PATH, invoke_name.clone());
                let _ = tool_ctx.set_payload(args.clone());
                let p2 = p.clone();
                let tool_ctx2 = tool_ctx.clone();
                Box::pin(async move { p2.route(tool_ctx2).await })
            }
        } else {
            tool_ctx.set(symbio_sdk::keys::PATH, invoke_name.clone());
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
                _ = wait_tool_abort(&abort) => {
                    plugin_warn!(
                        "session",
                        "[Tool] 执行期间被用户中止: {} (耗时 {}ms, call_id={})",
                        tool_name,
                        started_at.elapsed().as_millis(),
                        tool_call_id
                    );
                    return outcome((
                        format!("Error: 工具 {} 被用户中止，未返回结果", tool_name),
                        false,
                        None,
                    ));
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
                    return outcome((
                        format!(
                            "Error: 工具 {} 已 {} 秒无任何进展，已强制中断（疑似挂死）",
                            tool_name, TOOL_EXEC_IDLE_TIMEOUT_SECS
                        ),
                        false,
                        None,
                    ));
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
                            return outcome((
                                "Error: Failed to deserialize payload".into(),
                                false,
                                None,
                            ))
                        }
                    };

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
                    outcome((res, true, pending))
                }

                _ => outcome(("Error: Unexpected payload type".into(), false, None)),
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
                outcome((format!("Error: {e}"), false, None))
            }
        }
    }
}

/// 迁移前的返回三元组 `(result_text, success, pending)` → [`ToolCallOutcome`]。
fn outcome(
    (result_text, success, pending): (String, bool, Option<PendingPrompt>),
) -> ToolCallOutcome {
    ToolCallOutcome {
        result_text,
        success,
        pending,
    }
}

// ── 迁移来的执行私件（B1–B3 的实现细节，本模块私有）────────────────────────

/// 参数摘要：截断到 max_chars，用于日志打印（避免超长参数刷屏）。
///
/// 安全截断：`max_chars` 按**字节**解释但必须落在字符边界上。
/// 历史事故：中文参数（如 `write_file` 的正文）使 `&s[..200]` 落在
/// 多字节字符内部 → `tokio-rt-worker` panic → 整轮 ChatLoop 异常终止。
fn args_summary(args: &serde_json::Value, max_chars: usize) -> String {
    let s = args.to_string();
    if s.len() <= max_chars {
        s
    } else {
        let end = symbio_sdk::text::text_floor_char_boundary(&s, max_chars);
        format!("{}…(len={})", &s[..end], s.len())
    }
}

/// 工具结果里「给模型看的正文」字段名（[`extract_result`] 的第 1 条判据）。
pub(crate) const RESULT_CONTENT: &str = "content";
/// 命令类工具的结果正文字段名（`shell` 等；[`extract_result`] 的第 2 条判据）。
pub(crate) const RESULT_OUTPUT: &str = "output";

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
/// 需要「本轮结束于等待用户动作」这类判断时，用 [`symbio_kernel::capability::failure_kind`]
/// 这个**约定字段**（见 [`pending_prompt_from`]），不要靠猜 JSON 形状。
pub(crate) fn extract_result(data: &serde_json::Value) -> String {
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

/// 等待「用户中止」信号（工具执行期间）。
///
/// 只有一个来源：[`ExecAbortSignal`]。它由编排层创建、经 `ctx` 注入（键
/// `symbio_kernel::exec::ABORT_SIGNAL`），因此本函数不必再分辨「abort 帧 / 取消令牌 /
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

/// 从工具返回的 `Data` 里读出「需要用户动作」的意图。
///
/// 判据是 `failure_kind` 这个**约定字段**（[`symbio_kernel::capability::failure_kind::is_pending`]），
/// 不是靠猜 JSON 形状——[`extract_result`] 那种"从任意 JSON 里找 content/output"
/// 的启发式可以接受（**结果文本本就没有契约**，见其文档），但**控制流**不能建立
/// 在启发式上。
fn pending_prompt_from(data: &serde_json::Value) -> Option<PendingPrompt> {
    let kind = data.get("failure_kind").and_then(|v| v.as_str())?;
    if !symbio_kernel::capability::failure_kind::is_pending(kind) {
        return None;
    }
    Some(PendingPrompt {
        text: data
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("需要用户操作")
            .to_string(),
        prompt: data
            .get("prompt")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        failure_kind: kind.to_string(),
    })
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
