//! 驱动环的**通用契约**：宿主实现，环使用。
//!
//! 本模块只有中性形状——债键 / 事实 / 债行 / 出口。**没有端口名，也没有
//! 业务事实名**：端口名与种类都是值，由数据源给出；这里只规定形状，不解释语义。
//! 判据与模块边界见 `docs/003-design/051-机制形状规格.md`。
//!
//! 注册走项目既有的**能力收集机制**：环在 `traverse` 里往 [`DRIVE_FAMILY`] 放一行
//! [`DrivePayload`]（载荷是 [`DriveRunner`]）；宿主 `list` 该族取回执行者。能力束是
//! **调用参数**，不是注册面。宿主因此**不认识环的任何类型**，环也不认识宿主。

use std::fmt;
use std::sync::Arc;

use serde_json::Value;
use symbio_kernel::capability::{CapabilityFamily, CapabilityRow};

use crate::port::{PortExecHandles, PortExecutor, PortFactEmit, PortFactReader, PortReply};

/// 环协议里「应答落账」的事实名（机制词汇，不是世界词）。
pub const DRIVE_CLOSING_OK: &str = "response";

/// 环协议里「判死落账」的事实名（机制词汇，不是世界词）。
pub const DRIVE_CLOSING_FAILED: &str = "failed";

/// 驱动执行者所在的唯一 `multi` 族（与 `port` / `debt` 同一手法：族是值，加族不改内核）。
pub const DRIVE_FAMILY: CapabilityFamily = CapabilityFamily::multi("drive");

/// 驱动执行者：给一份中性能力束，跑到没有可还的债为止。
///
/// 内容（能力束）是参数；注册面只有 [`DRIVE_FAMILY`] 的那一行。
#[async_trait::async_trait]
pub trait DriveRunner: Send + Sync {
    /// 驱动本会话；报告原样交回宿主。
    async fn drive(&self, support: Arc<dyn DriveSupport>) -> Result<DriveReport, String>;
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

/// 债身份（中性形状）：端口名 + 欠债方派生的 id + 租约尝试号。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DriveKey {
    /// 端口名（数据；环不解释）。**拥有所有权**：世界事实的端口位常常来自运行期
    /// 解析（如入库身份），不是编译期常量。
    pub port: String,
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
    /// 事实种类（字符串透传；五类之一由环判定）。**拥有所有权**：投影来源既有
    /// 环协议事实，也有任意世界事实，种类在运行期才知道。
    pub kind: String,
    /// 这笔账的身份。
    pub key: DriveKey,
    /// 载荷：宿主原样保存，不解释内容。
    pub payload: Value,
    /// 记账时刻（毫秒）。环的「现在」取本字段的最大值（`D-376`），不另读墙钟。
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
        match self.kind.as_str() {
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
    /// 种类级：本会话里有一条这个种类的事实。
    Kind(String),
    /// 身份级：本会话里有一条这个种类、这个身份的事实。
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
    /// 触发条件（种类 + 身份）：该条输入事实存在才欠（与 `when` 同一条读面）。
    pub when_id: Option<(String, String)>,
    /// 排除条件：该事实已存在 ⇒ 本行不欠（与 `when` 同一条读面）。
    pub unless: Option<Unless>,
    /// 该端口的债全部还清才欠（读债状态）。
    pub when_no_open: Option<&'static str>,
    /// 前提（读债状态）：该端口**这一轮**还有人认领（可派发 / 在飞 / 挂起）⇒ 本行
    /// 不进计划；没人认领 ⇒ 前提已了（**缺席即收束**）。与 [`Self::when_no_open`]
    /// 对偶：依赖方只写端口名，不写兄弟插件的行名与债 id。
    pub after_port: Option<&'static str>,
    /// 单次派发租约（毫秒）。
    pub lease_ms: u64,
    /// 最多尝试次数（含首次）。
    pub max_attempts: u32,
}

/// 驱动出口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveOutcome {
    /// 没有可还的债，且账上没有未收束的债。
    NoDebt,
    /// 只剩**在飞的债**（已派发、租约未过）：环自己还能推进（应答落格或租约到期都会
    /// 再踢一次）。调用方**不得**据此认为本轮已空闲——更不得据此投影或收口。
    Waiting(Vec<DriveKey>),
    /// 只剩**需要外部事实才能推进**的债（等答复 / 等授权 / 等超时唤醒）：环自己已经
    /// 没有可动作了，这才把控制权交还调用方。
    ///
    /// 三种「停」的分界：本态与 [`Self::Waiting`] 的区别是「活还在不在自己手上」——
    /// 在飞 = 还在（继续等）；只有外部事实能推进 = 不在（交还）。与 [`Self::Exhausted`] /
    /// [`Self::Stalled`] 的区别是停得正不正当——后两者是没跑完的安全阀，本态是正常的
    /// 一轮结束（还在等，但等的不是环）。
    NeedsHuman(Vec<DriveKey>),
    /// 步数预算用尽：还欠着债，本轮不再继续。
    ///
    /// 这是**安全阀，不是正常出口**：正常一轮的步数远低于预算。调用方必须把它
    /// 当成「这一轮没跑完」处理，不能记 `idle`，也不能静默吞掉。
    Exhausted {
        /// 实际走完的步数（一圈一次 `plan`）。
        steps: usize,
        /// 仍未收束的债。
        outstanding: Vec<DriveKey>,
    },
    /// 一圈没有新增任何事实：继续跑只会自旋，因此停下并交回未收束的债。
    Stalled(Vec<DriveKey>),
}

/// 一次驱动报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveReport {
    /// 本次实际派发笔数。
    pub dispatched: usize,
    /// 出口。
    pub outcome: DriveOutcome,
}

/// 宿主交给环的**全部中性能力**：按字符串派发执行体、按本会话读写事实、
/// 按「种类」查存在性，以及时钟与并发上限。
///
/// 实现归宿主（唯一实现方）；环只调用，不解释端口名与事实种类。
pub trait DriveSupport: Send + Sync {
    /// 本会话的事实快照（只读投影）。
    fn facts(&self) -> Vec<DriveFact>;

    /// 落一条事实；重复投递的去重规则归宿主。
    fn append(&self, fact: DriveFact);

    /// 本会话可还的债（行数据由申报方决定）。
    fn rows(&self) -> Vec<DriveRow>;

    /// 按端口名解析执行体——清单发现归宿主。
    fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>>;

    /// 「这种事实存在吗」——宿主看得见全部事实，环只透传字符串。
    fn exists(&self, kind: &str) -> bool;

    /// 「这种事实、这个身份存在吗」——与 [`Self::exists`] 同一条读面，
    /// 只多看一位身份。环只透传，不解释种类与身份。
    fn exists_id(&self, kind: &str, id: &str) -> bool {
        let _ = (kind, id);
        false
    }

    /// 「这种事实在 `since_ms` 之后落过格吗」——与 [`Self::exists`] 同一条读面，
    /// 只多看一位时刻（挂起债重燃的触发判断）。默认实现走 [`Self::facts`]；
    /// 宿主若维护**本轮计划快照**，应覆盖它，使计划内的三次存在性查询读同一份。
    fn exists_since(&self, kind: &str, since_ms: u64) -> bool {
        self.facts()
            .iter()
            .any(|fact| fact.kind == kind && fact.at_ms > since_ms)
    }

    /// 同轮并发派发的上限（宿主给的配置数据；环不写死常数）。
    fn concurrency(&self) -> usize;

    /// 非环协议事实的去处（世界事实留在应用侧写口）。
    fn outlet(&self) -> PortFactEmit {
        PortFactEmit::silent()
    }

    /// 交给执行体的事实读面（宿主实现）。环只**原样转交**，不调用、不解释。
    fn reader(&self) -> Option<Arc<dyn PortFactReader>> {
        None
    }

    /// 交给执行体的不透明句柄束（宿主实现）。环只**原样转交**，不读取其中任何字段。
    fn exec_handles(&self) -> PortExecHandles {
        PortExecHandles::default()
    }
}
