//! `chat_loop/inputs.rs` 的单元测试 —— 系统提示词解析与输入准备。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。
//!
//! 重点钉住 [`resolve_system_prompt`] 的三条口径：
//!
//! - **全送达**：注册的每一段都在，没有「按优先级链取一个」的竞争；
//! - **显式段不吞注册段**：`req.system_prompt` 只决定开头，不决定后面还有没有；
//! - **全空才兜底**：兜底串只在「显式段与注册段都为空」时出现。

use super::*;
use crate::symbio_core::{CapabilityVisitor, DefaultToolVisitor, PluginSimpleRequest};

/// 构造带 CAPABILITY_VISITOR 的上下文，按顺序注册 `prompts`
async fn ctx_prompts(prompts: &[(&str, &str)]) -> Arc<dyn PluginInvokeRequest> {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
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

// ==================== 全送达 ====================

/// 注册的全部段落按注册顺序拼接——人格、记忆、会话记忆各就各位
#[tokio::test]
async fn every_registered_prompt_is_delivered_in_registration_order() {
    let ctx = ctx_prompts(&[("persona", "人格"), ("work", "记忆"), ("agent", "智能体")]).await;
    let got = resolve_system_prompt(None, &ctx).await;
    assert_eq!(got, "人格\n\n记忆\n\n智能体");
}

/// 没有「取一个」的竞争：注册多少就送出多少
#[tokio::test]
async fn no_competition_between_registered_prompts() {
    let ctx = ctx_prompts(&[("a", "A"), ("b", "B"), ("c", "C")]).await;
    let got = resolve_system_prompt(None, &ctx).await;
    for part in ["A", "B", "C"] {
        assert!(got.contains(part), "「{part}」不得被挤掉：{got}");
    }
}

// ==================== 显式段 ====================

/// 关键口径：**显式段不吞掉注册段**——换了调用方也要送达模型
#[tokio::test]
async fn explicit_prompt_leads_but_does_not_swallow_registered_ones() {
    let ctx = ctx_prompts(&[("persona", "注册人格"), ("work", "记忆")]).await;
    let got = resolve_system_prompt(Some("explicit"), &ctx).await;
    assert_eq!(got, "explicit\n\n注册人格\n\n记忆");
}

/// 空 / 纯空白显式段 = 没给，不占位
#[tokio::test]
async fn blank_explicit_prompt_is_ignored() {
    let ctx = ctx_prompts(&[("persona", "注册人格")]).await;
    for blank in ["", "   ", "\n"] {
        let got = resolve_system_prompt(Some(blank), &ctx).await;
        assert_eq!(got, "注册人格", "空白显式段不得产出前导空行");
    }
}

// ==================== 兜底 ====================

#[tokio::test]
async fn fallback_without_visitor() {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let got = resolve_system_prompt(None, &ctx).await;
    assert_eq!(got, FALLBACK_SYSTEM_PROMPT);
}

#[tokio::test]
async fn empty_visitor_registry_uses_fallback() {
    let ctx = ctx_prompts(&[]).await;
    let got = resolve_system_prompt(None, &ctx).await;
    assert_eq!(got, FALLBACK_SYSTEM_PROMPT);
}

/// 兜底只在**全空**时出现：有注册段就不该再见到它
#[tokio::test]
async fn fallback_does_not_ride_along_with_registered_prompts() {
    let ctx = ctx_prompts(&[("work", "记忆")]).await;
    let got = resolve_system_prompt(None, &ctx).await;
    assert_eq!(got, "记忆", "兜底串不该混进来：{got}");
}

// ==================== 空段 ====================

/// 空 / 纯空白段不占位（不产生多余分隔符）
#[tokio::test]
async fn blank_registered_prompts_are_skipped() {
    let ctx = ctx_prompts(&[
        ("persona", "人格"),
        ("a", ""),
        ("b", "   "),
        ("work", "记忆"),
    ])
    .await;
    let got = resolve_system_prompt(None, &ctx).await;
    assert_eq!(got, "人格\n\n记忆");
}
