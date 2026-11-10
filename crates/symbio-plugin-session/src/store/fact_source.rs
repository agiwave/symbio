//! 会话的事实源实现与开面。
//!
//! 方向：**装配层经 [`FactSourceOpener`] 打开**，句柄与上下文键 `FACT_SOURCE`
//! 共用同一份底层 `WalStore`；会话内部不再自己造事实源。这里只把 `WalStore`
//! 包成契约要求的五个方法——信封（`event_id` / `seq` / `ts` / `cost_ms`）由
//! `WalStore::append` 在入库时填，写方只交草稿。

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock, Weak};

use symbio_kernel::event::{Event, Seq};
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::store::Store;
use symbio_plugin_api::events::EVENT_ARTIFACT_ADDED;
use symbio_plugin_api::FactDraft;
use symbio_sdk::plugin::drive_entry::{DriveEntry, DriveTarget};
use symbio_sdk::plugin::fact_source::{FactSource, FactSourceOpener, Principal};

use super::layout::SessionParams;
use super::wal::WalStore;
use crate::declarations::EVENT_SESSION_CONSOLIDATED;
use crate::transcript::live::{self, LiveIndex};
use crate::transcript::supplements::SupplementDrain;

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
    /// 会话主体来源（metadata 的 `agent_id`）：随驱动目标交回，`append` 不再反查。
    agent_id: Option<String>,
    /// 会话运行参数（`mode` / `risk_level` / `provider_id`）：随驱动目标交回，
    /// `append` 不再反查 metadata。
    params: SessionParams,
    /// Agent 权限作用域（D-153）：由**收集方**（轮装配 `collect_capabilities` 之后）
    /// 发布，`append` 时随驱动目标带进本轮 ctx。`None` = 本会话没选中 Agent。
    ///
    /// 不放在 `params` 里：它不是 metadata 的静态选项，而是收集期才得到的运行期
    /// 事实；打开那一刻还没人发布。
    agent_scope: Arc<RwLock<Option<symbio_sdk::agent::AgentScope>>>,
    /// 本轮入站来源主体（D-067）：由开轮写方从收件箱条目的 `params` 发布，`append`
    /// 时随驱动目标带进本轮 ctx。`None` = 本机用户发起（工具侧回落会话身份）。
    source_principal: Arc<RwLock<Option<String>>>,
    /// 在途 Turn 的中止信号：**不是本事实源的私有状态**——它只引用会话状态里的
    /// 同一个 `Arc`（唯一所有者是 `ActiveSessionState`）。每次踢驱动读当前值，
    /// 置位后本轮后续任何派发都读得到（取走会退化成「只打断当前这次调用」）。
    abort_signal: Arc<RwLock<Option<ExecAbortSignal>>>,
    /// 本会话所属**部署根**的驱动实例；`None` = 只读事实源，不踢驱动。
    drive: Option<Arc<dyn DriveEntry>>,
    /// 执行期出口：真实转写句柄，或本根**显式声明**的静默。
    sink: ExecEventSink,
    /// 补充整合的**批次边界折叠器**：`artifact.added`（工具结果）落格后，把忙窗内
    /// 排队的补充抽干并合并成一条，落一条 `message.added`。`None` = 只读事实源。
    supplements: Option<SupplementDrain>,
    /// 实时投影的**配对索引**（见 `transcript::live` 模块头）：派发清单与拒言事实。
    ///
    /// 只是索引不是事实：未命中时回读事实源兜底，故它丢了也不影响正确性。
    live: std::sync::Mutex<LiveIndex>,
}

impl SessionFactSource {
    /// 打开会话事实源；`drive` / `sink` / `supplements` 由打开者按自己的根绑定。
    ///
    /// 参数虽多，但每一个都是**打开那一刻的装订**（没有一个是运行期可变的第二份
    /// 状态）；拆成结构体只是把同一组值换个地方拼，故就地收下。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open(
        wal: Arc<WalStore>,
        session_id: String,
        workdir: Option<String>,
        agent_id: Option<String>,
        params: SessionParams,
        agent_scope: Arc<RwLock<Option<symbio_sdk::agent::AgentScope>>>,
        source_principal: Arc<RwLock<Option<String>>>,
        abort_signal: Arc<RwLock<Option<ExecAbortSignal>>>,
        drive: Option<Arc<dyn DriveEntry>>,
        sink: ExecEventSink,
        supplements: Option<SupplementDrain>,
    ) -> Arc<dyn FactSource> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            wal,
            session_id,
            workdir,
            agent_id,
            params,
            agent_scope,
            source_principal,
            abort_signal,
            drive,
            sink,
            supplements,
            live: std::sync::Mutex::new(LiveIndex::new()),
        })
    }

    /// 工具结果落格之后的**补充折叠**（同步、落在工具结果之后）。
    ///
    /// 为什么触发点在这里：合并消息必须排在工具结果**之后**（本轮的第二次请求
    /// 才会把它当已有输入）。写臂那一刻落格会插在 `tool_calls` 与结果之间，违反
    /// 协议顺序；而批次边界没有别的事实时刻——`artifact.added` 就是工具结果本身，
    /// 且它经过本事实源这唯一写口。此处折叠后，环的下一次请求重读事实自然读到。
    ///
    /// `blocking_read / blocking_write` 只在多线程 runtime 的 worker 上可用
    /// `block_in_place` 包住：其它形态（单线程 runtime / 无运行时）留队给轮收尾的
    /// A 线，**不在这里 panic**。
    fn fold_supplements(&self) {
        let Some(drain) = self.supplements.as_ref() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
            return;
        }
        // 取件要用 `blocking_write`，落格要走 `apply`（`Handle::block_on`）：两者都
        // 只能在 `block_in_place` 里做。`apply` 是消息列表的唯一写入口——它把事实与
        // `messages.json` 一起折，别处只落事实会让读缓存与重建分叉（LWF-001）。
        tokio::task::block_in_place(|| drain.land_merged_sync(&handle));
    }

    /// 事实落格即投影（`D-424` / `D-425`，见 `transcript::live` 模块头）：把刚落格的事实与
    /// 它的配对事实折成节点，**同步**交给转写唯一写入点——终态节点来自同一份 `rebuild`；
    /// 面向读者的模型派发额外发一个**临时轮骨架**（内容未到时的可见形态，只走流通道）。
    ///
    /// 同步而不是 spawn：帧必须先于下一轮模型的流式帧到达，否则「思考完成了」「工具
    /// 结果出来了」会晚于下一段正文出现。`block_in_place` 是既有的同步发射先例
    /// （见 [`Self::fold_supplements`]）：只在多线程 runtime 的 worker 上可用；其它
    /// 形态（单线程 runtime / 无运行时 / 单测）退化为不发——实时面丢的帧由轮收尾的
    /// 重建面补齐，正确性不变。
    fn project_live(&self, event: Event) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
            return;
        }
        let frames = {
            let mut index = self
                .live
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            index.observe(&event);
            index.frames_for(&event, &self.wal)
        };
        if frames.is_empty() {
            return;
        }
        tokio::task::block_in_place(|| {
            for frame in frames {
                handle.block_on(self.sink.emit(frame));
            }
        });
    }
}

impl FactSource for SessionFactSource {
    fn append(&self, actor: Principal, draft: FactDraft) -> Result<u64, String> {
        // D-339：`@principal` 是声明期的会话归属哨兵，由**会话装配**在落格时解析成
        // 本会话主体（`session/main` / `session/arbiter` / `session/worker` 由此得到）。
        // 载体缺失（metadata 读不到）就显式失败——不回落主体；载体在而值为空是
        // 「会话确实没有绑定智能体」，其主体定义就是 `session/main`。
        let actor = if actor == symbio_kernel::event::OWNER_PRINCIPAL {
            let Some(agent_id) = self.agent_id.as_deref() else {
                return Err(format!(
                    "事实归属为 `{}`，但会话主体不可用（metadata 取不到 agent_id；拒绝回落主体）",
                    symbio_kernel::event::OWNER_PRINCIPAL
                ));
            };
            symbio_plugin_api::principal_of(Some(agent_id.trim()))
        } else {
            actor
        };
        let kind = draft.kind.clone();
        let mut event = symbio_sdk::plugin::fact_source::draft_into_event(actor, draft);
        // 时刻属于信封，落格时由事实源兜底；这里先写同一时刻，落格后的投影就能用
        // 手上这一份（含 `event_id`，`seq` 在 append 后补），不必回读全史——
        // `Store::range` 要拼接归档段，长会话下每次都是全量克隆。
        event.ts = symbio_sdk::clock::clock_now_ms();
        let watched = live::watches(&event);
        // 只读事实源没有出口，落地即止；不需要投影时也不复制载荷。
        let replay = watched && self.drive.is_some();
        let stored = replay.then(|| event.clone());
        let seq = Store::append(&*self.wal, event)
            .map(|seq| seq.value())
            .map_err(|e| format!("{e:?}"))?;

        // **落格即投影**（D-424）：终态节点由投影器在此刻折出（见 `transcript::live`）。
        if let Some(mut event) = stored {
            // 投影副本不是入库信封：号直接写字段，不借 `Store` 的赋号/兜底纪律。
            event.seq = Some(Seq::new(seq));
            self.project_live(event);
        }

        // **批次边界**：工具结果刚进事实源，而环的下一次请求还没装配。此刻折进忙窗
        // 内排队的补充——合并事实的 seq 因此必然排在工具结果之后（见 `fold_supplements`）。
        if kind == EVENT_ARTIFACT_ADDED {
            self.fold_supplements();
        }

        // **取代格**：压缩快照一落进 live，被它取代的前缀就不该再留在 live 段
        // （[D-053] 决策 6）。归档段保留「零回退损失」，而读面（`Store::range`）
        // 本就含归档段——这一步只换 live 段起点，不改变谁能读到什么。
        //
        // 归档失败不动摇已落格的事实（事实已入库，重建面本来就按快照换基），
        // 因此不把它升级成写失败。
        if kind == EVENT_SESSION_CONSOLIDATED {
            let _ = self.wal.archive_before(Seq::new(seq));
        }

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
                let scope_now = self.agent_scope.read().ok().and_then(|scope| scope.clone());
                let target = DriveTarget {
                    session_id: self.session_id.clone(),
                    workdir: self.workdir.clone(),
                    // 主体是打开时取的；带驱动的事实源必须有它（打开处已校验）。
                    agent_id: Some(self.agent_id.clone().ok_or_else(|| {
                        "会话主体不可用：带驱动的事实源缺少 agent_id".to_string()
                    })?),
                    mode: self.params.mode.clone(),
                    risk_level: self.params.risk_level.clone(),
                    provider_id: self.params.provider_id.clone(),
                    // 作用域由轮装配发布（见字段文档）；这里只取当前值，不解析。
                    agent_scope: scope_now,
                    // 来源主体由开轮写方发布（见字段文档）；`None` = 本机用户发起。
                    source_principal: self
                        .source_principal
                        .read()
                        .ok()
                        .and_then(|source| source.clone()),
                    // 中止信号只取**当前值**（共享格子，见字段文档）。
                    abort: self
                        .abort_signal
                        .read()
                        .ok()
                        .and_then(|signal| signal.clone()),
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
        // 只读开面不带驱动：运行参数没有消费方，取默认值即可（作用域槽为独立空置）。
        Ok(SessionFactSource::open(
            wal,
            session_id.to_string(),
            workdir,
            None,
            SessionParams::default(),
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(None)),
            // 只读开面没有在途轮次：中止槽独立空置（不是任何会话状态的那一格）。
            Arc::new(RwLock::new(None)),
            None,
            ExecEventSink::silent(),
            None,
        ))
    }
}

static SESSION_FACT_OPENER: SessionFactOpener = SessionFactOpener;

symbio_sdk::submit_fact_source_opener!(SESSION_FACT_OPENER);
