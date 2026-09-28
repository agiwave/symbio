//! `memory/plugin.rs` 的单元测试 —— 收集期交出的两样东西与**两个作用域**。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 重点钉住四件事：
//!
//! - 两个装配作用域是**同一个插件的两个实例**（目录不同 ⇒ 落位不同），不是两份实现；
//! - 智能体腿与工作区腿**各自注入**（段名不同、地址不同），并集不撞名；
//! - 工作区腿的**作用域闸门**：没选工作区什么都不注，开关关掉只影响注入；
//! - 挂载点与注入面**同时**交出（只给其一等于没有记忆）。

use super::*;
use crate::providers::DefaultToolVisitor;
use crate::symbio_core::{CapabilityVisitor, PluginInvokeRequestExt, VDFS_PARENT_ADDR};
use std::path::Path;
use tempfile::TempDir;

/// 构造带能力收集器的上下文；`workdir` 模拟会话选择的工作区（`None` = 没选工作区），
/// 父地址模拟容器转发时写入的当前父地址
fn ctx(
    workdir: Option<&str>,
    parent_addr: &str,
) -> (Arc<dyn PluginInvokeRequest>, Arc<DefaultToolVisitor>) {
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
    if let Some(w) = workdir {
        ctx.set(WORKDIR, w.to_string());
    }
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    ctx.set(VDFS_PARENT_ADDR, parent_addr.to_string());
    let visitor = Arc::new(DefaultToolVisitor::new());
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::clone(&visitor) as Arc<dyn CapabilityVisitor>,
    );
    (ctx, visitor)
}

/// 在某个**宿主目录**下造一个本插件实例（目录 = `<宿主目录>/memory`）
fn plugin_in(host: &Path) -> Arc<MemoryPlugin> {
    Arc::new(MemoryPlugin::new(
        crate::symbio_core::PluginDir::at(host.join(PLUGIN_ID_MEMORY), PLUGIN_ID_MEMORY),
        MemoryConfig::default(),
    ))
}

// ==================== 智能体腿（分形） ====================

/// 系统树实例：`<homedir>/memory` ⇒ 记忆落在 `<homedir>/AGENTS.md`
#[tokio::test]
async fn system_instance_injects_the_host_memory() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    std::fs::write(tmp.path().join(memory::MEMORY_FILE), "只改必要之处。").unwrap();

    let (ctx, visitor) = ctx(None, "@vfs/memory");
    p.traverse(String::new(), ctx).await.unwrap();

    let segs = visitor.list_system_prompts().await;
    assert_eq!(
        segs.len(),
        1,
        "没选工作区 + 有智能体记忆 → 只有智能体腿一段"
    );
    assert_eq!(segs[0].0, SEGMENT_NAME);
    assert!(segs[0].1.contains("只改必要之处。"));
    assert!(segs[0].1.contains("@vfs/memory/AGENTS.md"), "{}", segs[0].1);
    assert!(segs[0].1.contains("【智能体记忆】"));

    // 挂载点同时交出：只注入不给地址 = 只读记忆，永远长不大
    assert!(visitor.get_vdfs_provider(PLUGIN_ID_MEMORY).await.is_some());
}

/// 子树实例：`<agentdir>/memory` ⇒ 记忆落在 `<agentdir>/AGENTS.md`
///
/// 这就是「同一个插件的两个实例」：与上面那个用例唯一的差别是**目录**。
#[tokio::test]
async fn sub_agent_instance_injects_its_own_memory() {
    let tmp = TempDir::new().unwrap();
    let agent_dir = tmp.path().join("agent").join("reviewer");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let p = plugin_in(&agent_dir);
    std::fs::write(agent_dir.join(memory::MEMORY_FILE), "评审时先读测试。").unwrap();

    let (ctx, visitor) = ctx(None, "@vfs/agent/reviewer/memory");
    p.traverse(String::new(), ctx).await.unwrap();

    let segs = visitor.list_system_prompts().await;
    assert_eq!(segs.len(), 1);
    assert!(segs[0].1.contains("评审时先读测试。"));
    assert!(
        segs[0].1.contains("@vfs/agent/reviewer/memory/AGENTS.md"),
        "地址必须落在子智能体自己的挂载点下：{}",
        segs[0].1
    );
    // 系统树那份不受影响：两个实例各读各的文件
    assert!(!tmp.path().join(memory::MEMORY_FILE).exists());
}

/// 还没写过 → 静默跳过（「还没写过」不是故障，不该进收集期错误桶）
#[tokio::test]
async fn empty_memory_injects_nothing_but_keeps_the_mount() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());

    let (ctx, visitor) = ctx(None, "@vfs/memory");
    p.traverse(String::new(), ctx).await.unwrap();

    assert!(visitor.list_system_prompts().await.is_empty());
    assert!(
        visitor.get_vdfs_provider(PLUGIN_ID_MEMORY).await.is_some(),
        "挂载点的存在不依赖「有没有写过」"
    );
}

// ==================== 工作区腿（会话作用域） ====================

/// 没选工作区 → 工作区腿什么都不注（作用域闸门；智能体腿照常）
#[tokio::test]
async fn no_workspace_injects_no_workspace_memory() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    std::fs::write(tmp.path().join(memory::MEMORY_FILE), "智能体自己的记忆").unwrap();

    let (ctx, visitor) = ctx(None, "@vfs/memory");
    p.traverse(String::new(), ctx).await.unwrap();

    let segs = visitor.list_system_prompts().await;
    assert_eq!(
        segs.len(),
        1,
        "只有智能体腿: {:?}",
        segs.iter().map(|(n, _)| n)
    );
    assert_eq!(segs[0].0, SEGMENT_NAME, "工作区腿缺席");
}

/// 选了工作区但记忆还空着 → 工作区腿**照注**（带空内容提示，教会模型建立记忆）
#[tokio::test]
async fn empty_workspace_still_injects_the_empty_hint() {
    let tmp = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());

    let (ctx, visitor) = ctx(Some(ws.path().to_string_lossy().as_ref()), "@vfs/memory");
    p.traverse(String::new(), ctx).await.unwrap();

    let segs = visitor.list_system_prompts().await;
    assert_eq!(segs.len(), 1, "智能体记忆为空省略，工作区腿空文件也注入");
    assert_eq!(segs[0].0, workspace::SEGMENT_NAME);
    assert!(
        segs[0].1.contains("暂无内容"),
        "空记忆也要教会模型怎么建立记忆：{}",
        segs[0].1
    );
    assert!(
        segs[0].1.contains("@vfs/memory/WORKSPACE.md"),
        "地址是挂载名（不是物理名 AGENTS.md）：{}",
        segs[0].1
    );
}

/// 工作区记忆原样注入（正文 + 标题 + 挂载名地址）
#[tokio::test]
async fn workspace_memory_is_injected_verbatim() {
    let tmp = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    std::fs::write(
        ws.path().join(workspace::WORK_MEMORY_FILE),
        "本仓库用中文提交信息。",
    )
    .unwrap();
    let p = plugin_in(tmp.path());

    let (ctx, visitor) = ctx(Some(ws.path().to_string_lossy().as_ref()), "@vfs/memory");
    p.traverse(String::new(), ctx).await.unwrap();

    let segs = visitor.list_system_prompts().await;
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].0, workspace::SEGMENT_NAME);
    assert!(segs[0].1.contains("本仓库用中文提交信息。"));
    assert!(segs[0].1.contains("@vfs/memory/WORKSPACE.md"));
    assert!(segs[0].1.contains("【工作区记忆】"));
    assert!(
        segs[0].1.contains("智能体记忆"),
        "两个作用域同住一个挂载点，片段要点明互属关系：{}",
        segs[0].1
    );
}

/// 两条腿**同时**注入：段名不同、地址不同、互不顶替
#[tokio::test]
async fn both_legs_inject_together() {
    let tmp = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    std::fs::write(tmp.path().join(memory::MEMORY_FILE), "智能体记忆内容").unwrap();
    std::fs::write(
        ws.path().join(workspace::WORK_MEMORY_FILE),
        "工作区记忆内容",
    )
    .unwrap();
    let p = plugin_in(tmp.path());

    let (ctx, visitor) = ctx(Some(ws.path().to_string_lossy().as_ref()), "@vfs/memory");
    p.traverse(String::new(), ctx).await.unwrap();

    let segs = visitor.list_system_prompts().await;
    assert_eq!(
        segs.len(),
        2,
        "两条腿各一段: {:?}",
        segs.iter().map(|(n, _)| n)
    );
    let names: Vec<&str> = segs.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&SEGMENT_NAME), "智能体腿: {names:?}");
    assert!(
        names.contains(&workspace::SEGMENT_NAME),
        "工作区腿: {names:?}"
    );
    let agent_seg = &segs.iter().find(|(n, _)| n == SEGMENT_NAME).unwrap().1;
    let ws_seg = &segs
        .iter()
        .find(|(n, _)| n == workspace::SEGMENT_NAME)
        .unwrap()
        .1;
    assert!(agent_seg.contains("智能体记忆内容") && agent_seg.contains("@vfs/memory/AGENTS.md"));
    assert!(ws_seg.contains("工作区记忆内容") && ws_seg.contains("@vfs/memory/WORKSPACE.md"));
}

/// 开关关掉只影响**注入**：文件与挂载点照旧可用（继承自原 work 插件的语义）
#[tokio::test]
async fn disabled_workspace_skips_injection_but_keeps_the_mount() {
    let tmp = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    std::fs::write(
        ws.path().join(workspace::WORK_MEMORY_FILE),
        "会被忽略的内容",
    )
    .unwrap();
    let cfg = MemoryConfig {
        workspace_enabled: false,
        ..MemoryConfig::default()
    };
    let p = Arc::new(MemoryPlugin::new(
        crate::symbio_core::PluginDir::at(tmp.path().join(PLUGIN_ID_MEMORY), PLUGIN_ID_MEMORY),
        cfg,
    ));

    let (ctx, visitor) = ctx(Some(ws.path().to_string_lossy().as_ref()), "@vfs/memory");
    p.traverse(String::new(), ctx).await.unwrap();

    assert!(visitor.list_system_prompts().await.is_empty());
    assert!(
        visitor.get_vdfs_provider(PLUGIN_ID_MEMORY).await.is_some(),
        "关掉注入不等于关掉编辑面"
    );
}

// ==================== 无自有路由 ====================

#[tokio::test]
async fn plugin_has_no_own_routes() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));
    ctx.set(PATH, "memory/whatever".to_string());
    let err = p.route(ctx).await.unwrap_err();
    assert!(
        format!("{err:?}").contains("VDFS"),
        "错误信息要指引到 VDFS：{err:?}"
    );
}
