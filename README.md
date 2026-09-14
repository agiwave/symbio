# Symbio

> **一个可组合、多协议的 AI Agent 平台**：用一套「分形插件」机制把对话、认知记忆、工具调用、外部集成统一编排起来，并同时提供 Rust 核心库、Tauri 桌面端和命令行入口。

| | |
| --- | --- |
| **核心库** | `symbio/`（Rust）— 全部业务逻辑：插件路由、LLM 多协议适配、工具调用循环、会话持久化、Agent 认知体系 |
| **桌面端** | `tauri/`（Tauri + Vue 3）— UI 渲染与 IPC 适配，后端仅暴露 3 个命令 |
| **命令行** | `cli/`（Rust）— 纯 Rust 终端前端：REPL / 单次 / 管道 / 心跳守护，进程内直连插件树 |

---

## 这是什么

Symbio 让你用**路径寻址**的方式调用任意能力（例如 `agent/chat`、`local/shell`、`model/chat`）。
所有能力都以"插件"形式存在，插件可以无限嵌套组合，从而把多智能体协作、长期记忆、外部工具（MCP / Web / 本地 shell / Telegram）编排进同一棵可寻址的插件树。

核心库 `symbio` **不依赖 UI**，可被桌面应用、命令行或后端服务复用。

### 你能用它做什么

- **智能体会话**：会话可绑定一个 Agent（OAB Bundle：人格提示词 + `prompt` / `skill` / `mcp` 三类子实体），由 `session` 统一编排工具调用循环；Agent 还可经 `agent_run` 能力委托子智能体。
- **统一 LLM 接入**：`model` 插件内置 OpenAI Chat / OpenAI Responses / Anthropic Messages / Gemini 四类协议适配器（模型资源经 `.vdfs/model` 寻址），支持流式与工具调用；由 `session` 在会话循环中直连调用。
- **工具与集成**：本地 shell / 文件读写、Web 请求与搜索、技能（skill）、MCP server 注册与调用、Telegram 消息通道。
- **会话与上下文**：`session/` 负责长连接消息持久化、历史裁剪与上下文压缩。
- **可扩展**：新能力只需实现 `Plugin` 并注册，即可挂入插件树、被 LLM 通过 `traverse("available_tools")` 自动发现。

---

## 前端 / 后端职责

| 层 | 目录 | 职责 |
| --- | --- | --- |
| **核心（后端）** | `symbio/` | 插件路由、LLM 多协议适配、工具调用循环、会话持久化、Agent 认知体系、存储后端 |
| **桌面端（前端）** | `tauri/` | Vue 3 组件 + Pinia 状态 + `services/`；通过 Tauri IPC 与后端通信，仅渲染 UI |
| **适配层** | `tauri/src-tauri/` | 薄适配层，仅 **3 个 Tauri command**：`route_v2` / `route_v2_send` / `route_v2_close` |
| **命令行前端** | `cli/` | 纯 Rust CLI 客户端（REPL / 单次 / 管道），进程内直连插件树，与 tauri 同源协议（详见 [cli/README.md](./cli/README.md)） |

设计原则：**UI 只做配置与展示，所有逻辑都在核心库**。桌面端不实现业务规则，只是核心库的一个宿主。

---

## 架构亮点（简述）

> 详细设计见 [docs/architecture/OVERVIEW.md](./docs/architecture/OVERVIEW.md)，请求全链路见 [docs/architecture/DATA_FLOW.md](./docs/architecture/DATA_FLOW.md)。

- **分形路由**：用 `/` 分隔的路径定位任意能力，容器与叶子插件接口完全一致。
- **LLM 原生**：递归收集插件树中的工具定义，深度支持 Function Calling。
- **VDFS 虚拟文件系统**：资源型插件（`agent` / `skill` / `mcp` / `model` / `session` / `setting`）统一以 `.vdfs/<插件名>` 挂载点对外，前端按后端下发的注册表与详情页定义动态渲染。
- **插件互不可见**：工具、选项与人格片段统一由 `traverse` 收集进 `CapabilityVisitor`；插件之间不直接引用，只依赖 `symbio_core` 的共享契约。

```
桌面端 / CLI  ──(route_v2)──►  Home / ── worker(Composite) ──┬─ agent / session / model
                                                            ├─ local / web / skill / mcp
                                                            └─ telegram
HTTP/WS 客户端 ──(gateway 插件)──►  Home /        setting / hook / event_bus 直挂根下
```

---

## 实际插件清单 (`symbio/src/plugins/`)

| 插件 | 角色 | 关键能力 |
| --- | --- | --- |
| `home` | 根容器 | 持工作区配置、挂载 `worker`（Composite 实例） |
| `composite` | 动态容器 | 按配置实例化任意子插件，是"分形"的关键 |
| `agent` | 智能体资产 | OAB Bundle 宿主：装配人格片段、声明 MCP、经 `traverse` 贡献 `agent_identity` 与 `agent_run` 能力 |
| `session` | 会话中心 | 会话编排唯一入口：工具调用循环、提示词组装、消息持久化与上下文压缩 |
| `model` | LLM 网关 | 无状态单轮推理（`ModelProvider::execute_turn`，由 session 收集后直调，不占路由），多协议适配（OpenAI Chat / Responses / Anthropic / Gemini） |
| `local` | 本地工具 | shell / file_read / file_write / file_edit / glob_search / content_search |
| `web` | Web 工具 | http_request / web_search / web_fetch |
| `skill` | 技能 | 加载与执行技能定义 |
| `mcp` | MCP 桥 | MCP server 注册（stdio / http）与工具调用 |
| `telegram` | Telegram 通道 | 消息收发与人机交互 |
| `gateway` | 入站网关 | HTTP/WebSocket 入站（与 route_v2 同构），外部客户端接入 |
| `setting` | 配置 | 系统级配置读写 |
| `hook` | 钩子 | 钩子注册与触发 |
| `event_bus` | 事件总线 | 进程内帧广播（连接级 SSE 风格推送） |

**Agent 路由**（源码：`symbio/src/plugins/agent/host/handlers.rs`）：`agent/bundle/list` · `bundle/get` · `bundle/upload` · `bundle/export` · `bundle/delete` · `bundle/preview`；实体侧为 `agent/entities/*`（Bundle 条目是容器，内部托管 `prompt` / `skill` / `mcp` 三类子实体）。

**Agent 贡献的能力**（经 `traverse` 注册进能力收集器，不占路由）：`agent_identity`（取回当前智能体完整人格文本）· `agent_run`（委托子智能体）。

---

## 快速开始

### 运行桌面端（推荐）

```bash
cd tauri
npm install
npm run tauri dev
```

### 导入智能体（OAB Bundle）

智能体以 OAB Bundle 形式经插件树导入，无独立二进制入口：导入路径为 `.vdfs/agent` 的**新建类型 `zip`**（载荷为 bundle 整包）。示例包见 [`examples/fullstack-dev/`](./examples/fullstack-dev)，包规范见 [OAB 规范](./docs/design/open-agent-bundle-spec.md)。

### 运行命令行前端（CLI）

纯 Rust 的第二种前端形态，复用同一套插件树与协议，仅替换传输层（Tauri IPC → 进程内直连）。

```bash
cd cli
node scripts/build-cli.mjs                                          # 构建（注入 C 工具链 + 路径归一化）
../symbio/target/debug/symbio-cli.exe \
  --homedir "D:\Bing\symbio\.symbio" -m "你好"                     # 非交互单条
../symbio/target/debug/symbio-cli.exe --homedir "D:\Bing\symbio\.symbio"   # 交互 REPL
```

文档：[cli/README.md](./cli/README.md) · [架构](./cli/docs/architecture.md) · [构建](./cli/docs/building.md) · [用法](./cli/docs/usage.md)。

### 编译与测试核心库

```bash
cd symbio
cargo build --lib
cargo test --lib                    # 运行全部单元测试
cargo clippy --lib --tests -- -D warnings   # 质量门禁（warning 视为 error）
```

---

## 质量指标

| 指标 | 现状 |
| --- | --- |
| 单元测试 | Rust：`cargo test --lib` 全通过（用例数随迭代增长，文档不锁死具体数字）；前端：vitest 覆盖纯逻辑层（`npm test`） |
| Clippy 警告 | 0（CI `-D warnings` 门禁） |
| cargo fmt | 0 diff（CI `--check` 门禁） |
| 前端类型检查 | vue-tsc 0 错误（CI 门禁） |
| 循环依赖 | 0 |
| 核心插件数 | 14 |

---

## 文档

详细文档见 **[文档中心 (docs/README.md)](./docs/README.md)**，快速导航：

- **架构**：[OVERVIEW](./docs/architecture/OVERVIEW.md) · [数据流与调用链](./docs/architecture/DATA_FLOW.md) · [协议规范](./docs/architecture/PROTOCOLS.md) · [决策记录](./docs/DECISIONS.md)
- **参考**：[路由清单](./docs/reference/ROUTES.md) · [错误码](./docs/reference/ERROR_CODES.md) · [配置参考](./docs/reference/CONFIGURATION.md)
- **开发**：[插件开发指南](./docs/guides/PLUGIN_DEVELOPMENT.md) · [排障手册](./docs/guides/TROUBLESHOOTING.md) · [贡献指南](./CONTRIBUTING.md)
- **现行设计**：[实体提供者机制](./docs/design/entity-provider-mechanism.md) · [上下文压缩分层总览](./docs/design/context-compression-design.md) · [OAB 规范](./docs/design/open-agent-bundle-spec.md)
- **模块文档**：每个插件与前端各自维护 `README.md`（详见 [文档中心的模块文档地图](./docs/README.md#模块文档地图)）
- **CLI 前端**：[cli/README.md](./cli/README.md) · [架构](./cli/docs/architecture.md) · [构建](./cli/docs/building.md) · [用法](./cli/docs/usage.md)
- **更新日志**：[CHANGELOG](./docs/CHANGELOG.md) · **历史归档**：[archive/](./docs/archive/)

---

## 仓库结构

```
symbio/
├── cli/                 # 纯 Rust CLI 前端（进程内直连插件树；文档见 cli/README.md）
├── tauri/               # Vue 3 桌面端（约 13K 行 TS/Vue）
│   └── src-tauri/       # 仅 3 个 Tauri command 的薄适配层
├── symbio/              # Rust 核心库
│   ├── src/
│   │   ├── symbio_core/ # 公共契约（Plugin trait / InvokeRequest / 路径常量）
│   │   ├── plugins/     # 14 个私有 plugin 实现（各含 README.md）
│   │   ├── providers/   # 基础设施实现（向量嵌入、文件存储）
│   │   ├── init.rs      # 日志初始化 + 根插件装配
│   │   └── lib.rs
│   └── Cargo.toml
├── tauri/docs/          # 前端文档（FRONTEND.md）
├── docs/                # 系统级文档（跨模块；历史归档在 docs/archive/）
├── scripts/             # 工具脚本
├── .github/             # CI / Release
├── README.md
├── CONTRIBUTING.md
├── LICENSE              # MIT
└── clippy.toml / rustfmt.toml
```

## 许可证

本项目采用 [MIT License](./LICENSE) 协议。
