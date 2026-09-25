//! 进程内 Symbio 客户端。
//!
//! ## 设计要点
//!
//! CLI 与 Tauri 前端的差别只在「传输层」：Tauri 走 `route_v2` IPC，本 CLI 直接
//! 拿到 `Arc<dyn Plugin>` 根节点在**进程内**调用 `root.route(ctx)`。二者用的是
//! 同一套上下文键（PATH / payload / SESSION_ID / WORKDIR / …），因此不需要任何
//! 协议改动 —— 换传输 = 换「请求 → PluginSimpleRequest」这一层适配。
//!
//! ## 下行通道：事件总线的 `vdfs` 频道
//!
//! 与 Tauri 前端**同构**（前端 `services/eventBus.ts` 的 `subscribe({ kind: 'vdfs' })`）：
//!
//! ```text
//! vdfs 频道   一条订阅、单一 FIFO、按路径归属
//!             消息（落点 = 节点自身 <sid>/message/<mid>，data = ChatMessage）
//!             会话运行态（落点 = 会话叶子 <sid>，data = VdfsNode 视图）
//!             资源信号（无载荷：标题 / metadata 等，与本轮渲染无关，丢弃）
//! ```
//!
//! 归属由信封的 `path` 给出（`<根>/session/<sid>/…`）：订阅一次覆盖所有会话，
//! 切会话不必重连。单一 FIFO 给出顺序保证：后端在「清在途 → 复位 is_working」
//! **之后**才发运行态帧，「会话离开 working」因此蕴含本轮全部消息帧已在它之前
//! 落地——不需要流内序号，也不需要跨通道推理。
//!
//! **这条推理曾经算不出来**：消息与会话运行态分属两条通道时，两者的到达顺序没有
//! 机制保证，「会话报不忙」推不出「本轮消息都已终态」，前端因此挂过一张宽限期复查
//! 的兜底网。根因是把「数据的属性」当成了「传输的属性」——顺序由单一订阅连接给出，
//! 不由帧里的序号给出（ADR-025）。
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
use symbio::symbio_core::event_bus::{
    EventBusSubscribeRequest, EVENT_BUS_KIND_VDFS, EVENT_BUS_RESYNC_MARKER_TYPE,
};
use symbio::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use symbio::symbio_core::schemas::session::session_chat;
use symbio::symbio_core::{
    vdfs_change_of, VdfsChange, VdfsNode, VDFS_STATUS_FAILED, VDFS_STATUS_WORKING,
};
use symbio::symbio_core::{
    Plugin, PluginFrame, PluginInvokeRequestExt, PluginPayload, PluginSimpleRequest,
    EVENT_BUS_SUBSCRIBE, PATH, SESSION_ID, WORKDIR,
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

/// 会话域的**挂载段名**（ASCII）。机制层的 `PLUGIN_SESSION` 不在 CLI 的可见面上，
/// CLI 按地址契约自持一份——与 [`MESSAGES_SEG`] 同一理由、同一约定。
///
/// 三处用到它（订阅 `<根>/session`、识别会话节点 `<根>/session/<sid>`、
/// 识别消息节点 `<根>/session/<sid>/message/<mid>`），因此**只在这里写一次**。
const SESSION_SEG: &str = "session";

/// 消息集合的**路径段**（ASCII）。机制层的 `SEG_MESSAGES` 是 session 插件的
/// `pub(crate)` 内部词汇、不对外导出，CLI 按地址契约自持一份——拼错表现为
/// 「消息帧收不到」，不会静默错乱。
///
/// 段名是 ASCII、展示名是中文（`title`），两者不是一回事：地址要能安全地进
/// URL / 命令行 / 日志，中文只出现在 UI 上。
const MESSAGES_SEG: &str = "message";

/// 会话运行态中「本轮被中止」的 `outcome` 取值。机制层的 `OUTCOME_ABORTED` 是
/// session 插件的内部词汇、不在 CLI 的可见面上，CLI 按地址契约自持一份——拼错
/// 表现为「中止被当成正常收尾」，不会静默错乱（与 [`SESSION_SEG`] 同一理由）。
pub(crate) const SESSION_OUTCOME_ABORTED: &str = "aborted";

/// 留痕开关（`SYMBIO_ROUTE_LOG=1`）：出站 `route` 与**入站变更帧**都打到 stderr。
///
/// CLI 走**进程内**路由，没有 Tauri 那层的 `Start routing`
/// （`tauri/src-tauri/src/commands.rs`），于是「一次会话到底发了多少请求」在 CLI
/// 上无处可查——排查多余 IPC 只能靠它。与 `SYMBIO_LOG` 同一套约定：默认关闭，
/// 逐行输出走 stderr（与 stdout 的正文分流，`2>&1 | grep '^\[route\]'` 即可统计）。
///
/// 入站帧（`[frame] path=… data=…`）同样留痕，因为「一条帧都没到」与「帧到了但没被
/// 认出来」在 CLI 上表现**完全一样**（都是正文为空 + 卡到超时），而两者的修法相反：
/// 前者查订阅有没有装上，后者查地址判定。区分它们只能靠这行日志。
fn route_log_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("SYMBIO_ROUTE_LOG")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false)
    })
}

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
            bus.get("kind").and_then(Value::as_str) == Some(EVENT_BUS_KIND_VDFS)
                && bus
                    .get("data")
                    .and_then(|d| d.get("type"))
                    .and_then(Value::as_str)
                    == Some(EVENT_BUS_RESYNC_MARKER_TYPE)
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
    /// `homedir` 通过 `SYMBIO_HOMEDIR` 环境变量注入 —— 这是 `home` 插件的
    /// `HomedirRegistry` 的最高优先级来源（高于 `~/.symbio_bootstrap` 与默认
    /// `~/.symbio`），因此必须在 `home` 首次读取它之前设置。插件树构造期就会读它
    /// （home 读 `<homedir>/PLUGIN.yml`，并把目录经 `PLUGIN_DIR` 传给子插件），
    /// 所以这里是"最早一刻"。
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
        let ctx = Arc::new(PluginSimpleRequest::new(None, None));
        // 绝对地址**取常量**，不写字面量：`set(PATH, "<字面量>")` 会让「路由改名」
        // 不产生任何编译错误，只在运行期表现为「订阅失败」——而失败的样子与
        // 「后端没发布」完全一样。这条由 `scripts/plugin-entry-audit.mjs` 的 E-003
        // 判定型守卫强制。
        //
        // 常量从 `symbio_core` **顶层**导入（`use symbio::symbio_core::EVENT_BUS_SUBSCRIBE`），
        // 而不是 `symbio_core::keys::paths::…`：`keys` 域自身是私有的（`mod keys;`），
        // 常量靠 `symbio_core/mod.rs` 的 `pub use keys::*;` 才对外可见。
        // 这条路径写错过一次，症状是编译期的 E0603（`module 'keys' is private`）——
        // 好在它是**编译期**失败，不会静默。
        ctx.set(PATH, EVENT_BUS_SUBSCRIBE.to_string());
        ctx.set_payload(EventBusSubscribeRequest {})
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
        client.watch_session_changes().await?;
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

    /// 订阅会话域的变更（`vdfs/watch`）——**下行帧能到达的前提**。
    ///
    /// ## 为什么 `event_bus/subscribe` 还不够（这里踩过，且症状极具误导性）
    ///
    /// 事件总线只是**广播口**：`EventBus::try_publish(EVENT_BUS_KIND_VDFS, …)` 只投给已注册
    /// 的总线订阅者，而**谁来 publish** 取决于 VDFS 那一侧有没有 sink。sink 由
    /// `vdfs/watch` 登记进 provider 的变更表（`ChangeSubscriptions::watch`）——
    /// 没有 watch 就没有 sink，也就没有任何帧会上总线。
    ///
    /// S27 把两条下行通道并成一条（`session/stream` 退役、会话实时面迁到 `vdfs`
    /// 频道）时，这里保留了 `event_bus/subscribe` 却**漏掉了 `vdfs/watch`**。
    /// 后果不是报错，而是**一条变更都收不到**：正文不渲染（stdout 为空）、
    /// 会话运行态永远停在「处理中」，一路卡到超时——与「模型没有产出内容」
    /// 的症状逐字相同，于是排查方向被引到 provider / 模型链路上。
    ///
    /// 判据：`SYMBIO_ROUTE_LOG=1` 下若只有出站请求、**没有任何 `[frame]` 行**，
    /// 就是这里没装上（帧到了但没认出来会有 `[frame]` 行）。
    ///
    /// 订阅 `<根>/session` 而不是根：与前端同一粒度（`prefix: mountDir`），
    /// 覆盖全部会话，切会话不必重连（归属由信封的 `path` 给出）。
    async fn watch_session_changes(&self) -> Result<(), String> {
        let addr = format!("{}/{SESSION_SEG}", self.root_addr.trim_end_matches('/'));
        // 路由字面量：`VDFS_WATCH` 常量是前端在用的那份，Rust 侧自持字面量
        // （与上面 `EVENT_BUS_SUBSCRIBE` 取常量的理由不同：这里是**插件内的相对臂
        // 消费方**，写错表现为 `vdfs/watch` 路由不存在 → 订阅失败，不会静默）。
        self.route("vdfs/watch", json!({ "path": addr }), None)
            .await?;
        Ok(())
    }

    /// 路由一次调用。`payload` 走进程内强类型通道（零拷贝），
    /// 因此无需为这些请求类型实现 Serialize。
    async fn route<T: Clone + Send + Sync + 'static>(
        &self,
        path: &str,
        payload: T,
        sid: Option<&str>,
    ) -> Result<Value, String> {
        if route_log_enabled() {
            eprintln!("[route] {path}");
        }
        let ctx = Arc::new(PluginSimpleRequest::new(None, None));
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
    /// 会话是**容器**，其下是若干**并列的集合**（消息 / 子会话 / 记忆 / 工作目录，
    /// 后续还会有任务列表、请求队列……），地址形状统一为 `<sid>/<集合段>/<项 id>`，
    /// **身份就是地址末段**：
    ///
    /// - **消息**（落点 = 那条消息节点自身 `<sid>/message/<mid>`）：`data` 就是那条
    ///   [`ChatMessage`]，正文按**字段语义**交渲染器——`delta` 追加、
    ///   `content` 整条替换、`status = removed` 删除，不需要任何折算。
    /// - **会话运行态**（落点 = 会话节点自身 `<sid>`）：**本轮结束的唯一判据**，见下。
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

        // 本轮的业务错误（失败结局的 `error`）。**不预置初值**：循环的每个出口都在
        // 赋值之后 `break`，预置一个 `None` 只会是"赋值了但没人读"的死写。
        let business_error: Option<String>;
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
                // 后端明示「你可能漏了变更」。
                //
                // 消息正文已打印、不可回收，但**运行态可恢复**——它是一次 `stat`
                // 就能取回的当前值。少了这一步，「漏掉收尾帧」就退化成静默挂起：
                // 会话早已结束，CLI 却一直等到 `TURN_TIMEOUT` 才报错。
                // 回读一次是**有代价的**，因此它只挂在「后端明示漏帧」这个信号上，
                // 而不是挂在每一帧无载荷的会话变更上（见 `session_state_of_change`）。
                Frame::Resync => {
                    r.warn("⚠ vdfs 频道背压（后端已重同步），本轮输出可能不完整");
                    if let Some(node) = self.stat_node(&self.session_addr()).await {
                        if let Some(outcome) = Self::turn_end_of(&node) {
                            if node.attributes.get("outcome").and_then(Value::as_str)
                                == Some(SESSION_OUTCOME_ABORTED)
                            {
                                r.on_abort();
                            }
                            business_error = outcome.err();
                            break;
                        }
                    }
                }
                Frame::Change(change) => {
                    if route_log_enabled() {
                        eprintln!(
                            "[frame] path={:?} data={}",
                            change.path,
                            if change.data.is_some() {
                                "some"
                            } else {
                                "none"
                            }
                        );
                    }
                    // ── 会话运行态：本轮结束的唯一判据 ──
                    // 归属过滤在 `session_state_of_change` 里按地址完成（`scope`）。
                    if let Some((_sid, node)) = self
                        .session_state_of_change(&change, Some(&self.session_id))
                        .await
                    {
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
                        if node.attributes.get("outcome").and_then(Value::as_str)
                            == Some(SESSION_OUTCOME_ABORTED)
                        {
                            r.on_abort();
                        }
                        business_error = Self::turn_end_of(&node).and_then(Result::err);
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
    /// 落点判定：`path` 恰为 `<根>/session/<sid>`（会话节点自身，后面没有更多段）。
    /// 集合项（`<sid>/<集合段>/<项 id>`）与集合目录都不算——会话节点与它下面的
    /// 东西是两类地址。
    ///
    /// `scope` 决定**认哪些会话**，且这一步只看地址、不看载荷：
    /// - `Some(sid)` ⇒ 只要这一个。不归我的帧**零成本丢弃**。
    /// - `None` ⇒ 认任何会话（心跳守护模式要观察所有会话）。
    ///
    /// ## 无载荷 ⇒ 直接丢弃（这里曾经回读一次 `stat`）
    ///
    /// 运行态变更**恒带**全量节点视图（与 `stat` 同源构造），所以「无载荷」只有
    /// 两种来源，而它们都**不需要**回读：
    ///
    /// | 无载荷的来源 | 回读的结果 |
    /// |---|---|
    /// | 资源信号（标题 / metadata / 创建，见 `plugin::notify_change`） | 会话还在，`status` 没变 ⇒ 空转 |
    /// | `emit_session_state` 取不到视图（**唯一原因**：会话已不在） | `stat` 同样 `NotFound` ⇒ 仍然丢弃 |
    ///
    /// 也就是说：那次回读**永远改变不了结论**，却是一笔真实请求——实测每会话一次
    /// （首个用户消息落盘后的自动命名），心跳守护模式下还是「每个会话各一次」。
    /// 兜底改挂在 [`Frame::Resync`] 上：后端**明示**可能漏帧时才回读一次，那才是
    /// 真正需要「重读作用域」的时刻。
    pub async fn session_state_of_change(
        &self,
        change: &VdfsChange,
        scope: Option<&str>,
    ) -> Option<(String, VdfsNode)> {
        let prefix = format!("{}/{SESSION_SEG}/", self.root_addr.trim_end_matches('/'));
        let rest = change.path.strip_prefix(&prefix)?;
        let mut segs = rest.split('/');
        let sid = segs.next()?;
        if sid.is_empty() || segs.next().is_some() {
            return None; // 更深层级（集合目录 / 集合项）不是会话节点自身
        }
        if scope.is_some_and(|want| want != sid) {
            return None; // 不归我：地址已足够回答
        }
        let node = serde_json::from_value::<VdfsNode>(change.data.clone()?).ok()?;
        Some((sid.to_string(), node))
    }

    /// 会话节点自身的地址（`<根>/session/<sid>`）。
    fn session_addr(&self) -> String {
        format!(
            "{}/{SESSION_SEG}/{}",
            self.root_addr.trim_end_matches('/'),
            self.session_id
        )
    }

    /// 会话节点视图 → **本轮是否结束**。
    ///
    /// - `None` ⇒ 仍在 `working`（继续等）；
    /// - `Some(Ok(()))` ⇒ 正常 / 中止收尾；
    /// - `Some(Err(e))` ⇒ 以错误结束（`e` 取节点上的 `error`，缺省给一句通用说明）。
    ///
    /// 收成一处是因为它有**两个**触发点：实时面上的收尾帧，与 `Frame::Resync`
    /// 之后的那次回读。两处各写一遍「什么算结束」正是最容易分叉的写法。
    fn turn_end_of(node: &VdfsNode) -> Option<Result<(), String>> {
        if node.status == VDFS_STATUS_WORKING {
            return None;
        }
        if node.status != VDFS_STATUS_FAILED {
            return Some(Ok(()));
        }
        Some(Err(node
            .attributes
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| "本轮以错误结束".to_string())))
    }

    /// 从一条变更中解出**消息**：`(sid, ChatMessage)`。
    ///
    /// 落点判定：`path` 是那条消息**节点自身** `<根>/session/<sid>/message/<mid>`——
    /// 形状 `<sid>/<集合段>/<项 id>`，**身份在地址末段**。早先落点是消息**目录**
    /// 而身份在 `data.id`：那样 `path` 的含义随帧类型漂移，消费端必须反推地址，
    /// 且无法推广到第二类集合。无载荷 / 解析失败 ⇒ `None`（资源信号不归消息面管）。
    pub fn message_of_change(change: &VdfsChange) -> Option<(String, ChatMessage)> {
        let marker = format!("/{SESSION_SEG}/");
        let rest = change.path.split_once(marker.as_str())?.1;
        let mut segs = rest.split('/');
        let sid = segs.next()?;
        if sid.is_empty() || segs.next()? != MESSAGES_SEG {
            return None;
        }
        // 身份在地址末段：恰为 `<sid>/message/<mid>` 三段，不能再深。
        let mid = segs.next()?;
        if mid.is_empty() || segs.next().is_some() {
            return None;
        }
        let mut message = serde_json::from_value::<ChatMessage>(change.data.clone()?).ok()?;
        // 地址即身份（载荷里的 `id` 只是同一事实的复述，以地址为准）。
        message.id = mid.to_string();
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
