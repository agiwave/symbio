//! Principal 与 PrincipalKind 的判据钉死（[D-069 §决策]）。

use super::*;

/// 序列化形态钉死（wire 契约，前端逐字段镜像）：
/// `kind` snake_case、`id` 原样、`title` 缺省不出现。
#[test]
fn principal_wire_shape_is_three_fields() {
    let p = Principal::new(PrincipalKind::Tool, "vdfs/write").with_title("VDFS 写文件");
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(v["kind"], serde_json::json!("tool"));
    assert_eq!(v["id"], serde_json::json!("vdfs/write"));
    assert_eq!(v["title"], serde_json::json!("VDFS 写文件"));

    // title=None 不出现——skip_serializing_if = "Option::is_none"。
    let bare = Principal::new(PrincipalKind::Agent, "session/main");
    let v = serde_json::to_value(&bare).unwrap();
    assert_eq!(v["kind"], serde_json::json!("agent"));
    assert_eq!(v["id"], serde_json::json!("session/main"));
    assert!(v.get("title").is_none(), "title=None 时不上 wire");
}

/// `PrincipalKind::as_str` 与 serde 序列化**逐字一致**（[chat_node::NodeStatus::as_str] 同款纪律）。
#[test]
fn kind_words_match_serde_names() {
    for (kind, word) in [
        (PrincipalKind::Person, "person"),
        (PrincipalKind::Agent, "agent"),
        (PrincipalKind::Tool, "tool"),
        (PrincipalKind::System, "system"),
    ] {
        assert_eq!(kind.as_str(), word);
        let v = serde_json::to_value(kind).unwrap();
        assert_eq!(
            v,
            serde_json::json!(word),
            "{kind:?} 的 wire 词与 as_str 分叉"
        );
    }
}

/// `is_agent()` 只在 `Agent` 一族为真——这是 `visible_to` 与 `production_matrix` 的运行时判据。
#[test]
fn is_agent_covers_only_agent_kind() {
    assert!(PrincipalKind::Agent.is_agent());
    assert!(!PrincipalKind::Person.is_agent());
    assert!(!PrincipalKind::Tool.is_agent());
    assert!(!PrincipalKind::System.is_agent());
}

/// **旧形态一次性映射**（[D-069 §决策 6]）——历史 `messages.json` 里的 principal
/// 字符串读进协议时都从 `from_legacy` 走，读侧别名与 kind 归属**唯一入口**。
#[test]
fn from_legacy_maps_all_known_shapes() {
    // 新形态：`<ns>/<name>`——session 归 Agent，person 归 Person，vdfs/web/local 归 Tool，
    // system 归 System。
    let p = Principal::from_legacy("session/main");
    assert_eq!(p.kind, PrincipalKind::Agent);
    assert_eq!(p.id, "session/main");

    let p = Principal::from_legacy("person/local");
    assert_eq!(p.kind, PrincipalKind::Person);
    assert_eq!(p.id, "person/local");

    let p = Principal::from_legacy("vdfs/write");
    assert_eq!(p.kind, PrincipalKind::Tool);
    assert_eq!(p.id, "vdfs/write");

    let p = Principal::from_legacy("system/compactor");
    assert_eq!(p.kind, PrincipalKind::System);
    assert_eq!(p.id, "system/compactor");

    // 旧形态 `agent:xxx` → `{Agent, session/xxx}`（本机部署的 agent 都住在 session 下）。
    let p = Principal::from_legacy("agent:main");
    assert_eq!(p.kind, PrincipalKind::Agent);
    assert_eq!(p.id, "session/main");

    let p = Principal::from_legacy("agent:worker");
    assert_eq!(p.kind, PrincipalKind::Agent);
    assert_eq!(p.id, "session/worker");

    // 旧形态 `person:local` → `{Person, person:local}` 保留（namespace 段就是 person，
    // 只是分隔符不同——从 legacy 走时不重解析，原样存 id 也不影响判据）。
    let p = Principal::from_legacy("person:local");
    assert_eq!(p.kind, PrincipalKind::Person);
    assert_eq!(p.id, "person:local");

    // 旧形态 `tool:vdfs_write` → `{Tool, legacy/vdfs_write}`（未识别归属插件时的兜底 namespace）。
    let p = Principal::from_legacy("tool:vdfs_write");
    assert_eq!(p.kind, PrincipalKind::Tool);
    assert_eq!(p.id, "legacy/vdfs_write");

    // 裸字面量 `user` → Person + `person/local`（今天的会话属主）。
    let p = Principal::from_legacy("user");
    assert_eq!(p.kind, PrincipalKind::Person);
    assert_eq!(p.id, "person/local");

    // 完全裸的名字（无冒号无斜杠）——兜底当作 agent 挂 session 下；授权表按 id 精确判，
    // 未登记 ⇒ fail-closed，kind 猜宽不放大权限面。
    let p = Principal::from_legacy("reviewer");
    assert_eq!(p.kind, PrincipalKind::Agent);
    assert_eq!(p.id, "session/reviewer");
}

/// `from_legacy` **可幂等**——已经是新形态的 id 再喂一次得到同一 Principal（读侧
/// 别名对已迁移数据无害）。
#[test]
fn from_legacy_is_idempotent_on_new_shape() {
    for id in [
        "session/main",
        "person/local",
        "vdfs/write",
        "system/compactor",
        // `tool_principal` 合成的三个 namespace 都要读回 Tool 族
        "legacy/todo_write",
        "mcp/mockserv/echo",
        "agent/run",
    ] {
        let p1 = Principal::from_legacy(id);
        let p2 = Principal::from_legacy(&p1.id);
        assert_eq!(p1, p2, "from_legacy({id}) 应幂等");
    }
}

/// `tool_principal`：LLM 可见工具名（wire 形态）→ 可路由的 Tool 主体。
#[test]
fn tool_principal_maps_wire_tool_names() {
    // 首段是归属插件 ⇒ `ns/其余`，与 authz 登记的主体行逐一对齐
    let p = Principal::tool_principal("vdfs_write");
    assert_eq!(p.kind, PrincipalKind::Tool);
    assert_eq!(p.id, "vdfs/write");
    assert_eq!(Principal::tool_principal("web_fetch").id, "web/fetch");
    assert_eq!(
        Principal::tool_principal("local_content_search").id,
        "local/content_search"
    );
    assert_eq!(Principal::tool_principal("agent_run").id, "agent/run");

    // 限定名原样（含 MCP 的斜杠形态）
    assert_eq!(
        Principal::tool_principal("local/todo_write").id,
        "local/todo_write"
    );
    assert_eq!(
        Principal::tool_principal("mcp/github/create_issue").id,
        "mcp/github/create_issue"
    );

    // MCP 双下划线拼接名 ⇒ `mcp/server/tool`
    assert_eq!(
        Principal::tool_principal("mcp__mockserv__echo").id,
        "mcp/mockserv/echo"
    );

    // 只名历史工具 ⇒ legacy 兜底段，族不变
    assert_eq!(
        Principal::tool_principal("todo_write").id,
        "legacy/todo_write"
    );
    assert_eq!(
        Principal::tool_principal("context_compact").id,
        "legacy/context_compact"
    );
    assert_eq!(Principal::tool_principal("").id, "legacy/unknown");
    assert_eq!(Principal::tool_principal("  ").kind, PrincipalKind::Tool);
}
