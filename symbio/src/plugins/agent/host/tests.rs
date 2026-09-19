//! host 层端到端测试：Agent 导入 → traverse 装配 → 记忆落位 → 导出 → 删除。
//!
//! 覆盖规范 §4（目录结构）/ §5（manifest）/ §8（装配语义）/ §10（版本接入门槛）
//! 的主链路，全部进程内完成（tempdir + 内存 zip），不依赖真实文件系统布局。
//!
//! Agent 采用**目录即配置**布局（约定优于配置）：`manifest.yaml` + `AGENTS.md`
//! + 能力插件目录（`skill/` `mcp/` `setting/`），存在即安装，无需在 manifest 里登记。

use super::plugin::AgentPlugin;
use super::store::BundleStore;
use crate::symbio_core::{vdfs, vdfs_provider::VdfsProvider};
use crate::symbio_core::{
    CapabilityVisitor, ConfigurableVisitor, DefaultConfigurableVisitor, DefaultToolVisitor,
    InvokeRequest, InvokeRequestExt, Plugin, PluginDir, SimpleRequest, AGENT_ID,
    CAPABILITY_VISITOR, CONFIG_VISITOR, PATH, PLUGIN_AGENT, PLUGIN_DIR, TRAVERSE_AVAILABLE_TOOLS,
    VDFS_PARENT_ADDR, WORKDIR,
};
use std::path::Path;
use std::sync::Arc;

/// 在内存中生成一个 v2 布局的 Agent zip。
///
/// - `manifest.yaml`：身份 + 兼容门槛（§5 / §10）
/// - `AGENTS.md`：人格与记忆（§6）
/// - `skill/playbook/SKILL.md`：技能插件目录（§7）
fn build_agent_zip(agent_id: &str, requires_spec: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        let manifest = format!(
            r#"spec: "agent-dir/v2"
id: "{agent_id}"
name: "全栈开发智能体"
version: "1.0.0"
requires:
  spec: "{requires_spec}"
"#
        );
        let agents_md = "你是全栈开发人格：先澄清需求再动工。";
        let skill_md =
            "---\nname: playbook\ndescription: 交付流程手册：先澄清需求再动工\n---\n\n# 交付\n";

        let files: Vec<(&str, &str)> = vec![
            ("manifest.yaml", &manifest),
            ("AGENTS.md", agents_md),
            ("skill/playbook/SKILL.md", skill_md),
        ];
        for (name, content) in files {
            let arcname = format!("{agent_id}/{name}");
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
    // 合成父地址：模拟容器转发时写入的当前父地址（不依赖真实挂载名）
    ctx.set(VDFS_PARENT_ADDR, "@vfs/agent".to_string());
    let manager: Arc<DefaultToolVisitor> = Arc::new(DefaultToolVisitor::new());
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::clone(&manager) as Arc<dyn crate::symbio_core::CapabilityVisitor>,
    );
    (ctx, manager)
}

#[tokio::test]
async fn agent_import_traverse_and_memory() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().to_str().unwrap();

    // ── 1. 导入（zip → Agent 目录）──
    let store = BundleStore::new(dir.path().join("agent"), Some(workdir));
    let zip_bytes = build_agent_zip("com.symbio.test-fixture", "^2");
    let result = store
        .import(&zip_bytes, false)
        .unwrap_or_else(|e| panic!("import 失败: {e}"));
    assert_eq!(result.id, "com.symbio.test-fixture");
    assert!(!result.replaced);
    assert!(Path::new(&result.dir).join("AGENTS.md").exists());
    assert!(Path::new(&result.dir)
        .join("skill/playbook/SKILL.md")
        .exists());

    // ── 2. traverse：整棵插件树并进会话（能力带来源前缀，§8.2）──
    let plugin = Arc::new(AgentPlugin::new());
    let (ctx, manager) = ctx_with(Some(workdir), Some("com.symbio.test-fixture"));
    plugin
        .clone()
        .traverse(String::new(), ctx.clone())
        .await
        .unwrap();

    // 收集期错误桶应为空（装配成功）
    let errors = crate::symbio_core::take_errors(&ctx).await;
    assert!(errors.is_empty(), "不应有收集期错误: {errors:?}");

    // agent_run 无条件注册；子 Agent 的技能工具带来源前缀进来
    let caps = manager.list_capability().await;
    assert!(
        caps.iter().any(|c| c.name == "agent_run"),
        "agent_run 应始终注册: {caps:?}"
    );
    assert!(
        caps.iter()
            .any(|c| c.name.starts_with("agent_com_symbio_test-fixture_")),
        "子 Agent 的能力应带来源前缀: {:?}",
        caps.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    // ── 3. 系统提示词：子 Agent 自身的 AGENTS.md 由**本插件**注入 ──
    // 子树里没有 `agent` 实例，而认识 Agent 目录的正是本插件；段名经作用域 visitor
    // 加 `agent/<id>/` 前缀，因此与系统侧的同名条目互不覆盖
    let segments = manager.list_system_prompts().await;
    let names: Vec<&str> = segments.iter().map(|(n, _)| n.as_str()).collect();
    const OWN_SEGMENT: &str = "agent/com.symbio.test-fixture/agent-memory";
    assert!(
        names.contains(&OWN_SEGMENT),
        "智能体自身的 AGENTS.md 应由本插件带前缀注入: {names:?}"
    );
    let injected = segments
        .iter()
        .find(|(n, _)| n == OWN_SEGMENT)
        .map(|(_, t)| t.as_str())
        .unwrap();
    assert!(
        injected.contains("你是全栈开发人格"),
        "注入内容应来自该 bundle 自己的 AGENTS.md: {injected}"
    );
    assert!(
        injected.contains("@vfs/agent/com.symbio.test-fixture/AGENTS.md"),
        "片段应指路整包浏览面里的那个地址: {injected}"
    );

    // ── 3b. 记忆落位：Agent 自己的目录，不是工作区根 ──
    // 读写走内核（`MemoryFile`），本插件只提供落位
    let memory = plugin.memory_store(&store, "com.symbio.test-fixture").await;
    memory.write("该智能体记住：先写测试。").unwrap();
    assert_eq!(
        memory.path().unwrap(),
        Path::new(&result.dir).join("AGENTS.md"),
        "智能体记忆落在 Agent 目录"
    );
    assert!(
        !Path::new(workdir).join("AGENTS.md").exists(),
        "智能体记忆不得落到工作区根（那是 work 插件的作用域）"
    );

    // ── 4. 导出（打包下载语义）──
    let exported = store.export("com.symbio.test-fixture").unwrap();
    assert!(!exported.is_empty());
    assert!(exported.starts_with(b"PK")); // zip magic

    // ── 5. 删除 ──
    store.delete("com.symbio.test-fixture").unwrap();
    assert!(store.get("com.symbio.test-fixture").is_none());
}

#[tokio::test]
async fn version_mismatch_agent_is_rejected_and_unbound_session_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().to_str().unwrap();

    let store = BundleStore::new(dir.path().join("agent"), Some(workdir));
    // requires.spec = ^9 与宿主主版本 2 不匹配 → 导入即拒绝（规范 §10）
    let zip_bytes = build_agent_zip("com.acme.future", "^9");
    let err = store.import(&zip_bytes, false).unwrap_err();
    assert!(err.contains("拒绝导入"), "错误应含版本拒绝语义: {err}");

    // 未选择智能体的会话：不装配任何 Agent，错误桶为空；
    // agent_run 作为会话基础能力仍然注册。
    let plugin = Arc::new(AgentPlugin::new());
    let (ctx, manager) = ctx_with(None, None);
    plugin.traverse(String::new(), ctx.clone()).await.unwrap();
    let errors = crate::symbio_core::take_errors(&ctx).await;
    assert!(errors.is_empty(), "未绑定会话不应有收集期错误: {errors:?}");
    let caps = manager.list_capability().await;
    assert_eq!(caps.len(), 1, "仅注册 agent_run: {caps:?}");
    assert_eq!(caps[0].name, "agent_run");
    // 没选智能体 → 不注入任何人格 / 记忆片段（这是作用域闸门，不是优化）
    assert!(
        manager.list_system_prompts().await.is_empty(),
        "未选择智能体时不得注入任何人格 / 记忆片段"
    );
}

/// manifest 不合规的目录**拒绝接入**，且错误写明双侧版本（§10 不得静默降级）
#[tokio::test]
async fn nonconforming_agent_is_rejected_with_both_versions() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path().to_str().unwrap();
    let root = dir.path().join("agent");
    // 一个没有 manifest 的目录：store 扫不到 → 绑定它应报拒绝接入
    std::fs::create_dir_all(root.join("ghost")).unwrap();

    let plugin = Arc::new(AgentPlugin::new());
    let (ctx, _manager) = ctx_with(Some(workdir), Some("ghost"));
    plugin.traverse(String::new(), ctx.clone()).await.unwrap();

    let errors = crate::symbio_core::take_errors(&ctx).await;
    assert_eq!(errors.len(), 1, "应有且仅有一条拒绝接入: {errors:?}");
    assert!(
        errors[0].message.contains("拒绝接入") && errors[0].message.contains("agent-dir/v2"),
        "错误应写明本宿主支持的版本: {:?}",
        errors[0]
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// v2：子 Agent 是一棵 composite 插件树（规范 `docs/design/agent-directory-spec.md`）
// ---------------------------------------------------------------------
// 验证三件事：manifest 声明 `agent-dir/v2` 的目录会被挂成插件树；它的注册经
// 代理层带上来源前缀（与系统树不冲突）；它的 `setting` 实例把自己目录下的
// `AGENTS.md` 注入成【智能体指令】。
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn v2_sub_agent_tree_is_assembled_and_prefixed() {
    use crate::symbio_core::{PluginDir, PLUGIN_AGENT, PLUGIN_DIR};

    let tmp = tempfile::tempdir().unwrap();
    let agent_root = tmp.path().join("agent");
    let sub = agent_root.join("reviewer");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(
        sub.join("manifest.yaml"),
        "spec: \"agent-dir/v2\"\nid: \"reviewer\"\nname: \"评审\"\nversion: \"1.0.0\"\nrequires://n  spec: \"^2\"\n",
    )
    .unwrap();
    // 人格 / 记忆：`<agentdir>/AGENTS.md`，由子树的 `setting` 实例读取并注入
    // （宿主不再把子树的 WORKDIR 改指本目录——`work` 只认工作区，那个覆写是错的）
    std::fs::write(sub.join("AGENTS.md"), "你是评审专家。").unwrap();
    // 一个技能：落在子 Agent **自己的** skill 插件目录下（有技能它才注册 read_skill）
    let skill_dir = sub.join("skill").join("demo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo\ndescription: 演示技能：对改动做一次评审\n---\n\n# 演示\n",
    )
    .unwrap();

    // 构造插件：把 PLUGIN_DIR 指到 <tmp>/agent
    let build_ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    build_ctx.set(PLUGIN_DIR, PluginDir::at(&agent_root, PLUGIN_AGENT));
    let plugin = AgentPlugin::build(build_ctx);

    let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    // 合成父地址：模拟容器转发时写入的当前父地址（不依赖真实挂载名）
    ctx.set(VDFS_PARENT_ADDR, "@vfs/agent".to_string());
    ctx.set(AGENT_ID, "reviewer".to_string());
    ctx.set(WORKDIR, tmp.path().to_string_lossy().to_string());
    let manager: Arc<DefaultToolVisitor> = Arc::new(DefaultToolVisitor::new());
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::clone(&manager) as Arc<dyn CapabilityVisitor>,
    );

    plugin.traverse(String::new(), ctx).await.unwrap();

    // 1) 子 Agent 的 AGENTS.md 由本插件注入，段名带来源前缀
    //    （与系统侧的 `agent-memory` / `agent-instructions` 不冲突）
    let prompts = manager.list_system_prompts().await;
    let names: Vec<String> = prompts.iter().map(|(n, _)| n.clone()).collect();
    const OWN_SEGMENT: &str = "agent/reviewer/agent-memory";
    assert!(
        names.contains(&OWN_SEGMENT.to_string()),
        "子 Agent 的注册应带来源前缀，实际：{names:?}"
    );
    let injected = prompts
        .iter()
        .find(|(n, _)| n == OWN_SEGMENT)
        .map(|(_, t)| t.as_str())
        .unwrap();
    assert!(injected.contains("你是评审专家"), "{injected}");
    // 地址指向本插件的整包浏览面，且**印出真实写入闸门**（闸门由本插件执行）
    assert!(
        injected.contains("@vfs/agent/reviewer/AGENTS.md"),
        "片段应指路整包浏览面里的地址: {injected}"
    );
    assert!(
        injected.contains("本智能体私有，与【工作区记忆】相互独立"),
        "同名不同作用域必须点明，否则模型会把两件事写混: {injected}"
    );

    // 2) 子 Agent 的 skill 实例注册的工具被代理层改名，不与系统的 `read_skill` 撞名
    let tools: Vec<String> = manager
        .list_capability()
        .await
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(
        tools.iter().any(|t| t.starts_with("agent_reviewer_")),
        "子 Agent 的工具应带来源前缀，实际：{tools:?}"
    );
}

/// agent 挂载根**只列装进来的子智能体**，系统自身的指令（`AGENTS.md`）不在此列——
/// 它是「本 agent 的修改」，入口在设置页，混进列表会被读成某个包。
#[tokio::test]
async fn mount_root_lists_only_installed_agents() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path().to_string_lossy().to_string();
    let agent_root = tmp.path().join("agent");
    std::fs::create_dir_all(&agent_root).unwrap();

    let store = BundleStore::new(agent_root.clone(), Some(&workdir));
    store
        .import(&build_agent_zip("com.acme.demo", "^2"), false)
        .unwrap();

    let host: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    host.set(PLUGIN_DIR, PluginDir::at(&agent_root, PLUGIN_AGENT));
    host.set(WORKDIR, workdir);
    let ctx = vdfs::vdfs_context(&host);

    let plugin = AgentPlugin::new();
    let items = plugin.list(&ctx, "").await.unwrap();
    let names: Vec<&str> = items.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["com.acme.demo"],
        "挂载根只有装进来的智能体，系统指令不在此列：{names:?}"
    );

    // 不进清单 ≠ 不可达：地址照旧可读（设置页那个入口指向它）
    let n = plugin.stat(&ctx, "AGENTS.md").await.unwrap();
    assert_eq!(n.title, "全局指令");
}

/// 设置页收到两条 agent 条目：配置文档（`agent/PLUGIN.yml`）+ 系统自身指令
/// （`agent/AGENTS.md`）——两条地址都指向本插件，读写仍落在本插件文件上。
#[tokio::test]
async fn traverse_declares_config_and_instruction_in_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path().to_string_lossy().to_string();
    let agent_root = tmp.path().join("agent");
    std::fs::create_dir_all(&agent_root).unwrap();
    // 系统智能体自身的指令：挂在 agent 目录的**父目录**（homedir）
    std::fs::write(tmp.path().join("AGENTS.md"), "你是系统智能体。").unwrap();

    let store = BundleStore::new(agent_root.clone(), Some(&workdir));
    store
        .import(&build_agent_zip("com.acme.demo", "^2"), false)
        .unwrap();

    let plugin = Arc::new(AgentPlugin::new());
    let (ctx, _manager) = ctx_with(Some(&workdir), Some("com.acme.demo"));
    let configs: Arc<dyn ConfigurableVisitor> = Arc::new(DefaultConfigurableVisitor::new());
    ctx.set(CONFIG_VISITOR, Arc::clone(&configs));

    plugin.traverse(String::new(), ctx).await.unwrap();

    let entries = configs.list_configurables().await;
    let by_name: std::collections::HashMap<&str, &crate::symbio_core::vdfs_provider::VdfsNode> =
        entries.iter().map(|n| (n.name.as_str(), n)).collect();
    // 配置文档（name = 目录名 agent）
    assert!(
        by_name.contains_key("agent"),
        "应含 agent 配置文档：{by_name:?}"
    );
    // 系统自身指令（name = AGENTS.md，地址指向本插件）
    let instr = by_name
        .get("AGENTS.md")
        .expect("设置页应含系统指令条目（agent 列表里没有它）");
    assert_eq!(instr.path, "agent/AGENTS.md");
    assert_eq!(instr.title, "全局指令");
    assert_eq!(instr.ext.as_deref(), Some("md"));
}
