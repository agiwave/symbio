//! 轮中汇报的**落点与能力口**（DE-015 的 D-163 + `D-222` 裁决 5）。
//!
//! ## 这一侧只剩两样东西
//!
//! 汇报的执行体是 [`super::turn_exec::run_report`]，动作在环上的 `policy.report` 行
//! 里被选中——它和另外十四行借的是同一块 [`super::turn_exec::Ctx`]，因此不再有
//! 「另一台 host 的 `Step`」。这里留下的是那一行**自己造不出来**的两半：
//!
//! - [`ReportPort`]：措辞能力口。编排器、请求句柄与轮首的会话视图都在 `chat_loop`
//!   那一侧，落格与上线在这一侧，所以交下来的只能是**能力**（产一句什么话），
//!   不是**决定**（要不要说）——后者已经在行表与视图里；
//! - [`ReportLanding`]：那句话的两个面。实时桥与落库镜像都在本轮的执行侧，
//!   而它们由装配段交出、由动作写入。
//!
//! ## 「一段静默」为什么仍只在批次边界可观测
//!
//! 判据（`report_due` 的 `at_boundary`）要求最后一条模型应答是一批**已关账**的工具
//! 调用，所以这一行在任何时刻至多欠一笔，且落地即关账。它现在由同一个环按 `prio`
//! 选中，而不是由批次边界另起一次派发——环里其它行照旧与它竞争，先还小的。

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use symbio_core::chat_node::ChatNode;

/// 汇报正文的**能力口**：给一份运行现状，交回一句要说的话（`None` = 不说）。
///
/// 与 [`super::turn_exec::SupplementFn`] 同族：编排层手里有 compose 的能力，而落格
/// 与上线在这一侧。它**只产文本**——判定（该不该说）在行表，配额与静默在视图，
/// 这里都不掺。这正是旧 `RoundHookFn` 退场的那一半：回调不再回答「要不要说」，
/// 也不再自己数批次。
pub(crate) type ReportTextFn = Arc<
    dyn Fn(
            symbio_kernel::dialog::RunSnapshot,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send>>
        + Send
        + Sync,
>;

/// 汇报的**另一半出口**：这句话得同时出现在两个面上。
///
/// - `collected`：本轮落库镜像的收集器（`persist_messages` 只写它的尾部增量），
///   不推就等于「实时面看见了、重启后没了」；
/// - `live`：实时桥（`UiBridge`）。**产生即上线**是这条产线的目的：攒到轮末再送，
///   一次长工具轮就全程无反馈；
/// - `no`：同一幕里第几句汇报的序号（事件 id `rp-{turn}-{no}`；与轮内补充共用这一个
///   计数器，两者用的是同一个 `<turn>-<no>` 名字空间）。
///
/// `Clone` 是因为动作要在一次派发里同时够到两面（`Arc` 各自加一，不是第二个真源）。
#[derive(Clone)]
pub(crate) struct ReportLanding {
    pub(crate) collected: Arc<Mutex<Vec<ChatNode>>>,
    pub(crate) live: Arc<super::turn_exec::UiBridge>,
    pub(crate) no: Arc<AtomicU64>,
}

/// 编排层交给一幕的一次性能力包：这一幕的汇报开没开（外层 `None` = compose 产线关着）。
///
/// 静默的右端读数（`now`）不在这里：执行体读的是本幕状态上的同一个值，与选债时
/// 喂给视图的那个逐字相同（判据 12——一条时钟）。
#[derive(Clone)]
pub(crate) struct ReportPort {
    /// 措辞能力口（compose 产线）。
    pub(crate) text: ReportTextFn,
}
