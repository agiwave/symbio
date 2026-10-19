//! 驱动器 → 会话的**动作面端口**（[ADR-086](../../../../docs/010-adr/086.md "ADR-086") §4）。
//!
//! 驱动器切成独立插件后仍要读写**本会话**的转写、事实与压缩状态——那些状态的
//! owner 全是 session。E-009 禁止 `use crate::plugins::session`，于是这一侧只认
//! 一个 trait，调用一律经统一地址面 `session/<sid>/_run/<op>` 派发（与 vdfs/route
//! 同款「主动发起流程走端口/协议」的纪律）。
//!
//! ## 为什么 trait 在本插件内
//!
//! 消费方只有 loop 一家，生产实现也只有 VDFS 一家。把 trait 提到 core 会把
//! 「驱动器恰好需要这 15 个动作」冻结成机制层契约——而动作集随驱动器演化
//! （ADR-083 §4：新增跨墙面要有论证，不是随手加一格）。
//!
//! 测试实现（fake port）同样在本插件内：E-009 连测试一起判，测试路径里的
//! `crate::plugins::session` 同样是墙外引用。
//!
//! ## 授权不在这里
//!
//! 本端口只是**客户端**。三条授权判据（`TURN_DISPATCH` 存在 / 令牌会话相符 /
//! op 在闭集内）全在 session 的 `run_at`——端口不做、也无法做第二遍判定。

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::{
    run_ops, vdfs_context, AddNodesRequest, CompressionBeginReply, CompressionBeginRequest,
    CompressionFinishRequest, ConsolidateRequest, DrainedSupplement, EditFrame, EditNodesRequest,
    FleetSnapshot, Plugin, PluginError, PluginInvokeRequest, RemoveNodesRequest, ReplaceAllRequest,
    VdfsRequest, VdfsResponse, WorkerProgress, PLUGIN_ID_SESSION,
};

/// 驱动器对会话动作面的**窄端口**（方法集 = 驱动器真实需要的动作，不多一格）。
#[async_trait]
pub(crate) trait SessionPort: Send + Sync {
    /// 会话全部消息（含工具节点）。
    async fn messages(&self) -> Result<Vec<ChatNode>, PluginError>;

    /// 上下文候选（已按轮数窗口与协议投影裁剪）。
    async fn context_messages(&self) -> Result<Vec<ChatNode>, PluginError>;

    /// 以 `Carrier::Runner` 追加（不落出生事实，承载方另有事实）。
    async fn add_by_runner(&self, messages: Vec<ChatNode>) -> Result<Vec<ChatNode>, PluginError>;

    /// 改写既有节点（帧形由调用方指定：状态帧 / 完整消息帧）。
    async fn edit_nodes(
        &self,
        nodes: Vec<ChatNode>,
        frame: EditFrame,
    ) -> Result<Vec<ChatNode>, PluginError>;

    /// 删除节点（前端逐条收 Remove）。
    async fn remove_nodes(&self, ids: Vec<String>) -> Result<(), PluginError>;

    /// 整表重写（压缩语义）：落库 + 广播被丢的与新的首条。
    async fn replace_all(
        &self,
        nodes: Vec<ChatNode>,
        dropped: Vec<String>,
        head: ChatNode,
    ) -> Result<(), PluginError>;

    /// 补落寄存的变更（持写者令牌的一方执行）。
    async fn drain_parked(&self) -> Result<(), PluginError>;

    /// 压缩事实写入事实源。
    async fn consolidate(&self, messages: Vec<ChatNode>) -> Result<(), PluginError>;

    /// 取一批用户补充（无补充 ⇒ `None`）。
    async fn take_supplements(&self) -> Result<Option<DrainedSupplement>, PluginError>;

    /// 子会话进展。
    async fn worker_progress(&self) -> Result<Vec<WorkerProgress>, PluginError>;

    /// 事实源召回快照（agent 插件建树用）。
    async fn recall_snapshots(&self) -> Result<Vec<FleetSnapshot>, PluginError>;

    /// 本主体的 `assign.work` 是否被收回（折叠在会话侧，驱动器只收结论）。
    async fn elimination_revoked(&self) -> Result<bool, PluginError>;

    /// 发布压缩占位节点；返回 `skipped=true` 表示熔断中、本次压缩应跳过。
    async fn compression_begin(
        &self,
        node_id: &str,
        force: bool,
    ) -> Result<CompressionBeginReply, PluginError>;

    /// 定稿压缩节点（含熔断记账）并返回落库后的权威副本。
    async fn compression_finish(
        &self,
        req: CompressionFinishRequest,
    ) -> Result<ChatNode, PluginError>;
}

/// 经 VDFS 地址面调 session 的实现。
///
/// 每次调用都现场取一次 provider（`parent.get_vfs_provider()`）：插件可能在运行期
/// 重新装配，缓存 `Arc<dyn VdfsProvider>` 会钉住旧实例；而取 provider 本身是
/// 一次读，不是热路径。
pub(crate) struct VdfsSessionPort {
    parent: Option<std::sync::Arc<dyn Plugin>>,
    host_ctx: std::sync::Arc<dyn PluginInvokeRequest>,
    session_id: String,
}

impl VdfsSessionPort {
    pub(crate) fn new(
        parent: Option<std::sync::Arc<dyn Plugin>>,
        host_ctx: std::sync::Arc<dyn PluginInvokeRequest>,
        session_id: String,
    ) -> Self {
        Self {
            parent,
            host_ctx,
            session_id,
        }
    }

    /// 发一个动作，把 `data` 反序列化成 `T`；`unit = true` 表示该动作无返回载荷。
    async fn call<P: Serialize, T: DeserializeOwned>(
        &self,
        op: &str,
        payload: P,
        unit: bool,
    ) -> Result<Option<T>, PluginError> {
        let parent = self.parent.as_ref().ok_or_else(|| {
            PluginError::NotFound("会话插件不可达（驱动器没有父插件）".to_string())
        })?;
        let provider = parent.clone().get_vfs_provider().ok_or_else(|| {
            PluginError::NotFound("会话插件不可达（父插件未提供 VDFS 面）".to_string())
        })?;
        let path = format!(
            "{PLUGIN_ID_SESSION}/{}/{}/{}",
            self.session_id,
            run_ops::SEG,
            op
        );
        let ctx = vdfs_context(&self.host_ctx);
        let req = VdfsRequest::Action {
            action: op.to_string(),
            payload: Some(
                serde_json::to_value(payload)
                    .map_err(|e| PluginError::InternalError(format!("动作载荷序列化失败：{e}")))?,
            ),
        };
        let resp = provider
            .dispatch(&ctx, &path, req)
            .await
            .map_err(|e| match e {
                crate::symbio_core::VdfsError::NotFound(m) => PluginError::NotFound(m),
                other => PluginError::InternalError(format!("会话动作派发失败（{op}）：{other}")),
            })?;
        let VdfsResponse::Action(result) = resp else {
            return Err(PluginError::InternalError(format!(
                "动作面返回了非动作响应：{path}"
            )));
        };
        if !result.ok {
            return Err(PluginError::InternalError(format!(
                "会话动作失败（{op}）：{}",
                result.message
            )));
        }
        if unit {
            return Ok(None);
        }
        let data = result
            .data
            .ok_or_else(|| PluginError::InternalError(format!("会话动作缺少返回载荷（{op}）")))?;
        serde_json::from_value(data)
            .map(Some)
            .map_err(|e| PluginError::InternalError(format!("会话动作返回载荷不合法（{op}）：{e}")))
    }

    async fn call_data<P: Serialize, T: DeserializeOwned>(
        &self,
        op: &str,
        payload: P,
    ) -> Result<T, PluginError> {
        self.call(op, payload, false)
            .await?
            .ok_or_else(|| PluginError::InternalError(format!("会话动作缺少返回载荷（{op}）")))
    }

    async fn call_unit<P: Serialize>(&self, op: &str, payload: P) -> Result<(), PluginError> {
        self.call::<P, ()>(op, payload, true).await.map(|_| ())
    }
}

#[async_trait]
impl SessionPort for VdfsSessionPort {
    async fn messages(&self) -> Result<Vec<ChatNode>, PluginError> {
        self.call_data(run_ops::MESSAGES, serde_json::json!({}))
            .await
    }

    async fn context_messages(&self) -> Result<Vec<ChatNode>, PluginError> {
        self.call_data(run_ops::CONTEXT_MESSAGES, serde_json::json!({}))
            .await
    }

    async fn add_by_runner(&self, messages: Vec<ChatNode>) -> Result<Vec<ChatNode>, PluginError> {
        self.call_data(run_ops::ADD_BY_RUNNER, AddNodesRequest { messages })
            .await
    }

    async fn edit_nodes(
        &self,
        nodes: Vec<ChatNode>,
        frame: EditFrame,
    ) -> Result<Vec<ChatNode>, PluginError> {
        self.call_data(run_ops::EDIT_NODES, EditNodesRequest { nodes, frame })
            .await
    }

    async fn remove_nodes(&self, ids: Vec<String>) -> Result<(), PluginError> {
        self.call_unit(run_ops::REMOVE_NODES, RemoveNodesRequest { ids })
            .await
    }

    async fn replace_all(
        &self,
        nodes: Vec<ChatNode>,
        dropped: Vec<String>,
        head: ChatNode,
    ) -> Result<(), PluginError> {
        self.call_unit(
            run_ops::REPLACE_ALL,
            ReplaceAllRequest {
                nodes,
                dropped,
                head,
            },
        )
        .await
    }

    async fn drain_parked(&self) -> Result<(), PluginError> {
        self.call_unit(run_ops::DRAIN_PARKED, serde_json::json!({}))
            .await
    }

    async fn consolidate(&self, messages: Vec<ChatNode>) -> Result<(), PluginError> {
        self.call_unit(run_ops::CONSOLIDATE, ConsolidateRequest { messages })
            .await
    }

    async fn take_supplements(&self) -> Result<Option<DrainedSupplement>, PluginError> {
        let batch: Option<DrainedSupplement> = self
            .call_data(run_ops::TAKE_SUPPLEMENTS, serde_json::json!({}))
            .await?;
        Ok(batch)
    }

    async fn worker_progress(&self) -> Result<Vec<WorkerProgress>, PluginError> {
        self.call_data(run_ops::WORKER_PROGRESS, serde_json::json!({}))
            .await
    }

    async fn recall_snapshots(&self) -> Result<Vec<FleetSnapshot>, PluginError> {
        self.call_data(run_ops::RECALL_SNAPSHOTS, serde_json::json!({}))
            .await
    }

    async fn elimination_revoked(&self) -> Result<bool, PluginError> {
        self.call_data(run_ops::ELIMINATION_REVOKED, serde_json::json!({}))
            .await
    }

    async fn compression_begin(
        &self,
        node_id: &str,
        force: bool,
    ) -> Result<CompressionBeginReply, PluginError> {
        self.call_data(
            run_ops::COMPRESSION_BEGIN,
            CompressionBeginRequest {
                node_id: node_id.to_string(),
                force,
            },
        )
        .await
    }

    async fn compression_finish(
        &self,
        req: CompressionFinishRequest,
    ) -> Result<ChatNode, PluginError> {
        self.call_data(run_ops::COMPRESSION_FINISH, req).await
    }
}
