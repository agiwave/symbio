//! 会话的事实源实现与开面。
//!
//! 方向：**装配层经 [`FactSourceOpener`] 打开**，句柄与上下文键 `FACT_SOURCE`
//! 共用同一份底层 `WalStore`；会话内部不再自己造事实源。这里只把 `WalStore`
//! 包成契约要求的五个方法——信封（`event_id` / `seq` / `ts` / `cost_ms`）由
//! `WalStore::append` 在入库时填，写方只交草稿。

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use symbio_kernel::event::{Event, Seq};
use symbio_kernel::store::Store;
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
    wal: Arc<WalStore>,
}

impl SessionFactSource {
    pub(crate) fn new(wal: Arc<WalStore>) -> Arc<dyn FactSource> {
        Arc::new(Self { wal })
    }
}

impl FactSource for SessionFactSource {
    fn append(&self, scope: u64, actor: Principal, draft: FactDraft) -> Result<u64, String> {
        let event = symbio_plugin_api::fact_source::draft_into_event(scope, actor, draft);
        Store::append(&*self.wal, event)
            .map(|seq| seq.value())
            .map_err(|e| format!("{e:?}"))
    }

    fn facts(&self, scope: u64) -> Vec<Event> {
        Store::range(&*self.wal, Seq::new(0))
            .into_iter()
            .filter(|event| event.scope == scope)
            .collect()
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
        Ok(SessionFactSource::new(wal))
    }
}

static SESSION_FACT_OPENER: SessionFactOpener = SessionFactOpener;

symbio_plugin_api::submit_fact_source_opener!(SESSION_FACT_OPENER);
