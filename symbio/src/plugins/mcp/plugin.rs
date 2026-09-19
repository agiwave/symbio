//! MCP (Model Context Protocol) 插件实现
//!
//! ## 职责划分
//!
//! - **后端**（本插件）：MCP **配置**（CRUD）+ **客户端 transport**（stdio/http）
//! - **前端**（`tauri`）：**仅**负责 MCP Server 的配置管理（CRUD UI），
//!   不实现任何 transport 客户端
//!
//! ## 系统工具机制集成
//!
//! 与 [`web`](crate::plugins::web) 插件完全一致：
//! - 静态插件持有 `manager: Arc<McpManager>`（无状态 transport 路由器）
//! - `traverse(TRAVERSE_AVAILABLE_TOOLS)` 时遍历 `McpConfig.servers`，
//!   对每个 enabled server 调用 `manager.discover_tools` 动态发现工具，
//!   把每个工具包装为 [`McpToolCapability`] 注册到 `ctx.get(CAPABILITY_VISITOR)`
//! - agent 通过 `tool_visitor.invoke("mcp.<server>.<tool>", ctx)` 调用
//!
//! ## 存储策略
//!
//! 每个 MCP Server 是一个**目录型条目**：`<本插件目录>/<name>/server.json`
//! （主文件 `server.json` + 可选附属文件），由
//! [`DirVdfs`](crate::providers::vdfs_service::DirVdfs) 承载落盘。
//! `McpConfig` 的内存视图（`servers: HashMap<name, McpServerConfig>`）
//! 通过从磁盘加载/回写保持一致。

pub use crate::plugins::mcp::schemas::mcp_config::{McpConfig, McpServerConfig};
use crate::providers::vdfs_service::DirVdfs;
use crate::symbio_core::{
    dir_from_ctx, Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse,
    Plugin, PluginDir, PluginError, PluginMeta, PluginPayload, PLUGIN_MCP,
};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::warn;

use super::capability::McpToolCapability;
use super::manager::McpManager;

/// Server 主文件（磁盘布局：`<本插件目录>/<id>/server.json`）
const MANIFEST: &str = "server.json";

/// MCP 插件
pub struct McpPlugin {
    /// 配置 (server_name -> config)
    config: Arc<RwLock<McpConfig>>,
    /// MCP transport 路由器（无状态，跨调用共享）
    manager: Arc<McpManager>,
    /// 首次加载标志（防止 traverse 在 load_from_storage 完成前访问旧 config）
    loaded: Arc<tokio::sync::Mutex<bool>>,
    /// 本插件的目录（`<本插件目录>`）——配置文件 `PLUGIN.yml` 就在这里
    dir: PluginDir,
}

impl McpPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        // 自己的目录由容器经 `PLUGIN_DIR` 告知；旧形态残留的 `servers` 明细可能还在
        // 那里的 `PLUGIN.yml` 里，由 `load_from_storage` 搬成资源
        let dir = dir_from_ctx(&*ctx, PLUGIN_MCP);
        let config: McpConfig = match dir.load::<McpConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => McpConfig::default(),
            Err(e) => {
                crate::plugin_warn!("mcp", "读取自身配置失败，改用默认值：{e}");
                McpConfig::default()
            }
        };

        let plugin = Arc::new(McpPlugin::new(config, dir));

        // 启动后异步触发：从存储加载（并触发首启动数据迁移）
        let plugin_weak = Arc::downgrade(&plugin);
        let ctx_clone = ctx.clone();
        tokio::spawn(async move {
            if let Some(mcp) = plugin_weak.upgrade() {
                mcp.load_from_storage(&ctx_clone).await;
            }
        });

        plugin as Arc<dyn Plugin>
    }

    /// 主构造函数
    pub fn new(config: McpConfig, dir: PluginDir) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            manager: Arc::new(McpManager::new()),
            loaded: Arc::new(tokio::sync::Mutex::new(false)),
            dir,
        }
    }

    /// 磁盘底座（每次现取，跟随 homedir 切换）
    ///
    /// 根 = **本插件自己的目录**（构造时由父插件经 `PLUGIN_DIR` 告知）——
    /// 这里不按插件名反推落位，插件不知道、也不该知道自己被放在哪。
    fn store(&self) -> DirVdfs {
        DirVdfs::at(self.dir.dir(), PLUGIN_MCP, MANIFEST).with_label(LABEL)
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("mcp", "MCP 工具集成")
            .with_description("提供与 MCP 服务器的连接和交互功能")
            .with_version("0.3.0")
    }

    /// 异步加载：从 `<本插件目录>/` 读取所有 MCP Server
    ///
    /// - 若存储为空，则触发首启动迁移（从 ctx.config()）
    pub async fn load_from_storage(&self, _ctx: &Arc<dyn InvokeRequest>) {
        let store = self.store();

        // 1. 存储中的所有 MCP Server
        let entries = match store.entries().await {
            Ok(v) => v,
            Err(_e) => {
                crate::plugin_warn!("mcp", "list mcps 失败");
                return;
            }
        };

        // 2. 如果存储为空，触发首启动迁移
        if entries.is_empty() {
            self.migrate_from_legacy_config(&store).await;
            return;
        }

        // 3. 加载存储内容
        let mut new_servers = std::collections::HashMap::new();
        for e in &entries {
            match e
                .raw
                .as_deref()
                .and_then(|c| serde_json::from_str::<McpServerConfig>(c).ok())
            {
                Some(s) => {
                    new_servers.insert(e.id.clone(), s);
                }
                None => crate::plugin_warn!("mcp", "解析 server {} 失败", e.id),
            }
        }

        let mut cfg = self.config.write().await;
        cfg.servers = new_servers;
        crate::plugin_info!(
            "mcp",
            "从 <本插件目录>/ 加载了 {} 个 MCP Server",
            cfg.servers.len()
        );
    }

    /// 幂等保证：首次调用时阻塞执行 `load_from_storage`，后续调用直接返回
    ///
    /// 用于在 `route` / `traverse` 入口确保 `cfg.servers` 反映磁盘最新状态，
    /// 避免与 `build` 中 spawn 的异步加载发生时序竞争。
    async fn ensure_loaded(&self, ctx: &Arc<dyn InvokeRequest>) {
        let mut guard = self.loaded.lock().await;
        if !*guard {
            self.load_from_storage(ctx).await;
            *guard = true;
        }
    }

    /// 首启动迁移：把配置文件里残留的旧 Server 明细迁到存储
    ///
    /// 旧形态把 Server 整包存在配置里（`PLUGIN.yml` 的 `servers` 键）。迁移把它们
    /// 写成 `<本插件目录>/<id>/server.json`，随后把配置文件里这些遗留键摘掉——
    /// 本插件**没有跨条目配置**（它的配置就是资源树），配置文件只剩身份字段。
    async fn migrate_from_legacy_config(&self, store: &DirVdfs) {
        let current = self.config.read().await.clone();
        if current.servers.is_empty() {
            return;
        }

        crate::plugin_info!(
            "mcp",
            "检测到旧 config 中的 MCP Servers，开始迁移到 <本插件目录>/"
        );

        for (id, s) in &current.servers {
            let content = match serde_json::to_string_pretty(s) {
                Ok(s) => s,
                Err(_e) => {
                    crate::plugin_error!("mcp", "序列化 server {id} 失败");
                    continue;
                }
            };
            if let Err(_e) = store.write_text(id, &content).await {
                crate::plugin_error!("mcp", "迁移 server {id} 失败");
            }
        }

        // 明细已是资源：配置文件里不该再留一份（`_storage` 是更早形态的遗留键）
        if let Err(e) = self.dir.remove_keys(&["servers", "_storage"]) {
            crate::plugin_warn!("mcp", "清理配置文件中的遗留字段失败：{e}");
        }
    }
}

impl Default for McpPlugin {
    fn default() -> Self {
        Self::new(McpConfig::default(), PluginDir::of(PLUGIN_MCP))
    }
}

// ==================== VDFS 挂载点（`.vdfs/mcp`） ====================
//
// 本插件**直接实现 `VdfsProvider`**：VDFS 是唯一协议、唯一地址空间，列 / 读 /
// 写 / 删 / 动作的语义都在这里表达。
//
// 存储走 `providers::vdfs_service::DirVdfs`（一个 server = 一个目录，主文件
// `server.json`）——条目寻址、原子写、mtime、整包 zip、变更广播都在集中实现里。
// 本模块只剩 **mcp 特有的三件事**：详情定义随节点下发、transport 必填项校验、
// 写后把 server 回灌进内存 config 与 manager 缓存。

use crate::symbio_core::vdfs::{from_plugin_error, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsActionResult, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNewType,
    VdfsNode, VdfsProvider, VdfsResult, VdfsWriteResponse, VDFS_ACTION_EXPORT, VDFS_ACTION_TEST,
    VDFS_EXT_FORM, VDFS_EXT_ZIP, VDFS_NEW_SOURCE_FILE, VDFS_STATUS_ACTIVE, VDFS_STATUS_DISABLED,
    VDFS_STATUS_UNKNOWN,
};

const LABEL: &str = "MCP";

/// 路径末段 → 条目 id（去掉 `.mcp` 呈现扩展名）
fn id_of(path: &str) -> String {
    crate::providers::vdfs_service::entry::id_of(path, PLUGIN_MCP)
}

/// 目标地址 → 条目 id（`write` 与测试共用的**唯一**判据）。
///
/// 末段非空 ⇒ 名字由使用方给；地址为空 ⇒ **使用方没给名字**（写挂载点目录自身，
/// 即「点新建 → 在详情页填好 → 保存」），id 由本插件生成。目录自身没有可覆盖的
/// 目标，所以必须带 `create` 意图（见 [`VdfsProvider::write`]）。
fn resolve_id(path: &str, create: bool) -> VdfsResult<String> {
    if !path.trim_matches('/').is_empty() {
        return Ok(id_of(path));
    }
    if !create {
        return Err(VdfsError::invalid(format!(
            "写{LABEL}挂载根需要 create 意图：目录自身没有可覆盖的目标"
        )));
    }
    Ok(crate::providers::vdfs_service::entry::auto_id(PLUGIN_MCP))
}

/// 导入的**建议名**：末段再去掉 `.zip`（新建地址是 `<name>.zip`；id 的最终解释权
/// 仍在插件——整包导入时以包内 server.json 为准）
fn import_name_of(path: &str) -> String {
    crate::providers::vdfs_service::entry::pack_name_of(path, PLUGIN_MCP)
}

/// 主文件原文 → VDFS 节点（`ext = form` + 详情定义随节点 `schema` 下发）
///
/// `server.json` **没有展示名字段**（`McpServerConfig` 只有 transport 相关字段），
/// 因此标题就是条目 id——与原摘要口径一致；`transport` 进 `attributes` 供列表
/// 卡片直接呈现，副标题取 `command` / `url`。
/// 详情定义（JSON 形态）——**唯一出处**：节点 `schema` 与新建类型 `schema` 都读它，
/// 因此「点新建」的草稿表单与「选中一项」的详情表单是同一张。
fn detail_definition() -> serde_json::Value {
    serde_json::to_value(super::detail::mcp_detail_definition()).unwrap_or(serde_json::Value::Null)
}

fn node_of(id: &str, raw: Option<&str>) -> VdfsNode {
    let mut n = VdfsNode::file(id, id, VdfsAccess::READ_WRITE);
    n.kind = PLUGIN_MCP.to_string();
    n.ext = Some(VDFS_EXT_FORM.to_string());
    n.schema = Some(detail_definition());
    let Some(server) = raw.and_then(|c| serde_json::from_str::<McpServerConfig>(c).ok()) else {
        // 坏条目降级：以 id 呈现、状态未知，但**仍在列表里**（可点开看到原文再修）
        n.status = VDFS_STATUS_UNKNOWN.to_string();
        return n;
    };
    n.status = if server.enabled {
        VDFS_STATUS_ACTIVE.to_string()
    } else {
        VDFS_STATUS_DISABLED.to_string()
    };
    n.description = server.command.clone().or_else(|| server.url.clone());
    n.attributes.insert(
        "transport".to_string(),
        format!("{:?}", server.transport_type).to_lowercase().into(),
    );
    n
}

/// 表单 manifest → `McpServerConfig`（即 server.json 内容）——写盘前的校验/规范化。
///
/// 表单 manifest 与 server.json 同构（`type` = 传输类型），多余键（`name`/`id` 等）
/// 由 serde 忽略；此处校验 transport 必填项并序列化回规范化 JSON。
/// zip 导入路径（目录内 server.json 原文）不经过本函数。
fn validate_manifest(manifest: &serde_json::Value) -> Result<serde_json::Value, PluginError> {
    let server: McpServerConfig = serde_json::from_value(manifest.clone())
        .map_err(|e| PluginError::ValidationError(format!("MCP Server 配置无效: {e}")))?;
    match server.transport_type {
        crate::plugins::mcp::schemas::mcp_config::McpTransportType::Stdio => {
            if server.command.as_deref().unwrap_or("").trim().is_empty() {
                return Err(PluginError::ValidationError(
                    "stdio 类型必须填写 command".to_string(),
                ));
            }
        }
        _ => {
            if server.url.as_deref().unwrap_or("").trim().is_empty() {
                return Err(PluginError::ValidationError(
                    "http / sse 类型必须填写 url".to_string(),
                ));
            }
        }
    }
    serde_json::to_value(&server)
        .map_err(|e| PluginError::InternalError(format!("序列化 server.json 失败: {e}")))
}

/// `write { create }` 的最小清单。
///
/// 校验要求 stdio 有 `command`、http/sse 有 `url`；这里给一份 stdio 骨架，
/// 用户随后在详情里补命令参数，或改用其它传输类型。
fn new_manifest(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": id,
        "type": "stdio",
        "command": "npx",
        "args": ["-y"],
    })
}

#[async_trait]
impl VdfsProvider for McpPlugin {
    fn label(&self) -> Option<&str> {
        Some(LABEL)
    }

    fn description(&self) -> Option<&str> {
        Some("MCP server 配置（每项一份 server.json），是工具的唯一来源。")
    }

    fn order(&self) -> i32 {
        5
    }

    fn icon(&self) -> Option<&str> {
        Some(PLUGIN_MCP)
    }

    /// 根下可新建两类：表单新建 + 整包导入（zip）
    ///
    /// `ext = mcp` 是**呈现扩展名**（`id_of` 按它剥地址后缀），落成后的节点
    /// `ext = form`——两者不同，故显式声明 `node_ext` 与详情定义（草稿详情页据此
    /// 渲染出与落成后同一张表单）。
    fn root_new_types(&self) -> Vec<VdfsNewType> {
        vec![
            VdfsNewType::new(PLUGIN_MCP, LABEL)
                .with_description("新建 MCP Server（在详情页里填好，保存时一次写入）")
                .with_node_ext(VDFS_EXT_FORM)
                .with_schema(detail_definition()),
            VdfsNewType::new(VDFS_EXT_ZIP, "MCP 包")
                .with_description("导入 MCP Server 整包（.zip）——整目录覆盖同名条目")
                .with_source(VDFS_NEW_SOURCE_FILE),
        ]
    }

    async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        if !path.is_empty() {
            return Err(VdfsError::not_found(format!(
                "{LABEL}是叶子资源，没有子项：{path}"
            )));
        }
        Ok(self
            .store()
            .entries()
            .await?
            .iter()
            .map(|e| node_of(&e.id, e.raw.as_deref()))
            .collect())
    }

    async fn stat(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        if path.is_empty() {
            // 自身根：名字留空——provider 不知道自己的挂载名，由使用方回填
            return Ok(VdfsNode::dir("", LABEL, VdfsAccess::LIST));
        }
        let e = self.store().entry(&id_of(path)).await?;
        Ok(node_of(&e.id, e.raw.as_deref()))
    }

    /// 详情读的是**表单能填的形状**（server.json），条目内部文件走
    /// `read(<id>/<rel>)` 这一真实地址（目录型天然支持）。
    async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        if path.is_empty() {
            return Err(VdfsError::invalid(format!(
                "该路径是目录，不可读取内容：{path}"
            )));
        }
        let text = self.store().read_text(&id_of(path)).await?;
        Ok(VdfsContent::text(path, text).with_mime("application/json"))
    }

    async fn write(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let store = self.store();
        // 二进制写入 = 整包导入（导入不是第二条协议，它就是「新建」的一种内容来源）。
        // 导入的**名字来自目标地址末段**（使用方由文件名推导），所以必须有名字：
        // 「无名字导入」无从命名，直接拒绝。
        if content.binary {
            if path.trim_matches('/').is_empty() {
                return Err(VdfsError::invalid(format!(
                    "{LABEL}整包导入需要目标名（地址末段）：{path}"
                )));
            }
            let bytes = crate::providers::vdfs_service::decode_b64(
                content.b64.as_deref().unwrap_or_default(),
            )
            .map_err(|e| VdfsError::invalid(e.0))?;
            let name = import_name_of(path);
            let created = store.import_pack(&name, &bytes).await?;
            self.reload_server_from_storage(&name).await?;
            return Ok(VdfsWriteResponse {
                path: name,
                created,
                etag: None,
            });
        }
        // 无名字（写在挂载点目录自身）→ 「新建一项，名字由本插件生成」。
        // 目录自身没有可覆盖的目标，因此必须有 create 意图（见 `VdfsProvider::write`）。
        let id = resolve_id(path, content.create)?;
        let text = content.as_text().unwrap_or_default();
        // `create` 只管「不存在时怎么办」，**不改变内容的处理方式**：草稿详情页
        // 填好的字段必须原样落盘。唯一例外是**内容为空**——「先建一个，随后再填」
        // 是合法形态，此时落一份最小配置。
        let manifest = if content.create && text.trim().is_empty() {
            // 使用方只给了地址（或连名字都没有），最小配置由本插件自持
            new_manifest(&id)
        } else {
            serde_json::from_str::<serde_json::Value>(text)
                .map_err(|e| VdfsError::invalid(format!("manifest 不是合法 JSON：{e}")))?
        };
        // 「条目 id 由路径承载」：编辑链路下发的 manifest 不含 id，校验前先补齐
        // （`validate_manifest` 会反序列化到 id 必填的结构体）
        let manifest = with_id(&manifest, &id);
        let normalized = validate_manifest(&manifest).map_err(from_plugin_error)?;
        let created = store.write_json(&id, &normalized).await?;
        // 落盘成功后把 server 从磁盘回灌到内存 config（并失效相关缓存）
        self.reload_server_from_storage(&id).await?;
        Ok(VdfsWriteResponse {
            path: id,
            created,
            etag: None,
        })
    }

    async fn delete(&self, _ctx: &VdfsContext, path: &str, _recursive: bool) -> VdfsResult<()> {
        if path.is_empty() {
            return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
        }
        let store = self.store();
        let id = id_of(path);
        // 存在性校验：删除不存在的条目应报 NotFound 而非静默成功
        store.entry(&id).await?;
        store.remove(&id).await?;
        self.forget_server(&id).await;
        Ok(())
    }

    async fn action(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        action: &str,
        _payload: Option<&serde_json::Value>,
    ) -> VdfsResult<VdfsActionResult> {
        if path.is_empty() {
            return Err(VdfsError::invalid(format!(
                "动作只对{LABEL}条目可用：{path}"
            )));
        }
        let id = id_of(path);
        // 存在性校验：对不存在的条目做动作应报 NotFound
        self.store().entry(&id).await?;
        match action {
            // 连接测试：stdio 握手 / http streams。连接失败映射为 ok=false（失败是
            // **结果**，不是协议错误）
            VDFS_ACTION_TEST => {
                let server = self.server_config(&id).await?;
                let (ok, message) = match self.manager.test_connection(&id, &server).await {
                    Ok(r) => (
                        true,
                        format!(
                            "{tools} tools · protocol {protocol}",
                            tools = r.tool_count,
                            protocol = r.protocol_version
                        ),
                    ),
                    Err(e) => (false, e),
                };
                Ok(VdfsActionResult {
                    action: action.to_string(),
                    ok,
                    message,
                    data: None,
                })
            }
            // 导出：整包打包下载（与「新建类型 zip」的导入互为逆向）
            VDFS_ACTION_EXPORT => {
                let pack = self.store().export_pack(&id).await?;
                let data = serde_json::to_value(&pack)
                    .map_err(|e| VdfsError::internal(format!("导出结果序列化失败: {e}")))?;
                Ok(VdfsActionResult {
                    action: action.to_string(),
                    ok: true,
                    message: format!("已打包「{}」", pack.filename),
                    data: Some(data),
                })
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }

    async fn watch(&self, _ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        watch_changes(PLUGIN_MCP, path, sink).await
    }

    async fn unwatch(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        unwatch_changes(PLUGIN_MCP, path).await
    }
}

/// manifest 缺 `id`（或空串）时以路径段补全，已有值原样保留
///
/// 这是写入路径的不变量：`DetailForm` 的 option 绑定只回纯字段值（id 不在表单
/// 字段里），而 `McpServerConfig` 的 id 是必填 ⇒ 不补就报「missing field `id`」。
fn with_id(manifest: &serde_json::Value, id: &str) -> serde_json::Value {
    let mut m = manifest.clone();
    let missing = m
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty();
    if missing {
        if let serde_json::Value::Object(map) = &mut m {
            map.insert("id".to_string(), serde_json::json!(id));
        }
    }
    m
}

impl McpPlugin {
    /// 从存储读取并解析单个 server 配置
    async fn server_config(&self, id: &str) -> VdfsResult<McpServerConfig> {
        let content = self.store().read_text(id).await?;
        serde_json::from_str::<McpServerConfig>(&content)
            .map_err(|e| VdfsError::internal(format!("解析 {id} 配置失败: {e}")))
    }

    /// 写盘 / 删除后清理内存 config 与 manager 缓存
    async fn forget_server(&self, id: &str) {
        {
            let mut cfg = self.config.write().await;
            cfg.servers.remove(id);
        }
        self.manager.forget_server(id).await;
    }

    /// 落盘后把单个 server 从磁盘回灌到内存 config（并失效相关缓存）
    pub async fn reload_server_from_storage(&self, name: &str) -> VdfsResult<()> {
        let server = self.server_config(name).await?;
        {
            let mut cfg = self.config.write().await;
            cfg.servers.insert(name.to_string(), server);
        }
        self.manager.invalidate_discover_cache(name).await;
        self.manager.session_cache.remove(name).await;
        Ok(())
    }
}

#[async_trait]
impl Plugin for McpPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        self.ensure_loaded(&ctx).await;

        let sub_path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        if sub_path != crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS {
            return Err(crate::symbio_core::PluginError::NotFound(format!(
                "未知遍历路径: {}",
                sub_path
            )));
        }

        // 仅当上游传入了 CAPABILITY_VISITOR 时才注册
        let Some(tool_visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) else {
            return Ok(PluginPayload::new(&Vec::<CapabilityMeta>::new()));
        };

        // 直接注册插件自身为 VDFS 挂载点（不经实体层与适配器）
        let vdfs_provider: Arc<dyn crate::symbio_core::vdfs_provider::VdfsProvider> = self.clone();
        tool_visitor
            .register_vdfs_provider(PLUGIN_MCP, vdfs_provider)
            .await;

        let cfg = self.config.read().await.clone();

        // 遍历已启用的 server；动态 discover 并注册
        for (name, server_cfg) in &cfg.servers {
            if !server_cfg.enabled {
                continue;
            }
            match self.manager.discover_tools(name, server_cfg).await {
                Ok(tools) => {
                    for tool in tools {
                        let cap: Arc<dyn Capability> = Arc::new(McpToolCapability::new(
                            name.clone(),
                            tool,
                            server_cfg.clone(),
                            self.manager.clone(),
                        ));
                        tool_visitor.register(cap).await;
                    }
                }
                Err(e) => {
                    // 单个 server 失败不影响其它 server 的注册
                    warn!(server = name, error = %e, "MCP 工具发现失败，跳过该 server");
                }
            }
        }

        Ok(PluginPayload::new(&Vec::<CapabilityMeta>::new()))
    }

    /// mcp 已无自有路由：每个 MCP Server 都是 VDFS 上的一个可寻址条目
    /// （`.vdfs/mcp/<name>`，见 `impl VdfsProvider`），配置因此没有第二条入口。
    async fn route(self: Arc<Self>, _ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        Err(PluginError::NotFound(format!(
            "{PLUGIN_MCP} 已无自有路由，请改用 VDFS 地址"
        )))
    }
}

crate::submit_object_creator!(PLUGIN_MCP, McpPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
