---
lists: [repo-layout=owner]
kind: guide
id: README
title: Symbio
status: active
created: 2026-07-20
---

# Symbio

> **一个可组合、多协议的 AI Agent 平台**：一条 Rust 核心库，桌面与命令行两个前端、HTTP/WS 与 Telegram 两个入站通道，共用同一套**分形插件**装配与同一条**路径协议**。一条规则：每个能力都是一个**路径**。

<!-- list:repo-layout owner -->

| 层 | 目录 / 挂载点 | 职责 |
| --- | --- | --- |
| **核心库** | `symbio/` | `symbio_core` 契约 · 分形插件路由 · 多协议 LLM 适配 · 会话（持久化 / 生命周期 / 压缩） · Agent 资产与记忆 |
| **桌面前端** | `tauri/` | Vue 3 + Pinia + Tauri 2；`src-tauri/` 是只暴露 3 个 IPC 命令（`route_v2` / `route_v2_send` / `route_v2_close`）的薄适配层 |
| **命令行前端** | `cli/` | 纯 Rust：REPL / 单次 / 管道 / 心跳守护，进程内直连插件树，不经 IPC |
| **入站通道** | `gateway` · `telegram` 插件 | 树内插件：HTTP / WS 入站请求转发给父级路由（与 `route_v2` 同构）；Telegram 长轮询收发 |

设计原则：**UI 只做配置与展示，业务逻辑全在核心库**；各前端各自建树、不共享内存，但共用同一套插件装配与路径协议。
**用它做什么**：多轮 Agent 会话 · 本地开发助手（shell / 搜索 / VDFS / 技能 / MCP） · 无头自动化（CLI 管道 / 网关 webhook / Telegram） · 多供应商 LLM。

---

## 什么使 Symbio 与众不同

- **分形插件** — 每个能力都是插件，每个插件都可含子插件；容器与叶子共享同一 `Plugin` trait。
- **路径即路由** — 模型输出 `vdfs_read` 就能读文件、`http_request` 就能发请求，无代码生成。
- **会话 / 执行 / 模型三件分离** — `session` 持转写与事实源、只在**轮的边界**派发；`loop` 是单轮执行引擎（上下文装配 / 轮首判决 / 工具循环 / 收束）；`model` 是无状态 LLM 网关。Agent 只是可绑定的资产。
- **VDFS** — 文件、模型、Agent、技能、MCP、设置……统统是有地址的资源，一个心智模型。
- **Agent = 一个目录** — 技能 / MCP 复用宿主的子树，没有独立二进制。

> 展开的架构说明见 [架构总览](docs/003-design/002-架构总览.md "DE-002-架构总览")；运行时拓扑（谁挂在谁下面）见 [系统地图](docs/003-design/004-系统地图.md "DE-004-系统地图")——本文不复述这两者。

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
node cli/scripts/build-cli.mjs # 产物落在仓库根：target/debug/symbio-cli.exe
./target/debug/symbio-cli.exe -m "你好" # 单条；系统目录默认 <当前目录>/.symbio
./target/debug/symbio-cli.exe # REPL；系统目录在别处时加 --homedir <路径>
```

### 编译与门禁

```bash
cd symbio && cargo build --lib # 只编核心库
node scripts/gate.mjs # 提交前一条命令：Rust 编译 / 单测 / clippy / fmt · 前端类型检查与 vitest · 静态审计 · 事实表生成
node e2e/run-tests.mjs # 端到端：mock LLM / MCP / HTTP，不需要真 API Key
```

> 完整教程见 [快速上手](docs/006-guide/006-QUICK_START.md "GU-006-QUICK_START")；门禁的阶段划分与参数见 [CONTRIBUTING.md](CONTRIBUTING.md "CONTRIBUTING")。

---

## 仓库结构

```
symbio/
├── symbio/ # Rust 核心库：symbio_core（契约）· plugins（插件树）· providers（嵌入 / 存储）
├── tauri/ # Vue 3 桌面端（src-tauri = 3 命令的薄适配层）
├── cli/ # 纯 Rust CLI 前端（根 workspace 成员，共用仓库根的 target/）
├── docs/ # 系统级文档（目标 / 场景 / 计划 / 架构 / 参考 / 指南 / 规范 / 任务 / 决策，历史在 archive/）
├── e2e/ # 端到端用例（mock LLM / MCP / HTTP + 网关 WS）
├── examples/ # Agent 示例包
└── scripts/ # 事实表生成、判据审计与门禁
```

> 插件 / 挂载点 / 路由 / 工具 / 协议 / 代码规模这些会变的数字只有一个来源：[当前事实表](docs/004-ref/006-当前事实表.md "RE-006-当前事实表")（从代码生成，CI `--check` 防漂移）。本文不复述。

---

## 文档

| | |
| --- | --- |
| **入门** | [快速上手](docs/006-guide/006-QUICK_START.md "GU-006-QUICK_START") · [系统地图](docs/003-design/004-系统地图.md "DE-004-系统地图") · [价值主张](docs/001-goal/000-价值主张.md "GO-000-价值主张") · [场景](docs/002-scene/README.md "SC-000-场景汇总") |
| **架构** | [总览](docs/003-design/002-架构总览.md "DE-002-架构总览") · [数据流](docs/003-design/003-数据流与调用链.md "DE-003-数据流与调用链") · [协议](docs/003-design/005-协议规范.md "DE-005-协议规范") · [决策记录](docs/003-design/README.md "DE-000-设计总览") |
| **参考** | [事实表](docs/004-ref/006-当前事实表.md "RE-006-当前事实表") · [路由](docs/004-ref/003-ROUTES.md "RE-003-ROUTES") · [错误码](docs/004-ref/002-ERROR_CODES.md "RE-002-ERROR_CODES") · [配置](docs/004-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION") |
| **开发** | [插件开发](docs/006-guide/005-PLUGIN_DEVELOPMENT.md "GU-005-PLUGIN_DEVELOPMENT") · [排障](docs/006-guide/007-TROUBLESHOOTING.md "GU-007-TROUBLESHOOTING") · [文档中心与登记](docs/README.md "docs/README") · [贡献指南](CONTRIBUTING.md "CONTRIBUTING") |
| **设计** | [VDFS](docs/003-design/010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") · [Agent 目录](docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范") · [上下文压缩](symbio/src/plugins/session/docs/101-context-compression-design.md "DE-101-context-compression-design") · [目标架构 v2](docs/003-design/002-架构总览.md "DE-002-架构总览") |
| **CLI** | [cli/README](cli/README.md "cli/README") · [架构](cli/docs/112-architecture.md "DE-112-architecture") · [构建](cli/docs/003-building.md "GU-003-building") · [用法](cli/docs/004-usage.md "GU-004-usage") |

> 每个插件与前端都维护自己的 `README.md`，全部模块的文档地图在[文档中心](docs/README.md "docs/README")。
> 变更历史 = `git log`（本仓库不另维护变更日志，理由见[贡献指南](CONTRIBUTING.md "CONTRIBUTING")）。

---

## 许可证

本项目采用 [MIT License](LICENSE) 协议。
