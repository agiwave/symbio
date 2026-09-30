//! `providers/collectors/tool_visitor.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//! 这三条原在 `symbio_core/capability/tools.test.rs`，随默认收集器一起迁出 core。

use super::*;

use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::TurnOutput;
use crate::symbio_core::{CapabilityMeta, ExecEnv, PluginError};
use async_trait::async_trait;

/// 最小模型服务桩：实现纯 trait 契约，仅用于验证注册存储语义，不发起真实请求
struct MockProvider {
    tag: String,
}

#[async_trait]
impl ModelProvider for MockProvider {
    fn provider_id(&self) -> &str {
        &self.tag
    }

    fn api_protocol(&self) -> &str {
        "openai_chat"
    }

    fn rate_limit_ms(&self) -> u64 {
        0
    }

    fn max_context_tokens(&self) -> u32 {
        262_144
    }

    async fn effective_context_tokens(&self) -> u32 {
        262_144
    }

    async fn execute_turn(
        &self,
        _system_prompt: &str,
        _messages: &[ChatMessage],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        _env: &ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        Err(PluginError::InternalError("mock".to_string()))
    }
}

fn provider(id: &str) -> Arc<dyn ModelProvider> {
    Arc::new(MockProvider {
        tag: id.to_string(),
    })
}

#[tokio::test]
async fn model_slot_set_get_and_missing() {
    let mgr = DefaultToolVisitor::new();
    // 未注册 → 生效者与目录都是空
    assert!(mgr.get_model_provider().await.is_none());
    assert!(mgr.get_model_provider_by_id("p1").await.is_none());

    // 注册后可取回，身份字段一致
    mgr.register_model_providers(Some("p1"), vec![provider("p1")])
        .await;
    let got = mgr.get_model_provider().await;
    let got = got.expect("注册后应可取回");
    assert_eq!(got.provider_id(), "p1");
}

/// **空集是一等输入**：一个可用的都没有时，两个取值口都给 `None`（不 panic、不补占位）
#[tokio::test]
async fn empty_registration_yields_no_provider() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_model_providers(None, vec![]).await;
    assert!(mgr.get_model_provider().await.is_none());
    assert!(mgr.get_model_provider_by_id("p1").await.is_none());
}

/// 生效者**从目录里取**：`active_id` 在目录里查不到 ⇒ `None`，**不**替调用方挑一个别的
///
/// 静默换模型比没有模型更难查——会话会用另一个模型跑完一整轮，而配置里那个坏条目
/// 永远不会被注意到。
#[tokio::test]
async fn active_id_absent_from_catalog_yields_none() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_model_providers(Some("missing"), vec![provider("p1")])
        .await;
    assert!(mgr.get_model_provider().await.is_none());
    // 目录本身照常可用（一个配坏的条目不该把其余可用的模型一起带走）
    assert_eq!(
        mgr.get_model_provider_by_id("p1")
            .await
            .unwrap()
            .provider_id(),
        "p1"
    );
}

/// 目录按 `provider_id` 索引，且是**严格查找**：不在目录里就是 `None`，
/// 不降级到生效者（插件问的是"有没有这一个"）
#[tokio::test]
async fn catalog_lookup_is_by_id_and_strict() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_model_providers(Some("p1"), vec![provider("p1"), provider("p2")])
        .await;

    assert_eq!(
        mgr.get_model_provider_by_id("p2")
            .await
            .unwrap()
            .provider_id(),
        "p2"
    );
    assert!(mgr.get_model_provider_by_id("nope").await.is_none());
    // 生效者仍是 p1（目录不影响它）
    assert_eq!(mgr.get_model_provider().await.unwrap().provider_id(), "p1");
}

/// 重新注册**整体替换**目录（不是累积）：provider 被停用 / 删除后，
/// 旧实例不该留在目录里被插件取到
#[tokio::test]
async fn reregistration_replaces_the_catalog() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_model_providers(Some("p1"), vec![provider("p1"), provider("p2")])
        .await;
    mgr.register_model_providers(Some("p2"), vec![provider("p2")])
        .await;

    assert!(mgr.get_model_provider_by_id("p1").await.is_none());
    assert_eq!(
        mgr.get_model_provider_by_id("p2")
            .await
            .unwrap()
            .provider_id(),
        "p2"
    );
    assert_eq!(mgr.get_model_provider().await.unwrap().provider_id(), "p2");
}

/// 同名覆盖只改内容、**不改槽位**（顺序 = 首次注册顺序）
#[tokio::test]
async fn system_prompt_overwrite_keeps_first_registration_slot() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_system_prompt("persona", "默认提示词".to_string())
        .await;
    mgr.register_system_prompt("work", "记忆".to_string()).await;
    mgr.register_system_prompt("persona", "默认提示词（覆盖）".to_string())
        .await;
    mgr.register_system_prompt("agent", "人格".to_string())
        .await;

    assert_eq!(
        mgr.list_system_prompts().await,
        vec![
            ("persona".to_string(), "默认提示词（覆盖）".to_string()),
            ("work".to_string(), "记忆".to_string()),
            ("agent".to_string(), "人格".to_string()),
        ]
    );
}
