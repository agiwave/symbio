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
//! ① 消息实时面  session/stream            一条流、按会话归属、单调 seq、显式操作（NodeOp）
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
use symbio::symbio_core::event_bus::{SubscribeRequest, KIND_VDFS};
use symbio::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use symbio::symbio_core::schemas::session::session_chat;
use symbio::symbio_core::schemas::session::session_chat_response::NodeOp;
use symbio::symbio_core::schemas::session::session_update;
use symbio::symbio_core::transcript_stream::NodeEvent;
use symbio::symbio_core::vdfs_provider::{
    VdfsChange, VDFS_OUTCOME_ABORTED, VDFS_STATUS_FAILED, VDFS_STATUS_WORKING,
};
use symbio::symbio_core::{
    InvokeRequestExt, Plugin, PluginFrame, PluginPayload, SimpleRequest, EVENT_BUS_SUBSCRIBE, PATH,
    PLUGIN_SESSION, SESSION_ID, VDFS_ROOT, VDFS_UNWATCH, VDFS_WATCH, WORKDIR,
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

/// 前端侧统一下行帧。
///
/// 两条通道（转写流 / 事件总线）的转发任务都往**同一个**接收端投递，因此调用侧
/// 只面对一个 `Frame`，不必 `select!`、也不必知道它来自哪条连接。
pub enum Frame {
    /// 一条转写事件（消息实时面，`session/stream`）。
    Transcript(NodeEvent),
    /// 转写流背压标记：后端明示「你可能漏了帧」。唯一恢复路径是整份重读，
    /// 而 CLI 只做流式输出（已打印的正文不可回收）——留痕即可。
    Resync,
    /// 会话节点运行态变更（VDFS watch 域）。
    ///
    /// 载荷装箱：`VdfsChange` 内联着节点视图（约 400 字节），而同一枚举的
    /// `Transcript` 是**每 token 一帧**的热路径。装箱后每帧固定 80 字节量级，
    /// 代价只是低频运行态帧上的一次分配。
    Node(Box<VdfsChange>),
}

/// 事件总线帧 → `Frame::Node`；非 `kind = "vdfs"` 的帧返回 `None`。
///
/// 信封里的 `session_id` 是死字段——会话身份在 VDFS 变更的**地址**里
/// （`VdfsChange::path`），而 vdfs 插件发帧时本来就不填它
/// （`plugins/vdfs/host.rs::event_bus_sink`）。
///
/// `VdfsChange` 是 core 类型：进程内消费者（`plugins/agent/host/subagent.rs`
/// 与这里）都按它读，而 vdfs 插件的线路信封刻意留在插件内部。
fn node_frame_of(frame: PluginFrame) -> Option<Frame> {
    let PluginFrame::Data(v) = frame else {
        return None;
    };
    let inner = v.get("data")?;
    if inner.get("kind")?.as_str()? != KIND_VDFS {
        return None;
    }
    serde_json::from_value::<VdfsChange>(inner.get("data")?.clone())
        .ok()
        .map(|change| Frame::Node(Box::new(change)))
}

/// 转写流帧 → [`Frame`]；非本流帧返回 `None`。
///
/// 按信封的 `type` 分派（与后端 `transcript_stream::publish_frame` 逐字对应）：
/// 数据帧 `transcript_event` 解出 [`NodeEvent`]，背压帧 `transcript_resync` 无载荷。
fn transcript_frame_of(frame: PluginFrame) -> Option<Frame> {
    let PluginFrame::Data(v) = frame else {
        return None;
    };
    match v.get("type")?.as_str()? {
        "transcript_event" => serde_json::from_value::<NodeEvent>(v.get("data")?.clone())
            .ok()
            .map(Frame::Transcript),
        "transcript_resync" => Some(Frame::Resync),
        _ => None,
    }
}

pub struct SymbioClient {
    root: Arc<dyn Plugin>,
    /// 两条下行通道汇入的统一下行帧（见 [`Frame`]）
    events: mpsc::UnboundedReceiver<Frame>,
    /// 当前会话 id
    pub session_id: String,
    /// 当前会话在 VDFS 上的地址（`<根>/session/<sid>`）——变更按它过滤
    session_addr: String,
    /// 会话工作目录（绝对路径字符串）
    pub workdir: String,
    /// Model Provider id；None = 用系统目录配置的默认 Provider
    pub provider: Option<String>,
    /// 会话运行模式
    pub mode: String,
    /// 可选 Agent 绑定
    pub agent: Option<String>,
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

        // ① 会话运行态：event_bus 收件地址（一次订阅覆盖所有会话，切会话无需重连）。
        //    路径取 `symbio_core::paths` 常量——调用侧不写字面量（见该模块「地址规则」）。
        let ctx = Arc::new(SimpleRequest::new(None, None));
        ctx.set(PATH, EVENT_BUS_SUBSCRIBE.to_string());
        ctx.set_payload(SubscribeRequest { kinds: None })
            .map_err(|e| format!("设置订阅载荷失败: {e}"))?;
        let mut bus = match Arc::clone(&root)
            .route(ctx)
            .await
            .map_err(|e| format!("订阅事件总线失败: {e}"))?
        {
            PluginPayload::Session(c) => c,
            other => return Err(format!("event_bus/subscribe 返回了非会话载荷: {other:?}")),
        };

        // ② 消息实时面：转写流。后端**广播**给全部订阅者，归属由帧里的 `session_id`
        //    给出，因此与 event_bus 同款性质——订阅一次覆盖所有会话，切会话不重连。
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

        // 两条连接各起一个转发任务，把帧解包后送进**同一个**无界通道：
        // 主逻辑因此只面对 [`Frame`]，不必 select、也不必关心连接细节。
        let (tx, events) = mpsc::unbounded_channel();
        let tx_bus = tx.clone();
        tokio::spawn(async move {
            while let Some(frame) = bus.rx.recv().await {
                let Some(f) = node_frame_of(frame) else {
                    continue;
                };
                if tx_bus.send(f).is_err() {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            while let Some(frame) = stream.rx.recv().await {
                let Some(f) = transcript_frame_of(frame) else {
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
            session_addr: String::new(),
            workdir,
            provider,
            mode,
            agent,
        };
        client.ensure_session().await?;
        // 登记当前会话的 VDFS 订阅：**必须在发送之前**，否则首帧（`working`
        // 状态变更与首批流式增量）会在闸门打开前就流过去。
        client.open_session_watch().await?;
        Ok(client)
    }

    /// 会话在 VDFS 上的**展示地址**（`<根>/session/<sid>`）。
    ///
    /// 两个段各有归属，都不许在这里写死：
    /// - **根名**归 vdfs 插件（仓级守卫 S-010 禁止它出现在别处），经 `vdfs/root`
    ///   取回后当**运行期数据**持有（前端 `schemas/vdfsRoot` 同款做法）；
    /// - **挂载段**是容器的分发键——目录名 = 实例名，故取 `PLUGIN_SESSION`。
    ///
    /// 与 `plugins/agent/host/subagent.rs::session_vdfs_addr` 同构：两处爬同一个
    /// 地址规则，所以都只能从同一对常量 / 回包里拼。
    async fn session_vdfs_addr(&self, sid: &str) -> Result<String, String> {
        let resp = self.route(VDFS_ROOT, json!(null), None).await?;
        let root = resp
            .get("path")
            .and_then(Value::as_str)
            .map(|s| s.trim_end_matches('/'))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("vdfs/root 未返回根地址，无法订阅会话变更: {resp}"))?;
        Ok(format!("{root}/{PLUGIN_SESSION}/{sid}"))
    }

    /// 登记 / 摘除一条 VDFS 订阅（与 `vdfs/watch` 的引用计数严格配对）。
    ///
    /// 失败**上报**而不是像子智能体转播那样只告警：那里收到的只是「过程看不到」，
    /// 最终文本仍会由工具结果给出；而 CLI 的全部输出都挂在这条通道上——
    /// 静默失败的表现是「进程永远不返回」，用户拿不到任何线索。
    async fn set_vdfs_watch(&self, addr: &str, subscribe: bool) -> Result<(), String> {
        let path = if subscribe { VDFS_WATCH } else { VDFS_UNWATCH };
        let resp = self.route(path, json!({ "path": addr }), None).await?;
        if resp.get("success").and_then(Value::as_bool) == Some(false) {
            return Err(format!("{path} 失败（{addr}）: {resp}"));
        }
        Ok(())
    }

    /// 把闸门开到当前会话上（切会话时先 [`Self::close_session_watch`]）。
    async fn open_session_watch(&mut self) -> Result<(), String> {
        let addr = self.session_vdfs_addr(&self.session_id).await?;
        self.set_vdfs_watch(&addr, true).await?;
        self.session_addr = addr;
        Ok(())
    }

    /// 关上当前会话的闸门。摘除失败只告警：它是清理动作，不该让「切会话」失败。
    async fn close_session_watch(&mut self) -> Result<(), String> {
        if self.session_addr.is_empty() {
            return Ok(());
        }
        let addr = std::mem::take(&mut self.session_addr);
        self.set_vdfs_watch(&addr, false).await
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

        let req = session_update::Request {
            session_id: self.session_id.clone(),
            metadata: Value::Object(metadata),
            title: None,
        };
        let resp = self
            .route("session/update", req, Some(&self.session_id))
            .await?;
        if resp.get("success").and_then(Value::as_bool) != Some(true) {
            return Err(format!("会话初始化未成功: {resp}"));
        }
        Ok(())
    }

    /// 切换会话：换闸门、更新 id 并重新写入元数据。
    ///
    /// 顺序是「先关旧、再开新」：开着旧会话的闸门不影响正确性（变更按地址
    /// 前缀过滤），但会让后端持续往本连接投递一个已不关心的会话的变更。
    pub async fn switch_session(&mut self, session_id: String) -> Result<(), String> {
        self.close_session_watch().await?;
        self.session_id = session_id;
        self.ensure_session().await?;
        self.open_session_watch().await
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
    /// ## 两类帧各司其职
    ///
    /// - [`Frame::Transcript`]（消息实时面）：显式操作直接交渲染器——正文在
    ///   `upsert` / `append` 里增量输出，不需要任何折算。
    /// - [`Frame::Node`]（会话运行态）：**本轮结束的唯一判据**，见下。
    ///
    /// ## 为什么结束判据只能看会话节点
    ///
    /// 一轮请求里模型会多次定格根 Turn 节点（每个工具轮次一次），所以「Turn
    /// 到达终态」只说明**这一轮模型输出**结束，不说明整轮请求结束。真正的终态
    /// 在会话节点上：`status` 离开 `working` 即整轮结束，结局从 `attributes` 读
    /// ——`outcome == aborted` 是中止，`error` 非空是失败。
    ///
    /// 旧 EventBus 的 `Status{idle}` / `Abort` 帧也不再需要（旧事件频道已废除）：
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
        let prefix = format!("{}/", self.session_addr);
        // 上一帧的会话运行态：状态提示只在**迁移**上报（否则一次轮次要喊两遍「处理中」）
        let mut last_status: Option<String> = None;
        // 转写流内**单调 seq**：跳号 = 已知有损。CLI 只做流式输出（已打印的正文
        // 不可回收），因此恢复不了，只能留痕，让用户知道本轮输出不完整。
        let mut last_seq: Option<u64> = None;

        loop {
            let next = tokio::time::timeout(TURN_TIMEOUT, self.events.recv())
                .await
                .map_err(|_| "等待模型响应超时".to_string())?;
            let Some(frame) = next else {
                return Err("下行连接已关闭".to_string());
            };

            match frame {
                // ── 消息实时面：显式操作，直接落地 ──
                Frame::Transcript(ev) => {
                    // 转写流是**全会话广播**，本进程只渲染当前会话
                    if ev.session_id != self.session_id {
                        continue;
                    }
                    if let Some(last) = last_seq {
                        if ev.seq <= last {
                            continue; // 重连窗口内可能重发，重复应用 append 会叠字
                        }
                        if ev.seq != last + 1 {
                            r.warn(&format!(
                                "⚠ 转写流跳号（{last} → {}），本轮输出可能不完整",
                                ev.seq
                            ));
                        }
                    }
                    last_seq = Some(ev.seq);

                    match ev.op {
                        NodeOp::Upsert { message } => r.on_upsert(&message),
                        NodeOp::Append { message_id, delta } => r.on_append(&message_id, &delta),
                        NodeOp::Remove { message_id } => r.on_remove(&message_id),
                        // reset 后正文需整份重读，而 CLI 不落消息树：本地快照作废即可。
                        NodeOp::Reset => r.warn("⚠ 转写被截断，本轮流式输出已作废"),
                        // 会话级告警走会话节点（VDFS watch 域），不经转写流。
                        NodeOp::Warn { .. } => {}
                    }
                }
                // 后端明示「你可能漏了帧」：无历史可重读，只能留痕。
                Frame::Resync => {
                    r.warn("⚠ 转写流背压（后端已重同步），本轮输出可能不完整");
                }
                // ── 会话运行态：本轮结束的唯一判据 ──
                Frame::Node(change) => {
                    // 地址前缀过滤：后端按订阅路径投递，而本进程只登记了当前会话
                    if change.path != self.session_addr && !change.path.starts_with(prefix.as_str())
                    {
                        continue;
                    }
                    // 深于会话叶子的变更（消息的 VDFS 投影）已不再是实时面，忽略。
                    if change.path != self.session_addr {
                        continue;
                    }
                    let Some(node) = change.node.as_ref() else {
                        continue;
                    };
                    if node.status == VDFS_STATUS_WORKING {
                        // 只在**迁移**上报一次：运行态是节点属性，同一次「开始工作」
                        // 可能因元数据写入等原因重复下发同一份视图（视图幂等，重复无害）。
                        if last_status.as_deref() != Some(node.status.as_str()) {
                            r.on_status(&node.status);
                        }
                        last_status = Some(node.status.clone());
                        continue;
                    }
                    // 离开 `working` = 本轮结束（中止 / 失败 / 正常收尾三种结局之一，
                    // 由 `outcome` 区分）。
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
    /// 无人值守轮次。守护模式只关心会话运行态（[`Frame::Node`]），转写帧在此被
    /// 丢弃——但它**必须被取走**：不取就会把转写流的通道塞满，触发后端摘除订阅。
    /// 返回 `None` 表示下行连接已关闭。
    pub async fn next_frame(&mut self) -> Option<Frame> {
        self.events.recv().await
    }

    /// 心跳守护模式：订阅**会话挂载根**（所有会话，而不是某一个）。
    ///
    /// 守护模式不知道将来会有哪些会话被心跳调度器触发，因此订阅必须在
    /// 「所有会话」这一层：`<根>/session`。provider 收到的相对路径是空串，
    /// 而空串在订阅表里**恒命中**（`ChangeSubscriptions::related`），
    /// 于是各会话的运行态变更都会到达本连接。
    ///
    /// 与单会话订阅（[`Self::open_session_watch`]）用的是同一对动作，
    /// 差别只在闸门开在哪一级地址上。
    /// 返回**会话挂载根**的地址（`<根>/session`），调用方据此认会话（末段即会话 id）。
    pub async fn watch_all_sessions(&self) -> Result<String, String> {
        let resp = self.route(VDFS_ROOT, json!(null), None).await?;
        let root = resp
            .get("path")
            .and_then(Value::as_str)
            .map(|s| s.trim_end_matches('/'))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("vdfs/root 未返回根地址，无法订阅会话变更: {resp}"))?;
        let mount = format!("{root}/{PLUGIN_SESSION}");
        self.set_vdfs_watch(&mount, true).await?;
        Ok(mount)
    }

    /// 查询当前 Provider（用于 REPL 的 `/provider` 状态显示）。
    pub fn provider_label(&self) -> String {
        self.provider
            .clone()
            .unwrap_or_else(|| "default（系统目录配置）".to_string())
    }
}
