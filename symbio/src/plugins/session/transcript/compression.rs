//! 压缩节点的发射器：把「正在压缩」作为**消息节点**呈现（[D-086](../../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §6）。
//!
//! ## 为什么是消息节点，而不是会话级提示
//!
//! 压缩是会话里真实发生的一步。用户应当像看到一次工具调用那样看到它：
//! 有自己的位置（当前时刻）、自己的状态（进行中 / 已完成），被压掉的历史
//! 就发生在它之前。挂在窗口顶部的横幅没有位置概念——用户滚到消息流中间时
//! 看不见它，事后也无法回溯「上次压缩发生在哪里、压掉了多少」。
//!
//! ## 为什么能在静音窗口里发出去
//!
//! 压缩 LLM 请求的出帧被刻意静音（`send_compression_request` 用哑 `tx` 接住
//! 全部流式帧，以免泄漏一个永不 finalize 的空 Turn 骨架）。但本发射器走
//! `emit_message_patch` → VDFS 变更订阅，**不经过**那条被静音的 turn channel。
//!
//! ## 为什么留 session（不随驱动器迁 loop）
//!
//! 它持有活跃名册、转写与会话配置的当前值，是 session 状态的一部分；驱动器经
//! `session/<sid>/_run/compression-*` 动作使用它（[D-086](../../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §4/§6）。
//! 同理，压缩熔断的三个计数器也住在这里的状态上：它们是**会话级**节奏状态。

use std::sync::Arc;

use super::super::active::ActiveSessionState;
use crate::symbio_core::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeRole, NodeStatus, CHAT_STYLE_TAG_COMPRESSION,
};

/// 把「正在压缩」作为**消息节点**呈现的发射器（压缩期 UI 的唯一出口）。
pub(crate) struct CompressionEmitter {
    pub(crate) state: Arc<ActiveSessionState>,
}

impl CompressionEmitter {
    pub(crate) fn new(state: Arc<ActiveSessionState>) -> Self {
        Self { state }
    }

    /// 自动压缩熔断：连续失败达到阈值后应跳过本次自动压缩。
    pub(crate) async fn should_skip(&self) -> bool {
        self.state.compression_should_skip().await
    }

    /// 记录一次压缩成功（清零熔断计数）。
    pub(crate) async fn record_success(&self) {
        self.state.compression_record_success().await;
    }

    /// 记录一次压缩失败（`permanent` = 输入超限类，需人工处理才会恢复）。
    pub(crate) async fn record_failure(&self, permanent: bool) {
        self.state.compression_record_failure(permanent).await;
    }

    /// 插入「正在压缩」节点：进在途图 + 发布（经 Transcript 唯一入口）。
    ///
    /// 进在途图是必要的：会话叶子 `read` 会叠加在途（见 `overlay_live`），
    /// 因此压缩期间切走再切回，这个节点仍然可见——否则用户切回来只看到「什么
    /// 都没有」，又变回最初那个"卡死"观感。
    pub(crate) async fn begin(&self, node_id: &str) {
        // 一条完整消息：身份 + 状态 + 占位正文一帧到位。
        self.state.transcript.lock().await.apply(
            ChatNode {
                id: node_id.to_string(),
                role: Some(NodeRole::Assistant),
                msg_type: Some(NodeKind::Notice),
                status: Some(NodeStatus::Streaming),
                content: Some(MessageContent::Text("正在压缩上下文…".to_string())),
                ..Default::default()
            }
            .with_style_tag(CHAT_STYLE_TAG_COMPRESSION),
        );
    }

    /// 定稿节点并使其离开在途图，返回终态副本供调用方落库。
    ///
    /// 在途图里找不到该 id 时（例如前端是在压缩开始之后才连上的）**照样构造
    /// 并发布**——终态必须到达，否则那个 `Streaming` 节点会永远留在前端转圈。
    ///
    /// `failure_kind`：失败原因码（写进 `meta.failure_kind`）。
    ///
    /// `stats`：这次压缩的**结构化交代**（来源 / 前后水位 / 上限 / 丢了几条）。
    /// 正文只说得了一条「N → M 条」，而用户真正想知道的是「我离上限还有多远、
    /// 这次是谁触发的」——那些都是字段，不是文案，因此随 `meta` 下发由前端渲染。
    /// 缺字段时前端退回正文那一行，旧前端（不认识这些字段）也不受影响。
    pub(crate) async fn finish(
        &self,
        node_id: &str,
        status: NodeStatus,
        text: &str,
        failure_kind: Option<&str>,
        stats: Option<serde_json::Value>,
    ) -> ChatNode {
        let node = {
            let mut tr = self.state.transcript.lock().await;
            let mut node = tr.get(node_id).unwrap_or_else(|| {
                ChatNode {
                    id: node_id.to_string(),
                    role: Some(NodeRole::Assistant),
                    msg_type: Some(NodeKind::Notice),
                    ..Default::default()
                }
                .with_style_tag(CHAT_STYLE_TAG_COMPRESSION)
            });
            node.status = Some(status.clone());
            node.content = Some(MessageContent::Text(text.to_string()));
            if let Some(kind) = failure_kind {
                merge_meta(&mut node, serde_json::json!({ "failure_kind": kind }));
            }
            if let Some(stats) = stats {
                merge_meta(&mut node, stats);
            }
            // 发布终态 + 落库回执：权威副本即将由调用方落库，
            // 在途副本必须作废（否则同一条消息以「存储 + 在途」两种形态参与叠加）。
            //
            // **完整消息帧**（不是状态帧）：本节点的正文从未经 `delta` 上线过——
            // `begin` 发的是一句占位（"正在压缩上下文…"），这里的正文（"已压缩
            // N → M 条"）是**首次也是唯一**一次上线。用状态帧剥掉正文，前端会一直
            // 停在占位文案上，直到重开会话才从存储读到结果（实测回归）。
            // 状态帧只适用于「正文已由 delta 逐帧上线」的节点。
            tr.apply(crate::symbio_core::llm_message_frame(&node));
            tr.persisted(std::slice::from_ref(&node.id));
            node
        };
        node
    }

    /// 压缩请求的**增量改道落点**：返回直连本会话转写唯一写入点的 writer。
    ///
    /// 只服务压缩流水线的过滤桥（摘要帧经白名单改写后从这里进转写）。压缩流水线
    /// 仍在本插件时经此取 writer；驱动器切出后由 `TurnDispatch.writer` 交付。
    pub(crate) fn transcript_writer(&self) -> Arc<dyn crate::symbio_core::ExecTranscriptWriter> {
        super::compression_writer(self.state.transcript.clone())
    }
}

/// 把一个字段补丁（对象）**合并**进节点 `meta`，不覆盖已有键——
/// `failure_kind` 与统计字段因此可以各写各的，谁都不会把对方抹掉。
fn merge_meta(node: &mut ChatNode, patch: serde_json::Value) {
    let serde_json::Value::Object(patch) = patch else {
        return;
    };
    let mut meta = node
        .meta
        .take()
        .filter(|m| m.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = meta.as_object_mut() {
        for (k, v) in patch {
            obj.insert(k, v);
        }
    }
    node.meta = Some(meta);
}
