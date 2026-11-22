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
cargo build -p symbio          # 宿主装配 crate（连带编出 kernel / sdk 与全部插件）
# 只编某个插件：cargo build -p symbio-plugin-<id>
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
 "model": "gpt-4o",
 "api_protocol": "openai_responses"
}
```

> 字段权威来源是[当前事实表](../004-ref/006-当前事实表.md "RE-006-当前事实表")与
> `crates/symbio-plugin-model/src/model_providers.rs` 的 `ModelProviderConfig`：
> `provider` 取预设表里的值（`openai` / `anthropic` / `openai_compatible` / `custom` 等），
> `api_protocol` 取该预设支持的协议（此处 `openai` 默认 `openai_responses`，也可选 `openai_chat`）。
> 其余字段（`temperature` / `max_tokens` / `timeout_secs` …）都有默认值，可省略。

### 4. 开始对话

> **先选工作目录**：会话绑定一个工作目录，工具在其中读写你的项目文件。新建会话若没带目录，
> 界面会引导你用原生对话框选一个；输入框下方的会话选项栏里也能随时改。
> 这一步是会话的前置条件——不设置，工具调用会落到默认路径而不是你的项目。

1. 在左侧面板选择或创建一个 Agent
2. 点击 "New Session"，按引导选择工作目录
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
# 与门禁 scripts/gate.d/10-backend.mjs 保持同一条命令（dead_code 由 D-331/SD-001 台账单独管）
cargo clippy --all-targets --workspace -- -D warnings -A dead_code
# 期望: 0 warnings。`dead_code` 一族被让开，因为「待接入能力」必然暂未被调用；
# 新增死码仍由 session-dead-code-audit 按台账判红，不是放宽。
```

---

## 目录结构速览

**一个插件 = 一个目录**（配置与数据同处，整目录可拷贝移植；机制见
[VDFS 前端页面规范](../003-design/010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") §3 地址模型与 `symbio_sdk::plugin::dir`）：

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
