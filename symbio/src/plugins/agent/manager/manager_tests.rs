//! AgentManager 单元测试
//!
//! 对应源文件: `manager.rs`

use super::*;

#[tokio::test]
async fn test_new_initializes_global_dir() {
    let mgr = AgentManager::new();
    let dir = mgr.global_dir();
    // ⭐ 统一来源：所有 agent 都存储在 ~/.symbio/plugins/agent/
    assert!(dir.ends_with(".symbio/plugins/agent"));
}

#[tokio::test]
async fn test_list_agents_empty() {
    let mgr = AgentManager::new();
    // 使用 None 作为 workdir，只扫描全局目录
    // 如果全局目录也为空，则返回空列表
    let agents = mgr.list_agents(None).await;
    // 这个测试验证 list_agents 不会 panic，且返回有效的结果
    // 全局目录可能存在默认 agent，所以不强制断言为空
    assert!(agents.iter().all(|a| !a.id.is_empty()));
}

#[tokio::test]
async fn test_get_agent_path_empty_id_rejected() {
    let mgr = AgentManager::new();
    let res = mgr.get_agent_path(None, "  ");
    assert!(res.is_err());
}

#[tokio::test]
async fn test_get_agent_returns_none_for_unknown() {
    let mgr = AgentManager::new();
    let p = mgr.get_agent(None, "no_such_agent_xyz").await;
    assert!(p.is_none());
}

#[tokio::test]
async fn test_initialize_marks_workdir() {
    let mgr = AgentManager::new();
    assert!(!mgr.is_initialized(Some("/tmp/p1")).await);
    mgr.mark_initialized(Some("/tmp/p1")).await;
    assert!(mgr.is_initialized(Some("/tmp/p1")).await);
}

#[tokio::test]
async fn test_cache_invalidation() {
    let mgr = AgentManager::new();
    // 写缓存
    let _ = mgr.list_agents(Some("/tmp/p2")).await;
    mgr.invalidate_cache_for_workdir(Some("/tmp/p2")).await;
    // 第二次仍可正常返回（这里不验证具体内容，仅验证不 panic）
    let _ = mgr.list_agents(Some("/tmp/p2")).await;
}

#[test]
fn test_default_works() {
    let _ = AgentManager::default();
}
