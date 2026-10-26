//! 技能指令组装的判据：三条路径（`skill/execute` / 名称路由 / `read_skill`）只此一份实现。

use super::assemble_response;
use crate::types::Skill;
use symbio_core::PluginError;

fn skill(argument_hint: Option<&str>) -> Skill {
    Skill {
        name: "demo".into(),
        description: "demo skill".into(),
        body: "run ${target}".into(),
        allowed_tools: None,
        argument_hint: argument_hint.map(str::to_string),
        when_to_use: None,
        model: None,
        disable_model_invocation: false,
        file_path: "/skills/demo/SKILL.md".into(),
    }
}

#[test]
fn response_has_single_prefix_and_substitutes_args() {
    let out = assemble_response(&skill(None), Some(r#"{"target":"build"}"#)).expect("组装");

    assert_eq!(out.base_dir, "/skills/demo");
    assert!(
        out.body
            .starts_with("Base directory for this skill: /skills/demo\n"),
        "前缀在正文之前：{}",
        out.body
    );
    assert!(out.body.ends_with("run build"), "参数已替换：{}", out.body);
    assert_eq!(out.args, Some(serde_json::json!({"target": "build"})));
}

#[test]
fn declared_argument_hint_requires_args_on_every_path() {
    let err = assemble_response(&skill(Some("<target>")), None).expect_err("缺参数必须报错");
    match err {
        PluginError::ValidationError(msg) => {
            assert!(msg.contains("demo"), "错误要带技能名：{msg}");
            assert!(msg.contains("<target>"), "错误要带参数提示：{msg}");
        }
        other => panic!("应为校验错误，实得 {other:?}"),
    }
}

#[test]
fn declared_argument_hint_accepts_args() {
    let out = assemble_response(&skill(Some("<target>")), Some(r#"{"target":"test"}"#))
        .expect("带参数应组装成功");
    assert!(out.body.ends_with("run test"), "{}", out.body);
}

#[test]
fn skill_without_hint_needs_no_args() {
    let out = assemble_response(&skill(None), None).expect("无参数技能可直接组装");
    assert!(
        out.body.ends_with("run ${target}"),
        "无参数时模板原样保留：{}",
        out.body
    );
    assert_eq!(out.args, None);
}

#[test]
fn invalid_args_json_is_a_validation_error() {
    let err = assemble_response(&skill(Some("<target>")), Some("{not json"))
        .expect_err("非法 JSON 必须报错");
    assert!(
        matches!(err, PluginError::ParseError(_)),
        "非法参数应转为解析错误，实得 {err:?}"
    );
}
