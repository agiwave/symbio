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
use symbio_plugin_api::VdfsCaller;
use symbio_plugin_api::{
    plugin_dir_from_ctx, sandbox_publish, vdfs_host_ctx, vdfs_unwatch_changes, vdfs_watch_changes,
    Plugin, PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload, SandboxFace, VdfsAccess, VdfsActionResult,
    VdfsContent, VdfsContext, VdfsError, VdfsNewType, VdfsNode, VdfsProvider, VdfsRequest,
    VdfsResponse, VdfsResult, VdfsValidationError, VdfsWriteResponse, PLUGIN_ID_SANDBOX,
    VDFS_CALLER, VDFS_EXT_FORM, VDFS_STATUS_NONE,
};
use symbio_providers::{vdfs_id_of, vdfs_safe_segment, SingleFileVdfs};

use crate::detail::sandbox_detail_definition;
use crate::engine::{Engine, SandboxProfileConfig, SandboxRegistry, SandboxValidationError};

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
                description: Some("交互模式 + 低风险阈值，仅低风险操作自动放行".into()),
                builtin: true,
                approval_mode: Some("interactive".into()),
                risk_threshold: Some("low".into()),
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

/// 详情定义（JSON 形态）——**唯一出处**：档案节点的 `schema` 与新建类型的 `schema`
/// 都读它，因此「点新建」的草稿表单与「选中一项」的详情表单是同一张。
/// 与 mcp / skill 的 `detail_definition()` 同形；缺了它，form 渲染器拿不到 `sections`。
fn detail_definition() -> serde_json::Value {
    serde_json::to_value(sandbox_detail_definition()).unwrap_or(serde_json::Value::Null)
}

/// 管理面只接受宿主显式注入的可信 Manager；模型与未知主体一律拒绝。
fn ensure_management_context(ctx: &VdfsContext) -> VdfsResult<()> {
    let host = vdfs_host_ctx(ctx)
        .map_err(|_| VdfsError::Forbidden("沙箱管理缺少可信宿主上下文".into()))?;
    if host.get(VDFS_CALLER) != Some(VdfsCaller::Manager) {
        return Err(VdfsError::Forbidden("只有管理界面可以修改沙箱策略".into()));
    }
    Ok(())
}

fn invalid_profile(errors: Vec<SandboxValidationError>) -> VdfsError {
    let mut validation = VdfsValidationError::new("沙箱档案不合法");
    for error in errors {
        validation = validation.with_field(error.field, error.message);
    }
    VdfsError::Invalid(validation)
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
    reload_lock: Arc<tokio::sync::Mutex<()>>,
}

impl SandboxPlugin {
    /// 静态工厂（满足 `submit_object_creator!` 协议）。
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_SANDBOX);
        Arc::new(SandboxPlugin {
            dir,
            registry: Arc::new(RwLock::new(SandboxRegistry::default())),
            reload_lock: Arc::new(tokio::sync::Mutex::new(())),
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
        vdfs_safe_segment(&vdfs_id_of(path.trim_matches('/'), PLUGIN_ID_SANDBOX))
    }

    /// 读完全部档案并构造/校验候选集；任一条读取、解析或校验失败都不发布。
    async fn load_registry(&self) -> Result<SandboxRegistry, String> {
        let entries = self
            .store()
            .entries()
            .await
            .map_err(|e| format!("读取沙箱档案失败：{e}"))?;
        let mut profiles = builtin_profiles();
        for entry in entries {
            let raw = entry
                .raw
                .ok_or_else(|| format!("读取沙箱档案 {} 失败：主文件不可用", entry.id))?;
            let cfg = serde_json::from_str::<SandboxProfileConfig>(&raw)
                .map_err(|e| format!("解析沙箱档案 {} 失败：{e}", entry.id))?;
            profiles.push((entry.id, cfg));
        }
        SandboxRegistry::try_from_profiles(profiles).map_err(|errors| {
            errors
                .iter()
                .map(|error| format!("{}：{}", error.field, error.message))
                .collect::<Vec<_>>()
                .join("；")
        })
    }

    /// 事务性重载：完整候选集通过后才在同一个写锁内替换已知良好快照。
    async fn reload(&self) {
        let _reload = self.reload_lock.lock().await;
        if let Err(error) = self.reload_locked().await {
            let has_snapshot = self
                .registry
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .has_snapshot();
            if has_snapshot {
                symbio_plugin_api::plugin_warn!(
                    "sandbox",
                    "重载沙箱档案失败，保留上一份快照：{error}"
                )
            } else {
                symbio_plugin_api::plugin_warn!(
                    "sandbox",
                    "首次加载沙箱档案失败，没有可用快照，执行将失败关闭：{error}"
                )
            }
        }
    }

    async fn reload_locked(&self) -> Result<(), String> {
        match self.load_registry().await {
            Ok(next) => {
                let mut registry = self.registry.write().unwrap_or_else(|p| p.into_inner());
                registry.replace(next);
                Ok(())
            }
            Err(error) => {
                let mut registry = self.registry.write().unwrap_or_else(|p| p.into_inner());
                registry.reload_error = Some(error.clone());
                Err(error)
            }
        }
    }

    async fn previous_profile(&self, id: &str) -> VdfsResult<Option<String>> {
        match self.store().entry(id).await {
            Ok(entry) => entry
                .raw
                .map(Some)
                .ok_or_else(|| VdfsError::internal(format!("沙箱档案「{id}」原文不可读"))),
            Err(VdfsError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// 调用方持有 `reload_lock`；候选集发布失败时恢复本次变更，不能向管理面报假成功。
    async fn reload_after_mutation(&self, id: &str, previous: Option<String>) -> VdfsResult<()> {
        if let Err(error) = self.reload_locked().await {
            let rollback = match previous {
                Some(text) => self.store().write_text(id, &text).await.map(|_| ()),
                None => self.store().remove(id).await,
            };
            match rollback {
                Ok(()) => {
                    if let Err(rollback_reload_error) = self.reload_locked().await {
                        symbio_plugin_api::plugin_warn!(
                            "sandbox",
                            "沙箱档案「{id}」已回滚但重载仍失败：{rollback_reload_error}"
                        )
                    }
                    Err(VdfsError::internal(format!(
                        "沙箱档案「{id}」已回滚；重载失败：{error}"
                    )))
                }
                Err(rollback_error) => Err(VdfsError::internal(format!(
                    "沙箱档案「{id}」重载失败且回滚失败：{error}；回滚错误：{rollback_error}"
                ))),
            }
        } else {
            Ok(())
        }
    }

    /// 管理写入的定义校验 + 类型/语义校验；错误保留表单可定位的字段路径。
    fn parse_profile(value: &serde_json::Value) -> VdfsResult<SandboxProfileConfig> {
        let definition = sandbox_detail_definition();
        definition.validate(value).map_err(VdfsError::Invalid)?;
        Self::validate_rule_table_options(&definition, value)?;
        let cfg: SandboxProfileConfig = serde_json::from_value(value.clone()).map_err(|e| {
            VdfsError::Invalid(VdfsValidationError::new(format!(
                "沙箱档案字段类型不合法：{e}"
            )))
        })?;
        let errors = cfg.validate();
        if errors.is_empty() {
            Ok(cfg)
        } else {
            Err(invalid_profile(errors))
        }
    }

    /// 通用 table 校验只保证单元格是标量；select 列的候选集合由本插件补齐校验。
    fn validate_rule_table_options(
        definition: &symbio_plugin_api::DetailDefinition,
        value: &serde_json::Value,
    ) -> VdfsResult<()> {
        let Some(rows) = value.get("rules").and_then(serde_json::Value::as_array) else {
            return Ok(());
        };
        let Some(rules) = definition
            .sections
            .iter()
            .flat_map(|section| section.fields.iter())
            .find(|field| field.key == "rules")
        else {
            return Ok(());
        };
        let mut validation = VdfsValidationError::new("规则表格不合法");
        for (i, row) in rows.iter().enumerate() {
            let Some(row) = row.as_object() else {
                continue; // 通用定义校验已报告行形状
            };
            for column in &rules.columns {
                let Some(cell) = row.get(&column.key) else {
                    continue; // 通用定义校验已报告缺列
                };
                let field = format!("rules.{i}.{}", column.key);
                let Some(text) = cell.as_str() else {
                    validation = validation.with_field(field, "必须是字符串");
                    continue;
                };
                // kind 是开放事实种类，只由语义校验要求非空，不用 UI 候选封死。
                if column.key != "kind"
                    && !column.options.is_empty()
                    && !column.options.iter().any(|option| option.value == text)
                {
                    validation = validation.with_field(
                        field,
                        format!(
                            "必须是以下之一：{}",
                            column
                                .options
                                .iter()
                                .map(|option| option.value.as_str())
                                .collect::<Vec<_>>()
                                .join(" / ")
                        ),
                    );
                }
            }
        }
        if validation.has_fields() {
            Err(VdfsError::Invalid(validation))
        } else {
            Ok(())
        }
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
        // 与 mcp / skill 同形：把描述下发到列表卡片与右栏副标题。
        node.description = cfg.description.clone();
        // 节点自携带详情定义：form 渲染器的 definition 完全来自 node.schema。
        // 漏挂会让右栏拿到空定义（无 sections）而白屏，标题也回退成「新建沙箱」。
        node.schema = Some(detail_definition());
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
                let warning = {
                    let reg = self.registry.read().unwrap_or_else(|p| p.into_inner());
                    reg.reload_error.clone()
                };
                let has_snapshot = self
                    .registry
                    .read()
                    .unwrap_or_else(|p| p.into_inner())
                    .has_snapshot();
                let field = crate::detail::sandbox_option_field(
                    &archives,
                    selected.as_deref(),
                    warning.as_deref(),
                    has_snapshot,
                );
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
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        if matches!(
            &req,
            VdfsRequest::Write { .. } | VdfsRequest::Delete { .. } | VdfsRequest::Action { .. }
        ) {
            ensure_management_context(ctx)?;
        }
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
                                .with_schema(detail_definition()),
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
                // 表单的 json 字段直接消费结构化值（allowed_facts / blocked_tools 已在
                // cfg 里）；不再注入 `_json` 字符串——那层转换是表单机制没有 json
                // widget 时的历史绕法，现已由通用 widget 承担。
                let value = serde_json::to_value(cfg)
                    .map_err(|e| VdfsError::internal(format!("序列化沙箱详情失败：{e}")))?;
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
                    vdfs_safe_segment(&symbio_providers::vdfs_auto_id(PLUGIN_ID_SANDBOX))
                } else {
                    Self::id_of(path)
                };
                // 地址先按存储口径规范化，再保护内置 id；路径字符串不是授权依据。
                if builtin_profiles().iter().any(|(k, _)| k == &id) {
                    return Err(VdfsError::Forbidden(format!(
                        "内置沙箱「{id}」只读；请复制后修改"
                    )));
                }
                let text = content.as_text().unwrap_or_default();
                if text.trim().is_empty() {
                    return Err(VdfsError::Invalid(
                        VdfsValidationError::new("沙箱档案不能为空")
                            .with_field("content", "请提交完整的沙箱档案 JSON"),
                    ));
                }
                let mut value: serde_json::Value = serde_json::from_str(text)
                    .map_err(|e| VdfsError::invalid(format!("沙箱档案不是合法 JSON：{e}")))?;
                // json widget 提交的是结构化值（allowed_facts / blocked_tools 已是对象），
                // serde 直接反序列化即可；留空的 json 字段被前端省略 ⇒ `#[serde(default)]`
                // 的空 HashMap 生效（留空 = 不限制）。旧的 `_json` 字符串转换已随机制增强删除。
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("builtin".into(), serde_json::json!(false));
                }
                let cfg = Self::parse_profile(&value)?;
                let _reload = self.reload_lock.lock().await;
                let previous = self.previous_profile(&id).await?;
                let created = self
                    .store()
                    .write_json(&id, &serde_json::to_value(&cfg).unwrap_or_default())
                    .await?;
                self.reload_after_mutation(&id, previous).await?;
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
                let _reload = self.reload_lock.lock().await;
                let previous = self.previous_profile(&id).await?;
                self.store().remove(&id).await?;
                self.reload_after_mutation(&id, previous).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Action { action, .. } => {
                if action != crate::detail::ACTION_COPY {
                    return Err(VdfsError::invalid(format!("{LABEL}不支持动作：{action}")));
                }
                let id = Self::id_of(path);
                let _reload = self.reload_lock.lock().await;
                let mut cfg = {
                    let reg = self.registry.read().unwrap_or_else(|p| p.into_inner());
                    reg.archives()
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| VdfsError::not_found(format!("未找到{LABEL}档案：{id}")))?
                };
                cfg.builtin = false;
                cfg.name = format!("{}（副本）", cfg.name);
                let cfg = Self::parse_profile(&serde_json::to_value(cfg).map_err(|error| {
                    VdfsError::internal(format!("序列化沙箱档案失败：{error}"))
                })?)?;
                let new_id = vdfs_safe_segment(&symbio_providers::vdfs_auto_id(PLUGIN_ID_SANDBOX));
                self.store()
                    .write_json(&new_id, &serde_json::to_value(&cfg).unwrap_or_default())
                    .await?;
                self.reload_after_mutation(&new_id, None).await?;
                Ok(VdfsResponse::Action(VdfsActionResult {
                    action,
                    ok: true,
                    message: format!("已创建沙箱「{}」", cfg.name),
                    data: Some(serde_json::json!({ "id": new_id })),
                }))
            }
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
