//! `turn` —— 轮次面域：一次轮次**怎么交付**、**怎么造节点**、**产出什么记录**。
//!
//! **实现住 `symbio-sdk`**（D-259）：本域读的「轮」是**内核信封上的 `u64`**
//! （`symbio_kernel::event::Event::turn`，注释即「所属回合，N3 按它分桶」），
//! 不是要被持有或推进的容器。D-222 之后幕已降成身份（`turn` 端口整组退场、
//! `execute-turn` 改名 `announce-model`），本域因此只剩「按身份交付一份工作包 /
//! 按形状造节点 / 记一笔产物」三件没有生命周期决策的事。
//!
//! 本模块只转发，`symbio_core::turn::…` 与控制面沿用的根别名
//! （`symbio_core::TurnSpec` / `symbio_core::node_build::…`）都不变。
//! 域目录不删，是因为它是 `core-naming-audit` N-004 的登记面：搬的是住址，不是归属。

pub use symbio_sdk::turn::*;
