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

```bash
cd symbio
cargo build --lib
```

### 2. 启动桌面端

```bash
cd tauri
npm install
npm run tauri dev
```

### 3. 配置 API Key

首次启动后，进入设置页面配置 LLM API Key：

- OpenAI: `sk-...`
- Anthropic: `sk-ant-...`

或直接编辑配置文件：

```bash
# ~/.symbio/plugins/model/default/provider.json
{
  "default_provider_id": "openai_main",
  "providers": {
    "openai_main": {
      "provider_type": "openai_chat",
      "api_key": "你的 API Key",
      "model": "gpt-4-turbo"
    }
  }
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
cd symbio
cargo test --lib
# 期望: 全部通过（0 failed）。测试数量随迭代增长，文档不锁死具体数字
```

### 前端测试

```bash
cd tauri
npm test
```

### Clippy 检查

```bash
cd symbio
cargo clippy --lib --tests -- -D warnings
# 期望: 0 warnings
```

---

## 目录结构速览

```
~/.symbio/
├── config.yaml              # 主配置
├── plugins/
│   ├── model/               # Model Provider 配置
│   ├── agent/               # Agent Bundle 存储
│   └── ...                  # 其他插件数据
├── agents/                  # Agent 定义文件
├── storage/                 # 认知存储 (DirStorage 或 SQLite)
└── logs/                    # 日志文件
```

---

## 下一步

- 了解架构: [OVERVIEW.md](../architecture/OVERVIEW.md)
- 开发插件: [PLUGIN_DEVELOPMENT.md](PLUGIN_DEVELOPMENT.md)
- 查看所有路由: [ROUTES.md](../reference/ROUTES.md)

---

> **维护原则**：本文档必须保持可执行，CI 应验证步骤的正确性。
