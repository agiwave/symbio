//! 统一资源门面（协议 re-export + 通用工具）
//!
//! - 协议定义见 [`schemas::resources`]（共享给各插件 route 复用）
//! - 本模块提供 zip 上传的通用解压 / 实体目录写盘工具，让 mcp / skill / agent
//!   三类插件共享同一套"zip → `~/.symbio/plugins/<category>/<id>/`"机制，避免重复实现。

pub use crate::symbio_core::schemas::resources::*;

use crate::symbio_core::providers::EntityStore;
use base64::Engine;
use std::io::{Cursor, Read};

/// 统一资源操作错误（转为 PluginError::Other 抛出）
#[derive(Debug, thiserror::Error)]
#[error("resource error: {0}")]
pub struct ResourceError(pub String);

/// base64 解码 zip（上传 payload 携带 `zip_b64`）
pub fn decode_zip_b64(s: &str) -> Result<Vec<u8>, ResourceError> {
    use base64::engine::general_purpose::STANDARD;
    STANDARD
        .decode(s)
        .map_err(|e| ResourceError(format!("zip base64 解码失败: {e}")))
}

/// 解析 zip 字节为 `(相对路径, 内容)` 列表。
///
/// - 跳过目录条目、`__MACOSX` 元数据、隐藏文件
/// - 强行去掉条目前导的 `./` / `/`
pub fn parse_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ResourceError> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| ResourceError(format!("非法 zip: {e}")))?;

    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| ResourceError(format!("读取 zip 条目失败: {e}")))?;

        let raw = file.name().replace('\\', "/");
        if file.is_dir() {
            continue;
        }
        // 跳过 macOS 元数据 / 隐藏文件
        if raw.contains("__MACOSX")
            || raw.split('/').any(|seg| seg.starts_with('.') && !seg.is_empty())
        {
            continue;
        }
        let rel = normalize_zip_path(&raw);
        if rel.is_empty() {
            continue;
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| ResourceError(format!("读取 zip 条目内容失败: {e}")))?;
        out.push((rel, buf));
    }
    Ok(out)
}

/// 若 zip 内所有条目共享一个顶层根目录（常见打包方式），剥离该层，
/// 使内容平铺到目标资源目录下。
pub fn strip_common_root(entries: &mut [(String, Vec<u8>)]) {
    if entries.is_empty() {
        return;
    }
    let root_candidates: Option<String> = entries
        .iter()
        .filter_map(|(p, _)| p.split('/').next())
        .filter(|seg| !seg.is_empty())
        .min()
        .map(|s| s.to_string());
    // 仅当每个条目都以此根目录开头时才剥离
    if let Some(root) = root_candidates.as_ref() {
        let prefix = root.to_string() + "/";
        if entries.iter().all(|(p, _)| p.starts_with(&prefix)) {
            for (p, _) in entries.iter_mut() {
                if let Some(rest) = p.strip_prefix(&prefix) {
                    *p = rest.to_string();
                }
            }
        }
    }
}

/// 把已解析的 zip 内容解压写入 `EntityStore` 的 `<category>/<id>/` 目录。
///
/// - 若目录已存在则整体删除重建（上传即覆盖整包）
/// - 返回写入的文件数量
pub async fn extract_zip_to_entity(
    es: &dyn EntityStore,
    category: &str,
    id: &str,
    bytes: &[u8],
) -> Result<usize, ResourceError> {
    let mut entries = parse_zip(bytes)?;
    strip_common_root(&mut entries);
    if entries.is_empty() {
        return Err(ResourceError("zip 中没有任何可用的资源文件".to_string()));
    }

    let dir = es.entity_dir(category, id);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir)
            .await
            .map_err(|e| ResourceError(format!("清理旧资源目录失败: {e}")))?;
    }

    for (rel, content) in &entries {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ResourceError(format!("创建目录失败: {e}")))?;
        }
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ResourceError(format!("写入资源文件失败: {e}")))?;
    }
    Ok(entries.len())
}

/// 规范化 zip 内部相对路径文本（去掉前导 `./` 与 `/`）
fn normalize_zip_path(p: &str) -> String {
    p.trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

/// 自定义资源错误转 PluginError
impl From<ResourceError> for crate::symbio_core::PluginError {
    fn from(e: ResourceError) -> Self {
        crate::symbio_core::PluginError::InternalError(e.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个内存 zip（按给定顺序写入条目）
    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::{Cursor, Write};
        use zip::write::SimpleFileOptions;
        let mut buf = Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        for (name, data) in entries {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn parse_zip_filters_meta_and_hidden() {
        let bytes = make_zip(&[
            ("__MACOSX/._x", b"meta"),
            (".hidden", b"y"),
            ("real.txt", b"hi"),
            ("dir/z.txt", b"z"),
        ]);
        let entries = parse_zip(&bytes).unwrap();
        let names: Vec<_> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["real.txt", "dir/z.txt"]);
        // 内容完整保留
        assert_eq!(entries[0].1, b"hi");
        assert_eq!(entries[1].1, b"z");
    }

    #[test]
    fn strip_common_root_peels_single_root() {
        let zip = make_zip(&[("skill/README.md", b"a"), ("skill/SKILL.md", b"b")]);
        let mut entries = parse_zip(&zip).unwrap();
        strip_common_root(&mut entries);
        let names: Vec<_> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["README.md", "SKILL.md"]);
    }

    #[test]
    fn strip_common_root_keeps_mixed_paths() {
        // 根目录不一致时不应剥离
        let zip = make_zip(&[("a.txt", b"a"), ("b/x.txt", b"b")]);
        let mut entries = parse_zip(&zip).unwrap();
        strip_common_root(&mut entries);
        let names: Vec<_> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "b/x.txt"]);
    }

    #[test]
    fn zip_b64_round_trip() {
        let raw = b"zip-bytes";
        let b64 = use_base64(raw);
        let back = decode_zip_b64(&b64).unwrap();
        assert_eq!(back, raw);
    }

    fn use_base64(input: &[u8]) -> String {
        use base64::engine::general_purpose::STANDARD;
        STANDARD.encode(input)
    }
}