# Symbio

> **一个可组合、多协议的 AI Agent 平台**：一条 Rust 核心库 + 三种接入方式（桌面 / 命令行 / 网关），由**分形插件架构**统一编排对话、记忆、工具调用与外部集成。一条规则：每个能力都是一个**路径**。

| 层 | 目录 / 挂载点 | 职责 |
| --- | --- | --- |
| **核心库** | `symbio/` | 分形插件路由 · 多协议 LLM 适配 · 工具调用循环 · 会话持久化 · Agent 资产与记忆 |
| **桌面前端** | `tauri/` | Vue 3 + Pinia；`src-tauri/` 是只暴露 3 个 IPC 命令（`route_v2` / `route_v2_send` / `route_v2_close`）的薄适配层 |
| **命令行前端** | `cli/` | 纯 Rust：REPL / 单次 / 管道 / 心跳守护，进程内直连插件树 |
| **入站网关** | `gateway` 插件 | 树内插件：把 HTTP / WS 入站请求转发给父级路由（与 Tauri 的 `route_v2` 同构） |

设计原则：**UI 只做配置与展示，业务逻辑全在核心库**；三种接入共用同一套插件装配与同一条路径协议（各自建树，不共享内存）。**用它做什么**：多轮 Agent 会话 · 本地开发助手（shell / 搜索 / VDFS / 技能 / MCP） · 无头自动化（CLI 管道 / 网关 webhook） · 多供应商 LLM。

---

## 什么使 Symbio 与众不同

### 1. 分形插件 — 一种接口，任意嵌套

每个能力都是插件，每个插件都可包含子插件；容器（`home` / `composite`）与叶子（`local` / `web` / `model` …）**共享同一 `Plugin` trait**（`route` + `traverse`）。新增能力只要实现 `Plugin`、经 `inventory` 静态注册、挂到任意位置，无需改动核心。

```
home
└── worker (composite)
    ├── session · agent · model · local · vdfs · mcp · skill · web · telegram …
    └── gateway        ← HTTP / WS 入站（转发父级路由）
```

入口：Tauri（桌面）· CLI（进程内）· Gateway（入站）—— 同一套装配、同一条协议。

> 示意图，结构事实以 [CURRENT.md](docs/CURRENT.md) 为准。

### 2. 路径即路由 — LLM 用字符串呼唤工具

没有代码生成：模型输出 `vdfs_read` 就能读文件、`http_request` 就能发请求。插件树经 `traverse` 递归收集进 `CapabilityVisitor`，LLM **自动发现**全树的工具。

### 3. Session 作为唯一指挥

`session` 是**唯一**的工具调用循环入口：直连 `model` 执行回合，并负责历史裁剪与上下文压缩（L0–L6 分层，不耦合 LLM 传输层）。`agent` 只持有 Agent 目录（`AGENTS.md` 人格 + 技能 / MCP 子树装配），并以 `agent_run` 工具发起子智能体会话。因此一个会话可绑定任意 Agent，也可以不绑定。

### 4. VDFS — 把一切都当作地址

文件、模型、Agent、技能、MCP、设置……统统是资源插件，挂在 `<根>/<名称>` 下：**14 个前端操作 + 10 个面向 LLM 的 `vdfs_*` 工具**，一个心智模型。

### 5. Agent v2 — 插件树，而非独立二进制

Agent 就是一个目录：其中的技能 / MCP **复用宿主已有的** `skill` / `mcp` 子树，人格由根 `AGENTS.md` 表达（规范见下方文档表「设计」行）。

---

## 快速开始

### 桌面端（推荐）

```bash
cd tauri && npm install && npm run tauri dev
```

首次启动后在左侧**模型**面板新建条目填入 API Key（模型不是「设置」页的选项——资源型插件的配置就是它自己在 `<根>/model/<id>` 的资源树）。

### 导入智能体

在 `<根>/agent` 里用**新建类型 `zip`** 导入整包——Agent 没有独立二进制入口。示例包见 [`examples/fullstack-dev/`](examples/fullstack-dev)。

### 命令行

```bash
cd cli && node scripts/build-cli.mjs
../symbio/target/debug/symbio-cli.exe --homedir ~/.symbio -m "你好"   # 单条
../symbio/target/debug/symbio-cli.exe --homedir ~/.symbio             # REPL
```

### 编译与测试核心库

```bash
cd symbio && cargo build --lib && cargo test --lib
cargo clippy --lib --tests -- -D warnings
```

> 完整教程见 [快速上手](docs/guides/QUICK_START.md)。

---

## 事实速览

| 项目 | 现状 |
| --- | --- |
| 核心插件 | 16（权威清单见 [CURRENT.md](docs/CURRENT.md) §1；由代码生成，CI `--check` 防漂移） |
| LLM 协议 | 4 种：OpenAI Chat / OpenAI Responses / Anthropic / Gemini |
| 质量门禁 | Clippy 0 警告 · `cargo fmt` 0 diff · vue-tsc 0 错误 · vitest 通过 · 循环依赖 0（测试用例数随迭代增长，本文不锁死具体数字） |

---

## 仓库结构

```
symbio/
├── symbio/          # Rust 核心库：symbio_core（契约）· plugins（16 插件）· providers（嵌入 / 存储）
├── tauri/           # Vue 3 桌面端（src-tauri = 3 命令的薄适配层）
├── cli/             # 纯 Rust CLI 前端（共享 symbio 的 target 缓存）
├── docs/            # 系统级文档（架构 / 参考 / 指南 / 设计 / 归档）
├── e2e/             # 端到端用例（mock-llm / mock-mcp + 网关 WS）
├── examples/        # Agent 示例包
└── scripts/         # 事实表生成、静态审计与门禁
```

---

## 文档

| | |
| --- | --- |
| **入门** | [快速上手](docs/guides/QUICK_START.md) · [系统地图](docs/SYSTEM_MAP.md) |
| **架构** | [总览](docs/architecture/OVERVIEW.md) · [数据流](docs/architecture/DATA_FLOW.md) · [协议](docs/architecture/PROTOCOLS.md) · [决策记录](docs/DECISIONS.md) |
| **参考** | [事实表](docs/CURRENT.md) · [路由](docs/reference/ROUTES.md) · [错误码](docs/reference/ERROR_CODES.md) · [配置](docs/reference/CONFIGURATION.md) |
| **开发** | [插件开发](docs/guides/PLUGIN_DEVELOPMENT.md) · [排障](docs/guides/TROUBLESHOOTING.md) · [贡献指南](CONTRIBUTING.md) |
| **设计** | [VDFS 规范](docs/design/vdfs.md) · [上下文压缩分层](symbio/src/plugins/session/docs/context-compression-design.md) · [Agent 目录规范 v2](docs/design/agent-directory-spec.md) |
| **CLI** | [cli/README.md](cli/README.md) · [架构](cli/docs/architecture.md) · [构建](cli/docs/building.md) · [用法](cli/docs/usage.md) |
| **变更** | [更新日志](docs/CHANGELOG.md) · [历史归档](docs/archive/) |

> 每个插件与前端都维护自己的 `README.md`，详见 [文档中心 · 模块文档地图](docs/README.md#模块文档地图)。

---

## 许可证

本项目采用 [MIT License](./LICENSE) 协议。
