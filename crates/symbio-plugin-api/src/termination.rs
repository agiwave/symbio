//! 宿主侧簿记写方：每次驱动前后读同一份只读投影，谓词成立就交回收束事实。
//!
//! 形状与 `drive` / `debt` / `port` 同一手法：族是值，加写方不改宿主，也不改环。
//! 环对「终止」零知识——终止事实落进同一事实源，下一圈由同一投影读回。
//!
//! 判据与缺口见 `docs/003-design/051-机制形状规格.md` §5 与 D-367：
//!
//! - **谁有权终止**：在能力收集里往 [`TERMINATION_FAMILY`] 放一行 [`TerminationPayload`]
//!   的插件。权利是**声明数据**，不是代码分支；宿主按行收集，不认插件名。
//! - **落哪条事实**：写方交回 [`FactDraft`]，种类必须在声明表里（写闸按格位校验）。
//!   草稿**不指定写方**：落库前由宿主按事件声明解析 owner（I1）。
//! - **环怎么读到**：与任何世界事实一样经事实源 → 投影。环不加任何读取路径；
//!   需要它的债行用 `unless` 谓词按名字问，宿主只负责把同一份投影交给写方。

use std::fmt;
use std::sync::Arc;

use symbio_kernel::capability::{CapabilityFamily, CapabilityRow};

use crate::drive::DriveFact;
use crate::FactDraft;

/// 簿记写方所在的唯一 `multi` 族（与 `drive` / `port` / `debt` 同一手法）。
pub const TERMINATION_FAMILY: CapabilityFamily = CapabilityFamily::multi("termination");

/// 一次观察的只读输入。
///
/// 宿主只交**投影**与「本次驱动是否已穷尽」两件事：写方不读环的内部状态，
/// 也不自行判「有没有未还债」——那是宿主的结论（`NoDebt`）。
#[derive(Debug, Clone, Copy)]
pub struct TerminationView<'a> {
    /// 本会话全量投影（世界事实 + 环协议事实），按入库顺序升序。
    pub facts: &'a [DriveFact],
    /// 宿主本次驱动已穷尽（`NoDebt`）。驱动前观察时恒为 `false`。
    pub exhausted: bool,
}

/// 簿记写方：谓词成立就交回收束事实草稿。
///
/// 幂等由写方自己保证（同一谓词在前后两次观察里成立时，事实键必须相同）；
/// 事实源按 `event_id` 吸收重复，本层不再做去重。
pub trait TerminationWriter: Send + Sync {
    fn observe(&self, view: &TerminationView<'_>) -> Vec<FactDraft>;
}

/// `termination` 行的载荷。
///
/// 消费方经 `row.payload::<TerminationPayload>()` 下转型取回——类型是声明方与
/// 消费方之间的私有约定，表不参与校验（与 `DrivePayload` 同一条开放载荷形状）。
#[derive(Clone)]
pub struct TerminationPayload(Arc<dyn TerminationWriter>);

impl TerminationPayload {
    /// 由写方造载荷。
    pub fn new(writer: Arc<dyn TerminationWriter>) -> Self {
        Self(writer)
    }

    /// 取写方。
    pub fn writer(&self) -> Arc<dyn TerminationWriter> {
        Arc::clone(&self.0)
    }
}

impl fmt::Debug for TerminationPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TerminationPayload")
    }
}

/// 造一行 `termination` 族登记（`active = false`，与 port / debt / drive 同一手法）。
pub fn termination_row(name: &str, writer: Arc<dyn TerminationWriter>) -> CapabilityRow {
    CapabilityRow::new(
        TERMINATION_FAMILY,
        format!("<termination>{name}"),
        TerminationPayload::new(writer),
        false,
    )
}
