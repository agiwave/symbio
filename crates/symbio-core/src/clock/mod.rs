//! 当前时间（Unix 毫秒）的唯一实现。
//!
//! **本域的实现住 `symbio-sdk`**（D-258）：它只碰内核词汇与自己的表，不认容器概念，
//! 所以属于「插件可以依赖的宿主机制」那一层。本模块只转发，`symbio_core::clock::…` 这条
//! 稳定路径因此不变——控制面的 import 面不需要跟着搬家。
//!
//! **为什么转发而不是删掉目录**：core 的域目录是 `core-naming-audit` 与
//! `crates/symbio-core/README.md` 的登记面（N-004），一个域不能因为「实现搬走了」
//! 就从表里消失——搬的是住址，不是归属。

pub use symbio_sdk::clock::*;
