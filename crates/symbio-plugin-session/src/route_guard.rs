//! 只读出口的两道公共守卫：会话 id 校验与读方身份载荷解析。
//!
//! 几条只读路由的入口逐字同一份读法，集中在此避免多份副本各自漂移。

use std::sync::Arc;

use symbio_kernel::plugin::{PluginError, PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_sdk::keys::SESSION_ID;

use crate::stats_reading::ViewerRequest;

/// 取 `metadata.session_id`：缺席 / 空串 ⇒ 拒绝本路由。
pub(crate) fn require_sid(
    ctx: &Arc<dyn PluginInvokeRequest>,
    route: &str,
) -> Result<String, PluginError> {
    let sid = ctx.get(SESSION_ID).unwrap_or_default();
    if sid.is_empty() {
        return Err(PluginError::ValidationError(format!(
            "{route} 需要 metadata.session_id"
        )));
    }
    Ok(sid)
}

/// 解析读方身份声明（可选）：载荷整体缺席或解析不出 ⇒ 拒绝（fail-closed）。
pub(crate) fn viewer_of(
    ctx: &Arc<dyn PluginInvokeRequest>,
    route: &str,
) -> Result<Option<String>, PluginError> {
    Ok(match ctx.payload::<serde_json::Value>()? {
        serde_json::Value::Null => None,
        v => {
            serde_json::from_value::<ViewerRequest>(v)
                .map_err(|e| PluginError::ValidationError(format!("{route} 载荷不合法：{e}")))?
                .principal
        }
    })
}
