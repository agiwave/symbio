//! 进程内 Symbio 客户端。
//!
//! ## 设计要点
//!
//! CLI 与 Tauri 前端的差别只在「传输层」：Tauri 走 `route_v2` IPC，本 CLI 直接
//! 拿到 `Arc<dyn Plugin>` 根节点在**进程内**调用 `root.route(ctx)`。二者用的是
//! 同一套上下文键（PATH / PAYLOAD / SESSION_ID / WORKDIR / …），因此不需要任何
//! 协议改动 —— 换传输 = 换「请求 → SimpleRequest」这一层适配。
//!
//! ## 下行通道：两条，各归其域
//!
//! 与 Tauri 前端**同构**（前端 `services/transcriptStream.ts` + `stores/sessionNodeSync.ts`）：
//!
//! ```text
//! ① 消息实时面  session/stream            一条流、按会话归属、单调 seq、帧即一条消息
//! ② 会话运行态  event_bus + vdfs/watch    会话节点自身（status / attributes.outcome·error）
//! ```
//!
//! 分治的依据是**语义**而非实现：消息是高频突发的转写，与会话清单无关，归转写流；
//! 会话运行态是低频的**节点属性**，与侧栏会话清单共用同一份 VDFS 订阅
//! （`kind = "vdfs"`）。曾经「消息也寄生在 VDFS 变更上」——那条路有两个结构性缺陷：
//! VDFS 变更没有流内序号（丢一帧不可检测），且全量载荷要消费端自己猜「追加还是替换」。
//!
//! ```text
//! ① event_bus/subscribe        收件地址（一次订阅覆盖所有会话，切会话不必重连）
//! ② vdfs/watch <会话地址>      **开闸**：后端只向登记过路径的订阅者投递变更
//! ```
//!
//! 两步缺一不可：只做 ① 是一条永远不响的频道；只做 ② 则没有收件人。
//! 旧的 `kind = "session"` 事件频道（`Status` / `Update` / `Abort` 帧）已废除
//! ——会话运行态由**会话节点**（`status` + `attributes.outcome` / `.error`）承载
//! （见 `session/docs/node-state-streaming.md`）。
//!
//! 因此 `event_bus` 插件对本 CLI 而言**只是传输层**，不再是「会话事件的中转站」。

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
use symbio::symbio_core::transcript_stream::{
    event_of, is_resync, session_state_of, NodeEvent, SessionStateEvent,
};
use symbio::symbio_core::vdfs_provider::{
    VDFS_OUTCOME_ABORTED, VDFS_STATUS_FAILED, VDFS_STATUS_WORKING,
};
use symbio::symbio_core::{
    InvokeRequestExt, Plugin, PluginFrame, PluginPayload, SimpleRequest, PATH, PLUGIN_SESSION,
    SESSION_ID, WORKDIR,
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

/// 下行帧。**只有一条通道**：转写流。
///
/// 会话运行态与消息共用这条流、共用 `seq` 空间（见
/// `transcript_stream::SessionStateEvent`），因此调用侧不必 `select!` 两条连接，
/// 也不必做任何跨通道的顺序推理——「读到 `status != working`」本身就蕴含
/// 「本轮全部消息帧已在它之前落地」。
pub enum Frame {
    /// 一条转写事件（消息实时面）。
    ///
    /// 载荷装箱：`NodeEvent` 内联着整条 `ChatMessage`（约 300 字节），而本枚举经
    /// `mpsc` 逐帧搬运——装箱后枚举固定在一个指针量级，代价是每帧一次分配
    /// （相比该帧已走过的 JSON 反序列化，可忽略）。
    Transcript(Box<NodeEvent>),
    /// **会话运行态**（与消息同流、同 `seq` 空间）。本轮结束的唯一判据。
    ///
    /// 同上装箱：`SessionStateEvent` 内联着节点视图。
    Session(Box<SessionStateEvent>),
    /// 转写流背压标记：后端明示「你可能漏了帧」。唯一恢复路径是整份重读，
    /// 而 CLI 只做流式输出（已打印的正文不可回收）——留痕即可。
    Resync,
}

/// 转写流帧 → [`Frame`]；非本流帧返回 `None`。
///
/// 拆解交给 `transcript_stream` 的公共入口（与产帧方同模块，形状改了那边先响）。
/// **分派一律按信封的 `type`**，不靠「解不出消息帧就当状态帧」这种推断——
/// 那样一旦有第三种帧，分派就退化成猜。
///
/// 顺序不可颠倒：`event_of` / `session_state_of` 对背压帧必然返回 `None`，
/// 得靠 `is_resync` 把它认出来。
fn stream_frame_of(frame: PluginFrame) -> Option<Frame> {
    if let Some(ev) = event_of(&frame) {
        return Some(Frame::Transcript(Box::new(ev)));
    }
    if let Some(state) = session_state_of(&frame) {
        return Some(Frame::Session(Box::new(state)));
    }
    is_resync(&frame).then_some(Frame::Resync)
}

/// 流内序号闸门：**消息帧与会话运行态帧共用**（同一个 `seq` 空间）。
///
/// - `seq == last + 1`：连续，放行；
/// - `seq <= last`：重复帧（重连窗口内可能重发），丢弃——重复应用增量会叠字；
/// - `seq > last + 1`：**跳号 = 已知有损**，留痕后**仍放行**。CLI 没有历史可重读
///   （已打印的正文不可回收），丢掉这一帧只会更糟——它可能正是「本轮结束」的判据，
///   丢在那里就是永久卡在「处理中」。
///
/// 返回 `false` = 本帧应丢弃。
fn advance_seq(last: &mut Option<u64>, seq: u64, r: &mut Renderer) -> bool {
    if let Some(prev) = *last {
        if seq <= prev {
            return false;
        }
        if seq != prev + 1 {
            r.warn(&format!(
                "⚠ 转写流跳号（{prev} → {seq}），本轮输出可能不完整"
            ));
        }
    }
    *last = Some(seq);
    true
}

pub struct SymbioClient {
    root: Arc<dyn Plugin>,
    /// 转写流的下行帧（见 [`Frame`]）
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

        // **一条**下行通道：转写流（消息 + 会话运行态，共用 `seq` 空间）。
        //
        // 曾经这里是两条：`event_bus/subscribe` 收会话运行态、`session/stream` 收消息。
        // 两条的到达顺序没有机制保证，于是「会话报不忙」推不出「本轮消息都已终态」。
        // 现在运行态也在转写流上（见 `transcript_stream::SessionStateEvent`），
        // 因此 event_bus 订阅与 `vdfs/watch` 开闸动作**一并删除**——不是省略，
        // 是没有需要它们的地方了。
        //
        // 后端**广播**给全部订阅者，归属由帧里的 `session_id` 给出：订阅一次覆盖
        // 所有会话，切会话不必重连。
        let ctx = Arc::new(SimpleRequest::new(None, None));
        ctx.set(PATH, format!("{PLUGIN_SESSION}/stream"));
        let mut stream = match Arc::clone(&root)
            .route(ctx)
            .await
            .map_err(|e| format!("订阅转写流失败: {e}"))?
        {
            PluginPayload::Session(c) => c,
            other => return Err(format!("session/stream 返回了非会话载荷: {other:?}")),
        };

        // 一个转发任务：解包后送进无界通道，主逻辑只面对 [`Frame`]。
        let (tx, events) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(frame) = stream.rx.recv().await {
                let Some(f) = stream_frame_of(frame) else {
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
    /// 不需要「换闸门」——转写流是**全会话广播**（归属由帧里的 `session_id` 给出），
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
    /// ## 两类帧各司其职（**同一条流、同一个 `seq` 空间**）
    ///
    /// - [`Frame::Transcript`]（消息）：一帧就是一条消息，正文按**字段语义**
    ///   交渲染器——`delta` 追加、`content` 整条替换，不需要任何折算。
    /// - [`Frame::Session`]（会话运行态）：**本轮结束的唯一判据**，见下。
    ///
    /// 两者同流是**必要条件**：`Frame::Session` 说「不忙」时，所有 `seq` 更小的
    /// 消息帧（含本轮全部终态帧）必然已经交到渲染器手上——单通道保序给出的，
    /// 不是调度巧合。
    ///
    /// ## 为什么结束判据只能看会话节点
    ///
    /// 一轮请求里模型会多次定格根 Turn 节点（每个工具轮次一次），所以「Turn
    /// 到达终态」只说明**这一轮模型输出**结束，不说明整轮请求结束。真正的终态
    /// 在会话节点上：`status` 离开 `working` 即整轮结束，结局从 `attributes` 读
    /// ——`outcome == aborted` 是中止，`error` 非空是失败。
    ///
    /// 状态是节点的属性，而节点变更携带**全量节点视图**（幂等、与顺序无关），
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
        // 流内**单调 seq**：跳号 = 已知有损。CLI 只做流式输出（已打印的正文
        // 不可回收），因此恢复不了，只能留痕，让用户知道本轮输出不完整。
        // 消息与会话运行态**共用**这一个计数器（这正是顺序保证的来源）。
        let mut last_seq: Option<u64> = None;

        loop {
            let next = tokio::time::timeout(TURN_TIMEOUT, self.events.recv())
                .await
                .map_err(|_| "等待模型响应超时".to_string())?;
            let Some(frame) = next else {
                return Err("下行连接已关闭".to_string());
            };

            match frame {
                // ── 消息：一帧一条消息，直接落地 ──
                Frame::Transcript(ev) => {
                    // 转写流是**全会话广播**，本进程只渲染当前会话
                    if ev.session_id != self.session_id {
                        continue;
                    }
                    if !advance_seq(&mut last_seq, ev.seq, r) {
                        continue;
                    }

                    // 一帧就是一条消息：`delta` 追加 / `content` 整条替换 /
                    // `status = removed` 删除，全在 `on_message` 里按字段落地。
                    r.on_message(&ev.message);
                }
                // 后端明示「你可能漏了帧」：无历史可重读，只能留痕。
                Frame::Resync => {
                    r.warn("⚠ 转写流背压（后端已重同步），本轮输出可能不完整");
                }
                // ── 会话运行态：本轮结束的唯一判据 ──
                Frame::Session(state) => {
                    if state.session_id != self.session_id {
                        continue;
                    }
                    if !advance_seq(&mut last_seq, state.seq, r) {
                        continue;
                    }
                    let node = &state.node;
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
                    // 同一个 `seq` 空间给出的保证，不是调度巧合。
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
            }
        }

        r.end_turn();
        match business_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 读取下一条下行帧（心跳守护模式用）。
    ///
    /// 心跳触发的会话与普通对话走同一套编排与发布，守护进程在这里消费即可观察到
    /// 无人值守轮次。守护模式只关心会话运行态（[`Frame::Session`]），转写帧在此被
    /// 丢弃——但它**必须被取走**：不取就会把转写流的通道塞满，触发后端摘除订阅。
    /// 返回 `None` 表示下行连接已关闭。
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
