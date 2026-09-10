//! 网关插件：本应用作为「服务端」与「客户端」的协议与参数配置
//!
//! ## 职责
//!
//! 1. **入站（inbound）**：按配置启动/停止对外 HTTP/WebSocket 服务，使第三方或另一个
//!    Symbio 实例能像前端一样访问本应用。
//! 2. **出站（outbound）**：持久化「前端应以何种协议连接后端」的配置（进程内直连 or 远程 http），
//!    供前端 `callPlugin` 启动时读取，从而把整个前端指向另一个实例。
//! 3. **配置**：通过标准 `config/get`、`config/set` 暴露，并由设置页「开放接口」分区渲染。
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
