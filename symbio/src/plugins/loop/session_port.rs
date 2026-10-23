//! 驱动器 → 会话的**动作面端口**（[D-086](../../../../docs/003-design/014-单环一表.md "D-086") §4；
//! 收窄见 [D-094](../../../../docs/003-design/014-单环一表.md "D-094")）。
//!
//! 驱动器切成独立插件后仍要读写**本会话**的转写、事实与压缩状态——那些状态的
//! owner 全是 session。E-009 禁止 `use crate::plugins::session`，于是这一侧只认
//! 一个 trait，调用一律经统一地址面 `session/<sid>/_run/<op>` 派发（与 vdfs/route
//! 同款「主动发起流程走端口/协议」的纪律）。
//!
//! ## 为什么只有一个方法
//!
//! 它是一个 `call`（[D-094](../../../../docs/003-design/014-单环一表.md "D-094")）：op 名词表与
//! 请求结构体都住 core（**载荷构造仍是编译期类型化的**），回体类型由调用点在
//! `LoopSession::run_op<T>` 上标注。分派机制不按动作复制——加一个 loop→session
//! 交互不用在这里加方法；丢的只有回体的编译期检查，而那会红，是响声
//! （D-094「代价」1）。
//!
//! ## 为什么 trait 在本插件内
//!
//! 消费方只有 loop 一家，生产实现也只有 VDFS 一家。把 trait 提到 core 会把
//! 「驱动器恰好需要这 15 个动作」冻结成机制层契约——而动作集随驱动器演化
//! （D-083 §4：新增跨墙面要有论证，不是随手加一格）。
//!
//! 测试实现（fake port）同样在本插件内：E-009 连测试一起判，测试路径里的
//! `crate::plugins::session` 同样是墙外引用。
//!
//! ## 授权不在这里
//!
//! 本端口只是**客户端**。三条授权判据（`TURN_DISPATCH` 存在 / 令牌会话相符 /
//! op 在闭集内）全在 session 的 `run_at`——端口不做、也无法做第二遍判定。

use async_trait::async_trait;
use serde_json::Value;

use crate::symbio_core::{
    run_ops, vdfs_context, Plugin, PluginError, PluginInvokeRequest, VdfsRequest, VdfsResponse,
    PLUGIN_ID_SESSION,
};

/// 驱动器对会话动作面的**窄端口**：一个方法，op 词表取 [`run_ops`] 常量。
#[async_trait]
pub(crate) trait SessionPort: Send + Sync {
    /// 发一个内部动作：`op` 取 [`run_ops`] 常量，载荷与回体的形状由 core 的
    /// 请求结构体定（两侧共用的词汇，不在本插件内私有）；无回体的动作
    /// （session 侧 `data: None`）返回 [`Value::Null`]。
    async fn call(&self, op: &str, payload: Value) -> Result<Value, PluginError>;
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
}

#[async_trait]
impl SessionPort for VdfsSessionPort {
    async fn call(&self, op: &str, payload: Value) -> Result<Value, PluginError> {
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
            payload: Some(payload),
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
        Ok(result.data.unwrap_or(Value::Null))
    }
}
