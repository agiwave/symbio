//! 单文件型集中实现——**一个条目 = 一份主文件**
//!
//! 磁盘布局与其它两型**完全一致**（`<homedir>/plugins/<category>/<id>/<manifest>`），
//! 区别只在**访问拓扑**：条目对外呈现为**叶子文件**，不暴露条目内部。
//! 因此「一个资源到底有没有子结构」由**挂载点选用哪一型**声明，
//! 而不是由机制去看目录里有什么（这正是当年适配器「有目录就当容器」那类猜测的
//! 反面）。
//!
//! 适用：内容就是**一份清单文件**的资源——Model Provider（`provider.json`）。
//! 有多份文件、需要浏览条目内部的资源用 `DirVdfs`。

use super::entry;
use async_trait::async_trait;
use std::future::Future;
use std::path::{Path, PathBuf};
use symbio_platform::kernel::vdfs::{
    VdfsAccess, VdfsActionResult, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsProvider,
    VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse, VDFS_ACTION_EXPORT,
};
use symbio_platform::sdk::vdfs::host::{
    vdfs_notify_change, vdfs_unwatch_changes, vdfs_watch_changes,
};
use tokio::sync::OwnedMutexGuard;

struct ImportGuard {
    root: PathBuf,
    target: PathBuf,
    old: PathBuf,
    new: PathBuf,
    old_moved: bool,
    published: bool,
}

impl ImportGuard {
    fn new(root: PathBuf, target: PathBuf) -> Self {
        Self {
            old: root.join("old"),
            new: root.join("new"),
            root,
            target,
            old_moved: false,
            published: false,
        }
    }
}

impl Drop for ImportGuard {
    fn drop(&mut self) {
        if self.old_moved && !self.published {
            let restored = match std::fs::symlink_metadata(&self.target) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if ordinary_components(&self.target) {
                        std::fs::rename(&self.old, &self.target).is_ok()
                    } else {
                        false
                    }
                }
                _ => false,
            };
            // 回滚失败时保留私有暂存中的唯一旧副本，不能用清理把它一起删掉。
            if !restored {
                return;
            }
        }
        remove_owned_tree(&self.root);
    }
}

fn ordinary_components(path: &Path) -> bool {
    let mut current = path.to_path_buf();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if entry::is_reparse_point(&meta) => return false,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
        let Some(parent) = current.parent() else {
            return true;
        };
        if parent.as_os_str().is_empty() || parent == current {
            return true;
        }
        current = parent.to_path_buf();
    }
}

fn remove_owned_tree(path: &Path) {
    if !ordinary_components(path) {
        return;
    }
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if entry::is_reparse_point(&meta) {
        return;
    }
    if meta.is_dir() {
        let Ok(rd) = std::fs::read_dir(path) else {
            return;
        };
        for child in rd.flatten() {
            remove_owned_tree(&child.path());
        }
        let _ = std::fs::remove_dir(path);
    } else if meta.is_file() {
        let _ = std::fs::remove_file(path);
    }
}

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
    // 注：早先这里有 `for_category(kind)`——按类别名反推全局落位
    //（`<homedir>/<kind>`）。homedir 下沉到 `home` 后 core 不再有全局根，
    // 调用方（model 迁移旧分类）改为自己推出作用域根再走 `at`。

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

    /// 调用方已持有同一 base 的 mutation guard，避免递归加锁。
    pub async fn entries_locked(&self) -> VdfsResult<Vec<entry::Entry>> {
        let ids = entry::list_entry_ids(&self.base).await?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(entry::read_entry_tolerant_locked(&self.base, &id, &self.manifest).await);
        }
        Ok(out)
    }

    /// 单个条目（存在性校验 + 原文 + 时间戳）
    pub async fn entry(&self, id: &str) -> VdfsResult<entry::Entry> {
        entry::read_entry(&self.base, id, &self.manifest).await
    }

    /// 调用方已持有同一 base 的 mutation guard，避免递归加锁。
    pub async fn entry_locked(&self, id: &str) -> VdfsResult<entry::Entry> {
        entry::read_entry_locked(&self.base, id, &self.manifest).await
    }

    /// 条目主文件原文
    pub async fn read_text(&self, id: &str) -> VdfsResult<String> {
        Ok(self.entry(id).await?.raw.unwrap_or_default())
    }

    /// 写主文件（原子落盘）+ 变更广播；返回是否为**新建**
    pub async fn write_text(&self, id: &str, text: &str) -> VdfsResult<bool> {
        let lock = entry::mutation_lock(&self.base);
        let _guard = lock.lock().await;
        let created = entry::write_entry_locked(&self.base, id, &self.manifest, text).await?;
        // 广播也留在锁内：并发 save 的通知顺序与落盘顺序一致，reload 不会倒序。
        self.announce(id, created);
        Ok(created)
    }

    /// 写 JSON 主文件（`to_string_pretty`）+ 变更广播
    ///
    /// **主文件不是 JSON 的资源不要用本方法**——把 Markdown 包成 JSON 值会写上
    /// 引号并把换行转义成字面 `\n`，文件被静默写坏而写入报成功（用
    /// `write_text`）。
    pub async fn write_json(&self, id: &str, value: &serde_json::Value) -> VdfsResult<bool> {
        let text = serde_json::to_string_pretty(value)
            .map_err(|e| VdfsError::internal(format!("序列化主文件失败：{e}")))?;
        self.write_text(id, &text).await
    }

    /// 读/合并/写盘与内存发布共用锁；提交任务不随调用方取消。
    pub async fn update_json<T, F, Fut, P, Pfut>(
        &self,
        id: &str,
        update: F,
        publish: P,
    ) -> VdfsResult<(bool, T)>
    where
        F: FnOnce(Option<String>) -> Fut + Send + 'static,
        Fut: Future<Output = VdfsResult<(serde_json::Value, T)>> + Send + 'static,
        P: FnOnce(T) -> Pfut + Send + 'static,
        Pfut: Future<Output = ()> + Send + 'static,
        T: Clone + Send + 'static,
    {
        let base = self.base.clone();
        let manifest = self.manifest.clone();
        let id = id.to_string();
        let task = tokio::spawn(async move {
            let _guard = entry::mutation_guard(&base).await;
            let previous = match entry::read_entry_locked(&base, &id, &manifest).await {
                Ok(entry) => Some(entry.raw.ok_or_else(|| {
                    VdfsError::internal(format!("条目「{id}」的主文件不可用，无法合并更新"))
                })?),
                Err(VdfsError::NotFound(_)) => None,
                Err(e) => return Err(e),
            };
            let (next, published) = update(previous).await?;
            let created = entry::write_entry_locked(
                &base,
                &id,
                &manifest,
                &serde_json::to_string_pretty(&next)
                    .map_err(|e| VdfsError::internal(format!("序列化主文件失败：{e}")))?,
            )
            .await?;
            publish(published.clone()).await;
            Ok((created, published))
        });
        task.await
            .map_err(|e| VdfsError::internal(format!("存储提交任务失败：{e}")))?
    }

    /// 删除条目目录，在释放锁前发布内存清理；提交任务不随调用方取消。
    pub async fn remove_with<F, Fut>(&self, id: &str, publish: F) -> VdfsResult<()>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let base = self.base.clone();
        let kind = self.kind.clone();
        let id = id.to_string();
        let task = tokio::spawn(async move {
            let _guard = entry::mutation_guard(&base).await;
            match entry::remove_entry_locked(&base, &id).await {
                Ok(()) => {}
                Err(VdfsError::NotFound(_)) => {
                    symbio_platform::sdk::plugin_warn!(
                        &kind,
                        "磁盘上已无条目 {id} 目录，仅清理内存"
                    );
                }
                Err(e) => return Err(e),
            }
            publish().await;
            vdfs_notify_change(&kind, &id);
            Ok(())
        });
        task.await
            .map_err(|e| VdfsError::internal(format!("存储删除任务失败：{e}")))?
    }

    /// 供同一 provider 的加载、默认指针等跨条目操作加入资源变更的同一串行边界。
    pub async fn mutation_guard(&self) -> OwnedMutexGuard<()> {
        entry::mutation_guard(&self.base).await
    }

    /// 删除条目目录 + 变更广播（磁盘上已无目录时告警而非报错——**幂等**）
    pub async fn remove(&self, id: &str) -> VdfsResult<()> {
        self.remove_with(id, || async {}).await
    }

    /// 条目是否存在
    pub fn exists(&self, id: &str) -> bool {
        entry::entry_exists(&self.base, id)
    }

    /// 整包导入（zip）：私有暂存完整后发布，覆盖旧条目
    pub async fn import_pack(&self, id: &str, bytes: &[u8]) -> VdfsResult<bool> {
        let lock = entry::mutation_lock(&self.base);
        let _guard = lock.lock().await;
        entry::validate_base(&self.base).await?;
        tokio::fs::create_dir_all(&self.base)
            .await
            .map_err(|e| VdfsError::internal(format!("创建类别根失败：{e}")))?;
        entry::validate_base(&self.base).await?;
        let target = self.entry_dir(id);
        let existed = match tokio::fs::symlink_metadata(&target).await {
            Ok(_) => {
                entry::validate_entry_tree(&self.base, id).await?;
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(VdfsError::internal(format!("检查旧条目失败：{e}"))),
        };

        let root = loop {
            let candidate = self.base.join(format!(
                ".symbio-import-{}-{}.tmp",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(VdfsError::internal(format!("创建导入暂存目录失败：{e}"))),
            }
        };
        let mut import_guard = ImportGuard::new(root, target.clone());
        super::pack::extract_pack(&import_guard.new, bytes).await?;
        entry::sync_tree(&import_guard.new).await?;
        entry::validate_base(&self.base).await?;
        if existed {
            entry::validate_entry_tree(&self.base, id).await?;
        } else if tokio::fs::symlink_metadata(&target).await.is_ok() {
            return Err(VdfsError::Conflict(format!("导入目标已存在：{id}")));
        }
        if existed {
            // 两步发布之间没有取消点，避免 tokio 的脱离式 rename 在 Drop 后才完成。
            std::fs::rename(&target, &import_guard.old)
                .map_err(|e| VdfsError::internal(format!("暂存旧条目失败：{e}")))?;
            import_guard.old_moved = true;
        }
        std::fs::rename(&import_guard.new, &target)
            .map_err(|e| VdfsError::internal(format!("发布导入条目失败：{e}")))?;
        import_guard.published = true;
        entry::sync_parent_best_effort(&self.base);
        // 发布即提交；暂存清理由 guard 尽力执行，不再引入发布后的取消点。
        self.announce(id, !existed);
        Ok(!existed)
    }

    /// 整包导出：条目目录打成 zip（顶层目录名 = id，可原样导回）
    pub async fn export_pack(&self, id: &str) -> VdfsResult<super::pack::VdfsPack> {
        let lock = entry::mutation_lock(&self.base);
        let _guard = lock.lock().await;
        let dir = entry::validate_entry_tree(&self.base, id).await?;
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
        // 信封没有操作枚举：「新建还是更新」不单独成字段——
        // 消费端回读即得当前状态，不需要为它保留一个分派键。
        vdfs_notify_change(&self.kind, id);
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
                Ok(VdfsResponse::list(
                    self.entries()
                        .await?
                        .iter()
                        .map(|e| self.node_of(e))
                        .collect::<Vec<VdfsNode>>(),
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
                Ok(VdfsResponse::Read(VdfsContent::text(text)))
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
                    name: None,
                    created,
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

            VdfsRequest::Mkdir => Err(VdfsError::NotImplemented),

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
                vdfs_watch_changes(&self.kind, path, sink).await?;
                Ok(VdfsResponse::Unit)
            }

            VdfsRequest::Unwatch => {
                vdfs_unwatch_changes(&self.kind, path).await?;
                Ok(VdfsResponse::Unit)
            }
        }
    }
}

#[cfg(test)]
#[path = "single_file.test.rs"]
mod tests;
