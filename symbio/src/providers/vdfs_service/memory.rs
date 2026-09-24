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

use crate::symbio_core::now_ms;
use crate::symbio_core::vdfs::host::{notify_change, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsProvider, VdfsRequest,
    VdfsResponse, VdfsResult, VdfsWriteResponse,
};
use crate::symbio_core::{lock_read, lock_write};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

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
        lock_read(&self.entries).keys().cloned().collect()
    }

    /// 条目原文（不存在 → `None`）
    pub fn get(&self, id: &str) -> Option<String> {
        lock_read(&self.entries).get(id).map(|e| e.text.clone())
    }

    /// 条目写入时间（Unix 毫秒；不存在 → `None`）
    pub fn updated_at(&self, id: &str) -> Option<i64> {
        lock_read(&self.entries).get(id).map(|e| e.updated_at)
    }

    /// 写条目（新建或覆盖）+ 变更广播；返回是否为**新建**
    pub fn set(&self, id: &str, text: impl Into<String>) -> bool {
        let created = {
            let mut table = lock_write(&self.entries);
            created_of(
                &mut table,
                id,
                MemEntry {
                    text: text.into(),
                    updated_at: now_ms(),
                },
            )
        };
        notify_change(&self.kind, id);
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
        *lock_write(&self.entries) = table;
    }

    /// 删除条目 + 变更广播（不存在 = `false`，不报错）
    pub fn remove(&self, id: &str) -> bool {
        let removed = lock_write(&self.entries).remove(id).is_some();
        if removed {
            notify_change(&self.kind, id);
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

/// 自述（标题 / 图标 / 根访问位）已并入 PluginMeta，由持有方在 `Plugin::meta()` 提供。
#[async_trait]
impl VdfsProvider for MemoryVdfs {
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
                        "内存条目是叶子，没有子项：{path}"
                    )));
                }
                let table = lock_read(&self.entries);
                Ok(VdfsResponse::List(
                    table
                        .iter()
                        .map(|(id, e)| {
                            let mut n =
                                VdfsNode::file(id.clone(), id.clone(), VdfsAccess::READ_WRITE);
                            n.kind = self.kind.clone();
                            n.size = Some(e.text.len() as u64);
                            n.updated_at = Some(e.updated_at);
                            n
                        })
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
                let id = self.id_of(path);
                let table = lock_read(&self.entries);
                let e = table
                    .get(&id)
                    .ok_or_else(|| VdfsError::NotFound(format!("未找到条目「{id}」")))?;
                let mut n = VdfsNode::file(id.clone(), id, VdfsAccess::READ_WRITE);
                n.kind = self.kind.clone();
                n.size = Some(e.text.len() as u64);
                n.updated_at = Some(e.updated_at);
                Ok(VdfsResponse::Stat(n))
            }

            VdfsRequest::Read => {
                if path.is_empty() {
                    return Err(VdfsError::invalid("该路径是目录，不可读取内容"));
                }
                let id = self.id_of(path);
                let content = self
                    .get(&id)
                    .map(|text| VdfsContent::text(path, text))
                    .ok_or_else(|| VdfsError::NotFound(format!("未找到条目「{id}」")))?;
                Ok(VdfsResponse::Read(content))
            }

            VdfsRequest::Write { content } => {
                if path.is_empty() {
                    return Err(VdfsError::invalid("内存条目只能写到条目地址上"));
                }
                if content.binary {
                    return Err(VdfsError::invalid("内存型条目不支持整包导入"));
                }
                let id = self.id_of(path);
                let created = self.set(&id, content.as_text().unwrap_or_default());
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
                if !self.remove(&id) {
                    return Err(VdfsError::NotFound(format!("未找到条目「{id}」")));
                }
                Ok(VdfsResponse::Unit)
            }

            VdfsRequest::Mkdir | VdfsRequest::Move { .. } | VdfsRequest::Action { .. } => {
                Err(VdfsError::NotImplemented)
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
#[path = "memory.test.rs"]
mod tests;
