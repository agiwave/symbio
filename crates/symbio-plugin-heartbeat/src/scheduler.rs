//! 扫描循环：本插件唯一那条常驻任务
//!
//! 它只回答一个问题——「这个会话现在该不该开一轮」，判据全在本文件的
//! [`due`] 里（一个不碰 IO 的纯函数，可单测）。循环体只做三件事：取清单、问 `due`、
//! 写一次收件箱。
//!
//! ## 为什么每 tick 重新取清单（不缓存）
//!
//! 空闲判定要的是**磁盘侧的 `updated_at`**，而它是回合内每条消息落盘持续推进的那一格。
//! 缓存清单等于把 `metadata.heartbeat` 抄成第二份 owner——[D-076](README.md#d-076-心跳片的八个宿主触点逐条落位调度随片走触发事实落进收件箱写臂 "crates/symbio-plugin-heartbeat/src/README")
//! #5 要避免的正是这个形态。代价（每 tick 一次全项目 options 广播）已在
//! [D-077](README.md#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出 "crates/symbio-plugin-heartbeat/src/README") 后果段**认下并命名**，不在本批优化。
//!
//! ## 触发失败为什么不写锚点
//!
//! 落格失败（会话正被写 / 无事实源 / 开档失败）都是**暂时的**：留着原锚点、下一个
//! tick 重试。按整段 `interval_seconds` 推迟等于把一次本可成功的触发白白睡掉。
//! 失败也**不会**压进队列——写臂在入队之前就把它拒了（收件箱是 FIFO，一条永远跑不起来
//! 的队首会把后面的用户消息一起堵死）。

use super::plugin::HeartbeatPlugin;
use super::setting::HeartbeatConfig;
use symbio_deploy::PLUGIN_ID_SESSION;
use symbio_core::chat_node as cm;
use symbio_core::{
    clock_now_ms, vdfs_context, DynVdfsProvider, PluginInvokeRequest, PluginSimpleRequest,
    VdfsContent, VdfsContext, VdfsNode, VdfsRequest, VdfsResponse, VdfsResult,
    EVENT_TRIGGER_KIND_SCHEDULED, EVENT_TRIGGER_META_KEY, VDFS_STATUS_WORKING,
};
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

/// 扫描间隔（秒）
const TICK_SECS: u64 = 15;
/// 每次扫描最多触发的会话数（防止启动瞬间大量旧会话同时打爆模型服务）
const MAX_PER_TICK: usize = 2;
/// 心跳间隔的确定性抖动上限（秒）：按会话 id 哈希错峰，避免同间隔会话同时触发
const JITTER_SECS: u64 = 30;

/// 收件箱段名（地址契约 `<会话>/inbox` 的一段）。
///
/// 机制层的正主是 session 插件的 `pub(crate)` 内部词汇、插件间不可见（E-009），
/// 故按地址契约自持一份——`agent` / `telegram` 两处同此办理（理由各自写在文件里）。
/// 拼错的表现为「消息进不去队列」，不会静默错乱。
const SEG_INBOX: &str = "inbox";

/// 按会话 id 派生的确定性抖动（0..`JITTER_SECS` 秒），用于错峰触发。
///
/// 采用确定性哈希而非随机：同一会话每次求得的抖动稳定，避免不同 tick 间反复抖动；
/// 同时让间隔相同的多个会话按 id 自然错开，缓解启动惊群。
/// 对早已超时的会话，抖动相对 `interval_seconds` 可忽略，仍会在首个 tick 触发。
pub(crate) fn phase_ms(id: &str) -> i64 {
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    (hasher.finish() % JITTER_SECS) as i64 * 1_000
}

/// 空闲基线：内存锚点与磁盘侧 `updated_at` 取较新者。
///
/// - 正常路径：回合内的消息落盘持续刷新 `updated_at`，其最终值 ≈ 活动真正结束
///   时刻，空闲时长因此从「会话无活动之后」起算；
/// - 异常路径：回合零写盘退出时 `updated_at` 停留在回合开始前，触发时写入的
///   锚点（较新）作为下限，防止逐 tick 热循环重触发。
pub(crate) fn idle_baseline(anchor: Option<i64>, updated_at: i64) -> i64 {
    anchor.map_or(updated_at, |a| a.max(updated_at))
}

/// 清单里一个会话项中**本插件用得上的那四格**。
///
/// 从 [`VdfsNode`] 投影出来的目的只有一个：让 [`due`] 不依赖地址面，从而能被
/// 逐条断言（切片前它长在宿主实例的方法里，判空闲的算术因此没法单独测）。
pub(crate) struct SessionView {
    pub(crate) id: String,
    /// 正在工作（`status == working`）：地址面读不到运行态时 provider **保守报 working**
    /// （见 `plugins/session/plugin/vdfs_provider.rs` 的运行态投影），本片因此继承
    /// 「宁可不触发也不打断在跑的回合」这一侧。
    pub(crate) working: bool,
    pub(crate) updated_at: Option<i64>,
    pub(crate) metadata: Value,
}

impl SessionView {
    pub(crate) fn of(node: &VdfsNode) -> Self {
        Self {
            id: node.name.clone(),
            working: node.status == VDFS_STATUS_WORKING,
            updated_at: node.updated_at,
            metadata: node
                .attributes
                .get("metadata")
                .cloned()
                .unwrap_or_else(|| json!({})),
        }
    }

    /// 投影的**自检**：地址面给不出这三格的那个节点不是会话项。
    ///
    /// `session` 挂载根的清单里只有会话（provider 自己保证），但本插件是按名字取数的
    /// 外来读者——把「不是我认的形状就跳过」写成代码，而不是写成对别人的信任。
    fn is_session_kind(node: &VdfsNode) -> bool {
        node.kind == PLUGIN_ID_SESSION
    }
}

/// 这个会话现在该不该开一轮：给出 ⇒ 已空闲的毫秒数，`None` = 不该（或判不了）。
///
/// 四条判据的顺序有讲究：前两条是**配置**（没启用就什么都不用看），第三条是
/// **不打断在跑的回合**，第四条才是时间。`updated_at` 缺失时返回 `None`——
/// 「不知道它什么时候活动过」不能读成「它一直空闲」，那会让一个刚建好的会话在
/// 首个 tick 就被触发。
pub(crate) fn due(view: &SessionView, anchor: Option<i64>, now: i64) -> Option<i64> {
    let hb = HeartbeatConfig::from_metadata(&view.metadata);
    if !hb.enabled || hb.prompt.trim().is_empty() {
        return None;
    }
    if view.working {
        return None;
    }
    let updated_at = view.updated_at?;
    let idle = now - idle_baseline(anchor, updated_at);
    let interval_ms = (hb.interval_seconds as i64) * 1_000 + phase_ms(&view.id);
    (idle >= interval_ms).then_some(idle)
}

/// 心跳这一轮要发出去的用户消息：提示词 + 两片 `meta`。
///
/// - `meta.heartbeat = true`：给**前端**区分展示用（这条是自动发的）；
/// - `meta.trigger = {kind, idle_ms}`：给**会话写臂**就地消费的触发声明
///   （[D-076](README.md#d-076-心跳片的八个宿主触点逐条落位调度随片走触发事实落进收件箱写臂 "crates/symbio-plugin-heartbeat/src/README") 的标记落法）。它由写臂在入队前
///   折叠成事实源里的四格，**不要求活到转写**——两片各归各的消费者，不合成一个。
///
/// 消息 id 带触发时刻的毫秒（`hb_<sid>_<毫秒>`），同一会话的两次心跳消息因此天然不同 id。
/// 「这次触发**是不是自主的**」由事实源里那四格说话，不靠消息上的这个布尔值。
pub(crate) fn trigger_message(
    session_id: &str,
    hb: &HeartbeatConfig,
    idle_ms: i64,
) -> cm::ChatNode {
    let now = clock_now_ms();
    cm::ChatNode {
        id: format!("hb_{}_{}", session_id, now),
        // 心跳以「人」的身份入会话：它触发的是同一轮对话（provider 侧必须是 user
        // 请求），不是会话主体的自说自话。主体地址沿用 `person/local`——这是
        // 落库点 `attributed` 对 user 形的既有归属，换新地址只会让同一类消息
        // 在存量与新写之间分成两个主体。
        principal: Some(symbio_core::principal::Principal::from_legacy(
            symbio_core::PRINCIPAL_USER,
        )),
        msg_type: Some(cm::NodeKind::Body),
        content: Some(cm::MessageContent::Text(hb.prompt.clone())),
        status: Some(cm::NodeStatus::Completed),
        timestamp: Some(now),
        meta: Some(json!({
            "heartbeat": true,
            EVENT_TRIGGER_META_KEY: {
                "kind": EVENT_TRIGGER_KIND_SCHEDULED,
                "idle_ms": idle_ms,
            }
        })),
        ..Default::default()
    }
}

impl HeartbeatPlugin {
    /// 扫描循环主体（由 [`HeartbeatPlugin::start_scan`] 在构造点 spawn，常驻）。
    ///
    /// 每 [`TICK_SECS`] 秒一轮；停机位在**每轮开始前**与**扫描体内每次触发前**各查一次，
    /// 于是 `stop` 之后最多再走完当前这一轮，且此后**一条也不触发**。
    pub(crate) async fn run_scan_loop(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(Duration::from_secs(TICK_SECS));
        // 跳过首 tick（interval 首次 tick 立即返回），避免启动瞬间集中触发
        ticker.tick().await;

        symbio_core::plugin_info!("heartbeat", "扫描循环已启动（间隔 {TICK_SECS}s）");

        loop {
            let (wake, stopping) = self.stop_gate();
            if stopping {
                break;
            }
            tokio::select! {
                _ = ticker.tick() => {}
                _ = wake => break,
            }
            self.scan_once().await;
        }
    }

    /// 一轮扫描：取清单 → 逐个问 [`due`] → 最多触发 [`MAX_PER_TICK`] 个。
    ///
    /// 清单取不到（容器已释放 / session 不在本作用域 / 读失败）就**整轮跳过**并记一条
    /// 告警：这不是错误路径而是「本棵子树没有会话」的常态之一，但它持续发生就该看得见。
    async fn scan_once(&self) {
        let Some(provider) = self.provider() else {
            return;
        };
        // 经容器取数需要一个宿主态来承载本轮的访客表（`empty()` 在容器转发下当场失败，
        // 见 D-077 裁决三）。这里给的是「无前端会话、无配置」的那个宿主。
        let req: Arc<dyn PluginInvokeRequest> =
            Arc::new(PluginSimpleRequest::new(self.parent_handle(), None));
        let vctx = vdfs_context(&req);
        let now = clock_now_ms();

        let items = match list_sessions(&provider, &vctx).await {
            Ok(items) => items,
            Err(e) => {
                symbio_core::plugin_warn!("heartbeat", "会话清单读取失败，本轮跳过: {e:?}");
                return;
            }
        };

        let mut fired = 0usize;
        for item in items {
            if fired >= MAX_PER_TICK {
                break;
            }
            // 停机位可能在扫描途中才置上：此后一条也不触发
            let (wake, stopping) = self.stop_gate();
            drop(wake);
            if stopping {
                break;
            }
            if !SessionView::is_session_kind(&item.node) {
                continue;
            }
            let view = SessionView::of(&item.node);
            let anchor = self.anchor_of(&view.id).await;
            let Some(idle_ms) = due(&view, anchor, now) else {
                continue;
            };
            if let Err(e) = self.fire_once(&provider, &vctx, &view, idle_ms).await {
                symbio_core::plugin_warn!(
                    "heartbeat",
                    "会话 {} 的心跳本轮未触发（下个 tick 重试）: {e:?}",
                    view.id
                );
                continue;
            }
            // 锚点只在**触发成功**后推进（理由见文件头「触发失败为什么不写锚点」）
            self.mark_fired(&view.id, now).await;
            fired += 1;
            symbio_core::plugin_info!("heartbeat", "已触发会话 {} 的心跳任务", view.id);
        }
    }

    /// 开一轮 = 向 `<会话>/inbox` 写一次（写臂负责落格与入队，见 `D-076` #4）。
    async fn fire_once(
        &self,
        provider: &DynVdfsProvider,
        vctx: &VdfsContext,
        view: &SessionView,
        idle_ms: i64,
    ) -> VdfsResult<()> {
        let hb = HeartbeatConfig::from_metadata(&view.metadata);
        let message = trigger_message(&view.id, &hb, idle_ms);
        let body = serde_json::to_string(&message)
            .map_err(|e| symbio_core::VdfsError::Internal(e.to_string()))?;
        provider
            .dispatch(
                vctx,
                &format!("{PLUGIN_ID_SESSION}/{}/{SEG_INBOX}", view.id),
                VdfsRequest::Write {
                    content: VdfsContent::text(body),
                },
            )
            .await
            .map(|_| ())
    }
}

/// 取会话挂载根的清单（本插件的**唯一**读通道）。
async fn list_sessions(
    provider: &DynVdfsProvider,
    vctx: &VdfsContext,
) -> VdfsResult<Vec<symbio_core::VdfsItem>> {
    match provider
        .dispatch(vctx, PLUGIN_ID_SESSION, VdfsRequest::List {})
        .await?
    {
        VdfsResponse::List(items) => Ok(items),
        _ => Err(symbio_core::VdfsError::Internal(
            "vdfs/list 未返回清单".to_string(),
        )),
    }
}

#[cfg(test)]
#[path = "scheduler.test.rs"]
mod tests;
