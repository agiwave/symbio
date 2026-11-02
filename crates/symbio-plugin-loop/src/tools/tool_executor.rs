//! 工具执行层
//!
//! 负责：
//! - 单个工具调用的端口取用（`execute_tool_async`：经 `port.tool` 拿执行合同——
//!   线上名解析、空闲超时、结果契约都住提供者一侧（session 的 `ToolPort`），
//!   本层不再认识它们）
//! - 批量工具调用处理与结果广播（`process_tool_calls_async`）
//!
//! 设计说明（会话激活/恢复状态机）：
//! - 本层**不阻塞**等待任何用户输入。需要用户确认（confirm）或主动询问（ask_user）
//!   的工具会自行产出 `user_prompt` 消息节点并标记 `WaitingUserAction`，由编排层
//!   （chat_loop）在本轮结束时将会话置于 `AwaitingInput(user)`；用户答案以一条普通
//!   `user` 消息回填后，新一轮会重跑该工具。详见 USER_INPUT_MECHANISM 设计文档。

use symbio_kernel::llm::TurnToolCallInfo;

use serde_json::{json, Value};
use std::sync::Arc;
use symbio_kernel::event::Event;
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest};
use symbio_kernel::store::Store;
use symbio_plugin_api::plugin_error;
use symbio_sdk::chat::chat_node::{ChatNode, MessageContent, NodeKind, NodeStatus};

mod breaker;
mod decls;
mod effects;

pub use self::breaker::BreakerInputs;
pub use self::decls::ToolDecls;
pub use self::effects::record_effect;
use symbio_plugin_api::effects::EffectInput;
// tool 端口的合同类型（下沉到 plugin-api）：`PendingPrompt` 经本模块 re-export，
// dispatch 一系（`dispatch.rs` / `dispatch/batch.rs`）与测试的导入面不变。
pub use symbio_plugin_api::port::PendingPrompt;
use symbio_plugin_api::port::ToolCallRequest;

/// 同参重复调用的判据键：`(工具名, 参数文本)`。
///
/// 参数取 `Value::to_string()`（serde_json 对象按键序规范化），于是模型两次发出
/// 的同一调用——空白与键序不同也无妨——落到同一个键上。
pub(super) fn call_signature(name: &str, args: &str) -> String {
    format!("{name}\u{1}{args}")
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
        principal: Some(symbio_sdk::chat::principal::Principal::tool_principal(
            tool_name,
        )),
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

/// 执行单个工具调用（不阻塞等待用户）：经 `port.tool` 的执行合同跑。
///
/// 本函数只是**端口取用面**：按端口名从能力表读提供者，把调用现场整份交给
/// [`ToolCallExecutor`](symbio_plugin_api::port::ToolCallExecutor)，收回
/// `(result_text, success, pending)` 三格——线上名怎么解析、挂死怎么判（空闲而不是
/// 总时长）、任意 JSON 怎么压成一段文本，都是提供者一侧的私事，行为等价由 e2e
/// 全量守着。
///
/// 端口缺席（没装提供者插件——零工具的装配没有工具世界的提供者）或合同缺席
/// （提供者不实现 tool 合同）都**响亮失败**：错误文本回传给模型（失败属信息性，
/// 模型据此改道）、`success = false`、日志出声——与既有 route 错误同形。不许
/// 静默回退、不许按「判不出来」继续：装配缺了提供者，继续转只会把缺失往后推。
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
    let absent = |why: &str| {
        let text = format!("Error: `port.tool` 缺席：{why}（工具 {tool_name} 未执行）");
        plugin_error!("session", "[Tool] {text}");
        (text, false, None)
    };
    let Some(provider) = super::super::port::tool_port(&ctx).await else {
        return absent("没有可用的工具提供者");
    };
    let Some(executor) = provider.tool_execute() else {
        return absent("提供者不实现工具执行合同");
    };
    let out = executor
        .execute(ToolCallRequest {
            parent: parent.clone(),
            tool_name: tool_name.to_string(),
            args,
            tool_call_id: tool_call_id.to_string(),
            sink: sink.clone(),
            abort: abort.clone(),
            result_msg_id,
            ctx,
        })
        .await;
    (out.result_text, out.success, out.pending)
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
/// 3. **外部执行闸门在每个调用点各判一次**：判据批首读一次
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
    // 效果出参（承诺与后续效果族共用一条通路，DE-025 D-207）：每次终态调用
    // 交出一条 `EffectInput`，轮末由写口问能力包的行为要草稿并落格。
    // out-param 而不是返回值：本函数的返回值是节点对（消费方三处都在用），
    // 再加一个只会逼着每个调用方都写 `let _ = `。
    effects: &mut Vec<EffectInput>,
    // 熔断出参：外部执行闸门判 `Break` 的**理由**，同一条口径交回
    // 调用方随收束入格。**只记 `Break`**——`Refuse`（未授权）按 DE-039 §6 验收 1
    // 不得产生任何事件，两种拒绝在事件面上必须分得开。
    breaks: &mut Vec<&'static str>,
) -> (Vec<ChatNode>, Vec<ChatNode>) {
    // ① 批首闸门读数（阶段见 `dispatch/{gates,batch,unfinished}`）。
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
