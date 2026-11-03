//! 驱动器侧的**会话窄句柄**：把一次轮次的交付包（[`TurnDispatch`]）收成
//! 驱动器实际会用的那几个读法。
//!
//! ## 为什么要有这一格
//!
//! 搬家前驱动器直接持 `Arc<PersistentChatSession>`（会话引擎）。切插件之后
//! 那份引擎留在 session 墙内，驱动器手里只剩两样东西：纯值 [`TurnSpec`]（本结构
//! 只留**存档目录**两格）与动作面端口（[`SessionPort`]）。
//!
//! ## 配置与事实不再从这里读（P2-3b2-1）
//!
//! 八个配置格的唯一源是 `TurnSpec::view()` 产出的 `SessionView`（`session-config`
//! 回体），事实史的唯一源是 `TurnDispatch.facts`（`session-facts` 回体）。
//! 本结构因此不再带 `facts` 字段、不再有 `settings` / `principal` / `scope` /
//! `context_window` / `is_persistent` / `session_id` / `facts` / `facts_snapshot`
//! 读面——读它们会绕开端口那一条源。`spec` 只为两个存档目录留守（b2-2 再切）。
//!
//! ## 写方法一律走端口
//!
//! 转写写入的唯一落点是 session（转写唯一写入点 + 事实写者令牌都在墙内）。
//! 所有会话动作都经 [`SessionPort::call`] 递过去——这正是「主动发起流程走
//! 统一端口/协议」的形态。动作集是**装配期冻结的闭集**（session 行表），
//! 本结构不为每个动作设包装：两个泛化助手（`run_op` / `run_op_unit`）
//! 加调用点的回型标注就是全部（[D-094](../../../docs/003-design/014-单环一表.md#d-094-内部动作段注册化行表即闭集run_opsall-删除sessionport-收成一个-call "DE-014-单环一表")）。

use std::path::Path;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;

use symbio_kernel::plugin::PluginError;
use symbio_sdk::turn::dispatch::TurnSpec;

use super::session_port::SessionPort;

/// 驱动器侧的会话句柄（构造点唯一：loop 的 `provider.rs`）。
pub(crate) struct LoopSession {
    spec: TurnSpec,
    port: Arc<dyn SessionPort>,
}

impl LoopSession {
    pub(crate) fn new(spec: TurnSpec, port: Arc<dyn SessionPort>) -> Self {
        Self { spec, port }
    }

    /// 工具结果存档目录（session 在派发前按会话子目录算好）。
    pub(crate) fn tool_archive_dir(&self) -> &Path {
        &self.spec.tool_archive_dir
    }

    /// 压缩前历史转存目录（同上）。
    pub(crate) fn transcript_archive_dir(&self) -> &Path {
        &self.spec.transcript_archive_dir
    }

    /// 发一个内部动作并取回体：`op` 取 [`run_ops`] 常量，回体类型 `T` 由调用点
    /// 标注，载荷形状由 core 的请求结构体定（`run_op(run_ops::EDIT_NODES, EditNodesRequest { nodes, frame })`）。
    ///
    /// 这是本结构上**唯一**的动作出口——没有「一动作一方法」的宽面（D-094）：
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
