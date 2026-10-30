//! 测试替身自身的判据：桩必须**确定性**，否则依赖它的用例会随机红。
//!
//! 判据跟着定义走：**谁定义，谁守**。桩住本 crate，它的确定性就由本文件判——
//! 否则 core 的测试面要继续为一个不属于它的类型背书。

use symbio_core::{AdapterError, LlmAdapter as _, TokenIssuer};
use symbio_kernel::llm::PromptMessage;

use super::StubLlmAdapter;

fn one(text: &str) -> Vec<PromptMessage> {
    vec![PromptMessage {
        role: "user".to_string(),
        text: text.to_string(),
        ..Default::default()
    }]
}

/// 桩的两种演练形态：确定性成功 / 确定性失败。
#[tokio::test]
async fn stub_adapter_succeeds_and_fails_deterministically() {
    let ok = StubLlmAdapter::succeed("stub-model");
    let tok = TokenIssuer::issue_deep();
    let greeting = one("你好");
    let out = ok.generate(&tok, &greeting).await.expect("成功桩必答");
    assert!(out.contains("stub-model"), "{}", out);

    let bad = StubLlmAdapter::always_fail("模型不可用");
    let x = one("x");
    assert!(
        matches!(bad.generate(&tok, &x).await, Err(AdapterError::GenerationFailed(m)) if m.contains("不可用")),
        "失败桩必须确定性失败（演练兜底）"
    );
}
