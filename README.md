# Symbio

> **一个可组合、多协议的 AI Agent 平台**：用一套「分形插件」机制把对话、长期记忆、工具调用、外部集成统一编排起来，并同时提供 Rust 核心库、Tauri 桌面端和命令行入口。

| | |
| --- | --- |
| **核心库** | `symbio/`（Rust）— 全部业务逻辑：插件路由、LLM 多协议适配、工具调用循环、会话持久化、智能体资产与记忆 |
| **桌面端** | `tauri/`（Tauri + Vue 3）— UI 渲染与 IPC 适配，后端仅暴露 3 个命令 |
| **命令行** | `cli/`（Rust）— 纯 Rust 终端前端：REPL / 单次 / 管道 / 心跳守护，进程内直连插件树 |

---

## 这是什么

Symbio 让你用**路径寻址**的方式调用任意能力（例如 `session/chat/send`、`skill/execute`、`vdfs/list`）。
所有能力都以"插件"形式存在，插件可以无限嵌套组合，从而把多智能体协作、长期记忆、外部工具（MCP / Web / 本地 shell / Telegram）编排进同一棵可寻址的插件树。

核心库 `symbio` **不依赖 UI**，可被桌面应用、命令行或后端服务复用。

### 你能用它做什么

- **智能体会话**：会话可绑定一个 Agent（一个 Agent = 一个目录，按 [`agent-dir/v2`](./docs/design/agent-directory-spec.md) 装配：根 `AGENTS.md` 人格 + 复用宿主既有的 `skill` / `mcp` 插件子树），由 `session` 统一编排工具调用循环；Agent 还可经 `agent_run` 工具委托子智能体。
- **统一 LLM 接入**：`model` 插件内置 OpenAI Chat / OpenAI Responses / Anthropic Messages / Gemini 四类协议适配器（模型资源经 `<根>/model` 寻址），支持流式与工具调用；由 `session` 在会话循环中直连调用。
- **工具与集成**：本地 shell / 文件读写、Web 请求与搜索、技能（skill）、MCP server 注册与调用、Telegram 消息通道。
- **会话与上下文**：`session/` 负责长连接消息持久化、历史裁剪与上下文压缩。
- **可扩展**：新能力只需实现 `Plugin` 并注册，即可挂入插件树、被 LLM 通过 `traverse("available_tools")` 自动发现。

---

## 前端 / 后端职责

| 层 | 目录 | 职责 |
| --- | --- | --- |
| **核心（后端）** | `symbio/` | 插件路由、LLM 多协议适配、工具调用循环、会话持久化、智能体资产与记忆 |
| **桌面端（前端）** | `tauri/` | Vue 3 组件 + Pinia 状态 + `services/`；通过 Tauri IPC 与后端通信，仅渲染 UI |
| **适配层** | `tauri/src-tauri/` | 薄适配层，仅 **3 个 Tauri command**：`route_v2` / `route_v2_send` / `route_v2_close` |
| **命令行前端** | `cli/` | 纯 Rust CLI 客户端（REPL / 单次 / 管道），进程内直连插件树，与 tauri 同源协议（详见 [cli/README.md](./cli/README.md)） |

设计原则：**UI 只做配置与展示，所有逻辑都在核心库**。桌面端不实现业务规则，只是核心库的一个宿主。

---

## 架构亮点（简述）

> 详细设计见 [docs/architecture/OVERVIEW.md](./docs/architecture/OVERVIEW.md)，请求全链路见 [docs/architecture/DATA_FLOW.md](./docs/architecture/DATA_FLOW.md)。

- **分形路由**：用 `/` 分隔的路径定位任意能力，容器与叶子插件接口完全一致。
- **LLM 原生**：递归收集插件树中的工具定义，深度支持 Function Calling。
- **VDFS 虚拟文件系统**：资源型插件统一以 `<根>/<插件名>` 挂载点对外（挂载点全表见 [CURRENT.md](./docs/CURRENT.md) §1），前端按后端下发的注册表与详情页定义动态渲染。
- **插件互不可见**：工具、选项与人格片段统一由 `traverse` 收集进 `CapabilityVisitor`；插件之间不直接引用，只依赖 `symbio_core` 的共享契约。

> 分层结构与请求流转图见 [docs/SYSTEM_MAP.md](./docs/SYSTEM_MAP.md)——本文不重复画（手绘树易漂移）。

---

## 插件清单

> **权威事实表见 [docs/CURRENT.md](./docs/CURRENT.md)**——由 `scripts/gen-current-facts.mjs`
> 从代码提取生成、CI `--check` 门禁防漂移（插件 × 注册名 × VDFS 挂载点 × 自有路由 × trait × 配置）。
> 各插件的**一句话职责**与模块文档见 [文档中心 · 模块文档地图](./docs/README.md#模块文档地图)。
> 本文不再手抄这张表——手抄必漂移（曾漏 `vdfs` / `work`、残留已移除的 `agent_identity`）。

---

## 快速开始

### 运行桌面端（推荐）

```bash
cd tauri
npm install
npm run tauri dev
```

### 导入智能体（Agent 目录 / zip 整包）

智能体以目录整包（zip）形式经插件树导入，无独立二进制入口：导入路径为 `<根>/agent` 的**新建类型 `zip`**。示例包见 [`examples/fullstack-dev/`](./examples/fullstack-dev)，包规范见 [Agent 目录规范 v2](./docs/design/agent-directory-spec.md)（取代已归档的 OAB v1 规范）。

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
| 核心插件数 | 16（权威清单见 [CURRENT.md](./docs/CURRENT.md) §1） |

---

## 文档

详细文档见 **[文档中心 (docs/README.md)](./docs/README.md)**，快速导航：

- **架构**：[OVERVIEW](./docs/architecture/OVERVIEW.md) · [数据流与调用链](./docs/architecture/DATA_FLOW.md) · [协议规范](./docs/architecture/PROTOCOLS.md) · [决策记录](./docs/DECISIONS.md)
- **参考**：[当前事实表](./docs/CURRENT.md) · [路由清单](./docs/reference/ROUTES.md) · [错误码](./docs/reference/ERROR_CODES.md) · [配置参考](./docs/reference/CONFIGURATION.md)
- **开发**：[插件开发指南](./docs/guides/PLUGIN_DEVELOPMENT.md) · [排障手册](./docs/guides/TROUBLESHOOTING.md) · [贡献指南](./CONTRIBUTING.md)
- **现行设计**：[VDFS 虚拟动态文件系统](./docs/design/vdfs.md) · [上下文压缩分层总览](./symbio/src/plugins/session/docs/context-compression-design.md) · [Agent 目录规范 v2](./docs/design/agent-directory-spec.md)
- **模块文档**：每个插件与前端各自维护 `README.md`（详见 [文档中心的模块文档地图](./docs/README.md#模块文档地图)）
- **CLI 前端**：[cli/README.md](./cli/README.md) · [架构](./cli/docs/architecture.md) · [构建](./cli/docs/building.md) · [用法](./cli/docs/usage.md)
- **更新日志**：[CHANGELOG](./docs/CHANGELOG.md) · **历史归档**：[archive/](./docs/archive/)

---

## 仓库结构

```
symbio/
├── cli/                 # 纯 Rust CLI 前端（进程内直连插件树；文档见 cli/README.md）
├── tauri/               # Vue 3 桌面端
│   └── src-tauri/       # 仅 3 个 Tauri command 的薄适配层
├── symbio/              # Rust 核心库
│   ├── src/
│   │   ├── symbio_core/ # 公共契约（Plugin trait / InvokeRequest / 路径常量）
│   │   ├── plugins/     # 16 个插件实现（各含 README.md）
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
