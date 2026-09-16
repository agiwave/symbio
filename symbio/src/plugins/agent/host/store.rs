//! Bundle Store —— OAB bundle 实例的磁盘存储与打包分发。
//!
//! ## 目录布局
//!
//! ```text
//! {系统目录}/agent/<bundle_id>/                   全局级（= 本插件自己的目录）
//! {workdir}/.symbio/agent/<bundle_id>/            工作区级（同名覆盖全局级）
//! ```
//!
//! 全局级：智能体（bundle）统一存放在**本插件自己的目录**下（装配态即
//! `<homedir>/agent`）。该目录由父插件经 `PLUGIN_DIR` 告知（见
//! [`BundleStore::new`]），本模块**不自己拼**——系统目录可被「切换系统目录」
//! 改变并持久化到 bootstrap，手拼就会与装配态不一致。每个子目录即一个 bundle
//! （含 `manifest.yaml` 与约定能力目录 prompts/ skills/ mcps/）。
//! 工作区级仅在工作区上下文存在时参与，为按项目安装与测试隔离提供位置。
//!
//! bundle 即规范 §3 的完整目录（manifest + 约定能力目录 + 条目），导入导出
//! 均为整目录 zip——**分发的是完整 agent 能力**，这正是 OAB 与 Skill/MCP
//! 单件分发的根本差异。
//!
//! ## 安全
//!
//! - zip-slip 防护：解压前逐 entry 校验规范化路径落在目标目录内；
//! - 导入即校验：manifest 必须通过 [`validate_manifest`]（含版本匹配）才落盘。

use crate::plugins::agent::core::spec::manifest::BundleManifest;
use crate::plugins::agent::core::spec::validate::validate_manifest;
use crate::plugins::agent::core::SPEC_MAJOR;
use crate::symbio_core::AGENTS_FILE;
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
    /// `{workdir}/.symbio/agent/`
    Workspace,
    /// `{系统目录}/agent/`（本插件自己的目录）
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

/// bundle 内部条目（prompts / skills / mcps 的结构化清单项）。
///
/// `path` 是相对 bundle 目录的路径，也是 VDFS 容器语义
/// （`vdfs/*` 携带 `container`）下的条目操作键 `id`；
/// 命名与 [`assemble_bundle`] 的扫描规则严格一致（装配结果可直接复现）。
#[derive(Debug, Clone, Serialize)]
pub struct BundleItemEntry {
    /// 条目类别：`prompt` | `skill` | `mcp`
    pub kind: String,
    /// 条目名（prompt=文件 stem；skill=目录名；mcp=文件名或目录名，与装配 source 命名一致）
    pub name: String,
    /// 相对 bundle 目录的路径（如 `prompts/persona.md`）
    pub path: String,
    /// 提示词片段优先级（prompt 缺省 10 / skill 缺省 50；mcp 为 None）
    pub priority: Option<i64>,
    /// 文件字节数
    pub size: u64,
}

/// 校验并分类 bundle 内部条目的相对路径。
///
/// 规则与 [`crate::plugins::agent::core::spec::assembly`] 的扫描严格对齐：
/// - `prompts/<name>.md`（或 `.markdown`，单层，装配只扫直接子文件）
/// - `skills/<name>/SKILL.md`（目录形态）
/// - `mcps/<name>.{yaml,yml,json}`（文件形态）或
///   `mcps/<name>/{config,server,mcp}.{yaml,yml,json}`（目录形态）
///
/// 拒绝绝对路径、`..` 段与一切不合规布局（路径沙箱第一道闸）。
/// 返回 `(kind, name)`。
pub fn classify_item_path(rel: &str) -> Result<(&'static str, String), String> {
    let rel = rel.trim().replace('\\', "/");
    let rel = rel.trim_start_matches("./");
    if rel.is_empty() || rel.starts_with('/') {
        return Err(format!("非法条目路径 `{rel}`"));
    }
    if rel.split('/').any(|seg| seg == ".." || seg.is_empty()) {
        return Err(format!("条目路径 `{rel}` 含非法段（`..` / 空段）"));
    }
    if let Some(rest) = rel.strip_prefix("prompts/") {
        if rest.contains('/') {
            return Err("prompt 条目必须位于 prompts/ 直接子层（prompts/<name>.md）".into());
        }
        let stem = rest
            .strip_suffix(".md")
            .or_else(|| rest.strip_suffix(".markdown"))
            .ok_or_else(|| format!("prompt 条目必须是 Markdown（`{rest}`）"))?;
        if stem.is_empty() {
            return Err("prompt 条目名不能为空".into());
        }
        return Ok(("prompt", stem.to_string()));
    }
    if let Some(rest) = rel.strip_prefix("skills/") {
        let mut parts = rest.splitn(2, '/');
        let (Some(dir), Some(file)) = (parts.next(), parts.next()) else {
            return Err("skill 条目必须是目录形态（skills/<name>/SKILL.md）".into());
        };
        if dir.is_empty() || dir.contains('/') {
            return Err(format!("非法 skill 目录名 `{dir}`"));
        }
        if file != "SKILL.md" {
            return Err(format!("skill 条目文件必须是 SKILL.md（得到 `{file}`）"));
        }
        return Ok(("skill", dir.to_string()));
    }
    if let Some(rest) = rel.strip_prefix("mcps/") {
        if let Some((dir, file)) = rest.split_once('/') {
            // 目录形态：mcps/<name>/{config,server,mcp}.{yaml,yml,json}
            let base = file.rsplit('.').next().unwrap_or("");
            if !matches!(base, "yaml" | "yml" | "json") {
                return Err(format!("mcp 目录形态配置必须是 yaml/yml/json（`{file}`）"));
            }
            let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
            if !matches!(stem, "config" | "server" | "mcp") {
                return Err(format!(
                    "mcp 目录形态配置文件必须是 config/server/mcp.*（`{file}`）"
                ));
            }
            if dir.is_empty() {
                return Err("mcp 目录名不能为空".into());
            }
            return Ok(("mcp", dir.to_string()));
        }
        // 文件形态：mcps/<name>.{yaml,yml,json}
        let ext = rest.rsplit('.').next().unwrap_or("");
        if !matches!(ext, "yaml" | "yml" | "json") {
            return Err(format!("mcp 文件形态必须是 yaml/yml/json（`{rest}`）"));
        }
        // 命名与装配一致：文件形态 name = 文件全名（含扩展名）
        return Ok(("mcp", rest.to_string()));
    }
    Err(format!(
        "条目路径必须以 prompts/ skills/ mcps/ 开头（得到 `{rel}`）"
    ))
}

/// 解析 Markdown 文件的 frontmatter priority（无 / 非数值 → None）。
fn frontmatter_priority(content: &str) -> Option<i64> {
    let (fm, _) = crate::plugins::agent::core::spec::assembly::split_frontmatter(content);
    fm.get("priority").and_then(|v| v.as_i64())
}

/// 单文件的 (size, priority) 元数据（prompt / skill 共用）。
fn file_meta(path: &Path) -> (u64, Option<i64>) {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let priority = std::fs::read_to_string(path)
        .ok()
        .and_then(|c| frontmatter_priority(&c));
    (size, priority)
}

/// Bundle Store。
pub struct BundleStore {
    /// 工作区级根（None = 无工作区上下文）
    workspace_root: Option<PathBuf>,
    /// 全局根 = **本插件自己的目录**（`<homedir>/agent`）
    global_root: PathBuf,
}

impl BundleStore {
    /// `global_root` = 本插件自己的目录。
    ///
    /// ⚠️ 这个目录**必须由调用方给**（装配态下来自父插件经 `PLUGIN_DIR` 传下的
    /// [`PluginDir`]，见 `AgentPlugin::build`），本文件**不得**自己拼
    /// `<homedir>/…/agent`：插件只认父插件告知的目录，这是「插件不认识全局布局」
    /// 这条约束的一部分——手拼就等于把布局知识复制一份，改布局时必然漏改。
    pub fn new(global_root: impl Into<PathBuf>, workdir: Option<&str>) -> Self {
        // 两级发现：
        // - 全局级 = 本插件目录下的 `<id>`（目录由父插件告知，可被「切换系统目录」
        //   改变并持久化到 bootstrap）—— 这是用户安装 bundle 的主位置，必须始终被扫描；
        // - 工作区级 = `{workdir}/.symbio/agent/<id>`（同名时覆盖全局级）。
        //   工作区层同时为测试提供隔离：测试用 tempdir 作 workdir 时不会污染真实系统目录。
        let workspace_root = workdir.map(|w| Path::new(w).join(".symbio").join("agent"));
        Self {
            workspace_root,
            global_root: global_root.into(),
        }
    }

    /// 全量 bundle 列表（当前为单来源：本插件目录下的 `<id>`）。
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

    // ==================== bundle 内部条目（prompts / skills / mcps） ====================
    //
    // 单文件级读写，供宿主 UI 在 Agent 详情页内直接管理 bundle 能力来源。
    // 安全模型：rel_path 必须先过 [`classify_item_path`]（白名单布局 +
    // 拒绝 `..`），再经 [`absolutize`] 逐段构建（免疫穿越），双重闸门。

    /// 列出 bundle 内部条目（结构化清单，扫描规则与装配严格一致）。
    pub fn list_items(&self, bundle_id: &str) -> Result<Vec<BundleItemEntry>, String> {
        let record = self
            .get(bundle_id)
            .ok_or_else(|| format!("bundle `{bundle_id}` 不存在"))?;
        let mut out: Vec<BundleItemEntry> = Vec::new();

        // prompts/<name>.md（直接子文件）
        if let Ok(entries) = std::fs::read_dir(record.dir.join("prompts")) {
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_file() {
                    continue;
                }
                let fname = e.file_name().to_string_lossy().to_string();
                let Some(stem) = fname
                    .strip_suffix(".md")
                    .or_else(|| fname.strip_suffix(".markdown"))
                else {
                    continue; // 非 Markdown 忽略（与装配一致）
                };
                let (size, priority) = file_meta(&p);
                out.push(BundleItemEntry {
                    kind: "prompt".into(),
                    name: stem.to_string(),
                    path: format!("prompts/{fname}"),
                    priority,
                    size,
                });
            }
        }

        // skills/<name>/SKILL.md
        if let Ok(entries) = std::fs::read_dir(record.dir.join("skills")) {
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let name = e.file_name().to_string_lossy().to_string();
                let skill = p.join("SKILL.md");
                if !skill.is_file() {
                    continue;
                }
                let (size, priority) = file_meta(&skill);
                out.push(BundleItemEntry {
                    kind: "skill".into(),
                    name,
                    path: format!("skills/{}/SKILL.md", e.file_name().to_string_lossy()),
                    priority,
                    size,
                });
            }
        }

        // mcps/（文件形态 + 目录形态）
        if let Ok(entries) = std::fs::read_dir(record.dir.join("mcps")) {
            for e in entries.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if p.is_file() {
                    let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
                    if !matches!(ext, "yaml" | "yml" | "json") {
                        continue;
                    }
                    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    out.push(BundleItemEntry {
                        kind: "mcp".into(),
                        name: name.clone(),
                        path: format!("mcps/{name}"),
                        priority: None,
                        size,
                    });
                } else if p.is_dir() {
                    let candidates = [
                        "config.yaml",
                        "config.yml",
                        "server.yaml",
                        "server.yml",
                        "mcp.yaml",
                        "mcp.yml",
                        "config.json",
                        "server.json",
                        "mcp.json",
                    ];
                    let Some(cfg) = candidates.iter().map(|c| p.join(c)).find(|c| c.is_file())
                    else {
                        continue; // 无配置的目录形态：与装配一样记不了条目，跳过
                    };
                    let cfg_name = cfg.file_name().unwrap_or_default().to_string_lossy();
                    let size = std::fs::metadata(&cfg).map(|m| m.len()).unwrap_or(0);
                    out.push(BundleItemEntry {
                        kind: "mcp".into(),
                        name,
                        path: format!("mcps/{}/{}", e.file_name().to_string_lossy(), cfg_name),
                        priority: None,
                        size,
                    });
                }
            }
        }

        out.sort_by(|a, b| (&a.kind, &a.path).cmp(&(&b.kind, &b.path)));
        Ok(out)
    }

    /// 读取 bundle 内部条目文件内容。
    pub fn read_item(&self, bundle_id: &str, rel_path: &str) -> Result<String, String> {
        let record = self
            .get(bundle_id)
            .ok_or_else(|| format!("bundle `{bundle_id}` 不存在"))?;
        classify_item_path(rel_path)?;
        let full = absolutize(&record.dir, rel_path);
        debug_assert!(full.starts_with(&record.dir));
        std::fs::read_to_string(&full)
            .map_err(|e| format!("读取条目失败（{}）: {e}", full.display()))
    }

    /// 写入（创建/覆盖）bundle 内部条目文件；父目录自动创建。
    ///
    /// `max_bytes` 是**写入闸门**：条目内容超过上限直接拒绝，不截断——人格 / 技能是
    /// 跨会话生效的东西，「以为写进去了、实际少了一段」是这里最坏的失败形态
    /// （没有任何报错，只表现为智能体行为异常）。拒绝则是一次显式、可重试的失败。
    pub fn write_item(
        &self,
        bundle_id: &str,
        rel_path: &str,
        content: &str,
        max_bytes: usize,
    ) -> Result<(), String> {
        let record = self
            .get(bundle_id)
            .ok_or_else(|| format!("bundle `{bundle_id}` 不存在"))?;
        classify_item_path(rel_path)?;
        if content.len() > max_bytes {
            return Err(format!(
                "条目内容超出容量上限：当前 {} 字节，上限 {max_bytes} 字节（{rel_path}）。\
                 请精简后再写入。",
                content.len()
            ));
        }
        let full = absolutize(&record.dir, rel_path);
        debug_assert!(full.starts_with(&record.dir));
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败（{}）: {e}", parent.display()))?;
        }
        std::fs::write(&full, content)
            .map_err(|e| format!("写入条目失败（{}）: {e}", full.display()))
    }

    /// 删除 bundle 内部条目文件；skill / mcp 目录形态下若父目录因此变空则一并清理。
    pub fn delete_item(&self, bundle_id: &str, rel_path: &str) -> Result<(), String> {
        let record = self
            .get(bundle_id)
            .ok_or_else(|| format!("bundle `{bundle_id}` 不存在"))?;
        let (kind, _) = classify_item_path(rel_path)?;
        let full = absolutize(&record.dir, rel_path);
        debug_assert!(full.starts_with(&record.dir));
        if !full.is_file() {
            return Err(format!("条目不存在（{}）", full.display()));
        }
        std::fs::remove_file(&full)
            .map_err(|e| format!("删除条目失败（{}）: {e}", full.display()))?;
        // 目录形态（skills/<name>/、mcps/<name>/）清空后顺手移除空目录
        if kind != "prompt" {
            if let Some(parent) = full.parent() {
                if parent != record.dir
                    && parent.starts_with(&record.dir)
                    && std::fs::read_dir(parent)
                        .map(|mut d| d.next().is_none())
                        .unwrap_or(false)
                {
                    let _ = std::fs::remove_dir(parent);
                }
            }
        }
        Ok(())
    }

    // ==================== 智能体记忆（bundle 根下的 `AGENTS.md`） ====================
    //
    // 记忆**不是条目**：它不参与 `classify_item_path` 的白名单（那条白名单描述的是
    // 「bundle 的约定能力目录」prompts/ skills/ mcps/，记忆不在其列），也不计入
    // 概览里的条目计数。它是 bundle 根下的一个普通文件，与工作区级的
    // `{workdir}/AGENTS.md` **同名同语义**（见 `symbio_core::memory`）——
    // 放哪个作用域就管哪个作用域。
    //
    // ⚠️ 本模块**只负责回答「记忆文件在哪」**：读 / 写 / 两道容量闸门一律走内核
    // （`symbio_core::memory::MemoryFile`）。此前这里自带一份 `read_memory` /
    // `write_memory` 与自己的字节闸门，与 work / session 两层各写一份口径——
    // 「超限是拒绝还是截断」「读不到算不算错误」一旦分叉，用户看到的行为就会随
    // 「这条记忆属于哪一层」而变化。收口后三层共用同一份实现，本模块不再持有闸门。

    /// 智能体记忆文件：`<bundle 目录>/AGENTS.md`
    ///
    /// bundle 不存在 → 明确报错（调用方 [`super::memory::store`] 据此构造「无作用域」门面）。
    pub fn memory_path(&self, bundle_id: &str) -> Result<PathBuf, String> {
        let record = self
            .get(bundle_id)
            .ok_or_else(|| format!("bundle `{bundle_id}` 不存在"))?;
        Ok(record.dir.join(AGENTS_FILE))
    }

    /// zip entry 名 → bundle 内相对路径。    ///
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

    #[test]
    fn classify_accepts_convention_layouts() {
        assert_eq!(
            classify_item_path("prompts/persona.md"),
            Ok(("prompt", "persona".into()))
        );
        assert_eq!(
            classify_item_path("prompts/a.markdown"),
            Ok(("prompt", "a".into()))
        );
        assert_eq!(
            classify_item_path("skills/playbook/SKILL.md"),
            Ok(("skill", "playbook".into()))
        );
        assert_eq!(
            classify_item_path("mcps/search.yaml"),
            Ok(("mcp", "search.yaml".into()))
        );
        assert_eq!(
            classify_item_path("mcps/search/config.yaml"),
            Ok(("mcp", "search".into()))
        );
        assert_eq!(
            classify_item_path("mcps/search/server.json"),
            Ok(("mcp", "search".into()))
        );
        // 反斜杠 + ./ 前缀归一化
        assert_eq!(
            classify_item_path("./prompts\\x.md"),
            Ok(("prompt", "x".into()))
        );
    }

    #[test]
    fn classify_rejects_escapes_and_misfits() {
        for bad in [
            "manifest.yaml",
            "providers/x.yaml",
            "../etc/passwd",
            "/abs/path.md",
            "prompts/sub/deep.md",
            "prompts/x.txt",
            "skills/x/other.md",
            "skills/x",
            "mcps/x.txt",
            "mcps/x/other.yaml",
            "mcps/x/deep/config.yaml",
            "prompts/",
            "",
        ] {
            assert!(classify_item_path(bad).is_err(), "should reject `{bad}`");
        }
    }

    /// 在工作区级落一个最小 bundle（不经 zip：以下用例只关心写入闸门）
    fn workspace_store_with_bundle() -> (tempfile::TempDir, BundleStore) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = BundleStore::new(
            dir.path().join("global-agent"),
            Some(dir.path().to_str().unwrap()),
        );
        let bundle = dir.path().join(".symbio/agent/b");
        std::fs::create_dir_all(bundle.join("prompts")).unwrap();
        std::fs::write(
            bundle.join("manifest.yaml"),
            "spec: \"oab/v1\"\nid: \"b\"\nname: \"B\"\nversion: \"1.0.0\"\nrequires:\n  spec: \"^1\"\n",
        )
        .unwrap();
        assert!(store.get("b").is_some(), "前置：bundle 应被扫描到");
        (dir, store)
    }

    /// 写入闸门：超限**拒绝**，且不留下半截内容
    #[test]
    fn write_item_rejects_oversized_content_without_touching_the_file() {
        let (_dir, store) = workspace_store_with_bundle();

        store
            .write_item("b", "prompts/persona.md", "0123456789", 10)
            .unwrap();
        let err = store
            .write_item("b", "prompts/persona.md", "0123456789X", 10)
            .unwrap_err();
        assert!(err.contains("超出容量上限"), "{err}");
        assert!(err.contains("10"), "错误信息要带上限值：{err}");
        assert_eq!(
            store.read_item("b", "prompts/persona.md").unwrap(),
            "0123456789",
            "被拒绝的写入不得改动文件"
        );
    }

    /// 智能体记忆**落位**在 bundle 自己的目录（不是工作区目录），文件名与工作区级同名。
    ///
    /// 读写与两道容量闸门不在这里测——它们已收口到内核，用例在
    /// `agent/host/memory.test.rs`（本模块只回答「记忆文件在哪」）。
    #[test]
    fn memory_lives_in_the_bundle_dir() {
        let (dir, store) = workspace_store_with_bundle();
        let path = store.memory_path("b").unwrap();
        assert_eq!(path, dir.path().join(".symbio/agent/b").join(AGENTS_FILE));
        assert_eq!(path.file_name().unwrap(), "AGENTS.md");
        // 不是工作区根的那个 AGENTS.md
        assert_ne!(path, dir.path().join(AGENTS_FILE));
        // bundle 不存在 → 明确报错（`memory::store` 据此构造「无作用域」门面）
        assert!(store.memory_path("nope").is_err());
    }
}
