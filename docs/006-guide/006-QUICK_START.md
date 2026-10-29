---
kind: guide
id: GU-006-QUICK_START
title: Symbio 快速上手
status: active
created: 2026-09-07
---

# Symbio 快速上手

> **文档类型：教程** — 从零跑通一个完整场景。

## 前置要求

| 工具 | 版本 | 安装方式 |
|------|------|----------|
| Rust | ≥ 1.91 | [rustup.rs](https://rustup.rs/) |
| Node.js | ≥ 18 | [nodejs.org](https://nodejs.org/) |
| Tauri CLI | 2.x | `cargo install tauri-cli --version "^2.0"` |

## 5 分钟跑通

### 1. 编译核心库

以下命令都在**仓库根**执行（根 workspace 单 target，成员产物统一落在 `target/`）：

```bash
cargo build -p symbio-core
```

### 2. 启动桌面端

```bash
cd tauri
npm install
npm run tauri dev
```

### 3. 配置 API Key

首次启动后，在左侧导航选择**模型**，新建（或打开已有）一个模型条目，填入 API Key：

- OpenAI: `sk-...`
- Anthropic: `sk-ant-...`

> 模型配置不是「设置」页里的一项：设置页列的是**各插件自己交出来的配置文档**
> （会话 / 网络工具 / 本地工具 / 开放接口 / Telegram），模型这类「资源型」插件的
> 配置就是它的资源树本身（`<根>/model/<id>`）。

或直接编辑配置文件：

```bash
# ~/.symbio/model/openai_main/provider.json
{
 "id": "openai_main",
 "provider": "openai",
 "api_base": "https://api.openai.com/v1",
 "api_key": "你的 API Key",
 "model": "gpt-4o"
}
```

### 4. 开始对话

1. 在左侧面板选择或创建一个 Agent
2. 点击 "New Session"
3. 输入消息并发送
4. 观察流式响应

---

## 验证安装

### 核心库测试

```bash
cargo test --workspace
# 期望: 全部通过（0 failed）。测试数量随迭代增长，文档不锁死具体数字
```

### 前端测试

```bash
cd tauri
npm test
```

### Clippy 检查

```bash
cargo clippy --all-targets --workspace -- -D warnings
# 期望: 0 warnings
```

---

## 目录结构速览

**一个插件 = 一个目录**（配置与数据同处，整目录可拷贝移植；机制见
[VDFS 前端页面规范](../003-design/010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") §3 地址模型与 `symbio_core::plugin::dir`）：

```
<homedir>/ # 默认 ~/.symbio
├── PLUGIN.yml # 系统级插件（home）的配置：工作区与最近记录
├── <插件>/ # session · model · agent · mcp · skill · plugin_manager …
│ ├── PLUGIN.yml # 该插件的配置（对外地址 <根>/<插件>/PLUGIN.yml）
│ └── <id>/<主文件> # 资源条目：model/<id>/provider.json、mcp/<id>/server.json…
├── session/<id>/ # 会话：session.json（元数据）+ messages.json（消息）
├── agent/<id>/ # Agent 目录（工作区级 + 全局级）
└── cache/ # 可重建的缓存（如代码索引）
```

> 插件根**不额外嵌套一层**：系统根下就是「一个插件一个目录」的扁平结构。

---

## 下一步

- 了解架构: [架构总览与目标形态](../003-design/002-架构总览.md "DE-002-架构总览")
- 开发插件: [插件开发指南](005-PLUGIN_DEVELOPMENT.md "GU-005-PLUGIN_DEVELOPMENT")
- 查看所有路由: [路由参考](../004-ref/003-ROUTES.md "RE-003-ROUTES")

---

> **维护原则**：本文档必须保持可执行，CI 应验证步骤的正确性。
