//! `symbio/src/plugins/agent/host/vdfs.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// `<id>` 之后是 Agent 目录内的任意相对路径（可多段）
#[test]
fn rel_path_splits_agent_and_inner_path() {
    assert!(matches!(parse_rel_path(""), RelPath::Root));
    assert!(matches!(parse_rel_path("/"), RelPath::Root));
    assert!(matches!(parse_rel_path("b1"), RelPath::Agent { id: "b1" }));
    assert!(matches!(
        parse_rel_path("b1.agent"),
        RelPath::Agent { id: "b1.agent" }
    ));
    match parse_rel_path("b1/skill/foo/SKILL.md") {
        RelPath::File { id, rel } => {
            assert_eq!(id, "b1");
            assert_eq!(rel, "skill/foo/SKILL.md");
        }
        other => panic!("期望 File，实际：{other:?}"),
    }
    // 第二段是记忆文件名 → 记忆，而不是普通文件
    match parse_rel_path("b1/AGENTS.md") {
        RelPath::Memory { id } => assert_eq!(id, "b1"),
        other => panic!("期望 Memory，实际：{other:?}"),
    }
    // 更深处的同名文件仍是普通文件（记忆只在 Agent 根这一层）
    assert!(matches!(
        parse_rel_path("b1/skill/AGENTS.md"),
        RelPath::File { .. }
    ));
}

/// 挂载根下的 `AGENTS.md` 是**本应用自身的指令**，不是名为它的 agent 目录
///
/// 两者不可能相撞：agent id 的首字符必须是小写字母或数字（§5.1），
/// 而保留名以大写 `A` 开头。
#[test]
fn root_agents_md_is_the_host_instruction_not_an_agent_dir() {
    assert!(matches!(
        parse_rel_path(MEMORY_AGENTS_FILE),
        RelPath::Instruction
    ));
    assert!(matches!(parse_rel_path("/AGENTS.md"), RelPath::Instruction));
    // 带子路径时不再命中保留名（那是一条指向不存在条目的普通 agent 目录路径）
    assert!(matches!(
        parse_rel_path("AGENTS.md/x"),
        RelPath::File { .. }
    ));
}

/// 路径末段 → Agent id（去掉 `.agent` 呈现扩展名）
#[test]
fn id_of_strips_presentation_extension() {
    assert_eq!(id_of("demo"), "demo");
    assert_eq!(id_of("demo.agent"), "demo");
}
