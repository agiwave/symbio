//! `symbio/src/plugins/setting/plugin.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[tokio::test]
async fn list_returns_sections_in_declared_order() {
    let plugin = SettingPlugin;
    let items = plugin
        .dispatch(
            &vctx(),
            "",
            vdfs::VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();

    // 固定清单、按声明顺序（前端据此展示，不做二次排序）。
    // provider 返回的节点自带 `name`，全路径由分发层补挂载名后合成。
    let names: Vec<&str> = items.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["appearance", "about"]);

    // kind 标记为 setting；`name` 是地址段、`title` 是人读标签
    let first = &items[0];
    assert_eq!(first.kind, PLUGIN_SETTING);
    assert_eq!(first.name, "appearance");
    assert_eq!(first.title, "外观");

    // 前端自持分区：`ext` 即分区 id —— 前端按 `ext → 渲染器` 的纯 UI
    // 映射回退到专属 editor（外观设置 / 关于）
    assert_eq!(items[0].ext.as_deref(), Some("appearance"));
    assert_eq!(items[1].ext.as_deref(), Some("about"));
}

// ==================== VDFS provider ====================

fn vctx() -> VdfsContext {
    VdfsContext::empty()
}

/// provider 自描述：**不含挂载名**——挂载名由使用方在注册时选定
/// （见 `traverse` 里的 `register_vdfs_provider(PLUGIN_SETTING, ..)`）
#[tokio::test]
async fn vdfs_self_description_has_no_mount() {
    let p = SettingPlugin;
    let meta = p.meta();
    assert_eq!(meta.name, "设置");
    assert_eq!(meta.icon.as_deref(), Some("settings"));
    assert_eq!(meta.order, 6);

    let root = p
        .dispatch(&vctx(), "", vdfs::VdfsRequest::Stat)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
    assert_eq!(root.name, "", "provider 不知道自己的挂载名");
    assert!(root.is_dir());
}

/// 分区是叶子：不参与树遍历，也不接受新建
#[tokio::test]
async fn sections_are_leaves_without_new_type() {
    let p = SettingPlugin;
    assert_eq!(p.meta().root_access, VdfsAccess::LIST);
    assert!(p.root_new_type().await.is_none());

    let s = p
        .dispatch(&vctx(), "appearance", vdfs::VdfsRequest::Stat)
        .await
        .unwrap()
        .into_stat()
        .unwrap();
    assert!(!s.is_dir(), "分区是叶子文档");
    assert!(p
        .dispatch(
            &vctx(),
            "appearance",
            vdfs::VdfsRequest::List {
                limit: None,
                before: None
            },
        )
        .await
        .is_err());
}

/// 前端自持分区在 VDFS 侧无正文：读写都明确拒绝（而非静默返回空）
#[tokio::test]
async fn frontend_owned_sections_reject_read_and_write() {
    let p = SettingPlugin;
    assert!(matches!(
        p.dispatch(&vctx(), "appearance", vdfs::VdfsRequest::Read)
            .await,
        Err(VdfsError::Forbidden(_))
    ));
    let c = vdfs::VdfsContent::text("", "{}");
    assert!(matches!(
        p.dispatch(
            &vctx(),
            "appearance",
            vdfs::VdfsRequest::Write { content: c }
        )
        .await,
        Err(VdfsError::Forbidden(_))
    ));
    // 未知分区：NotFound 而非 Forbidden（区分「不存在」与「不可读写」）
    assert!(matches!(
        p.dispatch(&vctx(), "session", vdfs::VdfsRequest::Stat)
            .await,
        Err(VdfsError::NotFound(_))
    ));
}

// ==================== 插件配置清单 ====================

/// 造一份带可配置声明的请求 ctx（声明通常由容器在广播中收集，这里直接给）
async fn ctx_with_configs() -> VdfsContext {
    use crate::symbio_core::schemas::detail::DetailDefinition;
    use crate::symbio_core::{
        entry_of, vdfs::vdfs_context, ConfigFile, ConfigurableVisitor, DefaultConfigurableVisitor,
        PluginDir, SimpleRequest, CONFIG_VISITOR,
    };

    let visitor: Arc<dyn ConfigurableVisitor> = Arc::new(DefaultConfigurableVisitor::new());
    visitor
        .register_configurable(entry_of(&ConfigFile::new(
            PluginDir::at(std::env::temp_dir(), "web"),
            "网络工具",
            DetailDefinition::default(),
        )))
        .await;

    let host: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    host.set(CONFIG_VISITOR, visitor);
    vdfs_context(&host)
}

/// 各插件交出来的配置文档排在**前**，自有分区垫后
#[tokio::test]
async fn list_puts_declared_plugin_configs_before_the_sections() {
    let p = SettingPlugin;
    let items = p
        .dispatch(
            &ctx_with_configs().await,
            "",
            vdfs::VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();

    let names: Vec<&str> = items.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["web", "appearance", "about"]);

    let web = &items[0];
    assert_eq!(web.title, "网络工具");
    // 地址指向**拥有者自己的文件**：读写不经过本插件，同一份配置只有一个地址
    assert_eq!(web.path, "web/PLUGIN.yml");
    // 场景标签换成本列表的 kind（前端据此查图标 `setting:web`）
    assert_eq!(web.kind, PLUGIN_SETTING);
    assert_eq!(web.ext.as_deref(), Some("form"));
    assert!(!web.is_dir(), "条目是文档，不是目录");
}

/// 没有声明通道时只列自有分区——本通道是增益，缺了不影响本插件工作
#[tokio::test]
async fn list_without_declarations_is_just_the_sections() {
    let p = SettingPlugin;
    let items = p
        .dispatch(
            &vctx(),
            "",
            vdfs::VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();
    let names: Vec<&str> = items.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, vec!["appearance", "about"]);
}
