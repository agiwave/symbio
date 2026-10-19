//! `providers/collectors/tool_visitor.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//! 这三条原在 `symbio_core/capability/tools.test.rs`，随默认收集器一起迁出 core。

use super::*;

use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::TurnOutput;
use crate::symbio_core::{
    capability_configurables, capability_model_active, capability_model_resolve, capability_mount,
    capability_mounts, capability_option_fields, capability_prompts, capability_vdfs_root,
    Capability, CapabilityMeta, CapabilityRow, CapabilityRowKind, DetailField, ExecEnv,
    ModelProvider, PluginError, VdfsContent, VdfsContext, VdfsError, VdfsProvider, VdfsRequest,
    VdfsResponse, VdfsResult,
};
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

// ==================== 模型族（AR-025 4b：一行一个 provider，生效位在行上） ====================
//
// 机制化之前这族是**单槽目录 + 批量注册**（`register_model_providers`）；4b 把它改成
// 与工具族同一张表：写是逐行 `put`，读是 `capability_model_*` 两个自由函数。
// 本文件钉住三件事：**生效位只有一个**（单槽）、**空集是一等输入**、**目录严格按 id**。

/// 模拟 model 插件的一次 traverse：逐行 put，当次生效者带生效位。
async fn register(mgr: &DefaultToolVisitor, active_id: Option<&str>, available: &[&'static str]) {
    for &id in available {
        mgr.put(CapabilityRow::model(provider(id), active_id == Some(id)))
            .await;
    }
}

#[tokio::test]
async fn model_rows_carry_provider_and_active_flag() {
    let mgr = DefaultToolVisitor::new();
    // 未注册 → 没有生效者，也没有目录
    assert!(capability_model_active(&mgr).await.is_none());

    register(&mgr, Some("p1"), &["p1", "p2"]).await;

    // 生效者 = 带位那一行
    assert_eq!(
        capability_model_active(&mgr).await.unwrap().provider_id(),
        "p1"
    );
    // 目录按 `provider_id` 索引（行名就是 provider_id），且顺序 = 注册序
    let ids: Vec<String> = mgr
        .list(CapabilityRowKind::Model)
        .await
        .into_iter()
        .map(|row| row.name().to_string())
        .collect();
    assert_eq!(ids, ["p1", "p2"]);
    // 模型行**不**混进工具面（`tools` 数组只由 `list(Tool)` 拼）
    assert!(mgr.list(CapabilityRowKind::Tool).await.is_empty());
}

/// **空集是一等输入**：一个可用的都没有时，两个取值口都给 `None`（不 panic、不补占位）
#[tokio::test]
async fn empty_registration_yields_no_provider() {
    let mgr = DefaultToolVisitor::new();
    register(&mgr, None, &[]).await;
    assert!(capability_model_active(&mgr).await.is_none());
    assert!(capability_model_resolve(&mgr, Some("p1")).await.is_none());
}

/// 生效者**必须是目录里的一员**：生效 id 不在目录里（未启用 / 协议工厂不可用）
/// ⇒ 没有任何行带位 ⇒ 没有生效者，**不**替调用方挑一个别的
///
/// 静默换模型比没有模型更难查——会话会用另一个模型跑完一整轮，而配置里那个坏条目
/// 永远不会被注意到。
#[tokio::test]
async fn active_absent_from_catalog_yields_none() {
    let mgr = DefaultToolVisitor::new();
    // 生效者 "missing" 不在这一批里：目录有 p1，但没有一行带生效位
    register(&mgr, Some("missing"), &["p1"]).await;
    assert!(capability_model_active(&mgr).await.is_none());
    // 目录本身照常可用（一个配坏的条目不该把其余可用的模型一起带走）
    assert_eq!(
        capability_model_resolve(&mgr, Some("p1"))
            .await
            .unwrap()
            .provider_id(),
        "p1"
    );
}

/// 目录查找**严格**（插件问的是"有没有这一个"）：不在目录里就落回生效者，
/// 而生效者不受查询影响
#[tokio::test]
async fn model_lookup_is_by_id_then_falls_back() {
    let mgr = DefaultToolVisitor::new();
    register(&mgr, Some("p1"), &["p1", "p2"]).await;

    assert_eq!(
        capability_model_resolve(&mgr, Some("p2"))
            .await
            .unwrap()
            .provider_id(),
        "p2"
    );
    // 写错的 id（含未注册的名字）：落回当次生效者，**降级而不失效**
    assert_eq!(
        capability_model_resolve(&mgr, Some("nope"))
            .await
            .unwrap()
            .provider_id(),
        "p1"
    );
    assert_eq!(
        capability_model_resolve(&mgr, None)
            .await
            .unwrap()
            .provider_id(),
        "p1"
    );
}

/// **单槽移交**：新生效行入表时，同族旧生效行降位——表里恒好只有一个生效位。
///
/// 旧行**不**从表里消失（降为普通目录行）：表的同名语义是覆盖，没有删除动作
/// （工具族同样如此），且目录归插件按 id 取用。
#[tokio::test]
async fn new_active_row_demotes_the_previous_one() {
    let mgr = DefaultToolVisitor::new();
    register(&mgr, Some("p1"), &["p1", "p2"]).await;
    register(&mgr, Some("p2"), &["p2"]).await;

    let actives: Vec<String> = mgr
        .list(CapabilityRowKind::Model)
        .await
        .into_iter()
        .filter(|row| row.active())
        .map(|row| row.name().to_string())
        .collect();
    assert_eq!(actives, ["p2"], "生效位只有一个：旧生效行必须降位");
    assert_eq!(
        capability_model_active(&mgr).await.unwrap().provider_id(),
        "p2"
    );
    // 降位那一行仍在表里（且已无位），插件按 id 照常取它
    let p1 = mgr.get("p1").await.expect("p1 仍在目录里");
    assert!(!p1.active());
    assert_eq!(
        capability_model_resolve(&mgr, Some("p1"))
            .await
            .unwrap()
            .provider_id(),
        "p1"
    );
}

/// 表键是**全表唯一的名字空间**（族不参与键）：跨族同名 = 后写者胜
///
/// 这条不是「正常覆盖」而是记账错误，故写入实现会告警；本用例钉住语义本身。
#[tokio::test]
async fn key_space_is_shared_across_kinds() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::tool(decl_tool("web/search"))).await;
    register(&mgr, Some("web/search"), &["web/search"]).await;

    assert_eq!(
        mgr.get("web/search").await.unwrap().kind(),
        CapabilityRowKind::Model,
        "同名的后写者胜（拿回的是模型行）"
    );
    assert!(mgr.list(CapabilityRowKind::Tool).await.is_empty());
}

/// 同名覆盖只改内容、**不改槽位**（顺序 = 首次注册顺序）
#[tokio::test]
async fn system_prompt_overwrite_keeps_first_registration_slot() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::prompt("persona", "默认提示词".to_string()))
        .await;
    mgr.put(CapabilityRow::prompt("work", "记忆".to_string()))
        .await;
    mgr.put(CapabilityRow::prompt(
        "persona",
        "默认提示词（覆盖）".to_string(),
    ))
    .await;
    mgr.put(CapabilityRow::prompt("agent", "人格".to_string()))
        .await;

    assert_eq!(
        capability_prompts(&mgr).await,
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
}

// ==================== 挂载族（AR-025 4d：挂载点与根同表，按根位区分） ====================

/// 可辨认的 VDFS 桧：`dispatch(Read)` 回自己的标签（身份 = 内容）。
struct StubMount(&'static str);

#[async_trait]
impl VdfsProvider for StubMount {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        _path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::Read => Ok(VdfsResponse::Read(VdfsContent::text(self.0))),
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

/// 读一份挂载实现的标签（`dispatch(Read)` 的回声就是它的身份）
async fn mount_label(provider: &Arc<dyn VdfsProvider>) -> String {
    provider
        .dispatch(&VdfsContext::empty(), "", VdfsRequest::Read)
        .await
        .expect("桩的 Read 必须成功")
        .into_read()
        .and_then(|c| c.text)
        .expect("桩的 Read 必须带正文")
}

/// 挂载点与根进**同一张表**：三个读函数是同一张表的三个视图——
/// 挂载点按名可查、根只按根位找（行名是保留值不对外），根**不在**挂载点清单里。
#[tokio::test]
async fn mount_family_is_three_views_of_one_table() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::mount("memory", Arc::new(StubMount("记忆"))))
        .await;
    mgr.put(CapabilityRow::mount_root(Arc::new(StubMount("系统根"))))
        .await;

    // 挂载点清单：只有挂载点，服务者单槽不冒充「一个挂载点」
    assert_eq!(
        capability_mounts(&mgr)
            .await
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        vec!["memory".to_string()]
    );
    // 按名查挂载点拿得到；根按根位找（内容 = 系统根那份）
    assert!(capability_mount(&mgr, "memory").await.is_some());
    let root = capability_vdfs_root(&mgr).await.expect("根已在表里");
    assert_eq!(mount_label(&root).await, "系统根");
}

/// 再登记一个根 = **单槽移交**（同一保留行名后写者胜，旧根不留痕）：
/// 清单不因二次登记长出尾巴，根位上的永远只有一份。
#[tokio::test]
async fn root_re_registration_is_a_single_slot_handover() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::mount_root(Arc::new(StubMount("旧根"))))
        .await;
    mgr.put(CapabilityRow::mount_root(Arc::new(StubMount("新根"))))
        .await;

    let root = capability_vdfs_root(&mgr).await.expect("根应在槽位里");
    assert_eq!(mount_label(&root).await, "新根");
    assert!(
        capability_mounts(&mgr).await.is_empty(),
        "退位的根不得留在挂载点清单里"
    );
}

// ==================== 选项 / 可配置两族（AR-025 4e：两个专服单族 trait 退场） ====================

// （原 2 个默认收集器的 4 条守门人测试逐字迁入本文件：行词汇合一后，去重 / 覆盖 /
// 排序都成了表与读协议的语义，被测对象从两个专用实现换成这个全表共用的唯一实现。）

fn field(key: &str, widget: &str) -> DetailField {
    DetailField {
        key: key.to_string(),
        label: key.to_string(),
        widget: widget.to_string(),
        ..Default::default()
    }
}

/// 原 `option_fields_sort_by_order_and_drop_it`（`option_visitor.test.rs`）：
/// `order` 只影响**读侧**排序，不出现在结果里（守门人：`capability_option_fields`）。
#[tokio::test]
async fn option_fields_sort_by_order_and_drop_it() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::option(60, field("heartbeat", "form")))
        .await;
    mgr.put(CapabilityRow::option(10, field("workdir", "path")))
        .await;
    mgr.put(CapabilityRow::option(40, field("risk_level", "select")))
        .await;

    let keys: Vec<String> = capability_option_fields(&mgr)
        .await
        .into_iter()
        .map(|f| f.key)
        .collect();
    assert_eq!(keys, vec!["workdir", "risk_level", "heartbeat"]);
}

/// 原 `option_fields_dedupe_by_key_keeping_first_slot`：同 key 覆盖（后写者胜），
/// 排序由读侧给；并钉一格新事实——选项行**不混进工具族**（两族同表互不可见）。
#[tokio::test]
async fn option_fields_dedupe_by_key_keeping_first_slot() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::option(20, field("agent_id", "select")))
        .await;
    mgr.put(CapabilityRow::option(10, field("workdir", "path")))
        .await;
    // 同 key 覆盖：字段换了，后写者胜（表语义：同名覆盖，槽位由 IndexMap 保留）
    mgr.put(CapabilityRow::option(99, field("agent_id", "text")))
        .await;

    let list = capability_option_fields(&mgr).await;
    let keys: Vec<&str> = list.iter().map(|f| f.key.as_str()).collect();
    assert_eq!(keys, vec!["workdir", "agent_id"]);
    assert_eq!(list[1].widget, "text", "后者覆盖前者");
    assert!(
        !mgr.list(CapabilityRowKind::Tool)
            .await
            .iter()
            .any(|row| row.name() == "workdir"),
        "选项行不混进工具族"
    );
}

/// 原 `register_keeps_order_and_overwrites_same_name`（`configurable_visitor.test.rs`）：
/// 按条目名（目录名）去重、后写者胜、列表保注册顺序。
#[tokio::test]
async fn configurable_declaration_keeps_order_and_overwrites_same_name() {
    use crate::symbio_core::{capability_entry_of, DetailDefinition, PluginConfigFile, PluginDir};
    fn config(dir_name: &str, title: &str) -> PluginConfigFile {
        PluginConfigFile::new(
            PluginDir::at(std::env::temp_dir(), dir_name),
            title,
            DetailDefinition::default(),
        )
    }

    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::configurable(capability_entry_of(&config(
        "web",
        "网络工具",
    ))))
    .await;
    mgr.put(CapabilityRow::configurable(capability_entry_of(&config(
        "session",
        "会话设置",
    ))))
    .await;
    mgr.put(CapabilityRow::configurable(capability_entry_of(&config(
        "web",
        "网络工具（改）",
    ))))
    .await;

    let list = capability_configurables(&mgr).await;
    assert_eq!(
        list.iter()
            .map(|it| it.node.name.as_str())
            .collect::<Vec<_>>(),
        vec!["web", "session"]
    );
    assert_eq!(list[0].node.title, "网络工具（改）");
}

/// 原 `empty_visitor_lists_nothing` 迁行：空表列不出东西；并钉两族互不可见。
#[tokio::test]
async fn empty_table_lists_nothing_for_both_families() {
    let mgr = DefaultToolVisitor::new();
    assert!(capability_configurables(&mgr).await.is_empty());
    assert!(capability_option_fields(&mgr).await.is_empty());
}
