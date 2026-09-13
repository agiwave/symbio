//! 本地文件系统的 VDFS provider —— `local` 挂载点的唯一实现
//!
//! ## 职责
//!
//! 把「工作目录（workdir）下的一棵真实文件树」暴露为一棵 VDFS 子树：
//! 相对路径从 workdir 开始，绝对路径直用——与既有本地文件工具**同一套地址规则**，
//! 因此大模型的既有用法在虚拟地址空间下保持不变。
//!
//! 访问层（vdfs 插件）在调用前把 `/local/<rel>` 拆成挂载名 + 相对路径，本 provider
//! 只见到 `rel`（`""` = workdir 根），**不知道**自己被挂在哪里。
//!
//! ## workdir 从哪来
//!
//! provider 不持有会话状态，workdir 由访问层经 [`VFDS_PARAM_WORKDIR`] 透传
//! （见 [`VdfsContext::param`]）。缺失即 `Internal`——那是接线错误，不是用户错误。
//!
//! ## 安全规则（自持，不经机制）
//!
//! 与既有本地工具一致，全部落在本 provider 内：
//! 路径白名单（[`SecurityPolicy::is_path_allowed_for_read`] /
//! [`SecurityPolicy::is_path_allowed_for_write`]）、速率限制、读大小上限、
//! 拒绝符号链接写入 / 删除。

use super::policy::SecurityPolicy;
use crate::symbio_core::vdfs_provider::*;
use async_trait::async_trait;
use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// 单文件读取上限（10MB），与既有 `read_file` 一致
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// 单次列目录上限（防止超大目录撑爆上下文）
const MAX_LIST_ENTRIES: usize = 2000;

/// 工作目录基准
fn workdir(ctx: &VdfsContext) -> VdfsResult<PathBuf> {
    let raw = ctx.param_str(VFDS_PARAM_WORKDIR).ok_or_else(|| {
        VdfsError::internal(format!(
            "缺少 {VFDS_PARAM_WORKDIR} 参数（访问层未透传工作目录）"
        ))
    })?;
    if raw.is_empty() {
        return Err(VdfsError::internal(format!(
            "{VFDS_PARAM_WORKDIR} 参数为空"
        )));
    }
    Ok(PathBuf::from(shellexpand::tilde(raw).into_owned()))
}

/// 相对路径 → 真实路径（本地挂载固定以 workdir 为基准，故地址一律 workdir 相对）
///
/// 进入本 provider 的路径已是「挂载名被容器剥掉后的相对路径」；其首部的 `/`
/// 只是虚拟根分隔符（如前端经 `/local/README.md` 寻址、容器剥掉 `local` 后仍
/// 残留一个 `/`），并非文件系统根——统一剥掉，保证 LLM 给的 `README.md` 与
/// 前端给的 `/local/README.md` 在 local 收到的是**同一**地址。仅真正的绝对路径
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

/// 末段名称（`read_file`/`dir_list` 展示用）
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

/// 本地文件子树 provider
#[derive(Clone)]
pub struct LocalVdfs {
    security: Arc<SecurityPolicy>,
}

impl LocalVdfs {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    /// 读路径守卫：白名单 + 解析符号链接后复验（防链接逃逸）
    async fn guard_read(&self, base: &Path, target: &Path) -> VdfsResult<PathBuf> {
        if !self.security.is_path_allowed_for_read(target, base).await {
            return Err(VdfsError::Forbidden(format!(
                "路径不允许访问：{}",
                target.display()
            )));
        }
        let resolved = tokio::fs::canonicalize(target)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法解析路径 {}：{e}", target.display())))?;
        if !self
            .security
            .is_path_allowed_for_read(&resolved, base)
            .await
        {
            return Err(VdfsError::Forbidden(format!(
                "路径解析后超出工作区范围：{}",
                resolved.display()
            )));
        }
        Ok(resolved)
    }

    /// 写路径守卫：白名单 + 拒绝符号链接 + 已存在时复验解析结果
    async fn guard_write(&self, base: &Path, target: &Path) -> VdfsResult<()> {
        if !self.security.is_path_allowed_for_write(target, base).await {
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
            if !self
                .security
                .is_path_allowed_for_write(&resolved, base)
                .await
            {
                return Err(VdfsError::Forbidden(format!(
                    "路径解析后超出工作区范围：{}",
                    resolved.display()
                )));
            }
        }
        Ok(())
    }

    fn rate_guard(&self) -> VdfsResult<()> {
        if self.security.is_rate_limited() {
            return Err(VdfsError::internal("速率限制：操作过于频繁"));
        }
        Ok(())
    }
}

#[async_trait]
impl VdfsProvider for LocalVdfs {
    fn label(&self) -> Option<&str> {
        Some("本地文件")
    }

    fn description(&self) -> Option<&str> {
        Some("工作目录下的本地文件系统；相对路径从工作目录开始")
    }

    fn order(&self) -> i32 {
        10
    }

    fn root_access(&self) -> VdfsAccess {
        // 可列、可遍历、可在其下创建 —— 根目录本身不是文件（无 r）
        VdfsAccess::dir(true, true)
    }

    /// 本地文件树**不是资源类别**：它是 VDFS 挂载点（可寻址、可读写、LLM 可用），
    /// 但不与 session / model / agent / skill / mcp / setting 并列占用左栏导航位
    /// （其入口在工作目录等处，而非资源导航）。
    fn nav_visible(&self) -> bool {
        false
    }

    async fn list(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
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
                continue; // 非 UTF-8 名称跳过（无法在虚拟地址空间里表达）
            };
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            let is_dir = meta.is_dir();
            let mut node = if is_dir {
                VdfsNode::dir(name.clone(), name.clone(), VdfsAccess::dir(true, true))
            } else {
                VdfsNode::file(name.clone(), name.clone(), VdfsAccess::file(true))
            };
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

        // 目录在前，各自按名称升序（与既有 dir_list 顺序一致）
        nodes.sort_by(|a, b| {
            b.is_dir()
                .cmp(&a.is_dir())
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(nodes)
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
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
                .unwrap_or(VFDS_ROOT)
                .to_string()
        } else {
            basename(path)
        };
        let mut node = if meta.is_dir() {
            VdfsNode::dir(name.clone(), name.clone(), VdfsAccess::dir(true, true))
        } else {
            VdfsNode::file(name.clone(), name.clone(), VdfsAccess::file(true))
        };
        node.updated_at = meta.modified().ok().and_then(to_unix);
        if !meta.is_dir() {
            node.size = Some(meta.len());
            node.binary = image_mime(&name).is_some();
        } else {
            node.children = count_children(&resolved).await;
        }
        Ok(node)
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        let resolved = self.guard_read(&base, &target).await?;

        let meta = tokio::fs::metadata(&resolved)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法读取文件元数据：{e}")))?;
        if meta.is_dir() {
            return Err(VdfsError::Forbidden(format!(
                "目录不可读：{}（请用 vdfs/list 列举）",
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
        self.rate_guard()?;
        self.security.record_action();

        // 图片：base64 载荷（多模态）
        if let Some(mime) = image_mime(path) {
            let bytes = tokio::fs::read(&resolved)
                .await
                .map_err(|e| VdfsError::internal(format!("读取文件失败：{e}")))?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Ok(VdfsContent::binary("", b64, bytes.len() as u64).with_mime(mime));
        }

        let text = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|e| VdfsError::internal(format!("读取文件失败（非文本？）：{e}")))?;
        Ok(VdfsContent::text("", text))
    }

    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        self.guard_write(&base, &target).await?;
        self.rate_guard()?;

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
        self.security.record_action();
        tokio::fs::write(&target, &bytes)
            .await
            .map_err(|e| VdfsError::internal(format!("写入文件失败：{e}")))?;

        Ok(VdfsWriteResponse {
            path: String::new(),
            created,
            etag: Some(bytes.len().to_string()),
        })
    }

    async fn delete(&self, ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
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
        self.security.record_action();
        Ok(())
    }

    async fn mkdir(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        self.guard_write(&base, &target).await?;
        tokio::fs::create_dir_all(&target)
            .await
            .map_err(|e| VdfsError::internal(format!("创建目录失败：{e}")))?;
        self.security.record_action();
        Ok(())
    }

    async fn move_item(&self, ctx: &VdfsContext, from: &str, to: &str) -> VdfsResult<()> {
        let base = workdir(ctx)?;
        let src = join_target(&base, from);
        let dst = join_target(&base, to);
        // 源需可读（存在 + 在范围内），目标需可写
        self.guard_read(&base, &src).await?;
        self.guard_write(&base, &dst).await?;
        if let Some(parent) = dst.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| VdfsError::internal(format!("创建目录失败：{e}")))?;
        }
        tokio::fs::rename(&src, &dst)
            .await
            .map_err(|e| VdfsError::internal(format!("移动失败：{e}")))?;
        self.security.record_action();
        Ok(())
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
