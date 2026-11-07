//! 工具世界插件：`port.tool` 的提供者（执行 / 路由 / 结果提取）。
//!
//! 本插件的职责边界只有一件事：把一次工具调用（[`PortInput`]）执行成结果事实
//! （[`PortOutcome`]）。**不持有轮次语义**、不做批编排、不写协议事实——那些由环的
//! 通用受理入口（`execute_one` / `run_port`）负责。
//!
//! 端口名是 `tool`（本插件自己定义），行声明与端口声明都随本插件
//! 提交（[`declarations`]），所以接入工具世界不需要改 loop 任何文件。
//!
//! [`PortInput`]: symbio_plugin_api::PortInput
//! [`PortOutcome`]: symbio_plugin_api::PortOutcome

mod breaker;
mod debt;
mod declarations;
mod plugin;
mod port;
mod result_guard;
mod round_done;
