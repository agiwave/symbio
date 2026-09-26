//! host 层端到端测试：Agent 导入 → traverse 装配 → 记忆落位 → 导出 → 删除。
//!
//! 覆盖规范 §4（目录结构）/ §5（manifest）/ §8（装配语义）/ §10（版本接入门槛）
//! 的主链路，全部进程内完成（tempdir + 内存 zip），不依赖真实文件系统布局。
//!
//! Agent 采用**目录即配置**布局（约定优于配置）：`manifest.yaml` + `AGENTS.md`
//! + 能力插件目录（`skill/` `mcp/` `plugin_manager/`），存在即安装，无需在 manifest 里登记。

use super::plugin::AgentPlugin;
use super::store::AgentDirStore;
use crate::providers::{DefaultConfigurableVisitor, DefaultToolVisitor};
use crate::symbio_core::{vdfs, VdfsProvider};
use crate::symbio_core::{
    CapabilityVisitor, ConfigurableVisitor, Plugin, PluginDir, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginSimpleRequest, AGENT_ID, CAPABILITY_VISITOR,
    CONFIGURABLE_VISITOR, PATH, PLUGIN_ID_AGENT, TRAVERSE_AVAILABLE_TOOLS, VDFS_PARENT_ADDR,
    WORKDIR,
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

/// 内存 zip：只放一份 manifest（文件名与内容由调用方给）——导入门槛用例用。
fn build_minimal_zip(manifest_name: &str, manifest: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
        w.start_file(manifest_name, opts).unwrap();
        w.write_all(manifest.as_bytes()).unwrap();
        w.finish().unwrap();
    }
    buf.into_inner()
}

/// 构造 traverse 所需的请求上下文（含 CAPABILITY_VISITOR）。
fn ctx_with(
    workdir: Option<&str>,
    agent_id: Option<&str>,
) -> (Arc<dyn PluginInvokeRequest>, Arc<DefaultToolVisitor>) {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
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
    let store = AgentDirStore::new(dir.path().join("agent"));
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
    // 插件作用域到导入目录，使「发现根 == 导入位置」（与生产态 `PLUGIN_DIR` 语义一致）
    let plugin = Arc::new(AgentPlugin::new_with_dir(PluginDir::at(
        dir.path().join("agent"),
        PLUGIN_ID_AGENT,
    )));
    let (ctx, manager) = ctx_with(Some(workdir), Some("com.symbio.test-fixture"));
    plugin
        .clone()
        .traverse(String::new(), ctx.clone())
        .await
        .unwrap();

    // 收集期错误桶应为空（装配成功）
    let errors = crate::symbio_core::capability_take_errors(&ctx).await;
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
        "注入内容应来自该 agent 目录自己的 AGENTS.md: {injected}"
    );
    assert!(
        injected.contains("@vfs/agent/com.symbio.test-fixture/AGENTS.md"),
        "片段应指路整包浏览面里的那个地址: {injected}"
    );

    // ── 3b. 记忆落位：Agent 自己的目录，不是工作区根 ──
    // 读写走共享实现（`MemoryFile`），本插件只提供落位
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

    let store = AgentDirStore::new(dir.path().join("agent"));
    // requires.spec = ^9 与宿主主版本 2 不匹配 → 导入即拒绝（规范 §10）
    let zip_bytes = build_agent_zip("com.acme.future", "^9");
    let err = store.import(&zip_bytes, false).unwrap_err();
    assert!(err.contains("拒绝导入"), "错误应含版本拒绝语义: {err}");

    // 未选择智能体的会话：不装配任何 Agent，错误桶为空；
    // agent_run 作为会话基础能力仍然注册。
    let plugin = Arc::new(AgentPlugin::new());
    let (ctx, manager) = ctx_with(None, None);
    plugin.traverse(String::new(), ctx.clone()).await.unwrap();
    let errors = crate::symbio_core::capability_take_errors(&ctx).await;
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

    let errors = crate::symbio_core::capability_take_errors(&ctx).await;
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
// 代理层带上来源前缀（与系统树不冲突）；它的 `plugin_manager` 实例把自己目录下的
// `AGENTS.md` 注入成【智能体指令】。
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn v2_sub_agent_tree_is_assembled_and_prefixed() {
    use crate::symbio_core::{PluginDir, PLUGIN_DIR, PLUGIN_ID_AGENT};

    let tmp = tempfile::tempdir().unwrap();
    let agent_root = tmp.path().join("agent");
    let sub = agent_root.join("reviewer");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(
        sub.join("manifest.yaml"),
        "spec: \"agent-dir/v2\"\nid: \"reviewer\"\nname: \"评审\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^2\"\n",
    )
    .unwrap();
    // 人格 / 记忆：`<agentdir>/AGENTS.md`，由子树的 `plugin_manager` 实例读取并注入
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
    let build_ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    build_ctx.set(PLUGIN_DIR, PluginDir::at(&agent_root, PLUGIN_ID_AGENT));
    let plugin = AgentPlugin::build(build_ctx);

    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
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

/// 钻进 v2 子智能体根（`agent/<id>`）应**穿过挂载点**列出子 composite 的根视图，
/// 因此与父（系统）根完全一致——只显示可见插件，`root_hidden` 的配置型挂载点
/// （gateway/web/telegram/local/work）不出现在侧边栏。这正是「父只显示可见、
/// 子却显示全部」这一回归的根因修复：之前 `RelPath::Agent` 直接裸列 agent 目录，
/// 绕过了 `CompositeVfs` 的 `root_hidden` 过滤。
#[tokio::test]
async fn sub_agent_root_crosses_mount_and_hides_root_hidden() {
    use crate::symbio_core::{PluginDir, PLUGIN_ID_AGENT};

    let tmp = tempfile::tempdir().unwrap();
    let agent_root = tmp.path().join("agent");
    let sub = agent_root.join("reviewer");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(
        sub.join("manifest.yaml"),
        "spec: \"agent-dir/v2\"\nid: \"reviewer\"\nname: \"评审\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^2\"\n",
    )
    .unwrap();
    std::fs::write(sub.join("AGENTS.md"), "你是评审专家。").unwrap();

    let plugin = AgentPlugin::new_with_dir(PluginDir::at(&agent_root, PLUGIN_ID_AGENT));

    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    // 合成父地址：模拟容器转发时写入的当前父地址（不依赖真实挂载名）
    ctx.set(VDFS_PARENT_ADDR, "@vfs/agent".to_string());
    // 请求上下文**不带 `PLUGIN_DIR`**——那是**装配期**的键（见
    // `sub_agent_agent_list_is_scoped_to_its_own_space`）：插件的作用域由它**自己**
    // 持有的目录决定，不由请求方喂。请求方塞一个进来，测的就不是生产形状了。
    ctx.set(AGENT_ID, "reviewer".to_string());
    ctx.set(WORKDIR, tmp.path().to_string_lossy().to_string());
    let vctx = vdfs::vdfs_context(&ctx);

    // 钻进子智能体根：列 `agent/reviewer` 的「下一层」
    let items = plugin
        .dispatch(&vctx, "reviewer", LIST)
        .await
        .unwrap()
        .into_list()
        .unwrap();
    let names: Vec<&str> = items.iter().map(|it| it.node.name.as_str()).collect();

    // 1) 穿过挂载点：返回的是子 composite 视图（名字是资源入口），而不是裸目录
    //    （`AGENTS.md` / `manifest.yaml` / `skill` 目录）。
    assert!(
        names
            .iter()
            .any(|n| matches!(*n, "session" | "mcp" | "skill" | "plugin_manager" | "agent")),
        "子根应经子 composite 列出可见资源入口，实际：{names:?}"
    );
    //    ⚠️ 判据**不是**「条目路径带挂载段」：条目地址由访问层按请求地址回填，
    //    挂载点填不出来（它不知道自己被挂在哪，`mount_rel` 只是本插件空间内的
    //    首段）——本层只会留空。见
    //    `sub_agent_mount_crossing_is_uniform_across_operations` ①。
    assert!(
        items.iter().all(|it| it.path.is_empty()),
        "条目地址应留空交访问层回填（挂载点不填地址），实际：{:?}",
        items.iter().map(|it| &it.path).collect::<Vec<_>>()
    );
    // 2) 关键回归：`root_hidden` 的配置型挂载点不得出现在侧边栏（与系统根一致）
    for hidden in ["gateway", "web", "telegram", "local", "work"] {
        assert!(
            !names.contains(&hidden),
            "子根不应列出 root_hidden 的 {hidden}（与系统根一致），实际：{names:?}"
        );
    }
}

/// agent 挂载根**只列装进来的子智能体**，系统自身的指令（`AGENTS.md`）不在此列——
/// 它是「本 agent 的修改」，入口在设置页，混进列表会被读成某个包。
#[tokio::test]
async fn mount_root_lists_only_installed_agents() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path().to_string_lossy().to_string();
    let agent_root = tmp.path().join("agent");
    std::fs::create_dir_all(&agent_root).unwrap();

    let store = AgentDirStore::new(agent_root.clone());
    store
        .import(&build_agent_zip("com.acme.demo", "^2"), false)
        .unwrap();

    // 请求上下文**不带 `PLUGIN_DIR`**（同 `sub_agent_agent_list_is_scoped_to_its_own_space`：
    // 作用域来自插件自持的目录，不是请求方喂的）
    let host: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    host.set(WORKDIR, workdir);
    let ctx = vdfs::vdfs_context(&host);

    let plugin = AgentPlugin::new_with_dir(PluginDir::at(&agent_root, PLUGIN_ID_AGENT));
    let items = plugin
        .dispatch(&ctx, "", LIST)
        .await
        .unwrap()
        .into_list()
        .unwrap();
    let names: Vec<&str> = items.iter().map(|it| it.node.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["com.acme.demo"],
        "挂载根只有装进来的智能体，系统指令不在此列：{names:?}"
    );

    // 不进清单 ≠ 不可达：地址照旧可读（设置页那个入口指向它）
    let n = plugin
        .dispatch(&ctx, "AGENTS.md", STAT)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
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

    let store = AgentDirStore::new(agent_root.clone());
    store
        .import(&build_agent_zip("com.acme.demo", "^2"), false)
        .unwrap();

    let plugin = Arc::new(AgentPlugin::new());
    let (ctx, _manager) = ctx_with(Some(&workdir), Some("com.acme.demo"));
    let configs: Arc<dyn ConfigurableVisitor> = Arc::new(DefaultConfigurableVisitor::new());
    ctx.set(CONFIGURABLE_VISITOR, Arc::clone(&configs));

    plugin.traverse(String::new(), ctx).await.unwrap();

    let entries = configs.list_configurables().await;
    let by_name: std::collections::HashMap<&str, &crate::symbio_core::VdfsItem> = entries
        .iter()
        .map(|it| (it.node.name.as_str(), it))
        .collect();
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
    assert_eq!(instr.node.title, "全局指令");
    assert_eq!(instr.node.ext.as_deref(), Some("md"));
}
/// 子智能体挂载点穿越必须**九操作一致**：`agent/<id>/…` 下的每个操作都交给子
/// composite，而不是「list / stat / delete 穿了，read / write 没穿」。
///
/// 这是 `read` / `write` 绕过挂载点的回归钉：绕过时 `read` 会去读裸 agent 目录里
/// 的同名物理文件（对只在虚拟视图里存在的路径必然 NotFound），`write` 会**报成功
/// 却把文件撒进智能体包**。
///
/// 断言用的是**数据落点**而不是「有没有报错」：`work` 挂载点在子树里读/写的是父
/// 会话工作区的记忆文件——看得见落点，才分得清「写了哪儿」。
#[tokio::test]
async fn sub_agent_mount_crossing_is_uniform_across_operations() {
    use crate::symbio_core::VdfsError;
    use crate::symbio_core::{PluginDir, PLUGIN_ID_AGENT};

    let tmp = tempfile::tempdir().unwrap();
    let agent_root = tmp.path().join("agent");
    let sub = agent_root.join("reviewer");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(
        sub.join("manifest.yaml"),
        "spec: \"agent-dir/v2\"\nid: \"reviewer\"\nname: \"评审\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^2\"\n",
    )
    .unwrap();
    std::fs::write(sub.join("AGENTS.md"), "你是评审专家。").unwrap();
    // 父会话的工作区记忆：子树 `work` 挂载点读写的就是它
    std::fs::write(tmp.path().join("AGENTS.md"), "工作区记忆内容").unwrap();

    let plugin = AgentPlugin::new_with_dir(PluginDir::at(&agent_root, PLUGIN_ID_AGENT));
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(VDFS_PARENT_ADDR, "@vfs/agent".to_string());
    // 请求上下文**不带 `PLUGIN_DIR`**——那是**装配期**的键（见
    // `sub_agent_agent_list_is_scoped_to_its_own_space`）：插件的作用域由它**自己**
    // 持有的目录决定，不由请求方喂。请求方塞一个进来，测的就不是生产形状了。
    ctx.set(AGENT_ID, "reviewer".to_string());
    ctx.set(WORKDIR, tmp.path().to_string_lossy().to_string());
    let vctx = vdfs::vdfs_context(&ctx);

    // 物理落点（= 绕过挂载点时会写进去的那个目录）
    let physical = agent_root.join("reviewer").join("work").join("AGENTS.md");

    // ① list：列出的是子 composite 的可见入口，**条目地址由访问层回填**——
    //    本层是挂载点，**不得**填条目地址：它不知道自己被挂在哪（`agent/`），
    //    填出来的会是一个少了外层挂载段的假地址，而访问层见到非空地址就
    //    「不再回填」。
    //
    //    断言必须是**空**而不是「不含根名」：错误实现填的正是挂载段本身
    //    （`reviewer`），它同样不含根名——用「不含根名」当判据会放过这个 bug，
    //    而它的表现是子空间里列出的**每一条**都顶着子空间根地址
    //    （`<根>/<agent-id>`，那是个目录），前端点开会话即「读取会话转写失败」。
    let listed = plugin
        .dispatch(&vctx, "reviewer", LIST)
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert!(
        !listed.is_empty() && listed.iter().all(|it| it.path.is_empty()),
        "挂载点不填条目地址（与容器同款契约：留空交访问层按请求地址回填），实际：{:?}",
        listed.iter().map(|it| &it.path).collect::<Vec<_>>()
    );

    // ② stat / ③ list（子路径）：两者都穿过挂载点
    plugin.dispatch(&vctx, "reviewer/work", STAT).await.unwrap();
    plugin.dispatch(&vctx, "reviewer/work", LIST).await.unwrap();

    // ④ read：读到的是**子树 provider 的数据**（工作区记忆），不是裸目录里的文件
    let c = plugin
        .dispatch(&vctx, "reviewer/work/AGENTS.md", READ)
        .await
        .unwrap()
        .into_read()
        .unwrap();
    assert_eq!(
        c.text.as_deref(),
        Some("工作区记忆内容"),
        "read 必须穿过挂载点（读到 work 挂载点的数据），实际：{c:?}"
    );

    // ⑤ write：写进子树 provider（工作区记忆），**不得**落进智能体包
    plugin
        .dispatch(
            &vctx,
            "reviewer/work/AGENTS.md",
            vdfs::VdfsRequest::Write {
                content: crate::symbio_core::VdfsContent::text("改过的记忆"),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap(),
        "改过的记忆",
        "写入应交给子树的 work 挂载点"
    );
    assert!(
        !physical.exists(),
        "写入不得绕过挂载点落到裸 agent 目录：{}",
        physical.display()
    );

    // ⑥ delete：同样穿过挂载点 —— `work` 的记忆不可删除，这条**拒绝**来自子树 provider
    // （绕过挂载点时会是另一套错误：物理路径不存在）
    let err = plugin
        .dispatch(&vctx, "reviewer/work/AGENTS.md", DEL)
        .await
        .unwrap_err();
    assert!(
        matches!(err, VdfsError::Forbidden(_)),
        "删除应交给子树 provider 裁决（记忆不可删除），实际：{err:?}"
    );

    // ⑦ mkdir：可挂载子树内的新建交给子树裁决。子树里的 provider 目前都不支持在
    // 自己根下造目录（`NotImplemented`），因此这里**不断言成败**——只钉住外部可观测的
    // 一点：它不得绕过挂载点、把目录造进裸 agent 目录。
    let mkdir_on_mount = plugin
        .dispatch(&vctx, "reviewer/skill/new-dir", vdfs::VdfsRequest::Mkdir)
        .await;
    assert!(
        !agent_root
            .join("reviewer")
            .join("skill")
            .join("new-dir")
            .exists(),
        "mkdir 不得绕过挂载点落到裸 agent 目录，实际：{mkdir_on_mount:?}"
    );
    // ⑧ 操作一致性的**判据**收在一处：`sub_vfs` 唯一决定「是否穿过挂载点」，
    // 各操作只负责把结果交回去——本测试覆盖了 list / stat / read / write / delete 的
    // 数据落点，以及 mkdir 的越界防护（它在子树里目前无 provider 实现，外部表现与
    // 未委托时相同，故只能钉住「不得落到裸目录」）。
    //
    // 曾经还有 ⑨：`move` 跨 `<id>` 明确拒绝。移动整条下线后该断言随之删除——
    // 不再有第二个地址可供越界。
}

/// 子智能体空间里的「智能体」清单必须**只反映那个空间自己**。
///
/// 生产态的请求上下文**不带 `PLUGIN_DIR`**——那个键只在**装配期**给出
/// （`composite::build` 构造子插件时、[`AgentPlugin::sub_agent`] 造子树时）。
/// 因此目录若从**请求上下文**取（`plugin_dir_from_ctx`），嵌套实例就会回退到
/// `PluginDir::of(PLUGIN_ID_AGENT)` = **全局 agent 根**：子空间里列出的「智能体」
/// 其实是**顶层**清单，用户看到的是**它自己**。
///
/// 顶层看不出这个错，是因为回退值与它自己的目录**恰好相同**。这类「只在嵌套层
/// 显形」的缺陷只能由一条**钻进嵌套层**的断言钉住——所以本用例刻意不在请求
/// 上下文里塞 `PLUGIN_DIR`（其余用例塞了，等于绕开了生产态的真实形状）。
///
/// 判据取「清单里必须有子空间**自己**的那个下级智能体」而不是「清单必须为空」：
/// 后者在全局 agent 根恰好为空时会**放过**这个 bug（错误实现在那里读到的是空清单，
/// 与正确实现无从区分）。放一个只在子空间里存在的下级智能体，两种实现必然分岔。
#[tokio::test]
async fn sub_agent_agent_list_is_scoped_to_its_own_space() {
    use crate::symbio_core::{PluginDir, PLUGIN_ID_AGENT};

    let tmp = tempfile::tempdir().unwrap();
    let agent_root = tmp.path().join("agent");
    let sub = agent_root.join("reviewer");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(
        sub.join("manifest.yaml"),
        "spec: \"agent-dir/v2\"\nid: \"reviewer\"\nname: \"评审\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^2\"\n",
    )
    .unwrap();
    std::fs::write(sub.join("AGENTS.md"), "你是评审专家。").unwrap();
    // 子空间里**真正**的下一级智能体：只存在于 `<sub>/agent/` 下
    let inner = sub.join("agent").join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::write(
        inner.join("manifest.yaml"),
        "spec: \"agent-dir/v2\"\nid: \"inner\"\nname: \"内层\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^2\"\n",
    )
    .unwrap();

    let plugin = AgentPlugin::new_with_dir(PluginDir::at(&agent_root, PLUGIN_ID_AGENT));

    // 生产态请求上下文：只有宿主该有的键，**没有 `PLUGIN_DIR`**
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(VDFS_PARENT_ADDR, "@vfs/agent".to_string());
    ctx.set(WORKDIR, tmp.path().to_string_lossy().to_string());
    let vctx = vdfs::vdfs_context(&ctx);

    // 钻进子智能体，看它自己的「智能体」挂载点
    let nested = plugin
        .dispatch(&vctx, "reviewer/agent", LIST)
        .await
        .unwrap()
        .into_list()
        .unwrap();
    let ids: Vec<&str> = nested.iter().map(|it| it.node.name.as_str()).collect();
    assert_eq!(
        ids,
        vec!["inner"],
        "子空间的智能体清单必须来自**它自己的** agent 目录（实际：{ids:?}）——\
         出现顶层智能体（尤其它自己）即表示目录回退到了全局 agent 根"
    );
}

/// 导入即校验（§10）：spec 不符 / 缺 `requires.spec` / 缺 `name` 一律整包拒收；
/// `manifest.yml`（规范名之外的容忍名）照常可导入。
#[test]
fn import_rejects_any_manifest_failing_the_access_gate() {
    let dir = tempfile::tempdir().unwrap();
    let store = AgentDirStore::new(dir.path().join("agent"));
    let base = "id: \"com.acme.x\"\nversion: \"1.0.0\"\n";
    let old_spec = format!("spec: \"oab/v1\"\n{base}name: \"X\"\nrequires:\n  spec: \"^1\"\n");

    for (what, manifest) in [
        ("spec 不符", old_spec.clone()),
        (
            "缺 requires.spec",
            format!("spec: \"agent-dir/v2\"\n{base}name: \"X\"\n"),
        ),
        (
            "缺 name",
            format!("spec: \"agent-dir/v2\"\n{base}requires:\n  spec: \"^2\"\n"),
        ),
    ] {
        let err = store
            .import(&build_minimal_zip("manifest.yaml", &manifest), false)
            .unwrap_err();
        assert!(err.contains("拒绝导入"), "{what} 应被拒收：{err}");
    }

    // §10 不得静默降级：版本门槛的错误信息写明双侧版本
    let err = store
        .import(&build_minimal_zip("manifest.yaml", &old_spec), false)
        .unwrap_err();
    assert!(err.contains("agent-dir/v2"), "{err}");

    // 三名之内（`manifest.yml`）照常导入——判据是「过 §10」，不是文件名
    assert!(store
        .import(
            &build_minimal_zip(
                "manifest.yml",
                "spec: \"agent-dir/v2\"\nid: \"com.acme.y\"\nname: \"Y\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^2\"\n",
            ),
            false,
        )
        .is_ok());
}

/// 不在挂载根清单里的目录（manifest 未过 §10 门槛）**一律不可浏览**：
/// list / stat / read / write / delete 全部 `NotFound`，且**磁盘不落新文件**。
///
/// 「可列出」与「可挂载」共用同一判据（[`AgentDirStore::load_record`]），
/// 因此不存在「列得出来、点进去却报别的错」的中间态。
#[tokio::test]
async fn non_mountable_agent_dir_is_not_browsable_at_all() {
    use crate::symbio_core::VdfsError;

    let tmp = tempfile::tempdir().unwrap();
    let agent_root = tmp.path().join("agent");
    let sub = agent_root.join("legacyish");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(
        sub.join("manifest.yaml"),
        "spec: \"oab/v1\"\nid: \"legacyish\"\nname: \"旧\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^1\"\n",
    )
    .unwrap();

    let plugin = AgentPlugin::new_with_dir(PluginDir::at(&agent_root, PLUGIN_ID_AGENT));
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(VDFS_PARENT_ADDR, "@vfs/agent".to_string());
    let vctx = vdfs::vdfs_context(&ctx);

    // 列不出来：挂载根清单里没有它
    let listed = plugin
        .dispatch(&vctx, "", LIST)
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert!(
        !listed.iter().any(|it| it.node.name == "legacyish"),
        "未过门槛的目录不得出现在挂载根清单里"
    );

    for (what, path, req) in [
        ("list", "legacyish", LIST),
        ("list 子路径", "legacyish/foo", LIST),
        ("stat", "legacyish/foo.md", STAT),
        ("read", "legacyish/foo.md", READ),
        (
            "write",
            "legacyish/foo.md",
            VdfsRequest::Write {
                content: crate::symbio_core::VdfsContent::text("x"),
            },
        ),
        ("delete", "legacyish/foo.md", DEL),
    ] {
        let err = plugin
            .dispatch(&vctx, path, req)
            .await
            .err()
            .unwrap_or_else(|| panic!("{what} 应报 NotFound"));
        assert!(matches!(err, VdfsError::NotFound(_)), "{what}：{err:?}");
    }
    assert!(
        !sub.join("foo.md").exists(),
        "被拒绝的写入不得在磁盘上落文件"
    );
}

// ==================== dispatch 请求形态（测试辅助） ====================

use crate::symbio_core::VdfsRequest;

const LIST: VdfsRequest = VdfsRequest::List {
    limit: None,
    before: None,
};
const STAT: VdfsRequest = VdfsRequest::Stat;
const READ: VdfsRequest = VdfsRequest::Read;
const DEL: VdfsRequest = VdfsRequest::Delete { recursive: false };
