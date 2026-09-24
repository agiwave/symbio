//! `symbio/src/plugins/agent/host/manifest.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn unknown_fields_are_ignored() {
    let m: AgentManifest = serde_yaml_ng::from_str(
        "spec: \"agent-dir/v2\"\nid: \"com.acme.cr\"\nname: \"评审\"\nversion: \"1.0.0\"\n\
         requires:\n  spec: \"^2\"\nfuture_field: 1\n",
    )
    .unwrap();
    assert_eq!(m.id, "com.acme.cr");
}

#[test]
fn rejects_spec_and_version_mismatch() {
    let base = "id: \"x\"\nname: \"X\"\nversion: \"1.0.0\"\n";
    let v1 =
        serde_yaml_ng::from_str::<AgentManifest>(&format!("spec: \"oab/v1\"\n{base}")).unwrap();
    assert!(validate(&v1).unwrap_err().contains("agent-dir/v2"));

    let wrong_req = serde_yaml_ng::from_str::<AgentManifest>(&format!(
        "spec: \"agent-dir/v2\"\n{base}requires:\n  spec: \"^1\"\n"
    ))
    .unwrap();
    let e = validate(&wrong_req).unwrap_err();
    assert!(e.contains("版本不匹配"), "{e}");

    let ok = serde_yaml_ng::from_str::<AgentManifest>(&format!(
        "spec: \"agent-dir/v2\"\n{base}requires:\n  spec: \"^2\"\n"
    ))
    .unwrap();
    assert!(validate(&ok).is_ok());
}

#[test]
fn rejects_bad_id() {
    for bad in ["", "Com.Upper", "has space", "-lead", &"x".repeat(129)] {
        let m = AgentManifest {
            spec: "agent-dir/v2".into(),
            id: bad.to_string(),
            name: "n".into(),
            version: "1.0.0".into(),
            requires: Requires {
                spec: "^2".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(validate(&m).is_err(), "id `{bad}` 应被拒绝");
    }
}

#[test]
fn load_reads_yaml_and_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("manifest.yaml"),
        "spec: \"agent-dir/v2\"\nid: \"a\"\nname: \"A\"\nversion: \"1.0.0\"\n",
    )
    .unwrap();
    assert_eq!(load(tmp.path()).unwrap().id, "a");
}
