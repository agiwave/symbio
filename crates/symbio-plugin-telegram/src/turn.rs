//! 一次会话轮次：**入队 → 收实时面 → 取最终答复**——telegram 调用会话的唯一路径。
//!
//! ## 为什么不是 `session/chat/send`
//!
//! 那条路由的 `message` 分支只是「入队」的一层薄包装，回包**恒为**
//! `{"status":"accepted"}`，不是 `session_chat::Response`。本插件曾按后者解析回包
//! （`payload.get::<session_chat::Response>()`），于是 `full_text` 恒空、用户永远
//! 收到「无响应」——而 `if let Ok(..)` 把反序列化失败吞掉了，全程没有任何报错。
//! 同一缺陷在 2026-09-22 修过一次（设计文档
//! §4.9），后随实时面下沉 VDFS（D-025）而回归。
//!
//! 现在走**地址面**，答复从**实时面**取回——与 `cli` 和 `agent_run` 的转播桥
//! 同一条通道（D-025「会话实时面只有一条」）：
//!
//! ```text
//! vdfs/root                    → 根地址（唯一的「不给地址」入口）
//! vdfs/write <会话>            → 建会话 / 落运行选项（一次写就是 upsert）
//! 订阅总线 + vdfs/watch        → 开实时面（两步缺一不可，且**先于入队**以免丢首帧）
//! vdfs/write <会话>/inbox      → 用户消息入队
//! 收帧 …… 会话节点离开 working  → 本轮结束的唯一判据
//! ```
//!
//! ## 运行选项为什么写成会话 metadata
//!
//! 收件箱入队走的是 `session_chat::Request::default()`——**地址写入不承载运行选项**
//! （D-066）。因此「telegram 统一走 auto 模式」这条通道策略只能先落成**会话属性**，
//! 而不是随每条消息携带。落法与 `cli` 的 `ensure_session` 同款：一次带 `create`
//! 意图的 `vdfs/write` 同时覆盖「新建会话」与「改元数据」两种情形。
//!
//! ## 为什么没有与另两个消费方共用一份
//!
//! | 消费方 | 拿帧做什么 |
//! |---|---|
//! | `crates/symbio-cli/src/client.rs` | 逐帧**渲染**（增量 / 状态提示 / 重同步） |
//! | `agent/host/subagent.rs` | **翻译并转发**进父会话视图（锚定 / 改角色） |
//! | 本文件 | **只攒最终正文**——不渲染、不翻译、不转发 |
//!
//! 三者共有的只是上面那段协议舞步；帧语义各不相同，抽成一份反而要把三种行为
//! 参数化。因此这里只保留「收 → 攒 → 取最后一条已完成的助手正文」。

use symbio_deploy::PLUGIN_ID_SESSION;
use symbio_plugin_api::routes::{ROUTE_VDFS_ROOT, ROUTE_VDFS_UNWATCH, ROUTE_VDFS_WATCH};
use symbio_core::chat_node::{
    message_of_node, ChatNode, NodeRole, NodeStatus, SEG_MESSAGES,
};
use symbio_core::{
    event_bus_register_subscriber, event_bus_resync_of, event_bus_unregister_subscriber,
    event_bus_vdfs_change_of, Plugin, PluginFrame, PluginInvokeRequest, PluginInvokeRequestExt,
    VdfsChange, VdfsContent, VdfsNode, EVENT_BUS_KIND_VDFS, PATH, SESSION_ID, VDFS_STATUS_FAILED,
    VDFS_STATUS_WORKING, WORKDIR,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// 等待一轮结束的上限。与 `crates/symbio-cli/src/client.rs` 的 `TURN_TIMEOUT` 同量级：一轮里
/// 可能有长工具调用（构建 / 测试），超时是**兜底**，不是预期路径。
const TURN_TIMEOUT: Duration = Duration::from_secs(900);

/// 实时面收件队列容量。与 `event_bus` 插件建连接时的容量同量级——本插件**直接**
/// 注册收件端（不经 `event_bus/subscribe` 建通道，理由见 [`run_turn`]）。
const BUS_QUEUE: usize = 4096;

/// 收件箱段名（地址契约 `<会话>/inbox` 的一段）。
///
/// 机制层的 `SEG_INBOX` 是 session 插件的 `pub(crate)` 内部词汇、插件间不可见
/// （`plugin-entry-audit` E-009），本插件按地址契约自持一份——与 `cli` 自持
/// `SESSION_SEG` / `MESSAGES_SEG` 同一约定。拼错表现为「消息进不去队列」，
/// 不会静默错乱。
const SEG_INBOX: &str = "inbox";

/// telegram 通道的会话工作目录。接入的是无人值守通道，进程的工作目录就是它的
/// 会话根——与旧实现 `sub_ctx.set(WORKDIR, ".")` 逐字一致。
const CHANNEL_WORKDIR: &str = ".";

/// telegram 通道的会话模式：无人值守，交互式工具应返回友好错误由 LLM 自行决策，
/// 不产 `user_prompt` 阻塞等用户（与旧实现的 `mode: Some("auto")` 同一意图）。
const CHANNEL_MODE: &str = "auto";

/// 一轮的结局。
///
/// 三态而不是 `Result`：**「本轮以错误结束」与「实时面没接上」是两件事**——前者是
/// 会话跑完给出的结论（用户该看到那句话），后者是本插件自己没接上（该留下运维痕迹）。
/// 压成 `Err(String)` 之后调用方无从区分，只能二选一地把两类原因讲成一种。
pub(super) enum TurnOutcome {
    /// 本轮正常收尾。`text` 是最后一条**已完成**的助手正文（可能为空串）。
    Replied(String),
    /// 本轮以错误结束：`error` 取自会话节点。
    Failed(String),
    /// 地址面 / 实时面本身出错（会话没跑起来）。
    Broken(String),
}

/// 发一条用户消息给会话，等本轮结束，返回最终答复。
///
/// `router` 是**父容器**（`ctx.parent()`）：`telegram` 与 `session` 在容器下平级，
/// 只有容器的 `route` 认识绝对地址，且地址分发用的是同一个键——因此这里拼出的
/// `<根>/session/<id>` 命中的正是 `session` 插件实例（与 `agent_run` 的
/// `register_subsession` 同一推理）。
pub(super) async fn run_turn(
    router: Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    session_id: &str,
    text: &str,
) -> TurnOutcome {
    // ── ① 根地址 ──
    // 根叫什么**归 vdfs 插件**，使用方不写死：问一次，之后一律从父地址往下拼
    // （与 `cli` 的 `fetch_root_addr`、`agent_run` 的 `session_vdfs_addr` 同款）。
    let root = match route_json(&router, ctx, ROUTE_VDFS_ROOT, session_id, json!({})).await {
        Ok(v) => {
            let p = v
                .get("path")
                .and_then(Value::as_str)
                .map(|s| s.trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty());
            match p {
                Some(p) => p,
                None => return TurnOutcome::Broken(format!("vdfs/root 未返回根地址：{v}")),
            }
        }
        Err(e) => return TurnOutcome::Broken(e),
    };
    let session_addr = format!("{root}/{PLUGIN_ID_SESSION}/{session_id}");

    // ── ② 建会话 / 落运行选项 ──
    // 这一步**不可省**：收件箱的写入要求会话已存在（provider 的 `session_of`），
    // 而新建会话最自然的写法正是「带 create 意图写一次 metadata」。
    let meta =
        json!({ "metadata": { "mode": CHANNEL_MODE, "workdir": CHANNEL_WORKDIR } }).to_string();
    if let Err(e) = route_json(
        &router,
        ctx,
        "vdfs/write",
        session_id,
        json!({ "path": session_addr, "create": true, "text": meta }),
    )
    .await
    {
        return TurnOutcome::Broken(e);
    }

    // ── ③ 开实时面（**先于入队**，否则首帧会丢）──
    //
    // 与 `cli` / 前端 / `agent_run` 同构，但**不经 `event_bus/subscribe` 路由**：
    // 那条路由是为**跨进程**消费方（前端）建 `PluginChannel` 的，而本插件与总线同
    // 进程，直接注册收件端即可（`agent_run` 的转播桥正是这么做的——先例在那里）。
    // 于是这里只有两步，而不是三步。
    let (bus_tx, mut bus_rx) = mpsc::channel::<PluginFrame>(BUS_QUEUE);
    let conn_id = format!("telegram-{session_id}");
    event_bus_register_subscriber(conn_id.clone(), bus_tx);
    if let Err(e) = route_json(
        &router,
        ctx,
        ROUTE_VDFS_WATCH,
        session_id,
        json!({ "path": session_addr }),
    )
    .await
    {
        event_bus_unregister_subscriber(&conn_id);
        return TurnOutcome::Broken(e);
    }

    // ── ④ 入队 ──
    // 正文直接是用户消息文本（`parse_inbox_message` 的判别式：不以 `{` 起头即整段
    // 正文），因此不构造 `ChatNode`——身份与缺省由 provider 一侧补齐。
    let queued = route_json(
        &router,
        ctx,
        "vdfs/write",
        session_id,
        json!({ "path": format!("{session_addr}/{SEG_INBOX}"), "text": text }),
    )
    .await;
    if let Err(e) = queued {
        unwatch(&router, ctx, session_id, &session_addr).await;
        event_bus_unregister_subscriber(&conn_id);
        return TurnOutcome::Broken(e);
    }

    // ── ⑤ 收帧直到本轮结束 ──
    let mut acc = Accumulator::default();
    let deadline = Instant::now() + TURN_TIMEOUT;
    let outcome = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break TurnOutcome::Broken("等待会话响应超时".to_string());
        }
        let frame = match tokio::time::timeout(left, bus_rx.recv()).await {
            // 超时：会话没在本插件的等待窗口内收尾。可能仍在跑（长工具调用），
            // 但调用方需要一个答复，如实报出来比无限等下去好。
            Err(_) => break TurnOutcome::Broken("等待会话响应超时".to_string()),
            Ok(None) => break TurnOutcome::Broken("实时面已关闭".to_string()),
            Ok(Some(f)) => f,
        };

        let Some(change) = event_bus_vdfs_change_of(&frame) else {
            // 非 vdfs 频道的帧（系统事件）不归本面管；背压重同步单独认。
            if is_resync(&frame) {
                if let Some(node) = stat_node(&router, ctx, session_id, &session_addr).await {
                    if node.status != VDFS_STATUS_WORKING {
                        break finish(&node, acc);
                    }
                }
            }
            continue;
        };

        // ① 会话运行态：**本轮结束的唯一判据**。一轮里根 Turn 会多次定格，只有
        // 会话节点离开 `working` 才是整轮结束。无载荷 = 资源信号（标题 / metadata /
        // 创建），不是运行态 ⇒ 丢弃（回读也只会得到同一个 `status`）。
        if change.path == session_addr {
            let node = change
                .data
                .clone()
                .and_then(|v| serde_json::from_value::<VdfsNode>(v).ok());
            if let Some(node) = node {
                if node.status != VDFS_STATUS_WORKING {
                    break finish(&node, acc);
                }
            }
            continue;
        }

        // ② 消息节点自身：`<会话>/message/<mid>`——身份在地址末段。
        if let Some(mut m) = message_at(&session_addr, &change) {
            // 窄增量撞上未知身份（订阅晚于流开始，或中间漏过帧）⇒ 回读基线，
            // 否则前缀会丢。回读是幂等的当前值，与帧到达顺序无关。
            if m.content.is_none() && m.delta.is_some() && !acc.known(&m.id) {
                let rel = format!("{session_addr}/{SEG_MESSAGES}/{}", m.id);
                if let Some(base) = readback(&router, ctx, session_id, &rel).await {
                    m = base;
                }
            }
            acc.apply(m);
        }
    };

    // ── ⑥ 摘除（与登记严格配对：先关闸门，再从总线摘除收件端）──
    unwatch(&router, ctx, session_id, &session_addr).await;
    event_bus_unregister_subscriber(&conn_id);
    outcome
}

/// 会话节点视图 → 本轮结局。
fn finish(node: &VdfsNode, acc: Accumulator) -> TurnOutcome {
    // 结局从节点属性读：`error` 非空即失败（与 `cli` 的 `turn_end_of` 同一判据）。
    if let Some(err) = node
        .attributes
        .get("error")
        .and_then(Value::as_str)
        .filter(|e| !e.is_empty())
    {
        return TurnOutcome::Failed(err.to_string());
    }
    if node.status == VDFS_STATUS_FAILED {
        return TurnOutcome::Failed("本轮以错误结束".to_string());
    }
    TurnOutcome::Replied(acc.final_text())
}

/// 助手正文的累积器：按消息 id 攒，最终取**最后一条已完成的助手正文**。
///
/// 取「已完成」而不是「最后处理到的文本」：流式片段与推理独白都不该被当成最终
/// 答复（与 `agent_run` 的转播桥同一判据）。`is_reasoning()` 因此不进累积器
/// ——它进的是同一个地址空间，但它是思考过程，不是答复。
#[derive(Default)]
struct Accumulator {
    /// 按消息 id 的助手正文
    text_by_id: HashMap<String, String>,
    /// 正文到达顺序（取「最后一条」的依据）
    order: Vec<String>,
    /// 已进入终态的消息 id
    completed: HashSet<String>,
}

impl Accumulator {
    fn known(&self, mid: &str) -> bool {
        self.text_by_id.contains_key(mid)
    }

    fn apply(&mut self, m: ChatNode) {
        let mid = m.id.clone();

        // 删除优先：`status = removed` 是消息词汇里的删除语义，本地索引与父视图
        // 同步移除——身份改写、正文累积对已删节点没有意义。
        if m.status == Some(NodeStatus::Removed) {
            self.text_by_id.remove(&mid);
            self.order.retain(|x| x != &mid);
            self.completed.remove(&mid);
            return;
        }

        if m.node_role() == Some(NodeRole::Assistant) && (m.is_text() || m.msg_type.is_none()) {
            if let Some(delta) = m.delta.as_deref() {
                // 纯增量：追加。身份未知时也照收——回读兜底在帧循环里，
                // 走到这里说明基线已拿不到，攒下来的部分仍好过丢掉整条。
                self.text_by_id
                    .entry(mid.clone())
                    .or_default()
                    .push_str(delta);
                if !self.order.contains(&mid) {
                    self.order.push(mid.clone());
                }
            } else if let Some(content) = m.content.as_ref() {
                // 全量帧：整条替换。
                if !self.text_by_id.contains_key(&mid) {
                    self.order.push(mid.clone());
                }
                self.text_by_id.insert(mid.clone(), content.to_text());
            }
            // 既无 `delta` 又无 `content`（纯状态帧）⇒ 正文一个字都不动。
        }

        if matches!(
            m.status,
            Some(NodeStatus::Completed) | Some(NodeStatus::Failed) | Some(NodeStatus::Aborted)
        ) {
            self.completed.insert(mid);
        }
    }

    /// 最后一条**已完成**的助手正文；一条都没有时退化为最后一条正文。
    fn final_text(&self) -> String {
        self.order
            .iter()
            .rev()
            .find(|id| self.completed.contains(*id))
            .or(self.order.last())
            .and_then(|id| self.text_by_id.get(id).cloned())
            .unwrap_or_default()
    }
}

/// 一条变更是否落在**某条消息节点自身**的地址上（`<会话>/message/<mid>`）。
///
/// 判据是**地址形状**，不是「路径等于某个目录」：会话容器的统一形状是
/// `<会话>/<集合段>/<项 id>`，本函数只认「消息」那一段的**项**——集合目录本身、
/// 更深的层级、别的集合段（收件箱 / 记忆 / 工作目录）都不认（与 `agent_run` 的
/// 转播桥同一判据，理由见那里：按地址形状认人才能推广到第二类集合）。
fn message_at(session_addr: &str, change: &VdfsChange) -> Option<ChatNode> {
    let prefix = format!("{session_addr}/{SEG_MESSAGES}/");
    let mid = change.path.strip_prefix(&prefix)?;
    if mid.is_empty() || mid.contains('/') {
        return None;
    }
    let mut m = serde_json::from_value::<ChatNode>(change.data.clone()?).ok()?;
    // 地址即身份（载荷里的 `id` 只是同一事实的复述，以地址为准）。
    m.id = mid.to_string();
    Some(m)
}

/// 回读一个消息节点的**基线**：`stat` 给身份与状态，`read` 给正文。
///
/// 两者都是幂等的当前值，因此这次回读与帧到达顺序无关——这正是它敢做兜底的原因。
/// 逆投影走 `message_of_node`（与 session 侧拼节点成对演进的那一份），不自己拼
/// `ChatNode`：拼错了不会报错，只会让某条消息悄悄少一截。
async fn readback(
    router: &Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    session_id: &str,
    rel: &str,
) -> Option<ChatNode> {
    let node: VdfsNode = serde_json::from_value(
        route_json(router, ctx, "vdfs/stat", session_id, json!({ "path": rel }))
            .await
            .ok()?,
    )
    .ok()?;
    let content: VdfsContent = serde_json::from_value(
        route_json(router, ctx, "vdfs/read", session_id, json!({ "path": rel }))
            .await
            .ok()?,
    )
    .ok()?;
    message_of_node(&node, content.text?)
}

/// 回读一个节点的**视图**（背压重同步后重读运行态用）。
async fn stat_node(
    router: &Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    session_id: &str,
    addr: &str,
) -> Option<VdfsNode> {
    serde_json::from_value(
        route_json(
            router,
            ctx,
            "vdfs/stat",
            session_id,
            json!({ "path": addr }),
        )
        .await
        .ok()?,
    )
    .ok()
}

/// 登记 / 摘除一条 VDFS 订阅（与 `vdfs/watch` 的引用计数严格配对）。
///
/// 失败只告警不中断：登记的收益是「能收到中间帧」，最坏结果是转播看不到过程——
/// 为此把整轮判失败不成比例（与 `agent_run` 的 `vdfs_watch` 同一处置）。
async fn unwatch(
    router: &Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    session_id: &str,
    addr: &str,
) {
    if let Err(e) = route_json(
        router,
        ctx,
        ROUTE_VDFS_UNWATCH,
        session_id,
        json!({ "path": addr }),
    )
    .await
    {
        symbio_core::plugin_warn!("telegram", "vdfs/unwatch 摘除失败（{}）: {}", addr, e);
    }
}

/// 帧是否是一条**背压重同步标记**。
///
/// 判别本身住在 core（`event_bus_resync_of`）：它读的是线上字面量，且与产帧方同层——
/// 本插件只声明「vdfs 频道的那一条指令归我」。手写一份判别等于给漂移留一个位置
/// （子智能体转播桥就漏写过，见该函数文档）。
fn is_resync(frame: &PluginFrame) -> bool {
    event_bus_resync_of(frame, EVENT_BUS_KIND_VDFS)
}

/// 一次 `vdfs/*` 调用：fork 请求上下文、设 PATH / WORKDIR / SESSION_ID、取回 JSON 载荷。
///
/// `PATH` 取常量而非字面量：`set(PATH, "<字面量>")` 会让「路由改名」不产生任何编译
/// 错误，只在运行期表现为「调用失败」——`plugin-entry-audit` E-003 拦的正是这个。
/// 尚无 core 常量的两条（`vdfs/write` / `vdfs/stat`）按 `cli` 的先例自持字面量：
/// core 只把 `root` / `watch` / `unwatch` 三条列为跨插件例外，其余协议路径归 vdfs
/// 插件自己（见 `plugins/vdfs/protocol.rs` 的模块文档）。
async fn route_json(
    router: &Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    path: &str,
    session_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let c = ctx.fork();
    c.set(PATH, path.to_string());
    c.set(WORKDIR, CHANNEL_WORKDIR.to_string());
    c.set(SESSION_ID, session_id.to_string());
    c.set_payload(payload)
        .map_err(|e| format!("构造 {path} 载荷失败: {e}"))?;
    let resp = router
        .clone()
        .route(c)
        .await
        .map_err(|e| format!("{path} 调用失败: {e}"))?;
    let v: Value = resp
        .get::<Value>()
        .map_err(|e| format!("{path} 响应解析失败: {e}"))?;
    // 失败也可能经**数据载荷**回传（`{"error": …}`）而不表现为传输错误——
    // 照实报出来，否则会退化成「响应解析失败」这种看不出原因的消息（`cli` 的
    // `ensure_session` 同一处置）。
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        if !err.is_empty() {
            return Err(format!("{path} 未成功: {err}"));
        }
    }
    Ok(v)
}

#[cfg(test)]
#[path = "turn.test.rs"]
mod tests;
