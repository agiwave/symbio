//! 整包（zip）导入 / 导出——目录型资源的「一个地址、一整个条目」通道
//!
//! **导出与导入是一对逆向的节点动作**（`action: "export"` / `action: "import"`，
//! 见 `symbio_core/vdfs` 的动作一节），两者都不新增操作，因此打包 /
//! 解包也不该是第二条协议，而是本层的一组工具 + 一对载荷形状
//! （出向 [`VdfsPack`] / 入向 [`VdfsUnpack`]）。
//!
//! 往返契约：[`zip_dir`] 以条目 id 作**唯一顶层目录**，[`extract_pack`] 端
//! [`strip_common_root`] 恰好剥掉这一层——导出的包能原样导回。

use crate::symbio_core::vdfs_has_parent_segment;
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};
use std::path::Path;

/// 整包载荷的**出向**形态（VDFS 动作 `export` 的 `data`）
///
/// 字段名与 [`VdfsContent::b64`](crate::symbio_core::VdfsContent) 同构——
/// 前端据此把它当**文件载荷**处理（有 `filename` + `b64` 就下载），
/// 不认识「导出」这个动作本身。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsPack {
    /// 被导出的条目 id
    pub id: String,
    /// 建议文件名（`<id>.zip`）
    pub filename: String,
    /// 打包字节（base64）
    pub b64: String,
}

impl VdfsPack {
    pub fn new(id: &str, bytes: &[u8]) -> Self {
        Self {
            id: id.to_string(),
            filename: format!("{id}.zip"),
            b64: encode_b64(bytes),
        }
    }
}

/// 整包载荷的**入向**形态（VDFS 动作 `import` 的 `payload`）
///
/// 与 [`VdfsPack`] **同一形状**：导出与导入是一对逆向动作，载荷也对称——
/// `filename` 让 provider 推导目标名、`b64` 是包字节。差别只有 `id`：
/// 导出时它是被导出的条目，导入时**由 provider 决定**（agent 取自包内
/// manifest、mcp / skill 取自 `filename`），使用方指定不了。
///
/// 使用方（前端）取不到用户刚选的本地文件字节，故这一步由它做：声明
/// `DetailAction::pack` 的动作，使用方先取文件、按本形状装好载荷再执行动作。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VdfsUnpack {
    /// 包的文件名（如 `demo.zip`）——provider 按它推导目标名
    pub filename: String,
    /// 包字节（base64）
    pub b64: String,
}

impl VdfsUnpack {
    /// 从动作载荷解出（形状不符时说清缺了什么，别让每个 provider 各写一遍）
    pub fn from_payload(payload: Option<&serde_json::Value>) -> Result<Self, PackError> {
        let p = payload.ok_or_else(|| PackError("导入需要整包载荷（filename + b64）".into()))?;
        Self::deserialize(p)
            .map_err(|e| PackError(format!("整包载荷不合法（需要 filename + b64）：{e}")))
    }

    /// 包字节（base64 → 字节）
    pub fn bytes(&self) -> Result<Vec<u8>, PackError> {
        decode_b64(&self.b64)
    }

    /// 由文件名推导的**目标条目名**（`demo.zip` → `demo`）
    pub fn name_of(&self, kind: &str) -> String {
        super::entry::pack_name_of(&self.filename, kind)
    }
}

/// 整包处理错误（zip 解码 / 解包 / 落盘）
#[derive(Debug, Clone)]
pub struct PackError(pub String);

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PackError {}

impl From<PackError> for crate::symbio_core::VdfsError {
    fn from(e: PackError) -> Self {
        Self::Internal(e.0)
    }
}

/// base64 解码（VDFS 二进制通道 `VdfsContent.b64` → 字节）
pub fn decode_b64(s: &str) -> Result<Vec<u8>, PackError> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD
        .decode(s.trim())
        .map_err(|e| PackError(format!("base64 解码失败: {e}")))
}

/// base64 编码（字节 → VDFS 二进制通道的载荷）
pub fn encode_b64(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD.encode(bytes)
}

/// 把目录递归打包成 zip 字节（包内顶层目录名 = `root`）
///
/// 与 [`extract_pack`] 的 `strip_common_root` 配对：导出的包可直接导回。
pub fn zip_dir(dir: &Path, root: &str) -> Result<Vec<u8>, PackError> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        add_dir_to_zip(&mut w, dir, root, opts)?;
        w.finish()
            .map_err(|e| PackError(format!("zip 生成失败: {e}")))?;
    }
    Ok(buf.into_inner())
}

/// 递归写目录（`arcname` 前缀形成单顶层目录布局）
fn add_dir_to_zip<W: Write + std::io::Seek>(
    w: &mut zip::ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    opts: zip::write::SimpleFileOptions,
) -> Result<(), PackError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| PackError(format!("读取目录失败: {e}")))?
        .flatten()
        .collect::<Vec<_>>();
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // 跳过隐藏文件（与导入端同一口径）
        if name.starts_with('.') {
            continue;
        }
        let arc = format!("{prefix}/{name}");
        if path.is_dir() {
            w.add_directory(&arc, opts)
                .map_err(|e| PackError(format!("zip 写目录失败: {e}")))?;
            add_dir_to_zip(w, &path, &arc, opts)?;
        } else {
            w.start_file(&arc, opts)
                .map_err(|e| PackError(format!("zip 写文件失败: {e}")))?;
            let bytes =
                std::fs::read(&path).map_err(|e| PackError(format!("读取文件失败: {e}")))?;
            w.write_all(&bytes)
                .map_err(|e| PackError(format!("zip 写内容失败: {e}")))?;
        }
    }
    Ok(())
}

/// 解析 zip 字节为 `(相对路径, 内容)` 列表。
///
/// - 跳过目录条目、`__MACOSX` 元数据、隐藏文件
/// - 强行去掉条目前导的 `./` / `/`
pub fn parse_pack(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, PackError> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| PackError(format!("非法 zip: {e}")))?;

    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| PackError(format!("读取 zip 条目失败: {e}")))?;

        let raw = file.name().replace('\\', "/");
        if file.is_dir() {
            continue;
        }
        // 先规范化再判隐藏：否则 `./a/b.txt` 的首段 `.` 会被当成隐藏文件整条丢弃
        let rel = normalize_pack_path(&raw);
        if rel.is_empty() {
            continue;
        }
        // 跳过 macOS 元数据 / 隐藏文件
        if rel.contains("__MACOSX")
            || rel
                .split('/')
                .any(|seg| seg.starts_with('.') && !seg.is_empty())
        {
            continue;
        }
        // 路径穿越防线：包内条目只允许落在目标目录**之内**（zip-slip）
        if vdfs_has_parent_segment(&rel) {
            return Err(PackError(format!("zip 条目越出目标目录：{rel}")));
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| PackError(format!("读取 zip 条目内容失败: {e}")))?;
        out.push((rel, buf));
    }
    Ok(out)
}

/// 若 zip 内所有条目共享一个顶层根目录（常见打包方式），剥离该层，
/// 使内容平铺到目标条目目录下。
pub fn strip_common_root(entries: &mut [(String, Vec<u8>)]) {
    if entries.is_empty() {
        return;
    }
    let prefix = entries
        .iter()
        .filter_map(|(p, _)| p.split('/').next())
        .filter(|seg| !seg.is_empty())
        .min()
        .map(|root| format!("{root}/"));
    // 仅当每个条目都以此根目录开头时才剥离
    if let Some(prefix) = prefix {
        if entries.iter().all(|(p, _)| p.starts_with(&prefix)) {
            for (p, _) in entries.iter_mut() {
                if let Some(rest) = p.strip_prefix(&prefix) {
                    *p = rest.to_string();
                }
            }
        }
    }
}

/// 把 zip 字节解到 `<entry_dir>/` 整目录（**导入即整目录覆盖**，无合并语义）
///
/// 返回写入的文件数量。
pub async fn extract_pack(entry_dir: &Path, bytes: &[u8]) -> Result<usize, PackError> {
    let mut entries = parse_pack(bytes)?;
    strip_common_root(&mut entries);
    if entries.is_empty() {
        return Err(PackError("zip 中没有任何可用的文件".to_string()));
    }

    if entry_dir.exists() {
        tokio::fs::remove_dir_all(entry_dir)
            .await
            .map_err(|e| PackError(format!("清理旧条目目录失败: {e}")))?;
    }

    for (rel, content) in &entries {
        let path = entry_dir.join(rel);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| PackError(format!("创建目录失败: {e}")))?;
        }
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| PackError(format!("写入文件失败: {e}")))?;
    }
    Ok(entries.len())
}

/// 规范化 zip 内部相对路径文本（去掉前导 `./` 与 `/`）
fn normalize_pack_path(p: &str) -> String {
    p.trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

// 解包侧的路径穿越防线复用协议层的 `vdfs_has_parent_segment`（按**段**判定，`/` 与
// `\` 都算）：zip 条目是**包内自带**的路径，不经过请求地址那条守卫，所以必须
// 自己判一次；但**规则本体只该有一份**，这里不再另写实现。

#[cfg(test)]
#[path = "pack.test.rs"]
mod tests;
