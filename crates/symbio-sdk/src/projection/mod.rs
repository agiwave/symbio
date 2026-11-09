//! 投影函数（行为）住这里；`Projection` 构造器是机制，住 `symbio_kernel`
//! （DE-023 D-180 / D-188 C2a）。
//!
//! 视图类型与投影函数跟消费方走；本模块只声明投影函数，不转出内核的构造器——
//! 内核符号只保留 `symbio_kernel::Projection` 一条路径（DE-018 的 D-419）。

pub mod branch;
pub(crate) mod elision;
pub mod mutations;
pub mod prompt_fidelity;
pub mod rebuild;
pub mod task_vocab;
pub mod transcript;
pub mod vocab;
