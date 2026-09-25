//! 插件管理插件 —— 本智能体的**插件集合**：启用 / 停用 / 添加 / 卸载，以及各插件的配置
//!
//! ## 定位
//!
//! 「这个智能体由哪些插件组成」是**容器的事实**：插件根下一层目录 = 一个插件
//! （见 `symbio_core::plugin_dir`）。本插件是那件事在界面上的**唯一门面**——它把容器
//! 产出的注册表条目列成一张清单，并把「启用 / 停用 / 添加 / 卸载」转发回容器执行。
//!
//! | 谁 | 干什么 |
//! |---|---|
//! | 容器（`composite`） | 注册表的**权威**：扫目录、三条判据（合格 / 必需 / 启用）、启停与增删 |
//! | 本插件 | 注册表的**视图**：合成条目节点与动作、转发动词；**不自己扫目录** |
//!
//! 「有哪些插件」因此只有一份实现。本插件不持有任何状态，也不认识任何具体插件——
//! 它只持所在容器的 VDFS 视图（注册表动词的入口，见 `Composite::get_vfs_provider`）。
//!
//! ## 一个条目 = 一个插件 = 一份配置
//!
//! 条目**不是**第二种东西：它就是那个插件的配置，只是入口挂在本插件子树里
//! （`<插件管理>/<插件名>`），读 / 写转发到容器根下的 `<插件名>/PLUGIN.yml`——
//! 同一份配置仍然只有一个**文件**、一份**定义**（拥有者给的那份，见
//! `symbio_core::configurable`），本插件只转地址，不代管。
//!
//! 为什么需要这个入口：装配动作（启用 / 停用 / 卸载）必须出现在**能表达装配态**的
//! 节点上。而配置条目的真实地址（`<插件名>/PLUGIN.yml`）落在插件自己的资源域里——
//! 在那儿摆装配按钮，等于让每个插件都长出一套「我该不该存在」的界面。
//!
//! ## 动作按装配态显隐：投影键
//!
//! 三个动作各有前提（`启用` 只在停用时、`停用` 只在可停用时、`卸载` 只在非必需时），
//! 而 `DetailAction.when` 只能对**表单模型**求值（见 `schemas/detail.rs`）。因此 `read`
//! 会在配置 JSON 上叠几个投影键（`plugin_enabled` / `plugin_required` /
//! `plugin_can_disable` / `plugin_version`，见 `plugin_dir` 的同名常量），条件据此判定。
//!
//! 三个动作**恒在定义里**（只靠条件显隐），不按当下状态增删：定义是节点的一部分，
//! 而状态一变（见下）调用方只会**重读正文**、不会重新取节点——按状态增删动作会让
//! 按钮停在旧状态上（停用之后连「启用」都不出现）。条件求值用的是重读回来的模型，
//! 所以显隐永远跟得上。
//!
//! ## 装配态一变就广播
//!
//! `act` / `install` / `uninstall` 成功后各广播一次变更（[`notify_change`]）：条目集合
//! 与当前选中项的**动作集**都变了，订阅方据此重拉并重读当前项（前端 `useVdfs` 的既有
//! 收敛路径），按钮因此不会停在旧状态上。订阅走 [`watch_changes`]（本插件是自管变更源）。

use crate::symbio_core::schemas::detail::{
    DetailAction, DetailCondition, DetailDefinition, DetailField, DetailSection,
};
use crate::symbio_core::vdfs::{
    host_ctx, notify_change, unwatch_changes, watch_changes, DynVdfsProvider,
};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsContent, VdfsContext, VdfsError, VdfsItem, VdfsNode, VdfsProvider, VdfsRequest,
    VdfsResponse, VdfsResult, VDFS_ACTION_DISABLE, VDFS_ACTION_ENABLE, VDFS_ACTION_PLUGINS,
    VDFS_EXT_FORM, VDFS_PLUGINS_FIELD, VDFS_PLUGIN_NAME_FIELD, VDFS_STATUS_ACTIVE,
    VDFS_STATUS_DISABLED,
};
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginEntry, PluginError, PluginMeta,
    PluginPayload, CAPABILITY_VISITOR, CONFIG_VISITOR, KEY_CAN_DISABLE, KEY_ENABLED, KEY_NAME,
    KEY_PROVIDER, KEY_REQUIRED, KEY_VERSION, PLUGIN_FILE, PLUGIN_MANAGER, UNDISABLABLE_PLUGINS,
};
use serde_json::Value;
use std::sync::Arc;

/// 自有分区（固定清单）：数据由**前端状态**自持，VDFS 侧无正文。
///
/// 这两个分区曾经和插件配置并列在设置页里；插件配置回到拥有者名下之后，它们就是
/// 本插件剩下的全部自有内容——`id` 同时是前端 editor 的「扩展名」（`ext`），
/// 前端按 `ext → 渲染器` 的纯 UI 映射回退到专属面板。
struct SectionSpec {
    id: &'static str,
    label: &'static str,
}

const OWN_SECTIONS: [SectionSpec; 2] = [
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
    OWN_SECTIONS.iter().find(|s| s.id == id)
}

/// 分区节点：静态、无正文、只读（数据在前端 store）。
///
/// ⚠️ **显式声明无状态**（[`VDFS_STATUS_NONE`]）：分区是静态的，没有「运行中 / 就绪」
/// 可言。节点 `status` 缺省是 `active`，不清掉就会在列表里画一个绿点——那是个不存在
/// 的信息（见 `docs/design/vdfs-frontend.md` §4.2）。
///
/// [`VDFS_STATUS_NONE`]: crate::symbio_core::vdfs_provider::VDFS_STATUS_NONE
fn section_node(s: &SectionSpec) -> VdfsNode {
    let mut n = VdfsNode::file(s.id, s.label, VdfsAccess::READ);
    n.kind = PLUGIN_MANAGER.to_string();
    n.ext = Some(s.id.to_string());
    n.status = crate::symbio_core::vdfs_provider::VDFS_STATUS_NONE.to_string();
    n
}

/// 条件：`key == value`
fn cond_equals(key: &str, value: Value) -> DetailCondition {
    DetailCondition {
        key: key.to_string(),
        equals: Some(value),
        ..Default::default()
    }
}

/// 条件：全部成立（`all` 存在时其余字段被忽略，故只填 `all`）
fn cond_all(conds: Vec<DetailCondition>) -> DetailCondition {
    DetailCondition {
        all: conds,
        ..Default::default()
    }
}

/// 条目名：本目录下的**单段**路径。
///
/// 插件名 = 插件根下的一层目录名（见 `plugin_dir`），不含 `/`——多段路径在本子树里
/// 没有对应的东西，如实报「未知插件」而不是硬拆首段。
fn leaf(path: &str) -> VdfsResult<&str> {
    let p = path.trim_matches('/');
    if p.is_empty() || p.contains('/') {
        return Err(VdfsError::not_found(format!("未知插件：{path}")));
    }
    Ok(p)
}

/// 条目节点：一个插件在界面上的样子
///
/// 有配置文档的插件沿用**拥有者给的呈现定义**（标题 / 表单定义都在里面），本插件只
/// 追加装配动作；没有配置文档的（如 `telegram`）合成一张只读概览——否则它在列表里
/// 点开是一片空白，连装配按钮都没有落脚处。
///
/// `access` 恒为 `rw`：条目的正文（那份配置）是可改的，与它是不是必需无关——「不可
/// 删除」由动作的条件表达（见 [`management_actions`]），不靠把节点降成只读。
fn entry_node(entry: &PluginEntry, config: Option<VdfsItem>) -> VdfsNode {
    let mut definition = match config.as_ref() {
        Some(item) => item
            .node
            .schema
            .clone()
            .and_then(|v| serde_json::from_value::<DetailDefinition>(v).ok())
            .unwrap_or_else(|| overview_definition(entry)),
        None => overview_definition(entry),
    };
    definition.actions.extend(management_actions());

    // 标题的回落链：配置声明的标题 → 插件元数据的名字 → 目录名。
    // 停用的插件**未被构造**，后两项都可能为空（见 `PluginEntry`），因此不能只取一处。
    let mut title = config
        .as_ref()
        .map(|c| c.node.title.clone())
        .unwrap_or_default();
    if title.is_empty() {
        title = entry.title.clone();
    }
    if title.is_empty() {
        title = entry.name.clone();
    }

    let mut n = VdfsNode::file(entry.name.clone(), title, VdfsAccess::READ_WRITE);
    n.kind = PLUGIN_MANAGER.to_string();
    n.ext = Some(VDFS_EXT_FORM.to_string());
    n.description = config
        .as_ref()
        .and_then(|c| c.node.description.clone())
        .or_else(|| entry.description.clone());
    n.status = if entry.enabled {
        VDFS_STATUS_ACTIVE
    } else {
        VDFS_STATUS_DISABLED
    }
    .to_string();
    n.schema = serde_json::to_value(&definition).ok();
    n
}

/// 无配置文档的插件的条目定义：一张只读概览。
///
/// `binding = info` 表示**没有可保存的东西**（它不是资源）——条目上的动作是装配动作，
/// 与保存无关。三个 `static` 字段的值来自 `read` 注入的投影键。
fn overview_definition(entry: &PluginEntry) -> DetailDefinition {
    let static_field = |key: &str, label: &str| DetailField {
        key: key.to_string(),
        label: label.to_string(),
        widget: "static".to_string(),
        ..Default::default()
    };
    DetailDefinition {
        binding: "info".to_string(),
        title_fallback: Some(entry.name.clone()),
        sections: vec![DetailSection {
            title: None,
            collapsed: false,
            fields: vec![
                static_field(KEY_NAME, "插件名"),
                static_field(KEY_PROVIDER, "插件工厂"),
                static_field(KEY_VERSION, "版本"),
            ],
        }],
        ..Default::default()
    }
}

/// 条目上的**装配动作**：启用 / 停用 / 卸载。
///
/// 显隐判据全部取自 `read` 注入的投影键（见模块文档「动作按装配态显隐」）。
///
/// `delete` 这个 id 是刻意的：它就是前端**机制动作**的同名动作（渲染器 `case 'delete'`
/// 走统一删除通道），因此声明它既换掉了兜底文案（「删除」→「卸载」），也**顶掉了**
/// 兜底那一个——必需插件于是连按钮都不出现，而不是点了才报错。
fn management_actions() -> Vec<DetailAction> {
    vec![
        // 启用：只在停用时可见
        DetailAction {
            id: VDFS_ACTION_ENABLE.to_string(),
            label: "启用".to_string(),
            style: "primary".to_string(),
            when: Some(cond_equals(KEY_ENABLED, Value::Bool(false))),
            busy_label: Some("启用中…".to_string()),
            ..Default::default()
        },
        // 停用：已启用**且**允许停用时可见（界面底座插件拒绝停用，见 `UNDISABLABLE_PLUGINS`）
        DetailAction {
            id: VDFS_ACTION_DISABLE.to_string(),
            label: "停用".to_string(),
            style: "secondary".to_string(),
            when: Some(cond_all(vec![
                cond_equals(KEY_ENABLED, Value::Bool(true)),
                cond_equals(KEY_CAN_DISABLE, Value::Bool(true)),
            ])),
            busy_label: Some("停用中…".to_string()),
            ..Default::default()
        },
        // 卸载：非必需时可见（必需插件「可以停用，但不可删除」）
        DetailAction {
            id: "delete".to_string(),
            label: "卸载".to_string(),
            style: "danger".to_string(),
            when: Some(cond_equals(KEY_REQUIRED, Value::Bool(false))),
            busy_label: Some("卸载中…".to_string()),
            ..Default::default()
        },
    ]
}

/// 无状态：注册表在容器那里，本插件只持它的 VDFS 视图（转发动词用）。
pub struct PluginManagerPlugin {
    /// 所在容器的 VDFS 视图。
    ///
    /// 装配期经 `ctx.parent()` 取一次——子插件回到容器是既有通道（`hook` / `local`
    /// 取父引用同款），取回它的 provider 即注册表动词的入口（见
    /// `Composite::get_vfs_provider`）。取不到时（未装配 / 单测）为 `None`，
    /// 注册表类操作如实报错，条目合成与分区仍照常可用。
    registry_view: Option<DynVdfsProvider>,
}

impl PluginManagerPlugin {
    /// 直接给定容器的 VDFS 视图（测试用：不必真的装配一个容器）
    fn with_registry_view(registry_view: Option<DynVdfsProvider>) -> Self {
        Self { registry_view }
    }

    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let registry_view = ctx
            .parent()
            .and_then(|w| w.upgrade())
            .and_then(|p| p.get_vfs_provider());
        Arc::new(Self::with_registry_view(registry_view)) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_MANAGER, "插件管理")
            .with_description("本智能体的插件：启用 / 停用 / 添加 / 卸载，以及各插件的配置。")
            .with_version("0.1.0")
            .with_order(6)
            .with_icon("settings")
    }

    // ==================== 转发：本插件是视图，动词在容器 ====================

    /// 把一次请求转发给**所在容器的根**。
    ///
    /// 转发前把当前父地址**清空**：插件根就是地址空间的根，容器据此为每个子目录续出
    /// `<根>/<插件>`（见 `symbio_core::vdfs::descend_addr` 的「顶层为空，落到静态声明
    /// 的根」）。带着本插件的挂载点转过去，容器的子上下文会算成
    /// `<根>/plugin_manager/<插件>`——那是错的。
    async fn forward(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        let view = self
            .registry_view
            .as_ref()
            .ok_or_else(|| VdfsError::internal("插件管理插件不在容器中：无法访问插件注册表"))?;
        let at_root = ctx.clone().with_parent_addr(String::new());
        view.dispatch(&at_root, path, req).await
    }

    /// 容器的注册表条目（经 `Action(plugins)`）——本插件**不自己扫目录**。
    async fn entries(&self, ctx: &VdfsContext) -> VdfsResult<Vec<PluginEntry>> {
        let resp = self
            .forward(
                ctx,
                "",
                VdfsRequest::Action {
                    action: VDFS_ACTION_PLUGINS.to_string(),
                    payload: None,
                },
            )
            .await?;
        let r = resp
            .into_action()
            .ok_or_else(|| VdfsError::internal("容器对注册表动词的响应类型不匹配"))?;
        if !r.ok {
            return Err(VdfsError::internal(r.message));
        }
        let list = r
            .data
            .and_then(|d| d.get(VDFS_PLUGINS_FIELD).cloned())
            .ok_or_else(|| {
                VdfsError::internal(format!(
                    "容器未在 {VDFS_ACTION_PLUGINS} 回包里给出 {VDFS_PLUGINS_FIELD} 字段"
                ))
            })?;
        serde_json::from_value(list)
            .map_err(|e| VdfsError::internal(format!("插件注册表条目解析失败：{e}")))
    }

    /// 单个条目（不存在 → `NotFound`）
    async fn entry_of(&self, ctx: &VdfsContext, name: &str) -> VdfsResult<PluginEntry> {
        self.entries(ctx)
            .await?
            .into_iter()
            .find(|e| e.name == name)
            .ok_or_else(|| VdfsError::not_found(format!("未找到插件：{name}")))
    }

    /// 各插件自己交出来的配置声明（`CONFIG_VISITOR` 通道，容器广播时收集）。
    ///
    /// 条目自带**真实地址**与呈现定义，本插件只按插件名对上号——「这份配置长什么样」
    /// 只有拥有者说得出来（见 `symbio_core::configurable`）。收集器缺失时（例如容器
    /// 没参与本次请求）静默为空：没有它条目照样列得出，只是少一份表单定义。
    async fn config_entries(ctx: &VdfsContext) -> Vec<VdfsItem> {
        let Ok(host) = host_ctx(ctx) else {
            return Vec::new();
        };
        let Some(visitor) = host.get(CONFIG_VISITOR) else {
            return Vec::new();
        };
        visitor.list_configurables().await
    }

    /// 某插件的配置声明（按条目名对号）
    async fn config_of(ctx: &VdfsContext, name: &str) -> Option<VdfsItem> {
        Self::config_entries(ctx)
            .await
            .into_iter()
            .find(|c| c.node.name == name)
    }

    // ==================== 节点合成 ====================

    /// 本插件的根：标题 + **可新建**（= 添加插件）
    ///
    /// 可新建类型**原样转发**容器的声明（见 `CompositeVdfs::install_new_type`）：
    /// 安装表单长什么样、候选有哪些，只有真正执行安装的那一层说得出来。转发失败
    /// （容器不可达）时按「不可新建」处理——少一个入口，好过给一个必然报错的入口。
    async fn root_node(&self, ctx: &VdfsContext) -> VdfsNode {
        let mut n = VdfsNode::dir("", "插件管理", VdfsAccess::LIST);
        n.description = Some("本智能体的插件，以及各插件的配置".to_string());
        n.new_type = match self.forward(ctx, "", VdfsRequest::Stat).await {
            Ok(resp) => resp.into_stat().and_then(|root| root.new_type),
            Err(_) => None,
        };
        n
    }

    /// 清单 = **各插件的条目** + 自有分区。
    ///
    /// 顺序上插件在前、自有分区在后：前者是用户在管理页里真正要动手的东西，后者是
    /// 应用自身的展示项，排尾不挡路。两段各自保序（插件段按容器给的 `(order, name)`，
    /// 分区段按 [`OWN_SECTIONS`]）。
    async fn list_items(&self, ctx: &VdfsContext) -> VdfsResult<Vec<VdfsItem>> {
        let entries = self.entries(ctx).await?;
        let configs = Self::config_entries(ctx).await;
        let mut out: Vec<VdfsItem> = Vec::with_capacity(entries.len() + OWN_SECTIONS.len());
        for entry in &entries {
            let config = configs.iter().find(|c| c.node.name == entry.name).cloned();
            out.push(VdfsItem::new(entry_node(entry, config)));
        }
        out.extend(OWN_SECTIONS.iter().map(|s| VdfsItem::new(section_node(s))));
        Ok(out)
    }

    // ==================== 条目的读写与装配动作 ====================

    /// 读条目：转发到 `<插件名>/PLUGIN.yml`，并在结果上叠**投影键**。
    ///
    /// 没有配置文档的插件（容器侧报 `NotFound` / 未实现 VDFS）得到一份只含投影键的
    /// 模型——条目详情因此照常渲染出概览与装配动作，而不是一片错误。
    async fn read_entry(&self, ctx: &VdfsContext, entry: &PluginEntry) -> VdfsResult<VdfsResponse> {
        let rel = format!("{}/{PLUGIN_FILE}", entry.name);
        let mut model = match self.forward(ctx, &rel, VdfsRequest::Read).await {
            Ok(resp) => {
                let content = resp
                    .into_read()
                    .ok_or_else(|| VdfsError::internal("容器对读取的响应类型不匹配"))?;
                serde_json::from_str::<Value>(content.as_text().unwrap_or_default())
                    .unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
            }
            Err(VdfsError::NotFound(_)) | Err(VdfsError::NotImplemented) => {
                Value::Object(serde_json::Map::new())
            }
            Err(e) => return Err(e),
        };
        let obj = model
            .as_object_mut()
            .ok_or_else(|| VdfsError::internal("插件配置的顶层必须是映射"))?;
        obj.insert(KEY_NAME.to_string(), Value::String(entry.name.clone()));
        obj.insert(
            KEY_PROVIDER.to_string(),
            Value::String(entry.provider.clone()),
        );
        obj.insert(KEY_ENABLED.to_string(), Value::Bool(entry.enabled));
        obj.insert(KEY_REQUIRED.to_string(), Value::Bool(entry.required));
        obj.insert(
            KEY_CAN_DISABLE.to_string(),
            Value::Bool(!UNDISABLABLE_PLUGINS.contains(&entry.name.as_str())),
        );
        obj.insert(
            KEY_VERSION.to_string(),
            Value::String(entry.version.clone().unwrap_or_default()),
        );

        let text = serde_json::to_string_pretty(&model)
            .map_err(|e| VdfsError::internal(format!("插件配置序列化失败：{e}")))?;
        Ok(VdfsResponse::Read(
            VdfsContent::text(text).with_mime("application/json"),
        ))
    }

    /// 写条目：转发到 `<插件名>/PLUGIN.yml`（**具名写**，`create` 无意义）。
    ///
    /// 校验归拥有者的定义（`ConfigFile::apply`），本插件不碰内容——它只把地址接过去。
    async fn write_entry(
        &self,
        ctx: &VdfsContext,
        name: &str,
        content: VdfsContent,
    ) -> VdfsResult<VdfsResponse> {
        let rel = format!("{name}/{PLUGIN_FILE}");
        self.forward(ctx, &rel, VdfsRequest::Write { content })
            .await
    }

    /// 装配动作：启停（转发容器的注册表动词，插件名随载荷给）。
    ///
    /// 名字走**载荷**而不是路径：容器根上的注册表动词作用在「注册表里的某一项」上，
    /// 而那一项在资源树里可能根本没有位置（停用的插件不在资源树里，见
    /// `composite/vdfs.rs` 的「两个视图」）。
    async fn act(
        &self,
        ctx: &VdfsContext,
        name: &str,
        action: String,
        payload: Option<Value>,
    ) -> VdfsResult<VdfsResponse> {
        let mut fields = payload
            .and_then(|v| match v {
                Value::Object(m) => Some(m),
                _ => None,
            })
            .unwrap_or_default();
        fields.insert(
            VDFS_PLUGIN_NAME_FIELD.to_string(),
            Value::String(name.to_string()),
        );
        let resp = self
            .forward(
                ctx,
                "",
                VdfsRequest::Action {
                    action,
                    payload: Some(Value::Object(fields)),
                },
            )
            .await?;
        self.announce(name);
        Ok(resp)
    }

    /// 卸载：转发到容器的 `Delete(<插件名>)`——「删掉这个目录」与「卸载这个插件」
    /// 是同一件事（必需插件由容器拒绝）。
    async fn uninstall(
        &self,
        ctx: &VdfsContext,
        name: &str,
        recursive: bool,
    ) -> VdfsResult<VdfsResponse> {
        self.forward(ctx, name, VdfsRequest::Delete { recursive })
            .await?;
        self.announce(name);
        Ok(VdfsResponse::Unit)
    }

    /// 安装：转发到**容器根上的写**（根上的写就是安装，见
    /// `CompositeVdfs::install_plugin`）。
    ///
    /// 新插件的名字由容器生成并经 [`VdfsWriteResponse::name`] 交回——与「新建一项
    /// 资源」同形，调用方本来就知道它请求的是哪个目录，地址不必由 provider 代拼。
    ///
    /// [`VdfsWriteResponse::name`]: crate::symbio_core::vdfs_provider::VdfsWriteResponse::name
    async fn install(&self, ctx: &VdfsContext, content: VdfsContent) -> VdfsResult<VdfsResponse> {
        let w = self
            .forward(ctx, "", VdfsRequest::Write { content })
            .await?
            .into_write()
            .ok_or_else(|| VdfsError::internal("容器对安装的响应类型不匹配"))?;
        if let Some(name) = w.name.clone() {
            self.announce(&name);
        }
        Ok(VdfsResponse::Write(w))
    }

    /// 广播一次变更：本目录下 `<name>` 这一条（新增 / 移除 / 状态变了）不再是旧样子。
    ///
    /// 这是**既有机制**（`notify_change` → 前端 `useVdfs` 的防抖重拉 + 重读当前项），
    /// 不是为启停新开的一条通道：装配态变了本来就该让订阅方知道。
    fn announce(&self, name: &str) {
        notify_change(PLUGIN_MANAGER, name);
    }
}

#[async_trait::async_trait]
impl Plugin for PluginManagerPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<DynVdfsProvider> {
        Some(self)
    }

    /// 本插件已无自有路由：清单与呈现由 `<根>/plugin_manager` 承担
    /// （更早已随 VDFS 下线）。
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
        // 挂载名由**使用方**（此处即本插件）选定：约定用插件名（`PLUGIN_MANAGER`），
        // 插件名在宿主内唯一，天然就是合格的挂载名。provider 自身不含此概念。
        // 会话链路（LLM 工具）与前端链路因此拿到同一份 (挂载名, 实现) 集合。
        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            let me: DynVdfsProvider = self.clone();
            visitor.register_vdfs_provider(PLUGIN_MANAGER, me).await;
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_MANAGER, PluginManagerPlugin::build, dyn Plugin);

#[async_trait::async_trait]
impl VdfsProvider for PluginManagerPlugin {
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        // 订阅与路径无关（本目录的变更源就是自己，见 `announce`），故在拆段之前统一处理。
        match &req {
            VdfsRequest::Watch { sink } => {
                let sink = sink.clone();
                watch_changes(PLUGIN_MANAGER, path, sink).await?;
                return Ok(VdfsResponse::Unit);
            }
            VdfsRequest::Unwatch => {
                unwatch_changes(PLUGIN_MANAGER, path).await?;
                return Ok(VdfsResponse::Unit);
            }
            _ => {}
        }

        // 自身根：可列 / 可 stat / 可写（= 安装一个插件），其余拒绝
        if path.trim_matches('/').is_empty() {
            return match req {
                VdfsRequest::List { .. } => Ok(VdfsResponse::list(self.list_items(ctx).await?)),
                VdfsRequest::Stat => Ok(VdfsResponse::Stat(self.root_node(ctx).await)),
                VdfsRequest::Write { content } => self.install(ctx, content).await,
                _ => Err(VdfsError::invalid(
                    "插件管理根不是可操作节点，请给出 <插件名> 路径",
                )),
            };
        }

        let name = leaf(path)?;

        // 自有分区：不是插件，数据在前端 store（见 [`OWN_SECTIONS`]）
        if let Some(s) = section_of(name) {
            return match req {
                VdfsRequest::Stat => Ok(VdfsResponse::Stat(section_node(s))),
                VdfsRequest::Read | VdfsRequest::Write { .. } => Err(VdfsError::Forbidden(
                    format!("分区 {} 的数据由前端状态自持，VDFS 侧无正文", s.id),
                )),
                _ => Err(VdfsError::not_found(format!("未知路径：{path}"))),
            };
        }

        match req {
            VdfsRequest::Stat => {
                let entry = self.entry_of(ctx, name).await?;
                let config = Self::config_of(ctx, name).await;
                Ok(VdfsResponse::Stat(entry_node(&entry, config)))
            }
            VdfsRequest::Read => {
                let entry = self.entry_of(ctx, name).await?;
                self.read_entry(ctx, &entry).await
            }
            VdfsRequest::Write { content } => self.write_entry(ctx, name, content).await,
            VdfsRequest::Action { action, payload } => self.act(ctx, name, action, payload).await,
            VdfsRequest::Delete { recursive } => self.uninstall(ctx, name, recursive).await,
            VdfsRequest::List { .. } | VdfsRequest::Mkdir => Err(VdfsError::invalid(format!(
                "插件条目不是目录，没有子项：{path}"
            ))),
            // Watch / Unwatch 已在前面处理
            VdfsRequest::Watch { .. } | VdfsRequest::Unwatch => {
                Err(VdfsError::internal("订阅已在入口处处理"))
            }
        }
    }
}

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
