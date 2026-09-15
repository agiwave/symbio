//! 内存型集中实现——**一个条目 = 内存里一条记录**，不落盘
//!
//! 存在的理由：**清单的真相源不止磁盘一种**。规范 §13.4 早就点明了这条差异——
//! 「model 的列表来自内存」。旧写法是每个插件自己拿 `RwLock<HashMap<..>>` 再手写
//! 一遍列 / 读 / 写 / 删 / 广播；本型把这份 plumbing 收进来，语义与磁盘两型
//! **完全同构**（同一套 `VdfsProvider` 操作、同一条 `notify_change` 广播频道），
//! 因此消费者分不清也不必分清条目住在哪儿。
//!
//! 典型用法：
//!
//! - **内存清单**：运行期注册表直接作为挂载点内容（无 IO、无漂移）；
//! - **磁盘镜像**：启动时 [`DirVdfs`](super::dir::DirVdfs) /
//!   [`SingleFileVdfs`](super::single_file::SingleFileVdfs) 灌入，写盘成功后回灌
//!   ——列表读走内存，落盘走另一型。

use crate::symbio_core::vdfs::host::{notify_change, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsProvider,
    VdfsResult, VdfsWriteResponse, VFDS_CHANGE_DELETED,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 一条内存条目
#[derive(Debug, Clone)]
struct MemEntry {
    text: String,
    updated_at: i64,
}

/// 内存型条目存储（同时是一个可直接注册的 `VdfsProvider`）
///
/// 克隆是**浅克隆**（同一张表）——与磁盘两型「多处构造、指向同一目录」的
/// 语义对齐：provider 会被每次 `traverse` 重新构造一份，共享同一份状态才能
/// 让订阅与投递天然配对。
#[derive(Debug, Clone)]
pub struct MemoryVdfs {
    kind: String,
    label: String,
    entries: Arc<RwLock<BTreeMap<String, MemEntry>>>,
}

impl MemoryVdfs {
    /// 建一个内存型挂载底座；`kind` = vdfs 子目录名 = 广播频道键
    pub fn new(kind: impl Into<String>) -> Self {
        let kind = kind.into();
        let label = kind.clone();
        Self {
            kind,
            label,
            entries: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// 条目 id 清单（按名升序——`BTreeMap` 天然有序）
    pub fn ids(&self) -> Vec<String> {
        self.entries.read().unwrap().keys().cloned().collect()
    }

    /// 条目原文（不存在 → `None`）
    pub fn get(&self, id: &str) -> Option<String> {
        self.entries.read().unwrap().get(id).map(|e| e.text.clone())
    }

    /// 条目写入时间（Unix 毫秒；不存在 → `None`）
    pub fn updated_at(&self, id: &str) -> Option<i64> {
        self.entries.read().unwrap().get(id).map(|e| e.updated_at)
    }

    /// 写条目（新建或覆盖）+ 变更广播；返回是否为**新建**
    pub fn set(&self, id: &str, text: impl Into<String>) -> bool {
        let created = {
            let mut table = self.entries.write().unwrap();
            created_of(
                &mut table,
                id,
                MemEntry {
                    text: text.into(),
                    updated_at: now_ms(),
                },
            )
        };
        notify_change(
            &self.kind,
            id,
            if created {
                crate::symbio_core::vdfs_provider::VFDS_CHANGE_CREATED
            } else {
                crate::symbio_core::vdfs_provider::VFDS_CHANGE_UPDATED
            },
        );
        created
    }

    /// 整表替换（静默）——启动时从磁盘镜像一份清单的标准动作
    pub fn replace_all(&self, items: impl IntoIterator<Item = (String, String)>) {
        let mut table = BTreeMap::new();
        let stamp = now_ms();
        for (id, text) in items {
            table.insert(
                id,
                MemEntry {
                    text,
                    updated_at: stamp,
                },
            );
        }
        *self.entries.write().unwrap() = table;
    }

    /// 删除条目 + 变更广播（不存在 = `false`，不报错）
    pub fn remove(&self, id: &str) -> bool {
        let removed = self.entries.write().unwrap().remove(id).is_some();
        if removed {
            notify_change(&self.kind, id, VFDS_CHANGE_DELETED);
        }
        removed
    }

    /// 路径末段 → 条目 id（剥呈现扩展名，与磁盘两型同一规则）
    pub fn id_of(&self, path: &str) -> String {
        super::entry::id_of(path, &self.kind)
    }
}

/// 插入并回答「是否为新建」
fn created_of(table: &mut BTreeMap<String, MemEntry>, id: &str, e: MemEntry) -> bool {
    table.insert(id.to_string(), e).is_none()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl VdfsProvider for MemoryVdfs {
    fn label(&self) -> Option<&str> {
        Some(&self.label)
    }

    fn description(&self) -> Option<&str> {
        Some("内存型资源：条目只在进程内，不落盘。")
    }

    fn icon(&self) -> Option<&str> {
        Some(self.kind.as_str())
    }

    fn root_access(&self) -> VdfsAccess {
        VdfsAccess::LIST
    }

    async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        if !path.is_empty() {
            return Err(VdfsError::not_found(format!(
                "内存条目是叶子，没有子项：{path}"
            )));
        }
        let table = self.entries.read().unwrap();
        Ok(table
            .iter()
            .map(|(id, e)| {
                let mut n = VdfsNode::file(id.clone(), id.clone(), VdfsAccess::READ_WRITE);
                n.kind = self.kind.clone();
                n.size = Some(e.text.len() as u64);
                n.updated_at = Some(e.updated_at);
                n
            })
            .collect())
    }

    async fn stat(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        if path.is_empty() {
            // 自身根：名字留空——provider 不知道自己的挂载名，由使用方回填
            return Ok(VdfsNode::dir("", self.label.clone(), VdfsAccess::LIST));
        }
        let id = self.id_of(path);
        let table = self.entries.read().unwrap();
        let e = table
            .get(&id)
            .ok_or_else(|| VdfsError::NotFound(format!("未找到条目「{id}」")))?;
        let mut n = VdfsNode::file(id.clone(), id, VdfsAccess::READ_WRITE);
        n.kind = self.kind.clone();
        n.size = Some(e.text.len() as u64);
        n.updated_at = Some(e.updated_at);
        Ok(n)
    }

    async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        if path.is_empty() {
            return Err(VdfsError::invalid("该路径是目录，不可读取内容"));
        }
        let id = self.id_of(path);
        self.get(&id)
            .map(|text| VdfsContent::text(path, text))
            .ok_or_else(|| VdfsError::NotFound(format!("未找到条目「{id}」")))
    }

    async fn write(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        if path.is_empty() {
            return Err(VdfsError::invalid("内存条目只能写到条目地址上"));
        }
        if content.binary {
            return Err(VdfsError::invalid("内存型条目不支持整包导入"));
        }
        let id = self.id_of(path);
        let created = self.set(&id, content.as_text().unwrap_or_default());
        Ok(VdfsWriteResponse {
            path: id,
            created,
            etag: None,
        })
    }

    async fn delete(&self, _ctx: &VdfsContext, path: &str, _recursive: bool) -> VdfsResult<()> {
        if path.is_empty() {
            return Err(VdfsError::Forbidden("不可删除挂载根".to_string()));
        }
        let id = self.id_of(path);
        if !self.remove(&id) {
            return Err(VdfsError::NotFound(format!("未找到条目「{id}」")));
        }
        Ok(())
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
    use crate::symbio_core::vdfs_provider::VdfsChange;

    #[tokio::test]
    async fn ops_mirror_the_disk_shapes() {
        let m = MemoryVdfs::new("model");
        let ctx = VdfsContext::empty();

        assert!(m.list(&ctx, "").await.unwrap().is_empty());
        let r = m
            .write(&ctx, "p1.model", &VdfsContent::text("", "{\"id\":\"p1\"}"))
            .await
            .unwrap();
        assert!(r.created);
        assert_eq!(r.path, "p1");
        // 呈现扩展名不是地址的一部分
        assert_eq!(
            m.read(&ctx, "p1").await.unwrap().as_text(),
            Some("{\"id\":\"p1\"}")
        );
        assert_eq!(m.stat(&ctx, "p1.model").await.unwrap().name, "p1");
        assert_eq!(m.list(&ctx, "").await.unwrap().len(), 1);
        m.delete(&ctx, "p1", false).await.unwrap();
        assert!(matches!(
            m.delete(&ctx, "p1", false).await.unwrap_err(),
            VdfsError::NotFound(_)
        ));
    }

    /// 克隆共享同一张表：每次 traverse 重新构造 provider 也不会读到旧清单
    #[test]
    fn clones_share_one_table() {
        let a = MemoryVdfs::new("model");
        let b = a.clone();
        a.set("p1", "x");
        assert_eq!(b.get("p1").as_deref(), Some("x"));
        b.remove("p1");
        assert!(a.get("p1").is_none());
    }

    /// 镜像刷新（整表替换）不广播，逐条写广播——批量灌入不该打扰订阅方
    ///
    /// 广播频道按 `kind` 全局持有（同一 provider 会被每次 traverse 重新构造），
    /// 因此订阅类测试必须用**独占的 kind**，否则与同 binary 内其它测试互相串台。
    #[tokio::test]
    async fn replace_all_is_silent_set_is_announced() {
        let m = MemoryVdfs::new("watch-model");
        let seen: Arc<RwLock<Vec<VdfsChange>>> = Arc::new(RwLock::new(Vec::new()));
        let sink = {
            let seen = seen.clone();
            Arc::new(move |c: VdfsChange| {
                seen.write().unwrap().push(c);
            })
        };
        m.watch(&VdfsContext::empty(), "", sink).await.unwrap();

        m.replace_all(vec![("quiet".to_string(), "1".to_string())]);
        m.set("loud", "2");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let paths: Vec<String> = seen
            .read()
            .unwrap()
            .iter()
            .map(|c| c.path.clone())
            .collect();
        assert_eq!(paths, vec!["loud".to_string()]);
        assert_eq!(
            seen.read().unwrap()[0].change,
            crate::symbio_core::vdfs_provider::VFDS_CHANGE_CREATED,
            "首次写入是新建"
        );

        // 整表替换同样是静默的（且丢掉旧条目）
        m.replace_all(vec![("p9".to_string(), "3".to_string())]);
        assert!(m.get("quiet").is_none());
        assert!(m.get("p9").is_some());
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(seen.read().unwrap().len(), 1);
        m.unwatch(&VdfsContext::empty(), "").await.unwrap();
        m.set("again", "4");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            seen.read().unwrap().len(),
            1,
            "unwatch 后不得再收到事件（watch/unwatch 严格配对）"
        );
    }

    /// 无持久化能力的操作保持 `NotImplemented`
    #[tokio::test]
    async fn unsupported_ops_stay_not_implemented() {
        let m = MemoryVdfs::new("model");
        let ctx = VdfsContext::empty();
        assert!(m.mkdir(&ctx, "d").await.unwrap_err().is_not_implemented());
        assert!(m
            .move_item(&ctx, "a", "b")
            .await
            .unwrap_err()
            .is_not_implemented());
        assert!(m
            .action(&ctx, "p1", "export", None)
            .await
            .unwrap_err()
            .is_not_implemented());
        let err = m
            .write(&ctx, "p1", &VdfsContent::binary("", "eA==", 1))
            .await
            .unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)));
    }
}
