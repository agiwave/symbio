//! `memory/vdfs.rs` 的单元测试 —— 挂载点的寻址、闸门与不可删除（两个作用域）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use crate::symbio_core::vdfs_context;
use crate::symbio_core::{
    PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest, WORKDIR,
};
use std::sync::Arc;
use tempfile::TempDir;

/// 在某宿主目录下造一个本插件实例（插件目录 = `<宿主目录>/memory`）
fn plugin_in(host: &std::path::Path) -> MemoryPlugin {
    MemoryPlugin::new(
        crate::symbio_core::PluginDir::at(host.join(PLUGIN_ID_MEMORY), PLUGIN_ID_MEMORY),
        super::super::config::MemoryConfig::default(),
    )
}

/// 构造 VDFS 调用上下文；`workdir` 为 `None` 即「没有工作区」
fn vctx(workdir: Option<&str>) -> VdfsContext {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    if let Some(w) = workdir {
        ctx.set(WORKDIR, w.to_string());
    }
    vdfs_context(&ctx)
}

// ==================== 寻址 ====================

#[tokio::test]
async fn root_node_leaves_its_name_to_the_caller() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    let root = p
        .dispatch(&vctx(None), "", VdfsRequest::Stat)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
    assert_eq!(root.name, "", "provider 不知道自己被挂在哪里");
    assert!(root.is_dir());
    assert_eq!(root.access, VdfsAccess::LIST_TRAVERSE);
    assert!(root.new_type.is_none(), "根下不可新建");
}

/// 根下列出**两份记忆**：智能体的恒在；工作区的选了工作区才在
#[tokio::test]
async fn root_lists_both_memory_files() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    let ws = TempDir::new().unwrap();

    // 没选工作区：只有智能体记忆
    let nodes = p
        .dispatch(
            &vctx(None),
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert_eq!(nodes.len(), 1, "没选工作区 → 只有智能体记忆");
    assert_eq!(nodes[0].node.name, MEMORY_FILE);
    assert_eq!(nodes[0].node.kind, PLUGIN_ID_MEMORY);

    // 选了工作区：两份都在
    let nodes = p
        .dispatch(
            &vctx(Some(ws.path().to_string_lossy().as_ref())),
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert_eq!(nodes.len(), 2, "智能体 + 工作区各一条");
    assert_eq!(nodes[0].node.name, MEMORY_FILE);
    assert_eq!(
        nodes[1].node.name,
        workspace::MOUNT_FILE,
        "工作区条目用挂载名"
    );
    assert_eq!(nodes[1].node.access, VdfsAccess::READ_WRITE);
    assert_eq!(
        nodes[1].node.effective_ext().as_deref(),
        Some("md"),
        "挂载名即呈现扩展名"
    );
    assert_eq!(nodes[1].node.kind, "workspace");
}

/// 工作区记忆没有工作区 = 资源不存在（不是「空文件」）
#[tokio::test]
async fn workspace_without_workdir_is_absent() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    let stat = p
        .dispatch(&vctx(None), workspace::MOUNT_FILE, VdfsRequest::Stat)
        .await;
    assert!(stat.is_err(), "没有工作区 → stat 如实 NotFound");
    let read = p
        .dispatch(&vctx(None), workspace::MOUNT_FILE, VdfsRequest::Read)
        .await;
    assert!(read.is_err(), "没有工作区 → read 无从谈起");
}

/// 根下没有子目录
#[tokio::test]
async fn no_subdirectories_under_the_root() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    let err = p
        .dispatch(
            &vctx(None),
            "sub",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap_err();
    assert!(format!("{err:?}").contains("没有子目录"), "{err:?}");
}

#[tokio::test]
async fn memory_file_is_readable_and_writable() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());

    // 还没写过 → 空串（不是错误）
    let text = p
        .dispatch(&vctx(None), MEMORY_FILE, VdfsRequest::Read)
        .await
        .unwrap()
        .into_read()
        .unwrap()
        .text
        .unwrap();
    assert_eq!(text, "");

    let resp = p
        .dispatch(
            &vctx(None),
            MEMORY_FILE,
            VdfsRequest::Write {
                content: VdfsContent::text("只改必要之处"),
            },
        )
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert!(resp.created, "首次写入 = 新建");

    assert_eq!(
        std::fs::read_to_string(tmp.path().join(MEMORY_FILE)).unwrap(),
        "只改必要之处",
        "写到宿主目录下，与落位规则同源"
    );
}

/// 工作区记忆经挂载名读写，物理落点是 `{workdir}/AGENTS.md`
#[tokio::test]
async fn workspace_file_roundtrips_to_the_physical_agents_file() {
    let tmp = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    let c = vctx(Some(ws.path().to_string_lossy().as_ref()));

    let first = p
        .dispatch(
            &c,
            workspace::MOUNT_FILE,
            VdfsRequest::Write {
                content: VdfsContent::text("偏好中文"),
            },
        )
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert!(first.created, "首次写入即创建");

    let again = p
        .dispatch(
            &c,
            workspace::MOUNT_FILE,
            VdfsRequest::Write {
                content: VdfsContent::text("偏好中文 + 简洁"),
            },
        )
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert!(!again.created, "覆盖写入不是创建");

    let content = p
        .dispatch(&c, workspace::MOUNT_FILE, VdfsRequest::Read)
        .await
        .unwrap()
        .into_read()
        .unwrap();
    assert_eq!(content.text.as_deref(), Some("偏好中文 + 简洁"));

    // 物理落点：工作区根的 AGENTS.md（行业约定），不是挂载名
    assert_eq!(
        std::fs::read_to_string(ws.path().join(workspace::WORK_MEMORY_FILE)).unwrap(),
        "偏好中文 + 简洁",
        "写的是物理文件，挂载名只是寻址面"
    );
    assert!(
        !tmp.path().join(workspace::MOUNT_FILE).exists(),
        "宿主目录下不得出现挂载名文件（那写错地方了）"
    );
}

/// 容量闸门在共享实现里（本插件不重复实现），但拒绝必须能透到调用方——两个作用域同规
#[tokio::test]
async fn writes_over_the_limit_are_rejected_for_both_scopes() {
    let tmp = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    let cfg = super::super::config::MemoryConfig {
        memory_max_bytes: 8,
        workspace_max_bytes: 8,
        ..super::super::config::MemoryConfig::default()
    };
    let p = MemoryPlugin::new(
        crate::symbio_core::PluginDir::at(tmp.path().join(PLUGIN_ID_MEMORY), PLUGIN_ID_MEMORY),
        cfg,
    );
    let c = vctx(Some(ws.path().to_string_lossy().as_ref()));

    for (path, label) in [(MEMORY_FILE, "智能体"), (workspace::MOUNT_FILE, "工作区")] {
        let err = p
            .dispatch(
                &c,
                path,
                VdfsRequest::Write {
                    content: VdfsContent::text("x".repeat(64)),
                },
            )
            .await
            .unwrap_err();
        assert!(
            format!("{err:?}").contains("超出容量上限"),
            "{label}超限必须拒绝（不静默截断）：{err:?}"
        );
    }
}

/// 二进制内容两个作用域都拒绝
#[tokio::test]
async fn binary_writes_are_rejected_for_both_scopes() {
    let tmp = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    let c = vctx(Some(ws.path().to_string_lossy().as_ref()));
    for path in [MEMORY_FILE, workspace::MOUNT_FILE] {
        let err = p
            .dispatch(
                &c,
                path,
                VdfsRequest::Write {
                    content: VdfsContent::binary("AA==", 1),
                },
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, VdfsError::Invalid(_)),
            "`{path}` 应拒绝二进制：{err:?}"
        );
    }
}

/// 记忆不可删除（累积型资源）：要清空就写入空内容——两个作用域同规
#[tokio::test]
async fn memory_cannot_be_deleted() {
    let tmp = TempDir::new().unwrap();
    let ws = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    let c = vctx(Some(ws.path().to_string_lossy().as_ref()));

    for path in [MEMORY_FILE, workspace::MOUNT_FILE] {
        let err = p
            .dispatch(&c, path, VdfsRequest::Delete { recursive: false })
            .await
            .unwrap_err();
        assert!(
            format!("{err:?}").contains("写入空内容"),
            "`{path}` 删除应被拒绝：{err:?}"
        );
    }

    let err = p
        .dispatch(&c, "", VdfsRequest::Delete { recursive: true })
        .await
        .unwrap_err();
    assert!(format!("{err:?}").contains("不可删除挂载点"), "{err:?}");
}

/// 配置文档不并列在列表里，但按**真实文件名**仍然可达
#[tokio::test]
async fn config_document_is_reachable_by_its_real_name() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    let node = p
        .dispatch(&vctx(None), PLUGIN_FILE, VdfsRequest::Stat)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
    assert_eq!(node.name, PLUGIN_FILE);
    assert_eq!(node.ext.as_deref(), Some(crate::symbio_core::VDFS_EXT_FORM));
}

/// 未知路径如实报未知（不为「看起来像记忆」的路径编造语义）
#[tokio::test]
async fn unknown_path_is_not_found() {
    let tmp = TempDir::new().unwrap();
    let p = plugin_in(tmp.path());
    let err = p
        .dispatch(&vctx(None), "nope.md", VdfsRequest::Read)
        .await
        .unwrap_err();
    assert!(format!("{err:?}").contains("未知路径"), "{err:?}");
}
