//! `view` 域的单元测试——可见域判据（[plan/11 批 1](../../../../docs/plan/11-多执行器与多主体加固实施方案.md) ③）。
//!
//! 判据**只有一条**（[`visible_to`]），本文件把它钉死在三条规则上：
//! 任何一次「顺手多放行 / 多挡一条」都得先改这里的断言——规则的改动必须是
//! 一次显式的决定，而不是某个调用点顺手写下的第二个条件。

use super::*;

/// 三条规则各打一枪。
#[test]
fn visible_to_hides_only_other_agents() {
    // 1. 自己的发言可见。
    assert!(visible_to("agent:main", "agent:main"));
    assert!(visible_to("agent:reviewer", "agent:reviewer"));

    // 2. 会话内的另一方（人）可见——`session/principal` 的平凡值，前缀判定的
    //    负半轴：不是 `agent:` 就不是第三方智能体。
    assert!(visible_to("user", "agent:main"));
    assert!(visible_to("user", "agent:reviewer"));

    // 3. 别的智能体的发言不可见（双向——谁也不是默认的特权主体）。
    assert!(!visible_to("agent:reviewer", "agent:main"));
    assert!(!visible_to("agent:main", "agent:reviewer"));

    // 身份是**数据**：新来一个对等体，判据照样成立，不必改函数。
    assert!(!visible_to("agent:peer-b", "agent:main"));
    assert!(visible_to("agent:peer-b", "agent:peer-b"));
}

/// 前缀是**形状**不是清单：`agent:` 后面接什么都走同一条判定。
#[test]
fn agent_prefix_is_a_shape_not_a_roster() {
    assert_eq!(AGENT_PREFIX, "agent:");
    assert!(!"agent:".starts_with("agent:x"));
    // 主体 id 带空格 / 中文 / 长 id —— 只要比对前缀，形状不变。
    for id in ["", "main", "sub-1", "带空格 的 id"] {
        let actor = format!("{AGENT_PREFIX}{id}");
        assert!(visible_to(&actor, &actor), "{actor} 自己必然可见");
        assert!(
            !visible_to(&actor, "agent:main") || actor == "agent:main",
            "别人的主体 {actor} 不该进 agent:main 的视图"
        );
    }
}
