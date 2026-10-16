//! `view` 域的单元测试——可见域判据（[plan/11 批 1](../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案") ③，
//! [ADR-069 §决策 3](../../../../docs/010-adr/069.md "ADR-069")）。
//!
//! 判据**只有一条**（[`visible_to`]），本文件把它钉死在三条规则上：
//! 任何一次「顺手多放行 / 多挡一条」都得先改这里的断言——规则的改动必须是
//! 一次显式的决定，而不是某个调用点顺手写下的第二个条件。

use super::*;

/// 三条规则各打一枪。id 形态是可路由地址 `<namespace>/<name>`。
#[test]
fn visible_to_hides_only_other_agents() {
    // 1. 自己的发言可见。
    assert!(visible_to("session/main", "session/main"));
    assert!(visible_to("session/reviewer", "session/reviewer"));

    // 2. 会话内的另一方（人）可见——`person/*` 不在 agent 命名空间登记表里，
    //    于是不是"第三方智能体"。
    assert!(visible_to("person/local", "session/main"));
    assert!(visible_to("person/local", "session/reviewer"));

    // 3. 别的智能体的发言不可见（双向——谁也不是默认的特权主体）。
    assert!(!visible_to("session/reviewer", "session/main"));
    assert!(!visible_to("session/main", "session/reviewer"));

    // 身份是**数据**：新来一个对等体，判据照样成立，不必改函数。
    assert!(!visible_to("session/peer-b", "session/main"));
    assert!(visible_to("session/peer-b", "session/peer-b"));
}

/// **命名空间登记表**是判据形状，不是主体清单：session 后面接什么都走同一条判定；
/// 加一个 namespace（例如 `decider`）就扩容一族智能体，无需改函数（[ADR-069 §决策 3]）。
#[test]
fn agent_namespaces_are_a_registry_not_a_prefix() {
    // 登记表今天的取值——加一条就多一族，不改判据。
    assert_eq!(AGENT_NAMESPACES, &["session"]);

    // session 命名空间下无论 name 段是什么形状（含空、含中文、含 `-`），都算 agent。
    for name in ["main", "sub-1", "带空格 的 id", "worker"] {
        let id = format!("session/{name}");
        assert!(
            is_agent_id(&id),
            "`{id}` 在 session 命名空间下 ⇒ agent 一族"
        );
        assert!(visible_to(&id, &id), "{id} 自己必然可见");
        assert!(
            !visible_to(&id, "session/main") || id == "session/main",
            "别人的主体 {id} 不该进 session/main 的视图"
        );
    }

    // 非 agent 命名空间——person / vdfs / web / system 都不在登记表里 ⇒ 不是 agent。
    for id in [
        "person/local",
        "vdfs/write",
        "web/fetch",
        "local/content_search",
        "system/compactor",
    ] {
        assert!(!is_agent_id(id), "`{id}` 不在 agent 命名空间登记表里");
    }

    // 未带 namespace 的裸 id 与空串：`principal_namespace` 返回 None，`is_agent_id` 亦 false。
    assert_eq!(principal_namespace("user"), None);
    assert_eq!(principal_namespace(""), None);
    assert!(!is_agent_id("user")); // 裸 id，无 `/`
    assert!(!is_agent_id("Session/main")); // 大小写敏感
    assert!(!is_agent_id("decider/reflex")); // 未登记 namespace（将来 reflex 拆到独立插件时，登记表加一条 "decider"）
}

/// **可路由地址的归属段解析**：`<ns>/<name>` 或更长的路径都能取到第一段——
/// 与 VDFS 地址、route 常量同族形状；这是 [ADR-069 §决策 2] 的"地址"含义。
#[test]
fn principal_namespace_is_the_first_route_segment() {
    assert_eq!(principal_namespace("session/main"), Some("session"));
    assert_eq!(principal_namespace("vdfs/write"), Some("vdfs"));
    assert_eq!(principal_namespace("person/local"), Some("person"));
    // 多段路径也取首段——将来 `<ns>/<sub>/<name>` 的形态不破坏判据。
    assert_eq!(principal_namespace("session/s42/worker"), Some("session"));
    // 边界：首段空（`/foo`）→ 返回 Some("")，登记表不匹配 ⇒ is_agent_id false；
    // 这不是智能体，也不该是——地址必须以归属段起头才能路由。
    assert_eq!(principal_namespace("/foo"), Some(""));
    assert!(!is_agent_id("/foo"));
}

/// `visible_to` 的「非 agent 一律可见」这一半与 `is_agent_id` 判据**一一对应**：
/// person / tool / system / 未识别 namespace 都在会话线程里可见。
#[test]
fn visible_to_treats_non_agent_namespaces_as_peers() {
    for p in [
        "person/local",
        "vdfs/write",
        "system/compactor",
        "unregistered/foo",
        "legacy-bare-id",
    ] {
        assert!(
            visible_to(p, "session/main"),
            "`{p}` 不在 agent 命名空间登记表，会话线程里可见（visible_to 规则 2）"
        );
    }
}
