//! host 层端到端测试：bundle 导入 → traverse 装配 → 身份提示词 → 导出 → 删除。
//!
//! 覆盖规范 §3（约定目录）/ §5（能力单元）/ §6（装配语义）/ §10（版本接入门槛）
//! 的主链路，全部进程内完成（tempdir + 内存 zip），不依赖真实文件系统布局。
//!
//! bundle 采用**约定目录布局**（约定优于配置）：`prompts/`、`skills/`、
//! `mcps/`，存在即安装，无任何额外配置文件。工具只经 `mcps/`（MCP）接入，
//! 协议不含 `oab.echo` 之类的宿主专有执行器。

use super::plugin::AgentPlugin;
use super::store::BundleStore;
use crate::symbio_core::{
    CapabilityVisitor, DefaultToolVisitor, InvokeRequest, InvokeRequestExt, Plugin, SimpleRequest,
    AGENT_ID, CAPABILITY_VISITOR, PATH, TRAVERSE_AVAILABLE_TOOLS, WORKDIR,
};
use std::path::Path;
use std::sync::Arc;

/// 在内存中生成一个约定目录布局的 bundle zip。
///
/// 三个能力来源：
/// - `prompts/persona.md`：OAB 原生系统提示词片段（frontmatter `priority: 0` 身份锚定）
/// - `skills/playbook/SKILL.md`：行业 SKILL 格式（正文作为提示词片段）
/// - `mcps/demo.yaml`：行业 MCP server 配置（工具唯一来源）
fn build_bundle_zip(bundle_id: &str, requires_spec: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        let manifest = format!(
            r#"spec: "oab/v1"
id: "{bundle_id}"
name: "全栈开发智能体"
version: "1.0.0"
requires:
  spec: "{requires_spec}"
permissions:
  host_apis:
    - "host/log"
"#
        );
        // prompts/persona.md：frontmatter 携带 priority（装配时剥离），支持 $bundle.* 变量
        let persona_md = "---\npriority: 0\n---\n你是 $bundle.id 的全栈开发人格。";
        // skills/playbook/SKILL.md：行业 SKILL 格式，正文作为提示词片段
        let skill_md = "---\nname: delivery\n---\n先澄清需求再动工。";
        // mcps/demo.yaml：行业 MCP server 配置（原样透传给宿主 MCP 客户端）
        let mcp_yaml = "command: npx\nargs: [\"-y\", \"@modelcontextprotocol/server-demo\"]\n";

        let files: Vec<(&str, &str)> = vec![
            ("manifest.yaml", &manifest),
            ("prompts/persona.md", persona_md),
            ("skills/playbook/SKILL.md", skill_md),
            ("mcps/demo.yaml", mcp_yaml),
        ];
        for (name, content) in files {
            let arcname = format!("{bundle_id}/{name}");
            w.start_file(arcname.as_str(), opts).unwrap();
            w.write_all(content.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

/// 构造 traverse 所需的请求上下文（含 CAPABILITY_VISITOR）。
fn ctx_with(
    workdir: Option<&str>,
    agent_id: Option<&str>,
) -> (Arc<dyn InvokeRequest>, Arc<DefaultToolVisitor>) {
    let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    if let Some(w) = workdir {
        ctx.set(WORKDIR, w.to_string());
    }
    if let Some(b) = agent_id {
        ctx.set(AGENT_ID, b.to_string());
    }
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    let manager: Arc<DefaultToolVisitor> = Arc::new(DefaultToolVisitor::new());
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::clone(&manager) as Arc<dyn crate::symbio_core::CapabilityVisitor>,
    );
    (ctx, manager)
}

#[tokio::test]
async fn bundle_import_traverse_and_identity() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().to_str().unwrap();

    // ── 1. 导入（zip → bundle store）──
    let store = BundleStore::new(dir.path().join("agent"), Some(workdir));
    let zip_bytes = build_bundle_zip("com.symbio.test-fixture", "^1");
    let result = store
        .import(&zip_bytes, false)
        .unwrap_or_else(|e| panic!("import 失败: {e}"));
    assert_eq!(result.id, "com.symbio.test-fixture");
    assert!(!result.replaced);
    assert!(Path::new(&result.dir).join("prompts/persona.md").exists());

    // ── 2. traverse：扫描约定目录装配 → 身份工具注册（工具经 MCP，不由本插件注册）──
    let plugin = Arc::new(AgentPlugin::new());
    let (ctx, manager) = ctx_with(Some(workdir), Some("com.symbio.test-fixture"));
    // `traverse` 是 `self: Arc<Self>`，会吃掉这个 Arc —— 后面还要用它构造记忆门面
    plugin
        .clone()
        .traverse(String::new(), ctx.clone())
        .await
        .unwrap();

    // 收集期错误桶应为空（装配成功）
    let errors = crate::symbio_core::take_errors(&ctx).await;
    assert!(errors.is_empty(), "不应有收集期错误: {errors:?}");

    // 注册两个能力：agent_identity（persona + skill 拼接）+ agent_run（始终注册的
    // 子智能体委托工具，不依赖"是否选择智能体"）
    let caps = manager.list_capability().await;
    assert_eq!(
        caps.len(),
        2,
        "应注册 identity + agent_run 两个能力: {caps:?}"
    );
    let identity_meta = caps.iter().find(|c| c.name == "agent_identity").unwrap();
    assert!(
        identity_meta.description.contains("全栈开发人格"),
        "身份描述应含 persona 片段（身份锚定）: {}",
        identity_meta.description
    );

    // ── 3. 身份工具调用：取回全文（persona + skill）──
    let tool_ctx = ctx.fork();
    let resp = manager.invoke("agent_identity", tool_ctx).await.unwrap();
    let data: serde_json::Value = resp.get().unwrap();
    let identity = data["content"].as_str().unwrap();
    assert!(identity.contains("com.symbio.test-fixture"));
    assert!(identity.contains("先澄清需求再动工"));
    assert!(!identity.contains("priority"), "frontmatter 应被剥离");

    // ── 3b. 系统提示词片段：人格 + 智能体记忆（选了这个智能体才注入）──
    let segments = manager.list_system_prompts().await;
    let names: Vec<&str> = segments.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        vec!["agent-identity", "agent-memory"],
        "两个片段都要注册: {names:?}"
    );
    let identity_seg = &segments[0].1;
    assert!(
        identity_seg.contains("全栈开发人格") && identity_seg.contains("先澄清需求再动工"),
        "人格片段要带全文（persona + skill）: {identity_seg}"
    );
    assert!(
        identity_seg.contains(".vdfs/agent/com.symbio.test-fixture/"),
        "人格条目要带可编辑来源目录: {identity_seg}"
    );
    assert!(
        identity_seg.contains("prompts/<name>.md"),
        "人格条目要带条目地址模板: {identity_seg}"
    );
    let memory_seg = &segments[1].1;
    assert!(
        memory_seg.contains(".vdfs/agent/com.symbio.test-fixture/AGENTS.md"),
        "记忆片段要带地址（在智能体自己的目录里）: {memory_seg}"
    );

    // ── 3c. 记忆落位：bundle 自己的目录，不是工作区根 ──
    // 读写走内核（`MemoryFile`），本插件只提供落位与地址
    let memory = plugin.memory_store(&store, "com.symbio.test-fixture").await;
    memory.write("该智能体记住：先写测试。").unwrap();
    assert_eq!(
        memory.path().unwrap(),
        Path::new(&result.dir).join("AGENTS.md"),
        "智能体记忆落在 bundle 目录"
    );
    assert!(
        !Path::new(workdir).join("AGENTS.md").exists(),
        "智能体记忆不得落到工作区根（那是 work 插件的作用域）"
    );
    assert_eq!(memory.read().unwrap(), "该智能体记住：先写测试。");

    // ── 4. 导出（打包下载语义）──
    let exported = store.export("com.symbio.test-fixture").unwrap();
    assert!(!exported.is_empty());
    assert!(exported.starts_with(b"PK")); // zip magic

    // ── 5. 删除 ──
    store.delete("com.symbio.test-fixture").unwrap();
    assert!(store.get("com.symbio.test-fixture").is_none());
}

#[tokio::test]
async fn version_mismatch_bundle_is_rejected_and_unbound_session_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().to_str().unwrap();

    let store = BundleStore::new(dir.path().join("agent"), Some(workdir));
    // requires.spec = ^2 与宿主 SPEC_MAJOR=1 不匹配 → 导入即拒绝（规范 §10.2）
    let zip_bytes = build_bundle_zip("com.acme.future", "^2");
    let err = store.import(&zip_bytes, false).unwrap_err();
    assert!(err.contains("拒绝导入"), "错误应含版本拒绝语义: {err}");

    // 未选择智能体的会话：bundle 装配静默跳过（无 identity），错误桶为空；
    // agent_run 作为会话基础能力仍然注册（agent_id 可选，默认沿用当前会话智能体，
    // 两者皆空则子会话以纯对话模式运行）。
    let plugin = Arc::new(AgentPlugin::new());
    let (ctx, manager) = ctx_with(None, None);
    plugin.traverse(String::new(), ctx.clone()).await.unwrap();
    let errors = crate::symbio_core::take_errors(&ctx).await;
    assert!(errors.is_empty(), "未绑定会话不应有收集期错误: {errors:?}");
    let caps = manager.list_capability().await;
    assert_eq!(caps.len(), 1, "仅注册 agent_run: {caps:?}");
    assert_eq!(caps[0].name, "agent_run");
    // 没选智能体 → 人格与记忆都**不注入**（这是作用域闸门，不是优化）
    assert!(
        manager.list_system_prompts().await.is_empty(),
        "未选择智能体时不得注入任何人格 / 记忆片段"
    );
}
