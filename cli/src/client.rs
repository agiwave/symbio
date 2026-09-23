//! 进程内 Symbio 客户端。
//!
//! ## 设计要点
//!
//! CLI 与 Tauri 前端的差别只在「传输层」：Tauri 走 `route_v2` IPC，本 CLI 直接
//! 拿到 `Arc<dyn Plugin>` 根节点在**进程内**调用 `root.route(ctx)`。二者用的是
//! 同一套上下文键（PATH / PAYLOAD / SESSION_ID / WORKDIR / …），因此不需要任何
//! 协议改动 —— 换传输 = 换「请求 → SimpleRequest」这一层适配。
//!
//! ## 下行通道：事件总线的 `vdfs` 频道
//!
//! 与 Tauri 前端**同构**（前端 `services/eventBus.ts` 的 `subscribe({ kind: 'vdfs' })`）：
//!
//! ```text
//! vdfs 频道   一条订阅、单一 FIFO、按路径归属
//!             消息（落点 = 目录 <sid>/消息，data = ChatMessage）
//!             会话运行态（落点 = 会话叶子 <sid>，data = VdfsNode 视图）
//!             资源信号（无载荷，回读收敛）
//! ```
//!
//! 归属由信封的 `path` 给出（`<根>/session/<sid>/…`）：订阅一次覆盖所有会话，
//! 切会话不必重连。单一 FIFO 给出顺序保证：后端在「清在途 → 复位 is_working」
//! **之后**才发运行态帧，「会话离开 working」因此蕴含本轮全部消息帧已在它之前
//! 落地——不需要流内序号，也不需要跨通道推理。
//!
//! **曾经这里是两条**：① `session/stream` 收消息、② `event_bus/subscribe` +
//! `vdfs/watch` 收会话运行态。两条的到达顺序没有机制保证，于是「会话报不忙」推不出
//! 「本轮消息都已终态」——前端因此挂了一张宽限期复查的兜底网。批次 E 把运行态并进
//! 转写流（`transcript_stream`）；ADR-025（2026-09-23）把实时面**迁回 VDFS 变更**，
//! `session/stream` 与 `transcript_stream` 一并退役，本 CLI 改订 `vdfs` 频道。
//! **批次 E 的合并理由（「需要顺序保证」）与 S23–S25 的拆分理由（「没有流内序号」）
//! 是同一个错误**：把「数据的属性」当成了「传输的属性」——顺序由单一订阅连接给，
//! 不由帧里的序号给。
//!
//! 信封没有操作枚举（S27）：形状 `{path, data?}`，语义全在 `data` 的字段上
//! （`delta` 追加 / `content` 替换 / `status = removed` 移除）。
//!
//! 旧的 `kind = "session"` 事件频道（`Status` / `Update` / `Abort` 帧）已废除
//! ——会话运行态由**会话节点**（`status` + `attributes.outcome` / `.error`）承载
//! （见 `session/docs/node-state-streaming.md`）。

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::sync::mpsc;

use symbio::init::create_root_plugin;
use symbio::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use symbio::symbio_core::schemas::session::session_chat;
use symbio::symbio_core::event_bus::{KIND_VDFS, RESYNC_MARKER_TYPE, SubscribeRequest};
use symbio::symbio_core::vdfs_provider::{
    vdfs_change_of, VdfsChange, VdfsNode, VDFS_OUTCOME_ABORTED, VDFS_STATUS_FAILED,
    VDFS_STATUS_WORKING,
};
use symbio::symbio_core::{
    InvokeRequestExt, Plugin, PluginFrame, PluginPayload, SimpleRequest, PATH, SESSION_ID, WORKDIR,
};

use crate::render::Renderer;

/// 单轮等待上限。取一个远大于真实模型耗时的值，只在「后端彻底无响应」时兜底失败。
const TURN_TIMEOUT: Duration = Duration::from_secs(900);

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 生成本进程内唯一的 id（会话 id / 用户消息 id 共用）。
fn gen_id(prefix: &str) -> String {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}{millis:x}{n:x}")
}

/// 消息目录段名。机制层的 `SEG_MESSAGES` 是 session 插件的 `pub(crate)` 内部
/// 词汇、不对外导出，CLI 按地址契约自持一份——拼错表现为「消息帧收不到」，
/// 不会静默错乱。
const MESSAGES_SEG: &str = "消息";

/// 下行帧。**只有一条通道**：事件总线的 `vdfs` 频道。
///
/// 消息 / 会话运行态 / 资源信号共用这条频道（见模块文档）；单一 FIFO 给出
/// 顺序保证，调用侧不必 `select!` 多条连接，也不必做任何跨通道的顺序推理。
pub enum Frame {
    /// 一条数据变更。会话归属由 `path` 给出（`<根>/session/<sid>/…`），
    /// 语义全在 `data` 的字段上（信封没有操作枚举，S27）。
    Change(Box<VdfsChange>),
    /// 背压标记：后端明示「你可能漏了变更」。唯一恢复路径是整份重读，
    /// 而 CLI 只做流式输出（已打印的正文不可回收）——留痕即可。
    Resync,
}

/// 总线帧 → [`Frame`]；非 `vdfs` 频道帧返回 `None`。
///
/// 变更解析交给 [`vdfs_change_of`]（与 [`VdfsChange`] 同模块：**形状改了那边先响**）。
/// resync 标记**不是一条变更**（刻意不带 `path`，`vdfs_change_of` 解不出），
/// 单独认——顺序不可颠倒：它靠这个「解不出」与真变更区分。
fn bus_frame_of(frame: PluginFrame) -> Option<Frame> {
    if let Some(c) = vdfs_change_of(&frame) {
        return Some(Frame::Change(Box::new(c)));
    }
    let is_resync = match &frame {
        PluginFrame::Data(v) => {
            let bus = v.get("data")?;
            bus.get("kind").and_then(Value::as_str) == Some(KIND_VDFS)
                && bus.get("data").and_then(|d| d.get("type")).and_then(Value::as_str)
                    == Some(RESYNC_MARKER_TYPE)
        }
        _ => false,
    };
    is_resync.then_some(Frame::Resync)
}

pub struct SymbioClient {
    root: Arc<dyn Plugin>,
    /// `vdfs` 频道的下行帧（见 [`Frame`]）
    events: mpsc::UnboundedReceiver<Frame>,
    /// 当前会话 id
    pub session_id: String,
    /// 会话工作目录（绝对路径字符串）
    pub workdir: String,
    /// Model Provider id；None = 用系统目录配置的默认 Provider
    pub provider: Option<String>,
    /// 会话运行模式
    pub mode: String,
    /// 可选 Agent 绑定
    pub agent: Option<String>,
    /// VDFS 根地址（启动期经 `vdfs/root` 取回；会话地址从它往下拼）
    root_addr: String,
}

impl SymbioClient {
    /// 启动进程内插件树并订阅事件总线。
    ///
    /// `homedir` 通过 `SYMBIO_HOMEDIR` 环境变量注入 —— 这是 [`HomedirRegistry`]
    /// 的最高优先级来源（高于 `~/.symbio_bootstrap` 与默认 `~/.symbio`），
    /// 因此必须在任何 `HomedirRegistry::get()` 之前设置。插件树构造期就会读它
    /// （home 读 `<homedir>/PLUGIN.yml`、session 派生存储目录），所以这里是"最早一刻"。
    pub async fn start(
        homedir: &Path,
        workdir: &Path,
        session: Option<String>,
        provider: Option<String>,
        mode: String,
        agent: Option<String>,
    ) -> Result<Self, String> {
        std::env::set_var("SYMBIO_HOMEDIR", homedir);
        std::fs::create_dir_all(homedir)
            .map_err(|e| format!("创建系统目录失败 {}: {e}", homedir.display()))?;

        let root = create_root_plugin().await;

        // **一条**下行通道：事件总线的 `vdfs` 频道（消息 + 会话运行态 + 资源信号）。
        //
        // 与前端同构（`services/eventBus.ts` 的 `subscribe({ kind: 'vdfs' })`）。
        // 单一订阅连接 = 单一 FIFO：后端在「清在途 → 复位 is_working」**之后**才发
        // 运行态帧，因此「会话离开 working」蕴含本轮全部消息帧已在它之前落地——
        // 顺序由通道给，不需要流内序号，也不需要跨通道推理。
        //
        // 后端**广播**给全部订阅者，归属由信封的 `path` 给出（`<根>/session/<sid>/…`）：
        // 订阅一次覆盖所有会话，切会话不必重连。
        let ctx = Arc::new(SimpleRequest::new(None, None));
        // 路由常量归 `symbio_core::paths`（模块对外私有），CLI 与其他外部使用方
        // 一样自持字面量——拼错表现为「订阅失败」，不会静默。
        ctx.set(PATH, "event_bus/subscribe".to_string());
        ctx.set_payload(SubscribeRequest {})
            .map_err(|e| format!("构造订阅载荷失败: {e}"))?;
        let mut stream = match Arc::clone(&root)
            .route(ctx)
            .await
            .map_err(|e| format!("订阅 vdfs 频道失败: {e}"))?
        {
            PluginPayload::Session(c) => c,
            other => return Err(format!("event_bus/subscribe 返回了非会话载荷: {other:?}")),
        };

        // 一个转发任务：解包后送进无界通道，主逻辑只面对 [`Frame`]。
        let (tx, events) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(frame) = stream.rx.recv().await {
                let Some(f) = bus_frame_of(frame) else {
                    continue;
                };
                if tx.send(f).is_err() {
                    break;
                }
            }
        });

        let workdir = workdir.to_string_lossy().to_string();
        let session_id = session.unwrap_or_else(|| gen_id("cli"));

        let mut client = Self {
            root,
            events,
            session_id,
            workdir,
            provider,
            mode,
            agent,
            root_addr: String::new(),
        };
        // 会话初始化要往 `<根>/session/<id>` 写，因此先把根地址取回来。
        // 根叫什么**归 vdfs 插件**（`fs::VDFS_ADDR_ROOT`），使用方不写死——
        // 与前端同款：启动期问一次，之后一律从父地址往下拼。
        client.root_addr = client.fetch_root_addr().await?;
        client.ensure_session().await?;
        Ok(client)
    }

    /// 取 VDFS 根地址（`vdfs/root` 是唯一「不给地址」的入口）。
    async fn fetch_root_addr(&self) -> Result<String, String> {
        let resp = self.route("vdfs/root", json!({}), None).await?;
        resp.get("path")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("vdfs/root 未返回根地址：{resp}"))
    }

    /// 路由一次调用。`payload` 走进程内强类型通道（零拷贝），
    /// 因此无需为这些请求类型实现 Serialize。
    async fn route<T: Clone + Send + Sync + 'static>(
        &self,
        path: &str,
        payload: T,
        sid: Option<&str>,
    ) -> Result<Value, String> {
        let ctx = Arc::new(SimpleRequest::new(None, None));
        ctx.set(PATH, path.to_string());
        ctx.set(WORKDIR, self.workdir.clone());
        if let Some(sid) = sid {
            ctx.set(SESSION_ID, sid.to_string());
        }
        ctx.set_payload(payload)
            .map_err(|e| format!("构造 {path} 载荷失败: {e}"))?;

        let resp = Arc::clone(&self.root)
            .route(ctx)
            .await
            .map_err(|e| format!("{path} 调用失败: {e}"))?;
        resp.get::<Value>()
            .map_err(|e| format!("{path} 响应解析失败: {e}"))
    }

    /// 把会话元数据写入存储（workdir / provider / mode / risk_level）。
    ///
    /// 这些字段是后端 `resolve_session_params` 的回退来源：会话一旦绑定，
    /// 后续每次发送都不必重复携带（前端也正是这么做的）。
    /// `--workdir` 等 REPL 内改动后需重新调用一次使其落库。
    ///
    /// ## 一次 `vdfs/write` 就是 upsert（旧 `session/update` 的全部职责）
    ///
    /// 写的是**具名目标** `<根>/session/<id>` 且带 `create` 意图，而 VDFS 对这两件事
    /// 的约定正好覆盖旧路由的两种情形（见 `VdfsProvider::write` 的 `create` 位表）：
    ///
    /// - 目标不存在 ⇒ **就地创建**，地址末段就是会话 id —— 这正是 CLI 需要的
    ///   「客户端指定会话 id」；
    /// - 目标已存在 ⇒ 覆盖（浅合并 metadata），`created = false`。
    ///
    /// 因此「新建会话」与「改元数据」是**同一次调用**，不需要先 `stat` 再决定
    /// 写还是建——那条多出来的往返（以及随之而来的竞态）正是旧路由存在的理由，
    /// 现在由 VDFS 的通用语义承担，专用路由已整体退役。
    ///
    /// `switch_session` 因此也不必区分「切到已有」与「切到新的」：改完 id 重新调用
    /// 本方法即可（`/session <ID>` 与 `--session <ID>` 都是「打开或新建」）。
    pub async fn ensure_session(&self) -> Result<(), String> {
        let mut metadata = serde_json::Map::new();
        metadata.insert("workdir".to_string(), json!(self.workdir));
        metadata.insert("mode".to_string(), json!(self.mode));
        metadata.insert("risk_level".to_string(), json!("medium"));
        if let Some(p) = &self.provider {
            metadata.insert("provider_id".to_string(), json!(p));
        }
        if let Some(a) = &self.agent {
            metadata.insert("agent_id".to_string(), json!(a));
        }

        let addr = format!(
            "{}/session/{}",
            self.root_addr.trim_end_matches('/'),
            self.session_id
        );
        let resp = self
            .route(
                "vdfs/write",
                json!({
                    "path": addr,
                    "create": true,
                    // 内容体是会话写入的两种字段之一（`metadata` / `title`）——
                    // 与前端选项栏、后端 provider 同一种信封形状
                    "text": json!({ "metadata": Value::Object(metadata) }).to_string(),
                }),
                None,
            )
            .await?;
        // 失败经数据载荷回传（`{"error": …}`），不表现为传输错误——照实报出来，
        // 否则会退化成「响应解析失败」这种看不出原因的消息。
        if let Some(err) = resp.get("error").and_then(Value::as_str) {
            if !err.is_empty() {
                return Err(format!("会话初始化未成功: {err}"));
            }
        }
        Ok(())
    }

    /// 切换会话：更新 id 并重新写入元数据。
    ///
    /// 不需要「换闸门」——订阅是**全会话广播**（归属由信封的 `path` 给出），
    /// 一次订阅覆盖所有会话。切会话因此只剩「改 id + 落元数据」两件事。
    pub async fn switch_session(&mut self, session_id: String) -> Result<(), String> {
        self.session_id = session_id;
        self.ensure_session().await
    }

    /// 切换 Provider（下次发送即生效，无需重建连接）。
    pub fn switch_provider(&mut self, provider: Option<String>) {
        self.provider = provider;
    }

    /// 丢弃本轮开始前残留的事件，避免上一轮的补丁被重复渲染。
    fn drain_stale(&mut self) {
        while self.events.try_recv().is_ok() {}
    }

    /// 发送一条用户消息，并把本轮响应流实时渲染到终端。
    ///
    /// ## 两类变更各司其职（同一条订阅、单一 FIFO）
    ///
    /// - **消息**（落点 = 消息目录 `<sid>/消息`）：`data` 就是那条
    ///   [`ChatMessage`]，正文按**字段语义**交渲染器——`delta` 追加、
    ///   `content` 整条替换、`status = removed` 删除，不需要任何折算。
    /// - **会话运行态**（落点 = 会话叶子 `<sid>`）：**本轮结束的唯一判据**，见下。
    ///
    /// 同一条订阅是**必要条件**：订阅连接是单一 FIFO，后端在「清在途 → 复位
    /// `is_working`」**之后**才发运行态帧，因此「会话离开 working」蕴含本轮
    /// 全部消息帧已交到渲染器手上——顺序是通道给的，不是调度巧合。
    ///
    /// ## 为什么结束判据只能看会话节点
    ///
    /// 一轮请求里模型会多次定格根 Turn 节点（每个工具轮次一次），所以「Turn
    /// 到达终态」只说明**这一轮模型输出**结束，不说明整轮请求结束。真正的终态
    /// 在会话节点上：`status` 离开 `working` 即整轮结束，结局从 `attributes` 读
    /// ——`outcome == aborted` 是中止，`error` 非空是失败。
    ///
    /// 状态是节点的属性，而运行态变更携带**全量节点视图**（幂等、与顺序无关），
    /// 因此丢一帧不会让 CLI 卡在「处理中」——下一帧就把它纠了回来。
        pub async fn ask(&mut self, text: &str, r: &mut Renderer) -> Result<(), String> {
        self.drain_stale();
        r.begin_turn();

        let req = session_chat::Request {
            session_id: Some(self.session_id.clone()),
            agent_id: self.agent.clone(),
            message: Some(ChatMessage {
                id: gen_id("u"),
                role: Some(MessageRole::User),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text(text.to_string())),
                ..Default::default()
            }),
            // 随请求携带 provider/mode：即便会话元数据被外部改写，本次发送依然生效
            provider_id: self.provider.clone(),
            include_history: None,
            mode: Some(self.mode.clone()),
            risk_level: Some("medium".to_string()),
            resume: None,
        };

        let resp = self
            .route("session/chat/send", req, Some(&self.session_id))
            .await?;
        match resp.get("status").and_then(Value::as_str) {
            Some("accepted") => {}
            Some(other) => return Err(format!("发送未被接受（{other}）: {resp}")),
            None => return Err(format!("发送响应异常: {resp}")),
        }

        let mut business_error: Option<String> = None;
        // 上一帧的会话运行态：状态提示只在**迁移**上报（否则一次轮次要喊两遍「处理中」）
        let mut last_status: Option<String> = None;

        loop {
            let next = tokio::time::timeout(TURN_TIMEOUT, self.events.recv())
                .await
                .map_err(|_| "等待模型响应超时".to_string())?;
            let Some(frame) = next else {
                return Err("下行连接已关闭".to_string());
            };

            match frame {
                // 后端明示「你可能漏了变更」：无历史可重读，只能留痕。
                Frame::Resync => {
                    r.warn("⚠ vdfs 频道背压（后端已重同步），本轮输出可能不完整");
                }
                Frame::Change(change) => {
                    // ── 会话运行态：本轮结束的唯一判据 ──
                    if let Some((sid, node)) = self.session_state_of_change(&change).await {
                        if sid != self.session_id {
                            continue;
                        }
                        if node.status == VDFS_STATUS_WORKING {
                            // 只在**迁移**上报一次：运行态是节点属性，同一次「开始工作」
                            // 可能因告警等原因重复下发同一份视图（视图幂等，重复无害）。
                            if last_status.as_deref() != Some(node.status.as_str()) {
                                r.on_status(&node.status);
                            }
                            last_status = Some(node.status.clone());
                            continue;
                        }
                        // 离开 `working` = 本轮结束（中止 / 失败 / 正常收尾三种结局之一，
                        // 由 `outcome` 区分）。此刻本轮全部消息帧**已在它之前落地**——
                        // 单一订阅 FIFO 给出的保证，不是调度巧合。
                        if node.status == VDFS_STATUS_FAILED {
                            business_error = node
                                .attributes
                                .get("error")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .or_else(|| Some("本轮以错误结束".to_string()));
                        }
                        if node.attributes.get("outcome").and_then(Value::as_str)
                            == Some(VDFS_OUTCOME_ABORTED)
                        {
                            r.on_abort();
                        }
                        break;
                    }
                    // ── 消息：`data` 就是那条 ChatMessage，按字段落地 ──
                    if let Some((sid, message)) = Self::message_of_change(&change) {
                        if sid == self.session_id {
                            // `delta` 追加 / `content` 整条替换 / `status = removed`
                            // 删除，全在 `on_message` 里按字段落地。
                            r.on_message(&message);
                        }
                    }
                    // 其余资源信号（记忆 / 工作目录 / 插件…）与本轮渲染无关，丢弃。
                }
            }
        }

        r.end_turn();
        match business_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 从一条变更中解出**会话运行态**：`(sid, node)`。
    ///
    /// 落点判定：`path` 恰为 `<根>/session/<sid>`（会话叶子自身，后面没有更多段）。
    /// `data` 是全量节点视图（与 `stat` 同源构造）时零回读；缺失 ⇒ 回读
    /// `vdfs/stat` 分辨——返回 `None` 也涵盖「会话已删」（stat 无此节点）。
    pub async fn session_state_of_change(&self, change: &VdfsChange) -> Option<(String, VdfsNode)> {
        let prefix = format!("{}/session/", self.root_addr.trim_end_matches('/'));
        let rest = change.path.strip_prefix(&prefix)?;
        let sid = rest.split('/').next()?;
        if sid.is_empty() || rest.contains('/') {
            return None; // 更深层级（消息目录等）不是会话叶子
        }
        let node = match &change.data {
            Some(v) => serde_json::from_value::<VdfsNode>(v.clone()).ok()?,
            None => self.stat_node(&change.path).await?,
        };
        Some((sid.to_string(), node))
    }

    /// 从一条变更中解出**消息**：`(sid, ChatMessage)`。
    ///
    /// 落点判定：`path` 是消息**目录** `<根>/session/<sid>/消息`（信封的 `path`
    /// = 变更文件所在的**目录**），具体是哪条消息由 `data.id` 回答——对象身份
    /// 在载荷里，不在路径上。无载荷 / 解析失败 ⇒ `None`（资源信号不归消息面管）。
    pub fn message_of_change(change: &VdfsChange) -> Option<(String, ChatMessage)> {
        let rest = change.path.split_once("/session/")?.1;
        let mut segs = rest.split('/');
        let sid = segs.next()?;
        if sid.is_empty() || segs.next()? != MESSAGES_SEG || segs.next().is_some() {
            return None;
        }
        let message = serde_json::from_value::<ChatMessage>(change.data.clone()?).ok()?;
        Some((sid.to_string(), message))
    }

    /// 回读一个节点的当前视图（`vdfs/stat`）。
    async fn stat_node(&self, addr: &str) -> Option<VdfsNode> {
        let resp = self
            .route("vdfs/stat", json!({ "path": addr }), None)
            .await
            .ok()?;
        serde_json::from_value::<VdfsNode>(resp).ok()
    }

    /// 读取下一条下行帧（心跳守护模式用）。
    ///
    /// 心跳触发的会话与普通对话走同一套编排与发布，守护进程在这里消费即可观察到
    /// 无人值守轮次。守护模式只关心会话运行态（会话叶子上的变更），消息帧在此被
    /// 丢弃——但它**必须被取走**：不取就会把订阅通道塞满（满了后端补 resync，
    /// 代价是消费端整份重读）。返回 `None` 表示下行连接已关闭。
        pub async fn next_frame(&mut self) -> Option<Frame> {
        self.events.recv().await
    }

    /// 查询当前 Provider（用于 REPL 的 `/provider` 状态显示）。
    pub fn provider_label(&self) -> String {
        self.provider
            .clone()
            .unwrap_or_else(|| "default（系统目录配置）".to_string())
    }
}
