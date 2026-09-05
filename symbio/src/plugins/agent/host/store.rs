//! Bundle Store —— OAB bundle 实例的磁盘存储与打包分发。
//!
//! ## 目录布局
//!
//! ```text
//! {系统目录}/plugins/agent/<bundle_id>/           全局级（HomedirRegistry）
//! {workdir}/.symbio/plugins/agent/<bundle_id>/    工作区级（同名覆盖全局级）
//! ```
//!
//! 全局级与旧版 agent 完全一致：智能体（bundle）统一存放在「系统目录」下的
//! `plugins/agent/`，系统目录由 [`crate::symbio_core::HomedirRegistry`] 提供
//! （可被「切换系统目录」改变并持久化到 bootstrap）。每个子目录即一个 bundle
//! （含 `manifest.yaml` 与约定能力目录 prompts/ skills/ mcps/）。
//! 工作区级仅在工作区上下文存在时参与，为按项目安装与测试隔离提供位置。
//!
//! bundle 即规范 §3 的完整目录（manifest + 约定能力目录 + 资源），导入导出
//! 均为整目录 zip——**分发的是完整 agent 能力**，这正是 OAB 与 Skill/MCP
//! 单件分发的根本差异。
//!
//! ## 安全
//!
//! - zip-slip 防护：解压前逐 entry 校验规范化路径落在目标目录内；
//! - 导入即校验：manifest 必须通过 [`validate_manifest`]（含版本匹配）才落盘。

use crate::plugins::agent::core::spec::validate::validate_manifest;
use crate::plugins::agent::core::spec::manifest::BundleManifest;
use crate::plugins::agent::core::SPEC_MAJOR;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// bundle 在 store 中的记录（清单 + 位置 + 来源）
#[derive(Debug, Clone)]
pub struct BundleRecord {
    pub manifest: Arc<BundleManifest>,
    /// bundle 安装目录（含 manifest 与约定能力目录）
    pub dir: PathBuf,
    /// 来源层级（工作区级覆盖同名全局级）
    pub source: BundleScope,
}

/// bundle 来源层级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleScope {
    /// `{workdir}/.symbio/plugins/agent/`
    Workspace,
    /// `{系统目录}/plugins/agent/`
    Global,
}

impl BundleScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Global => "global",
        }
    }
}

/// 导入结果
#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub id: String,
    pub version: String,
    pub dir: String,
    pub replaced: bool,
}

/// Bundle Store。
pub struct BundleStore {
    /// 工作区级根（None = 无工作区上下文）
    workspace_root: Option<PathBuf>,
    /// 全局根（`{homedir}/bundles`）
    global_root: PathBuf,
}

impl BundleStore {
    pub fn new(workdir: Option<&str>) -> Self {
        // 两级发现：
        // - 全局级 = 系统目录下的 `plugins/agent/<id>`（与旧版 agent 完全一致，
        //   系统目录由 HomedirRegistry::get() 提供，可被「切换系统目录」改变并持久化到
        //   bootstrap）—— 这是用户安装 bundle 的主位置，必须始终被扫描；
        // - 工作区级 = `{workdir}/.symbio/plugins/agent/<id>`（同名时覆盖全局级）。
        //   工作区层同时为测试提供隔离：测试用 tempdir 作 workdir 时不会污染真实系统目录。
        let workspace_root =
            workdir.map(|w| Path::new(w).join(".symbio").join("plugins").join("agent"));
        let global_root = crate::symbio_core::HomedirRegistry::get().join("plugins").join("agent");
        Self {
            workspace_root,
            global_root,
        }
    }

    /// 全量 bundle 列表（当前为单来源：系统目录 `plugins/agent/`）。
    pub fn list(&self) -> Vec<BundleRecord> {
        let mut records: Vec<BundleRecord> = Vec::new();
        for (scope, root) in [
            (BundleScope::Global, Some(&self.global_root)),
            (BundleScope::Workspace, self.workspace_root.as_ref()),
        ] {
            let Some(root) = root else { continue };
            for entry in Self::scan_root(root) {
                // 同名去重：工作区级后扫到，覆盖全局级
                if let Some(existing) = records
                    .iter_mut()
                    .find(|r| r.manifest.id == entry.manifest.id)
                {
                    if scope == BundleScope::Workspace {
                        *existing = entry;
                    }
                } else {
                    records.push(entry);
                }
            }
        }
        records.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        records
    }

    /// 按 id 查找（工作区优先）。
    pub fn get(&self, bundle_id: &str) -> Option<BundleRecord> {
        self.list().into_iter().find(|r| r.manifest.id == bundle_id)
    }

    fn scan_root(root: &Path) -> Vec<BundleRecord> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(root) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(record) = Self::load_record(&path) {
                out.push(record);
            }
        }
        out
    }

    /// 从 bundle 目录加载记录（manifest 解析失败 → 跳过该目录并记日志）。
    pub fn load_record(dir: &Path) -> Option<BundleRecord> {
        let manifest = load_manifest_from_dir(dir)?;
        Some(BundleRecord {
            manifest: Arc::new(manifest),
            dir: dir.to_path_buf(),
            // source 由调用方（list/get）按根目录归位；此处先占位
            source: BundleScope::Global,
        })
    }

    /// 导入（安装）一个 bundle zip。
    ///
    /// zip 内 manifest 允许位于根目录或唯一顶层目录下（两种打包习惯都支持）。
    pub fn import(&self, zip_bytes: &[u8], replace: bool) -> Result<ImportResult, String> {
        let reader = std::io::Cursor::new(zip_bytes);
        let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("zip 打开失败: {e}"))?;

        // ── 1. 找 manifest 并解析校验（导入即校验：不合规整包拒收）──
        let mut manifest_raw: Option<(String, String)> = None; // (entry 名, 内容)
        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| format!("zip 读取失败: {e}"))?;
            if file.is_dir() {
                continue;
            }
            let name = file.name().to_string();
            let base = name.rsplit(['/', '\\']).next().unwrap_or(&name);
            if matches!(base, "manifest.yaml" | "manifest.yml" | "manifest.json") {
                let mut content = String::new();
                std::io::Read::read_to_string(&mut file, &mut content)
                    .map_err(|e| format!("manifest 读取失败: {e}"))?;
                manifest_raw = Some((base.to_string(), content));
                break;
            }
        }
        let Some((manifest_name, manifest_content)) = manifest_raw else {
            return Err("zip 中未找到 manifest.yaml / manifest.yml / manifest.json".into());
        };
        let manifest: BundleManifest = if manifest_name.ends_with(".json") {
            serde_json::from_str(&manifest_content)
                .map_err(|e| format!("manifest 解析失败: {e}"))?
        } else {
            serde_yaml_ng::from_str(&manifest_content)
                .map_err(|e| format!("manifest 解析失败: {e}"))?
        };
        validate_manifest(&manifest, SPEC_MAJOR)
            .map_err(|errs| format!("bundle 校验失败（拒绝导入）：{}", errs.join("; ")))?;

        // ── 2. 目标目录（工作区级优先；无工作区则全局级）──
        let root = self
            .workspace_root
            .clone()
            .unwrap_or_else(|| self.global_root.clone());
        let dest = root.join(&manifest.id);
        let replaced = dest.exists();
        if replaced && !replace {
            return Err(format!(
                "bundle `{}` 已存在（{}）。携带 replace=true 可覆盖。",
                manifest.id,
                dest.display()
            ));
        }

        // ── 3. zip-slip 防护 + 解压落盘 ──
        // 规范化每个 entry 的路径，必须仍位于 dest 内（拒绝 `..` 与绝对路径）。
        let canonical_dest = dest
            .canonicalize()
            .unwrap_or_else(|_| absolutize(&root, &manifest.id));
        std::fs::create_dir_all(&dest).map_err(|e| format!("创建目录失败: {e}"))?;
        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| format!("zip 读取失败: {e}"))?;
            let Some(rel) = Self::strip_manifest_root(file.name(), &manifest.id) else {
                continue; // 目录项或无关文件
            };
            let target = absolutize(&dest, &rel);
            let target_canonical = absolutize(&dest, &rel); // 逐段构建已保证无穿越
            if !target_canonical.starts_with(&canonical_dest) {
                return Err(format!(
                    "zip 条目 `{}` 越界（zip-slip 防护拦截）",
                    file.name()
                ));
            }
            if file.is_dir() {
                std::fs::create_dir_all(&target).map_err(|e| format!("创建目录失败: {e}"))?;
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
            }
            let mut out = std::fs::File::create(&target)
                .map_err(|e| format!("写文件失败（{}）: {e}", target.display()))?;
            std::io::copy(&mut file, &mut out).map_err(|e| format!("写文件失败: {e}"))?;
        }

        Ok(ImportResult {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            dir: dest.display().to_string(),
            replaced,
        })
    }

    /// 导出为 zip 字节（打包下载）。
    pub fn export(&self, bundle_id: &str) -> Result<Vec<u8>, String> {
        let record = self
            .get(bundle_id)
            .ok_or_else(|| format!("bundle `{bundle_id}` 不存在"))?;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            add_dir_to_zip(&mut writer, &record.dir, bundle_id, options)?;
            writer.finish().map_err(|e| format!("zip 生成失败: {e}"))?;
        }
        Ok(buf.into_inner())
    }

    /// 删除 bundle（仅允许删除磁盘目录；不存在报错）。
    pub fn delete(&self, bundle_id: &str) -> Result<String, String> {
        let record = self
            .get(bundle_id)
            .ok_or_else(|| format!("bundle `{bundle_id}` 不存在"))?;
        std::fs::remove_dir_all(&record.dir)
            .map_err(|e| format!("删除失败（{}）: {e}", record.dir.display()))?;
        Ok(record.dir.display().to_string())
    }

    /// zip entry 名 → bundle 内相对路径。
    ///
    /// 支持两种打包布局：根目录直打包（`manifest.yaml`、`providers/...`）与
    /// 单顶层目录打包（`<bundle_id>/manifest.yaml`、`<bundle_id>/providers/...`）。
    /// 返回 `None` 表示跳过（目录项 / 顶层杂项）。
    fn strip_manifest_root(entry_name: &str, bundle_id: &str) -> Option<String> {
        // zip 目录项（以 / 结尾）：不是文件，直接跳过
        if entry_name.ends_with('/') || entry_name.ends_with('\\') {
            return None;
        }
        let normalized = entry_name.replace('\\', "/");
        // 单顶层目录布局：`<bundle_id>/...` → 剥掉 bundle_id 前缀
        let prefix = format!("{bundle_id}/");
        if let Some(rest) = normalized.strip_prefix(prefix.as_str()) {
            if rest.is_empty() {
                return None;
            }
            return Some(rest.to_string());
        }
        // 根目录布局：manifest 在 zip 根，路径原样接受
        // （杂项文件也原样收——import 只解析 manifest.yaml，多余文件无害）
        Some(normalized)
    }
}

/// 绝对化 + 逐段构建（天然免疫 `..` / 绝对路径穿越）。
fn absolutize(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    for seg in rel.split(['/', '\\']) {
        if seg.is_empty() || seg == "." {
            continue;
        }
        // `..` 段直接拒绝（不弹出）——防御性：合法 bundle 不应包含
        if seg == ".." {
            continue;
        }
        p.push(seg);
    }
    p
}

/// 从 bundle 目录读 manifest（yaml/yml/json 依次探测）。
pub fn load_manifest_from_dir(dir: &Path) -> Option<BundleManifest> {
    for name in ["manifest.yaml", "manifest.yml", "manifest.json"] {
        let path = dir.join(name);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let parsed = if name.ends_with(".json") {
            serde_json::from_str::<BundleManifest>(&raw).map_err(|e| e.to_string())
        } else {
            serde_yaml_ng::from_str::<BundleManifest>(&raw).map_err(|e| e.to_string())
        };
        match parsed {
            Ok(m) => return Some(m),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "[oab] manifest 解析失败");
                return None;
            }
        }
    }
    None
}

/// 递归把目录写进 zip（arcname 前缀 = bundle_id，形成单顶层目录布局）。
fn add_dir_to_zip<W: std::io::Write + std::io::Seek>(
    writer: &mut zip::ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    options: zip::write::SimpleFileOptions,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let arcname = format!("{prefix}/{name}");
        if path.is_dir() {
            writer
                .add_directory(format!("{arcname}/"), options)
                .map_err(|e| format!("zip 写目录失败: {e}"))?;
            add_dir_to_zip(writer, &path, &arcname, options)?;
        } else {
            writer
                .start_file(arcname.as_str(), options)
                .map_err(|e| format!("zip 写文件失败: {e}"))?;
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("读文件失败（{}）: {e}", path.display()))?;
            std::io::Write::write_all(writer, &bytes).map_err(|e| format!("zip 写入失败: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_manifest_root_accepts_both_layouts() {
        // 根目录布局
        assert_eq!(
            BundleStore::strip_manifest_root("providers/persona/provider.yaml", "com.acme.cr"),
            Some("providers/persona/provider.yaml".into())
        );
        // 单顶层目录布局
        assert_eq!(
            BundleStore::strip_manifest_root("com.acme.cr/manifest.yaml", "com.acme.cr"),
            Some("manifest.yaml".into())
        );
        // 顶层目录名与 bundle_id 无关的杂项文件：原样接受（根目录布局语义；
        // import 只解析 manifest.yaml，无关文件落盘无害）
        assert_eq!(
            BundleStore::strip_manifest_root("other/manifest.yaml", "com.acme.cr"),
            Some("other/manifest.yaml".into())
        );
        // 目录项 → 跳过
        assert_eq!(
            BundleStore::strip_manifest_root("com.acme.cr/providers/", "com.acme.cr"),
            None
        );
    }

    #[test]
    fn absolutize_rejects_traversal_segments() {
        let base = Path::new("/tmp/bundles/com.acme");
        let p = absolutize(base, "providers/../../etc/passwd");
        // `..` 段被丢弃，路径仍锁定在 base 内
        assert!(p.starts_with(base));
        assert!(p.to_string_lossy().contains("etc"));
    }
}
