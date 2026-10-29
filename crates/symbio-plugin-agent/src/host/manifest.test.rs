//! `crates/symbio-plugin-agent/src/host/manifest.rs` 的单元测试 —— 拆自源码末尾的测试模块。
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

/// **带命名空间的 id 必须拒**（反例用例）。
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

/// 权限清单的**线格式键是 `permissions`**，Rust 侧字段同名（执行面落地后从
/// `declared_permissions` 改回，D-140 的两态之名）——两轮改名都不曾动过公开键。
#[test]
fn permissions_stay_on_the_wire_under_their_own_name() {
    let m: AgentManifest = serde_yaml_ng::from_str(
        "permissions:\n  fs:\n    read: [\"data/**\"]\n  network:\n    domains: [\"example.com\"]\n",
    )
    .unwrap();
    assert_eq!(m.permissions.fs.read, vec!["data/**"]);
    assert_eq!(m.permissions.network.domains, vec!["example.com"]);

    let back = serde_yaml_ng::to_string(&m).unwrap();
    assert!(
        back.contains("permissions:"),
        "线格式键必须是 permissions，实得：{back}"
    );
    assert!(
        !back.contains("declared_permissions"),
        "旧名不得回到 Agent 包的公开键里"
    );
}

/// 空清单是**合法形状**（不是错）：它意味着「一个授权都没要」，运行期据此全拒。
/// 拒它会把「作者忘了写」和「作者刻意不授权」混成同一个错。
#[test]
fn an_empty_permissions_block_is_valid_and_means_no_grant() {
    let m: AgentManifest = serde_yaml_ng::from_str(
        "spec: \"agent-dir/v2\"\nid: \"a\"\nname: \"A\"\nversion: \"1.0.0\"\n\
         requires:\n  spec: \"^2\"\npermissions: {}\n",
    )
    .unwrap();
    assert!(validate(&m).is_ok(), "{:?}", validate(&m).err());
    assert!(m.permissions.is_empty());
    assert!(!m.permissions.host_allowed("example.com"), "空 ⇒ 拒");
    assert!(!m.permissions.path_allowed("AGENTS.md", false), "空 ⇒ 拒");
}

/// glob 含 `..` ⇒ **装载期拒**（§5.1）。它现在是执行面的输入：放行一个能穿越到
/// 目录外的 glob，整份读 / 写清单就失去意义（`../..` + `**` = 整个磁盘）。
#[test]
fn a_glob_that_escapes_the_package_is_rejected_at_load() {
    let base = "spec: \"agent-dir/v2\"\nid: \"a\"\nname: \"A\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^2\"\n";
    for bad in ["../**", "a/../../b", r"..\..\x"] {
        let yaml = format!("{base}permissions:\n  fs:\n    read:\n      - '{bad}'\n");
        let m = serde_yaml_ng::from_str::<AgentManifest>(&yaml).unwrap();
        let e = validate(&m).unwrap_err();
        assert!(e.contains("glob"), "应点名 glob，实得：{e}");
    }
    // 写清单同案（两个清单各是一条入口，不能只校验读那半）。
    let yaml = format!("{base}permissions:\n  fs:\n    write:\n      - \"../x\"\n");
    let m = serde_yaml_ng::from_str::<AgentManifest>(&yaml).unwrap();
    assert!(validate(&m).unwrap_err().contains("fs.write"));
}

/// `host_apis` 仍在线格式里（四字段之外、没有执行方），但**不得被静默吞掉**——
/// 包作者今天写着它。
#[test]
fn host_apis_survive_the_round_trip() {
    let m: AgentManifest =
        serde_yaml_ng::from_str("permissions:\n  host_apis: [\"vdfs/read\", \"model/chat\"]\n")
            .unwrap();
    assert_eq!(m.permissions.host_apis, vec!["vdfs/read", "model/chat"]);
    let back = serde_yaml_ng::to_string(&m).unwrap();
    assert!(back.contains("host_apis"), "{back}");
}
