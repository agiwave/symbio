//! 会话的世界端口（`port=session`）：声明位与执行体（`DE-048` D-315）。
//!
//! ## 为什么会话是端口，不是环的一部分
//!
//! 会话配置、动作面（转写 / 压缩 / 补充）、事实史读取回答的是「这个世界此刻允许这轮
//! 看到什么、能改什么」，不是「这一笔债欠不欠、还了没有」——按 `DE-047` §1 的判定式
//! 它们既不是七项职务，也写不成 `decl` 的两行式子，所以归**世界端口**
//! （`I-6`：端口身份是数据，环只按端口名解析提供者）。
//!
//! ## 四条行契约
//!
//! | 行 | 载荷（中性键） | 回体 |
//! |---|---|---|
//! | `session-config` | `{}` | 八个纯值字段（会话身份 / 主体 / 作用域 / 上下文窗 / 持久化 / 设置 / 两个存档目录） |
//! | `session-call` | `{ op, payload }` | `{ data }` |
//! | `session-call-unit` | `{ op, payload }` | `{}` |
//! | `session-facts` | `{}` | `{ events }` |
//!
//! 载荷里的 [`ROW_KEY`] 是**行判别键**（中性键）：一条 `request(port=session)` 靠它
//! 指向上面四条契约之一；`op` 仍取 `run_ops` 那个装配期冻结的闭集（`D-094`），端口
//! 不为每个动作加一行。
//!
//! ## 回体放在 `PortOutcome::facts`
//!
//! 环**不解释**端口载荷与产出（`I-6.2`）：`PortOutcome::facts` 里的每一项由引擎原样
//! 搬进 `response` 的载荷（`{"facts": [...]}`）。本端口约定 **`facts` 的第一项就是
//! 回体**，且只回纯值（标量 / 数组 / 事件），不回环类型、不回 `LoopSession` 概念。
//!
//! ## 执行体不读环状态
//!
//! 执行体只读 [`PortExecContext`] 的两个只读面：`ctx`（本轮交付包 `TURN_DISPATCH` 与
//! 会话句柄）和 `emit`。它不回调环、不读环的在途债，也不持有 `Store` 句柄——历史
//! 切片从交付包里的 `facts` 取（`LoopSession::facts_snapshot` 的同一条来源）。

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use serde_json::{json, Value};

use symbio_kernel::actors::Scope;
use symbio_kernel::event::Seq;
use symbio_kernel::plugin::PluginInvokeRequestExt;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider,
};
use symbio_sdk::chat::principal::Principal;
use symbio_sdk::turn::dispatch::{TurnDispatch, TurnSpec, TURN_DISPATCH};

use crate::plugin::SessionPlugin;

/// 端口名（数据，不是类型：环按名字解析提供者，`DE-047` I-6.1）。
pub(crate) const SESSION: &str = "session";

/// 四条行契约的名字（同时是 `Decl.name` 与载荷判别键的取值）。
pub(crate) const ROW_CONFIG: &str = "session-config";
pub(crate) const ROW_CALL: &str = "session-call";
pub(crate) const ROW_CALL_UNIT: &str = "session-call-unit";
pub(crate) const ROW_FACTS: &str = "session-facts";

/// 载荷里的行判别键（中性键，不出现世界名）。
pub(crate) const ROW_KEY: &str = "row";

/// 本端口的提供者：只申报端口名与 id，执行面在 [`SessionPortExecutor`]。
///
/// 持有插件的是 [`Weak`]：端口行由活插件在 `traverse` 里挂上，插件被停用 / 卸载时
/// 执行体 `upgrade` 失败 ⇒ **fail-closed**（不 panic、不退回第二份读法）。
pub(crate) struct SessionPortProvider {
    executor: Arc<SessionPortExecutor>,
}

impl SessionPortProvider {
    pub(crate) fn new(plugin: Weak<SessionPlugin>) -> Self {
        Self {
            executor: Arc::new(SessionPortExecutor { plugin }),
        }
    }
}

impl PortProvider for SessionPortProvider {
    fn ports(&self) -> &'static [&'static str] {
        &[SESSION]
    }

    fn provider_id(&self) -> &str {
        SESSION
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::clone(&self.executor) as Arc<dyn PortExecutor>)
    }
}

/// 一笔债的执行体：按 [`ROW_KEY`] 选行契约，回体写进 `PortOutcome::facts`。
pub(crate) struct SessionPortExecutor {
    plugin: Weak<SessionPlugin>,
}

#[async_trait]
impl PortExecutor for SessionPortExecutor {
    async fn execute(&self, ctx: PortExecContext, input: PortInput) -> PortOutcome {
        let reply = match input.payload.get(ROW_KEY).and_then(Value::as_str) {
            Some(ROW_CONFIG) => config_reply(&ctx),
            Some(ROW_FACTS) => facts_reply(&ctx),
            Some(ROW_CALL) => self.call_reply(&ctx, &input.payload, false).await,
            Some(ROW_CALL_UNIT) => self.call_reply(&ctx, &input.payload, true).await,
            Some(other) => Err(format!("未知端口行：`{other}`")),
            None => Err(format!(
                "载荷缺 `{ROW_KEY}` 字段（端口 `{}`）",
                input.port
            )),
        };
        match reply {
            Ok(value) => PortOutcome {
                facts: vec![value],
                error: None,
            },
            Err(why) => PortOutcome::failed(PortErrorClass::NonRetryable, why),
        }
    }
}

impl SessionPortExecutor {
    /// `session-call` / `session-call-unit`：走 session 的装配期冻结行表（`D-094`）。
    ///
    /// `unit = true` ⇒ 只回执（`{}`）；回体为空的动作不因端口而变形。
    async fn call_reply(
        &self,
        ctx: &PortExecContext,
        payload: &Value,
        unit: bool,
    ) -> Result<Value, String> {
        let op = payload
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| "载荷缺 `op` 字段".to_string())?;
        let body = payload.get("payload").cloned().unwrap_or(Value::Null);
        // 插件不在 ⇒ fail-closed：端口行还挂在表上（装配期收集的行不会自删），
        // 但执行体已失去宿主持有者，不能退回「再建一份」。
        let plugin = self
            .plugin
            .upgrade()
            .ok_or_else(|| "会话插件不在（端口执行体失去宿主持有者）".to_string())?;
        let dispatch = dispatch_of(ctx)?;
        let host = ctx
            .ctx
            .as_ref()
            .ok_or_else(|| "缺少宿主上下文".to_string())?;
        let data = plugin.run_op_row(host, &dispatch, op, Some(body)).await?;
        Ok(if unit { json!({}) } else { json!({ "data": data }) })
    }
}

/// `session-config`：把本轮交付包里的 [`TurnSpec`] 投影成八个纯值字段。
///
/// 投影而非转交：`TurnSpec` 是宿主→session 的交付值，环不再持它；回体只带环会读的
/// 那八格（`D-315` 行表第一行）。
fn config_reply(ctx: &PortExecContext) -> Result<Value, String> {
    Ok(config_value(&dispatch_of(ctx)?.spec))
}

/// `session-facts`：本会话事实史的纯值投影（`Store::range(Seq::new(0))`）。
///
/// 来源是交付包里的 `facts`（临时会话 ⇒ `None` ⇒ 空序列），不是执行体自己持的
/// 存储句柄——`D-315` 的「读面已由事实面覆盖」。
fn facts_reply(ctx: &PortExecContext) -> Result<Value, String> {
    let dispatch = dispatch_of(ctx)?;
    let events = dispatch
        .facts
        .as_ref()
        .map(|store| store.range(Seq::new(0)))
        .unwrap_or_default();
    Ok(json!({ "events": events }))
}

/// 本轮交付包：执行体只从宿主 ctx 的这一个只读键取（不读环的在途状态）。
fn dispatch_of(ctx: &PortExecContext) -> Result<Arc<TurnDispatch>, String> {
    let host = ctx
        .ctx
        .as_ref()
        .ok_or_else(|| "缺少宿主上下文（端口执行体读不到本轮交付包）".to_string())?;
    host.get(TURN_DISPATCH)
        .ok_or_else(|| "宿主上下文没有本轮交付包（`TURN_DISPATCH`）".to_string())
}

/// `TurnSpec` → 回体。键与 `LoopSession` 的八个读面逐条对应。
fn config_value(spec: &TurnSpec) -> Value {
    json!({
        "session_id": spec.session_id,
        "principal": Principal::from_legacy(&spec.principal),
        "scope": scope_value(&spec.scope),
        "context_window": spec.context_window,
        "persistent": spec.persistent,
        "settings": spec.settings,
        "tool_archive_dir": spec.tool_archive_dir.to_string_lossy(),
        "transcript_archive_dir": spec.transcript_archive_dir.to_string_lossy(),
    })
}

/// 作用域纯值：`{"root": true}` 或 `{"root": false, "parent": <父主体>}`。
fn scope_value(scope: &Scope) -> Value {
    match scope {
        Scope::Root => json!({ "root": true }),
        Scope::Child(parent) => json!({ "root": false, "parent": parent }),
    }
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
