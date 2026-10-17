//! `providers/collectors/tool_visitor.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//! 这三条原在 `symbio_core/capability/tools.test.rs`，随默认收集器一起迁出 core。

use super::*;

use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::TurnOutput;
use crate::symbio_core::{Capability, CapabilityMeta, ExecEnv, PluginError};
use async_trait::async_trait;

/// 最小模型服务桩：实现纯 trait 契约，仅用于验证注册存储语义，不发起真实请求
struct MockProvider {
    tag: String,
}

#[async_trait]
impl ModelProvider for MockProvider {
    fn provider_id(&self) -> &str {
        &self.tag
    }

    fn api_protocol(&self) -> &str {
        "openai_chat"
    }

    fn rate_limit_ms(&self) -> u64 {
        0
    }

    fn max_context_tokens(&self) -> u32 {
        262_144
    }

    async fn effective_context_tokens(&self) -> u32 {
        262_144
    }

    async fn execute_turn(
        &self,
        _system_prompt: &str,
        _messages: &[ChatNode],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        _env: &ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        Err(PluginError::InternalError("mock".to_string()))
    }
}

fn provider(id: &str) -> Arc<dyn ModelProvider> {
    Arc::new(MockProvider {
        tag: id.to_string(),
    })
}

#[tokio::test]
async fn model_slot_set_get_and_missing() {
    let mgr = DefaultToolVisitor::new();
    // 未注册 → 生效者与目录都是空
    assert!(mgr.get_model_provider().await.is_none());
    assert!(mgr.get_model_provider_by_id("p1").await.is_none());

    // 注册后可取回，身份字段一致
    mgr.register_model_providers(Some("p1"), vec![provider("p1")])
        .await;
    let got = mgr.get_model_provider().await;
    let got = got.expect("注册后应可取回");
    assert_eq!(got.provider_id(), "p1");
}

/// **空集是一等输入**：一个可用的都没有时，两个取值口都给 `None`（不 panic、不补占位）
#[tokio::test]
async fn empty_registration_yields_no_provider() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_model_providers(None, vec![]).await;
    assert!(mgr.get_model_provider().await.is_none());
    assert!(mgr.get_model_provider_by_id("p1").await.is_none());
}

/// 生效者**从目录里取**：`active_id` 在目录里查不到 ⇒ `None`，**不**替调用方挑一个别的
///
/// 静默换模型比没有模型更难查——会话会用另一个模型跑完一整轮，而配置里那个坏条目
/// 永远不会被注意到。
#[tokio::test]
async fn active_id_absent_from_catalog_yields_none() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_model_providers(Some("missing"), vec![provider("p1")])
        .await;
    assert!(mgr.get_model_provider().await.is_none());
    // 目录本身照常可用（一个配坏的条目不该把其余可用的模型一起带走）
    assert_eq!(
        mgr.get_model_provider_by_id("p1")
            .await
            .unwrap()
            .provider_id(),
        "p1"
    );
}

/// 目录按 `provider_id` 索引，且是**严格查找**：不在目录里就是 `None`，
/// 不降级到生效者（插件问的是"有没有这一个"）
#[tokio::test]
async fn catalog_lookup_is_by_id_and_strict() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_model_providers(Some("p1"), vec![provider("p1"), provider("p2")])
        .await;

    assert_eq!(
        mgr.get_model_provider_by_id("p2")
            .await
            .unwrap()
            .provider_id(),
        "p2"
    );
    assert!(mgr.get_model_provider_by_id("nope").await.is_none());
    // 生效者仍是 p1（目录不影响它）
    assert_eq!(mgr.get_model_provider().await.unwrap().provider_id(), "p1");
}

/// 重新注册**整体替换**目录（不是累积）：provider 被停用 / 删除后，
/// 旧实例不该留在目录里被插件取到
#[tokio::test]
async fn reregistration_replaces_the_catalog() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_model_providers(Some("p1"), vec![provider("p1"), provider("p2")])
        .await;
    mgr.register_model_providers(Some("p2"), vec![provider("p2")])
        .await;

    assert!(mgr.get_model_provider_by_id("p1").await.is_none());
    assert_eq!(
        mgr.get_model_provider_by_id("p2")
            .await
            .unwrap()
            .provider_id(),
        "p2"
    );
    assert_eq!(mgr.get_model_provider().await.unwrap().provider_id(), "p2");
}

/// 同名覆盖只改内容、**不改槽位**（顺序 = 首次注册顺序）
#[tokio::test]
async fn system_prompt_overwrite_keeps_first_registration_slot() {
    let mgr = DefaultToolVisitor::new();
    mgr.register_system_prompt("persona", "默认提示词".to_string())
        .await;
    mgr.register_system_prompt("work", "记忆".to_string()).await;
    mgr.register_system_prompt("persona", "默认提示词（覆盖）".to_string())
        .await;
    mgr.register_system_prompt("agent", "人格".to_string())
        .await;

    assert_eq!(
        mgr.list_system_prompts().await,
        vec![
            ("persona".to_string(), "默认提示词（覆盖）".to_string()),
            ("work".to_string(), "记忆".to_string()),
            ("agent".to_string(), "人格".to_string()),
        ]
    );
}

// ==================== 工具行表（AR-025 4a：put / get / list） ====================
//
// 机制化之前这五条用例不存在——`register` / `invoke` / `list_capability` 的语义
// 只由插件侧的使用方式间接兜着。表的三方法落位后，把语义钉在本文件：
// **顺序 / 覆盖 / 按名取 / 两种口径逐条相等**。

/// 只带元数据的最小能力桩：注册面只读 `meta()`，执行永不到达。
struct DeclTool(CapabilityMeta);

#[async_trait]
impl Capability for DeclTool {
    fn meta(&self) -> CapabilityMeta {
        self.0.clone()
    }

    fn name(&self) -> String {
        self.0.name.clone()
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn crate::symbio_core::PluginInvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        Ok(serde_json::json!({"ok": true}))
    }
}

fn decl_tool(name: &str) -> Arc<dyn Capability> {
    Arc::new(DeclTool(CapabilityMeta {
        name: name.to_string(),
        description: format!("{name} 的说明"),
        input_schema: serde_json::json!({"type": "object"}),
        ..Default::default()
    }))
}

/// **守门人**（AR-025 §5）：同一份工具清单以两种口径导出，逐条相等——
/// 走表的 `list(Tool)` 与「入表时给的元数据」全量转储。顺序也逐条一致
/// （注册序即表序，ADR-057 的工具面有序性就架在这条上）。
#[tokio::test]
async fn tool_rows_match_the_meta_dump_entry_by_entry() {
    let mgr = DefaultToolVisitor::new();
    let names = ["local/shell", "local/read", "web/search"];
    for n in names {
        mgr.put(CapabilityRow::tool(decl_tool(n))).await;
    }

    let listed: Vec<String> = mgr
        .list(CapabilityRowKind::Tool)
        .await
        .into_iter()
        .map(|row| row.meta().name)
        .collect();
    assert_eq!(listed, names, "两种口径逐条相等，且顺序 = 注册顺序");

    // 反方向：按名取行，取到的就是入表那一个（含闸门收口点的壳）
    for n in names {
        let row = mgr
            .get(n)
            .await
            .unwrap_or_else(|| panic!("{n} 应可按名取到"));
        assert_eq!(row.name(), n);
        assert_eq!(row.kind(), CapabilityRowKind::Tool);
    }
}

/// 同名覆盖只改内容、**不改槽位**（与系统提示词 / 挂载点同一 IndexMap 语义——
/// 机制化之前 `register` 的行为逐字保持）。
#[tokio::test]
async fn tool_put_overwrite_keeps_first_registration_slot() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::tool(decl_tool("local/shell"))).await;
    mgr.put(CapabilityRow::tool(decl_tool("local/read"))).await;
    mgr.put(CapabilityRow::tool(decl_tool("local/shell"))).await;
    mgr.put(CapabilityRow::tool(decl_tool("web/search"))).await;

    let listed: Vec<String> = mgr
        .list(CapabilityRowKind::Tool)
        .await
        .into_iter()
        .map(|row| row.name().to_string())
        .collect();
    assert_eq!(listed, ["local/shell", "local/read", "web/search"]);
}

/// 按名取不到 = `None`，不猜、不回落（`invoke` 收口成「取行 + 执行」两步后，
/// NotFound 由调用方给——判据见 `tool_executor` 的分派点）。
#[tokio::test]
async fn tool_get_missing_name_is_none() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::tool(decl_tool("local/shell"))).await;
    assert!(mgr.get("local/shel").await.is_none());
    assert!(mgr.get("").await.is_none());
}

/// 非工具族本批无行：`list` 按族过滤，别的族落空表（4b–4e 各族落位后此格
/// 自然转为「只返回该族」，签名不变）。
#[tokio::test]
async fn tool_list_of_another_kind_is_empty() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::tool(decl_tool("local/shell"))).await;
    assert!(mgr.list(CapabilityRowKind::Model).await.is_empty());
    assert!(mgr.list(CapabilityRowKind::Policy).await.is_empty());
}
