//! `symbio/src/symbio_core/vdfs/host.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

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
    use crate::symbio_core::PluginSimpleRequest;
    let host: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
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

/// 同一路径重复订阅：计数累加，取消一次仍在（其余订阅者不受影响）
#[tokio::test]
async fn repeated_watch_on_same_path_is_counted() {
    let subs = VdfsChangeSubscriptions::default();
    let seen = Arc::new(Mutex::new(Vec::new()));
    for _ in 0..2 {
        let s = seen.clone();
        subs.watch(
            "a",
            Arc::new(move |c: VdfsChange| s.lock().unwrap().push(c.path)),
        );
    }
    subs.notify(&VdfsChange::bare("a"));
    assert_eq!(subs.subscriber_count(), 2);

    subs.unwatch("a");
    subs.notify(&VdfsChange::bare("a"));
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &["a".to_string(), "a".to_string()],
        "两位订阅者各收一次；取消一位后不再投递"
    );

    subs.unwatch("a");
    assert_eq!(subs.subscriber_count(), 0);
    assert!(!subs.has_subscribers());
}

/// **一条变更恰好出总线一次**：会话清单订根、转写订其子树，一条消息变更
/// 只会被投递一次（**不论落到哪一位订阅者**）⇒ 前端不会收到重复帧而叠字。
///
/// 断言的是**总次数**而不是「谁收到了」：投递器行为相同，选哪一个是确定性的
/// 实现细节，不是契约。把契约写成「窄的那条收到」会让下一个人以为
/// 订阅路径的**具体程度**有语义——它没有。
#[tokio::test]
async fn overlapping_subscriptions_deliver_once() {
    let subs = VdfsChangeSubscriptions::default();
    let broad = Arc::new(Mutex::new(Vec::new()));
    let narrow = Arc::new(Mutex::new(Vec::new()));
    {
        let b = broad.clone();
        subs.watch(
            "",
            Arc::new(move |c: VdfsChange| b.lock().unwrap().push(c.path)),
        );
        let n = narrow.clone();
        subs.watch(
            "abc/message",
            Arc::new(move |c: VdfsChange| n.lock().unwrap().push(c.path)),
        );
    }
    subs.notify(&VdfsChange::bare("abc/message/m1"));
    subs.notify(&VdfsChange::bare("xyz"));

    let hits: Vec<String> = broad
        .lock()
        .unwrap()
        .iter()
        .chain(narrow.lock().unwrap().iter())
        .cloned()
        .collect();
    assert_eq!(
        hits.iter().filter(|p| *p == "abc/message/m1").count(),
        1,
        "两条订阅都相关，但这条变更只能投递一次"
    );
    assert_eq!(
        hits.iter().filter(|p| *p == "xyz").count(),
        1,
        "只与根订阅相关，也必须恰好一次"
    );
}

/// 无相关订阅者时 `notify` 不投递（连通配的窄订阅也不该收到远亲变更）
#[tokio::test]
async fn unrelated_subscription_gets_nothing() {
    let subs = VdfsChangeSubscriptions::default();
    let seen = Arc::new(Mutex::new(Vec::new()));
    {
        let s = seen.clone();
        subs.watch(
            "abc/message",
            Arc::new(move |c: VdfsChange| s.lock().unwrap().push(c.path)),
        );
    }
    // `xyz` 与 `abc/message` 既不同支也不是祖先：不相关
    subs.notify(&VdfsChange::bare("xyz"));
    assert!(seen.lock().unwrap().is_empty());
}

/// 无订阅者时 `notify` 直接返回（不遍历、不投递）
#[tokio::test]
async fn notify_without_subscribers_is_noop() {
    let subs = VdfsChangeSubscriptions::default();
    subs.notify(&VdfsChange::bare("a"));
    assert_eq!(subs.subscriber_count(), 0);
}
