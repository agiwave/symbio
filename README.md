# Symbio

> **一个可组合、多协议的 AI Agent 平台**：用一套「分形插件」机制把对话、认知记忆、工具调用、外部集成统一编排起来，并同时提供 Rust 核心库、Tauri 桌面端和命令行入口。

| | |
| --- | --- |
| **核心库** | `symbio/`（Rust）— 全部业务逻辑：插件路由、LLM 多协议适配、工具调用循环、会话持久化、Agent 认知体系 |
| **桌面端** | `tauri/`（Tauri + Vue 3）— UI 渲染与 IPC 适配，后端仅暴露 3 个命令 |
| **命令行** | `symbio/src/bin/seed_agents` — 批量灌入种子 Agent |

---

## 这是什么

Symbio 让你用**路径寻址**的方式调用任意能力（例如 `agent/chat`、`local/shell`、`model/chat`）。
所有能力都以"插件"形式存在，插件可以无限嵌套组合，从而把多智能体协作、长期记忆、外部工具（MCP / Web / 本地 shell / Telegram）编排进同一棵可寻址的插件树。

核心库 `symbio` **不依赖 UI**，可被桌面应用、命令行或后端服务复用。

### 你能用它做什么

- **多智能体对话**：`agent/chat` 接入具备长期认知记忆的 Agent；可创建多个角色化 Agent（`project_manager` / `architect` / `coder` / `reviewer` / `tester` / `documenter` / `devops`）。
- **统一 LLM 接入**：`model/chat` 内置 OpenAI Chat / OpenAI Responses / Anthropic Messages / Gemini 四类协议适配器，支持流式与工具调用。
- **工具与集成**：本地 shell / 文件读写、Web 请求与搜索、技能（skill）、MCP server 注册与调用、Telegram 消息通道。
- **会话与记忆**：`session/` 负责长连接消息持久化与历史裁剪；`agent` 提供认知单元（CU）存储与记忆操作（保存 / 检索 / 图谱查询 / 反思 / 合并）。
- **可扩展**：新能力只需实现 `Plugin` 并注册，即可挂入插件树、被 LLM 通过 `traverse("available_tools")` 自动发现。

---

## 前端 / 后端职责

| 层 | 目录 | 职责 |
| --- | --- | --- |
| **核心（后端）** | `symbio/` | 插件路由、LLM 多协议适配、工具调用循环、会话持久化、Agent 认知体系、存储后端 |
| **桌面端（前端）** | `tauri/` | Vue 3 组件 + Pinia 状态 + `services/`；通过 Tauri IPC 与后端通信，仅渲染 UI |
| **适配层** | `tauri/src-tauri/` | 薄适配层，仅 **3 个 Tauri command**：`route_v2` / `route_v2_send` / `route_v2_close` |
| **命令行** | `symbio/src/bin/seed_agents` | 批量灌入种子 Agent（幂等重建） |

设计原则：**UI 只做配置与展示，所有逻辑都在核心库**。桌面端不实现业务规则，只是核心库的一个宿主。

---

## 架构亮点（简述）

> 详细设计见 [docs/explanation/ARCHITECTURE.md](./docs/explanation/ARCHITECTURE.md)。

- **分形路由**：用 `/` 分隔的路径定位任意能力，容器与叶子插件接口完全一致。
- **LLM 原生**：递归收集插件树中的工具定义，深度支持 Function Calling。
- **机制化认知（v9）**：Agent 内部以"属性认知单元（prop CU）"驱动关系与展示机制化，新增认知类型无需改核心代码。
- **多存储后端**：Agent 认知存储支持 DirStorage（多 YAML 文件）与 SQLite，可热切换。

```
桌面端 / CLI  ──(route_v2)──►  Home / ── worker(Composite) ──┬─ agent / session / model
                                                            ├─ local / web / skill / mcp
                                                            └─ telegram / explorer / setting / hook / event_bus
```

---

## 实际插件清单 (`symbio/src/plugins/`)

| 插件 | 角色 | 关键能力 |
| --- | --- | --- |
| `home` | 根容器 | 持工作区配置、挂载 `worker`（Composite 实例） |
| `composite` | 动态容器 | 按配置实例化任意子插件，是"分形"的关键 |
| `agent` | 认知中心 | 对话、认知注入、提示词组装、Agent 管理、Mindscape 认知存储 |
| `session` | 会话中心 | 长连接、消息持久化、历史裁剪 |
| `model` | LLM 引擎 | 多协议适配、流式编排、工具调用循环 |
| `local` | 本地工具 | shell / file_read / file_write / file_edit / glob_search / content_search |
| `web` | Web 工具 | http_request / web_search / web_fetch |
| `skill` | 技能 | 加载与执行技能定义 |
| `mcp` | MCP 桥 | MCP server 注册（stdio / http）与工具调用 |
| `telegram` | Telegram 通道 | 消息收发与人机交互 |
| `explorer` | 文件浏览 | 文件/目录列表与读写 |
| `setting` | 配置 | 系统级配置读写 |
| `hook` | 钩子 | 钩子注册与触发 |
| `event_bus` | 事件总线 | 进程内帧广播（连接级 SSE 风格推送） |

**Agent 记忆操作**（`agent/capabilities/ops/memory/`，当前落地 5 个）：`save`（保存）· `retrieve`（检索，支持结构化过滤 + 语义召回）· `graph_query`（关系图谱查询）· `reflect`（基于历史更新认知）· `consolidate`（合并整理）。`delete` 已废除，改用 `save {confidence:0}` 软删除。

**Agent 路由**（`agent/handlers/`）：`agent/list` · `agent/get` · `agent/chat`（含流式）· `agent/create` · `agent/delete`（物理目录 + 缓存清理，幂等）。

---

## 快速开始

### 运行桌面端（推荐）

```bash
cd tauri
npm install
npm run tauri:dev
```

### 灌入种子 Agent（命令行）

```bash
cd symbio
cargo run --bin seed_agents          # 首次灌入 7 个角色
cargo run --bin seed_agents -- --recreate   # 强制重建（先删后建）
```

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
| 单元测试 | 全部通过（以 `cargo test --lib 2>&1 | tail` 实时输出为准） |
| Clippy 警告 | 0（除 MSRV 1.91 const 提示，无害） |
| 循环依赖 | 0 |
| 核心插件数 | 14 |

---

## 文档

详细文档见 **[文档中心 (docs/README.md)](./docs/README.md)**，快速导航：

- **教程**：[快速上手](./docs/tutorials/getting-started.md)
- **愿景**：[VISION](./docs/explanation/VISION.md) · **架构**：[ARCHITECTURE](./docs/explanation/ARCHITECTURE.md) · [运作机制](./docs/reference/OPERATION_MECHANISM.md) · [API 设计](./docs/reference/API_DESIGN.md)
- **开发**：[开发指南](./docs/how-to/DEVELOPMENT_GUIDE.md) · [编译指南](./docs/how-to/BUILD_GUIDE.md) · [插件开发指南](./docs/how-to/PLUGIN_DEVELOPMENT_GUIDE.md) · [结构规范](./docs/how-to/STRUCTURE_GUIDE.md)
- **插件自文档**：[agent](./symbio/src/plugins/agent/README.md)（含认知体系 / 架构 / 测试）
- **历史归档**：[设计讨论（历史）](./docs/archive/design_docs/HISTORY_AND_REVIEWS.md) · [更新日志](./docs/CHANGELOG.md)

---

## 仓库结构

```
symbio/
├── tauri/               # Vue 3 桌面端（约 13K 行 TS/Vue）
│   └── src-tauri/       # 仅 3 个 Tauri command 的薄适配层
├── symbio/              # Rust 核心库
│   ├── src/
│   │   ├── symbio_core/ # 公共契约（Plugin trait / InvokeRequest / 路径常量）
│   │   ├── plugins/     # 14 个私有 plugin 实现
│   │   ├── init.rs      # 对象创建注册 + 根插件装配
│   │   ├── lib.rs
│   │   └── bin/         # seed_agents
│   └── Cargo.toml
├── docs/                # 架构 / 开发 / 插件文档（历史归档在 docs/archive/）
├── scripts/             # 工具脚本
├── .github/             # CI / Release
├── README.md
├── CONTRIBUTING.md
├── LICENSE              # MIT
└── clippy.toml / rustfmt.toml
```

## 许可证

本项目采用 [MIT License](./LICENSE) 协议。
