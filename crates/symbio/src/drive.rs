//! 宿主驱动入口——**只认三样东西**：事实源、`drive` 能力行、事实源里已有的新事实。
//!
//! 这一层**不认识端口名 / 业务事件名**，也不写任何业务事实：端口执行体与债行都由
//! 插件经 `port` / `debt` 族自报，`drive` 族由环自报，宿主只把「本会话现在有新事实」
//! 这件事交给环。**加一个新端口 / 新能力 / 新债行，这里一行都不改。**
//!
//! ## 为什么没有 `loop { drive }`
//!
//! 外层每看到一条新事实只触发**一次**（CP-009）：环内部的轮次推进归环自己，
//! 它返回 `NoDebt` / `Waiting` / `NeedsHuman` 就结束。`Waiting` 是**在飞**（某端口
//! 已派发、应答未落），这条应答落地后由同一个入口再次触发，因此那一刻不算空闲；
//! `NeedsHuman` 才把控制权交还（等答复 / 授权 / 超时唤醒）。两种「停」的分界见
//! `docs/003-design/051-机制形状规格.md` §2（D-359）。
//!
//! ## 按根构造，不按入口构造
//!
//! [`HostDrive`] 的每个实例绑一份部署根（主根 / 子智能体容器树 / 测试装配）：
//! 能力收集从这份根出发，会话的 `session_id` 只用来取上下文身份，**不再**反查
//! 进程级 store。实例由事实源在打开时持有，踢的时候连事实源一起交回。
//!
//! ## 幂等
//!
//! 入口没有跨轮状态：同一会话相邻两次触发，第一次把债还清后，第二次的债表现算
//! 是空的 ⇒ `NoDebt`。事实源是唯一输入，事实源本身绑定会话。

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use symbio_host::{DriveOutcome, DriveReport, Host};
use symbio_kernel::exec::{ExecAbortSignal, ABORT_SIGNAL};
use symbio_kernel::plugin::{
    Plugin, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest,
};
use symbio_plugin_api::fact_source::{declared_fact_source_opener, FactSource, FACT_SOURCE};
use symbio_plugin_api::{DriveBinding, DriveBindingDecl, DriveEntry, DriveTarget, ExecEventSink};
use symbio_sdk::keys::{AGENT_ID, MODE, RISK_LEVEL, SOURCE_PRINCIPAL, WORKDIR};

/// 一次驱动需要的**世界**：事实源 + 通用上下文（会话 / 提供方 / 工作目录 / 出口）。
///
/// 这里没有端口名，也没有业务事件名——环从 `drive` 族取能力行，从 `port` / `debt`
/// 族取执行体与债表。
pub struct FactWorld<'a> {
    pub facts: Arc<dyn FactSource>,
    pub session_id: &'a str,
    /// 选定的 provider（会话 metadata 的 `provider_id`）；`None` = 模型侧默认。
    pub provider: Option<&'a str>,
    pub workdir: &'a str,
    /// 会话主体来源（metadata 的 `agent_id`，空串 = 无绑定主体）。
    /// 由装配层随事实源带入，`drive_on_new_fact` 只在这里设一次 `AGENT_ID`。
    pub agent_id: &'a str,
    /// 会话运行参数（写口按 `req > metadata > 默认` 解析完的结果）。
    ///
    /// `append` 不携带运行选项，由事实源打开时取来；这里挂进本轮 ctx，
    /// 审批闸门（`RISK_LEVEL` / `MODE`）只读。
    pub mode: &'a str,
    pub risk_level: &'a str,
    /// 本会话的 Agent 权限作用域（D-153）：由轮装配发布、随驱动目标带来。
    /// `None` = 本会话没选中 Agent（宿主态）；驱动只发布槽，不解析 manifest。
    pub agent_scope: Option<&'a symbio_sdk::agent::AgentScope>,
    /// 本轮入站来源主体（D-067）：本机用户发起时 `None`。
    pub source_principal: Option<&'a str>,
    /// 本轮的中止信号（`DE-047` 驱动级信号）：随事实源的交回值带来。
    /// `None` = 没有在途轮次；读方按「不会中止」处理（`ExecAbortSignal::of` 同一语义）。
    pub abort: Option<&'a ExecAbortSignal>,
    /// 执行期出口：由会话插件按根交出的真实转写句柄（内部请求显式静默）。
    pub sink: ExecEventSink,
}

/// 装配层打开该会话的**只读事实源**（经登记开面；插件不自建）。
///
/// 这是读面（CLI 投影 / `session/project`）：不绑驱动、出口显式静默。开轮写口
/// 不走这里——它由会话插件按本根绑定，见 `SessionPlugin::fact_source`。
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
    // 会话主体的**唯一**装载点：值来自会话 metadata 的 `agent_id`，
    // 由事实源打开时取来。执行体只读，不再各自派生或回落。
    ctx.set(AGENT_ID, world.agent_id.to_string());
    // 会话运行参数同址装配：审批闸门与模型端口都从这个 ctx 读值，
    // 与 `workdir` / `agent_id` 同一来源（会话 metadata），不在执行体里二次反查。
    ctx.set(MODE, world.mode.to_string());
    ctx.set(RISK_LEVEL, world.risk_level.to_string());
    // 入站来源主体（D-067）：同址装配，工具侧 `EffectEnv.source` 只读它；
    // 缺席 = 本机用户发起，执行体回落会话身份（不在这里补默认值就是「没声明」）。
    if let Some(source) = world.source_principal {
        ctx.set(SOURCE_PRINCIPAL, source.to_string());
    }
    // 中止信号（`DE-047`）：与 `source_principal` 同一来源（会话状态），在这里
    // 挂进本轮 ctx。环与工具 / 模型执行体都经 `ExecAbortSignal::of(ctx)` 读同一个对象——
    // 不在执行体里各建一个（那正是「每次执行现造死信号」的旧断点）。
    if let Some(signal) = world.abort {
        ctx.set(ABORT_SIGNAL, signal.clone());
    }
    // 装配层 open 的同一句柄：宿主与插件上下文共享它，插件不自建 store。
    ctx.set(FACT_SOURCE, Arc::clone(&world.facts));
    let base: Arc<dyn PluginInvokeRequest> = Arc::new(ctx);
    // Agent 权限作用域（D-153）：随轮带来，在这里挂进本轮 ctx。执行体（web / fs
    // 权限闸门、工具侧）只读 `AGENT_PERMS`；取不到就不发布槽 = 宿主态。
    if let Some(scope) = world.agent_scope {
        scope.publish(&base);
    }

    let host = Host::with_store(Arc::clone(root), Arc::clone(&world.facts));
    host.drive(&base, world.session_id, world.provider, world.sink.clone())
        .await
}

// ── 驱动收口：事实写入成功之后踢一次 ────────────────────────────────────
//
// 这是**唯一实现**。写口（会话事实源 `append`）经 `drive_entry` 契约调到这
// 里；Tauri / gateway / CLI 都不再各自记得「写完要驱动」。

/// 一份部署根的驱动实例：根 + 按会话的重入位。
struct HostDrive {
    /// 本实例的部署根（能力收集从这里出发）。
    root: Arc<dyn Plugin>,
    /// 本根的会话推进状态（**不是进程级表**）。
    kicks: Mutex<HashMap<String, Arc<KickState>>>,
}

impl HostDrive {
    fn new(root: Arc<dyn Plugin>) -> Self {
        Self {
            root,
            kicks: Mutex::new(HashMap::new()),
        }
    }

    fn kick_state(&self, session_id: &str) -> Arc<KickState> {
        let mut guard = self
            .kicks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Arc::clone(guard.entry(session_id.to_string()).or_insert_with(|| {
            Arc::new(KickState {
                running: AtomicBool::new(false),
                dirty: AtomicBool::new(false),
                in_flight: AtomicBool::new(false),
                latest: Mutex::new(None),
                idle: tokio::sync::Notify::new(),
            })
        }))
    }
}

/// 本会话的推进状态：两个原子位 + 最新目标槽 + 一个唤醒句柄（**不加锁**）。
struct KickState {
    running: AtomicBool,
    dirty: AtomicBool,
    /// 上一轮驱动停在**在飞**（`Waiting`）：环自己还能推进，这一轮不算空闲。
    ///
    /// 与 `running` / `dirty` 分开：驱动循环退出后 `running` 为假，但「还有一笔已派发、
    /// 租约未过的债」意味着应答还没落格——此刻叫醒 `settled`，调用方就会在空档里
    /// 投影 / 收口（t29：巩固看到 3+3 而跳过）。`NeedsHuman` 落回假：那是交还。
    in_flight: AtomicBool,
    /// 最近一次 `kick` 交来的目标。驱动在跑时后续 `append` 只置 `dirty`，
    /// 但**目标必须换成最新那份**：作用域、中止信号、workdir 都随目标带过来，
    /// 拿首次那份跑完整轮 = 后续轮次读到旧的作用域（t84 的「该拒而放行」窗口）。
    latest: Mutex<Option<DriveTarget>>,
    /// 空闲时的唤醒句柄：`settled` 等它，驱动退出时叫醒。
    idle: tokio::sync::Notify,
}

tokio::task_local! {
    /// 当前任务是否正处在驱动体内（`settled` 的自锁防护用）。
    static IN_DRIVE: ();
}

impl DriveEntry for HostDrive {
    fn kick(
        &self,
        target: DriveTarget,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>> {
        let state = self.kick_state(&target.session_id);
        // 最新目标先落槽：即使已有驱动在跑，它下一圈也要用**这一份**（作用域 /
        // 中止信号 / workdir 都随目标带过来；拿首份跑完整轮就是 t84 放行窗口）。
        *state
            .latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(target);
        // 已在跑：只置 dirty，不嵌套。
        if state.running.swap(true, Ordering::AcqRel) {
            state.dirty.store(true, Ordering::Release);
            return None;
        }

        let root = Arc::clone(&self.root);
        Some(Box::pin(IN_DRIVE.scope((), async move {
            loop {
                let current = state
                    .latest
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                let Some(target) = current else {
                    // 槽被别的接管者取走（理论上不可达）：干净退出，不重复跑。
                    state.running.store(false, Ordering::Release);
                    state.idle.notify_waiters();
                    break;
                };
                drain(&root, &target, &state).await;
                // 跑着期间来过写入：再跑一轮，用最新目标，`running` 保持 true。
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
        Box::pin(wait_idle(self.kick_state(session_id)))
    }
}

/// 这一份会话推进状态是否已经空闲：没有在跑的驱动、没有待续的写入，也没有在飞的债。
///
/// 三个条件缺一不可：`Waiting`（在飞）时 `running` 为假但应答还没落格，此时放行会让
/// 调用方在空档里投影（t29：巩固只看到本轮一半的编码，合并被跳过）。
fn is_settled(state: &KickState) -> bool {
    !state.running.load(Ordering::Acquire)
        && !state.dirty.load(Ordering::Acquire)
        && !state.in_flight.load(Ordering::Acquire)
}

/// 等一份会话推进状态空闲（`settled` 的等待体）。
async fn wait_idle(state: Arc<KickState>) {
    loop {
        let notified = state.idle.notified();
        tokio::pin!(notified);
        // 先登记再查条件：避免「查完已空闲、通知却发生在查之前」的丢唤醒。
        notified.as_mut().enable();
        if is_settled(&state) {
            return;
        }
        notified.await;
    }
}

/// 按根绑定工厂：装配层登记的**唯一实现**；每个部署根各持一份 [`HostDrive`]。
struct HostDriveBinding;

static HOST_DRIVE_BINDING: HostDriveBinding = HostDriveBinding;

inventory::submit! {
    DriveBindingDecl { binding: &HOST_DRIVE_BINDING }
}

impl DriveBinding for HostDriveBinding {
    fn bind(&self, root: Arc<dyn Plugin>) -> Arc<dyn DriveEntry> {
        Arc::new(HostDrive::new(root))
    }
}

/// 主根的那一份实例（`settled` / CLI 等入口用）。
static MAIN_DRIVE: OnceLock<Arc<HostDrive>> = OnceLock::new();

/// 装配层登记主根插件：构造主根的驱动实例，供只认 `DriveEntryDecl` 的入口取用。
pub fn install_root(root: &Arc<dyn Plugin>) {
    let _ = MAIN_DRIVE.set(Arc::new(HostDrive::new(Arc::clone(root))));
}

/// 主根实例的静态入口（`inventory` 登记；未装配时 `None`）。
struct MainDriveEntry;

static MAIN_DRIVE_ENTRY: MainDriveEntry = MainDriveEntry;

inventory::submit! {
    symbio_plugin_api::DriveEntryDecl { entry: &MAIN_DRIVE_ENTRY }
}

impl DriveEntry for MainDriveEntry {
    fn kick(
        &self,
        target: DriveTarget,
    ) -> Option<Pin<Box<dyn Future<Output = ()> + Send + 'static>>> {
        MAIN_DRIVE.get()?.kick(target)
    }

    fn settled(&self, session_id: &str) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        match MAIN_DRIVE.get() {
            Some(drive) => drive.settled(session_id),
            None => Box::pin(async {}),
        }
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

/// 驱动一轮。**循环在 [`DriveEntry::kick`] 里**：环自己「读事实 → 选债 → 派发 →
/// 再读」，直到没有可还的债或只剩等外部事实。这里不一步一返回，也不在外层再包一圈
/// `loop { drive }`。
///
/// 出口只分两类：`Waiting`（在飞）**不算空闲**——应答还没落格，继续等；`NeedsHuman`
/// 与安全阀出口才算这一轮交还。置位放在 `running` 清零之前，`wait_idle` 因此不会
/// 看到「已停但还没置位」的中间态。
async fn drain(root: &Arc<dyn Plugin>, target: &DriveTarget, state: &KickState) {
    // 每次进入先清位：只有本轮确凿停在在飞才重新置上。
    state.in_flight.store(false, Ordering::Release);
    // 事实源就是本会话的那一份（打开时绑好，不按 id 反查）。
    if target.facts.head() == 0 {
        return;
    }
    // 会话 metadata 是 `workdir` 的唯一 owner；缺省才回落到进程当前目录。
    let workdir = target
        .workdir
        .as_deref()
        .filter(|dir| !dir.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|dir| dir.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    // 会话主体是装配层打开事实源时取的；推进路径不猜、不回落。
    let Some(agent_id) = target.agent_id.as_deref() else {
        eprintln!(
            "驱动失败（{}）：会话主体不可用（metadata 未给出 agent_id，拒绝回落 session/main）",
            target.session_id
        );
        return;
    };
    let world = FactWorld {
        facts: Arc::clone(&target.facts),
        session_id: &target.session_id,
        // 提供方随会话 metadata 一并取来（`req > metadata > 默认` 的元数据档）。
        provider: target.provider_id.as_deref(),
        workdir: &workdir,
        agent_id,
        mode: &target.mode,
        risk_level: &target.risk_level,
        agent_scope: target.agent_scope.as_ref(),
        source_principal: target.source_principal.as_deref(),
        abort: target.abort.as_ref(),
        sink: target.sink.clone(),
    };
    match drive_on_new_fact(root, &world).await {
        Ok(report) => {
            // 在飞 = 环自己还能推进：这一轮要继续算「没空闲」，不能让调用方在此
            // 投影 / 收口（应答还没落格，t29 的早投影就是从这里来的）。
            state.in_flight.store(
                matches!(&report.outcome, DriveOutcome::Waiting(_)),
                Ordering::Release,
            );
            if let Some(notice) = drive_notice(&target.session_id, &report.outcome) {
                // 宿主装配层不链 `tracing`（插件日志宏在这里展开不出来），退回 stderr。
                eprintln!("{notice}");
            }
        }
        Err(e) => {
            // 宿主装配层不链 `tracing`（插件日志宏在这里展开不出来），退回 stderr。
            eprintln!("驱动失败（{}）：{e}", target.session_id);
        }
    }
}

/// 安全阀出口的可见文案：`Exhausted` 与 `Stalled` **都意味着这一轮没跑完**，
/// 两条都必须让调用面看见；`Waiting`（在飞，等下一次踢）与 `NeedsHuman`（交还人工，
/// 等外部事实）是正常出口，不出声。
///
/// 判据与文案同址，是为了不出现「一条有声、一条闷着」的不对称——闷着的那条
/// 迟早被当成正常结束。
fn drive_notice(session_id: &str, outcome: &DriveOutcome) -> Option<String> {
    match outcome {
        DriveOutcome::Exhausted { steps, outstanding } => Some(format!(
            "驱动停摆（{session_id}）：步数预算用尽（{steps} 步），未收束 {} 笔：{outstanding:?}",
            outstanding.len()
        )),
        DriveOutcome::Stalled(open) => Some(format!(
            "驱动停摆（{session_id}）：一圈没有新增事实，未收束 {} 笔：{open:?}",
            open.len()
        )),
        _ => None,
    }
}

#[cfg(test)]
#[path = "drive.test.rs"]
mod tests;
