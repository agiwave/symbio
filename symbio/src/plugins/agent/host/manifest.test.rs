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

/// **带命名空间的 id 必须拒**（[T-210 项一] 的反例用例）。
///
/// 这一条拒的不是「格式不好看」：`authz::principal_of` 对已带命名空间的地址**原样认下**
/// （那是给 `person/local` / `vdfs/write` 这类完整地址准备的分支），所以一旦放行斜杠，
/// `id: person/local` 的包就自己声明了「我属人这一族」——授权表按主体行判，
/// 这一格漂移表现为**包自称的族说了算**。斜杠禁在字符集这一层，族就只剩 core 那一处能定。
#[test]
fn rejects_id_that_self_declares_a_namespace() {
    for bad in [
        "session/forged",
        "person/local",
        "vdfs/write",
        "system/compactor",
        "/leading-slash",
    ] {
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
        assert!(
            validate(&m).is_err(),
            "id `{bad}` 自带命名空间，应被拒绝（族只能由 core 的派生点定）"
        );
    }
    // 正例：裸 slug 照收（它兜底出来的主体恒为 `session/<id>`）。
    let ok = AgentManifest {
        spec: "agent-dir/v2".into(),
        id: "t210-agent".into(),
        name: "n".into(),
        version: "1.0.0".into(),
        requires: Requires {
            spec: "^2".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(validate(&ok).is_ok());
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

/// 权限清单是**声明性质**（[ADR-102](../../../../../docs/010-adr/102.md "ADR-102")）：
/// Rust 侧叫 `declared_permissions`（名字说清它没有执行方），但 Agent 包的公开键
/// 仍是 `permissions`——改名不得变成线格式破坏。
#[test]
fn permissions_stay_on_the_wire_while_the_rust_field_says_declared() {
    let m: AgentManifest = serde_yaml_ng::from_str(
        "permissions:\n  fs:\n    read: [\"data/**\"]\n  network:\n    domains: [\"example.com\"]\n",
    )
    .unwrap();
    assert_eq!(m.declared_permissions.fs.read, vec!["data/**"]);
    assert_eq!(m.declared_permissions.network.domains, vec!["example.com"]);

    let back = serde_yaml_ng::to_string(&m).unwrap();
    assert!(
        back.contains("permissions:"),
        "线格式键必须是 permissions，实得：{back}"
    );
    assert!(
        !back.contains("declared_permissions"),
        "Rust 字段名不得泄漏到 Agent 包的公开键里"
    );
}
