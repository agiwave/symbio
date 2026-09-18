//! 进程内 Symbio 客户端。
//!
//! ## 设计要点
//!
//! CLI 与 Tauri 前端的差别只在「传输层」：Tauri 走 `route_v2` IPC，本 CLI 直接
//! 拿到 `Arc<dyn Plugin>` 根节点在**进程内**调用 `root.route(ctx)`。二者用的是
//! 同一套上下文键（PATH / PAYLOAD / SESSION_ID / WORKDIR / …），因此不需要任何
//! 协议改动 —— 换传输 = 换「请求 → SimpleRequest」这一层适配。
//!
//! ## 下行三通道里选了哪一条
//!
//! 后端下行有三条：① session 私有长连接（当年经 `session/open` 拿通道，
//! 该路由已退役）；② 全局 `EventBus`（`event_bus/subscribe` 一条连接收全部事件）；
//! ③ 前端主动拉取。CLI 选 ②：它与"打开哪个会话"解耦，订阅一次即可覆盖后续
//! 所有会话（切会话不必重建连接），也是 Tauri 前端会话列表实时更新的同一机制。

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::sync::mpsc;

use symbio::init::create_root_plugin;
use symbio::symbio_core::event_bus::SubscribeRequest;
use symbio::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use symbio::symbio_core::schemas::session::session_chat;
use symbio::symbio_core::schemas::session::session_chat_response::StreamEvent;
use symbio::symbio_core::schemas::session::session_update;
use symbio::symbio_core::{
    InvokeRequestExt, Plugin, PluginFrame, PluginPayload, SimpleRequest, EVENT_BUS_SUBSCRIBE, PATH,
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

/// 事件总线帧（已从 PluginFrame 解包出的业务字段）。
pub struct BusEvent {
    pub kind: String,
    pub session_id: Option<String>,
    pub data: Value,
}

fn parse_bus_frame(frame: PluginFrame) -> Option<BusEvent> {
    let PluginFrame::Data(v) = frame else {
        return None;
    };
    let inner = v.get("data")?;
    Some(BusEvent {
        kind: inner.get("kind")?.as_str()?.to_string(),
        session_id: inner
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        data: inner.get("data").cloned().unwrap_or(Value::Null),
    })
}

pub struct SymbioClient {
    root: Arc<dyn Plugin>,
    events: mpsc::UnboundedReceiver<BusEvent>,
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

        // 订阅事件总线：一次订阅覆盖所有会话，切会话无需重连。
        // 路径取 `symbio_core::paths` 常量——调用侧不写字面量（见该模块「地址规则」）。
        let ctx = Arc::new(SimpleRequest::new(None, None));
        ctx.set(PATH, EVENT_BUS_SUBSCRIBE.to_string());
        ctx.set_payload(SubscribeRequest { kinds: None })
            .map_err(|e| format!("设置订阅载荷失败: {e}"))?;
        let mut chan = match Arc::clone(&root)
            .route(ctx)
            .await
            .map_err(|e| format!("订阅事件总线失败: {e}"))?
        {
            PluginPayload::Session(c) => c,
            other => return Err(format!("event_bus/subscribe 返回了非会话载荷: {other:?}")),
        };

        // 独立转发任务：把帧解包后送进无界通道，主逻辑就不必关心连接细节。
        let (tx, events) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(frame) = chan.rx.recv().await {
                match parse_bus_frame(frame) {
                    Some(ev) => {
                        if tx.send(ev).is_err() {
                            break;
                        }
                    }
                    None => continue,
                }
            }
        });

        let workdir = workdir.to_string_lossy().to_string();
        let session_id = session.unwrap_or_else(|| gen_id("cli"));

        let client = Self {
            root,
            events,
            session_id,
            workdir,
            provider,
            mode,
            agent,
        };
        client.ensure_session().await?;
        Ok(client)
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

    /// 切换会话：更新 id 并重新写入元数据。
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
    /// 完成判定（与前端 `sessionBusWatcher` 一致）：收到 `Status{idle}` 或 `Abort`。
    /// 业务错误（`Error`）只是记录，仍等 `idle` 收敛，以免把"还有后续帧"误判成结束。
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

        loop {
            let next = tokio::time::timeout(TURN_TIMEOUT, self.events.recv())
                .await
                .map_err(|_| "等待模型响应超时".to_string())?;
            let Some(ev) = next else {
                return Err("事件总线连接已关闭".to_string());
            };
            if ev.kind != "session" {
                continue;
            }
            if ev.session_id.as_deref() != Some(self.session_id.as_str()) {
                continue;
            }

            let Ok(stream_ev) = serde_json::from_value::<StreamEvent>(ev.data) else {
                continue;
            };

            match stream_ev {
                StreamEvent::Update { message } => r.on_update(&message),
                StreamEvent::Delete { message_id } => r.on_delete(&message_id),
                StreamEvent::Status { status } => {
                    r.on_status(&status);
                    if status == "idle" {
                        break;
                    }
                }
                StreamEvent::Error { error } => {
                    business_error = Some(error);
                }
                StreamEvent::Abort => {
                    r.on_abort();
                    break;
                }
                StreamEvent::Connected { .. } | StreamEvent::Disconnected => {}
            }
        }

        r.end_turn();
        match business_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 读取下一条事件总线事件（心跳守护模式用）。
    ///
    /// 心跳触发的会话与普通对话走同一套编排与事件发布，守护进程在这里
    /// 消费并渲染即可观察到无人值守轮次。返回 `None` 表示总线连接已关闭。
    pub async fn next_bus_event(&mut self) -> Option<BusEvent> {
        self.events.recv().await
    }

    /// 查询当前 Provider（用于 REPL 的 `/provider` 状态显示）。
    pub fn provider_label(&self) -> String {
        self.provider
            .clone()
            .unwrap_or_else(|| "default（系统目录配置）".to_string())
    }
}
