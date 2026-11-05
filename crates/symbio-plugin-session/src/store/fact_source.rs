//! 会话的事实源实现与开面。
//!
//! 方向：**装配层经 [`FactSourceOpener`] 打开**，句柄与上下文键 `FACT_SOURCE`
//! 共用同一份底层 `WalStore`；会话内部不再自己造事实源。这里只把 `WalStore`
//! 包成契约要求的五个方法——信封（`event_id` / `seq` / `ts` / `cost_ms`）由
//! `WalStore::append` 在入库时填，写方只交草稿。

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};

use symbio_kernel::event::{Event, Seq};
use symbio_kernel::exec::ExecEventSink;
use symbio_kernel::store::Store;
use symbio_plugin_api::drive_entry::{DriveEntry, DriveTarget};
use symbio_plugin_api::fact_source::{FactSource, FactSourceOpener, Principal};
use symbio_plugin_api::FactDraft;

use super::wal::WalStore;

/// 会话存储目录（进程内首次登记；开面据此解析共享实例）。
static STORAGE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 由会话插件构造时登记其存储目录。
pub(crate) fn remember_storage_dir(dir: &Path) {
    let _ = STORAGE_DIR.set(dir.to_path_buf());
}

/// 契约实现：一个会话的 `WalStore` + 五方法。
pub(crate) struct SessionFactSource {
    /// 自引用弱句柄：`append` 踢驱动时把本会话的事实源**原样交回**，
    /// 不再按会话 id 走开面反查（反查会落到进程级主根的 store 上）。
    me: Weak<Self>,
    wal: Arc<WalStore>,
    /// 会话身份：驱动收口据此踢对应会话（事实源本身就是会话边界）。
    session_id: String,
    /// 会话 metadata 的工作目录：`append` 不携带运行选项，驱动收口把它带进工具上下文。
    workdir: Option<String>,
    /// 本会话所属**部署根**的驱动实例；`None` = 只读事实源，不踢驱动。
    drive: Option<Arc<dyn DriveEntry>>,
    /// 执行期出口：真实转写句柄，或本根**显式声明**的静默。
    sink: ExecEventSink,
}

impl SessionFactSource {
    /// 打开会话事实源；`drive` / `sink` 由打开者按自己的根绑定。
    pub(crate) fn open(
        wal: Arc<WalStore>,
        session_id: String,
        workdir: Option<String>,
        drive: Option<Arc<dyn DriveEntry>>,
        sink: ExecEventSink,
    ) -> Arc<dyn FactSource> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            wal,
            session_id,
            workdir,
            drive,
            sink,
        })
    }
}

impl FactSource for SessionFactSource {
    fn append(&self, actor: Principal, draft: FactDraft) -> Result<u64, String> {
        let event = symbio_plugin_api::fact_source::draft_into_event(actor, draft);
        let seq = Store::append(&*self.wal, event)
            .map(|seq| seq.value())
            .map_err(|e| format!("{e:?}"))?;

        // 事实写入的**唯一收口**：落账成功后踢一次本根绑定的驱动。
        //
        // 驱动句柄、会话身份、工作目录、事实源都在打开时绑好；这里**不反查**
        // 任何进程级入口或 store。只读事实源（`drive == None`）落地即止。
        // 写口在异步环境里，future 交给当前运行时 spawn（装配层不持有运行时）。
        // 未装配（单测）或不在运行时里 ⇒ 不驱动，写入本身照常成功。
        if let Some(drive) = &self.drive {
            if let (Some(this), Ok(handle)) =
                (self.me.upgrade(), tokio::runtime::Handle::try_current())
            {
                let facts: Arc<dyn FactSource> = this;
                let target = DriveTarget {
                    session_id: self.session_id.clone(),
                    workdir: self.workdir.clone(),
                    facts,
                    sink: self.sink.clone(),
                };
                if let Some(task) = drive.kick(target) {
                    handle.spawn(task);
                }
            }
        }
        Ok(seq)
    }

    fn facts(&self) -> Vec<Event> {
        Store::range(&*self.wal, Seq::new(0))
    }

    fn range(&self, from_seq: u64) -> Vec<Event> {
        Store::range(&*self.wal, Seq::new(from_seq))
    }

    fn head(&self) -> u64 {
        Store::head(&*self.wal).value()
    }

    fn retire_prefix(&self, before_seq: u64) -> Result<(), String> {
        self.wal
            .archive_before(Seq::new(before_seq))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// 开面实现：按会话 id 取共享 `SessionStore` 的同一事实源。
///
/// 开面是**装配层的读面**（CLI 投影 / VDFS 读），不承载某个部署根的驱动：
/// 这里显式交出只读事实源（无驱动、静默出口），写入与开轮不走这条路。
pub(crate) struct SessionFactOpener;

impl FactSourceOpener for SessionFactOpener {
    fn open(&self, session_id: &str) -> Result<Arc<dyn FactSource>, String> {
        let dir = STORAGE_DIR
            .get()
            .cloned()
            .ok_or_else(|| "会话存储目录尚未登记".to_string())?;
        let store = super::shared_entry(&dir).store.clone();
        let wal = store
            .facts(session_id)?
            .ok_or_else(|| format!("会话 {session_id} 没有事实源"))?;
        let workdir = store
            .session_dir(session_id)
            .and_then(|dir| super::layout::read_workdir(&dir));
        Ok(SessionFactSource::open(
            wal,
            session_id.to_string(),
            workdir,
            None,
            ExecEventSink::silent(),
        ))
    }
}

static SESSION_FACT_OPENER: SessionFactOpener = SessionFactOpener;

symbio_plugin_api::submit_fact_source_opener!(SESSION_FACT_OPENER);
