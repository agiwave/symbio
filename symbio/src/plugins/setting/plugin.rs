//! Setting 插件 - 设置管理
//!
//! ## 本插件只有两样东西：自有分区 + 插件配置清单
//!
//! 曾经这里挂着六个分区，其中四个（会话 / 本地工具 / 网络工具 / 开放接口）是
//! **别的插件的配置**：本插件硬编码了它们的插件名与配置路由前缀，读写经
//! `route_config` 代理到 `<prefix>/config/get|set`。那是横向耦合的典型——
//! 同一份配置有两个地址，定义与校验寄居在不是配置拥有者的插件里。
//!
//! 配置回到插件目录之后，四个分区各自回到拥有者名下（`<根>/<插件>/PLUGIN.yml`），
//! 本插件只保留两个**前端自持**的分区：
//!
//! - `appearance`（外观）：取值与保存都在前端 store（即时生效），VDFS 侧无数据；
//! - `about`（关于）：纯信息展示。
//!
//! 两者都不是「资源」，因此不实现 `read` / `write`；节点的 `ext` 即分区 id，
//! 前端按 `ext → 渲染器` 的纯 UI 映射回退到各自的专属 editor。
//!
//! ## 插件配置清单：列出来，但不代管
//!
//! 各插件的配置文档仍归各插件（同一份配置只有一个地址），但用户在设置页也应该
//! 看得到它们——所以本插件的 `list` 会把**各插件自己交出来的条目**列出来，
//! 排在自有分区**之前**（用户真正要动手的是前者）。条目由 `ConfigurableVisitor`
//! 通道在 `traverse` 广播中收集（见
//! `symbio_core::configurable`），**不是**本插件去反查插件目录、更不是硬编码清单：
//!
//! - 条目自带**真实地址**（`<插件>/PLUGIN.yml`）与呈现定义，所以点开就是那个插件
//!   的配置表单，读写照旧落在它自己的文件上；
//! - 本插件只补一个场景标签（`kind = setting`）——列表在哪儿，场景就是哪儿。

use crate::symbio_core::vdfs::host_ctx;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta,
    PluginPayload, CONFIG_VISITOR, PLUGIN_SETTING,
};
use std::sync::Arc;

use crate::symbio_core::vdfs::{
    self, DynVdfsProvider, VdfsAccess, VdfsContext, VdfsError, VdfsNode, VdfsProvider, VdfsResult,
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
        PluginMeta::new("setting", "设置")
            .with_description("本应用自身的设置，以及各插件配置文档的清单。")
            .with_version("0.1.0")
            .with_order(6)
            .with_icon("settings")
    }
}

#[async_trait::async_trait]
impl Plugin for SettingPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(
        self: Arc<Self>,
    ) -> Option<Arc<dyn crate::symbio_core::vdfs_provider::VdfsProvider>> {
        Some(self)
    }

    /// 本插件已无自有路由：分区清单与呈现由 `<根>/setting` 承担
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
///
/// ⚠️ **不给 `description`**：该字段只出现在**用户看的列表**里（`VdfsCard` 副标题），
/// 而「数据由前端自持、VDFS 侧无正文」是机制说明——`label`（「外观」「关于」）已足够，
/// 实现细节不往列表里放。
///
/// ⚠️ **显式声明无状态**（[`vdfs::VDFS_STATUS_NONE`]）：分区是**静态**的，没有
/// 「运行中 / 就绪」可言。节点 `status` 缺省是 `active`，不清掉就会在列表里画一个
/// 绿点——那是个**不存在的信息**（列表据此不渲染状态点，见
/// `docs/design/vdfs-frontend.md` §4.2）。
fn section_node(s: &SectionSpec) -> VdfsNode {
    let mut n = VdfsNode::file(s.id, s.label, VdfsAccess::READ);
    n.kind = PLUGIN_SETTING.to_string();
    n.ext = Some(s.id.to_string());
    n.status = vdfs::VDFS_STATUS_NONE.to_string();
    n
}

/// 「插件配置」条目：**各插件自己交出来的**（见 `symbio_core::configurable`）。
///
/// 条目本身就是那些插件的配置文档——标题、呈现定义、**真实地址**
/// （`<插件>/PLUGIN.yml`）都由拥有者给出，本插件只按列表口径补一个场景标签
/// （`kind = setting`，前端据此查图标 `setting:<条目名>`）。
///
/// **不代管读写**：条目的地址指向拥有者自己的文件，读 / 写 / 校验照旧走那条路径，
/// 同一份配置因此只有一个地址。
///
/// 声明由容器在广播 `TRAVERSE_AVAILABLE_TOOLS` 时收集并写回请求 ctx
/// （见 `plugins/composite/vdfs.rs::children_of`），所以这里既不需要反查插件目录，
/// 也不需要硬编码任何插件名；收集器缺失时（例如容器没参与本次请求）静默为空。
async fn config_entries(ctx: &VdfsContext) -> Vec<VdfsNode> {
    let Ok(host) = host_ctx(ctx) else {
        return Vec::new();
    };
    let Some(visitor) = host.get(CONFIG_VISITOR) else {
        return Vec::new();
    };
    visitor
        .list_configurables()
        .await
        .into_iter()
        .map(|mut n| {
            n.kind = PLUGIN_SETTING.to_string();
            // 配置条目同样是**静态**的（它就是一份文档，没有运行态可言）——
            // 与分区一致地显式声明无状态，设置列表因此整列没有状态点。
            n.status = vdfs::VDFS_STATUS_NONE.to_string();
            n
        })
        .collect()
}

#[async_trait::async_trait]
impl VdfsProvider for SettingPlugin {
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: vdfs::VdfsRequest,
    ) -> VdfsResult<vdfs::VdfsResponse> {
        match req {
            vdfs::VdfsRequest::List { .. } => {
                if !path.is_empty() {
                    return Err(VdfsError::not_found(format!(
                        "设置分区是叶子节点，没有子项：{path}"
                    )));
                }
                // 清单 = **各插件交出来的配置条目** + 自有分区。
                //
                // 顺序上插件配置在前、`appearance` / `about` 在后：前者是用户在设置页里真正要
                // 动手的东西，后者是应用自身的展示项，排尾不挡路。两段各自保序（插件段按声明
                // 注册顺序，分区段按 `SETTING_SECTIONS`）。
                let mut items = config_entries(ctx).await;
                items.extend(SETTING_SECTIONS.iter().map(section_node));
                Ok(vdfs::VdfsResponse::List(items))
            }
            vdfs::VdfsRequest::Stat => {
                if path.is_empty() {
                    // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
                    return Ok(vdfs::VdfsResponse::Stat(VdfsNode::dir(
                        "",
                        "设置",
                        VdfsAccess::LIST,
                    )));
                }
                section_of(path)
                    .map(section_node)
                    .ok_or_else(|| VdfsError::not_found(format!("未知设置分区：{path}")))
                    .map(vdfs::VdfsResponse::Stat)
            }
            // 本插件的分区都不是资源：取值在前端 store，VDFS 侧无正文可读
            vdfs::VdfsRequest::Read => {
                let s = section_of(path)
                    .ok_or_else(|| VdfsError::not_found(format!("未知设置分区：{path}")))?;
                Err(VdfsError::Forbidden(format!(
                    "分区 {} 的数据由前端状态自持，VDFS 侧无正文",
                    s.id
                )))
            }
            vdfs::VdfsRequest::Write { .. } => {
                let s = section_of(path)
                    .ok_or_else(|| VdfsError::not_found(format!("未知设置分区：{path}")))?;
                Err(VdfsError::Forbidden(format!(
                    "分区 {} 的数据由前端状态自持，VDFS 侧不可写",
                    s.id
                )))
            }
            _ => Err(VdfsError::not_found(format!("未知路径：{path}"))),
        }
    }
}

#[cfg(test)]
mod tests {
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
    async fn sections_are_leaves_without_new_types() {
        let p = SettingPlugin;
        assert_eq!(p.meta().root_access, VdfsAccess::LIST);
        assert!(p.new_types().await.is_empty());

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
            entry_of, vdfs::vdfs_context, ConfigFile, ConfigurableVisitor,
            DefaultConfigurableVisitor, PluginDir, SimpleRequest, CONFIG_VISITOR,
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
}
