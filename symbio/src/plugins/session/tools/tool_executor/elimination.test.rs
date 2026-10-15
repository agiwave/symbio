//! 淘汰闸门判据（S13 §3，T-073）——部署级覆盖层的批首读。
//!
//! 三条判据各钉一边：**跨会话生效**（换个会话绕不开淘汰）、**恢复可逆**、
//! **按主体精确命中**（别人的淘汰不殃及本主体）。

use super::EliminationInputs;
use crate::symbio_core::{
    EventWalStore, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest, Store,
};

/// 带 `AGENT_ID` 的请求上下文：闸门按它派生主体（同 `BreakerInputs` 的取值口径）。
fn agent_ctx(agent_id: &str) -> std::sync::Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(crate::symbio_core::AGENT_ID, agent_id.to_string());
    std::sync::Arc::new(req)
}

/// 在 `<root>/<sid>/v2-events.wal` 里落一条事件（生产布局：存储根 = 插件目录）。
fn append_to_session(root: &std::path::Path, sid: &str, event: crate::symbio_core::Event) {
    let dir = crate::plugins::session::paths::session_dir(root, sid);
    std::fs::create_dir_all(&dir).expect("建会话目录");
    let store = EventWalStore::open(dir.join(crate::plugins::session::paths::V2_WAL_FILE))
        .expect("打开 WAL");
    store.append(event).expect("入格");
}

fn tmp_root(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("symbio-elim-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建插件目录");
    dir
}

/// 部署级：裁决落在**另一个会话**的 WAL 里，本会话的 `agent_run` 照样被拒。
///
/// 「换个会话就能绕开淘汰」是本闸门要消灭的静默面——按会话扫覆盖层就会得到它。
#[test]
fn a_verdict_in_another_session_revokes_delegation_here() {
    let root = tmp_root("cross-session");
    append_to_session(
        &root,
        "sess-a",
        crate::symbio_core::EliminationDecider.elimination_event(
            crate::symbio_core::authz::PRINCIPAL_MAIN,
            -1,
            7,
        ),
    );
    // 本会话（sess-b）自己的 WAL 里什么都没有。
    std::fs::create_dir_all(crate::plugins::session::paths::session_dir(&root, "sess-b")).ok();

    let ctx = agent_ctx("main");
    let inputs = EliminationInputs::of(&root, &ctx);
    assert_eq!(inputs.principal, crate::symbio_core::authz::PRINCIPAL_MAIN);
    assert!(
        inputs.delegation_revoked,
        "别处的淘汰裁决必须在本会话生效（部署级口径）"
    );

    std::fs::remove_dir_all(&root).ok();
}

/// 没有裁决 ⇒ 不拒（覆盖层只答减法，表是权威——空覆盖层 = 谁都没被收回）。
#[test]
fn no_verdict_means_no_revocation() {
    let root = tmp_root("empty");
    let inputs = EliminationInputs::of(&root, &agent_ctx("main"));
    assert!(!inputs.delegation_revoked);
    std::fs::remove_dir_all(&root).ok();
}

/// 恢复可逆：revoke + restore（同格两条，last-wins）⇒ 闸门放行。
///
/// 「可申诉」若不是机制而是文档话，这条用例就红。
#[test]
fn restoration_flips_the_gate_back() {
    let root = tmp_root("restore");
    let d = crate::symbio_core::EliminationDecider;
    append_to_session(&root, "sess-a", d.elimination_event("agent:main", -1, 7));
    append_to_session(
        &root,
        "sess-a",
        d.restoration_event("agent:main", "申诉受理：上游延迟有凭证", 9),
    );
    let inputs = EliminationInputs::of(&root, &agent_ctx("main"));
    assert!(
        !inputs.delegation_revoked,
        "恢复之后重新可委派（last-wins 折叠）"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// 按主体精确命中：淘汰的是 `agent:ghost`，本主体（main）不受殃及。
#[test]
fn another_principals_verdict_does_not_bite() {
    let root = tmp_root("other");
    append_to_session(
        &root,
        "sess-a",
        crate::symbio_core::EliminationDecider.elimination_event("agent:ghost", -1, 7),
    );
    let inputs = EliminationInputs::of(&root, &agent_ctx("main"));
    assert!(!inputs.delegation_revoked);
    // 而被淘汰的那一个自己来：拒。
    let ghost = EliminationInputs::of(&root, &agent_ctx("ghost"));
    assert!(ghost.delegation_revoked, "agent:ghost 自己被收回");
    std::fs::remove_dir_all(&root).ok();
}

/// 存储根不存在（全新部署，还没建过任何会话）⇒ 空覆盖层，不拒也不炸。
#[test]
fn a_missing_storage_root_is_an_empty_overlay() {
    let root = std::env::temp_dir().join(format!("symbio-elim-nope-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let inputs = EliminationInputs::of(&root, &agent_ctx("main"));
    assert!(!inputs.delegation_revoked);
}

/// 在 `<root>/<safe(父)>/sessions/<safe(子)>` 里落一条事件（嵌套子会话布局：
/// 子会话住在父目录的 `sessions/` 里，见 `store` 模块头）。
fn append_to_sub_session(
    root: &std::path::Path,
    parent_sid: &str,
    child_sid: &str,
    event: crate::symbio_core::Event,
) {
    let dir = root
        .join(parent_sid)
        .join(crate::plugins::session::paths::SUB_SESSIONS_SUBDIR)
        .join(child_sid);
    std::fs::create_dir_all(&dir).expect("建子会话目录");
    let store = EventWalStore::open(dir.join(crate::plugins::session::paths::V2_WAL_FILE))
        .expect("打开子会话 WAL");
    store.append(event).expect("入格");
}

/// **嵌套子会话**里的裁决在本会话生效（T-107 批发现并修的静默面）。
///
/// 子会话不是顶层平级目录，只扫一层的覆盖层整棵子树都看不见：一个**未绑智能体**
/// 的子会话（`agent_run` 省略 `agent_id` ⇒ 主体 = `agent:main`，持 `assign.work`）
/// 违约 ⇒ 裁决落在那棵子树的 WAL 里 ⇒ 父会话的闸扫不到它 ⇒「换个**子会话**就能
/// 绕开淘汰」——与这道闸要消灭的「换个会话」同形。本用例把这条路钉死。
#[test]
fn a_verdict_in_a_nested_sub_session_revokes_delegation_here() {
    let root = tmp_root("nested");
    append_to_sub_session(
        &root,
        "sess-parent",
        "sess-child",
        crate::symbio_core::EliminationDecider.elimination_event(
            crate::symbio_core::authz::PRINCIPAL_MAIN,
            -1,
            7,
        ),
    );

    let inputs = EliminationInputs::of(&root, &agent_ctx("main"));
    assert!(
        inputs.delegation_revoked,
        "子会话 WAL 里的裁决必须在本会话生效（部署级 = 全会话，含嵌套）"
    );

    std::fs::remove_dir_all(&root).ok();
}
