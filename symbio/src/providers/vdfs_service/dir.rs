//! 目录型集中实现——**一个条目 = 一个可下钻的目录**
//!
//! 磁盘布局与单文件型一致（`<homedir>/plugins/<category>/<id>/<manifest>`），
//! 拓扑上的区别是**这一型把条目内部也放进地址空间**：
//!
//! ```text
//! <根>/skill            条目清单（每个条目一个目录节点）
//! <根>/skill/demo       条目内部（SKILL.md / scripts/… 原样可浏览）
//! <根>/skill/demo/SKILL.md
//! ```
//!
//! 条目内容仍由**主文件**承载（`read(<id>)` = 读 `<id>/<manifest>`）——主文件名
//! 是构造时**声明**的，不是机制去看目录里有什么猜出来的。
//!
//! 适用：一个资源就是一包文件的场景——Skill（`SKILL.md` + `scripts/`）、
//! MCP Server（`server.json` + 附属文件）。

use super::entry;
use crate::symbio_core::vdfs::host::{notify_change, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    has_parent_segment, VdfsAccess, VdfsActionResult, VdfsContent, VdfsContext, VdfsError,
    VdfsNode, VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse,
    VDFS_ACTION_EXPORT,
};
use async_trait::async_trait;
use std::path::PathBuf;

/// 列目录上限（与物理层同一口径，避免超大目录打爆响应）
const MAX_LIST_ENTRIES: usize = 2000;

/// 目录型条目存储（同时是一个可直接注册的 `VdfsProvider`）
#[derive(Debug, Clone)]
pub struct DirVdfs {
    /// 变更广播频道键 = vdfs 子目录名（约定 = 插件名）
    kind: String,
    /// 类别根目录
    base: PathBuf,
    /// 条目主文件名（`SKILL.md` / `server.json`）
    manifest: String,
    /// 挂载根的展示标题
    label: String,
}

impl DirVdfs {
    /// 显式指定类别根
    ///
    /// 根**必须由调用方给**（生产上就是插件自己的目录，来自父插件经 `PLUGIN_DIR`
    /// 传下的 `PluginDir`）。早先这里有 `for_category(kind)` 按插件名反推落位，
    /// 那是让插件猜自己被放在哪，已去掉。
    pub fn at(
        base: impl Into<PathBuf>,
        kind: impl Into<String>,
        manifest: impl Into<String>,
    ) -> Self {
        let kind = kind.into();
        Self {
            label: kind.clone(),
            kind,
            base: base.into(),
            manifest: manifest.into(),
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 条目目录（`<base>/<id>`）
    pub fn entry_dir(&self, id: &str) -> PathBuf {
        entry::entry_dir(&self.base, id)
    }

    /// 条目内某个文件的绝对路径
    pub fn inner_path(&self, id: &str, rel: &str) -> PathBuf {
        self.entry_dir(id).join(rel)
    }

    /// 条目内相对路径 → 绝对路径，**并校验它不逃出条目目录**。
    ///
    /// [`inner_path`](Self::inner_path) 是裸拼接口径：条目 id 由
    /// [`safe_segment`](entry::safe_segment) 保证安全，但 `rel` 此前**未经任何
    /// 校验**。虚拟地址只以 `/` 分段，因此合法的 `rel` 必须既不含 `..` 段
    /// （`\` 也是分隔符，见 [`has_parent_segment`]），也不含 `\`——否则
    /// Windows 会把 `demo/..\..\escaped.txt` 解析到条目目录之外，而
    /// [`remove_inner`](Self::remove_inner) 用的是 `remove_dir_all`。
    ///
    /// 这是**纵深防御**：访问层的 `plugins::vdfs::fs::normalize_addr` 已拦一道，
    /// 但 provider 可能经其它访问路径被复用（解包、直接构造），不能假设上游
    /// 一定校验过。
    fn checked_inner(&self, id: &str, rel: &str) -> VdfsResult<PathBuf> {
        if has_parent_segment(rel) || rel.contains('\\') {
            return Err(VdfsError::invalid(format!(
                "条目内路径不允许向上穿越或含反斜杠：{rel}"
            )));
        }
        Ok(self.inner_path(id, rel))
    }

    // ==================== 条目级原语 ====================

    /// 条目清单（每个条目取其主文件；读失败降级不阻断列表）
    pub async fn entries(&self) -> VdfsResult<Vec<entry::Entry>> {
        let ids = entry::list_entry_ids(&self.base).await?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(entry::read_entry_tolerant(&self.base, &id, &self.manifest).await);
        }
        Ok(out)
    }

    /// 单个条目（主文件必须存在，否则 `NotFound`）
    pub async fn entry(&self, id: &str) -> VdfsResult<entry::Entry> {
        entry::read_entry(&self.base, id, &self.manifest).await
    }

    /// 条目主文件原文
    pub async fn read_text(&self, id: &str) -> VdfsResult<String> {
        Ok(self.entry(id).await?.raw.unwrap_or_default())
    }

    /// 写条目主文件（原子落盘）+ 变更广播；返回是否为**新建**
    pub async fn write_text(&self, id: &str, text: &str) -> VdfsResult<bool> {
        let created = entry::write_entry(&self.base, id, &self.manifest, text).await?;
        self.announce(id, created);
        Ok(created)
    }

    /// 写 JSON 主文件（`to_string_pretty`）+ 变更广播
    ///
    /// 主文件不是 JSON 的资源（如 `SKILL.md` 是 Markdown）必须用
    /// [`write_text`](Self::write_text)：把纯文本包成 JSON 值会写上引号、
    /// 并把换行转义成字面 `\n`，文件被静默写坏而写入报成功。
    pub async fn write_json(&self, id: &str, value: &serde_json::Value) -> VdfsResult<bool> {
        let text = serde_json::to_string_pretty(value)
            .map_err(|e| VdfsError::internal(format!("序列化主文件失败：{e}")))?;
        self.write_text(id, &text).await
    }

    /// 删除条目目录 + 变更广播（磁盘上已无目录时告警而非报错——**幂等**）
    pub async fn remove(&self, id: &str) -> VdfsResult<()> {
        match entry::remove_entry(&self.base, id).await {
            Ok(()) => {}
            Err(VdfsError::NotFound(_)) => {
                crate::plugin_warn!(&self.kind, "磁盘上已无条目 {id} 目录，仅清理内存");
            }
            Err(e) => return Err(e),
        }
        notify_change(&self.kind, id);
        Ok(())
    }

    pub fn exists(&self, id: &str) -> bool {
        entry::entry_exists(&self.base, id)
    }

    /// 整包导入（zip）：解到条目目录，**整目录覆盖**
    pub async fn import_pack(&self, id: &str, bytes: &[u8]) -> VdfsResult<bool> {
        let existed = self.exists(id);
        super::pack::extract_pack(&self.entry_dir(id), bytes).await?;
        self.announce(id, !existed);
        Ok(!existed)
    }

    /// 整包导出：条目目录打成 zip（顶层目录名 = id，可原样导回）
    pub async fn export_pack(&self, id: &str) -> VdfsResult<super::pack::VdfsPack> {
        let dir = self.entry_dir(id);
        if !dir.is_dir() {
            return Err(VdfsError::NotFound(format!("未找到条目「{id}」")));
        }
        let bytes = super::pack::zip_dir(&dir, id).map_err(|e| VdfsError::Internal(e.0))?;
        Ok(super::pack::VdfsPack::new(id, &bytes))
    }

    /// 路径 → 条目 id（剥呈现扩展名）
    pub fn id_of(&self, path: &str) -> String {
        entry::id_of(path, &self.kind)
    }

    /// 新建地址 → 整包导入的建议名（再剥 `.zip`）
    pub fn pack_name_of(&self, path: &str) -> String {
        entry::pack_name_of(path, &self.kind)
    }

    /// 条目节点（缺省呈现：目录形态、可下钻，`ext` 由主文件名推导）
    pub fn node_of(&self, e: &entry::Entry) -> VdfsNode {
        let mut n = VdfsNode::dir(e.id.clone(), e.id.clone(), VdfsAccess::dir(true, true));
        n.kind = self.kind.clone();
        n.updated_at = e.updated_at;
        n
    }

    // ==================== 条目内部（下钻）原语 ====================

    /// 列条目内部某个目录（`rel` 为空 = 条目根）；目录在前、各自按名升序
    pub async fn list_inner(&self, id: &str, rel: &str) -> VdfsResult<Vec<VdfsNode>> {
        let target = self.checked_inner(id, rel)?;
        let mut rd = tokio::fs::read_dir(&target)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法读取目录 {target:?}：{e}")))?;
        let mut nodes: Vec<VdfsNode> = Vec::new();
        while let Some(item) = rd
            .next_entry()
            .await
            .map_err(|e| VdfsError::internal(format!("遍历目录失败：{e}")))?
        {
            let Ok(name) = item.file_name().into_string() else {
                continue; // 非 UTF-8 名称无法在地址空间里表达
            };
            nodes.push(entry::tree_node(&item.path(), &name).await?);
            if nodes.len() >= MAX_LIST_ENTRIES {
                break;
            }
        }
        nodes.sort_by(|a, b| {
            b.is_dir()
                .cmp(&a.is_dir())
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(nodes)
    }

    /// 读条目内部某个文件
    pub async fn read_inner(&self, id: &str, rel: &str) -> VdfsResult<VdfsContent> {
        let path = self.checked_inner(id, rel)?;
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法读取 {path:?}：{e}")))?;
        if meta.is_dir() {
            return Err(VdfsError::Forbidden(format!(
                "目录不可读：{path:?}（请列出它的子项）"
            )));
        }
        match tokio::fs::read_to_string(&path).await {
            // `VdfsContent::text` 已按正文长度填好 size
            Ok(text) => Ok(VdfsContent::text(rel, text)),
            Err(_) => {
                let bytes = tokio::fs::read(&path)
                    .await
                    .map_err(|e| VdfsError::internal(format!("读取文件失败：{e}")))?;
                Ok(VdfsContent::binary(
                    rel,
                    super::pack::encode_b64(&bytes),
                    bytes.len() as u64,
                ))
            }
        }
    }

    /// 写条目内部某个文件（父目录自动创建）+ 变更广播
    pub async fn write_inner(&self, id: &str, rel: &str, text: &str) -> VdfsResult<bool> {
        if rel.is_empty() {
            return self.write_text(id, text).await;
        }
        let path = self.checked_inner(id, rel)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| VdfsError::internal(format!("创建目录失败：{e}")))?;
        }
        let created = !path.exists();
        tokio::fs::write(&path, text)
            .await
            .map_err(|e| VdfsError::internal(format!("写入文件失败：{e}")))?;
        self.announce(id, created);
        Ok(created)
    }

    /// 建条目（空目录）；已存在报冲突
    pub async fn mkdir_entry(&self, id: &str) -> VdfsResult<()> {
        if self.exists(id) {
            return Err(VdfsError::Conflict(format!("条目已存在：{id}")));
        }
        tokio::fs::create_dir_all(self.entry_dir(id))
            .await
            .map_err(|e| VdfsError::internal(format!("创建条目失败：{e}")))?;
        self.announce(id, true);
        Ok(())
    }

    /// 删除条目内部某个文件 / 子目录
    pub async fn remove_inner(&self, id: &str, rel: &str) -> VdfsResult<()> {
        if rel.is_empty() {
            return self.remove(id).await;
        }
        let path = self.checked_inner(id, rel)?;
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法访问 {path:?}：{e}")))?;
        if meta.is_dir() {
            tokio::fs::remove_dir_all(&path)
                .await
                .map_err(|e| VdfsError::internal(format!("删除目录失败：{e}")))?;
        } else {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| VdfsError::internal(format!("删除文件失败：{e}")))?;
        }
        notify_change(&self.kind, id);
        Ok(())
    }

    fn announce(&self, id: &str, _created: bool) {
        // 信封没有操作枚举（S27）：「新建还是更新」不再单独成字段——
        // 消费端回读即得当前状态，不需要为它保留一个分派键。
        notify_change(&self.kind, id);
    }
}

/// 目录型是一个**完整**的 provider：条目内部天然就是真实文件，
/// 无呈现差异的资源可以直接注册它（LLM 侧即可整树浏览）。
///
/// 自述（标题 / 图标 / 根访问位 / 根可新建类型）已并入 PluginMeta，
/// 由持有本 store 的插件在 `Plugin::meta()` 中提供。
#[async_trait]
impl VdfsProvider for DirVdfs {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => match entry::split_rel(path) {
                None => Ok(VdfsResponse::List(
                    self.entries()
                        .await?
                        .iter()
                        .map(|e| self.node_of(e))
                        .collect(),
                )),
                Some((id, rel)) => {
                    let id = self.id_of(id);
                    // 条目存在性先校验：不存在的条目应报 NotFound，而不是空目录
                    if !self.exists(&id) {
                        return Err(VdfsError::not_found(format!("未找到条目「{id}」")));
                    }
                    Ok(VdfsResponse::List(self.list_inner(&id, rel).await?))
                }
            },

            VdfsRequest::Stat => match entry::split_rel(path) {
                // 自身根：名字留空——provider 不知道自己的挂载名，由使用方回填
                None => Ok(VdfsResponse::Stat(VdfsNode::dir(
                    "",
                    self.label.clone(),
                    VdfsAccess::LIST,
                ))),
                Some((id, "")) => Ok(VdfsResponse::Stat(
                    self.node_of(&self.entry(&self.id_of(id)).await?),
                )),
                Some((id, rel)) => {
                    let id = self.id_of(id);
                    let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
                    let target = self.checked_inner(&id, rel)?;
                    Ok(VdfsResponse::Stat(entry::tree_node(&target, &name).await?))
                }
            },

            VdfsRequest::Read => {
                let (id, rel) = entry::split_rel(path)
                    .ok_or_else(|| VdfsError::invalid("该路径是目录，不可读取内容"))?;
                let id = self.id_of(id);
                let content = if rel.is_empty() {
                    VdfsContent::text(path, self.read_text(&id).await?)
                } else {
                    self.read_inner(&id, rel).await?
                };
                Ok(VdfsResponse::Read(content))
            }

            VdfsRequest::Write { content } => {
                // 二进制写入 = 整包导入（规范 §3.3：导入不占第二个操作）
                if content.binary {
                    let (id, _) = entry::split_rel(path)
                        .ok_or_else(|| VdfsError::invalid("整包只能导入到挂载根下的条目地址"))?;
                    let name = self.pack_name_of(id);
                    let bytes = super::pack::decode_b64(content.b64.as_deref().unwrap_or_default())
                        .map_err(|e| VdfsError::invalid(e.0))?;
                    let created = self.import_pack(&name, &bytes).await?;
                    return Ok(VdfsResponse::Write(VdfsWriteResponse {
                        path: name,
                        created,
                        etag: None,
                    }));
                }
                let (id, rel) = entry::split_rel(path)
                    .ok_or_else(|| VdfsError::invalid("目录型条目只能写到条目地址上"))?;
                let id = self.id_of(id);
                let created = self
                    .write_inner(&id, rel, content.as_text().unwrap_or_default())
                    .await?;
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    path: if rel.is_empty() {
                        id
                    } else {
                        format!("{id}/{rel}")
                    },
                    created,
                    etag: None,
                }))
            }

            VdfsRequest::Delete { recursive } => {
                let (id, rel) = entry::split_rel(path)
                    .ok_or_else(|| VdfsError::Forbidden("不可删除挂载根".to_string()))?;
                let id = self.id_of(id);
                if rel.is_empty() {
                    // 存在性校验：删不存在的条目报 NotFound，而不是静默成功
                    self.entry(&id).await?;
                    if !recursive {
                        let inner = self.list_inner(&id, "").await?;
                        if inner.iter().any(|n| n.name != self.manifest) {
                            return Err(VdfsError::invalid(format!(
                                "条目「{id}」内部还有 {count} 个文件，需要 recursive=true",
                                count = inner.len()
                            )));
                        }
                    }
                    self.remove(&id).await?;
                    return Ok(VdfsResponse::Unit);
                }
                self.remove_inner(&id, rel).await?;
                Ok(VdfsResponse::Unit)
            }

            VdfsRequest::Mkdir => {
                let (id, rel) = entry::split_rel(path)
                    .ok_or_else(|| VdfsError::invalid("只能在挂载根下新建条目目录"))?;
                let id = self.id_of(id);
                if rel.is_empty() {
                    return self.mkdir_entry(&id).await.map(|_| VdfsResponse::Unit);
                }
                let target = self.checked_inner(&id, rel)?;
                tokio::fs::create_dir_all(&target)
                    .await
                    .map_err(|e| VdfsError::internal(format!("创建目录失败：{e}")))?;
                notify_change(&self.kind, &id);
                Ok(VdfsResponse::Unit)
            }

            VdfsRequest::Action { action, payload: _ } => {
                if action != VDFS_ACTION_EXPORT {
                    return Err(VdfsError::NotImplemented);
                }
                let (id, _) = entry::split_rel(path)
                    .ok_or_else(|| VdfsError::invalid("「导出」只对条目可用"))?;
                let id = self.id_of(id);
                let pack = self.export_pack(&id).await?;
                let data = serde_json::to_value(&pack)
                    .map_err(|e| VdfsError::internal(format!("导出结果序列化失败: {e}")))?;
                Ok(VdfsResponse::Action(VdfsActionResult {
                    action: VDFS_ACTION_EXPORT.to_string(),
                    ok: true,
                    message: format!("已打包「{}」", pack.filename),
                    data: Some(data),
                }))
            }

            VdfsRequest::Watch { sink } => {
                watch_changes(&self.kind, path, sink).await?;
                Ok(VdfsResponse::Unit)
            }

            VdfsRequest::Unwatch => {
                unwatch_changes(&self.kind, path).await?;
                Ok(VdfsResponse::Unit)
            }
        }
    }
}

#[cfg(test)]
#[path = "dir.test.rs"]
mod tests;
