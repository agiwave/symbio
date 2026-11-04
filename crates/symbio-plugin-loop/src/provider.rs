//! `<sid>` 上的 `run_turn` 动作：一轮对话的**唯一入口**（[D-086] §5）。
//!
//! ## 动作体就是整轮 future
//!
//! 本模块不维护任务表、不记轮次、不开通道：授权与令牌校验之后把交付包交给
//! [`entry::run_turn`]，结局以 `TurnReport` 序列化进动作响应。session 侧
//! 拿到的是「这一轮怎么结束的」，不是一串帧——帧已经经 `TurnDispatch.sink` 与
//! 动作面写回会话转写了。
//!
//! 决策 D-086 见 [D-086](../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "D-086")

use symbio_kernel::plugin::PluginInvokeRequestExt;
use symbio_kernel::vdfs::{
    VdfsActionResult, VdfsContext, VdfsError, VdfsRequest, VdfsResponse, VdfsResult,
};
use symbio_sdk::turn::dispatch::{LOOP_RUN_TURN, TURN_DISPATCH};
use symbio_sdk::vdfs::host::vdfs_host_ctx;

use super::entry;

/// 唯一入口（授权三条见 `plugin.rs` 的模块文档）。
pub(super) async fn dispatch(
    ctx: &VdfsContext,
    path: &str,
    req: VdfsRequest,
) -> VdfsResult<VdfsResponse> {
    let VdfsRequest::Action { action, .. } = req else {
        return Err(VdfsError::invalid(format!(
            "驱动器地址只接受 action：{path}"
        )));
    };
    let sid = path.trim_matches('/');
    if sid.is_empty() || sid.contains('/') {
        return Err(VdfsError::not_found(format!("未知驱动器路径：{path}")));
    }
    // 动作名逐字相等（与 session `_run` 面同一条判据：地址与行为两套命名会
    // 让同一个地址借任意动作名绕过可读性）。
    if action != LOOP_RUN_TURN {
        return Err(VdfsError::Forbidden(format!("驱动器未授权动作：{action}")));
    }
    // 令牌：宿主 ctx 上的本轮交付包。没有它 = 不是会话派发进来的调用。
    let host = vdfs_host_ctx(ctx)?;
    let Some(dispatch) = host.get(TURN_DISPATCH) else {
        return Err(VdfsError::Forbidden(format!(
            "run_turn 只接受会话派发（缺少 TURN_DISPATCH）：{path}"
        )));
    };
    if dispatch.spec.session_id != sid {
        return Err(VdfsError::Forbidden(format!(
            "轮次交付包不属于会话 {sid}：{path}"
        )));
    }

    let report = entry::run_turn(host, dispatch).await;
    let data = serde_json::to_value(&report)
        .map_err(|e| VdfsError::internal(format!("轮次结局序列化失败：{e}")))?;
    Ok(VdfsResponse::Action(VdfsActionResult {
        action: LOOP_RUN_TURN.to_string(),
        ok: true,
        message: format!("轮次结束：{:?}", report.status),
        data: Some(data),
    }))
}
