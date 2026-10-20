//! 单环驱动器插件（[ADR-086](../../../../docs/010-adr/086.md "ADR-086") §8）。
//!
//! 本插件**只有一个入口**：`loop/<sid>` 上的 `Action("run_turn")`。动作体就是
//! 整轮 future（`run_chat_loop`），结局以 [`TurnReport`](crate::symbio_core::TurnReport)
//! 序列化进动作响应交回 session。
//!
//! ## 授权三条（与 session 的 `_run` 动作面同一条纪律）
//!
//! 地址面对前端、LLM 工具、心跳全部可达。只看地址等于给「跑任意会话的轮次」开了
//! 公开面，故：
//!
//! 1. 宿主 ctx 上有 [`TURN_DISPATCH`](crate::symbio_core::TURN_DISPATCH)
//!    （`WIRE=false` 的进程内键，线上不可伪造）；
//! 2. `dispatch.spec.session_id == <sid>`（交付包只对**本轮会话**有效）；
//! 3. 动作名逐字等于 `run_turn`，路径形如 `<sid>`（没有子路径）。
//!
//! ## 为什么 mount 行是 1、能力行是 0
//!
//! 本插件对前端与 LLM 都不贡献任何可列举的东西：没有配置文档、没有工具、没有
//! 独立资源树。挂载点是**回程地址**——session 经容器找 `loop` 的 provider 派发
//! `run_turn`（[ADR-086] §5），与前端经 VDFS 看它是两件事。

use async_trait::async_trait;
use std::sync::Arc;

use crate::symbio_core::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, VdfsContext, VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult,
    PLUGIN_ID_LOOP,
};

/// 单环驱动器。
///
/// 无字段：本插件没有配置、常驻任务与自有存储；作用域容器经 `TurnDispatch.parent`
/// 随每次 `run_turn` 交付（不在这里钉住兄弟插件实例）。
pub struct LoopPlugin;

impl LoopPlugin {
    /// 静态工厂：本插件没有要装配的东西。
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(LoopPlugin)
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_LOOP, "单环驱动器")
            .with_description("一轮对话的执行引擎：事实入格、工具循环与收束。仅供会话内部派发。")
            .with_version("0.1.0")
            .with_order(90)
            .with_icon("loop")
            .with_hidden(true)
    }
}

#[async_trait]
impl VdfsProvider for LoopPlugin {
    /// 唯一入口：`<sid>` 上的 `Action("run_turn")`（见模块文档的授权三条）。
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        super::provider::dispatch(ctx, path, req).await
    }
}

#[async_trait]
impl Plugin for LoopPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
        Some(self)
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!("未知路径: {path}")))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_ID_LOOP, LoopPlugin::build, dyn Plugin);
