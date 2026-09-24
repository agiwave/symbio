//! 单文件型集中实现——**一个条目 = 一份主文件**
//!
//! 磁盘布局与其它两型**完全一致**（`<homedir>/plugins/<category>/<id>/<manifest>`），
//! 区别只在**访问拓扑**：条目对外呈现为**叶子文件**，不暴露条目内部。
//! 因此「一个资源到底有没有子结构」由**挂载点选用哪一型**声明，
//! 而不是由机制去看目录里有什么（这正是当年适配器「有目录就当容器」那类猜测的
//! 反面）。
//!
//! 适用：内容就是**一份清单文件**的资源——Model Provider（`provider.json`）。
//! 有多份文件、需要浏览条目内部的资源用 [`DirVdfs`](super::dir::DirVdfs)。

use super::entry;
use crate::symbio_core::vdfs::host::{notify_change, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsActionResult, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsProvider,
    VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse, VDFS_ACTION_EXPORT,
};
use async_trait::async_trait;
use std::path::PathBuf;

/// 单文件型条目存储（同时是一个可直接注册的 `VdfsProvider`）
#[derive(Debug, Clone)]
pub struct SingleFileVdfs {
    /// 变更广播频道键 = vdfs 子目录名（约定 = 插件名，见规范 §2.4）
    kind: String,
    /// 类别根目录（`<homedir>/plugins/<category>`）
    base: PathBuf,
    /// 条目主文件名（`provider.json`）
    manifest: String,
    /// 挂载根的展示标题
    label: String,
}

impl SingleFileVdfs {
    /// 按类别名建一个单文件型存储：类别段 = 子目录名 = 广播频道键
    ///
    /// ⚠️ **仅迁移 / 兼容旧落位使用**。插件的正常存储根是它自己的目录，应由调用方
    /// 经 [`at`](Self::at) 显式传入；按名字反推落位等于让插件猜自己被放在哪。
    /// 当前唯一使用者是 model 插件迁移旧分类 `ai` 的那段代码。
    pub fn for_category(kind: impl Into<String>, manifest: impl Into<String>) -> Self {
        let kind = kind.into();
        let base = entry::category_dir(&kind);
        Self::at(base, kind, manifest)
    }

    /// 显式指定类别根（生产路径：根 = 调用方的插件目录）
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

    /// 挂载根的展示标题（缺省 = kind）
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 条目目录（`<base>/<id>`——布局与目录型一致，只是不外露内部）
    pub fn entry_dir(&self, id: &str) -> PathBuf {
        entry::entry_dir(&self.base, id)
    }

    // ==================== 类型化原语（插件侧入口） ====================

    /// 条目清单（一次目录枚举 + 逐条读主文件）
    ///
    /// 单个条目读失败**不让整次列表失败**（降级为 `raw = None`）。
    pub async fn entries(&self) -> VdfsResult<Vec<entry::Entry>> {
        let ids = entry::list_entry_ids(&self.base).await?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(entry::read_entry_tolerant(&self.base, &id, &self.manifest).await);
        }
        Ok(out)
    }

    /// 单个条目（存在性校验 + 原文 + 时间戳）
    pub async fn entry(&self, id: &str) -> VdfsResult<entry::Entry> {
        entry::read_entry(&self.base, id, &self.manifest).await
    }

    /// 条目主文件原文
    pub async fn read_text(&self, id: &str) -> VdfsResult<String> {
        Ok(self.entry(id).await?.raw.unwrap_or_default())
    }

    /// 写主文件（原子落盘）+ 变更广播；返回是否为**新建**
    pub async fn write_text(&self, id: &str, text: &str) -> VdfsResult<bool> {
        let created = entry::write_entry(&self.base, id, &self.manifest, text).await?;
        self.announce(id, created);
        Ok(created)
    }

    /// 写 JSON 主文件（`to_string_pretty`）+ 变更广播
    ///
    /// **主文件不是 JSON 的资源不要用本方法**——把 Markdown 包成 JSON 值会写上
    /// 引号并把换行转义成字面 `\n`，文件被静默写坏而写入报成功（用
    /// [`write_text`](Self::write_text)）。
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

    /// 条目是否存在
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

    /// 条目节点（缺省呈现：名字 = id、`ext` 由主文件名推导）
    pub fn node_of(&self, e: &entry::Entry) -> VdfsNode {
        let mut n = e.node(&self.manifest);
        n.kind = self.kind.clone();
        n
    }

    fn announce(&self, id: &str, _created: bool) {
        // 信封没有操作枚举（S27）：「新建还是更新」不单独成字段——
        // 消费端回读即得当前状态，不需要为它保留一个分派键。
        notify_change(&self.kind, id);
    }
}

/// 单文件型是一个**完整**的 provider：没有呈现差异的资源可以直接注册它。
/// 自述（标题 / 图标 / 根访问位 / 根可新建类型）已并入 PluginMeta，由持有方提供。
#[async_trait]
impl VdfsProvider for SingleFileVdfs {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                if !path.is_empty() {
                    return Err(VdfsError::not_found(format!(
                        "单文件型条目没有子项：{path}"
                    )));
                }
                Ok(VdfsResponse::List(
                    self.entries()
                        .await?
                        .iter()
                        .map(|e| self.node_of(e))
                        .collect(),
                ))
            }

            VdfsRequest::Stat => {
                if path.is_empty() {
                    // 自身根：名字留空——provider 不知道自己的挂载名，由使用方回填
                    return Ok(VdfsResponse::Stat(VdfsNode::dir(
                        "",
                        self.label.clone(),
                        VdfsAccess::LIST,
                    )));
                }
                Ok(VdfsResponse::Stat(
                    self.node_of(&self.entry(&self.id_of(path)).await?),
                ))
            }

            VdfsRequest::Read => {
                if path.is_empty() {
                    return Err(VdfsError::invalid("该路径是目录，不可读取内容"));
                }
                let id = self.id_of(path);
                let text = self.read_text(&id).await?;
                Ok(VdfsResponse::Read(VdfsContent::text(path, text)))
            }

            VdfsRequest::Write { content } => {
                if path.is_empty() {
                    return Err(VdfsError::invalid("单文件型条目只能写到条目地址上"));
                }
                let id = self.id_of(path);
                let created = if content.binary {
                    let bytes = super::pack::decode_b64(content.b64.as_deref().unwrap_or_default())
                        .map_err(|e| VdfsError::invalid(e.0))?;
                    self.import_pack(&self.pack_name_of(path), &bytes).await?
                } else {
                    self.write_text(&id, content.as_text().unwrap_or_default())
                        .await?
                };
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    path: id,
                    created,
                    etag: None,
                }))
            }

            VdfsRequest::Delete { recursive: _ } => {
                if path.is_empty() {
                    return Err(VdfsError::Forbidden("不可删除挂载根".to_string()));
                }
                let id = self.id_of(path);
                // 存在性校验：删不存在的条目应报 NotFound，而不是静默成功
                self.entry(&id).await?;
                self.remove(&id).await?;
                Ok(VdfsResponse::Unit)
            }

            VdfsRequest::Mkdir | VdfsRequest::Move { .. } => Err(VdfsError::NotImplemented),

            VdfsRequest::Action { action, payload: _ } => {
                if action != VDFS_ACTION_EXPORT {
                    return Err(VdfsError::NotImplemented);
                }
                if path.is_empty() {
                    return Err(VdfsError::invalid("「导出」只对条目可用"));
                }
                let id = self.id_of(path);
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
mod tests {
    use super::*;
    use crate::symbio_core::vdfs_provider::VdfsContent;
    use std::path::Path;

    fn store_in(base: &Path) -> SingleFileVdfs {
        SingleFileVdfs::at(base, "model", "provider.json").with_label("模型")
    }

    #[tokio::test]
    async fn writes_then_reads_the_main_file_only() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());

        assert!(s.entries().await.unwrap().is_empty(), "空类别是合法状态");
        assert!(s.write_text("p1", "{\"id\":\"p1\"}").await.unwrap());
        assert_eq!(s.read_text("p1").await.unwrap(), "{\"id\":\"p1\"}");

        // 条目内部不进地址空间：主文件之外的文件列不出来
        std::fs::create_dir_all(s.entry_dir("p1").join("secret")).unwrap();
        std::fs::write(s.entry_dir("p1").join("secret/note.txt"), b"x").unwrap();
        let err = s
            .dispatch(
                &VdfsContext::empty(),
                "p1",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, VdfsError::NotFound(_)),
            "单文件型不得暴露条目内部：{err:?}"
        );
        assert_eq!(s.entries().await.unwrap().len(), 1);
    }

    /// 磁盘布局不变：`<base>/<category>/<id>/<manifest>`
    #[tokio::test]
    async fn keeps_the_existing_disk_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(&tmp.path().join("model"));
        s.write_text("p1", "{}").await.unwrap();
        assert!(tmp.path().join("model/p1/provider.json").exists());
    }

    #[tokio::test]
    async fn default_provider_ops_work_without_any_differential() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let ctx = VdfsContext::empty();

        let _ = s
            .dispatch(
                &ctx,
                "p1.model",
                VdfsRequest::Write {
                    content: VdfsContent::text("", "{\"id\":\"p1\"}"),
                },
            )
            .await
            .unwrap();
        // 呈现扩展名不是地址的一部分
        assert_eq!(
            s.dispatch(&ctx, "p1", VdfsRequest::Read)
                .await
                .unwrap()
                .into_read()
                .unwrap()
                .as_text(),
            Some("{\"id\":\"p1\"}")
        );
        let node = s
            .dispatch(&ctx, "p1.model", VdfsRequest::Stat)
            .await
            .unwrap()
            .into_stat()
            .unwrap();
        assert_eq!(node.name, "p1");
        assert_eq!(node.ext.as_deref(), Some("json"));
        assert_eq!(node.access, VdfsAccess::READ_WRITE);

        let listed = s
            .dispatch(
                &ctx,
                "",
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
            .unwrap()
            .into_list()
            .unwrap();
        assert_eq!(
            listed.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
            vec!["p1"]
        );

        assert!(matches!(
            s.dispatch(&ctx, "nope", VdfsRequest::Delete { recursive: false })
                .await
                .unwrap_err(),
            VdfsError::NotFound(_)
        ));
        let _ = s
            .dispatch(&ctx, "p1", VdfsRequest::Delete { recursive: false })
            .await
            .unwrap();
        assert!(s.entries().await.unwrap().is_empty());
    }

    /// 删不存在的条目：落盘侧幂等告警（内存同步仍要跑到）
    #[tokio::test]
    async fn remove_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        s.remove("ghost").await.unwrap();
    }

    /// 导出 → 导入原样往返（导出的包能直接导回）
    #[tokio::test]
    async fn pack_roundtrips_through_the_binary_channel() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        s.write_text("p1", "{\"id\":\"p1\"}").await.unwrap();
        let pack = s.export_pack("p1").await.unwrap();
        let bytes = super::super::pack::decode_b64(&pack.b64).unwrap();

        s.write_text("p1", "stale").await.unwrap();
        assert!(
            !s.import_pack("p1", &bytes).await.unwrap(),
            "同名导入是覆盖，不是新建"
        );
        assert_eq!(s.read_text("p1").await.unwrap(), "{\"id\":\"p1\"}");
    }

    /// 未声明的操作保持 `NotImplemented`（机制据此隐藏入口）
    #[tokio::test]
    async fn unsupported_ops_stay_not_implemented() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store_in(tmp.path());
        let ctx = VdfsContext::empty();
        assert!(s
            .dispatch(&ctx, "d", VdfsRequest::Mkdir)
            .await
            .unwrap_err()
            .is_not_implemented());
        assert!(s
            .dispatch(
                &ctx,
                "a",
                VdfsRequest::Move {
                    to: "b".to_string()
                }
            )
            .await
            .unwrap_err()
            .is_not_implemented());
        assert!(s
            .dispatch(
                &ctx,
                "p1",
                VdfsRequest::Action {
                    action: "test".to_string(),
                    payload: None
                }
            )
            .await
            .unwrap_err()
            .is_not_implemented());
    }
}
