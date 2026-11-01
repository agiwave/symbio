//! 执行面（`ExecEnv` / 事件 / 中止）已在 `symbio_kernel`（D-188 C2b）。

// 本域实现已整体搬走（D-258 / D-259），这个文件只剩**登记面**（`core-naming-audit` N-004）：
// 仓内没有消费者，对外也不再经根转发（D-272）。下面的转出因此是「地址」而不是「依赖」。
#![allow(unused_imports)]

pub use symbio_kernel::exec::*;
