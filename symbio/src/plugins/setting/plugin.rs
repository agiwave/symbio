//! Setting 插件 - 设置管理
//!
//! ## 本插件现在只剩「本应用自身的设置」
//!
//! 曾经这里挂着六个分区，其中四个（会话 / 本地工具 / 网络工具 / 开放接口）是
//! **别的插件的配置**：本插件硬编码了它们的插件名与配置路由前缀，读写经
//! `route_config` 代理到 `<prefix>/config/get|set`。那是横向耦合的典型——
//! 同一份配置有两个地址，定义与校验寄居在不是配置拥有者的插件里。
//!
//! 配置地址化之后，四个分区各自回到拥有者名下（`.vdfs/<插件>/配置`），
//! 本插件只保留两个**前端自持**的分区：
//!
//! - `appearance`（外观）：取值与保存都在前端 store（即时生效），VDFS 侧无数据；
//! - `about`（关于）：纯信息展示。
//!
//! 两者都不是「资源」，因此不实现 `read` / `write`；节点的 `ext` 即分区 id，
//! 前端按 `ext → 渲染器` 的纯 UI 映射回退到各自的专属 editor。

use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta, PluginPayload,
    PLUGIN_SETTING,
};
use std::sync::Arc;

use crate::symbio_core::vdfs::{
    self, DynVdfsProvider, VdfsAccess, VdfsContext, VdfsError, VdfsNode, VdfsProvider, VdfsResult,
    VdfsWriteResponse,
};

/// 无状态：本插件的全部内容就是一份**固定分区清单**（两个前端自持分区），
/// 既无自有路由、也无配置可存，因此不需要持有任何东西。
#[derive(Clone, Default)]
pub struct SettingPlugin;

impl SettingPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(SettingPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("setting", "系统设置")
            .with_description("设置管理插件")
            .with_version("0.1.0")
    }
}

#[async_trait::async_trait]
impl Plugin for SettingPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    /// 本插件已无自有路由：分区清单与呈现由 `.vdfs/setting` 承担
    /// （`setting/list` / `setting/get` 更早已随 VDFS 下线）。
    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!("未知路径: {path}")))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        // 与工具共用同一次能力广播，把自己注册为一份 VDFS 资源。
        // 挂载名由**使用方**（此处即本插件）选定：约定用插件名（`PLUGIN_SETTING`），
        // 插件名在宿主内唯一，天然就是合格的挂载名。provider 自身不含此概念。
        // 会话链路（LLM 工具）与前端链路因此拿到同一份 (挂载名, 实现) 集合。
        if let Some(visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
            let me: DynVdfsProvider = self.clone();
            visitor.register_vdfs_provider(PLUGIN_SETTING, me).await;
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_SETTING, SettingPlugin::build, dyn Plugin);

// ==================== 设置分区清单（单一真相源） ====================

/// 设置分区（固定清单）是 VDFS 挂载点声明与前端 editor 的**同一份真相源**。
///
/// `id` 同时作为前端 editor 的「扩展名」（`ext`）：本插件的分区都不是资源，
/// 因此 `ext` 即分区 id，前端按 `ext → 渲染器` 的纯 UI 映射回退到专属 editor。
struct SectionSpec {
    id: &'static str,
    label: &'static str,
}

const SETTING_SECTIONS: [SectionSpec; 2] = [
    SectionSpec {
        id: "appearance",
        label: "外观",
    },
    SectionSpec {
        id: "about",
        label: "关于",
    },
];

/// 按 id 取分区声明
fn section_of(id: &str) -> Option<&'static SectionSpec> {
    SETTING_SECTIONS.iter().find(|s| s.id == id)
}

/// 分区节点。
///
/// 数据由前端状态自持（外观即时生效 / 关于纯展示），VDFS 侧无正文，
/// 因此 `access` 只声明 `r`，且 `read` / `write` 恒为 `Forbidden`——
/// 前端的专属 editor 不读 VDFS。
fn section_node(s: &SectionSpec) -> VdfsNode {
    let mut n = VdfsNode::file(s.id, s.label, VdfsAccess::READ);
    n.kind = PLUGIN_SETTING.to_string();
    n.ext = Some(s.id.to_string());
    n.description = Some("该分区数据由前端状态自持，VDFS 侧无正文".to_string());
    n
}

#[async_trait::async_trait]
impl VdfsProvider for SettingPlugin {
    fn label(&self) -> Option<&str> {
        Some("设置")
    }

    fn description(&self) -> Option<&str> {
        Some("本应用自身的设置（外观 / 关于）。插件配置在各插件自己的挂载点下。")
    }

    fn order(&self) -> i32 {
        6
    }

    fn icon(&self) -> Option<&str> {
        Some("settings")
    }

    /// 分区清单固定、每一项都是叶子：可列，但不可递归遍历
    fn root_access(&self) -> VdfsAccess {
        VdfsAccess::LIST
    }

    async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        if !path.is_empty() {
            return Err(VdfsError::not_found(format!(
                "设置分区是叶子节点，没有子项：{path}"
            )));
        }
        Ok(SETTING_SECTIONS.iter().map(section_node).collect())
    }

    async fn stat(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        if path.is_empty() {
            // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
            return Ok(VdfsNode::dir("", "设置", self.root_access()));
        }
        section_of(path)
            .map(section_node)
            .ok_or_else(|| VdfsError::not_found(format!("未知设置分区：{path}")))
    }

    /// 本插件的分区都不是资源：取值在前端 store，VDFS 侧无正文可读
    async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<vdfs::VdfsContent> {
        let s = section_of(path)
            .ok_or_else(|| VdfsError::not_found(format!("未知设置分区：{path}")))?;
        Err(VdfsError::Forbidden(format!(
            "分区 {} 的数据由前端状态自持，VDFS 侧无正文",
            s.id
        )))
    }

    async fn write(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        _content: &vdfs::VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let s = section_of(path)
            .ok_or_else(|| VdfsError::not_found(format!("未知设置分区：{path}")))?;
        Err(VdfsError::Forbidden(format!(
            "分区 {} 的数据由前端状态自持，VDFS 侧不可写",
            s.id
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_returns_sections_in_declared_order() {
        let plugin = SettingPlugin::default();
        let items = plugin.list(&vctx(), "").await.unwrap();

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
        let p = SettingPlugin::default();
        assert_eq!(p.label(), Some("设置"));
        assert_eq!(p.icon(), Some("settings"));
        assert_eq!(p.order(), 6);

        let root = p.stat(&vctx(), "").await.unwrap();
        assert_eq!(root.name, "", "provider 不知道自己的挂载名");
        assert!(root.is_dir());
    }

    /// 分区是叶子：不参与树遍历，也不接受新建
    #[tokio::test]
    async fn sections_are_leaves_without_new_types() {
        let p = SettingPlugin::default();
        assert_eq!(p.root_access(), VdfsAccess::LIST);
        assert!(p.root_new_types().is_empty());

        let s = p.stat(&vctx(), "appearance").await.unwrap();
        assert!(!s.is_dir(), "分区是叶子文档");
        assert!(p.list(&vctx(), "appearance").await.is_err());
    }

    /// 前端自持分区在 VDFS 侧无正文：读写都明确拒绝（而非静默返回空）
    #[tokio::test]
    async fn frontend_owned_sections_reject_read_and_write() {
        let p = SettingPlugin::default();
        assert!(matches!(
            p.read(&vctx(), "appearance").await,
            Err(VdfsError::Forbidden(_))
        ));
        let c = vdfs::VdfsContent::text("", "{}");
        assert!(matches!(
            p.write(&vctx(), "appearance", &c).await,
            Err(VdfsError::Forbidden(_))
        ));
        // 未知分区：NotFound 而非 Forbidden（区分「不存在」与「不可读写」）
        assert!(matches!(
            p.stat(&vctx(), "session").await,
            Err(VdfsError::NotFound(_))
        ));
    }
}
