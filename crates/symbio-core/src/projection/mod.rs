//! 投影函数（行为）住这里；`Projection` 构造器是机制，住 `symbio_kernel`
//! （DE-023 D-180 / D-188 C2a）。
//!
//! 视图类型与投影函数跟消费方走；本模块只声明投影函数，并以同名
//! `Projection` 转出内核的构造器，既有路径 `crate::projection::…` 不变。

pub use symbio_kernel::Projection;

pub mod autonomy;
pub mod branch;
pub mod calibration;
pub mod changes;
pub mod checkpoint;
pub mod conation;
pub mod consolidate;
pub mod cost;
pub mod fallback;
pub mod mutations;
pub mod prompt_fidelity;
pub mod readyset;
pub mod rebuild;
pub mod recall;
mod recall_score;
pub mod reputation;
pub mod slo;
pub mod span;
pub mod transcript;
pub mod turns;
pub mod turnstate;
