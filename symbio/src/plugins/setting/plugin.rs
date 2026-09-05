//! Setting 插件 - 设置管理

use crate::symbio_core::schemas::SuccessResponse;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginMeta,
    PluginPayload, CONFIG_GET, CONFIG_SET, PLUGIN_SETTING,
};
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

use crate::symbio_core::schemas::resources::{
    DetailAction, DetailDefinition, DetailField, DetailSection,
};
use crate::symbio_core::schemas::setting::{setting_get, setting_list};
use tracing::info;

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

        // 统一资源协议：resources/list（设置分区清单）；
        // get/upload/delete/status 走 trait 默认 NotImplemented（分区不可增删，
        // 保存由前端 editor 经各插件 config/set 自持完成）
        if let Some(resp) =
            crate::symbio_core::resources::dispatch(self.as_ref(), path, &ctx).await
        {
            return resp;
        }

        match path {
            "list" => {
                let categories = vec![
                    setting_list::SettingCategory {
                        id: "general".to_string(),
                        name: "常规设置".to_string(),
                        icon: "settings".to_string(),
                    },
                    setting_list::SettingCategory {
                        id: "model".to_string(),
                        name: "Model 设置".to_string(),
                        icon: "smart_toy".to_string(),
                    },
                ];
                Ok(PluginPayload::new(&setting_list::Response { categories }))
            }
            "get" => {
                let req: setting_get::Request = ctx.payload()?;

                let cfg = self.config.read().await;
                // 尝试从配置中按分类提取，如果没有该分类，则返回空对象
                let category_settings =
                    cfg.get(&req.category).cloned().unwrap_or_else(|| json!({}));

                info!(
                    category = %req.category,
                    keys = ?category_settings.as_object().map(|o| o.keys().collect::<Vec<_>>()),
                    "获取分类设置"
                );

                Ok(PluginPayload::new(&setting_get::Response {
                    category: req.category.clone(),
                    settings: category_settings,
                }))
            }
            CONFIG_GET => {
                let cfg = self.config.read().await;
                // 彻底扁平化：直接返回 Value 对象，不再包装在 Response.config 中
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
        _ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_SETTING, SettingPlugin::build, dyn Plugin);

// ==================== 统一资源协议接入 ====================

/// 设置分区（固定清单）。`id` 同时作为前端 editor 的"扩展名"（config_type），
/// 前端按 `setting:<config_type>` 复合键注入专属编辑表单。
const SETTING_SECTIONS: [(&str, &str); 5] = [
    ("appearance", "外观"),
    ("session", "会话设置"),
    ("local", "本地工具"),
    ("web", "网络工具"),
    ("about", "关于"),
];

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

/// config 绑定定义骨架：单分区 + 单「保存配置」动作（`_desc` 预留：schema 暂无描述字段）
fn config_definition(title: &str, _desc: &str, load: &str, save: &str, fields: Vec<DetailField>) -> DetailDefinition {
    DetailDefinition {
        binding: "config".into(),
        load_path: Some(load.into()),
        save_path: Some(save.into()),
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

fn session_detail_definition() -> DetailDefinition {
    config_definition(
        "会话设置",
        "控制会话存储与上下文行为",
        "session/config get",
        "session/config set",
        vec![
            detail_field_number("max_messages", "最大消息数", "每个会话保存的最大消息数量", 10.0, 1000.0, serde_json::json!(100)),
            detail_field_toggle("auto_compress", "自动压缩", "当消息数超过阈值时自动压缩历史", true),
            detail_field_number("compress_threshold", "压缩阈值", "触发自动压缩的消息数量", 10.0, 500.0, serde_json::json!(50)),
            detail_field_number(
                "context_messages",
                "上下文消息数量",
                "Model 对话时包含的上下文消息数量（0 表示不限制，6 表示 3 轮对话）",
                0.0,
                200.0,
                serde_json::json!(6),
            ),
        ],
    )
}

fn local_detail_definition() -> DetailDefinition {
    config_definition(
        "本地工具设置",
        "控制本地 Shell / 文件工具的启用与超时",
        "local/config get",
        "local/config set",
        vec![
            detail_field_toggle("shell_enabled", "启用 Shell 工具", "允许执行 Shell 命令", true),
            detail_field_toggle("file_enabled", "启用文件工具", "允许文件读写操作", true),
            detail_field_number("shell_timeout", "Shell 超时（秒）", "Shell 命令执行超时时间", 1.0, 3600.0, serde_json::json!(60)),
        ],
    )
}

fn web_detail_definition() -> DetailDefinition {
    config_definition(
        "网络工具设置",
        "控制 Web 工具的启用、超时与搜索服务凭据",
        "web/config get",
        "web/config set",
        vec![
            detail_field_toggle("web_enabled", "启用 Web 工具", "允许网络请求", true),
            detail_field_number("web_timeout", "Web 超时（秒）", "Web 请求超时时间", 1.0, 300.0, serde_json::json!(300)),
            detail_field_password("tavily_api_key", "Tavily API Key", "用于高级网页搜索（优先）", "输入 Tavily API Key"),
            detail_field_password("serper_api_key", "Serper API Key", "用于 Google 网页搜索（备用）", "输入 Serper API Key"),
        ],
    )
}

#[async_trait::async_trait]
impl crate::symbio_core::resources::ResourceProvider for SettingPlugin {
    fn kind(&self) -> &'static str {
        crate::symbio_core::resources::RESOURCE_SETTING
    }

    /// 设置分区为固定清单（非 EntityStore 实体目录）：
    /// 每个分区一项，extra 携带 config_type 供前端按"扩展名"分发 editor。
    async fn list_items(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<Vec<crate::symbio_core::resources::ResourceSummary>, PluginError> {
        Ok(SETTING_SECTIONS
            .iter()
            .map(|(id, name)| {
                let mut it = crate::symbio_core::resources::ResourceSummary::new(
                    crate::symbio_core::resources::RESOURCE_SETTING,
                    *id,
                    *name,
                );
                if let serde_json::Value::Object(ref mut m) = it.extra {
                    let _ = m.insert("config_type".to_string(), serde_json::json!(id));
                }
                it
            })
            .collect())
    }

    /// 分区详情定义：session/local/web 下发表单定义（前端 DetailForm 渲染）；
    /// appearance/about 返回 None（保留注册 editor：前端 store 即时生效 / 信息展示）。
    async fn detail_definition(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        id: &str,
    ) -> Option<DetailDefinition> {
        match id {
            "session" => Some(session_detail_definition()),
            "local" => Some(local_detail_definition()),
            "web" => Some(web_detail_definition()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::resources::ResourceProvider;
    use crate::symbio_core::resources::RESOURCE_SETTING;
    use crate::symbio_core::SimpleRequest;

    #[tokio::test]
    async fn list_items_returns_sections_in_declared_order() {
        let plugin = SettingPlugin::default();
        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        let items = plugin.list_items(&ctx).await.unwrap();

        // 固定清单、按声明顺序（前端据此展示，不做二次排序）
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["appearance", "session", "local", "web", "about"]);

        // kind 标记为 setting，名称正确
        let first = &items[0];
        assert_eq!(first.kind, RESOURCE_SETTING);
        assert_eq!(first.name, "外观");

        // 每项的 extra.config_type 即 editor"扩展名"（与 id 一致）
        for it in &items {
            assert_eq!(it.extra.get("config_type").and_then(Value::as_str), Some(it.id.as_str()));
        }
    }
}
