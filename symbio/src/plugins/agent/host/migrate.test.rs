//! `symbio/src/plugins/agent/host/migrate.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::plugins::agent::host::manifest;
use std::path::PathBuf;

fn v1_dir(tmp: &Path, id: &str) -> PathBuf {
    let d = tmp.join(id);
    std::fs::create_dir_all(d.join("prompts")).unwrap();
    std::fs::create_dir_all(d.join("skills").join("playbook")).unwrap();
    std::fs::create_dir_all(d.join("mcps")).unwrap();
    std::fs::write(
        d.join("manifest.yaml"),
        format!("spec: \"oab/v1\"\nid: \"{id}\"\nname: \"测试\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^1\"\n"),
    )
    .unwrap();
    std::fs::write(
        d.join("prompts").join("persona.md"),
        "---\npriority: 0\n---\n\n你是测试。\n",
    )
    .unwrap();
    std::fs::write(
        d.join("skills").join("playbook").join("SKILL.md"),
        "---\nname: playbook\ndescription: 交付流程手册\n---\n\n# 正文\n",
    )
    .unwrap();
    std::fs::write(
        d.join("mcps").join("demo.yaml"),
        "command: npx\nargs:\n  - \"-y\"\n  - \"demo\"\n",
    )
    .unwrap();
    d
}

#[test]
fn migrates_v1_layout_to_v2() {
    let tmp = tempfile::tempdir().unwrap();
    let d = v1_dir(tmp.path(), "demo");

    assert!(migrate_v1_to_v2(&d).unwrap());

    // manifest 的 spec 已改写
    assert_eq!(
        read_spec(&d.join("manifest.yaml")).as_deref(),
        Some(SPEC_V2)
    );

    // 兼容门槛同步升到 v2 —— 只改 spec 会得到被 §10 拒绝的清单（本用例即回归守卫）。
    let m = manifest::load(&d).unwrap();
    assert_eq!(m.requires.spec, format!("^{SPEC_MAJOR}"));
    assert!(
        manifest::validate(&m).is_ok(),
        "迁移产物必须能过 §10 接入门槛：{:?}",
        manifest::validate(&m)
    );
    // prompts → 根 AGENTS.md（frontmatter 已剥离）
    assert_eq!(
        std::fs::read_to_string(d.join("AGENTS.md")).unwrap().trim(),
        "你是测试。"
    );
    // skills → skill
    assert!(d.join("skill").join("playbook").join("SKILL.md").exists());
    assert!(!d.join("skills").exists());
    // mcps → mcp/<n>/server.json，transport 改名为 type
    let server: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(d.join("mcp").join("demo").join("server.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(server["type"], "stdio");
    assert_eq!(server["command"], "npx");
}

#[test]
fn is_idempotent_and_ignores_non_v1() {
    let tmp = tempfile::tempdir().unwrap();
    let d = v1_dir(tmp.path(), "demo");
    assert!(migrate_v1_to_v2(&d).unwrap());
    // 第二次执行：无事可做
    assert!(!migrate_v1_to_v2(&d).unwrap());

    // 非 v1 目录不动
    let other = tmp.path().join("plain");
    std::fs::create_dir_all(&other).unwrap();
    assert!(!migrate_v1_to_v2(&other).unwrap());
}
