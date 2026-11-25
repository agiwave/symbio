//! 作用域目录 provider —— 上下文三 provider 键（D-515）的**当前磁盘后端**。
//!
//! `ScopeDirVdfs` 把一个固定目录当作 VDFS 子树根：调用方只给相对地址
//! （`AGENTS.md`、`MEMORY.md`、`SKILL.md`…），本类型负责把地址落到根下，
//! 并挡住 `..` 与绝对路径。插件只见 `dyn VdfsProvider`，因此后端换成内存 /
//! 网络时插件无感——这正是 D-515 把「目录字符串」换成「存储接口」的收益。
//!
//! ## 为什么不放 vdfs 插件的 `physical.rs`
//!
//! `PhysicalFs` 从 `VdfsContext` 的 `workdir` 参数取根，是「访问层的物理半」：
//! 根随每次请求走参数进来。作用域 provider 的根**绑在实例上**（`ctx[WORKSPACE_VDFS]`
//! 里就带着它），两者生命周期与所有权不同，故独立实现，不复用那半边。
//!
//! ## 受控的物理根
//!
//! [`ScopeDirVdfs::physical_root`] 仅供宿主 / 执行层（`shell` / `hook` 的 cwd）
//! 使用，**不对扩展插件暴露**——插件拿到的是 `dyn VdfsProvider`，没有这个方法。
//! 这样「进程执行环境」能拿到真实目录，而「插件存数据」只走抽象接口。

use async_trait::async_trait;
use base64::Engine as _;
use std::path::{Path, PathBuf};
use symbio_platform::kernel::vdfs::{
    vdfs_has_parent_segment, VdfsAccess, VdfsContent, VdfsContext, VdfsError, VdfsNode,
    VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse,
};

/// 单次读取上限（与 `PhysicalFs` 同一口径，避免超大文件打爆响应）。
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
/// 单次列举上限。
const MAX_LIST_ENTRIES: usize = 2000;

/// 绑定到一个固定根的 VDFS provider。
#[derive(Debug, Clone)]
pub struct ScopeDirVdfs {
    root: PathBuf,
    /// 变更广播频道（约定 = 作用域名，`memory` / `session` / `workspace`）
    kind: String,
}

impl ScopeDirVdfs {
    /// 由根路径构造。根必须是绝对路径（构造方保证；这里不读全局 homedir）。
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            kind: String::new(),
        }
    }

    /// 带广播频道构造（供需要变更通知的作用域）。
    pub fn with_notify_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = kind.into();
        self
    }

    /// 受控的物理根 —— 只给宿主 / 执行层，不给扩展插件。
    pub fn physical_root(&self) -> &Path {
        &self.root
    }

    /// 相对地址 → 根目录内的真实路径。
    ///
    /// `..` 段与绝对路径一律拒绝：作用域 provider 只认自己子树内的地址，
    /// 不允许调用方跳出根（这也是 FB-002 的「数据目录不是事实源」边界）。
    fn resolve(&self, path: &str) -> VdfsResult<PathBuf> {
        if vdfs_has_parent_segment(path) {
            return Err(VdfsError::Invalid(
                symbio_platform::kernel::vdfs::VdfsValidationError::new(format!(
                    "作用域地址不允许父段：`{path}`"
                )),
            ));
        }
        let rel = Path::new(path);
        if rel.is_absolute() {
            return Err(VdfsError::Forbidden(format!(
                "作用域地址必须是相对路径：`{path}`"
            )));
        }
        Ok(self.root.join(rel))
    }

    /// 读守卫：拒绝符号链接（防逃逸），解析后复验仍在根内。
    async fn guard_read(&self, target: &Path, addr: &str) -> VdfsResult<PathBuf> {
        if let Ok(meta) = tokio::fs::symlink_metadata(target).await {
            if meta.file_type().is_symlink() {
                return Err(VdfsError::Forbidden(format!("拒绝读取符号链接：`{addr}`")));
            }
        }
        let resolved = tokio::fs::canonicalize(target)
            .await
            .map_err(|e| VdfsError::not_found(format!("路径不存在：`{addr}`：{e}")))?;
        // canonicalize 已解析所有链接；复验它仍在根内（root 本身也 canonicalize 一次）
        let root = tokio::fs::canonicalize(&self.root)
            .await
            .unwrap_or_else(|_| self.root.clone());
        if !resolved.starts_with(&root) {
            return Err(VdfsError::Forbidden(format!(
                "路径解析后超出作用域：`{addr}`"
            )));
        }
        Ok(resolved)
    }

    /// 写守卫：拒绝符号链接地址；已存在目标解析后必须在根内。
    async fn guard_write(&self, target: &Path) -> VdfsResult<()> {
        if let Ok(meta) = tokio::fs::symlink_metadata(target).await {
            if meta.file_type().is_symlink() {
                return Err(VdfsError::Invalid(
                    symbio_platform::kernel::vdfs::VdfsValidationError::new(format!(
                        "拒绝写入符号链接：{}",
                        target.display()
                    )),
                ));
            }
            let resolved = tokio::fs::canonicalize(target)
                .await
                .map_err(|e| VdfsError::internal(format!("解析路径失败：{e}")))?;
            let root = tokio::fs::canonicalize(&self.root)
                .await
                .unwrap_or_else(|_| self.root.clone());
            if !resolved.starts_with(&root) {
                return Err(VdfsError::Forbidden(format!(
                    "路径解析后超出作用域：{}",
                    target.display()
                )));
            }
        }
        Ok(())
    }

    fn node_from(name: String, is_dir: bool) -> VdfsNode {
        if is_dir {
            VdfsNode::dir(name.clone(), name, VdfsAccess::dir(true, true))
        } else {
            VdfsNode::file(name.clone(), name, VdfsAccess::file(true))
        }
    }

    /// 节点的最后修改时间（Unix **毫秒**）。`VdfsNode` 有这个字段，物理后端就该填——
    /// 否则消费方（`MemoryFile::updated_at`、按时间淘汰的存档、
    /// `codebase_search` 的增量索引指纹）永远只能拿到 `None`。
    ///
    /// 用毫秒而不是秒：`VdfsNode::updated_at` 的约定是「秒或毫秒都可，按量级判别」
    /// （`ts < 1e12` 视为秒），毫秒在约定内。 code index 的指纹要能分辨
    /// 「同一秒内、同样长度的内容改动」，秒级会漏检，毫秒把这条缝收窄 1000 倍。
    fn updated_at_of(meta: &std::fs::Metadata) -> Option<i64> {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
    }

    fn basename(path: &str) -> String {
        Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string()
    }

    /// 写后广播变更（有频道才广播）。
    fn notify(&self, path: &str) {
        if self.kind.is_empty() {
            return;
        }
        symbio_platform::sdk::vdfs::host::vdfs_notify_change(&self.kind, path);
    }
}

#[async_trait]
impl VdfsProvider for ScopeDirVdfs {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                let target = self.resolve(path)?;
                let resolved = self.guard_read(&target, path).await?;
                let mut rd = tokio::fs::read_dir(&resolved)
                    .await
                    .map_err(|e| VdfsError::invalid(format!("无法列举 `{path}`：{e}")))?;
                let mut nodes = Vec::new();
                while let Some(entry) = rd
                    .next_entry()
                    .await
                    .map_err(|e| VdfsError::internal(format!("遍历失败：{e}")))?
                {
                    let Ok(name) = entry.file_name().into_string() else {
                        continue;
                    };
                    let Ok(meta) = entry.metadata().await else {
                        continue;
                    };
                    let is_dir = meta.is_dir();
                    let mut node = Self::node_from(name, is_dir);
                    if !is_dir {
                        node.size = Some(meta.len());
                    }
                    node.updated_at = Self::updated_at_of(&meta);
                    nodes.push(node);
                    if nodes.len() >= MAX_LIST_ENTRIES {
                        break;
                    }
                }
                nodes.sort_by(|a, b| {
                    b.is_dir()
                        .cmp(&a.is_dir())
                        .then_with(|| a.name.cmp(&b.name))
                });
                Ok(VdfsResponse::list(nodes))
            }
            VdfsRequest::Stat => {
                let target = self.resolve(path)?;
                let resolved = self.guard_read(&target, path).await?;
                let meta = tokio::fs::metadata(&resolved)
                    .await
                    .map_err(|e| VdfsError::not_found(format!("无法读取元数据 `{path}`：{e}")))?;
                let name = if path.is_empty() {
                    self.root
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("作用域")
                        .to_string()
                } else {
                    Self::basename(path)
                };
                let mut node = Self::node_from(name, meta.is_dir());
                if !meta.is_dir() {
                    node.size = Some(meta.len());
                }
                node.updated_at = Self::updated_at_of(&meta);
                Ok(VdfsResponse::Stat(node))
            }
            VdfsRequest::Read => {
                let target = self.resolve(path)?;
                let resolved = self.guard_read(&target, path).await?;
                let meta = tokio::fs::metadata(&resolved)
                    .await
                    .map_err(|e| VdfsError::not_found(format!("无法读取元数据 `{path}`：{e}")))?;
                if meta.is_dir() {
                    return Err(VdfsError::Forbidden(format!(
                        "`{path}` 是目录，没有正文可读"
                    )));
                }
                if meta.len() > MAX_FILE_SIZE {
                    return Err(VdfsError::Forbidden(format!(
                        "文件过大：{} 字节（上限 {MAX_FILE_SIZE}）",
                        meta.len()
                    )));
                }
                let bytes = tokio::fs::read(&resolved)
                    .await
                    .map_err(|e| VdfsError::internal(format!("读取失败：{e}")))?;
                match String::from_utf8(bytes.clone()) {
                    Ok(text) => Ok(VdfsResponse::Read(VdfsContent::text(text))),
                    // 二进制文件（索引缓存、图片…）按 b64 走 VdfsContent::binary。
                    // 作用域存储若只能放文本，就挡住了 D-515 想覆盖的缓存类数据。
                    Err(_) => {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        Ok(VdfsResponse::Read(VdfsContent::binary(
                            b64,
                            bytes.len() as u64,
                        )))
                    }
                }
            }
            VdfsRequest::Write { content } => {
                let target = self.resolve(path)?;
                self.guard_write(&target).await?;
                let bytes: Vec<u8> = match (&content.text, &content.b64) {
                    (Some(t), _) => t.as_bytes().to_vec(),
                    (None, Some(b)) => base64::engine::general_purpose::STANDARD
                        .decode(b)
                        .map_err(|e| {
                            VdfsError::Invalid(
                                symbio_platform::kernel::vdfs::VdfsValidationError::new(format!(
                                    "b64 解码失败：{e}"
                                )),
                            )
                        })?,
                    (None, None) => {
                        return Err(VdfsError::invalid("写入需要 text 或 b64 之一"));
                    }
                };
                let created = !target.exists();
                if let Some(parent) = target.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|e| VdfsError::internal(format!("创建目录失败：{e}")))?;
                }
                tokio::fs::write(&target, &bytes)
                    .await
                    .map_err(|e| VdfsError::internal(format!("写入失败：{e}")))?;
                self.notify(path);
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: None,
                    created,
                }))
            }
            VdfsRequest::Delete { recursive } => {
                let target = self.resolve(path)?;
                let resolved = self.guard_read(&target, path).await?;
                let meta = tokio::fs::metadata(&resolved)
                    .await
                    .map_err(|e| VdfsError::not_found(format!("无法访问 `{path}`：{e}")))?;
                if meta.is_dir() {
                    if !recursive {
                        return Err(VdfsError::invalid(format!(
                            "目录需要 recursive=true：`{path}`"
                        )));
                    }
                    tokio::fs::remove_dir_all(&resolved)
                        .await
                        .map_err(|e| VdfsError::internal(format!("删除目录失败：{e}")))?;
                } else {
                    tokio::fs::remove_file(&resolved)
                        .await
                        .map_err(|e| VdfsError::internal(format!("删除文件失败：{e}")))?;
                }
                self.notify(path);
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Mkdir => {
                let target = self.resolve(path)?;
                self.guard_write(&target).await?;
                tokio::fs::create_dir_all(&target)
                    .await
                    .map_err(|e| VdfsError::internal(format!("创建目录失败：{e}")))?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

/// 把三个作用域 provider 挂进请求 ctx（D-515 的统一注入口）。
///
/// 传 `None` 表示该作用域不存在（没选工作区 / 无会话 / 无选中 agent）——
/// 插件读到 `None` 即降级，语义等同 `MemoryFile::absent`。
///
/// 所有注入点（宿主 drive / gather、session 开轮收集）都走这一个函数，
/// 避免各自拼 provider 形态而漂移。
pub fn attach_scope_providers(
    ctx: &dyn symbio_platform::sdk::plugin::PluginInvokeRequest,
    agent_dir: Option<&Path>,
    work_dir: Option<&Path>,
    session_dir: Option<&Path>,
) {
    use symbio_platform::kernel::plugin::PluginInvokeRequestExt;
    use symbio_platform::kernel::vdfs::DynVdfsProvider;
    use symbio_platform::sdk::keys::{AGENT_VDFS, SESSION_VDFS, WORKSPACE_VDFS};

    let mk = |dir: Option<&Path>| -> Option<DynVdfsProvider> {
        dir.map(|p| std::sync::Arc::new(ScopeDirVdfs::at(p.to_path_buf())) as DynVdfsProvider)
    };
    ctx.set(AGENT_VDFS, mk(agent_dir));
    ctx.set(WORKSPACE_VDFS, mk(work_dir));
    ctx.set(SESSION_VDFS, mk(session_dir));
}

/// 会话开轮时的作用域注入：**保留 `AGENT_VDFS` 的既有绑定**，只按本会话重挂
/// `WORKSPACE_VDFS` / `SESSION_VDFS`。
///
/// 为什么不能复用 [`attach_scope_providers`]：开轮 ctx 可能已经是**子树 ctx**——
/// 子会话的收件箱是经 `subagent/<id>/session/<sid>/inbox` 下钻写入的，owner 插件在
/// VDFS 下钻 / 子智能体转发时已按 `AgentScope.root` 把该子树的 `AGENT_VDFS` 重挂到
/// `{homedir}/subagent/<id>`（人格 / 记忆属于那个子智能体）。若再用 `agent_home_root()`
/// （`{homedir}`）覆盖，系统树人格就会顶掉子智能体人格：T15 现场是子空间开出的轮次
/// 注入了顶层 `AGENTS.md`，而不是「评审子智能体」的。
///
/// 顶层会话（ctx 由宿主注入，未下钻）的 `AGENT_VDFS` 本来就是 `agent_home_root()`，
/// 保留它同样正确。因此这里不碰 agent 作用域，把主动权留给上层插件。
pub fn attach_round_providers(
    ctx: &dyn symbio_platform::sdk::plugin::PluginInvokeRequest,
    work_dir: Option<&Path>,
    session_dir: Option<&Path>,
) {
    use symbio_platform::kernel::plugin::PluginInvokeRequestExt;
    use symbio_platform::kernel::vdfs::DynVdfsProvider;
    use symbio_platform::sdk::keys::{SESSION_VDFS, WORKSPACE_VDFS};

    let mk = |dir: Option<&Path>| -> Option<DynVdfsProvider> {
        dir.map(|p| std::sync::Arc::new(ScopeDirVdfs::at(p.to_path_buf())) as DynVdfsProvider)
    };
    ctx.set(WORKSPACE_VDFS, mk(work_dir));
    ctx.set(SESSION_VDFS, mk(session_dir));
}

/// 系统智能体根目录（`{homedir}`）——系统树 `AGENT_VDFS` 的绑定根。
///
/// 系统树的 `memory` 实例经 `AGENT_VDFS` 读写 `{homedir}/AGENTS.md`（系统智能体
/// 自身记忆）。**不是** `{homedir}/subagent/<id>`：那是「选中的子智能体」目录，
/// 由 owner 插件在转发时按 `AgentScope.root` 重挂到子树 ctx
/// （子树 ctx 的 `AGENT_VDFS` 才指向它）。宿主与会话开轮的注入点与 `agent_id`
/// 无关——把选中 id 的目录当根会让系统树去读子智能体的记忆（T39 回归）。
/// `homedir` 取 `SYMBIO_HOMEDIR`，缺省 `~/.symbio`。
pub fn agent_home_root() -> Option<PathBuf> {
    std::env::var("SYMBIO_HOMEDIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".symbio"))
        })
}

#[cfg(test)]
#[path = "scope.test.rs"]
mod tests;
