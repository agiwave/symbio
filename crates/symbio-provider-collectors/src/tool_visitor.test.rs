//! `providers/collectors/tool_visitor.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//! 这三条原在旧 core 的 `capability/tools` 用例，随默认收集器一起迁出。

use super::*;

use symbio_kernel::capability::{family, CapabilityMeta, CapabilityRow};
use symbio_kernel::present::DetailField;
use symbio_kernel::vdfs::{
    VdfsContent, VdfsContext, VdfsError, VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult,
};
use symbio_sdk::capability::{
    capability_configurables, capability_option_fields, capability_prompts, capability_vdfs_root,
};

// ==================== 模型族（DE-025 §3：一行一个目录项，生效位在行上） ====================
//
// 机制化之前这族是**单槽目录 + 批量注册**（`register_model_providers`）；4b 把它改成
// 与工具族同一张表：写是逐行 `put`。读口（`capability_model_*`）随 `D-437` 退场——
// 它自建立起没有生产调用方，而行的读者本来只需要「目录里有谁、谁是生效者」。
// 本文件钉住的仍是**表语义本身**：生效位只有一个（单槽）、空集是一等输入、行是数据。

/// 模拟 model 插件的一次 traverse：逐行 put，当次生效者带生效位。
async fn register(mgr: &DefaultToolVisitor, active_id: Option<&str>, available: &[&'static str]) {
    for &id in available {
        mgr.put(CapabilityRow::model(id, active_id == Some(id)))
            .await;
    }
}

#[tokio::test]
async fn model_rows_carry_id_protocol_and_one_slot() {
    let mgr = DefaultToolVisitor::new();
    // 未注册 → 目录为空，也没有生效者
    assert!(mgr.list(family::MODEL).await.is_empty());

    register(&mgr, Some("p1"), &["p1", "p2"]).await;

    // 目录按 `provider_id` 索引（行名就是 provider_id），且顺序 = 注册序
    let ids: Vec<String> = mgr
        .list(family::MODEL)
        .await
        .into_iter()
        .map(|row| row.name().to_string())
        .collect();
    assert_eq!(ids, ["p1", "p2"]);
    // 生效者 = 带位那一行；行里不装执行对象（`D-437`）
    let actives: Vec<String> = mgr
        .list(family::MODEL)
        .await
        .into_iter()
        .filter(|row| row.active())
        .map(|row| row.name().to_string())
        .collect();
    assert_eq!(actives, ["p1"], "生效位只有一个");
    assert_eq!(
        mgr.get("p1").await.expect("p1 在目录里").family(),
        family::MODEL
    );
    // 模型行**不**混进工具面（`tools` 数组只由 `list(Tool)` 拼）
    assert!(mgr.list(family::TOOL).await.is_empty());
}

/// **空集是一等输入**：一个可用的都没有时，目录为空且没有生效位（不 panic、不补占位）
#[tokio::test]
async fn empty_registration_yields_no_rows() {
    let mgr = DefaultToolVisitor::new();
    register(&mgr, None, &[]).await;
    assert!(mgr.list(family::MODEL).await.is_empty());
}

/// 生效者**必须是目录里的一员**：生效 id 不在目录里（未启用 / 协议工厂不可用）
/// ⇒ 没有任何行带位 ⇒ 没有生效者，**不**替调用方挑一个别的
///
/// 静默换模型比没有模型更难查——会话会用另一个模型跑完一整轮，而配置里那个坏条目
/// 永远不会被注意到。目录本身照常可用（一个配坏的条目不该把其余可用的模型一起带走）。
#[tokio::test]
async fn active_absent_from_catalog_leaves_no_slot() {
    let mgr = DefaultToolVisitor::new();
    register(&mgr, Some("missing"), &["p1"]).await;
    let actives: Vec<String> = mgr
        .list(family::MODEL)
        .await
        .into_iter()
        .filter(|row| row.active())
        .map(|row| row.name().to_string())
        .collect();
    assert!(actives.is_empty(), "目录里的 p1 不带位");
    assert!(mgr.get("p1").await.is_some(), "目录本身照常可用");
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
        .list(family::MODEL)
        .await
        .into_iter()
        .filter(|row| row.active())
        .map(|row| row.name().to_string())
        .collect();
    assert_eq!(actives, ["p2"], "生效位只有一个：旧生效行必须降位");
    // 降位那一行仍在表里（且已无位），插件按 id 照常取它
    let p1 = mgr.get("p1").await.expect("p1 仍在目录里");
    assert!(!p1.active());
}

/// 表键是**全表唯一的名字空间**（族不参与键）：跨族同名 = 后写者胜
///
/// 这条不是「正常覆盖」而是记账错误，故写入实现会告警；本用例钉住语义本身。
#[tokio::test]
async fn key_space_is_shared_across_kinds() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::tool(decl_meta("web/search"))).await;
    register(&mgr, Some("web/search"), &["web/search"]).await;

    assert_eq!(
        mgr.get("web/search").await.unwrap().family(),
        family::MODEL,
        "同名的后写者胜（拿回的是模型行）"
    );
    assert!(mgr.list(family::TOOL).await.is_empty());
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

// ==================== 工具行表（DE-025 §3：put / get / list） ====================
//
// 机制化之前这五条用例不存在——`register` / `invoke` / `list_capability` 的语义
// 只由插件侧的使用方式间接兜着。表的三方法落位后，把语义钉在本文件：
// **顺序 / 覆盖 / 按名取 / 两种口径逐条相等**。

/// 只带元数据的最小桩：工具行是纯数据（`D-440` ③），注册面只读 `meta()`。
fn decl_meta(name: &str) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        description: format!("{name} 的说明"),
        input_schema: serde_json::json!({"type": "object"}),
        ..Default::default()
    }
}

/// **守门人**（DE-025 §3）：同一份工具清单以两种口径导出，逐条相等——
/// 走表的 `list(Tool)` 与「入表时给的元数据」全量转储。顺序也逐条一致
/// （注册序即表序，D-057 的工具面有序性就架在这条上）。
#[tokio::test]
async fn tool_rows_match_the_meta_dump_entry_by_entry() {
    let mgr = DefaultToolVisitor::new();
    let names = ["local/shell", "local/read", "web/search"];
    for n in names {
        mgr.put(CapabilityRow::tool(decl_meta(n))).await;
    }

    let listed: Vec<String> = mgr
        .list(family::TOOL)
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
        assert_eq!(row.family(), family::TOOL);
    }
}

/// 同名覆盖只改内容、**不改槽位**（与系统提示词 / 挂载点同一 IndexMap 语义——
/// 机制化之前 `register` 的行为逐字保持）。
#[tokio::test]
async fn tool_put_overwrite_keeps_first_registration_slot() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::tool(decl_meta("local/shell"))).await;
    mgr.put(CapabilityRow::tool(decl_meta("local/read"))).await;
    mgr.put(CapabilityRow::tool(decl_meta("local/shell"))).await;
    mgr.put(CapabilityRow::tool(decl_meta("web/search"))).await;

    let listed: Vec<String> = mgr
        .list(family::TOOL)
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
    mgr.put(CapabilityRow::tool(decl_meta("local/shell"))).await;
    assert!(mgr.get("local/shel").await.is_none());
    assert!(mgr.get("").await.is_none());
}

/// 非工具族本批无行：`list` 按族过滤，别的族落空表（4b–4e 各族落位后此格
/// 自然转为「只返回该族」，签名不变）。
#[tokio::test]
async fn tool_list_of_another_kind_is_empty() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::tool(decl_meta("local/shell"))).await;
    assert!(mgr.list(family::MODEL).await.is_empty());
}

// ==================== 挂载族（这一族今天只有根一形） ====================

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

/// 挂载族只剩**根一形**：`mount` 与按名查挂载点的读口都已删，`mount_root` 是唯一入口。
///
/// 判据是「没有第二个入口」而不是「入口少了一个」：除了根，往里塞别的东西根本
/// 写不出来——所以这里连「挂载点清单为空」这样的断言都不需要，那个读口不存在。
#[tokio::test]
async fn root_is_the_only_mount_shape() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::mount_root(Arc::new(StubMount("系统根"))))
        .await;

    let root = capability_vdfs_root(&mgr).await.expect("根已在表里");
    assert_eq!(mount_label(&root).await, "系统根");
}

/// 再登记一个根 = **单槽移交**（同一保留行名后写者胜，旧根不留痕）。
#[tokio::test]
async fn root_re_registration_is_a_single_slot_handover() {
    let mgr = DefaultToolVisitor::new();
    mgr.put(CapabilityRow::mount_root(Arc::new(StubMount("旧根"))))
        .await;
    mgr.put(CapabilityRow::mount_root(Arc::new(StubMount("新根"))))
        .await;

    let root = capability_vdfs_root(&mgr).await.expect("根应在槽位里");
    assert_eq!(mount_label(&root).await, "新根");
}

// ==================== 选项 / 可配置两族（DE-025 §3：两个专服单族 trait 退场） ====================

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
        !mgr.list(family::TOOL)
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
    use symbio_kernel::present::DetailDefinition;
    use symbio_sdk::capability::capability_entry_of;
    use symbio_sdk::plugin::{PluginConfigFile, PluginDir};
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
