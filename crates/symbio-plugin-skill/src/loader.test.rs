//! Skill 加载器 单元测试
//!
//! 对应源文件: `loader.rs`

use super::*;
use std::fs as stdfs;
use tempfile::TempDir;

/// 创建一个包含指定 body 长度的 SKILL.md 的目录
fn make_skill_dir(parent: &Path, name: &str, body_chars: usize) -> PathBuf {
    let dir = parent.join(name);
    stdfs::create_dir_all(&dir).unwrap();
    let body = "x".repeat(body_chars);
    let md = format!("---\nname: {name}\ndescription: description for {name}\n---\n\n{body}\n",);
    stdfs::write(dir.join("SKILL.md"), md).unwrap();
    dir
}

/// max_skills 截断后续目录
#[tokio::test]
async fn max_skills_truncates_across_dirs() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();
    make_skill_dir(workdir, "skill1", 10);
    make_skill_dir(workdir, "skill2", 10);
    make_skill_dir(workdir, "skill3", 10);

    let budget = LoadBudget {
        max_skills: 2,
        max_body_chars: 8000,
    };
    let dirs = vec![".".to_string()];
    let skills = load_skills_from_dirs_with_budget(&dirs, workdir, budget)
        .await
        .unwrap();
    assert_eq!(skills.len(), 2);
}

/// max_body_chars 截断单个 skill body
#[tokio::test]
async fn max_body_chars_truncates_single_skill() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();
    make_skill_dir(workdir, "big", 5000);

    let budget = LoadBudget {
        max_skills: 20,
        max_body_chars: 100,
    };
    let dirs = vec![".".to_string()];
    let skills = load_skills_from_dirs_with_budget(&dirs, workdir, budget)
        .await
        .unwrap();
    assert_eq!(skills.len(), 1);
    // 截断后 body 长度应 <= 100 + 截断标记
    assert!(skills[0].body.len() < 200);
    assert!(skills[0].body.contains("body truncated"));
}

/// 不存在目录返回空 vec 而不报错
#[tokio::test]
async fn missing_dir_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();
    let dirs = vec!["./nonexistent_dir_xyz".to_string()];
    let skills = load_skills_from_dirs_with_budget(&dirs, workdir, LoadBudget::default())
        .await
        .unwrap();
    assert!(skills.is_empty());
}

/// 正常 SKILL.md 解析成功
#[tokio::test]
async fn parse_normal_skill_md() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("hello");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(
        dir.join("SKILL.md"),
        "---\nname: hello\ndescription: world desc test\nallowedTools:\n  - foo\n  - bar\nargument_hint: <name>\n---\n\nbody content\n",
    )
    .unwrap();
    let skill = parse_skill_file(&dir.join("SKILL.md"), 8000).await.unwrap();
    assert_eq!(skill.name, "hello");
    assert_eq!(skill.description, "world desc test");
    assert_eq!(
        skill.allowed_tools,
        Some(vec!["foo".to_string(), "bar".to_string()])
    );
    assert_eq!(skill.argument_hint, Some("<name>".to_string()));
    assert!(skill.body.contains("body content"));
}

/// 缺 name 字段 → 报错
#[tokio::test]
async fn parse_missing_name_errors() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("skill2");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(
        dir.join("SKILL.md"),
        "---\ndescription: no name here\n---\n\nbody\n",
    )
    .unwrap();
    let err = parse_skill_file(&dir.join("SKILL.md"), 8000)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("name"));
}

/// 缺 description 字段 → 报错
#[tokio::test]
async fn parse_missing_description_errors() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("x");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(dir.join("SKILL.md"), "---\nname: x\n---\n\nbody\n").unwrap();
    let err = parse_skill_file(&dir.join("SKILL.md"), 8000)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("description"));
}

/// 缺 frontmatter → 报错
#[tokio::test]
async fn parse_missing_frontmatter_errors() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("skill4");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(dir.join("SKILL.md"), "no frontmatter at all\n").unwrap();
    let err = parse_skill_file(&dir.join("SKILL.md"), 8000)
        .await
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("frontmatter"));
}

/// disable-model-invocation 默认 false
#[tokio::test]
async fn parse_disable_model_invocation_default_false() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("x");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(
        dir.join("SKILL.md"),
        "---\nname: x\ndescription: desc for x\n---\n\nbody\n",
    )
    .unwrap();
    let skill = parse_skill_file(&dir.join("SKILL.md"), 8000).await.unwrap();
    assert!(!skill.disable_model_invocation);
}

/// SKILL.md 目录名与 frontmatter name 不一致 → 报错
#[tokio::test]
async fn parse_dir_name_must_match_frontmatter_name() {
    let tmp = TempDir::new().unwrap();
    // 目录 "wrong-dir"，但 frontmatter 写 name: right-name
    let dir = tmp.path().join("wrong-dir");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(
        dir.join("SKILL.md"),
        "---\nname: right-name\ndescription: long enough desc\n---\n\nbody\n",
    )
    .unwrap();
    let err = parse_skill_file(&dir.join("SKILL.md"), 8000)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("BUG-SR6"),
        "expected BUG-SR6 error, got: {msg}"
    );
    assert!(msg.contains("wrong-dir"));
    assert!(msg.contains("right-name"));
}

/// SKILL.md 目录名与 frontmatter name 一致 → 成功
#[tokio::test]
async fn parse_dir_name_matches_frontmatter_name_ok() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("matched");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(
        dir.join("SKILL.md"),
        "---\nname: matched\ndescription: long enough desc\n---\n\nbody\n",
    )
    .unwrap();
    let skill = parse_skill_file(&dir.join("SKILL.md"), 8000).await.unwrap();
    assert_eq!(skill.name, "matched");
}

/// 跨目录同名 skill 后加载覆盖
#[tokio::test]
async fn duplicate_skill_name_later_overrides() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let dir1 = root.join("dir1");
    let dir2 = root.join("dir2");
    stdfs::create_dir_all(&dir1).unwrap();
    stdfs::create_dir_all(&dir2).unwrap();

    // dir1 里有 skill "dup"
    stdfs::create_dir_all(dir1.join("dup")).unwrap();
    stdfs::write(
        dir1.join("dup/SKILL.md"),
        "---\nname: dup\ndescription: from dir1 desc\n---\n\nbody1\n",
    )
    .unwrap();
    // dir2 里有同名 skill "dup"
    stdfs::create_dir_all(dir2.join("dup")).unwrap();
    stdfs::write(
        dir2.join("dup/SKILL.md"),
        "---\nname: dup\ndescription: from dir2 desc\n---\n\nbody2\n",
    )
    .unwrap();

    let budget = LoadBudget {
        max_skills: 20,
        max_body_chars: 8000,
    };
    let dirs = vec!["dir1".to_string(), "dir2".to_string()];
    let skills = load_skills_from_dirs_with_budget(&dirs, root, budget)
        .await
        .unwrap();
    assert_eq!(skills.len(), 1, "同名 skill 应去重");
    assert_eq!(
        skills[0].description, "from dir2 desc",
        "后加载的覆盖前加载的"
    );
}

/// 不同名 skill 全部保留
#[tokio::test]
async fn unique_skill_names_all_kept() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let dir1 = root.join("dir1");
    stdfs::create_dir_all(&dir1).unwrap();
    stdfs::create_dir_all(dir1.join("a")).unwrap();
    stdfs::write(
        dir1.join("a/SKILL.md"),
        "---\nname: a\ndescription: description for A\n---\n\n",
    )
    .unwrap();
    stdfs::create_dir_all(dir1.join("b")).unwrap();
    stdfs::write(
        dir1.join("b/SKILL.md"),
        "---\nname: b\ndescription: description for B\n---\n\n",
    )
    .unwrap();
    let budget = LoadBudget::default();
    let dirs = vec!["dir1".to_string()];
    let skills = load_skills_from_dirs_with_budget(&dirs, root, budget)
        .await
        .unwrap();
    assert_eq!(skills.len(), 2);
}

/// `max_skills` 是**清单**的名额，不是**单个目录**的名额。
///
/// 场景：`dir1 = [s1(旧), s2]`、`dir2 = [s1(新), s3]`——同名 `s1` 后到优先（BUG-SR1），
/// `s3` 只在 `dir2` 里。逐目录扣名额的旧写法在这里会出错两次：
///
/// - 预算 3：`dir1` 先占 2 格，`dir2` 只剩 1 格，尾部 `s3` 被截掉——明明 `s1` 的新旧两份
///   去重后只占 1 格，清单却比预算小；
/// - 预算 2：`dir1` 就把名额扣干，`dir2` 整个没被读过——`s1(新)` 顶不掉 `s1(旧)`，
///   后到优先这条判据被名额抢跑。
///
/// 合并后一次裁剪，两个分支各自落在正确的一侧。
#[tokio::test]
async fn budget_trim_applies_to_the_merged_collection() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    for (dir, name, desc) in [
        ("dir1", "s1", "from dir1 s1"),
        ("dir1", "s2", "from dir1 s2"),
        ("dir2", "s1", "from dir2 s1"),
        ("dir2", "s3", "from dir2 s3"),
    ] {
        let skill_dir = root.join(dir).join(name);
        stdfs::create_dir_all(&skill_dir).unwrap();
        stdfs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {desc}\n---\n\nbody\n"),
        )
        .unwrap();
    }
    let dirs = vec!["dir1".to_string(), "dir2".to_string()];

    // 预算 3：去重后的集合恰好填满——`s3` 不该因为 `dir1` 先读就消失
    let loaded = load_skills_from_dirs_with_budget(
        &dirs,
        root,
        LoadBudget {
            max_skills: 3,
            max_body_chars: 8000,
        },
    )
    .await
    .unwrap();
    let names: Vec<&str> = loaded.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        loaded.len(),
        3,
        "清单应有预算这么多个去重后的技能，实得 {names:?}"
    );
    assert!(
        names.contains(&"s3"),
        "尾部目录的技能被名额提前扣干丢了：{names:?}"
    );
    let s1 = loaded
        .iter()
        .find(|s| s.name == "s1")
        .expect("s1 在两个目录里都有");
    assert_eq!(
        s1.description, "from dir2 s1",
        "同名覆盖（后到优先）必须看得见"
    );

    // 预算 2：名额用满，且覆盖仍然赢——旧写法在这里连 `dir2` 都不会打开
    let loaded = load_skills_from_dirs_with_budget(
        &dirs,
        root,
        LoadBudget {
            max_skills: 2,
            max_body_chars: 8000,
        },
    )
    .await
    .unwrap();
    let names: Vec<&str> = loaded.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(loaded.len(), 2, "预算 2 就该给 2 个，实得 {names:?}");
    let s1 = loaded
        .iter()
        .find(|s| s.name == "s1")
        .expect("s1 占了名额，不该被预算挤掉");
    assert_eq!(
        s1.description, "from dir2 s1",
        "裁剪在合并之后 ⇒ 覆盖先于名额"
    );
}

// ===== BUG-SR7：description 长度校验 =====

/// description 长度 < 10 报错
#[tokio::test]
async fn parse_short_description_errors() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("short");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(
        dir.join("SKILL.md"),
        "---\nname: short\ndescription: hi\n---\n\nbody\n",
    )
    .unwrap();
    let err = parse_skill_file(&dir.join("SKILL.md"), 8000)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("BUG-SR7"),
        "expected BUG-SR7 error, got: {msg}"
    );
    assert!(msg.contains("description 太短"));
}

/// description 长度 = 9 (boundary 边界) 报错
#[tokio::test]
async fn parse_description_boundary_just_below_errors() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("nine");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(
        dir.join("SKILL.md"),
        "---\nname: nine\ndescription: \"123456789\"\n---\n\nbody\n",
    )
    .unwrap();
    let err = parse_skill_file(&dir.join("SKILL.md"), 8000)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("BUG-SR7"));
}

/// description 长度 = 10 成功
#[tokio::test]
async fn parse_description_min_length_ok() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("ten");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(
        dir.join("SKILL.md"),
        "---\nname: ten\ndescription: \"1234567890\"\n---\n\nbody\n",
    )
    .unwrap();
    let skill = parse_skill_file(&dir.join("SKILL.md"), 8000).await.unwrap();
    assert_eq!(skill.name, "ten");
    assert_eq!(skill.description, "1234567890");
}

/// description 仅含空白字符仍视为过短
#[tokio::test]
async fn parse_description_whitespace_only_errors() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("ws");
    stdfs::create_dir_all(&dir).unwrap();
    stdfs::write(
        dir.join("SKILL.md"),
        "---\nname: ws\ndescription: '   '\n---\n\nbody\n",
    )
    .unwrap();
    let err = parse_skill_file(&dir.join("SKILL.md"), 8000)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("BUG-SR7"));
}

// ===== BUG-SR8：跳过隐藏目录 =====

/// `.git`、``.symbio` 等隐藏目录被跳过
#[tokio::test]
async fn load_skills_skips_hidden_dirs() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();
    // 正常 skill 应被加载
    make_skill_dir(workdir, "valid", 10);
    // 隐藏目录不应被加载
    make_skill_dir(workdir, ".git", 10);
    make_skill_dir(workdir, ".symbio", 10);
    make_skill_dir(workdir, ".vscode", 10);

    let dirs = vec![".".to_string()];
    let skills = load_skills_from_dirs_with_budget(&dirs, workdir, LoadBudget::default())
        .await
        .unwrap();
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["valid"], "应只加载非隐藏目录");
}

/// 普通 skill 名称以 `.` 开头（罕见但合法）不应被错误跳过
///
/// 实际不会发生（目录名以 `.` 开头 = 隐藏），但保留行为：仅跳过**整目录名**以 `.` 开头。
#[tokio::test]
async fn load_skills_normal_dir_starts_with_dot_is_hidden() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();
    // 目录名 ".foo" 是隐藏目录
    stdfs::create_dir_all(workdir.join(".foo")).unwrap();
    stdfs::write(
        workdir.join(".foo/SKILL.md"),
        "---\nname: .foo\ndescription: dot foo\n---\n\nbody\n",
    )
    .unwrap();

    let dirs = vec![".".to_string()];
    let skills = load_skills_from_dirs_with_budget(&dirs, workdir, LoadBudget::default())
        .await
        .unwrap();
    // .foo 是隐藏目录，应被跳过
    assert!(skills.is_empty(), "整目录名以 . 开头应被跳过");
}
