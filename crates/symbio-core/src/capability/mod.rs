//! 能力登记表、四元组与调用栈（`capability_invoke` 的机制一半）。
//!
//! **实现住 `symbio-sdk`**（D-258）：这一域的每条都只碰内核词汇与自己的表，不认容器概念。
//! 本模块只转发，`symbio_core::capability::…` 这条稳定路径不变。

// 本域实现已整体搬走（D-258 / D-259），这个文件只剩**登记面**（`core-naming-audit` N-004）：
// 仓内没有消费者，对外也不再经根转发（D-272）。下面的转出因此是「地址」而不是「依赖」。
#![allow(unused_imports)]

pub use symbio_sdk::capability::*;
