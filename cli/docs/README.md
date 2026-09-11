# symbio-cli 文档中心

`symbio-cli` 是与 [`../tauri`](../..) **并列**的第二种 Symbio 前端形态：纯 Rust 的命令行客户端，
复用同一套插件树与协议，只替换传输层。本目录收录它的详细文档。

## 文档地图

| 文档 | 内容 |
| --- | --- |
| [架构 (architecture.md)](./architecture.md) | 进程内客户端设计、`event_bus` 下行通道选择、终端渲染器、模式选择逻辑、消息增量合并模型 |
| [构建 (building.md)](./building.md) | 为什么不能裸 `cargo build`、C 工具链注入三要素、共享 target 的绝对路径陷阱、target 缓存修复 |
| [用法 (usage.md)](./usage.md) | 三种运行模式示例、完整参数表、REPL 内置命令、输出通道约定、系统目录与 Provider 语义、已知边界 |

## 速览

- **入口**：`cli/src/main.rs`（`#[tokio::main]`，解析 → 启动 → 发送 → 渲染）
- **客户端**：`cli/src/client.rs`（进程内 `SymbioClient`，订阅 `event_bus/subscribe`）
- **渲染器**：`cli/src/render.rs`（stdout = 模型文本，stderr = 进度/工具/错误/日志）
- **参数**：`cli/src/args.rs`（手写解析，不引 clap，守最小依赖）
- **构建**：`cli/scripts/build-cli.mjs`（注入 C 工具链并归一化路径分隔符）

> 设计铁律：CLI **不改动** `symbio/` 的任何一行代码、不改动协议。它只是把「请求 →
> `SimpleRequest`」这一层适配从 Tauri IPC 换成进程内直连。
