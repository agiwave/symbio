//! Agent 目录的磁盘存储与整包分发（zip 导入 / 导出）
//!
//! ## 目录布局
//!
//! ```text
//! {系统目录}/agent/<id>/                   本插件自己的目录（= 全局唯一来源）
//! ```
//!
//! Agent 统一存放在**本插件自己的目录**下（装配态即 `<homedir>/agent`）。
//! 该目录由父插件经 `PLUGIN_DIR` 告知（见 [`AgentDirStore::new`]），本模块**不自己拼**
//! ——系统目录可被「切换系统目录」改变并持久化到 bootstrap，手拼就会与装配态不一致。
//!
//! **与 workdir / 工作区无关**：agent 目录只由 `homedir` 经 `PLUGIN_DIR` 推出，绝不
//! 把 workdir 当第二发现根（那是递归与「子 Agent 只看到 Skill/Mcp」的根因）。
//!
//! 一个 Agent 就是规范 §4 的一个目录，导入导出均为整目录 zip——**分发的是完整
//! agent 能力**。
//!
//! ## 职责边界：只管目录，不解释内容
//!
//! v1 时代这里**解释** agent 目录内部：`prompts/` `skills/` `mcps/` 各有白名单布局，
//! 条目按 `priority` 排序、MCP 配置按约定文件名探测……那等于在宿主里重写了一遍
//! 技能系统与 MCP 客户端的解析，两条链长期不同步（规范 §3.2 第 2 条）。
//!
//! v2 里 Agent 是**一棵插件树**，能力由目录里的插件实例自己解释。本模块因此降级
//! 为**枚举 / 建目录 / 导入导出 + 通用文件读写**：
//!
//! - 只认 `manifest.yaml`（身份与兼容门槛，§5），不认任何能力目录；
//! - 条目读写只做**路径沙箱**（§11.1），不校验「这个文件该长什么样」。
//!
//! ## 安全
//!
//! - zip-slip 防护：解压前逐 entry 校验规范化路径落在目标目录内；
//! - 导入即校验：manifest 必须通过 [`super::manifest::validate`]（§10 版本门槛）
//!   才落盘；不合规整包拒收，不静默降级。

use super::manifest::{self, AgentManifest};
use crate::symbio_core::AGENTS_FILE;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Agent 在 store 中的记录（清单 + 位置 + 来源）
#[derive(Debug, Clone)]
pub struct AgentDirRecord {
    pub manifest: Arc<AgentManifest>,
    /// agent 目录安装目录（含 manifest 与约定能力目录）
    pub dir: PathBuf,
    /// 来源层级（工作区级覆盖同名全局级）
    pub source: AgentDirScope,
}

/// agent 目录来源层级（当前仅全局级：本插件自己的目录）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDirScope {
    /// `{系统目录}/agent/`（本插件自己的目录；与 workdir 无关）
    Global,
}

impl AgentDirScope {
    pub fn as_str(&self) -> &'static str {
        match self {
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

/// Agent 目录内的一条文件记录（**通用**：不分类、不解释内容）
///
/// v1 的 `条目类型` 带 `kind`（prompt / skill / mcp）与 `priority`——那是
/// 宿主在替能力目录解释语义。v2 里能力由插件实例自己解释（§3.2），这里只回答
/// 「有哪些文件、多大、是不是目录」。
#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    /// 相对 Agent 目录的路径（如 `skill/foo/SKILL.md`）
    pub path: String,
    /// 文件字节数（目录为 0）
    pub size: u64,
    pub is_dir: bool,
}

/// 校验 Agent 内部条目的相对路径（**只做路径沙箱**，规范 §11.1）。
///
/// 拒绝绝对路径、`..` 段与空段；除此之外**不限制布局**——Agent 目录里该有什么
/// 由 §4 与宿主剖面决定，不是本模块的判断。
pub fn normalize_item_path(rel: &str) -> Result<String, String> {
    let rel = rel.trim().replace('\\', "/");
    let rel = rel.trim_start_matches("./");
    if rel.is_empty() || rel.starts_with('/') {
        return Err(format!("非法条目路径 `{rel}`"));
    }
    if rel.split('/').any(|seg| seg == ".." || seg.is_empty()) {
        return Err(format!("条目路径 `{rel}` 含非法段（`..` / 空段）"));
    }
    Ok(rel.to_string())
}

/// Agent 目录存储。
pub struct AgentDirStore {
    /// 全局根 = **本插件自己的目录**（`<homedir>/agent`，由 `PLUGIN_DIR` 告知）
    global_root: PathBuf,
}

impl AgentDirStore {
    /// `global_root` = 本插件自己的目录。
    ///
    /// ⚠️ 这个目录**必须由调用方给**（装配态下来自父插件经 `PLUGIN_DIR` 传下的
    /// [`PluginDir`]，见 `AgentPlugin::build`），本文件**不得**自己拼
    /// `<homedir>/…/agent`：插件只认父插件告知的目录，这是「插件不认识全局布局」
    /// 这条约束的一部分——手拼就等于把布局知识复制一份，改布局时必然漏改。
    ///
    /// agent 目录只由本插件目录决定，与 workdir / 工作区无关。
    pub fn new(global_root: impl Into<PathBuf>) -> Self {
        Self {
            global_root: global_root.into(),
        }
    }

    /// 全量 agent 目录列表（唯一来源：本插件目录下的 `<id>`）。
    pub fn list(&self) -> Vec<AgentDirRecord> {
        let mut records: Vec<AgentDirRecord> = Self::scan_root(&self.global_root);
        records.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        records
    }

    /// 按 id 查找（工作区优先）。
    pub fn get(&self, agent_id: &str) -> Option<AgentDirRecord> {
        self.list().into_iter().find(|r| r.manifest.id == agent_id)
    }

    fn scan_root(root: &Path) -> Vec<AgentDirRecord> {
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

    /// 从 agent 目录加载记录（manifest 解析失败 → 跳过该目录并记日志）。
    pub fn load_record(dir: &Path) -> Option<AgentDirRecord> {
        let manifest = manifest::load(dir)?;
        Some(AgentDirRecord {
            manifest: Arc::new(manifest),
            dir: dir.to_path_buf(),
            // source 由调用方（list/get）按根目录归位；此处先占位
            source: AgentDirScope::Global,
        })
    }

    /// 导入（安装）一个 agent 目录zip。
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
        let manifest: AgentManifest = if manifest_name.ends_with(".json") {
            serde_json::from_str(&manifest_content)
                .map_err(|e| format!("manifest 解析失败: {e}"))?
        } else {
            serde_yaml_ng::from_str(&manifest_content)
                .map_err(|e| format!("manifest 解析失败: {e}"))?
        };
        // §10：版本门槛是**接入前提**，不匹配整包拒收（不静默降级）
        manifest::validate(&manifest).map_err(|e| format!("智能体校验失败（拒绝导入）：{e}"))?;

        // ── 2. 目标目录（本插件自己的目录，与 workdir 无关）──
        let dest = self.global_root.join(&manifest.id);
        let replaced = dest.exists();
        if replaced && !replace {
            return Err(format!(
                "agent 目录`{}` 已存在（{}）。携带 replace=true 可覆盖。",
                manifest.id,
                dest.display()
            ));
        }

        // ── 3. zip-slip 防护 + 解压落盘 ──
        // 规范化每个 entry 的路径，必须仍位于 dest 内（拒绝 `..` 与绝对路径）。
        let canonical_dest = dest.canonicalize().unwrap_or_else(|_| dest.clone());
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
    pub fn export(&self, agent_id: &str) -> Result<Vec<u8>, String> {
        let record = self
            .get(agent_id)
            .ok_or_else(|| format!("agent 目录`{agent_id}` 不存在"))?;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default();
            add_dir_to_zip(&mut writer, &record.dir, agent_id, options)?;
            writer.finish().map_err(|e| format!("zip 生成失败: {e}"))?;
        }
        Ok(buf.into_inner())
    }

    /// 删除 agent 目录（仅允许删除磁盘目录；不存在报错）。
    pub fn delete(&self, agent_id: &str) -> Result<String, String> {
        let record = self
            .get(agent_id)
            .ok_or_else(|| format!("agent 目录`{agent_id}` 不存在"))?;
        std::fs::remove_dir_all(&record.dir)
            .map_err(|e| format!("删除失败（{}）: {e}", record.dir.display()))?;
        Ok(record.dir.display().to_string())
    }

    // ==================== Agent 目录内的通用文件读写 ====================
    //
    // 供宿主 UI 浏览 / 编辑 Agent 目录。安全模型：rel_path 先过
    // [`normalize_item_path`]（路径沙箱，§11.1），再经 [`absolutize`] 逐段构建
    // （免疫穿越），双重闸门。**不解释文件内容**——那属于对应的能力插件。

    /// 列出 Agent 目录（或其子目录）下的条目。
    ///
    /// `rel` 为 `""` 时列根目录；只列一层（子目录以 `is_dir` 标记，可再次进入）。
    pub fn list_files(&self, agent_id: &str, rel: &str) -> Result<Vec<FileEntry>, String> {
        let record = self
            .get(agent_id)
            .ok_or_else(|| format!("智能体 `{agent_id}` 不存在"))?;
        let rel = normalize_item_path(rel).unwrap_or_default();
        let dir = absolutize(&record.dir, &rel);
        debug_assert!(dir.starts_with(&record.dir));
        if !dir.is_dir() {
            return Err(format!("不是目录：{}", dir.display()));
        }
        let mut out: Vec<FileEntry> = Vec::new();
        for e in std::fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let meta = std::fs::metadata(&p);
            out.push(FileEntry {
                path,
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                is_dir: p.is_dir(),
            });
        }
        // 目录在前，其次按路径（与 VDFS 其它挂载点同一口径）
        out.sort_by(|x, y| (y.is_dir, &x.path).cmp(&(x.is_dir, &y.path)));
        Ok(out)
    }

    /// 取一条条目的元信息（不存在 / 逃逸 → `Err`）。
    pub fn stat_item(&self, agent_id: &str, rel: &str) -> Result<FileEntry, String> {
        let record = self
            .get(agent_id)
            .ok_or_else(|| format!("智能体 `{agent_id}` 不存在"))?;
        let rel = normalize_item_path(rel)?;
        let full = absolutize(&record.dir, &rel);
        debug_assert!(full.starts_with(&record.dir));
        let meta = std::fs::metadata(&full).map_err(|_| format!("条目不存在（`{rel}`）"))?;
        Ok(FileEntry {
            path: rel,
            size: meta.len(),
            is_dir: meta.is_dir(),
        })
    }

    /// 条目在磁盘上的绝对路径（沙箱化后；供删除目录用）。
    pub fn item_path(&self, agent_id: &str, rel: &str) -> Result<PathBuf, String> {
        let record = self
            .get(agent_id)
            .ok_or_else(|| format!("智能体 `{agent_id}` 不存在"))?;
        let rel = normalize_item_path(rel)?;
        Ok(absolutize(&record.dir, &rel))
    }

    /// 读取 Agent 目录内的文件内容。
    pub fn read_item(&self, agent_id: &str, rel_path: &str) -> Result<String, String> {
        let record = self
            .get(agent_id)
            .ok_or_else(|| format!("智能体 `{agent_id}` 不存在"))?;
        let rel_path = normalize_item_path(rel_path)?;
        let full = absolutize(&record.dir, &rel_path);
        debug_assert!(full.starts_with(&record.dir));
        std::fs::read_to_string(&full)
            .map_err(|e| format!("读取条目失败（{}）: {e}", full.display()))
    }

    /// 写入（创建/覆盖）Agent 目录内的文件；父目录自动创建。
    ///
    /// `max_bytes` 是**写入闸门**：条目内容超过上限直接拒绝，不截断——人格 / 技能是
    /// 跨会话生效的东西，「以为写进去了、实际少了一段」是这里最坏的失败形态
    /// （没有任何报错，只表现为智能体行为异常）。拒绝则是一次显式、可重试的失败。
    pub fn write_item(
        &self,
        agent_id: &str,
        rel_path: &str,
        content: &str,
        max_bytes: usize,
    ) -> Result<(), String> {
        let record = self
            .get(agent_id)
            .ok_or_else(|| format!("智能体 `{agent_id}` 不存在"))?;
        let rel_path = normalize_item_path(rel_path)?;
        if content.len() > max_bytes {
            return Err(format!(
                "条目内容超出容量上限：当前 {} 字节，上限 {max_bytes} 字节（{rel_path}）。\
                 请精简后再写入。",
                content.len()
            ));
        }
        let full = absolutize(&record.dir, &rel_path);
        debug_assert!(full.starts_with(&record.dir));
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败（{}）: {e}", parent.display()))?;
        }
        std::fs::write(&full, content)
            .map_err(|e| format!("写入条目失败（{}）: {e}", full.display()))
    }

    /// 删除 Agent 目录内的文件；所在目录因此变空则一并清理。
    pub fn delete_item(&self, agent_id: &str, rel_path: &str) -> Result<(), String> {
        let record = self
            .get(agent_id)
            .ok_or_else(|| format!("智能体 `{agent_id}` 不存在"))?;
        let rel_path = normalize_item_path(rel_path)?;
        let full = absolutize(&record.dir, &rel_path);
        debug_assert!(full.starts_with(&record.dir));
        if !full.is_file() {
            return Err(format!("条目不存在（{}）", full.display()));
        }
        std::fs::remove_file(&full)
            .map_err(|e| format!("删除条目失败（{}）: {e}", full.display()))?;
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
        Ok(())
    }

    // ==================== 智能体记忆（Agent 根下的 `AGENTS.md`，§6） ====================
    //
    // 记忆是 Agent 根下的一个普通文件，与工作区级的 `{workdir}/AGENTS.md`
    // **同名同语义**（见 `symbio_core::memory`）——放哪个作用域就管哪个作用域。
    // §6.2：一个作用域只有一个所有者；v2 里 Agent 作用域的所有者是它的 `work`
    // 插件实例，本模块只负责回答「文件在哪」。
    //
    // ⚠️ 本模块**只负责回答「记忆文件在哪」**：读 / 写 / 两道容量闸门一律走内核
    // （`symbio_core::memory::MemoryFile`）。此前这里自带一份 `read_memory` /
    // `write_memory` 与自己的字节闸门，与 work / session 两层各写一份口径——
    // 「超限是拒绝还是截断」「读不到算不算错误」一旦分叉，用户看到的行为就会随
    // 「这条记忆属于哪一层」而变化。收口后三层共用同一份实现，本模块不再持有闸门。

    /// 智能体记忆文件：`<Agent 目录>/AGENTS.md`
    ///
    /// Agent 不存在 → 明确报错（调用方 [`super::memory::store`] 据此构造「无作用域」门面）。
    pub fn memory_path(&self, agent_id: &str) -> Result<PathBuf, String> {
        let record = self
            .get(agent_id)
            .ok_or_else(|| format!("智能体 `{agent_id}` 不存在"))?;
        Ok(record.dir.join(AGENTS_FILE))
    }

    /// zip entry 名 → agent 目录内相对路径。    ///
    /// 支持两种打包布局：根目录直打包（`manifest.yaml`、`providers/...`）与
    /// 单顶层目录打包（`<agent_id>/manifest.yaml`、`<agent_id>/providers/...`）。
    /// 返回 `None` 表示跳过（目录项 / 顶层杂项）。
    fn strip_manifest_root(entry_name: &str, agent_id: &str) -> Option<String> {
        // zip 目录项（以 / 结尾）：不是文件，直接跳过
        if entry_name.ends_with('/') || entry_name.ends_with('\\') {
            return None;
        }
        let normalized = entry_name.replace('\\', "/");
        // 单顶层目录布局：`<agent_id>/...` → 剥掉 agent_id 前缀
        let prefix = format!("{agent_id}/");
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
        // `..` 段直接拒绝（不弹出）——防御性：合法 agent 目录不应包含
        if seg == ".." {
            continue;
        }
        p.push(seg);
    }
    p
}

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
#[path = "store.test.rs"]
mod tests;
