//! Setting 插件 - 设置管理

use crate::symbio_core::schemas::SuccessResponse;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta,
    PluginPayload, CONFIG_GET, CONFIG_SET, PLUGIN_SETTING,
};
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

use crate::symbio_core::schemas::entities::{
    DetailAction, DetailCondition, DetailDefinition, DetailField, DetailOption, DetailSection,
};
use crate::symbio_core::schemas::session::session_config::SessionConfig;
use crate::symbio_core::vdfs::{
    self, DynVdfsProvider, VdfsAccess, VdfsContent, VdfsContext, VdfsError, VdfsFieldError,
    VdfsNode, VdfsProvider, VdfsResult, VdfsValidationError, VdfsWriteResponse,
};

#[derive(Clone)]
pub struct SettingPlugin {
    config: Arc<RwLock<Value>>,
    /// 父插件引用
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

impl SettingPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let config = ctx.config().unwrap_or_else(|| serde_json::json!({}));
        let parent = ctx.parent();

        Arc::new(SettingPlugin::new(parent, config)) as Arc<dyn Plugin>
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: Value) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            parent: Arc::new(RwLock::new(parent)),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("setting", "系统设置")
            .with_description("设置管理插件")
            .with_version("0.1.0")
    }

    /// 获取父插件引用
    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }
}

impl Default for SettingPlugin {
    fn default() -> Self {
        Self::new(None, json!({}))
    }
}

#[async_trait::async_trait]
impl Plugin for SettingPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        // `setting/list` / `setting/get` 已随 VDFS 下线：分区清单与取值分别由
        // `.vdfs/setting` 的 `vdfs/list` / `vdfs/read` 承担（旧的 list 还硬编码
        // 着「常规设置 / Model 设置」两个历史分类，与现行六分区早已不符）。
        match path {
            CONFIG_GET => {
                let cfg = self.config.read().await;
                // 扁平化契约：直接返回 Value 对象，不包 Response.config
                let cfg_value = cfg.clone();
                Ok(PluginPayload::new(&cfg_value))
            }
            CONFIG_SET => {
                let payload: serde_json::Value = ctx.payload()?;
                {
                    let mut cfg = self.config.write().await;
                    *cfg = payload.clone();
                }
                if let Some(p) = self.get_parent().await {
                    let save_ctx = ctx.fork();
                    save_ctx.set(crate::symbio_core::PATH, "save_config".to_string());
                    p.route(save_ctx).await?;
                }
                Ok(PluginPayload::new(&SuccessResponse::default()))
            }
            _ => Err(PluginError::NotFound(format!("未知路径: {path}"))),
        }
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

/// 设置分区（固定清单）——实体提供者机制与 VDFS 挂载点**共用同一份声明**。
///
/// `id` 同时作为前端 editor 的"扩展名"（config_type）；`prefix` 是该分区配置
/// 读写的目标插件前缀（`None` = 数据由前端状态自持 / 纯展示，VDFS 侧只读）。
struct SectionSpec {
    id: &'static str,
    label: &'static str,
    prefix: Option<&'static str>,
}

const SETTING_SECTIONS: [SectionSpec; 6] = [
    SectionSpec {
        id: "appearance",
        label: "外观",
        prefix: None,
    },
    SectionSpec {
        id: "session",
        label: "会话设置",
        prefix: Some("session"),
    },
    SectionSpec {
        id: "local",
        label: "本地工具",
        prefix: Some("local"),
    },
    SectionSpec {
        id: "web",
        label: "网络工具",
        prefix: Some("web"),
    },
    SectionSpec {
        id: "gateway",
        label: "开放接口",
        prefix: Some("gateway"),
    },
    SectionSpec {
        id: "about",
        label: "关于",
        prefix: None,
    },
];

/// 按 id 取分区声明
fn section_of(id: &str) -> Option<&'static SectionSpec> {
    SETTING_SECTIONS.iter().find(|s| s.id == id)
}

/// 分区详情定义（`appearance` / `about` 无定义：前端状态自持 / 纯信息展示）
fn section_definition(id: &str) -> Option<DetailDefinition> {
    match id {
        "session" => Some(session_detail_definition()),
        "local" => Some(local_detail_definition()),
        "web" => Some(web_detail_definition()),
        "gateway" => Some(gateway_detail_definition()),
        _ => None,
    }
}

// ==================== 详情页定义（definition-driven detail） ====================
//
// 会话 / 本地工具 / 网络工具三个分区为「交互不复杂」的 config 绑定表单，
// 由后端下发定义、前端通用渲染器 DetailForm 动态生成（前端零页面开发）；
// 数据通道沿用各插件标准 config 路由（`<plugin>/config get|set`）。
// appearance（前端 store 即时生效）、about（信息展示）不适用定义，保留注册 editor。

fn detail_field_number(
    key: &str,
    label: &str,
    desc: &str,
    min: f64,
    max: f64,
    default: serde_json::Value,
) -> DetailField {
    DetailField {
        key: key.into(),
        label: label.into(),
        description: Some(desc.into()),
        widget: "number".into(),
        min: Some(min),
        max: Some(max),
        step: Some(1.0),
        default: Some(default),
        ..Default::default()
    }
}

fn detail_field_toggle(key: &str, label: &str, desc: &str, default: bool) -> DetailField {
    DetailField {
        key: key.into(),
        label: label.into(),
        description: Some(desc.into()),
        widget: "toggle".into(),
        default: Some(serde_json::json!(default)),
        ..Default::default()
    }
}

fn detail_field_password(key: &str, label: &str, desc: &str, placeholder: &str) -> DetailField {
    DetailField {
        key: key.into(),
        label: label.into(),
        description: Some(desc.into()),
        widget: "password".into(),
        placeholder: Some(placeholder.into()),
        ..Default::default()
    }
}

/// config 绑定定义骨架：单分区 + 单「保存配置」动作（`_desc` 预留：schema 暂无描述字段）。
///
/// load/save 路径由插件前缀 + 协议常量（`CONFIG_GET`/`CONFIG_SET`）构建，
/// 路径拼写单一来源，禁止手写字面量（防止与协议路由脱节）。
fn config_definition(
    title: &str,
    _desc: &str,
    prefix: &str,
    fields: Vec<DetailField>,
) -> DetailDefinition {
    DetailDefinition {
        binding: "config".into(),
        load_path: Some(format!("{prefix}/{CONFIG_GET}")),
        save_path: Some(format!("{prefix}/{CONFIG_SET}")),
        title_from: vec![],
        title_fallback: Some(title.into()),
        subtitle_from: vec![],
        sections: vec![DetailSection {
            title: None,
            collapsed: false,
            fields,
        }],
        badges: vec![],
        actions: vec![DetailAction {
            id: "save".into(),
            label: "保存配置".into(),
            style: "primary".into(),
            busy_label: Some("保存中…".into()),
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// 会话分区：字段默认值**一律从 [`SessionConfig::default()`] 读出**。
///
/// 本函数只描述 UI 元信息（类型 / 标题 / 说明 / 取值范围），默认值不写第二份
/// 字面量——历史上 schema 与 serde 各写一份，出现过 `max_tool_rounds`
/// schema=15 而 serde=65535 的漂移（面板显示的默认值与实际行为不符）。
/// 不变式由 `session_definition_defaults_come_from_session_config` 锁定。
fn session_detail_definition() -> DetailDefinition {
    let defaults = serde_json::to_value(SessionConfig::default()).unwrap_or_else(|_| json!({}));
    let num = |key: &str| defaults.get(key).cloned().unwrap_or(Value::Null);
    let flag = |key: &str| defaults.get(key).and_then(Value::as_bool).unwrap_or(false);
    config_definition(
        "会话设置",
        "控制会话存储与上下文行为",
        "session",
        vec![
            detail_field_number(
                "max_messages",
                "最大消息数",
                "每个会话保存的最大消息数量",
                10.0,
                1000.0,
                num("max_messages"),
            ),
            detail_field_toggle(
                "auto_compress",
                "自动压缩",
                "上下文 Token 用量达到有效上限 70% 时自动压缩历史（LLM 语义快照）",
                flag("auto_compress"),
            ),
            detail_field_toggle(
                "enable_compact_tool",
                "工具压缩",
                "向模型提供主动压缩工具（context_compact）与 55% 水位提醒；关闭后仅保留自动压缩兜底",
                flag("enable_compact_tool"),
            ),
            detail_field_number(
                "context_messages",
                "上下文消息数量",
                "Model 对话时包含的上下文消息数量（0 表示不限制，6 表示 3 轮对话）",
                0.0,
                200.0,
                num("context_messages"),
            ),
        ],
    )
}

fn local_detail_definition() -> DetailDefinition {
    config_definition(
        "本地工具设置",
        "控制本地 Shell / 文件工具的启用与超时",
        "local",
        vec![
            detail_field_toggle(
                "shell_enabled",
                "启用 Shell 工具",
                "允许执行 Shell 命令",
                true,
            ),
            detail_field_toggle("file_enabled", "启用文件工具", "允许文件读写操作", true),
            detail_field_number(
                "shell_timeout",
                "Shell 超时（秒）",
                "Shell 命令执行超时时间",
                1.0,
                3600.0,
                serde_json::json!(60),
            ),
        ],
    )
}

fn web_detail_definition() -> DetailDefinition {
    config_definition(
        "网络工具设置",
        "控制 Web 工具的启用、超时与搜索服务凭据",
        "web",
        vec![
            detail_field_toggle("web_enabled", "启用 Web 工具", "允许网络请求", true),
            detail_field_number(
                "web_timeout",
                "Web 超时（秒）",
                "Web 请求超时时间",
                1.0,
                300.0,
                serde_json::json!(300),
            ),
            detail_field_password(
                "tavily_api_key",
                "Tavily API Key",
                "用于高级网页搜索（优先）",
                "输入 Tavily API Key",
            ),
            detail_field_password(
                "serper_api_key",
                "Serper API Key",
                "用于 Google 网页搜索（备用）",
                "输入 Serper API Key",
            ),
        ],
    )
}

fn gateway_detail_definition() -> DetailDefinition {
    config_definition(
        "开放接口",
        "配置本应用如何被调用（入站，对外提供服务）。前端连向何处（出站）由左下角「系统目录」切换器统一管理，不在本页设置",
        "gateway",
        vec![
            // ---- 入站 ----
            DetailField {
                key: "inbound_enabled".into(),
                label: "启用入站服务".into(),
                description: Some(
                    "开启后本应用通过 HTTP/WebSocket 对外提供与前端完全一致的 API，第三方或其他 Symbio 实例可据此驱动本应用"
                        .into(),
                ),
                widget: "toggle".into(),
                default: Some(serde_json::json!(false)),
                ..Default::default()
            },
            DetailField {
                key: "inbound_protocol".into(),
                label: "入站协议".into(),
                widget: "select".into(),
                options: vec![
                    DetailOption {
                        value: "native".into(),
                        label: "native（仅本机 Tauri IPC，不监听端口）".into(),
                    },
                    DetailOption {
                        value: "http".into(),
                        label: "http（监听端口，第三方/其他实例可访问）".into(),
                    },
                ],
                default: Some(serde_json::json!("native")),
                ..Default::default()
            },
            DetailField {
                key: "inbound_bind".into(),
                label: "监听地址".into(),
                description: Some("127.0.0.1 仅本机；0.0.0.0 暴露给全网（需配合访问令牌）".into()),
                widget: "text".into(),
                default: Some(serde_json::json!("127.0.0.1")),
                ..Default::default()
            },
            DetailField {
                key: "inbound_port".into(),
                label: "监听端口".into(),
                widget: "number".into(),
                min: Some(1.0),
                max: Some(65535.0),
                step: Some(1.0),
                default: Some(serde_json::json!(9231)),
                ..Default::default()
            },
            DetailField {
                key: "inbound_token".into(),
                label: "访问令牌 (API Key)".into(),
                description: Some("Bearer Token；回环地址可留空，非回环地址必填".into()),
                widget: "password".into(),
                placeholder: Some("留空则不校验（仅限 127.0.0.1）".into()),
                ..Default::default()
            },
            DetailField {
                key: "inbound_readonly".into(),
                label: "只读模式".into(),
                description: Some("仅放行查询类路径，禁止写操作与命令执行".into()),
                widget: "toggle".into(),
                default: Some(serde_json::json!(false)),
                ..Default::default()
            },
        ],
    )
}

// ==================== VDFS 挂载点（/setting） ====================
//
// 设置模块是 VDFS 的**首个原生 provider**，示范要点五：
// provider 自己完成数据操作与**校验**——`write` 在把数据转给目标插件的
// `config/set` 之前逐字段校验，失败返回字段级错误，目标插件永远收到合法数据。

/// 分区节点。
///
/// `ext` 是**详情渲染器的唯一分发键**，因此这里按分区的呈现方式声明：
///
/// - **定义驱动分区**（有 `schema`：session / local / web / gateway）→ `ext = form`，
///   呈现描述经 `schema` 透传（VDFS 不解释其内容，前端按 `ext` 选通用表单渲染器）；
/// - **前端自持分区**（无 `schema`：appearance / about）→ `ext = 分区 id`，
///   由前端映射到各自的专属渲染器（外观设置 / 关于）。
///
/// 两种情形都不需要前端硬编码分区清单：前端只持有 `ext → 渲染器` 的纯 UI 映射。
fn section_node(s: &SectionSpec) -> VdfsNode {
    let writable = s.prefix.is_some();
    let access = if writable {
        VdfsAccess::READ_WRITE
    } else {
        VdfsAccess::READ
    };
    let definition = section_definition(s.id);
    let mut n = VdfsNode::file(s.id, s.label, access);
    n.kind = crate::symbio_core::entities::ENTITY_SETTING.to_string();
    n.ext = Some(if definition.is_some() {
        vdfs::VFDS_EXT_FORM.to_string()
    } else {
        s.id.to_string()
    });
    n.schema = definition.and_then(|def| serde_json::to_value(&def).ok());
    if !writable {
        n.description = Some("该分区数据由前端状态自持，VDFS 侧只读".to_string());
    }
    n
}

/// 空值判定（`null` / 空白字符串视为未填）
fn is_blank(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

/// 条件求值（与前端 DetailForm 的 `visible_when` 语义一致）
fn condition_holds(c: &DetailCondition, value: &serde_json::Value) -> bool {
    let cur = value.get(&c.key);
    if !c.all.iter().all(|x| condition_holds(x, value)) {
        return false;
    }
    if let Some(eq) = &c.equals {
        if cur != Some(eq) {
            return false;
        }
    }
    if let Some(ne) = &c.not_equals {
        if cur == Some(ne) {
            return false;
        }
    }
    if let Some(t) = c.truthy {
        if cur.and_then(serde_json::Value::as_bool).unwrap_or(false) != t {
            return false;
        }
    }
    true
}

/// 单字段类型 / 范围校验
fn field_error(f: &DetailField, v: &serde_json::Value) -> Option<String> {
    match f.widget.as_str() {
        "number" => {
            let Some(n) = v.as_f64() else {
                return Some("必须是数字".to_string());
            };
            if let Some(min) = f.min {
                if n < min {
                    return Some(format!("不能小于 {min}"));
                }
            }
            if let Some(max) = f.max {
                if n > max {
                    return Some(format!("不能大于 {max}"));
                }
            }
            None
        }
        "toggle" => (!v.is_boolean()).then(|| "必须是布尔值".to_string()),
        "select" => {
            let Some(s) = v.as_str() else {
                return Some("必须是字符串".to_string());
            };
            if !f.options.is_empty() && !f.options.iter().any(|o| o.value == s) {
                return Some(format!(
                    "必须是以下之一：{}",
                    f.options
                        .iter()
                        .map(|o| o.value.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                ));
            }
            None
        }
        "list" => (!v.is_array()).then(|| "必须是字符串数组".to_string()),
        "map" => (!v.is_object()).then(|| "必须是键值对象".to_string()),
        "text" | "password" | "textarea" | "datalist" => {
            (!v.is_string()).then(|| "必须是字符串".to_string())
        }
        _ => None,
    }
}

impl SettingPlugin {
    /// 按分区定义逐字段校验提交值（字段级错误；无错则 `Ok`）
    fn validate_section(
        &self,
        def: &DetailDefinition,
        value: &serde_json::Value,
    ) -> Result<(), VdfsValidationError> {
        let Some(obj) = value.as_object() else {
            return Err(VdfsValidationError::new("设置内容必须是 JSON 对象"));
        };
        let mut err = VdfsValidationError::new("设置校验未通过");

        for section in &def.sections {
            for f in &section.fields {
                // 只读展示字段不参与校验
                if f.widget == "static" {
                    continue;
                }
                // 条件隐藏字段不参与校验（与前端渲染保持一致）
                if let Some(cond) = &f.visible_when {
                    if !condition_holds(cond, value) {
                        continue;
                    }
                }
                let Some(current) = obj.get(&f.key) else {
                    if f.required {
                        err.fields.push(VdfsFieldError {
                            field: f.key.clone(),
                            message: "必填项缺失".to_string(),
                        });
                    }
                    continue;
                };
                if f.required && is_blank(current) {
                    err.fields.push(VdfsFieldError {
                        field: f.key.clone(),
                        message: "必填项不能为空".to_string(),
                    });
                    continue;
                }
                if let Some(message) = field_error(f, current) {
                    err.fields.push(VdfsFieldError {
                        field: f.key.clone(),
                        message,
                    });
                }
            }
        }

        if err.has_fields() {
            Err(err)
        } else {
            Ok(())
        }
    }

    /// 经父容器转发到目标插件的标准配置协议路由（`<prefix>/config/get|set`）
    async fn route_config(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        prefix: &str,
        op: &str,
        payload: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, VdfsError> {
        let parent = self
            .get_parent()
            .await
            .ok_or_else(|| VdfsError::internal("设置插件未挂载到容器，无法访问其他插件的配置"))?;
        let sub = ctx.fork();
        sub.set(crate::symbio_core::PATH, format!("{prefix}/{op}"));
        if let Some(p) = payload {
            sub.set_payload(p)
                .map_err(|e| VdfsError::internal(e.to_string()))?;
        }
        let resp = parent.route(sub).await.map_err(vdfs::from_plugin_error)?;
        Ok(resp
            .get::<serde_json::Value>()
            .unwrap_or(serde_json::Value::Null))
    }
}

#[async_trait::async_trait]
impl VdfsProvider for SettingPlugin {
    fn label(&self) -> Option<&str> {
        // 标签 / 顺序由本 provider 自持（实体注册表已随 VDFS 收敛下线）
        Some(
            "设置",
        )
    }

    fn description(&self) -> Option<&str> {
        Some("系统设置分区。分区清单固定，读 / 写经目标插件的标准配置协议；写入前由本模块校验。")
    }

    fn order(&self) -> i32 {
        6
    }

    fn icon(&self) -> Option<&str> {
        Some("settings")
    }

    /// 分区清单固定、每一项都是叶子文档：可列，但不可递归遍历
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

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let s = section_of(path)
            .ok_or_else(|| VdfsError::not_found(format!("未知设置分区：{path}")))?;
        let prefix = s.prefix.ok_or_else(|| {
            VdfsError::Forbidden(format!("分区 {} 的数据由前端状态自持，VDFS 侧不可读", s.id))
        })?;
        let value = self
            .route_config(
                &vdfs::host_ctx(ctx)?,
                prefix,
                crate::symbio_core::CONFIG_GET,
                None,
            )
            .await?;
        let text = serde_json::to_string_pretty(&value)
            .map_err(|e| VdfsError::internal(format!("配置序列化失败：{e}")))?;
        Ok(VdfsContent::text("", text).with_mime("application/json"))
    }

    /// 写入：**先校验，后转发**——校验失败时目标插件不会被调用。
    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let s = section_of(path)
            .ok_or_else(|| VdfsError::not_found(format!("未知设置分区：{path}")))?;
        let prefix = s.prefix.ok_or_else(|| {
            VdfsError::Forbidden(format!("分区 {} 的数据由前端状态自持，VDFS 侧不可写", s.id))
        })?;

        let text = content
            .text
            .as_deref()
            .ok_or_else(|| VdfsError::invalid("设置写入需要文本（JSON）内容"))?;
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| VdfsError::invalid(format!("设置内容不是合法 JSON：{e}")))?;

        if let Some(def) = section_definition(path) {
            self.validate_section(&def, &value)
                .map_err(VdfsError::Invalid)?;
        }

        self.route_config(
            &vdfs::host_ctx(ctx)?,
            prefix,
            crate::symbio_core::CONFIG_SET,
            Some(value),
        )
        .await?;

        Ok(VdfsWriteResponse {
            path: String::new(),
            created: false,
            etag: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::entities::ENTITY_SETTING;

    #[tokio::test]
    async fn list_returns_sections_in_declared_order() {
        let plugin = SettingPlugin::default();
        let items = plugin.list(&vctx(), "").await.unwrap();

        // 固定清单、按声明顺序（前端据此展示，不做二次排序）。
        // provider 返回的节点自带 `name`，全路径由分发层补挂载名后合成。
        let names: Vec<&str> = items.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["appearance", "session", "local", "web", "gateway", "about"]
        );

        // kind 标记为 setting；`name` 是地址段、`title` 是人读标签
        let first = &items[0];
        assert_eq!(first.kind, ENTITY_SETTING);
        assert_eq!(first.name, "appearance");
        assert_eq!(first.title, "外观");

        // 无 schema 的分区（appearance / about）：ext 即分区 id —— 前端按
        // `ext → 渲染器` 的纯 UI 映射回退到专属 editor
        assert_eq!(items[0].ext.as_deref(), Some("appearance"));
    }

    /// 回归：config 绑定分区的 load/save 路径必须命中目标插件的标准
    /// config 协议路由（`<prefix>/config/get|set`）。
    #[test]
    fn config_binding_paths_follow_protocol_constants() {
        for (def, prefix) in [
            (session_detail_definition(), "session"),
            (local_detail_definition(), "local"),
            (web_detail_definition(), "web"),
        ] {
            assert_eq!(def.binding, "config");
            assert_eq!(
                def.load_path.as_deref(),
                Some(&format!("{prefix}/{CONFIG_GET}")[..])
            );
            assert_eq!(
                def.save_path.as_deref(),
                Some(&format!("{prefix}/{CONFIG_SET}")[..])
            );
        }
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
        // 顺序取自实体注册表（**单一真相源**），使 `.vdfs` 左栏与实体页恒等
        // （S4 起不再是 provider 自定的 60）
        assert_eq!(
            p.order(),
            6
        );
        assert_eq!(p.icon(), Some("settings"));
        assert_eq!(p.root_access().flags(), "l");
        assert!(!p.root_access().traverse, "分区是叶子，不参与树遍历");
    }

    #[tokio::test]
    async fn vdfs_list_declares_ext_by_renderer_identity() {
        let p = SettingPlugin::default();
        let items = p.list(&vctx(), "").await.unwrap();
        assert_eq!(
            items.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
            vec!["appearance", "session", "local", "web", "gateway", "about"]
        );

        // 定义驱动分区：ext = form（前端据 ext 选通用表单渲染器）；
        // 呈现描述经 schema 透传
        for id in ["session", "local", "web", "gateway"] {
            let n = items.iter().find(|n| n.name == id).unwrap();
            assert_eq!(n.effective_ext().as_deref(), Some("form"), "{id} 应为 form");
            assert_eq!(n.access.flags(), "rw", "{id} 可写");
            assert!(n.schema.is_some(), "{id} 应携带表单定义");
            assert!(!n.is_dir(), "分区是文档而非目录");
        }

        // 前端自持分区：ext = 分区 id（前端映射到专属渲染器），无 schema、只读
        for id in ["appearance", "about"] {
            let n = items.iter().find(|n| n.name == id).unwrap();
            assert_eq!(n.effective_ext().as_deref(), Some(id), "{id} 以 id 为 ext");
            assert_eq!(n.access.flags(), "r", "{id} 只读");
            assert!(n.schema.is_none(), "{id} 无表单定义");
        }

        // 叶子节点无子项
        assert!(p.list(&vctx(), "session").await.is_err());
        // 未知分区
        assert!(p.stat(&vctx(), "nope").await.is_err());
    }

    /// 会话设置面板的默认值契约（被删的 `config_schema` 防漂移守卫的**继承者**，
    /// 守的是现在真正生效的那份定义）。锁定两个不变式：
    ///
    /// 1. **面板字段的 key 必须真实存在于 [`SessionConfig`]**，且其默认值取自
    ///    [`SessionConfig::default()`]——本就同源，故结构上不可能再出现历史上
    ///    「schema 写 15、serde 写 65535」的双份字面量漂移；
    /// 2. **`SessionConfig` 自身的两个真源必须一致**：`#[serde(default = …)]`
    ///    的默认值函数 与 `impl Default`。这两处是各自独立书写的，是本测试
    ///    唯一能真正抓到的漂移（空对象反序列化走前者，`default()` 走后者）。
    #[test]
    fn session_definition_defaults_come_from_session_config() {
        let defaults = serde_json::to_value(SessionConfig::default())
            .expect("SessionConfig::default() 必须可序列化");

        // ① 面板字段 ↔ SessionConfig：key 存在 + 默认值同源
        let def = session_detail_definition();
        let fields = &def.sections[0].fields;
        assert!(!fields.is_empty(), "会话分区必须有字段");
        for f in fields {
            let declared = f
                .default
                .clone()
                .unwrap_or_else(|| panic!("字段 {} 缺少 default", f.key));
            let actual = defaults
                .get(&f.key)
                .unwrap_or_else(|| panic!("SessionConfig 不存在字段 {}", f.key));
            assert_eq!(
                &declared, actual,
                "面板 {}.default={declared} 与 SessionConfig::default().{}={actual} 不一致",
                f.key, f.key
            );
        }

        // ② SessionConfig 内部：serde 默认值函数 == Default impl
        let from_empty: SessionConfig =
            serde_json::from_str("{}").expect("空对象应能反序列化出默认配置");
        assert_eq!(
            serde_json::to_value(&from_empty).unwrap(),
            defaults,
            "SessionConfig 的 serde 默认值与 Default impl 漂移了（两处各自书写）"
        );
    }

    #[test]
    fn validation_catches_required_range_type_and_enum() {
        let plugin = SettingPlugin::default();
        let def = gateway_detail_definition();

        // 越界端口
        let err = plugin
            .validate_section(&def, &json!({ "inbound_port": 70000 }))
            .unwrap_err();
        assert_eq!(err.fields[0].field, "inbound_port");

        // 类型错误
        let err = plugin
            .validate_section(&def, &json!({ "inbound_port": "abc" }))
            .unwrap_err();
        assert!(err.fields[0].message.contains("必须是数字"));

        // 枚举越界
        let err = plugin
            .validate_section(&def, &json!({ "inbound_protocol": "carrier-pigeon" }))
            .unwrap_err();
        assert_eq!(err.fields[0].field, "inbound_protocol");

        // 合法值 + 未提交字段 → 通过
        assert!(plugin
            .validate_section(
                &def,
                &json!({ "inbound_port": 9231, "inbound_protocol": "http" })
            )
            .is_ok());

        // 非对象载荷
        assert!(plugin.validate_section(&def, &json!("nope")).is_err());
    }

    /// 写入：校验先于转发——坏数据在触达目标插件之前就被拦下
    #[tokio::test]
    async fn vdfs_write_validates_before_forwarding() {
        let p = SettingPlugin::default();
        let bad = VdfsContent::text("", r#"{"inbound_port":0}"#);
        let err = p.write(&vctx(), "gateway", &bad).await.unwrap_err();
        match err {
            VdfsError::Invalid(v) => assert_eq!(v.fields[0].field, "inbound_port"),
            other => panic!("应为字段级校验错误，实得 {other:?}"),
        }

        // 非 JSON 文本
        let err = p
            .write(&vctx(), "gateway", &VdfsContent::text("", "{oops"))
            .await
            .unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)));

        // 缺少文本
        let err = p
            .write(&vctx(), "gateway", &VdfsContent::default())
            .await
            .unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)));

        // 合法数据：越过校验，随后因「未挂载到容器」而失败（证明校验未拦）
        let ok = VdfsContent::text("", r#"{"inbound_port":9231}"#);
        let err = p.write(&vctx(), "gateway", &ok).await.unwrap_err();
        assert!(
            matches!(err, VdfsError::Internal(_)),
            "合法数据应越过校验，实得 {err:?}"
        );
    }

    /// 前端自持分区在 VDFS 侧只读
    #[tokio::test]
    async fn vdfs_readonly_sections_reject_io() {
        let p = SettingPlugin::default();
        for path in ["appearance", "about"] {
            assert!(matches!(
                p.read(&vctx(), path).await.unwrap_err(),
                VdfsError::Forbidden(_)
            ));
            let err = p
                .write(&vctx(), path, &VdfsContent::text("", "{}"))
                .await
                .unwrap_err();
            assert!(matches!(err, VdfsError::Forbidden(_)));
        }
    }
}
