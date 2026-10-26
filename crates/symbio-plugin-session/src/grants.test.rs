//! `session/grant` 的契约锁：严格解析、只抬不降、返回新清单。
//!
//! 这三条各自对应一个**会静默出错**的方向：
//! - 未知风险档若被宽松折成 Medium，「永久放行」就会被写成比用户点的那一档更低
//!   （或更高）而不报错；
//! - 允许降低上限，「本会话放行」之后的一次误触就能把既有授权收紧；
//! - 响应该回**新清单**而不是回显请求：前端据此刷新的正是这张表。

use super::super::config::SessionConfig;
use super::super::plugin::SessionPlugin;
use std::sync::Arc;
use symbio_core::{
    CapabilityRiskLevel, PluginDir, PluginError, PluginInvokeRequest, PluginSimpleRequest,
};

/// 本用例独占的插件目录（`test_dir()` 是共享临时目录，多个写盘用例会互相覆盖）。
fn scratch_dir(tag: &str) -> PluginDir {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    PluginDir::at(
        std::env::temp_dir().join(format!("symbio-grant-{tag}-{}-{n}", std::process::id())),
        "session",
    )
}

fn plugin(tag: &str, config: SessionConfig) -> Arc<SessionPlugin> {
    Arc::new(SessionPlugin::new(None, config, scratch_dir(tag)))
}

/// 构造带 payload 的请求上下文（`payload()` 读的正是 `"payload"` 桶）。
fn request(payload: serde_json::Value) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.extensions
        .write()
        .unwrap()
        .insert("payload".to_string(), Arc::new(payload));
    Arc::new(req)
}

/// 升档：写入成功、返回新清单、内存槽同步（不重读磁盘也知道新值）。
#[tokio::test]
async fn grant_raises_ceiling_and_returns_new_map() {
    let p = plugin("raise", SessionConfig::default());
    let resp = p
        .handle_grant(request(
            serde_json::json!({ "tool": "shell", "risk": "high" }),
        ))
        .await
        .expect("合法请求应写入");

    assert_eq!(
        p.config.read().await.approval_grants.ceiling("shell"),
        Some(CapabilityRiskLevel::High)
    );
    let body = resp.serialize().expect("载荷应可序列化");
    assert_eq!(
        body,
        serde_json::json!({ "shell": "high" }),
        "响应是写后的整张清单，不是回显请求"
    );
}

/// 只抬不降：已在高档时再点低档，上限保持高档。
#[tokio::test]
async fn grant_only_raises_never_lowers() {
    let mut config = SessionConfig::default();
    config
        .approval_grants
        .raise("shell", CapabilityRiskLevel::High);
    let p = plugin("no-lower", config);

    p.handle_grant(request(
        serde_json::json!({ "tool": "shell", "risk": "low" }),
    ))
    .await
    .expect("只抬不降不是错误");

    assert_eq!(
        p.config.read().await.approval_grants.ceiling("shell"),
        Some(CapabilityRiskLevel::High)
    );
}

/// 未知风险档**拒绝**，而不是折成默认档：写错的词不能变成一次真实授权。
#[tokio::test]
async fn unknown_risk_is_rejected_without_writing() {
    let p = plugin("bad-risk", SessionConfig::default());
    let err = p
        .handle_grant(request(
            serde_json::json!({ "tool": "shell", "risk": "critical" }),
        ))
        .await
        .expect_err("未知风险档应拒绝");

    assert!(matches!(err, PluginError::ValidationError(_)), "{err:?}");
    assert_eq!(
        p.config.read().await.approval_grants.ceiling("shell"),
        None,
        "被拒的请求不得留下任何授权"
    );
}

/// 空白工具名拒绝：能力名为空时这本账没有主语。
#[tokio::test]
async fn blank_tool_is_rejected() {
    let p = plugin("blank-tool", SessionConfig::default());
    let err = p
        .handle_grant(request(
            serde_json::json!({ "tool": "   ", "risk": "high" }),
        ))
        .await
        .expect_err("空工具名应拒绝");

    assert!(matches!(err, PluginError::ValidationError(_)), "{err:?}");
}
