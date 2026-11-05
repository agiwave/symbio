//! 宿主驱动入口——**只认三样东西**：事实源、`drive` 能力行、事实信封里的 `scope`。
//!
//! 这一层**不认识端口名 / 业务事件名**，也不写任何业务事实：端口执行体与债行都由
//! 插件经 `port` / `debt` 族自报，`drive` 族由环自报，宿主只把「某个 scope 现在
//! 有新事实」这件事交给环。**加一个新端口 / 新能力 / 新债行，这里一行都不改。**
//!
//! ## 为什么没有 `loop { drive }`
//!
//! 外层每看到一条新事实只触发**一次**（CP-009）：环内部的轮次推进归环自己，
//! 它返回 `NoDebt` / `Waiting` 就结束。`Waiting` 等的是某个端口的**响应事实**，
//! 那条事实落地后由同一个入口再次触发——外层不持「正在跑」这类会话状态。
//!
//! ## 幂等
//!
//! 入口没有状态：同一个 scope 相邻两次触发，第一次把该 scope 的债还清后，第二次
//! 的债表现算是空的 ⇒ `NoDebt`。事实源是唯一输入，scope 是唯一域。

use std::sync::Arc;

use symbio_host::{DriveReport, Host};
use symbio_kernel::plugin::{
    Plugin, PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest,
};
use symbio_plugin_api::fact_source::{declared_fact_source_opener, FactSource, FACT_SOURCE};
use symbio_sdk::keys::WORKDIR;

/// 一次驱动需要的**世界**：事实源 + 通用上下文（会话 / 提供方 / 工作目录）。
///
/// 这里没有端口名，也没有业务事件名——环从 `drive` 族取能力行，从 `port` / `debt`
/// 族取执行体与债表。
pub struct FactWorld<'a> {
    pub facts: Arc<dyn FactSource>,
    pub session_id: &'a str,
    pub provider: Option<&'a str>,
    pub workdir: &'a str,
}

/// 装配层打开该会话的**唯一事实源**（经登记开面；插件不自建）。
pub fn open_fact_source(session_id: &str) -> Result<Arc<dyn FactSource>, String> {
    declared_fact_source_opener()
        .ok_or_else(|| "没有登记事实源开面".to_string())?
        .open(session_id)
}

/// 一条新事实落地后，对它信封里的 `scope` **单次**驱动。
///
/// 取「最新一条事实」的信封 `scope`——调用方不需要知道端口、债行，也不需要
/// 自己算 scope。事实源为空 ⇒ 没有可触发的域（错误，而不是一次空驱动）。
pub async fn drive_on_new_fact(
    root: &Arc<dyn Plugin>,
    world: &FactWorld<'_>,
) -> Result<DriveReport, String> {
    let head = world.facts.head();
    if head == 0 {
        return Err("事实源为空：没有可触发的 scope".to_string());
    }
    let mut latest = world.facts.range(head - 1);
    let Some(fact) = latest.pop() else {
        return Err("事实源 head 与事件表不一致".to_string());
    };
    let scope = fact.scope;

    // 父句柄交给执行体：工具 / 钩子按「调用方插件」路由，装配层给的是根插件
    // （环不碰这个字段，只原样转交）。
    let ctx = PluginSimpleRequest::new(Some(Arc::downgrade(root)), None);
    ctx.set(WORKDIR, world.workdir.to_string());
    // 装配层 open 的同一句柄：宿主与插件上下文共享它，插件不自建 store。
    ctx.set(FACT_SOURCE, Arc::clone(&world.facts));
    let base: Arc<dyn PluginInvokeRequest> = Arc::new(ctx);

    let host = Host::with_store(Arc::clone(root), Arc::clone(&world.facts));
    host.drive_at(scope, &base, world.session_id, world.provider)
        .await
}
