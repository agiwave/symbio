//! `symbio/src/symbio_core/plugin/mod.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

fn with_env(pairs: &[(&str, &str)]) -> Arc<PluginSimpleRequest> {
    let ctx = PluginSimpleRequest::new(None, None);
    for (k, v) in pairs {
        lock_write(&ctx.envs).insert((*k).to_string(), (*v).to_string());
    }
    Arc::new(ctx)
}

fn env_of(ctx: &dyn PluginInvokeRequest, key: &str) -> Option<String> {
    ctx.get_raw(key)
        .and_then(|v| v.downcast::<String>().ok())
        .map(|s| (*s).clone())
}

/// `child_of` 继承父环境，且拿到的是**快照**而非共享句柄——改子不影响父。
/// 这是三处装配点（composite / home / agent）收口后的语义契约。
#[test]
fn child_of_snapshots_envs_without_sharing() {
    let parent = with_env(&[("A", "1")]);
    let from: Arc<dyn PluginInvokeRequest> = Arc::clone(&parent) as Arc<dyn PluginInvokeRequest>;

    let child = PluginSimpleRequest::child_of(&from, None);
    assert_eq!(env_of(&child, "A").as_deref(), Some("1"), "应继承父环境");

    lock_write(&child.envs).insert("B".into(), "2".into());
    assert_eq!(env_of(&child, "B").as_deref(), Some("2"));
    assert_eq!(
        env_of(parent.as_ref(), "B"),
        None,
        "子上下文的写入不得回流到父"
    );
}

/// 来源不是标准上下文时无环境可继承，且不得 panic。
#[test]
fn child_of_tolerates_non_standard_source() {
    struct Opaque;
    impl PluginInvokeRequest for Opaque {
        fn get_raw(&self, _key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
            None
        }
        fn set_raw(&self, _key: &str, _value: Arc<dyn Any + Send + Sync>) {}
        fn fork(&self) -> Arc<dyn PluginInvokeRequest> {
            Arc::new(Opaque)
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let from: Arc<dyn PluginInvokeRequest> = Arc::new(Opaque);
    let child = PluginSimpleRequest::child_of(&from, None);
    assert!(lock_read(&child.envs).is_empty());
}

/// `fork` 复制扩展桶：改副本不影响原上下文（转发请求不改写当前链路）。
#[test]
fn fork_copies_extensions_without_sharing() {
    let ctx = PluginSimpleRequest::new(None, None);
    ctx.set_raw("k", Arc::new("v".to_string()));

    let forked = ctx.fork();
    assert_eq!(env_of(forked.as_ref(), "k").as_deref(), Some("v"));

    ctx.set_raw("k", Arc::new("changed".to_string()));
    assert_eq!(
        env_of(forked.as_ref(), "k").as_deref(),
        Some("v"),
        "fork 之后原上下文的变化不应影响副本"
    );
}

// ==================== 钩子没跑起来必须留下名字（T-199） ====================
//
// 钩子是旁路观察者：**失败不得改变判定链**（仍按放行处理），但「钩子跑了」与
// 「钩子根本没跑」不能同形——此前两条失败路径都返回 `HookOutput::default()`，
// 装配错了挂钩插件的会话因此看起来一切正常。判据：`should_proceed` 仍为 true，
// 且 `context` 里带固定标记。

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
