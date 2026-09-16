//! `agent/host/prompt.rs` 的单元测试 —— **人格**条目「一行头信息 + 正文」。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 人格那份自己排版（来源是多文件装配 `prompts/` + `skills/`，不是单文件记忆；
//! 记忆走内核，用例在 `memory.test.rs`）——因此这里除了钉内容，还要钉**简洁**：
//! 这段文字每一轮都要付 token，涨成「使用说明」就是持续的成本。

use super::*;

fn cfg() -> AgentConfig {
    AgentConfig::default()
}

// ==================== 人格 ====================

#[test]
fn identity_segment_carries_identity_address_and_capacity() {
    let seg = identity_segment("com.acme.cr", "你是资深代码审查员。", &cfg());

    assert!(seg.contains("【智能体人格】"), "要有一眼认出的标题: {seg}");
    assert!(seg.contains("com.acme.cr"), "要标明来源 bundle: {seg}");
    assert!(
        seg.contains("你是资深代码审查员。"),
        "人格全文要带上: {seg}"
    );
    assert!(
        seg.contains(".vdfs/agent/com.acme.cr/"),
        "要给出可编辑目录: {seg}"
    );
    // 三类条目的相对路径模板都要给（提示词 / 技能 / MCP）——与 VDFS 寻址规则同源
    for (_, path_hint) in item_address_templates() {
        assert!(seg.contains(path_hint), "缺少 {path_hint} 的模板: {seg}");
    }
    // `agent_identity` **不**出现在头信息里：它已在工具清单中，写进来是每轮白付的
    // token；只在**截断**时才需要指路（见 `truncated_identity_says_so_and_points_at_the_tool`）
    assert!(!seg.contains("agent_identity"), "未截断时不必提工具: {seg}");
    assert!(seg.contains(&cfg().effective_item_max_bytes().to_string()));
    assert!(
        seg.contains(&"你是资深代码审查员。".len().to_string()),
        "要给出当前容量: {seg}"
    );
}

/// **简洁是硬指标**：头信息只有一行，开销远小于注入预算
#[test]
fn identity_head_is_one_line_and_overhead_stays_small() {
    let text = "短人格";
    let seg = identity_segment("b", text, &cfg());

    assert_eq!(seg.lines().count(), 2, "一行头信息 + 正文，不加别的: {seg}");
    let overhead = seg.len() - text.len();
    assert!(
        overhead < 320,
        "头信息开销要小（实际 {overhead} 字节）：{seg}"
    );
}

/// 截断必须明确告知，否则模型会以为自己看到的就是全部人格
#[test]
fn truncated_identity_says_so_and_points_at_the_tool() {
    let c = AgentConfig {
        identity_inject_max_bytes: 8,
        ..AgentConfig::default()
    };
    let seg = identity_segment("b", "0123456789", &c);

    assert!(seg.contains("已截断"), "{seg}");
    assert!(seg.contains("agent_identity"), "要指路去取全文: {seg}");
    assert!(seg.contains("01234567"), "只带预算内的前缀: {seg}");
    assert!(!seg.contains("0123456789"), "{seg}");
    assert_eq!(seg.lines().count(), 3, "截断提示也只占一行: {seg}");
}

#[test]
fn short_identity_is_not_marked_as_truncated() {
    let seg = identity_segment("b", "短人格", &cfg());
    assert!(!seg.contains("已截断"));
    assert!(seg.contains("短人格"));
}

/// 预算按**字节**算，且不切坏多字节字符
#[test]
fn identity_truncation_never_splits_a_character() {
    let c = AgentConfig {
        identity_inject_max_bytes: 4, // "汉字" = 6 字节 → 回退到 3
        ..AgentConfig::default()
    };
    let seg = identity_segment("b", "汉字", &c);
    assert!(seg.contains("汉"));
    assert!(!seg.contains("汉字"));
}

/// 空人格也要产出条目：地址与容量口径是「怎么建立人格」的前提
#[test]
fn empty_identity_still_teaches_where_to_write() {
    let seg = identity_segment("b", "", &cfg());
    assert!(seg.contains(".vdfs/agent/b/"));
    assert!(seg.contains("prompts/"), "要指出人格写在哪个子类别: {seg}");
    assert!(seg.contains("vdfs_write"));
    assert!(!seg.contains("已截断"), "空人格不得谎报截断");
}
