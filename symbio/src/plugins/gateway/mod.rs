//! 网关插件：本应用作为「服务端」的协议与参数配置（仅入站）
//!
//! ## 职责
//!
//! 1. **入站（inbound）**：按配置启动/停止对外 HTTP/WebSocket 服务，使第三方或另一个
//!    Symbio 实例能像前端一样访问本应用。
//! 2. **配置**：通过标准 `config/get`、`config/set` 暴露，并由设置页「开放接口」分区渲染。
//!
//! ## 出站（前端连向何处）已不在本插件范畴
//!
//! 「前端以何种协议连接后端（native 直连 or 远程 http）」由前端「系统目录」切换器
//! 统一管理（localStorage 为权威），经 `initGatewayTransport` 决定。本插件的配置
//! 因此只描述「本实例如何被调用（入站）」，不再描述「前端连向何处」。
//!
//! ## 不做的事
//!
//! 本插件**不定义任何新协议**。所有对外端点都是既有 Tauri 命令的一对一映射，
//! 传输的是完全相同的 `PluginMessageWire` / `PluginPayloadWire` / `PluginFrame`。
//! 入站服务（`server.rs`）仅做「行李搬运」：把 HTTP/WS 请求翻译成 `SimpleRequest`，
//! 交给既有的分形路由（`parent.route(ctx)`），再把响应原样送回。
mod config;
mod plugin;
mod server;
