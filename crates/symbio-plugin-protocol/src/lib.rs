//! 抽象协议设计插件 —— **协议提案 + 第二方加入 + 读数**（名册 #43，DE-026 §2.3）。
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 工具（`Capability`） | `protocol_propose` / `protocol_join` | 模型（经工具面调用） |
//! | 行为（`ToolBehavior`） | 一次成功提案/加入 ⇒ 一条领域事实草稿 | 宿主唯一的写口（I-1） |
//! | 事实（`EventDecl` / `FactDecl` / 归宿行） | `protocol.proposed` / `protocol.joined` 落在 `domain × asserted` | 写闸按声明校验名字与格位 |
//! | 路由（`plugin_routes`） | `protocol/catalog`：第二方的**只读**读入口 | 外部调用方（网关 / 另一个实例） |
//! | 视图（`ViewDecl` + `shared_views`） | `protocol.catalog` 折出协议清单与成立状态 | 任何按名字读的插件（D-221） |
//!
//! `kernel` / `sdk` / `loop` / `session` 一行都不动；`plugin-api` 只有生成物
//! （`routes.rs` 增 `ROUTE_PROTOCOL_CATALOG`、`view_names.rs` 增 `protocol.catalog`）。
//!
//! ## 「协议需要第二方」怎么落
//!
//! 一条提案只有提出方，**不算成立**；视图按**不同参与者**计数：1 个仍是 `draft`，
//! 2 个及以上才是 `established`。`protocol_join` 是加入口，`protocol/catalog` 是
//! 第二方的读入口（它在网关侧可直接调用，不需要先知道本插件的任何类型）。
//!
//! ## 路由为什么只读
//!
//! 写口只有一个（I-1，D-213）：路由拿事实源读现状，不拿它写。加入仍走工具与行为端口，
//! 于是「谁写的」在转写里有 `produced_by`，不需要再给路由开一条旁路。

mod behavior;
mod decl;
mod plugin;
mod tool;
