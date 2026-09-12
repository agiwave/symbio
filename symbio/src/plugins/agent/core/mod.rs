//! OAB（Open Agent Bundle）协议核心 —— 规范的 Rust 参考实现。
//!
//! ## 隔离原则（宪法级约束）
//!
//! 本模块树（`core/`）**不得 import 任何 `symbio_core` 类型**——只允许使用
//! serde / serde_json / std 等外部库。它是 OAB 规范（见
//! `docs/design/open-agent-bundle-spec.md`）的自洽实现，未来可整体抽出为
//! 独立规范库 / SDK，供任何宿主复用。
//!
//! ## 协议是什么
//!
//! OAB **不是双协议**（没有 OAB-Engine / OAB-Provider 之分）。它就是一套
//! 「约定目录 + manifest」的**组合格式（composition format）**：
//!
//! - `manifest.yaml` 承载无法从目录推导的元信息（身份 / 兼容门槛 / 权限 / 配置）；
//! - `prompts/` `skills/` `mcps/` 三个约定目录承载能力单元，**存在即安装**，
//!   无需任何 provider 配置文件。
//!
//! 装配（扫描约定目录、合并提示词与工具）是**宿主的责任**，协议只定义目录
//! 约定与数据结构。`core/` 仅提供纯数据的扫描与装配逻辑（`spec::assembly`），
//! 不规定任何传输 / 引擎 / 运行时。
//!
//! ## 模块结构
//!
//! - [`spec`]：manifest 数据结构 + 加载期校验 + 约定目录装配（协议静态 + 扫瞄部分）

#![allow(dead_code)]

pub mod spec;

/// 协议版本标识（manifest.spec），固定 `"oab/v1"`
pub const SPEC_ID: &str = "oab/v1";

/// spec 主版本号（`requires.spec: "^1"` 的兼容判定基准）
pub const SPEC_MAJOR: u64 = 1;
