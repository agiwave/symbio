//! SkillPlugin 单元测试
//!
//! 对应源文件: `plugin.rs`

use super::*;
use std::fs as stdfs;
use tempfile::TempDir;

/// 把 dir 路径展开成绝对 forward-slash 路径
fn expand(dir: &str) -> String {
    if let Some(stripped) = dir.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(stripped).to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| dir.to_string())
    } else {
        dir.to_string()
    }
}

/// TEST-S5.1：`~/.symbio/plugins/skills` 路径归类为 system
#[test]
fn classify_system_skills_dir() {
    let dir = "~/.symbio/plugins/skills".to_string();
    let file_path = format!("{}/foo/SKILL.md", expand(&dir));
    let result = SkillPlugin::classify_skill_source(&file_path, &[dir]);
    assert_eq!(result, "system");
}

/// TEST-S5.2：`~` 开头的目录归类为 system
#[test]
fn classify_tilde_prefix_as_system() {
    let dir = "~/my-skills".to_string();
    let file_path = format!("{}/foo/SKILL.md", expand(&dir));
    let result = SkillPlugin::classify_skill_source(&file_path, &[dir]);
    assert_eq!(result, "system");
}

/// TEST-S5.3：`.qwen` `.sixth` `.qoder` 归类为 external
#[test]
fn classify_external_dirs() {
    let dirs = vec![
        ".qwen/skills".to_string(),
        ".sixth/skills".to_string(),
        ".qoder/skills".to_string(),
    ];
    // 相对路径会被展开为 <home>/.qwen/skills（与用户实际使用一致）
    let base = expand(".qwen/skills");
    let base2 = expand(".sixth/skills");
    let base3 = expand(".qoder/skills");
    assert_eq!(
        SkillPlugin::classify_skill_source(&format!("{base}/foo/SKILL.md"), &dirs),
        "external"
    );
    assert_eq!(
        SkillPlugin::classify_skill_source(&format!("{base2}/foo/SKILL.md"), &dirs),
        "external"
    );
    assert_eq!(
        SkillPlugin::classify_skill_source(&format!("{base3}/foo/SKILL.md"), &dirs),
        "external"
    );
}

/// TEST-S5.4：`.symbio/skills` 归类为 system（用户工作区内的 symbio 技能）
#[test]
fn classify_dot_symbio_skills_as_system() {
    let dirs = vec![".symbio/skills".to_string()];
    let base = expand(".symbio/skills");
    let result = SkillPlugin::classify_skill_source(&format!("{base}/foo/SKILL.md"), &dirs);
    assert_eq!(result, "system");
}

/// TEST-S5.5：其它目录归类为 workspace
#[test]
fn classify_other_as_workspace() {
    let dirs = vec!["my_project/skills".to_string()];
    let base = expand("my_project/skills");
    let result = SkillPlugin::classify_skill_source(&format!("{base}/foo/SKILL.md"), &dirs);
    assert_eq!(result, "workspace");
}

/// TEST-S5.6：file_path 不匹配任何 dir → unknown
#[test]
fn classify_unknown_when_no_match() {
    let dirs = vec!["some/dir".to_string()];
    let result = SkillPlugin::classify_skill_source("/work/other/path/SKILL.md", &dirs);
    assert_eq!(result, "unknown");
}

// ===== BUG-FR9：get_skill_detail 测试 =====

/// 创建一个 skill 目录（用于 get_skill_detail 测试）
fn make_skill(parent: &std::path::Path, dir_name: &str, body: &str) {
    let dir = parent.join(dir_name);
    stdfs::create_dir_all(&dir).unwrap();
    let md = format!(
        "---\nname: {dir_name}\ndescription: description for {dir_name} (long enough)\n---\n\n{body}\n"
    );
    stdfs::write(dir.join("SKILL.md"), md).unwrap();
}

/// 构造一个 SkillPlugin，skill_dirs 指向临时目录
async fn make_plugin_with_tmpdir(tmp: &TempDir) -> SkillPlugin {
    let skill_dir = tmp.path().to_string_lossy().replace('\\', "/");
    let config = SkillConfig {
        skill_dirs: vec![skill_dir],
        max_skills: 20,
        max_body_chars: 8000,
        report_token_estimate: true,
    };
    SkillPlugin::new(config)
}

/// TEST-FR9.1：成功获取 skill 详情（含 body）
#[tokio::test]
async fn get_skill_detail_returns_body() {
    let tmp = TempDir::new().unwrap();
    make_skill(tmp.path(), "demo", "this is the body content of demo skill");
    let plugin = make_plugin_with_tmpdir(&tmp).await;

    let detail = plugin
        .get_skill_detail("demo", None)
        .await
        .expect("get_skill_detail should succeed");
    assert_eq!(detail.name, "demo");
    assert!(detail.body.contains("body content of demo skill"));
    assert!(detail.body_chars > 0);
    assert!(!detail.body_truncated);
}

/// TEST-FR9.2：未找到 → PluginError::NotFound
#[tokio::test]
async fn get_skill_detail_not_found_errors() {
    let tmp = TempDir::new().unwrap();
    make_skill(tmp.path(), "exists", "body");
    let plugin = make_plugin_with_tmpdir(&tmp).await;

    let err = plugin
        .get_skill_detail("nonexistent", None)
        .await
        .expect_err("expected NotFound error");
    assert!(err.to_string().contains("Skill not found"));
}

/// TEST-FR9.3：body 超出 max_body_chars → 标记 truncated
#[tokio::test]
async fn get_skill_detail_marks_truncated_when_over_budget() {
    let tmp = TempDir::new().unwrap();
    let big = "x".repeat(5000);
    make_skill(tmp.path(), "big", &big);
    let mut plugin = make_plugin_with_tmpdir(&tmp).await;
    // 缩小预算
    {
        let cfg = plugin.config.write().await;
        // SkillConfig 默认 max_body_chars = 8000，重新 new 一个 plugin 更简单
        let _ = cfg;
    }
    let config = SkillConfig {
        skill_dirs: vec![tmp.path().to_string_lossy().replace('\\', "/")],
        max_skills: 20,
        max_body_chars: 100,
        report_token_estimate: true,
    };
    plugin = SkillPlugin::new(config);

    let detail = plugin
        .get_skill_detail("big", None)
        .await
        .expect("get_skill_detail should succeed");
    assert!(detail.body_truncated, "body 超出预算应被截断");
    assert!(detail.body.contains("body truncated"));
}
