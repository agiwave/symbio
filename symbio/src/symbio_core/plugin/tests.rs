//! `symbio/src/symbio_core/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
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
