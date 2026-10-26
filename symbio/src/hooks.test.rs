//! `symbio/src/hooks.rs` 的单元测试 —— 拆自 `symbio_core/plugin/tests.rs`。
//!
//! 钩子是旁路观察者：**失败不得改变判定链**（仍按放行处理），但「钩子跑了」与
//! 「钩子根本没跑」不能同形。判据：`should_proceed` 仍为 true，且 `context` 里带固定标记。

use std::sync::Arc;

use crate::hooks::{fire_hook, HookEvent, HOOK_UNAVAILABLE_MARKER};
use crate::symbio_core::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeResponse, PluginMeta, PluginPayload,
    PluginSimpleRequest,
};

/// 路由恒失败的插件：装配里没有钩子插件 / 钩子插件不可用。
struct BrokenHook;

#[async_trait::async_trait]
impl Plugin for BrokenHook {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("broken-hook", "BrokenHook")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("hook plugin missing".into()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("hook plugin missing".into()))
    }
}

/// 路由成功、但载荷不是钩子输出（外部脚本吐了别的帧 / 版本不匹配）。
struct WrongShapeHook;

#[async_trait::async_trait]
impl Plugin for WrongShapeHook {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("wrong-shape-hook", "WrongShapeHook")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(
            &serde_json::json!({ "not": "a hook output" }),
        ))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&serde_json::json!({})))
    }
}

fn pre_tool_use_ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

fn pre_tool_use() -> HookEvent {
    HookEvent::PreToolUse {
        tool_name: "shell".into(),
        tool_input: serde_json::json!({ "command": "ls" }),
    }
}

#[tokio::test]
async fn fire_hook_marks_a_missing_hook_plugin() {
    let parent: Option<Arc<dyn Plugin>> = Some(Arc::new(BrokenHook));
    let out = fire_hook(&parent, pre_tool_use(), pre_tool_use_ctx()).await;

    assert!(out.should_proceed, "钩子没跑不得拦住执行");
    let context = out.context.unwrap_or_default();
    assert!(
        context.contains(HOOK_UNAVAILABLE_MARKER),
        "必须带固定标记（实际: {context}）"
    );
    assert!(
        context.contains("PreToolUse"),
        "标记要说明是哪个事件（实际: {context}）"
    );
}

#[tokio::test]
async fn fire_hook_marks_an_unparseable_output() {
    let parent: Option<Arc<dyn Plugin>> = Some(Arc::new(WrongShapeHook));
    let out = fire_hook(&parent, pre_tool_use(), pre_tool_use_ctx()).await;

    assert!(out.should_proceed, "输出无法解析同样不得拦住执行");
    let context = out.context.unwrap_or_default();
    assert!(
        context.contains(HOOK_UNAVAILABLE_MARKER) && context.contains("无法解析"),
        "解析失败要与「没有钩子」可分（实际: {context}）"
    );
}

#[tokio::test]
async fn fire_hook_without_a_parent_stays_silent() {
    let out = fire_hook(&None, HookEvent::PreCompact, pre_tool_use_ctx()).await;

    assert!(out.should_proceed);
    assert!(
        out.context.is_none(),
        "没有父插件是合法形态（进程内直调、无编排层），不是失败——不许留标记"
    );
}
