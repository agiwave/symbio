//! 整包（zip）导入 / 导出——目录型资源的「一个地址、一整个条目」通道
//!
//! 规范 §3.3：**二进制写入 = 整包导入**，**导出是导入的逆动作**（节点动作
//! `action: "export"`）。两者都不新增操作，因此打包 / 解包也不该是第二条协议，
//! 而是本层的一组工具 + 一个载荷形状 [`VdfsPack`]。
//!
//! 往返契约：[`zip_dir`] 以条目 id 作**唯一顶层目录**，[`extract_pack`] 端
//! [`strip_common_root`] 恰好剥掉这一层——导出的包能原样导回。

use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};
use std::path::Path;

/// 整包载荷（VDFS 动作 `export` 的 `data`）
///
/// 字段名与 [`VdfsContent::b64`](crate::symbio_core::vdfs_provider::VdfsContent) 同构——
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

/// 整包处理错误（zip 解码 / 解包 / 落盘）
#[derive(Debug, Clone)]
pub struct PackError(pub String);

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PackError {}

impl From<PackError> for crate::symbio_core::vdfs_provider::VdfsError {
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
        if has_parent_segment(&rel) {
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

/// 路径中是否含 `..` 段——两种分隔符都算，按**段**判定
///
/// 协议层的 [`has_parent_segment`](crate::symbio_core::vdfs_provider::has_parent_segment)
/// 管的是**请求地址**；zip 条目是**包内自带**的路径，不经过那条守卫，
/// 因此解包侧必须自己判一次。
fn has_parent_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|seg| seg == "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造内存 zip（按给定顺序写入条目；`None` 内容 = 只建目录条目）
    fn make_zip(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            for (name, content) in entries {
                match content {
                    Some(bytes) => {
                        w.start_file(*name, opts).unwrap();
                        w.write_all(bytes).unwrap();
                    }
                    None => {
                        w.add_directory(*name, opts).unwrap();
                    }
                }
            }
            w.finish().unwrap();
        }
        buf
    }

    /// 解包：跳过目录 / 隐藏文件 / `__MACOSX`，并规范化前导 `./` 与 `/`
    #[test]
    fn parse_pack_skips_dirs_and_metadata() {
        let bytes = make_zip(&[
            ("SKILL.md", Some(b"# demo")),
            ("scripts/run.sh", Some(b"echo hi")),
            ("scripts/", None),
            ("__MACOSX/._SKILL.md", Some(b"junk")),
            (".hidden", Some(b"junk")),
            ("./nested/ok.txt", Some(b"ok")),
        ]);
        let out = parse_pack(&bytes).unwrap();
        let names: Vec<&str> = out.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            names,
            vec!["SKILL.md", "scripts/run.sh", "nested/ok.txt"],
            "目录条目与元数据不落盘：`{names:?}`"
        );
        assert_eq!(out[0].1, b"# demo".to_vec());
    }

    /// zip-slip：包内条目**一律不得**写到目标目录之外
    ///
    /// 两道防线任一生效即可（隐藏段过滤会先丢弃 `..` 段，显式守卫是第二道）；
    /// 这里断言的是**可观察性质**而不是走哪条分支——分支属于实现细节。
    #[test]
    fn parse_pack_never_yields_an_escaping_entry() {
        for hostile in [
            "evil/../../escape.txt",
            "../../escape.txt",
            "a/../../../escape.txt",
        ] {
            let bytes = make_zip(&[(hostile, Some(b"boom"))]);
            let out = parse_pack(&bytes).unwrap_or_default();
            assert!(
                out.iter()
                    .all(|(p, _)| !p.split(['/', '\\']).any(|s| s == "..")),
                "条目 `{hostile}` 逃出了目标目录：{out:?}"
            );
        }
    }

    /// 落盘侧的同一性质：解包后目标目录之外不得出现任何文件
    #[tokio::test]
    async fn extract_pack_confines_writes_to_the_entry_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let entry_dir = tmp.path().join("demo");
        let bytes = make_zip(&[
            ("SKILL.md", Some(b"# ok")),
            ("evil/../../escape.txt", Some(b"boom")),
            ("../../escape2.txt", Some(b"boom")),
        ]);
        extract_pack(&entry_dir, &bytes).await.unwrap();
        assert!(entry_dir.join("SKILL.md").exists());
        assert!(
            !tmp.path().join("escape.txt").exists() && !tmp.path().join("escape2.txt").exists(),
            "包内 `..` 条目不得写到条目目录之外"
        );
    }

    /// 单顶层目录的打包习惯：剥离该层，内容平铺到条目目录
    #[test]
    fn strip_common_root_flattens_single_root() {
        let mut entries: Vec<(String, Vec<u8>)> = vec![
            ("pkg/SKILL.md".into(), b"a".to_vec()),
            ("pkg/scripts/s.sh".into(), b"b".to_vec()),
        ];
        strip_common_root(&mut entries);
        let names: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, vec!["SKILL.md", "scripts/s.sh"]);

        // 不共享顶层目录 ⇒ 原样保留（避免误伤多根包）
        let mut mixed: Vec<(String, Vec<u8>)> =
            vec![("a/x.md".into(), Vec::new()), ("b/y.md".into(), Vec::new())];
        strip_common_root(&mut mixed);
        let names: Vec<&str> = mixed.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, vec!["a/x.md", "b/y.md"]);
    }

    /// 非法输入：非 base64 / 非 zip，都转为可读错误（不 panic）
    #[test]
    fn pack_errors_are_reported() {
        assert!(decode_b64("!!!not-base64!!!").is_err());
        assert!(parse_pack(b"not a zip at all").is_err());
        let empty = make_zip(&[("only-dir/", None)]);
        assert!(parse_pack(&empty).unwrap().is_empty());
    }

    /// 打包：单顶层目录 = 条目 id，跳过隐藏文件——与导入端恰好配对
    #[test]
    fn zip_dir_roundtrips_with_extract() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("SKILL.md"), b"# demo").unwrap();
        std::fs::write(dir.join("scripts").join("run.sh"), b"echo hi").unwrap();
        std::fs::write(dir.join(".DS_Store"), b"junk").unwrap();

        let bytes = zip_dir(&dir, "demo").unwrap();
        let mut entries = parse_pack(&bytes).unwrap();
        strip_common_root(&mut entries);
        let mut names: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec!["SKILL.md", "scripts/run.sh"],
            "导出 → 导入可原样往返：`{names:?}`"
        );
    }

    /// 解包即整目录覆盖：旧文件不得残留在新包里
    #[tokio::test]
    async fn extract_pack_replaces_whole_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        std::fs::create_dir_all(dir.join("stale")).unwrap();
        std::fs::write(dir.join("stale/old.txt"), b"old").unwrap();

        let bytes = make_zip(&[("SKILL.md", Some(b"# new"))]);
        let n = extract_pack(&dir, &bytes).await.unwrap();
        assert_eq!(n, 1);
        assert!(dir.join("SKILL.md").exists());
        assert!(
            !dir.join("stale/old.txt").exists(),
            "导入即替换：旧目录里未被包覆盖的文件必须消失"
        );
    }

    /// 载荷形状：`filename` + `b64` 就是前端认得的全部（它不认识「导出」）
    #[test]
    fn pack_payload_is_a_file_shape() {
        let p = VdfsPack::new("demo", b"hi");
        assert_eq!(p.filename, "demo.zip");
        assert_eq!(decode_b64(&p.b64).unwrap(), b"hi".to_vec());
    }
}
