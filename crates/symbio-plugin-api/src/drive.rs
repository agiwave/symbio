//! 驱动环的**通用契约**：宿主实现，环使用。
//!
//! 本模块只有中性形状——可见域 / 债键 / 事实 / 债行 / 出口。**没有端口名，也没有
//! 业务事实名**：端口名与种类都是值，由数据源给出；这里只规定形状，不解释语义。
//! 判据与模块边界见 `docs/003-design/051-机制形状规格.md`。
//!
//! 注册走项目既有的**能力收集机制**：环在 `traverse` 里往 [`DRIVE_FAMILY`] 放一行
//! [`DrivePayload`]（载荷是 [`DriveRunner`]）；宿主 `list` 该族取回执行者。能力束与
//! 可见域是**调用参数**，不是注册面。宿主因此**不认识环的任何类型**，环也不认识宿主。

use std::fmt;
use std::sync::Arc;

use serde_json::Value;
use symbio_kernel::capability::{CapabilityFamily, CapabilityRow};

use crate::port::{PortExecutor, PortFactEmit, PortFactReader, PortReply};

/// 环协议里「应答落账」的事实名（机制词汇，不是世界词）。
pub const DRIVE_CLOSING_OK: &str = "response";

/// 环协议里「判死落账」的事实名（机制词汇，不是世界词）。
pub const DRIVE_CLOSING_FAILED: &str = "failed";

/// 驱动执行者所在的唯一 `multi` 族（与 `port` / `debt` 同一手法：族是值，加族不改内核）。
pub const DRIVE_FAMILY: CapabilityFamily = CapabilityFamily::multi("drive");

/// 驱动执行者：给一个可见域与一份中性能力束，跑到没有可还的债为止。
///
/// 内容（能力束 / 可见域）是参数；注册面只有 [`DRIVE_FAMILY`] 的那一行。
#[async_trait::async_trait]
pub trait DriveRunner: Send + Sync {
    /// 驱动一个可见域；报告原样交回宿主。
    async fn drive(
        &self,
        support: Arc<dyn DriveSupport>,
        scope: u64,
    ) -> Result<DriveReport, String>;
}

/// `drive` 行的载荷。
///
/// 消费方经 `row.payload::<DrivePayload>()` 下转型取回——类型是声明方与消费方之间的
/// 私有约定，表不参与校验（与 `PortPayload` 同一条开放载荷形状）。
#[derive(Clone)]
pub struct DrivePayload(Arc<dyn DriveRunner>);

impl DrivePayload {
    /// 由执行者造载荷。
    pub fn new(runner: Arc<dyn DriveRunner>) -> Self {
        Self(runner)
    }

    /// 取执行者。
    pub fn runner(&self) -> Arc<dyn DriveRunner> {
        Arc::clone(&self.0)
    }
}

impl fmt::Debug for DrivePayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DrivePayload")
    }
}

/// 造一行 `drive` 族登记（`active = false`，与 port / debt 同一手法）。
pub fn drive_row(name: &str, runner: Arc<dyn DriveRunner>) -> CapabilityRow {
    CapabilityRow::new(
        DRIVE_FAMILY,
        format!("<drive>{name}"),
        DrivePayload::new(runner),
        false,
    )
}

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

/// `unless` 谓词：与 `when` 走**同一条读面**（世界事实，按种类读；不只环的五事实）。
///
/// 两种形态都是「存在即不欠」：种类级问「有没有这种事实」，身份级再问「是不是这条」。
/// **债表永不读载荷**：需要按内容分叉时，由写方把结论写成一条具体事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unless {
    /// 种类级：本可见域里有一条这个种类的事实。
    Kind(String),
    /// 身份级：本可见域里有一条这个种类、这个身份的事实。
    Id {
        /// 事实种类（申报方给的字符串）。
        kind: String,
        /// 事实身份（入库事件的 id）。
        id: String,
    },
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
    /// 排除条件：该事实已存在 ⇒ 本行不欠（与 `when` 同一条读面）。
    pub unless: Option<Unless>,
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

    /// 「这种事实、这个身份、这个可见域，存在吗」——与 [`Self::exists`] 同一条读面，
    /// 只多看一位身份。环只透传，不解释种类与身份。
    fn exists_id(&self, kind: &str, id: &str, scope: u64) -> bool {
        let _ = (kind, id, scope);
        false
    }

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

