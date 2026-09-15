//! 目录型集中实现——**一个条目 = 一个可下钻的目录**
//!
//! 磁盘布局与单文件型一致（`<homedir>/plugins/<category>/<id>/<manifest>`），
//! 拓扑上的区别是**这一型把条目内部也放进地址空间**：
//!
//! ```text
//! .vdfs/skill            条目清单（每个条目一个目录节点）
//! .vdfs/skill/demo       条目内部（SKILL.md / scripts/… 原样可浏览）
//! .vdfs/skill/demo/SKILL.md
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
    VdfsAccess, VdfsActionResult, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNewType,
    VdfsNode, VdfsProvider, VdfsResult, VdfsWriteResponse, VDFS_ACTION_EXPORT, VDFS_CHANGE_CREATED,
    VDFS_CHANGE_DELETED, VDFS_CHANGE_UPDATED, VDFS_EXT_ZIP, VDFS_NEW_SOURCE_FILE,
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
    /// 按插件名建一个目录型存储：类别段 = 子目录名 = 广播频道键
    pub fn for_category(kind: impl Into<String>, manifest: impl Into<String>) -> Self {
        let kind = kind.into();
        let base = entry::category_dir(&kind);
        Self::at(base, kind, manifest)
    }

    /// 显式指定类别根（测试与非常规落位用）
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
        notify_change(&self.kind, id, VDFS_CHANGE_DELETED);
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
        let target = self.inner_path(id, rel);
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
        let path = self.inner_path(id, rel);
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
        let path = self.inner_path(id, rel);
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
        let path = self.inner_path(id, rel);
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
        notify_change(&self.kind, id, VDFS_CHANGE_UPDATED);
        Ok(())
    }

    fn announce(&self, id: &str, created: bool) {
        notify_change(
            &self.kind,
            id,
            if created {
                VDFS_CHANGE_CREATED
            } else {
                VDFS_CHANGE_UPDATED
            },
        );
    }
}

/// 目录型是一个**完整**的 provider：条目内部天然就是真实文件，
/// 无呈现差异的资源可以直接注册它（LLM 侧即可整树浏览）。
#[async_trait]
impl VdfsProvider for DirVdfs {
    fn label(&self) -> Option<&str> {
        Some(&self.label)
    }

    fn description(&self) -> Option<&str> {
        Some("目录型资源：每个条目一个目录，条目内部可下钻浏览。")
    }

    fn icon(&self) -> Option<&str> {
        Some(self.kind.as_str())
    }

    /// 条目内部有子结构 ⇒ 根可递归遍历
    fn root_access(&self) -> VdfsAccess {
        VdfsAccess::LIST_TRAVERSE
    }

    fn root_new_types(&self) -> Vec<VdfsNewType> {
        vec![VdfsNewType::new(VDFS_EXT_ZIP, format!("{}包", self.label))
            .with_description("导入整包（.zip）——整目录覆盖同名条目")
            .with_source(VDFS_NEW_SOURCE_FILE)]
    }

    async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        match entry::split_rel(path) {
            None => Ok(self
                .entries()
                .await?
                .iter()
                .map(|e| self.node_of(e))
                .collect()),
            Some((id, rel)) => {
                let id = self.id_of(id);
                // 条目存在性先校验：不存在的条目应报 NotFound，而不是空目录
                if !self.exists(&id) {
                    return Err(VdfsError::not_found(format!("未找到条目「{id}」")));
                }
                self.list_inner(&id, rel).await
            }
        }
    }

    async fn stat(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        match entry::split_rel(path) {
            // 自身根：名字留空——provider 不知道自己的挂载名，由使用方回填
            None => Ok(VdfsNode::dir("", self.label.clone(), VdfsAccess::LIST)),
            Some((id, "")) => Ok(self.node_of(&self.entry(&self.id_of(id)).await?)),
            Some((id, rel)) => {
                let id = self.id_of(id);
                let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
                entry::tree_node(&self.inner_path(&id, rel), &name).await
            }
        }
    }

    async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let (id, rel) = entry::split_rel(path)
            .ok_or_else(|| VdfsError::invalid("该路径是目录，不可读取内容"))?;
        let id = self.id_of(id);
        if rel.is_empty() {
            return Ok(VdfsContent::text(path, self.read_text(&id).await?));
        }
        self.read_inner(&id, rel).await
    }

    async fn write(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        // 二进制写入 = 整包导入（规范 §3.3：导入不占第二个操作）
        if content.binary {
            let (id, _) = entry::split_rel(path)
                .ok_or_else(|| VdfsError::invalid("整包只能导入到挂载根下的条目地址"))?;
            let name = self.pack_name_of(id);
            let bytes = super::pack::decode_b64(content.b64.as_deref().unwrap_or_default())
                .map_err(|e| VdfsError::invalid(e.0))?;
            let created = self.import_pack(&name, &bytes).await?;
            return Ok(VdfsWriteResponse {
                path: name,
                created,
                etag: None,
            });
        }
        let (id, rel) = entry::split_rel(path)
            .ok_or_else(|| VdfsError::invalid("目录型条目只能写到条目地址上"))?;
        let id = self.id_of(id);
        let created = self
            .write_inner(&id, rel, content.as_text().unwrap_or_default())
            .await?;
        Ok(VdfsWriteResponse {
            path: if rel.is_empty() {
                id
            } else {
                format!("{id}/{rel}")
            },
            created,
            etag: None,
        })
    }

    async fn delete(&self, _ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
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
            return self.remove(&id).await;
        }
        self.remove_inner(&id, rel).await
    }

    async fn mkdir(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let (id, rel) = entry::split_rel(path)
            .ok_or_else(|| VdfsError::invalid("只能在挂载根下新建条目目录"))?;
        let id = self.id_of(id);
        if rel.is_empty() {
            return self.mkdir_entry(&id).await;
        }
        let target = self.inner_path(&id, rel);
        tokio::fs::create_dir_all(&target)
            .await
            .map_err(|e| VdfsError::internal(format!("创建目录失败：{e}")))?;
        notify_change(&self.kind, &id, VDFS_CHANGE_UPDATED);
        Ok(())
    }

    async fn action(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        action: &str,
        _payload: Option<&serde_json::Value>,
    ) -> VdfsResult<VdfsActionResult> {
        if action != VDFS_ACTION_EXPORT {
            return Err(VdfsError::NotImplemented);
        }
        let (id, _) =
            entry::split_rel(path).ok_or_else(|| VdfsError::invalid("「导出」只对条目可用"))?;
        let id = self.id_of(id);
        let pack = self.export_pack(&id).await?;
        let data = serde_json::to_value(&pack)
            .map_err(|e| VdfsError::internal(format!("导出结果序列化失败: {e}")))?;
        Ok(VdfsActionResult {
            action: VDFS_ACTION_EXPORT.to_string(),
            ok: true,
            message: format!("已打包「{}」", pack.filename),
            data: Some(data),
        })
    }

    async fn watch(&self, _ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        watch_changes(&self.kind, path, sink).await
    }

    async fn unwatch(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        unwatch_changes(&self.kind, path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn store_in(base: &Path) -> DirVdfs {
        DirVdfs::at(base, "skill", "SKILL.md").with_label("技能")
    }

    #[tokio::test]
    async fn entry_is_a_drillable_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(&tmp.path().join("plugins/skill"));
        let ctx = VdfsContext::empty();

        s.write(&ctx, "demo.skill", &VdfsContent::text("", "# demo"))
            .await
            .unwrap();
        assert!(tmp.path().join("plugins/skill/demo/SKILL.md").exists());

        // 条目内部进地址空间：写一个附属文件，再从根下钻读回
        s.write(
            &ctx,
            "demo/scripts/run.sh",
            &VdfsContent::text("", "echo hi"),
        )
        .await
        .unwrap();
        assert_eq!(
            s.read(&ctx, "demo/scripts/run.sh").await.unwrap().as_text(),
            Some("echo hi")
        );
        let items = s.list(&ctx, "demo").await.unwrap();
        let names: Vec<&str> = items.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["scripts", "SKILL.md"], "目录在前、按名升序");

        // 根清单把条目呈现为**目录**（可下钻），不是叶子文件
        let root = s.list(&ctx, "").await.unwrap();
        assert_eq!(root.len(), 1);
        assert!(root[0].is_dir());
        assert_eq!(root[0].name, "demo");
        assert!(root[0].access.traverse);
    }

    #[tokio::test]
    async fn non_ascii_text_degrades_to_binary_channel() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        std::fs::create_dir_all(s.entry_dir("demo")).unwrap();
        std::fs::write(s.inner_path("demo", "img.bin"), [0u8, 159, 1, 2]).unwrap();
        let c = s.read(&VdfsContext::empty(), "demo/img.bin").await.unwrap();
        assert!(c.binary, "非 UTF-8 内容必须走二进制通道，而不是报错");
    }

    /// 导出 → 导入往返，且导入即整目录覆盖（附属文件不残留）
    #[tokio::test]
    async fn pack_roundtrip_replaces_whole_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let ctx = VdfsContext::empty();
        s.write(&ctx, "demo", &VdfsContent::text("", "# demo"))
            .await
            .unwrap();
        s.write(&ctx, "demo/extra.txt", &VdfsContent::text("", "x"))
            .await
            .unwrap();

        let r = s
            .action(&ctx, "demo", VDFS_ACTION_EXPORT, None)
            .await
            .unwrap();
        assert!(r.ok);
        let b64 = r.data.unwrap()["b64"].as_str().unwrap().to_string();
        let bytes = super::super::pack::decode_b64(&b64).unwrap();

        // 换一个只含主文件的包导回：附属文件必须消失
        s.write(&ctx, "other", &VdfsContent::text("", "# only"))
            .await
            .unwrap();
        let only = s.export_pack("other").await.unwrap();
        s.import_pack("demo", &super::super::pack::decode_b64(&only.b64).unwrap())
            .await
            .unwrap();
        assert!(
            !s.inner_path("demo", "extra.txt").exists(),
            "导入即替换：旧包里没覆盖到的文件不得残留"
        );
        assert_eq!(s.read_text("demo").await.unwrap(), "# only");
        let _ = bytes.len();
    }

    /// 带子结构的条目非 `recursive` 不得删除
    #[tokio::test]
    async fn delete_with_children_needs_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let ctx = VdfsContext::empty();
        s.write(&ctx, "demo", &VdfsContent::text("", "# demo"))
            .await
            .unwrap();
        s.write(&ctx, "demo/a.txt", &VdfsContent::text("", "x"))
            .await
            .unwrap();
        assert!(s.delete(&ctx, "demo", false).await.is_err());
        s.delete(&ctx, "demo", true).await.unwrap();
        assert!(!s.exists("demo"));
    }

    #[tokio::test]
    async fn mkdir_conflicts_on_existing_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let ctx = VdfsContext::empty();
        s.mkdir(&ctx, "demo").await.unwrap();
        assert!(matches!(
            s.mkdir(&ctx, "demo").await.unwrap_err(),
            VdfsError::Conflict(_)
        ));
        // 空条目（无主文件）读不到内容，但列得出来
        assert!(s.read_text("demo").await.is_err());
        assert_eq!(s.entries().await.unwrap().len(), 1);
    }

    /// 呈现扩展名不是地址的一部分：`demo` / `demo.skill` 同解
    #[tokio::test]
    async fn presentation_ext_is_not_part_of_the_address() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let ctx = VdfsContext::empty();
        s.write(&ctx, "demo.skill", &VdfsContent::text("", "# a"))
            .await
            .unwrap();
        assert_eq!(s.read(&ctx, "demo").await.unwrap().as_text(), Some("# a"));
        assert_eq!(s.stat(&ctx, "demo.skill").await.unwrap().name, "demo");
    }

    /// 广播频道按 `kind` 全局持有（同一 provider 会被每次 traverse 重新构造），
    /// 因此订阅类测试必须用**独占的 kind**，否则与同 binary 内其它测试互相串台。
    #[tokio::test]
    async fn watch_and_unwatch_are_paired() {
        let tmp = tempfile::tempdir().unwrap();
        let s = DirVdfs::at(tmp.path(), "watch-skill", "SKILL.md");
        let ctx = VdfsContext::empty();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = {
            let seen = seen.clone();
            std::sync::Arc::new(move |c: crate::symbio_core::vdfs_provider::VdfsChange| {
                seen.lock().unwrap().push(c.path);
            })
        };
        s.watch(&ctx, "", sink).await.unwrap();
        s.write_text("demo", "# x").await.unwrap();
        // 广播是异步投递：给转发任务一点时间
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(seen.lock().unwrap().as_slice(), ["demo"]);
        s.unwatch(&ctx, "").await.unwrap();
        s.write_text("demo", "# y").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(seen.lock().unwrap().len(), 1, "unwatch 后不得再收到事件");
    }
}
