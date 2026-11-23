//! Sandbox 插件 —— 横切沙箱接口的实现方 / 发布方 + 档案 VDFS 管理面（D-531 / 裁决 B）。
//!
//! ## 两种身份同处一个插件
//!
//! 1. [`Plugin`]：能力收集期把引擎作为 [`SandboxFace`] 句柄发布进 ctx 共享槽，
//!    供 tool 拦截与各工具查询；在 `TRAVERSE_AVAILABLE_OPTIONS` 广播里贡献沙箱选项。
//! 2. [`VdfsProvider`]：沙箱档案的增删改查（挂 `<根>/sandbox`），落本插件自己的
//!    存储；前端按通用 VDFS 资源机制渲染，**没有专用组件、不理解沙箱概念**。
//!
//! 策略与配置全在本 crate 内部（[`crate::engine`] / [`crate::detail`]），不进架构层。

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use symbio_plugin_api::{
    plugin_dir_from_ctx, sandbox_publish, vdfs_unwatch_changes, vdfs_watch_changes, Plugin,
    PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, SandboxFace, VdfsAccess, VdfsContent, VdfsContext, VdfsError,
    VdfsNewType, VdfsNode, VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse,
    PLUGIN_ID_SANDBOX, VDFS_EXT_FORM, VDFS_STATUS_NONE,
};
use symbio_providers::{vdfs_id_of, SingleFileVdfs};

use crate::detail::sandbox_detail_definition;
use crate::engine::{Engine, SandboxProfileConfig, SandboxRegistry};

const LABEL: &str = "沙箱";
const MANIFEST: &str = "sandbox.json";

/// 内置沙箱档（只读）：用户可复制后改，不可删除/改名。
/// 它们是开箱即用的安全档位，承接原 mode/risk_level 的常见组合。
fn builtin_profiles() -> Vec<(String, SandboxProfileConfig)> {
    vec![
        (
            "default".into(),
            SandboxProfileConfig {
                name: "默认".into(),
                description: Some("交互模式 + 中风险阈值（等价旧默认）".into()),
                builtin: true,
                approval_mode: Some("interactive".into()),
                risk_threshold: Some("medium".into()),
                ..Default::default()
            },
        ),
        (
            "strict".into(),
            SandboxProfileConfig {
                name: "严格".into(),
                description: Some("交互模式 + 高风险阈值，更多操作需审批".into()),
                builtin: true,
                approval_mode: Some("interactive".into()),
                risk_threshold: Some("high".into()),
                ..Default::default()
            },
        ),
        (
            "unattended".into(),
            SandboxProfileConfig {
                name: "无人值守".into(),
                description: Some("自动模式 + 中风险阈值，需审批时拒绝而非弹卡".into()),
                builtin: true,
                approval_mode: Some("auto".into()),
                risk_threshold: Some("medium".into()),
                ..Default::default()
            },
        ),
    ]
}

/// Sandbox 插件本体。
///
/// **无插件级"当前选中项"**：沙箱选择是会话级数据，随 `ctx[SANDBOX_ID]` 传来，
/// 每次 traverse 现取现用并绑定进发布的 [`Engine`]。多会话并行不共享可变选中态。
pub struct SandboxPlugin {
    /// 本插件目录（VDFS 存储根）。
    dir: PluginDir,
    /// 注册表：内置档 + 用户自定义档，收集期重建，执行期只读。
    registry: Arc<RwLock<SandboxRegistry>>,
}

impl SandboxPlugin {
    /// 静态工厂（满足 `submit_object_creator!` 协议）。
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_SANDBOX);
        Arc::new(SandboxPlugin {
            dir,
            registry: Arc::new(RwLock::new(SandboxRegistry::default())),
        }) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_SANDBOX, LABEL)
            .with_description("安全运行环境：沙箱档案的 VDFS 管理 + 横切沙箱句柄发布")
            .with_version("0.2.0")
            .with_icon(
                r#"<path d="M12 1L3 5v6c0 5.55 3.84 10.74 9 12 5.16-1.26 9-6.45 9-12V5l-9-4z"/>"#,
            )
    }

    /// 磁盘底座（每次现取，跟随 homedir 切换）；根 = 本插件自己的目录。
    fn store(&self) -> SingleFileVdfs {
        SingleFileVdfs::at(self.dir.dir(), PLUGIN_ID_SANDBOX, MANIFEST).with_label(LABEL)
    }

    fn id_of(path: &str) -> String {
        vdfs_id_of(path, PLUGIN_ID_SANDBOX)
    }

    /// 从存储重建注册表（内置档 + 自定义档）。
    async fn reload(&self) {
        let mut profiles = builtin_profiles();
        match self.store().entries().await {
            Ok(entries) => {
                for e in entries {
                    match e
                        .raw
                        .as_deref()
                        .and_then(|t| serde_json::from_str::<SandboxProfileConfig>(t).ok())
                    {
                        Some(cfg) => profiles.push((e.id.clone(), cfg)),
                        None => {
                            symbio_plugin_api::plugin_warn!("sandbox", "解析沙箱档案 {} 失败", e.id)
                        }
                    }
                }
            }
            Err(e) => symbio_plugin_api::plugin_warn!("sandbox", "读取沙箱档案失败：{e}"),
        }
        *self.registry.write().unwrap_or_else(|p| p.into_inner()) =
            SandboxRegistry::from_profiles(profiles);
    }

    /// VDFS 列表节点：档案标题取 name，内置档以只读访问位呈现。
    fn archive_node(id: &str, cfg: &SandboxProfileConfig) -> VdfsNode {
        let access = if cfg.builtin {
            VdfsAccess::READ
        } else {
            VdfsAccess::READ_WRITE
        };
        let mut node = VdfsNode::file(id, cfg.name.clone(), access);
        node.kind = PLUGIN_ID_SANDBOX.to_string();
        node.status = VDFS_STATUS_NONE.to_string();
        node.ext = Some(VDFS_EXT_FORM.to_string());
        node
    }
}

#[async_trait]
impl Plugin for SandboxPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(symbio_plugin_api::PATH).unwrap_or_default();
        // 沙箱没有路由臂：它经 ctx 句柄被调用、经 VDFS 被管理，都不走 route。
        Err(PluginError::NotFound(format!(
            "[sandbox] 沙箱是横切句柄，不提供路由子命令：{path}"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        // 收集期重建注册表（读磁盘）保证候选与句柄都是最新的。
        self.reload().await;

        // 选项收集广播：贡献 sandbox_id（唯一的会话安全选项，承接 mode/risk_level）。
        // 号段 70（见 session::options 模块文档的号段约定）。
        // ⚠️ 判定必须读 `ctx[PATH]`：traverse 的 `path` 参数是路由子命令（本插件不提供，
        // 恒为空串），广播端点写在 ctx 上（与 session / heartbeat 同一条协议）。
        if ctx.get(symbio_plugin_api::PATH).unwrap_or_default()
            == symbio_plugin_api::TRAVERSE_AVAILABLE_OPTIONS
        {
            if let Some(visitor) = ctx.get(symbio_plugin_api::OPTION_VISITOR) {
                const ORDER: i32 = 70;
                // guard 不能跨 await：拷出候选后立即释放。
                let mut archives: Vec<(String, String)> = {
                    let reg = self.registry.read().unwrap_or_else(|p| p.into_inner());
                    reg.archives()
                        .iter()
                        .map(|(id, cfg)| (id.clone(), cfg.name.clone()))
                        .collect()
                };
                archives.sort_by(|a, b| a.1.cmp(&b.1));
                let selected = ctx.get(symbio_plugin_api::SANDBOX_ID);
                let field = crate::detail::sandbox_option_field(&archives, selected.as_deref());
                visitor
                    .put(symbio_plugin_api::CapabilityRow::option(ORDER, field))
                    .await;
            }
            return Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()));
        }

        // 按本会话选中的 sandbox_id 绑定引擎句柄（裁决 B / 按 id 解析）：
        // 选择随 ctx 来自会话 metadata，插件不留跨会话共享选中态。
        let selected = ctx.get(symbio_plugin_api::SANDBOX_ID);
        let face: Arc<dyn SandboxFace> = Arc::new(Engine::new(self.registry.clone(), selected));
        if !sandbox_publish(&ctx, face) {
            symbio_plugin_api::plugin_warn!(
                "sandbox",
                "沙箱句柄已被发布（同一趟收集里两个沙箱插件？）——保留先到的那一份"
            );
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
        Some(self)
    }
}

#[async_trait]
impl VdfsProvider for SandboxPlugin {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                if !path.trim_matches('/').is_empty() {
                    return Err(VdfsError::not_found(format!("{LABEL}是叶子资源：{path}")));
                }
                let reg = self.registry.read().unwrap_or_else(|p| p.into_inner());
                let mut nodes: Vec<VdfsNode> = reg
                    .archives()
                    .iter()
                    .map(|(id, cfg)| Self::archive_node(id, cfg))
                    .collect();
                nodes.sort_by(|a, b| a.name.cmp(&b.name));
                Ok(VdfsResponse::list(nodes))
            }
            VdfsRequest::Stat => {
                if path.trim_matches('/').is_empty() {
                    return Ok(VdfsResponse::Stat(
                        VdfsNode::dir("", LABEL, VdfsAccess::LIST).with_new_type(Some(
                            VdfsNewType::new(PLUGIN_ID_SANDBOX, LABEL)
                                .with_description("新建沙箱档案（内置档只读，可复制后修改）")
                                .with_node_ext(VDFS_EXT_FORM)
                                .with_schema(
                                    serde_json::to_value(sandbox_detail_definition())
                                        .unwrap_or_default(),
                                ),
                        )),
                    ));
                }
                let id = Self::id_of(path);
                let reg = self.registry.read().unwrap_or_else(|p| p.into_inner());
                let cfg = reg
                    .archives()
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}档案：{id}")))?;
                Ok(VdfsResponse::Stat(Self::archive_node(&id, cfg)))
            }
            VdfsRequest::Read => {
                if path.trim_matches('/').is_empty() {
                    return Err(VdfsError::invalid(format!("该路径是目录：{path}")));
                }
                let id = Self::id_of(path);
                let reg = self.registry.read().unwrap_or_else(|p| p.into_inner());
                let cfg = reg
                    .archives()
                    .get(&id)
                    .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}档案：{id}")))?;
                // 表单里的两个 JSON textarea 字段由结构化字段渲染而来。
                let mut value = serde_json::to_value(cfg)
                    .map_err(|e| VdfsError::internal(format!("序列化沙箱详情失败：{e}")))?;
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "allowed_facts_json".into(),
                        serde_json::json!(
                            serde_json::to_string_pretty(&cfg.allowed_facts).unwrap_or_default()
                        ),
                    );
                    obj.insert(
                        "blocked_tools_json".into(),
                        serde_json::json!(
                            serde_json::to_string_pretty(&cfg.blocked_tools).unwrap_or_default()
                        ),
                    );
                }
                let text = serde_json::to_string_pretty(&value)
                    .map_err(|e| VdfsError::internal(format!("序列化沙箱详情失败：{e}")))?;
                Ok(VdfsResponse::Read(
                    VdfsContent::text(text).with_mime("application/json"),
                ))
            }
            VdfsRequest::Write { content } => {
                if content.binary {
                    return Err(VdfsError::invalid(format!("{LABEL}不接受二进制写入")));
                }
                let id = if path.trim_matches('/').is_empty() {
                    if !content.create {
                        return Err(VdfsError::invalid("写挂载根需要 create 意图"));
                    }
                    symbio_providers::vdfs_auto_id(PLUGIN_ID_SANDBOX)
                } else {
                    Self::id_of(path)
                };
                // 内置档只读：拒绝写入内置 id。
                if builtin_profiles().iter().any(|(k, _)| k == &id) {
                    return Err(VdfsError::Forbidden(format!(
                        "内置沙箱「{id}」只读；请复制后修改"
                    )));
                }
                let text = content.as_text().unwrap_or_default();
                let mut value: serde_json::Value = if text.trim().is_empty() {
                    serde_json::json!({ "name": id })
                } else {
                    serde_json::from_str(text)
                        .map_err(|e| VdfsError::invalid(format!("沙箱档案不是合法 JSON：{e}")))?
                };
                // 把两个 JSON textarea 解析回结构化字段。
                // 先把两个 JSON 字符串字段取出（避免借用 obj 后不能再 insert）。
                let facts_json = value
                    .get("allowed_facts_json")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                let tools_json = value
                    .get("blocked_tools_json")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                if let Some(obj) = value.as_object_mut() {
                    obj.remove("allowed_facts_json");
                    obj.remove("blocked_tools_json");
                    if let Some(s) = facts_json {
                        let parsed: std::collections::HashMap<String, Vec<String>> =
                            serde_json::from_str(&s).map_err(|e| {
                                VdfsError::invalid(format!("事实白名单不是合法 JSON：{e}"))
                            })?;
                        if let Ok(v) = serde_json::to_value(parsed) {
                            obj.insert("allowed_facts".into(), v);
                        }
                    }
                    if let Some(s) = tools_json {
                        let parsed: serde_json::Value = serde_json::from_str(&s).map_err(|e| {
                            VdfsError::invalid(format!("工具拦截不是合法 JSON：{e}"))
                        })?;
                        obj.insert("blocked_tools".into(), parsed);
                    }
                    obj.insert("builtin".into(), serde_json::json!(false));
                }
                let cfg: SandboxProfileConfig = serde_json::from_value(value)
                    .map_err(|e| VdfsError::invalid(format!("沙箱档案字段不合法：{e}")))?;
                let created = self
                    .store()
                    .write_json(&id, &serde_json::to_value(&cfg).unwrap_or_default())
                    .await?;
                self.reload().await;
                let generated = path.trim_matches('/').is_empty().then_some(id);
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: generated,
                    created,
                }))
            }
            VdfsRequest::Delete { .. } => {
                if path.trim_matches('/').is_empty() {
                    return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
                }
                let id = Self::id_of(path);
                if builtin_profiles().iter().any(|(k, _)| k == &id) {
                    return Err(VdfsError::Forbidden(format!("内置沙箱「{id}」不可删除")));
                }
                self.store().entry(&id).await?;
                self.store().remove(&id).await?;
                self.reload().await;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Action { .. } => Err(VdfsError::invalid(format!("{LABEL}不支持动作"))),
            VdfsRequest::Mkdir => Err(VdfsError::NotImplemented),
            VdfsRequest::Watch { sink } => {
                vdfs_watch_changes(PLUGIN_ID_SANDBOX, path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                vdfs_unwatch_changes(PLUGIN_ID_SANDBOX, path).await?;
                Ok(VdfsResponse::Unit)
            }
        }
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_SANDBOX, SandboxPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
