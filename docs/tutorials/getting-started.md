# 快速上手（Getting Started）

> **目标**：跟着做完，你能在本地跑通 Symbio——灌入种子 Agent、启动桌面端、发起一次对话。
> 预计耗时 10–15 分钟。需要已安装 [Rust 工具链](https://www.rust-lang.org/tools/install) 与 [Node.js 18+](https://nodejs.org/)。

本教程是**循序渐进的操作指引**（Tutorial），不是完整参考。接口与配置细节见 [API 设计规范](../architecture/API_DESIGN.md) 与各插件自文档。

---

## 1. 克隆与目录概览

```bash
git clone <repo-url> symbio
cd symbio
```

你会看到：

```
symbio/
├── symbio/     # Rust 核心库（全部业务逻辑）
├── tauri/      # 桌面端（Vue 3 + Tauri）
├── docs/       # 文档
└── scripts/    # 工具脚本
```

核心库 `symbio/` 不依赖 UI；桌面端只是它的一个宿主。

---

## 2. 编译核心库

```bash
cd symbio
cargo build --lib
```

首次编译会拉取依赖，耗时稍长。编译通过说明工具链就绪。

---

## 3. 灌入种子 Agent

种子 Agent 是 7 个软件项目开发角色（`project_manager` / `architect` / `coder` / `reviewer` / `tester` / `documenter` / `devops`），首次运行会创建它们的认知数据：

```bash
cargo run --bin seed_agents
```

> 想强制重建（先删后建）：`cargo run --bin seed_agents -- --recreate`

跑完后，这些 Agent 已就绪，可在桌面端或代码中直接对话。

---

## 4. 启动桌面端并对话

```bash
cd tauri
npm install
npm run tauri:dev
```

桌面端启动后：

1. 在 Agent 列表中选择一个角色（例如 `coder`）。
2. 发起对话 `agent/chat`，例如："帮我写一个 Rust 函数，用递归计算斐波那契数列"。
3. Agent 会结合其认知记忆、调用 `model/chat` 走 LLM，并在需要时调用工具（本地 shell、文件读写等）。

> 桌面端通过 3 个 Tauri command（`route_v2` / `route_v2_send` / `route_v2_close`）与核心库通信，本身不实现业务逻辑。

---

## 5. （可选）用命令行/宿主调用核心库

不想开桌面端时，也可在 Rust 中直接驱动核心库：

```rust
use symbio::{initialize, create_root_plugin};
use symbio_core::{SimpleRequest, PATH};

#[tokio::main]
async fn main() {
    initialize();
    let root = create_root_plugin().await;
    let ctx = std::sync::Arc::new(SimpleRequest::new(None, None));
    ctx.set(PATH, "agent/chat".to_string());
    // 设置 payload 后：
    let _ = root.route(ctx).await;
}
```

插件通过 `agent/chat`、`model/chat`、`local/shell` 等路径寻址，详见 [架构设计](../architecture/ARCHITECTURE.md)。

---

## 6. 验证你的环境

```bash
cd symbio
cargo test --lib                 # 全部单元测试应通过
cargo clippy --lib --tests -- -D warnings   # 质量门禁
```

---

## 接下来去哪

- 想读懂整体运作：读 [运作机制](../architecture/OPERATION_MECHANISM.md)。
- 想写自己的插件：读 [插件开发实战](../development/PLUGIN_DEVELOPMENT_GUIDE.md)。
- 想了解方向：读 [产品愿景与方向](../VISION.md)。
