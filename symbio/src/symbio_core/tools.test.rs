//! `symbio/src/symbio_core/tools.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::llm::turn::TurnOutput;
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
async fn provider_slot_set_get_and_missing() {
    let mgr = DefaultToolVisitor::new();
    // 未注册 → None
    assert!(mgr.get_model_provider().await.is_none());

    // 注册后可取回，身份字段一致
    mgr.register_model_provider(provider("p1")).await;
    let got = mgr.get_model_provider().await;
    let got = got.expect("注册后应可取回");
    assert_eq!(got.provider_id(), "p1");
}

#[tokio::test]
async fn provider_overwrite_replaces_single_slot() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_model_provider(provider("p1")).await;
    mgr.register_model_provider(provider("p2")).await;

    // 单槽覆盖：后注册者生效
    let got = mgr.get_model_provider().await;
    let got = got.expect("覆盖注册后仍应可取回");
    assert_eq!(got.provider_id(), "p2");
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
