//! `chat_loop` 模块的单元测试 —— tests。
//!
//! 与实现分文件（约定同 `store/tests.rs` / `chat_session/tests.rs`）：
//! `chat_loop.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::{CapabilityVisitor, DefaultToolVisitor, SimpleRequest};

/// 构造带 CAPABILITY_VISITOR 的上下文；`prompts` 为 (名称, 内容) 注册表。
async fn ctx_prompts(prompts: &[(&str, &str)]) -> Arc<dyn InvokeRequest> {
    let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    let visitor = Arc::new(DefaultToolVisitor::new());
    for (name, text) in prompts {
        visitor.register_system_prompt(name, text.to_string()).await;
    }
    ctx.set(
        crate::symbio_core::CAPABILITY_VISITOR,
        visitor as Arc<dyn CapabilityVisitor>,
    );
    ctx
}

#[tokio::test]
async fn explicit_prompt_wins() {
    let ctx = ctx_prompts(&[("default", "from-visitor")]).await;
    let got = resolve_system_prompt(Some("explicit"), Some("openai"), &ctx).await;
    assert_eq!(got, "explicit");
}

#[tokio::test]
async fn empty_explicit_prompt_falls_through_to_default() {
    let ctx = ctx_prompts(&[("openai", "from-openai"), ("default", "from-default")]).await;
    let got = resolve_system_prompt(Some(""), Some("openai"), &ctx).await;
    assert_eq!(got, "from-default", "default 优先于 provider_id 匹配");
}

#[tokio::test]
async fn provider_id_matches_registered_key() {
    let ctx = ctx_prompts(&[("anthropic", "a"), ("openai", "o")]).await;
    let got = resolve_system_prompt(None, Some("openai"), &ctx).await;
    assert_eq!(got, "o");
}

#[tokio::test]
async fn first_registered_when_no_default_or_provider_match() {
    let ctx = ctx_prompts(&[("anthropic", "a"), ("openai", "o")]).await;
    let got = resolve_system_prompt(None, Some("unknown"), &ctx).await;
    assert_eq!(got, "a", "保序取首个注册项");
}

#[tokio::test]
async fn fallback_without_visitor() {
    let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    let got = resolve_system_prompt(None, None, &ctx).await;
    assert_eq!(got, "You are a helpful MODEL assistant.");
}

#[tokio::test]
async fn empty_visitor_registry_uses_fallback() {
    let ctx = ctx_prompts(&[]).await;
    let got = resolve_system_prompt(None, None, &ctx).await;
    assert_eq!(got, "You are a helpful MODEL assistant.");
}
