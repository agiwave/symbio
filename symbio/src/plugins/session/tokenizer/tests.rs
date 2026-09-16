//! `tokenizer` 模块的单元测试。
//!
//! 与实现分文件（约定同 `store/tests.rs` / `chat_session/tests.rs`）：
//! `tokenizer.rs` 只保留生产代码，测试全部放本文件。

use super::*;

#[test]
fn chinese_is_not_underestimated_by_byte_length() {
    // 回归守卫：若这里改回 text.len()/4，中文会被估成 0.75 token/字。
    let text = "你好世界";
    let est = default_tokenizer().count(text);
    // 4 个汉字 ≈ 4 token（外加 +1 兜底）
    assert!(est >= 4, "中文估算过低（{est}），疑似又退回按字节数估算");
    // 也不应离谱高估
    assert!(est <= 12, "中文估算过高：{est}");
}

#[test]
fn english_is_roughly_four_chars_per_token() {
    let text = "a".repeat(400);
    let est = default_tokenizer().count(&text);
    // 400 × 0.28 ≈ 112
    assert!((80..=200).contains(&est), "英文估算偏离预期：{est}");
}

#[test]
fn large_input_is_sampled_not_scanned() {
    let big = "a".repeat(2_000_000);
    let est = default_tokenizer().count(&big);
    // 2M × 0.28 ≈ 560k，采样外推应落在合理区间
    assert!((300_000..=900_000).contains(&est), "大输入外推异常：{est}");
}

#[test]
fn calibration_converges_toward_actual() {
    let cal = CalibratedTokenizer::new();
    let text = "x".repeat(10_000);
    let est = cal.count(&text);
    // 假设真实值是估算的 2 倍（模拟中文场景的系统性低估）。
    // 反馈必须用「原始启发式估算」做分母（见 feedback 文档）：
    // 若用校准后的 count() 自反馈，比值会收敛到 √2 而非 2。
    let raw = cal.count_raw(&text);
    let actual = raw * 2;
    for _ in 0..40 {
        cal.feedback(actual, raw);
    }
    let after = cal.count(&text);
    assert!(
        after > est + est / 2,
        "校准未生效：before={est}, after={after}"
    );
}
