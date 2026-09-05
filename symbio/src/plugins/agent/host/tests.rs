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
    CapabilityManager, DefaultToolManager, InvokeRequest, InvokeRequestExt, Plugin, SimpleRequest,
    AGENT_ID, CAPABILITY_MANAGER, PATH, TRAVERSE_AVAILABLE_TOOLS, WORKDIR,
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

/// 构造 traverse 所需的请求上下文（含 CAPABILITY_MANAGER）。
fn ctx_with(
    workdir: Option<&str>,
    agent_id: Option<&str>,
) -> (Arc<dyn InvokeRequest>, Arc<DefaultToolManager>) {
    let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    if let Some(w) = workdir {
        ctx.set(WORKDIR, w.to_string());
    }
    if let Some(b) = agent_id {
        ctx.set(AGENT_ID, b.to_string());
    }
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    let manager: Arc<DefaultToolManager> = Arc::new(DefaultToolManager::new());
    ctx.set(
        CAPABILITY_MANAGER,
        Arc::clone(&manager) as Arc<dyn crate::symbio_core::CapabilityManager>,
    );
    (ctx, manager)
}

#[tokio::test]
async fn bundle_import_traverse_and_identity() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().to_str().unwrap();

    // ── 1. 导入（zip → bundle store）──
    let store = BundleStore::new(Some(workdir));
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
    plugin.traverse(String::new(), ctx.clone()).await.unwrap();

    // 收集期错误桶应为空（装配成功）
    let errors = crate::symbio_core::take_errors(&ctx).await;
    assert!(errors.is_empty(), "不应有收集期错误: {errors:?}");

    // 只注册一个能力：agent_identity（persona + skill 拼接）
    let caps = manager.list_capability().await;
    assert_eq!(caps.len(), 1, "应只注册 identity 一个能力: {caps:?}");
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

    let store = BundleStore::new(Some(workdir));
    // requires.spec = ^2 与宿主 SPEC_MAJOR=1 不匹配 → 导入即拒绝（规范 §10.2）
    let zip_bytes = build_bundle_zip("com.acme.future", "^2");
    let err = store.import(&zip_bytes, false).unwrap_err();
    assert!(err.contains("拒绝导入"), "错误应含版本拒绝语义: {err}");

    // 未选择智能体的会话完全静默（无错误、无能力注册）
    let plugin = Arc::new(AgentPlugin::new());
    let (ctx, manager) = ctx_with(None, None);
    plugin.traverse(String::new(), ctx.clone()).await.unwrap();
    assert!(manager.list_capability().await.is_empty());
}
