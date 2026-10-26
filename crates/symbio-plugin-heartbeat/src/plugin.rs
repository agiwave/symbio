//! Heartbeat 插件本体：能力行贡献与常驻扫描循环的启停
//!
//! 本文件只干「一个插件要声明的那几件事」：自述、能力行（Tool + Option 两行）、
//! 常驻循环的启动与停机。判空闲的算术在 [`super::scheduler`]，设置入口在
//! [`super::tool`]，选项行声明在 [`super::option`]。
//!
//! ## 启动点为什么在 `build` 而不是 `start`
//!
//! [`Plugin::start`] 是**同步**钩子（[D-033](../../../docs/003-design/002-架构总览.md#d-033-生命周期钩子start-同步stop-异步停用与卸载各给理由 "DE-002-架构总览")：
//! 装配路径整条是同步的），而循环体是 task。session 的常驻任务同在此处
//! （`plugins/session/plugin.rs` 的 `start_resident_tasks`）——两处都在**构造点**交出句柄，
//! 「谁来持有句柄」因此只有一个注册点。
//!
//! ## 停机形状沿用 D-075 的 P1
//!
//! 一个停机位 + 一个唤醒 + 一个有界等待：循环体**先挂唤醒再读位**（反序会留一条
//! 「读完没停 ⇒ 挂唤醒之前置了位 ⇒ 从此睡死」的竞态窗口，而 `notify_waiters()`
//! 只叫醒已经在睡的人）。本片只有一条循环，句册因此退化成一格，判据不变。

use super::option::{heartbeat_field, ORDER_HEARTBEAT};
use super::tool::HeartbeatTool;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use symbio_core::{
    CapabilityRow, DynVdfsProvider, Plugin, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload, PluginStopReason,
    CAPABILITY_VISITOR, OPTION_VISITOR, PATH, TRAVERSE_AVAILABLE_OPTIONS, TRAVERSE_AVAILABLE_TOOLS,
};
use symbio_deploy::PLUGIN_ID_HEARTBEAT;
use tokio::sync::RwLock;

/// 停机时等扫描循环退出的上限：超时才 `abort`。
///
/// 循环体最长的不可取消段是一次 `vdfs/list` + 一次 `write(inbox)`，量级远小于此；
/// 这个上限是给**卡死的下游**兜底的，不是给正常节奏留的。
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Heartbeat 插件
pub struct HeartbeatPlugin {
    /// 父（容器）引用。**弱**引用：E-007 禁的是强引用持有兄弟实例，而本片要的只是
    /// 「经容器取本作用域的 VDFS 视图」，逐 tick `upgrade` 一次即可。
    parent: Option<Weak<dyn Plugin>>,
    /// 空闲锚点：会话 id → 最近一次**触发成功**的毫秒。
    ///
    /// 它是 `idle_baseline` 的下限那一半：磁盘侧
    /// `updated_at` 停在回合开始前（回合零写盘退出）时，靠它防止逐 tick 热循环重触发。
    /// 切出本插件前这一格住在宿主的 `heartbeat_state`，由回合侧的 `mark_activity` 维护；
    /// 切片后**只有本插件读写它**（[D-077](README.md#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出 "crates/symbio-plugin-heartbeat/src/README") 后果 #7）。
    anchors: RwLock<HashMap<String, i64>>,
    /// 常驻扫描循环的句柄（停机点据此等到它真的退出）。
    ///
    /// `std::sync::Mutex`：临界区只有 `take` / `put`，没有任何 `await`——S-002 按文件
    /// 文本判「std 锁 + 五行内有 await」，取放因此收成下面两个**不含 await** 的小方法。
    scan: Mutex<Option<tokio::task::JoinHandle<()>>>,
    stopping: AtomicBool,
    shutdown_wake: tokio::sync::Notify,
}

impl HeartbeatPlugin {
    pub fn new(parent: Option<Weak<dyn Plugin>>) -> Self {
        Self {
            parent,
            anchors: RwLock::new(HashMap::new()),
            scan: Mutex::new(None),
            stopping: AtomicBool::new(false),
            shutdown_wake: tokio::sync::Notify::new(),
        }
    }

    /// 静态工厂（满足 `submit_object_creator!` 协议）
    ///
    /// 本插件**没有自己的配置文档**：可调的东西全是**会话的**属性（间隔 / 提示词 /
    /// 携带历史），落在 `Session.metadata.heartbeat`；扫描节奏（tick / 每 tick 上限 /
    /// 抖动）是机制参数，不开放配置——把它做成一份 PLUGIN.yml 只会让「一个会话的空闲
    /// 判定」有两处可调的地方。
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let plugin = Arc::new(HeartbeatPlugin::new(ctx.parent()));
        plugin.start_scan();
        plugin
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_HEARTBEAT, "心跳任务")
            .with_description(
                "会话空闲后的自主开轮调度：扫描会话清单，把提示词写进空闲会话的收件箱",
            )
            .with_version("0.1.0")
    }

    /// 起常驻扫描循环。没有 runtime 时（裸构造的单测）**一条都不起**：那种场景没有
    /// 会话要跑，构造方不因此失败，而「没启动过」在停机侧就是空格，无事可做。
    pub(crate) fn start_scan(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let me = self.clone();
        let join = handle.spawn(async move { me.run_scan_loop().await });
        self.record_scan(join);
    }

    /// 记句柄（只在构造点写一次）。
    fn record_scan(&self, join: tokio::task::JoinHandle<()>) {
        if let Ok(mut slot) = self.scan.lock() {
            *slot = Some(join);
        }
    }

    /// 取句柄（只在停机点写一次）。取走不等于销毁：等完由 [`Self::restore_scan`] 还回来，
    /// 判据用例要凭在册的句柄断言「真的退出了」。
    ///
    /// 中毒只意味着**别处**已经 panic 过，这一格里没有一个需要保守的不变量。
    fn take_scan(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.scan
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn restore_scan(&self, join: tokio::task::JoinHandle<()>) {
        if let Ok(mut slot) = self.scan.lock() {
            *slot = Some(join);
        }
    }

    #[cfg(test)]
    pub(crate) fn scan_handle_finished(&self) -> bool {
        let Ok(slot) = self.scan.lock() else {
            return false;
        };
        slot.as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished)
    }

    /// 停机握手：返回（已挂好的唤醒, 当前停机位）。**先挂唤醒再读位**——理由见文件头。
    pub(crate) fn stop_gate(&self) -> (tokio::sync::futures::Notified<'_>, bool) {
        let wake = self.shutdown_wake.notified();
        (wake, self.stopping.load(Ordering::Acquire))
    }

    /// 本作用域的 VDFS 视图（经容器）。取不到 = 容器已释放 = 本轮无事可做。
    pub(crate) fn provider(&self) -> Option<DynVdfsProvider> {
        let parent = self.parent.as_ref().and_then(Weak::upgrade)?;
        parent.get_vfs_provider()
    }

    /// 父引用的一份拷贝：给「本轮合成的请求体」当宿主态用（见
    /// [D-077](README.md#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出 "crates/symbio-plugin-heartbeat/src/README") 裁决三——经容器取数**需要**一个
    /// 宿主态承载访客表，`VdfsContext::empty()` 在容器转发下当场失败）。
    pub(crate) fn parent_handle(&self) -> Option<Weak<dyn Plugin>> {
        self.parent.clone()
    }

    /// 锚点读出（`None` = 本进程里没见过这个会话）。
    pub(crate) async fn anchor_of(&self, id: &str) -> Option<i64> {
        self.anchors.read().await.get(id).copied()
    }

    /// 锚点写入（只在**触发成功**后）。
    pub(crate) async fn mark_fired(&self, id: &str, at: i64) {
        self.anchors.write().await.insert(id.to_string(), at);
    }
}

#[async_trait]
impl Plugin for HeartbeatPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn stop(self: Arc<Self>, _reason: PluginStopReason) -> Result<(), PluginError> {
        self.stopping.store(true, Ordering::Release);
        self.shutdown_wake.notify_waiters();

        // JoinHandle 不是 Clone，但它是 Unpin —— `&mut handle` 本身就是 Future，
        // 可以 timeout 之后再 abort；句柄等完还回册，判据用例据此断言「真的退出了」。
        let Some(mut join) = self.take_scan() else {
            return Ok(());
        };
        if tokio::time::timeout(STOP_WAIT, &mut join).await.is_err() {
            symbio_core::plugin_warn!("heartbeat", "扫描循环未在停机窗口内退出，强制取消");
            join.abort();
        }
        self.restore_scan(join);
        Ok(())
    }

    /// 本插件不占用**路由面**：它对世界的三句话全是地址面（见模块文档）。
    ///
    /// 留着这个 `impl` 是因为 `Plugin::route` 没有默认体。任何路径都回答
    /// `NotFound`，不猜、不回落。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "[heartbeat] 本插件没有路由（一切读写走地址面）：{path}"
        )))
    }

    /// 能力收集：贡献两行——`heartbeat` 工具（模型侧设置入口）与 `heartbeat` 选项行
    /// （会话选项栏那一格）。
    ///
    /// 命中哪条广播就注册哪一行，两次广播各走各自的收集器（工具族
    /// [`CAPABILITY_VISITOR`] / 选项族 [`OPTION_VISITOR`]），与会话的其他贡献方
    /// （agent → 智能体、model → Model）同构。**没有第三行**：本插件不暴露 VDFS
    /// 挂载点，它对世界说的三句话都是从**别人**的地址面读写的。
    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        if path == TRAVERSE_AVAILABLE_TOOLS {
            if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
                visitor
                    .put(CapabilityRow::tool(Arc::new(HeartbeatTool::new(
                        Arc::downgrade(&self),
                    ))))
                    .await;
            }
        }
        if path == TRAVERSE_AVAILABLE_OPTIONS {
            if let Some(visitor) = ctx.get(OPTION_VISITOR) {
                visitor
                    .put(CapabilityRow::option(ORDER_HEARTBEAT, heartbeat_field()))
                    .await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_core::submit_object_creator!(PLUGIN_ID_HEARTBEAT, HeartbeatPlugin::build, dyn Plugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
