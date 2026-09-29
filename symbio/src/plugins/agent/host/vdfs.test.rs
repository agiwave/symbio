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
        parse_rel_path(AGENT_MEMORY_FILE),
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

// ==================== 概览载荷（「已装能力」的形状） ====================

/// 在临时目录里手搭一个 agent 目录：`<root>/<id>/{manifest.yaml, skill/a, mcp/b, …}`
///
/// 直接写盘（不走 zip 导入）：本组用例关心的是**目录内容 → 概览载荷**这一步，
/// 导入链有自己的测试。这样每个类别下能有确定数目的条目，断言才钉得住。
fn agent_fixture(root: &std::path::Path, id: &str, subdirs: &[(&str, usize)]) -> AgentDirRecord {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.yaml"),
        format!(
            "spec: \"agent-dir/v2\"\nid: \"{id}\"\nname: \"演示\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^2\"\n"
        ),
    )
    .unwrap();
    for (kind, n) in subdirs {
        for i in 0..*n {
            std::fs::create_dir_all(dir.join(format!("{kind}/item{i}"))).unwrap();
        }
    }
    AgentDirStore::load_record(&dir).expect("夹具应是合法 agent 目录")
}

/// **「已装能力」是结构化的**，不是一句在源头拼好的话。
///
/// 从前的形状是 `capabilities: "skill、mcp、model"`（`join("、")` 的产物）——
/// 那一句话把「哪一类有几个条目」的信息在源头就丢掉了，详情页再想分开摆也
/// 摆不出来。这条用例钉住三件事：类别清单有序、逐类条目数正确、计数与清单
/// 长度一致。
#[test]
fn dir_info_emits_structured_capabilities() {
    let tmp = tempfile::tempdir().unwrap();
    // 三类，条目数各不相同（避免「碰巧相等」让断言失去区分度）
    let record = agent_fixture(
        tmp.path(),
        "demo",
        &[("skill", 2), ("mcp", 1), ("model", 3)],
    );
    let store = AgentDirStore::new(tmp.path());
    let info = agent_dir_info(&record, &store);

    // ① 类别清单（有序；顺序 = 目录枚举顺序，后端不重排）
    let kinds: Vec<String> = info["capability_kinds"]
        .as_array()
        .expect("capability_kinds 应是数组")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(kinds.len(), 3, "三类能力：{kinds:?}");
    for k in ["skill", "mcp", "model"] {
        assert!(kinds.contains(&k.to_string()), "缺少类别 {k}：{kinds:?}");
    }

    // ② 逐类条目数（对象形状——前端 `staticDisplay` 逐项展开）
    let caps = info["capabilities"]
        .as_object()
        .expect("capabilities 应是对象（类别 → 条目数），不是拼接好的字符串");
    assert_eq!(caps["skill"], serde_json::json!(2));
    assert_eq!(caps["mcp"], serde_json::json!(1));
    assert_eq!(caps["model"], serde_json::json!(3));

    // ③ 计数与清单长度一致（同一份真相的两种呈现，不许各算各的）
    assert_eq!(info["capability_count"], serde_json::json!(3));
    assert_eq!(caps.len(), kinds.len());
}

/// 没有装配任何能力目录的 agent：能力为空，但**不崩、不撒谎**。
///
/// 三个字段都要给出可用值（`0` / `[]` / `{}`），而不是缺键——缺键会让详情页的
/// static 字段显示成 `—`，而这里「确实是 0 类」是个确切事实。
#[test]
fn dir_info_with_no_capabilities_is_empty_not_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let record = agent_fixture(tmp.path(), "bare", &[]);
    let store = AgentDirStore::new(tmp.path());
    let info = agent_dir_info(&record, &store);

    assert_eq!(info["capability_count"], serde_json::json!(0));
    assert_eq!(info["capability_kinds"], serde_json::json!([]));
    assert_eq!(info["capabilities"], serde_json::json!({}));
}

/// 类别名**不设白名单**：目录里挂了一个没人认识的名字，也必须如实出现。
///
/// agent 目录的子目录由「挂了哪些插件」决定（见 `agent/README.md`），不是一张
/// 固定表。写死类别表就会与真实来源形成两份真相——本用例用一个显然不在任何
/// 预设清单里的名字来钉这一点。
#[test]
fn dir_info_lists_unknown_categories_verbatim() {
    let tmp = tempfile::tempdir().unwrap();
    let record = agent_fixture(tmp.path(), "odd", &[("custom_widget", 1)]);
    let store = AgentDirStore::new(tmp.path());
    let info = agent_dir_info(&record, &store);

    assert_eq!(
        info["capability_kinds"],
        serde_json::json!(["custom_widget"])
    );
    assert_eq!(info["capabilities"]["custom_widget"], serde_json::json!(1));
}
