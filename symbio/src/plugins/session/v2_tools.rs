//! v2 的工具分发实现（core 的 [`DispatchPort`] 契约，插件侧落地）。
//!
//! ## 为什么分发在插件侧
//!
//! 分发的**全部输入**都在这里：插件宿主（路由工具能力）、请求上下文（能力注册表 /
//! 会话 id / 工作区）、事件出口（结果节点写进对话图）、会话目录（超长结果存档）。
//! core 认识这些即违 E-009；契约留在 core、实现留在插件，依赖方向由编译器保证。
//!
//! ## 与 v1 的关系：**不是第二条执行链**
//!
//! 实际执行仍走 `process_tool_calls_async`——超长结果存档、生命周期钩子、
//! 审批/问答的「待用户动作」、批尾未执行收口、中止提前收口全在里面。本模块只补
//! 两件 v2 独有的事：
//!
//! 1. **补建助手轮消息**。v1 的工具调用节点是 model 插件在**流式期**逐帧广播的
//!    （`parse_sse_stream` 的 `ToolCallDelta`）；v2 经 `LlmAdapter` 拿到的是收口后的
//!    `LlmTurn`，没有那条帧流。节点形状必须与 v1 **同一份**（前端读同一套字段），
//!    因此复用 `llm_build_tool_call_nodes` / `llm_build_text_node`。
//! 2. **把结果映射成 core 的事实形状**（[`DispatchOutcome`]），供运行器落
//!    `artifact.added` 格与拼下一轮 prompt。
//!
//! ## 正文节点的定格
//!
//! 工具轮的中途正文（"我先查一下"）已经由 UI 桥逐片广播过。本模块在分发前把它
//! **定格**（`Completed`）并切节点（[`super::v2_exec::UiBridge::finalize_node`]）——
//! 不定格它永远转「流式中」，不切节点多轮正文会粘成一条。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::symbio_core::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::{
    llm_emit_message, DispatchOutcome, DispatchPort, ExecAbortSignal, ExecEventSink, LlmTurn,
    Plugin, PluginInvokeRequest,
};

use super::message_build::{llm_build_text_node, llm_build_tool_call_nodes};
use super::tools::process_tool_calls_async;
use super::transcript::llm_emit_state;
use super::v2_exec::UiBridge;

/// 工具执行层交回的**收束派生事实**（承诺 / 任务表 / 熔断）。
///
/// 三者的数据来源都在工具执行层（`Delegation` / `TaskDeclaration` / 熔断理由），
/// 而写方（落格）在收束处——`bridge` 档由 `v2_facts::record` 写，`full` 档由
/// `v2_exec::execute_turn` 轮末写（同一个 `v2_facts::record_derived`）。本结构是
/// 那份出参从分发点回到运行器的**唯一通道**：不收集它，`full` 档这三样就静默全丢
/// （与记忆/学习同一个坑——它们在 `full` 档整体失效，而档位名还自称「整体切换」）。
///
/// 形态仿 [`SessionDispatchPort::produced`]：`dispatch` 在 `&self` 上跑，出参却要
/// 跨 await 累积，故用 `Mutex` 收，调用方在桥释放前 `take_derived` 取走。
#[derive(Default)]
pub(crate) struct DerivedFacts {
    /// 代际立约出参（批⑧）：`agent_run` 的承诺记录。
    pub(crate) delegations: Vec<super::tools::Delegation>,
    /// 任务表出参（批⑨）：本次成功的 `todo_write` 声明的清单状态。
    pub(crate) tasks: Vec<super::tools::TaskDeclaration>,
    /// 熔断出参（批⑩ 步 20）：外部执行闸门判 `Break` 的理由。
    pub(crate) breaks: Vec<&'static str>,
}

/// v2 的工具分发方：一轮工具调用的执行 + 落节点 + 结果映射。
pub(crate) struct SessionDispatchPort {
    /// 插件宿主（工具能力经它路由）。
    parent: Option<Arc<dyn Plugin>>,
    /// 本轮请求上下文（能力注册表 / 会话 id / 工作区）。
    ctx: Arc<dyn PluginInvokeRequest>,
    /// 事件出口（工具节点与结果都写到对话图上）。
    sink: ExecEventSink,
    /// 本插件目录（工具结果存档落在这里）。
    session_dir: PathBuf,
    /// 本轮中止信号：批内逐工具检查，未执行的批尾必须定格。
    abort: ExecAbortSignal,
    /// 本轮 Turn 根节点 id（助手轮消息的父节点）。
    root_id: String,
    /// 流式桥：交出本轮正文节点并切下一轮。
    bridge: Arc<UiBridge>,
    /// 本轮产生的全部消息（供 chat_loop 并入落库权威镜像）。
    produced: Mutex<Vec<ChatMessage>>,
    /// 本轮工具执行层交回的收束派生事实（承诺 / 任务表 / 熔断）。
    derived: Mutex<DerivedFacts>,
}

impl SessionDispatchPort {
    #[allow(clippy::too_many_arguments)] // 构造点唯一（v2_exec），字段即上下文
    pub(crate) fn new(
        parent: Option<Arc<dyn Plugin>>,
        ctx: Arc<dyn PluginInvokeRequest>,
        sink: ExecEventSink,
        session_dir: PathBuf,
        abort: ExecAbortSignal,
        root_id: String,
        bridge: Arc<UiBridge>,
    ) -> Self {
        Self {
            parent,
            ctx,
            sink,
            session_dir,
            abort,
            root_id,
            bridge,
            produced: Mutex::new(Vec::new()),
            derived: Mutex::new(DerivedFacts::default()),
        }
    }

    /// 取走本轮产生的消息（调用方在桥释放前取）。
    pub(crate) fn take_produced(&self) -> Vec<ChatMessage> {
        std::mem::take(&mut self.produced.lock().unwrap())
    }

    /// 取走本轮工具执行层交回的收束派生事实（调用方在桥释放前取）。
    pub(crate) fn take_derived(&self) -> DerivedFacts {
        std::mem::take(&mut self.derived.lock().unwrap())
    }
}

#[async_trait]
impl DispatchPort for SessionDispatchPort {
    async fn dispatch(&self, turn: &LlmTurn) -> Vec<DispatchOutcome> {
        let mut produced: Vec<ChatMessage> = Vec::new();

        // ① 定格本轮正文与推理（若有）：节点已由 UI 桥逐片广播，这里只迁状态并切节点。
        //    没有正文（纯工具调用轮）就不建节点——空节点是噪声。
        //
        //    两条流**一起定格**：都由桥建节点、都在本轮结束时收口，分两处做只是把同一个
        //    动作写两遍，且漏一处就有一条流永远转「流式中」（界面上表现为思考块卡住）。
        let text_node = if turn.text.trim().is_empty() {
            None
        } else {
            self.bridge.finalize_node(&turn.text)
        };
        let reason_node = self.bridge.finalize_reasoning_node();
        // 同步点：定格帧在桥的通道里排队，而下面的工具节点**直接**写出口
        // （`process_tool_calls_async` 只认 `ExecEventSink`）。不先排空队列，
        // 工具卡片会跑到正文之前。
        self.bridge.flush().await;
        if let Some(node_id) = text_node {
            let msg = llm_build_text_node(&self.root_id, &turn.text, None, node_id);
            // 状态帧：正文已由增量帧传过，重发正文会与队列里的增量打架。
            llm_emit_state(&self.sink, msg.clone()).await;
            produced.push(msg);
        }
        if let Some((node_id, text)) = reason_node {
            let msg = ChatMessage {
                id: node_id,
                parent_id: Some(self.root_id.clone()),
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Reasoning),
                content: Some(MessageContent::Text(text)),
                status: Some(MessageStatus::Completed),
                ..Default::default()
            };
            // 状态帧：推理正文已由增量帧传过，同上——重发会与队列里的增量打架。
            llm_emit_state(&self.sink, msg.clone()).await;
            produced.push(msg);
        }

        // ② 工具调用节点：v2 没有流式工具帧，由本处按 v1 的同一形状补建。
        //    先以 `Streaming` 广播（工具还没跑，卡片应显示运行中），落库副本用终态。
        let call_nodes = llm_build_tool_call_nodes(&self.root_id, &turn.tool_calls);
        for node in &call_nodes {
            let mut frame = node.clone();
            frame.status = Some(MessageStatus::Streaming);
            llm_emit_message(&self.sink, frame).await;
        }

        // ③ 分发（v1 的执行器：存档 / 钩子 / 审批 / 批尾收口全在里面）。
        //    父节点查询用刚广播的这批节点——它们与执行器要求的形状同源。
        //
        // 三份收束派生事实出参（承诺 / 任务表 / 熔断）：数据来源在工具执行层，
        // 写方在收束处——`full` 档由本分发方收下、`v2_exec` 轮末调
        // `v2_facts::record_derived` 落格（与 bridge 档**同一写方**）。不收这三份，
        // `full` 档的代际立约 / 任务表 / 熔断就静默全丢（见 [`DerivedFacts`]）。
        let mut delegations: Vec<super::tools::Delegation> = Vec::new();
        let mut tasks: Vec<super::tools::TaskDeclaration> = Vec::new();
        let mut breaks: Vec<&'static str> = Vec::new();
        let (tool_msgs, parent_updates) = process_tool_calls_async(
            turn.tool_calls.clone(),
            &self.parent,
            &self.sink,
            &self.abort,
            self.ctx.clone(),
            &call_nodes,
            &self.session_dir,
            // 代际立约出参（批⑧）：`agent_run` 的承诺记录。
            &mut delegations,
            // 任务表出参（批⑨）：本次成功的 `todo_write` 声明的清单状态。
            &mut tasks,
            // 熔断出参（批⑩ 步 20）：外部执行闸门判 `Break` 的理由。
            &mut breaks,
        )
        .await;

        produced.extend(call_nodes);
        // 父节点终态同步进落库副本——整条替换（执行器交回的就是完整快照）。
        // id 不存在的（协议失败兜底父节点）补入，使结果子节点不悬空。
        for full in &parent_updates {
            match produced.iter_mut().find(|m| m.id == full.id) {
                Some(msg) => *msg = full.clone(),
                None => produced.push(full.clone()),
            }
        }
        produced.extend(tool_msgs.iter().cloned());
        self.produced.lock().unwrap().extend(produced);
        // 三份收束派生事实出参收进取件面（`take_derived`）——调用方在桥释放前取走。
        {
            let mut d = self.derived.lock().unwrap();
            d.delegations.extend(delegations);
            d.tasks.extend(tasks);
            d.breaks.extend(breaks);
        }

        // ④ 映射为 core 的事实形状（运行器据此落 `artifact.added` 并拼下一轮 prompt）。
        tool_msgs
            .iter()
            .map(|m| {
                let call_id = m.parent_id.clone().unwrap_or_default();
                let name = m
                    .name
                    .clone()
                    .or_else(|| {
                        turn.tool_calls
                            .iter()
                            .find(|tc| tc.id.as_deref() == Some(call_id.as_str()))
                            .and_then(|tc| tc.name.clone())
                    })
                    .unwrap_or_default();
                DispatchOutcome {
                    call_id,
                    name,
                    text: m.content.as_ref().map(|c| c.to_text()).unwrap_or_default(),
                    ok: m
                        .meta
                        .as_ref()
                        .and_then(|meta| meta.get("success"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    needs_user_action: m.msg_type == Some(MessageType::UserPrompt)
                        && m.status == Some(MessageStatus::WaitingUserAction),
                }
            })
            .collect()
    }
}

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow symbio/src/plugins/session/v2_tools.rs::take_produced: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/v2_tools.rs::take_derived: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/v2_tools.rs::dispatch: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
