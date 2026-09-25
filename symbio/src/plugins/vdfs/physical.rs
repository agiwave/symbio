//! 物理文件层 —— VDFS 的「真实磁盘」那一半
//!
//! ## 为什么在 vdfs 插件里
//!
//! VDFS 整体负责**一棵地址空间**：`.vdfsv2` 打头的是虚拟目录（系统资源类别），
//! 其余地址一律是磁盘上的真实文件。既然两类地址由同一个门面分发，物理文件
//! 访问自然也属于本插件——它不是「挂载点」，没有名字、不参与 `.vdfsv2` 的目录合成。
//!
//! （此前它住在 `plugins/local`，以 `local` 挂载点身份注册，于是每次本地文件访问
//! 都要先被翻译成 `local/<相对路径>`、再由分发层拆回挂载名与相对路径。那层翻译
//! 除了让 `local` 挤进资源类别清单、继而催生 `nav_visible` 之外，不产生任何价值。）
//!
//! ## 地址规则
//!
//! 进入本层的地址已是「确认非虚拟」的那一半，坐标系是**会话工作目录**
//! （[`VDFS_PARAM_WORKDIR`]，由访问层透传；缺失即 `Internal`——那是接线错误）：
//!
//! - `""` / `"."` = 工作目录根；
//! - `README.md`、`src/main.rs` = 工作目录相对；
//! - `D:/tmp/a.txt` = 真正的绝对路径直用。
//!
//! ## 安全规则（自持，不经机制）
//!
//! 见 [`FsPolicy`]：路径黑名单 + `..` 穿越拒绝 + 符号链接复验，另有读大小 /
//! 列目录条数上限。这些是**物理层专有**的规则，虚拟层没有对应物。

use crate::symbio_core::{
    has_parent_segment, path_within, VdfsAccess, VdfsContent, VdfsContext, VdfsError, VdfsNode,
    VdfsProvider, VdfsResult, VdfsValidationError, VdfsWriteResponse, VDFS_PARAM_WORKDIR,
};
use crate::symbio_core::{VdfsRequest, VdfsResponse};
use async_trait::async_trait;
use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 单文件读取上限（10MB）
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// 单次列目录上限（防止超大目录撑爆上下文）
const MAX_LIST_ENTRIES: usize = 2000;

// ==================== 路径守卫 ====================

/// 规范化路径用于比较（剥掉 Windows 的 `\\?\` 前缀，使前缀比较可靠）
fn normalize_for_comparison(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(stripped) => PathBuf::from(stripped),
        None => path.to_path_buf(),
    }
}

fn starts_with_normalized(base: &Path, prefix: &Path) -> bool {
    normalize_for_comparison(base).starts_with(normalize_for_comparison(prefix))
}

/// 物理文件层的访问策略。
///
/// 只有两类规则：`forbidden_paths` 黑名单，以及可选的「限定在工作区 /
/// 白名单根之内」（[`FsPolicy::workspace_only`]，缺省关闭 = 全盘可读，
/// 仅黑名单兜底）。
#[derive(Debug, Clone)]
pub struct FsPolicy {
    /// 是否只允许工作目录与 [`FsPolicy::allowed_roots`] 之内的绝对路径
    pub workspace_only: bool,
    /// 额外的允许根（`workspace_only` 为真时参与判定）
    pub allowed_roots: Vec<PathBuf>,
    /// 黑名单前缀（支持 `~`，命中即拒绝读与写）
    pub forbidden_paths: Vec<String>,
}

impl Default for FsPolicy {
    fn default() -> Self {
        Self {
            workspace_only: false,
            allowed_roots: Vec::new(),
            forbidden_paths: vec![
                "/etc".into(),
                "/root".into(),
                "/usr".into(),
                "~/.ssh".into(),
                "~/.gnupg".into(),
                "~/.aws".into(),
            ],
        }
    }
}

impl FsPolicy {
    /// 两道守卫都走机制层的地址规则（[`has_parent_segment`] / [`path_within`]），
    /// 不在此另写一份——访问层与物理层必须同一条规则，否则修了一处漏另一处。
    fn path_allowed(&self, path: &Path, workspace_dir: &Path) -> bool {
        let s = path.to_string_lossy();
        if has_parent_segment(&s) {
            return false;
        }
        for forbidden in &self.forbidden_paths {
            if path_within(&s, shellexpand::tilde(forbidden).as_ref()) {
                return false;
            }
        }
        if !self.workspace_only || !path.is_absolute() {
            return true;
        }
        starts_with_normalized(path, workspace_dir)
            || self
                .allowed_roots
                .iter()
                .any(|r| starts_with_normalized(path, r))
    }

    pub fn readable(&self, path: &Path, workspace_dir: &Path) -> bool {
        self.path_allowed(path, workspace_dir)
    }

    pub fn writable(&self, path: &Path, workspace_dir: &Path) -> bool {
        self.path_allowed(path, workspace_dir)
    }
}

// ==================== 地址解析 ====================

/// 工作目录基准
fn workdir(ctx: &VdfsContext) -> VdfsResult<PathBuf> {
    let raw = ctx.param_str(VDFS_PARAM_WORKDIR).ok_or_else(|| {
        VdfsError::internal(format!(
            "缺少 {VDFS_PARAM_WORKDIR} 参数（访问层未透传工作目录）"
        ))
    })?;
    if raw.is_empty() {
        return Err(VdfsError::internal(format!(
            "{VDFS_PARAM_WORKDIR} 参数为空"
        )));
    }
    Ok(PathBuf::from(shellexpand::tilde(raw).into_owned()))
}

/// 地址 → 真实路径。
///
/// 首部的 `/` 只是地址分隔符（虚拟根残留、或调用方习惯性带上），**不是**文件系统
/// 根——统一剥掉，使 `README.md` 与 `/README.md` 落到同一个位置。仅真正的绝对路径
/// （如 Windows 盘符 `D:/...`）直用。
fn join_target(base: &Path, rel: &str) -> PathBuf {
    let rel = rel.trim_start_matches('/');
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(rel)
    }
}

fn to_unix(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// 末段名称
fn basename(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

/// 常见图片扩展名 → MIME（多模态读取）
fn image_mime(name: &str) -> Option<&'static str> {
    match Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

/// 目录直接子项计数（`stat` 的 `children` 提示；上限与 [`MAX_LIST_ENTRIES`] 一致）。
///
/// 读不动就返回 `None`（未知）——提示信息不值得让 `stat` 失败。
async fn count_children(dir: &Path) -> Option<u32> {
    let mut rd = tokio::fs::read_dir(dir).await.ok()?;
    let mut n: u32 = 0;
    while let Ok(Some(_)) = rd.next_entry().await {
        n += 1;
        if n as usize >= MAX_LIST_ENTRIES {
            break;
        }
    }
    Some(n)
}

// ==================== 物理文件层 ====================

/// 磁盘上的真实文件树（工作目录为根）。
///
/// 实现 [`VdfsProvider`] 只为复用同一套分发与组合逻辑（`edit` / `search` 由
/// 访问层组合而成，不区分虚拟与物理）；它**不注册为挂载点**，因此
/// `label` / `order` / 根节点的 `new_type` 这些「被合成进 `.vdfsv2` 时才有人读」的
/// 声明在这里没有意义，一律不覆盖。
pub struct PhysicalFs {
    policy: FsPolicy,
}

impl Default for PhysicalFs {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicalFs {
    pub fn new() -> Self {
        Self {
            policy: FsPolicy::default(),
        }
    }

    /// 读路径守卫：黑名单 + 解析符号链接后复验（防链接逃逸）
    async fn guard_read(&self, base: &Path, target: &Path) -> VdfsResult<PathBuf> {
        if !self.policy.readable(target, base) {
            return Err(VdfsError::Forbidden(format!(
                "路径不允许访问：{}",
                target.display()
            )));
        }
        let resolved = tokio::fs::canonicalize(target)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法解析路径 {}：{e}", target.display())))?;
        if !self.policy.readable(&resolved, base) {
            return Err(VdfsError::Forbidden(format!(
                "路径解析后超出工作区范围：{}",
                resolved.display()
            )));
        }
        Ok(resolved)
    }

    /// 写路径守卫：黑名单 + 拒绝符号链接 + 已存在时复验解析结果
    async fn guard_write(&self, base: &Path, target: &Path) -> VdfsResult<()> {
        if !self.policy.writable(target, base) {
            return Err(VdfsError::Forbidden(format!(
                "路径不允许写入：{}",
                target.display()
            )));
        }
        if let Ok(meta) = tokio::fs::symlink_metadata(target).await {
            if meta.file_type().is_symlink() {
                return Err(VdfsError::Invalid(VdfsValidationError::new(format!(
                    "拒绝写入符号链接：{}",
                    target.display()
                ))));
            }
            let resolved = tokio::fs::canonicalize(target)
                .await
                .map_err(|e| VdfsError::internal(format!("解析路径失败：{e}")))?;
            if !self.policy.writable(&resolved, base) {
                return Err(VdfsError::Forbidden(format!(
                    "路径解析后超出工作区范围：{}",
                    resolved.display()
                )));
            }
        }
        Ok(())
    }

    /// 目录 / 文件节点形状（虚拟层与物理层共用同一套 `VdfsNode` 构造）
    fn node_from(name: String, is_dir: bool) -> VdfsNode {
        if is_dir {
            VdfsNode::dir(name.clone(), name, VdfsAccess::dir(true, true))
        } else {
            VdfsNode::file(name.clone(), name, VdfsAccess::file(true))
        }
    }
}

/// 唯一入口 `dispatch`：按请求变体落到各私有操作上；地址（`path`）是独立参数，
/// 分发先按 path 定位、再按操作落地。
#[async_trait]
impl VdfsProvider for PhysicalFs {
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => Ok(VdfsResponse::list(self.do_list(ctx, path).await?)),
            VdfsRequest::Stat => Ok(VdfsResponse::Stat(self.do_stat(ctx, path).await?)),
            VdfsRequest::Read => Ok(VdfsResponse::Read(self.do_read(ctx, path).await?)),
            VdfsRequest::Write { content } => Ok(VdfsResponse::Write(
                self.do_write(ctx, path, &content).await?,
            )),
            VdfsRequest::Delete { recursive } => {
                self.do_delete(ctx, path, recursive).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Mkdir => {
                self.do_mkdir(ctx, path).await?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

impl PhysicalFs {
    async fn do_list(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        let resolved = self.guard_read(&base, &target).await?;

        let mut rd = tokio::fs::read_dir(&resolved)
            .await
            .map_err(|e| VdfsError::invalid(format!("无法读取目录 {}：{e}", target.display())))?;

        let mut nodes: Vec<VdfsNode> = Vec::new();
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| VdfsError::internal(format!("遍历目录失败：{e}")))?
        {
            let Ok(name) = entry.file_name().into_string() else {
                continue; // 非 UTF-8 名称跳过（无法在地址空间里表达）
            };
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            let is_dir = meta.is_dir();
            let mut node = Self::node_from(name.clone(), is_dir);
            node.updated_at = meta.modified().ok().and_then(to_unix);
            if !is_dir {
                node.size = Some(meta.len());
                node.binary = image_mime(&name).is_some();
            }
            nodes.push(node);
            if nodes.len() >= MAX_LIST_ENTRIES {
                break;
            }
        }

        // 目录在前，各自按名称升序
        nodes.sort_by(|a, b| {
            b.is_dir()
                .cmp(&a.is_dir())
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(nodes)
    }

    async fn do_stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        let resolved = self.guard_read(&base, &target).await?;
        let meta = tokio::fs::metadata(&resolved).await.map_err(|e| {
            VdfsError::not_found(format!("无法读取元数据 {}：{e}", target.display()))
        })?;

        let name = if path.is_empty() {
            resolved
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("工作目录")
                .to_string()
        } else {
            basename(path)
        };
        let is_dir = meta.is_dir();
        let mut node = Self::node_from(name.clone(), is_dir);
        node.updated_at = meta.modified().ok().and_then(to_unix);
        if !is_dir {
            node.size = Some(meta.len());
            node.binary = image_mime(&name).is_some();
        } else {
            node.children = count_children(&resolved).await;
        }
        Ok(node)
    }

    async fn do_read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        let resolved = self.guard_read(&base, &target).await?;

        let meta = tokio::fs::metadata(&resolved)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法读取文件元数据：{e}")))?;
        if meta.is_dir() {
            return Err(VdfsError::Forbidden(format!(
                "目录不可读：{}（请用 vdfs_list 列举）",
                target.display()
            )));
        }
        if meta.len() > MAX_FILE_SIZE {
            return Err(VdfsError::Forbidden(format!(
                "文件过大：{} 字节（上限 {} 字节）",
                meta.len(),
                MAX_FILE_SIZE
            )));
        }

        // 图片：base64 载荷（多模态）
        if let Some(mime) = image_mime(path) {
            let bytes = tokio::fs::read(&resolved)
                .await
                .map_err(|e| VdfsError::internal(format!("读取文件失败：{e}")))?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Ok(VdfsContent::binary(b64, bytes.len() as u64).with_mime(mime));
        }

        let text = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|e| VdfsError::internal(format!("读取文件失败（非文本？）：{e}")))?;
        Ok(VdfsContent::text(text))
    }

    async fn do_write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        self.guard_write(&base, &target).await?;

        let bytes: Vec<u8> = match (&content.text, &content.b64) {
            (Some(t), _) => t.as_bytes().to_vec(),
            (None, Some(b)) => base64::engine::general_purpose::STANDARD
                .decode(b)
                .map_err(|e| {
                    VdfsError::Invalid(VdfsValidationError::new(format!("b64 解码失败：{e}")))
                })?,
            (None, None) => {
                return Err(VdfsError::invalid("写入需要 text 或 b64 之一作为内容"));
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
            .map_err(|e| VdfsError::internal(format!("写入文件失败：{e}")))?;

        Ok(VdfsWriteResponse {
            // 具名写：名字是调用方给的（磁盘层没有「匿名新建」这一形态）
            name: None,
            created,
            etag: Some(bytes.len().to_string()),
        })
    }

    async fn do_delete(&self, ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        let resolved = self.guard_read(&base, &target).await?;

        if tokio::fs::symlink_metadata(&target)
            .await
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(VdfsError::Forbidden(format!(
                "拒绝删除符号链接：{}",
                target.display()
            )));
        }

        let meta = tokio::fs::metadata(&resolved)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法访问 {}：{e}", target.display())))?;
        if meta.is_dir() {
            if !recursive {
                return Err(VdfsError::invalid(format!(
                    "目录需要 recursive=true 才能删除：{}",
                    target.display()
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
        Ok(())
    }

    async fn do_mkdir(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        self.guard_write(&base, &target).await?;
        tokio::fs::create_dir_all(&target)
            .await
            .map_err(|e| VdfsError::internal(format!("创建目录失败：{e}")))?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "physical.test.rs"]
mod tests;
