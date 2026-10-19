//! 驱动器侧的**会话窄句柄**：把一次轮次的交付包（[`TurnDispatch`]）收成
//! 驱动器实际会用的那几个读法。
//!
//! ## 为什么要有这一格
//!
//! 搬家前驱动器直接持 `Arc<PersistentChatSession>`（会话引擎）。切插件之后
//! 那份引擎留在 session 墙内，驱动器手里只有三样东西：纯值 [`TurnSpec`]、
//! 本会话事实轨（`Option<Arc<dyn Store>>`）、动作面端口（[`SessionPort`]）。
//! 本结构把这三样收成一个对象，**读法与旧句柄对齐**——于是执行链上的调用点
//! 只需换类型、不必重写语义。
//!
//! ## 它不持有存储实现
//!
//! `facts` 的类型是 core 的 [`Store`] trait object（`TurnDispatch` 已构造好），
//! 不是 `EventWalStore`/`SessionStore`——loop 不命名会话存储实现（E-009 的
//! 同一纪律）。「读完整史」在这条 trait 上就是 `range(Seq::new(0))`。
//!
//! ## 写方法一律走端口
//!
//! 转写写入的唯一落点是 session（转写唯一写入点 + 事实写者令牌都在墙内）。
//! 所有会话动作都经 [`SessionPort::call`] 递过去——这正是「主动发起流程走
//! 统一端口/协议」的形态。动作集是**装配期冻结的闭集**（session 行表），
//! 本结构不为每个动作设包装：两个泛化助手（`run_op` / `run_op_unit`）
//! 加调用点的回型标注就是全部（[ADR-094](../../../../docs/010-adr/094.md "ADR-094")）。

use std::path::Path;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::symbio_core::{Event, PluginError, Scope, Seq, Store, TurnSettings, TurnSpec};

use super::session_port::SessionPort;

/// 驱动器侧的会话句柄（构造点唯一：loop 的 `provider.rs`）。
pub(crate) struct LoopSession {
    spec: TurnSpec,
    facts: Option<Arc<dyn Store<Event = Event>>>,
    port: Arc<dyn SessionPort>,
}

impl LoopSession {
    pub(crate) fn new(
        spec: TurnSpec,
        facts: Option<Arc<dyn Store<Event = Event>>>,
        port: Arc<dyn SessionPort>,
    ) -> Self {
        Self { spec, facts, port }
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.spec.session_id
    }

    /// 作用域（子会话 ⇒ `Child(父主体)`，顶层 ⇒ `Root`；session 从元数据预计算）。
    pub(crate) fn scope(&self) -> Scope {
        self.spec.scope.clone()
    }

    pub(crate) fn context_window(&self) -> u64 {
        self.spec.context_window
    }

    pub(crate) fn is_persistent(&self) -> bool {
        self.spec.persistent
    }

    pub(crate) fn settings(&self) -> &TurnSettings {
        &self.spec.settings
    }

    pub(crate) fn skill_compile_enabled(&self) -> bool {
        self.spec.settings.skill_compile_enabled
    }

    pub(crate) fn skill_fast_path(&self) -> bool {
        self.spec.settings.skill_fast_path
    }

    /// 工具结果存档目录（session 在派发前按会话子目录算好）。
    pub(crate) fn tool_archive_dir(&self) -> &Path {
        &self.spec.tool_archive_dir
    }

    /// 压缩前历史转存目录（同上）。
    pub(crate) fn transcript_archive_dir(&self) -> &Path {
        &self.spec.transcript_archive_dir
    }

    /// 本会话事实轨（`None` = 临时会话：不落派生事实）。
    pub(crate) fn facts(&self) -> Option<Arc<dyn Store<Event = Event>>> {
        self.facts.clone()
    }

    /// 完整事实史（`facts = None` ⇒ 空）。
    ///
    /// 与「读会话消息」不是同一件事：这份是**事件网格**，派生事实与重建都读它。
    pub(crate) fn facts_snapshot(&self) -> Vec<Event> {
        self.facts
            .as_ref()
            .map(|s| s.range(Seq::new(0)))
            .unwrap_or_default()
    }

    /// 发一个内部动作并取回体：`op` 取 [`run_ops`] 常量，回体类型 `T` 由调用点
    /// 标注，载荷形状由 core 的请求结构体定（`run_op(run_ops::EDIT_NODES, EditNodesRequest { nodes, frame })`）。
    ///
    /// 这是本结构上**唯一**的动作出口——没有「一动作一方法」的宽面（ADR-094）：
    /// 动作集是 session 行表那个装配期冻结的闭集，加一个交互不用在这里加方法。
    pub(crate) async fn run_op<T: DeserializeOwned>(
        &self,
        op: &str,
        payload: impl Serialize,
    ) -> Result<T, PluginError> {
        let data = self.port.call(op, to_payload(op, payload)?).await?;
        serde_json::from_value(data)
            .map_err(|e| PluginError::InternalError(format!("会话动作回体不合法（{op}）：{e}")))
    }

    /// 发一个**无回体**的内部动作（session 侧只回执，`data: None`）。
    pub(crate) async fn run_op_unit(
        &self,
        op: &str,
        payload: impl Serialize,
    ) -> Result<(), PluginError> {
        self.port
            .call(op, to_payload(op, payload)?)
            .await
            .map(|_| ())
    }
}

/// 载荷序列化：失败只能是调用点传了不可序列化的值（如把 trait object 塞进请求体），
/// 那是编程错误，按 `Internal` 出声。
fn to_payload(op: &str, payload: impl Serialize) -> Result<serde_json::Value, PluginError> {
    serde_json::to_value(payload)
        .map_err(|e| PluginError::InternalError(format!("动作载荷序列化失败（{op}）：{e}")))
}
