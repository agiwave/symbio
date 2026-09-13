//! VDFS 宿主桥（symbio 侧）—— 把纯接口接到 symbio 类型上
//!
//! ⚠️ 这是 core 中**唯一**依赖宿主的部分。它只做两件事：
//!
//! 1. **上下文注入**：`Arc<dyn InvokeRequest>` ↔ [`VdfsContext`]
//!    （provider 经 `ctx.require::<Arc<dyn InvokeRequest>>()` 取回宿主句柄）；
//! 2. **错误翻译**：[`VdfsError`] ↔ [`PluginError`] 双向映射。
//!
//! ## 为什么只有这些
//!
//! 纯接口（[`VdfsProvider`] trait 与其域类型）在
//! [`crate::symbio_core::vdfs_provider`]；`vdfs/*` 的**分发**（收集挂载点、
//! 路径解析、树状遍历、事件投递）是 vdfs 插件的职责，见 `plugins/vdfs/host.rs`。
//!
//! 本模块之所以留在 core，是因为**实现方**（如 `setting` 插件）是插件而非
//! 宿主本身——它需要把宿主句柄从 [`VdfsContext`] 里取出、并把调用宿主服务
//! 得到的 [`PluginError`] 翻回协议错误。让它去依赖 `plugins/vdfs` 会破坏
//! 「插件之间不互相依赖」的分层。
//!
//! [`VdfsProvider`]: crate::symbio_core::vdfs_provider::VdfsProvider

use crate::symbio_core::vdfs_provider::{VdfsContext, VdfsError, VdfsResult, VdfsValidationError};
use crate::symbio_core::{InvokeRequest, PluginError};
use std::sync::Arc;

/// [`VdfsError`] → [`PluginError`]
///
/// `Invalid` 的结构化载荷（字段级错误）序列化为 JSON 置于错误文案位，
/// 宿主前端可解析后逐字段高亮；解析失败则按纯文本展示（向前兼容）。
impl From<VdfsError> for PluginError {
    fn from(e: VdfsError) -> Self {
        match e {
            VdfsError::NotFound(m) => PluginError::NotFound(m),
            VdfsError::NotImplemented => PluginError::NotImplemented,
            VdfsError::Invalid(v) => {
                let text = serde_json::to_string(&v).unwrap_or_else(|_| v.message.clone());
                PluginError::ValidationError(text)
            }
            VdfsError::Forbidden(m) => PluginError::Forbidden(m),
            // 宿主错误体系无 Conflict 变体：冲突对使用者同样是「输入不满足前提」，
            // 归入 ValidationError 以便前端在同一处提示
            VdfsError::Conflict(m) => PluginError::ValidationError(format!("冲突：{m}")),
            VdfsError::Internal(m) => PluginError::InternalError(m),
        }
    }
}

/// [`PluginError`] → [`VdfsError`]（provider 调宿主服务后翻译回协议错误）
pub fn from_plugin_error(e: PluginError) -> VdfsError {
    match e {
        PluginError::NotFound(m) => VdfsError::NotFound(m),
        PluginError::NotImplemented => VdfsError::NotImplemented,
        PluginError::ValidationError(m) => match serde_json::from_str::<VdfsValidationError>(&m) {
            Ok(v) => VdfsError::Invalid(v),
            Err(_) => VdfsError::Invalid(VdfsValidationError::new(m)),
        },
        PluginError::Forbidden(m) => VdfsError::Forbidden(m),
        PluginError::InternalError(m) => VdfsError::Internal(m),
        other => VdfsError::Internal(other.to_string()),
    }
}

/// 把 symbio 请求上下文装进不透明 [`VdfsContext`]
pub fn vdfs_context(ctx: &Arc<dyn InvokeRequest>) -> VdfsContext {
    VdfsContext::new(ctx.clone())
}

/// 从 [`VdfsContext`] 取回 symbio 请求上下文
pub fn host_ctx(ctx: &VdfsContext) -> VdfsResult<Arc<dyn InvokeRequest>> {
    ctx.require::<Arc<dyn InvokeRequest>>().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_translation_roundtrip() {
        let e = VdfsError::NotFound("x".into());
        assert!(matches!(PluginError::from(e), PluginError::NotFound(_)));
        assert!(matches!(
            PluginError::from(VdfsError::NotImplemented),
            PluginError::NotImplemented
        ));
        assert!(matches!(
            from_plugin_error(PluginError::Forbidden("f".into())),
            VdfsError::Forbidden(_)
        ));

        // Conflict 归入 ValidationError（宿主无 Conflict 变体）
        assert!(matches!(
            PluginError::from(VdfsError::Conflict("dup".into())),
            PluginError::ValidationError(_)
        ));

        // 字段级校验载荷 → 错误文案位 JSON → 可解析还原
        let payload = VdfsValidationError::new("坏").with_field("port", "越界");
        let text = match PluginError::from(VdfsError::Invalid(payload.clone())) {
            PluginError::ValidationError(t) => t,
            other => panic!("应为校验错误，实为 {other:?}"),
        };
        assert_eq!(
            from_plugin_error(PluginError::ValidationError(text)),
            VdfsError::Invalid(payload)
        );
    }

    /// provider 侧取回宿主句柄：类型匹配给 `Some`，不匹配报 InternalError
    #[test]
    fn host_ctx_downcasts_invoke_request() {
        use crate::symbio_core::SimpleRequest;
        let host: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        let vctx = vdfs_context(&host);
        let back = host_ctx(&vctx).expect("应取回宿主句柄");
        assert!(Arc::ptr_eq(&host, &back));

        // 宿主句柄不可 `Debug`，故不能用 `unwrap_err`（它要求 Ok 侧可 Debug）
        let err = match host_ctx(&VdfsContext::new(1u8)) {
            Ok(_) => panic!("宿主类型不匹配时应报错"),
            Err(e) => e,
        };
        assert_eq!(err.code(), "INTERNAL_ERROR");
    }
}
