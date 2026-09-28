//! `context::pipeline` 的单元测试。
//!
//! 与实现分文件（`X.rs` + `X.test.rs`，约定见 `CONTRIBUTING.md`）：
//! `pipeline.rs` 只保留生产代码，测试全在本文件。

use super::*;

// ==================== 压缩失败的可诊断性 ====================
//
// 回归动机：实测会话 `09d74431` 的两条 failed 压缩节点 `meta` / `error` 全空，
// 排查者无法判断是「模型请求失败」「模型输出不合法」还是「输入超限」——
// 三种应对完全不同。失败原因必须可机读（kind）且可人读（message）。
//
// 另一条硬不变式：**失败不改动历史**。曾经的"输入超限 → 本地机械兜底截断"
// 会静默丢掉早期历史并回报成功（节点显示"已压缩上下文（N → M 条）"），
// 用户只看到历史突然变短、后续内容与截断前失联，却拿不到原因也没有重试入口。
// 该路径已删除，改为如实报错（`InputOverLimit`）——这三条测试钉住它不再回来。

#[test]
fn failure_kind_is_machine_readable_and_distinct() {
    assert_eq!(
        CompressionFailure::Llm("rate limit".into()).kind(),
        "llm_error"
    );
    assert_eq!(
        CompressionFailure::InvalidSnapshot.kind(),
        "invalid_snapshot"
    );
    assert_eq!(
        CompressionFailure::InputOverLimit {
            pending: 900_000,
            limit: 128_000
        }
        .kind(),
        "input_over_limit"
    );
}

#[test]
fn failure_message_carries_the_reason() {
    // LLM 失败必须带上 provider 的错误文本——它是唯一能区分限流与参数错误的东西
    let llm = CompressionFailure::Llm("429 Too Many Requests".into());
    assert!(llm.message().contains("429 Too Many Requests"));
    assert!(llm.message().contains("模型请求失败"));
    // 三种失败都明示「已保留完整历史」，即失败不是破坏性的
    assert!(CompressionFailure::InvalidSnapshot
        .message()
        .contains("已保留完整历史"));
    // 输入超限必须给出**两侧的数字**（待压缩量 vs 上限），用户才能判断要换多大的模型
    let over = CompressionFailure::InputOverLimit {
        pending: 200_000,
        limit: 128_000,
    };
    let msg = over.message();
    assert!(msg.contains("200000") && msg.contains("128000"), "{msg}");
    assert!(msg.contains("已保留完整历史"), "{msg}");
    // 且必须指向可执行的下一步，而不是只说"失败了"
    assert!(msg.contains("上下文更大的模型"), "{msg}");
}

/// 失败路径的硬不变式：**不裁剪历史**。
///
/// 这条断言的对象是"不再存在"——`emergency_tail_compression` 已随该路径一并删除。
/// 用编译期事实（函数不存在）+ 运行期行为（报错而非返回 Ok）双重钉住：
/// 任何"压缩失败就本地截断"的写法都必须重新引入一个新函数，无法悄悄复活。
#[test]
fn compression_failure_never_truncates_history() {
    // `NoPayoff`（"本地兜底无收益"）这个变体已随兜底路径一起消失：它唯一的语义
    // 来源就是那次截断尝试。枚举里不再有"以截断为出路"的出口。
    let over = CompressionFailure::InputOverLimit {
        pending: 10,
        limit: 1,
    };
    assert_eq!(over.kind(), "input_over_limit");
    assert!(!over.message().contains("截断"), "{}", over.message());
}

#[test]
fn failure_displays_as_message() {
    let f = CompressionFailure::Llm("boom".into());
    // `Display` 即 `message()`：日志里 `auto_compress_process failed: {e}` 能拿到原因
    assert_eq!(format!("{f}"), f.message());
}

// ==================== 压缩节点的结构化交代（`meta`） ====================
//
// 动机：用户最先想知道的两件事——「我离上限还有多远」与「这次是谁触发的」——
// 都不是正文那句话能说清楚的（正文只说条数）。它们是**字段**，由前端按字段渲染，
// 因此字段名与「缺字段意味着什么」必须在这里钉住。

#[test]
fn stats_carry_trigger_both_waters_and_limit() {
    let s = compression_stats("threshold", 200_000, 150_000, Some(30_000), 9);
    assert_eq!(s["compact_trigger"], serde_json::json!("threshold"));
    assert_eq!(s["context_limit"], serde_json::json!(200_000));
    assert_eq!(s["before_tokens"], serde_json::json!(150_000));
    assert_eq!(s["after_tokens"], serde_json::json!(30_000));
    assert_eq!(s["dropped"], serde_json::json!(9));
}

/// 「压缩后水位」只在成功路径存在。
///
/// 失败 / 未触发时编一个 `0` 会在界面上显示成「水位已降到 0」——比不显示更坏：
/// 那会让用户以为上下文已经空了，而实际上历史一条都没动。
#[test]
fn stats_omit_after_tokens_when_there_is_no_such_number() {
    let s = compression_stats("retry", 200_000, 150_000, None, 0);
    assert!(s.get("after_tokens").is_none(), "{s}");
    assert!(s.get("dropped").is_none(), "{s}");
    // 但触发来源与当前水位照样有：用户在失败 / 未触发时更需要这两个数字
    assert_eq!(s["compact_trigger"], serde_json::json!("retry"));
    assert_eq!(s["before_tokens"], serde_json::json!(150_000));
}
