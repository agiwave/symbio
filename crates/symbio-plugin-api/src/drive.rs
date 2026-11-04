//! 驱动环的**通用契约**：宿主实现，环使用。
//!
//! 本模块只有中性形状——可见域 / 债键 / 事实 / 债行 / 出口。**没有端口名，也没有
//! 业务事实名**：端口名与种类都是值，由数据源给出；这里只规定形状，不解释语义。
//! 判据与模块边界见 `docs/003-design/051-机制形状规格.md`。
//!
//! 宿主经上下文键 [`DRIVE_SUPPORT`] 把实现交给环，经 [`DRIVE_SCOPE`] 指明本次可见域；
//! 环把报告经 [`DRIVE_REPORT`] 交回。宿主因此**不认识环的任何类型**，环也不认识宿主。

use std::sync::Arc;

use serde_json::Value;
use symbio_kernel::keys::SymbioKey;

use crate::port::{PortExecutor, PortFactEmit, PortFactReader, PortReply};

/// 环协议里「应答落账」的事实名（机制词汇，不是世界词）。
pub const DRIVE_CLOSING_OK: &str = "response";

/// 环协议里「判死落账」的事实名（机制词汇，不是世界词）。
pub const DRIVE_CLOSING_FAILED: &str = "failed";

/// 环入口在插件内的**相对臂**：宿主直呼环插件时放进 `PATH`。
pub const DRIVE_PATH: &str = "drive";

/// 债身份（中性形状）：可见域 + 端口名 + 欠债方派生的 id + 租约尝试号。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DriveKey {
    /// 债所属的可见域。
    pub scope: u64,
    /// 端口名（数据；环不解释）。
    pub port: &'static str,
    /// 欠债方派生的身份。
    pub id: String,
    /// 租约尝试号，从 1 起。
    pub attempt: u32,
}

/// 一条记账事实（中性形状）。
///
/// `kind` 的解释权在环；宿主只按同一形状读写，不认世界词汇。
#[derive(Debug, Clone)]
pub struct DriveFact {
    /// 这条事实所属的可见域。
    pub scope: u64,
    /// 事实种类（字符串透传；五类之一由环判定）。
    pub kind: &'static str,
    /// 这笔账的身份。
    pub key: DriveKey,
    /// 载荷：宿主原样保存，不解释内容。
    pub payload: Value,
    /// 记账时刻（毫秒，相对时间轴）。
    pub at_ms: u64,
    /// 收束事实携带的应答承载（只传递，不读世界字段）。
    pub reply: Option<PortReply>,
}

impl DriveFact {
    /// 是否为收束事实：`Some(true)` 应答、`Some(false)` 判死、`None` 非收束。
    ///
    /// 这两个名字是环协议的词汇（[`DRIVE_CLOSING_OK`] / [`DRIVE_CLOSING_FAILED`]），
    /// 不是任何业务事实名。
    pub fn closing(&self) -> Option<bool> {
        match self.kind {
            DRIVE_CLOSING_OK => Some(true),
            DRIVE_CLOSING_FAILED => Some(false),
            _ => None,
        }
    }
}

/// 一行债（中性形状）：申报方给出的数据，宿主原样转交环。
#[derive(Debug, Clone)]
pub struct DriveRow {
    /// 行名（申报方的标签）。
    pub name: &'static str,
    /// 欠哪个端口（数据）。
    pub port: &'static str,
    /// 债身份。
    pub id: String,
    /// 交给执行体的载荷。
    pub payload: Value,
    /// 前置债 id：它收束之前本行不可还。
    pub after: Option<String>,
    /// 触发条件：该**种类**的事实存在才欠。
    pub when: Option<String>,
    /// 排除条件：该 id 的事实已存在 ⇒ 本行不欠。
    pub unless: Option<String>,
    /// 该端口的债全部还清才欠（读债状态）。
    pub when_no_open: Option<&'static str>,
    /// 单次派发租约（毫秒）。
    pub lease_ms: u64,
    /// 最多尝试次数（含首次）。
    pub max_attempts: u32,
}

/// 驱动出口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveOutcome {
    /// 没有可还的债。
    NoDebt,
    /// 只剩在飞的债（租约未过）。
    Waiting(Vec<DriveKey>),
}

/// 一次驱动报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveReport {
    /// 当前可见域。
    pub scope: u64,
    /// 本次实际派发笔数。
    pub dispatched: usize,
    /// 出口。
    pub outcome: DriveOutcome,
}

/// 宿主交给环的**全部中性能力**：按字符串派发执行体、按可见域读写事实、
/// 按「种类 + 可见域」查存在性，以及时钟与并发上限。
///
/// 实现归宿主（唯一实现方）；环只调用，不解释端口名与事实种类。
pub trait DriveSupport: Send + Sync {
    /// 当前可见域的事实快照（只读投影）。
    fn facts(&self, scope: u64) -> Vec<DriveFact>;

    /// 落一条事实；重复投递的去重规则归宿主。
    fn append(&self, fact: DriveFact);

    /// 该可见域可还的债（行数据由申报方决定）。
    fn rows(&self, scope: u64) -> Vec<DriveRow>;

    /// 按端口名解析执行体——清单发现归宿主。
    fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>>;

    /// 「这种事实、这个可见域，存在吗」——宿主看得见全部事实，环只透传字符串。
    fn exists(&self, kind: &str, scope: u64) -> bool;

    /// 当前时刻（毫秒，宿主给的相对时间轴）。
    fn now_ms(&self) -> u64;

    /// 同轮并发派发的上限（宿主给的配置数据；环不写死常数）。
    fn concurrency(&self) -> usize;

    /// 非环协议事实的去处（世界事实留在应用侧写口）。
    fn outlet(&self, scope: u64) -> PortFactEmit {
        let _ = scope;
        PortFactEmit::silent()
    }

    /// 交给执行体的事实读面（宿主实现）。环只**原样转交**，不调用、不解释。
    fn reader(&self, scope: u64) -> Option<Arc<dyn PortFactReader>> {
        let _ = scope;
        None
    }
}

/// 驱动能力束的上下文键。
pub struct DriveSupportKey;

impl SymbioKey for DriveSupportKey {
    type Value = Arc<dyn DriveSupport>;
    const WIRE: bool = false;

    fn name(&self) -> &'static str {
        "drive_support"
    }

    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }

    fn format(&self, _v: &Self::Value) -> String {
        String::new()
    }
}

/// 驱动能力束的唯一实例。
pub const DRIVE_SUPPORT: DriveSupportKey = DriveSupportKey;

/// 本次驱动的可见域。
pub struct DriveScopeKey;

impl SymbioKey for DriveScopeKey {
    type Value = u64;

    fn name(&self) -> &'static str {
        "drive_scope"
    }

    fn parse(&self, s: &str) -> Option<Self::Value> {
        s.parse().ok()
    }

    fn format(&self, v: &Self::Value) -> String {
        v.to_string()
    }
}

/// 可见域键的唯一实例。
pub const DRIVE_SCOPE: DriveScopeKey = DriveScopeKey;

/// 驱动报告回的上下文键（进程内对象，环交回给宿主）。
pub struct DriveReportKey;

impl SymbioKey for DriveReportKey {
    type Value = DriveReport;
    const WIRE: bool = false;

    fn name(&self) -> &'static str {
        "drive_report"
    }

    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }

    fn format(&self, _v: &Self::Value) -> String {
        String::new()
    }
}

/// 报告键的唯一实例。
pub const DRIVE_REPORT: DriveReportKey = DriveReportKey;
