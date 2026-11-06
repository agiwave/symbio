//! `crates/symbio-plugin-compose/src/wording.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 这里只测**不需要模型**的部分：系统提示的拼接顺序与覆盖口径。生成路径本身由
//! e2e `t22-compose.mjs` 在真实边界上验（mock LLM）。

use super::*;

/// 注册段在前、指令段在后：顺序反了，指令压不住长人格段里的措辞习惯。
#[test]
fn registered_prompt_stays_in_front_of_the_instruction() {
    let prompt = system_prompt(&ComposeConfig::default(), "你是这个助手。");
    let (head, tail) = prompt.split_once("\n\n").expect("两段之间应当有空行分隔");

    assert_eq!(head, "你是这个助手。");
    assert_eq!(tail, INSTRUCTION);
}

/// 本轮没有装配出注册段（没有会话 / 空提示词）⇒ 只剩指令段，不留一个空段。
#[test]
fn empty_registered_prompt_leaves_only_the_instruction() {
    assert_eq!(system_prompt(&ComposeConfig::default(), ""), INSTRUCTION);
    assert_eq!(
        system_prompt(&ComposeConfig::default(), "  \n "),
        INSTRUCTION
    );
}

/// 配置的 `instruction` 只覆盖**指令段**，注册段不受影响。
#[test]
fn instruction_override_replaces_only_the_tail() {
    let config = ComposeConfig {
        instruction: Some("只答一句。".to_string()),
        ..ComposeConfig::default()
    };

    let prompt = system_prompt(&config, "注册段");
    assert_eq!(prompt, "注册段\n\n只答一句。");
}
