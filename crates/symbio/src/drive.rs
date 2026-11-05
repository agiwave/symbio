//! 宿主驱动入口——**只认三样东西**：事实源、`drive` 能力行、事实源里已有的新事实。
//!
//! 这一层**不认识端口名 / 业务事件名**，也不写任何业务事实：端口执行体与债行都由
//! 插件经 `port` / `debt` 族自报，`drive` 族由环自报，宿主只把「本会话现在有新事实」
//! 这件事交给环。**加一个新端口 / 新能力 / 新债行，这里一行都不改。**
//!
//! ## 为什么没有 `loop { drive }`
//!
//! 外层每看到一条新事实只触发**一次**（CP-009）：环内部的轮次推进归环自己，
//! 它返回 `NoDebt` / `Waiting` 就结束。`Waiting` 等的是某个端口的**响应事实**，
//! 那条事实落地后由同一个入口再次触发——外层不持「正在跑」这类会话状态。
//!
//! ## 幂等
//!
//! 入口没有状态：同一会话相邻两次触发，第一次把债还清后，第二次的债表现算是空的
//! ⇒ `NoDebt`。事实源是唯一输入，事实源本身绑定会话。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use symbio_host::{DriveOutcome, DriveReport, Host};
use symbio_kernel::plugin::{
    Plugin, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest,
};
use symbio_plugin_api::fact_source::{declared_fact_source_opener, FactSource, FACT_SOURCE};
use symbio_sdk::keys::WORKDIR;

/// 一次驱动需要的**世界**：事实源 + 通用上下文（会话 / 提供方 / 工作目录）。
///
/// 这里没有端口名，也没有业务事件名——环从 `drive` 族取能力行，从 `port` / `debt`
/// 族取执行体与债表。
pub struct FactWorld<'a> {
    pub facts: Arc<dyn FactSource>,
    pub session_id: &'a str,
    pub provider: Option<&'a str>,
    pub workdir: &'a str,
}

/// 装配层打开该会话的**唯一事实源**（经登记开面；插件不自建）。
pub fn open_fact_source(session_id: &str) -> Result<Arc<dyn FactSource>, String> {
    declared_fact_source_opener()
        .ok_or_else(|| "没有登记事实源开面".to_string())?
        .open(session_id)
}

/// 一条新事实落地后**单次**驱动本会话。
///
/// 调用方不需要知道端口、债行，也不需要自己算任何身份。事实源为空 ⇒
/// 没有可触发的事实（错误，而不是一次空驱动）。
pub async fn drive_on_new_fact(
    root: &Arc<dyn Plugin>,
    world: &FactWorld<'_>,
) -> Result<DriveReport, String> {
    if world.facts.head() == 0 {
        return Err("事实源为空：没有可触发的事实".to_string());
    }

    // 父句柄交给执行体：工具 / 钩子按「调用方插件」路由，装配层给的是根插件
    // （环不碰这个字段，只原样转交）。
    let ctx = PluginSimpleRequest::new(Some(Arc::downgrade(root)), None);
    ctx.set(WORKDIR, world.workdir.to_string());
    // 装配层 open 的同一句柄：宿主与插件上下文共享它，插件不自建 store。
    ctx.set(FACT_SOURCE, Arc::clone(&world.facts));
    let base: Arc<dyn PluginInvokeRequest> = Arc::new(ctx);

    let host = Host::with_store(Arc::clone(root), Arc::clone(&world.facts));
    host.drive(&base, world.session_id, world.provider).await
}

// ── 驱动收口：事实写入成功之后踢一次 ────────────────────────────────────
//
// 这是**唯一实现**。写口（会话事实源 `append`）经 `drive_entry` 契约调到这
// 里；Tauri / gateway / CLI 都不再各自记得「写完要驱动」。

/// 装配后的根插件（首个装配的根生效；进程内只有一个根）。
static ROOT: OnceLock<Arc<dyn Plugin>> = OnceLock::new();

/// 装配层登记根插件，供驱动收口取用。
pub fn install_root(root: &Arc<dyn Plugin>) {
    let _ = ROOT.set(Arc::clone(root));
}

/// 本会话的推进状态：两个原子位 + 一个唤醒句柄（**不加锁**）。
struct KickState {
    running: AtomicBool,
    dirty: AtomicBool,
    /// 空闲时的唤醒句柄：`settled` 等它，驱动退出时叫醒。
    idle: tokio::sync::Notify,
}

tokio::task_local! {
    /// 当前任务是否正处在驱动体内（`settled` 的自锁防护用）。
    static IN_DRIVE: ();
}

static KICKS: OnceLock<Mutex<HashMap<String, Arc<KickState>>>> = OnceLock::new();

fn kick_state(session_id: &str) -> Arc<KickState> {
    let map = KICKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(guard.entry(session_id.to_string()).or_insert_with(|| {
        Arc::new(KickState {
            running: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            idle: tokio::sync::Notify::new(),
        })
    }))
}

/// 装配层的驱动实现（进程内唯一）。
struct HostDriveEntry;

static HOST_DRIVE_ENTRY: HostDriveEntry = HostDriveEntry;

inventory::submit! {
    symbio_plugin_api::DriveEntryDecl { entry: &HOST_DRIVE_ENTRY }
}

impl symbio_plugin_api::DriveEntry for HostDriveEntry {
    fn kick(
        &self,
        session_id: &str,
        workdir: Option<&str>,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>> {
        // 未装配根插件（单测）⇒ 不驱动。
        let root = ROOT.get()?.clone();
        let state = kick_state(session_id);
        // 已在跑：只置 dirty，不嵌套。
        if state.running.swap(true, Ordering::AcqRel) {
            state.dirty.store(true, Ordering::Release);
            return None;
        }

        let sid = session_id.to_string();
        let workdir = workdir.map(str::to_string);
        Some(Box::pin(IN_DRIVE.scope((), async move {
            loop {
                drain(&root, &sid, workdir.as_deref()).await;
                // 跑着期间来过写入：再跑一轮，`running` 保持 true。
                if state.dirty.swap(false, Ordering::AcqRel) {
                    continue;
                }
                state.running.store(false, Ordering::Release);
                // 清位之后才置上的 dirty：要么自己接着跑，要么交给已接管的驱动。
                if !state.dirty.swap(false, Ordering::AcqRel) {
                    state.idle.notify_waiters();
                    break;
                }
                if state.running.swap(true, Ordering::AcqRel) {
                    break; // 别人接管，它退出时会 notify
                }
            }
        })))
    }

    fn settled(&self, session_id: &str) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        // 驱动自身调用 ⇒ 直接返回，不自锁。
        if IN_DRIVE.try_with(|_| ()).is_ok() {
            return Box::pin(async {});
        }
        let state = kick_state(session_id);
        Box::pin(async move {
            loop {
                let notified = state.idle.notified();
                tokio::pin!(notified);
                // 先登记再查条件：避免「查完已空闲、通知却发生在查之前」的丢唤醒。
                notified.as_mut().enable();
                if !state.running.load(Ordering::Acquire) && !state.dirty.load(Ordering::Acquire) {
                    return;
                }
                notified.await;
            }
        })
    }
}

/// 等本会话驱动空闲（**等，不是推**）。
///
/// CLI 在 `session/consume` 之后用它等写口踢出的那次驱动落地，再投影转写。
pub async fn settled(session_id: &str) {
    if let Some(entry) = symbio_plugin_api::declared_drive_entry() {
        entry.settled(session_id).await;
    }
}

/// 驱动一轮。**循环在 `drive_on_new_fact` 里**：环自己「读事实 → 选债 → 派发 →
/// 再读」，直到 `NoDebt`（还清）或 `Waiting`（只剩在飞的债：模型未回 / 答复未到）。
/// 这里不一步一返回，也不在外层再包一圈 `loop { drive }`。
async fn drain(root: &Arc<dyn Plugin>, session_id: &str, workdir: Option<&str>) {
    // 事实源为空 ⇒ 没有可触发的事实。
    let Ok(facts) = open_fact_source(session_id) else {
        return;
    };
    if facts.head() == 0 {
        return;
    }
    // 会话 metadata 是 `workdir` 的唯一 owner；缺省才回落到进程当前目录。
    let workdir = workdir
        .filter(|dir| !dir.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|dir| dir.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let world = FactWorld {
        facts,
        session_id,
        // 收口只知道会话身份；提供方由环在驱动内按会话取。
        provider: None,
        workdir: &workdir,
    };
    let _ = drive_on_new_fact(root, &world).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use symbio_plugin_api::DriveEntry;

    /// `settled` 是「等」不是「立即返回」：跑着时挂起，空闲后由通知唤醒。
    #[tokio::test]
    async fn settled_waits_for_idle() {
        let sid = "drive-entry-settled-test";
        let state = kick_state(sid);
        state.running.store(true, Ordering::Release);

        let waiting = tokio::spawn(HostDriveEntry.settled(sid));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!waiting.is_finished(), "驱动还在跑时 settled 不该返回");

        state.running.store(false, Ordering::Release);
        state.idle.notify_waiters();
        tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
            .await
            .expect("空闲后应被唤醒")
            .unwrap();
    }
}
