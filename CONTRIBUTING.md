# Contributing to Symbio

感谢你考虑为 Symbio 贡献代码！本项目采用**分形插件架构 (Fractal Plugin Architecture)**，
整套能力由一个 Rust 核心库 + Tauri (Vue 3) 前端 + 多个可选插件协同完成。
为了保持代码质量与可维护性，请先阅读以下约定。

---

## 1. 开发环境

| 工具 | 版本 | 说明 |
|---|---|---|
| Rust | stable (≥ 1.80) | 由 `symbio/rust-toolchain.toml` 自动锁定 |
| Node.js | ≥ 18 | 前端构建 |
| Tauri CLI | 2.x | `cargo install tauri-cli --version "^2.0"` |
| 平台 | Windows / macOS / Linux | Tauri 三平台均已配置 CI |

可选：使用 [rustup](https://rustup.rs/) 安装 Rust — `rust-toolchain.toml` 会自动激活对应的 channel 与组件。

---

## 2. 仓库结构

```
symbio/
├── symbio/              # Rust 核心库（29K 行，202 文件）
│   ├── src/
│   │   ├── symbio_core/ # 框架层：Plugin trait / 路由 / Schema
│   │   ├── plugins/     # 业务插件（home / composite / model / agent / session ...）
│   └── init.rs      # 根插件装配入口
│   ├── rust-toolchain.toml
│   ├── rustfmt.toml
│   └── clippy.toml
└── tauri/               # Vue 3 桌面端（13K 行 TS/Vue + 347 行 Rust）
    ├── src/             # 前端代码
    └── src-tauri/       # 仅 3 个 Tauri command 的薄适配层
```

---

## 3. 提交前清单（CI 必查）

CI 流水线位于 [.github/workflows/ci.yml](../.github/workflows/ci.yml)，**所有 PR 必须通过**：

| 步骤 | 命令 | 失败后果 |
|---|---|---|
| TypeScript 类型检查 | `cd tauri && npx vue-tsc --noEmit` | 阻止合入 |
| Rustfmt | `cd symbio && cargo fmt --check` | 阻止合入 |
| Clippy | `cd symbio && cargo clippy --all-targets -- -D warnings` | 阻止合入 |
| Rust 单元测试 | `cd symbio && cargo test --lib` | 阻止合入 |
| E2E 测试 | `cd symbio && cargo test --test verification` | 阻止合入 |
| 项目审计 | `node scripts/grep-audit.mjs` | 阻止合入 |

本地预检一条命令：

```bash
cd symbio && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --lib
```

---

## 4. 代码规范（要点）

### Rust 侧

- 遵循 `rustfmt.toml` + `clippy.toml`，**不要**手动格式化后被 CI 反复退回。
- 异步上下文请使用 `tokio::sync::Mutex` / `tokio::sync::RwLock`；`std::sync::*` 只在 `spawn_blocking` 内部使用。
  - 审计脚本 `scripts/grep-audit.mjs` 中的 **S-002 规则**专门检测此问题。
- 日志：使用 `tracing`（`info!` / `warn!` / `error!` / `debug!`），**不要**用 `eprintln!` / `println!`。
  - 项目在 `tauri/src-tauri/src/main.rs` 已初始化 `tracing-subscriber`。
- 错误：实现 `thiserror` 派生 `PluginError` 变体，不要用 `String` 当错误类型。
- 新增能力：优先在合适的 `Plugin` 下添加子路径（`worker/xxx/yyy`），
  路由规则见 [docs/explanation/ARCHITECTURE.md](../docs/explanation/ARCHITECTURE.md)。

### TypeScript / Vue 侧

- 使用 `<script setup lang="ts">`，**不要**使用 Options API。
- 严禁 `any`（callPlugin 泛型默认已是 `unknown`，调用方需显式标注）。
- 日志：使用 `logger` from `@/utils/logger`，**不要**用 `console.*`（CI 会查）。
- 插件路径：所有 worker 路径必须从 `@/constants/pluginPaths` 导入，
  **不要**在 `services/` / `stores/` / `composables/` 中硬编码 `'worker/...'` 字符串。

---

## 5. 插件开发流程

新增一个业务插件的最短路径：

1. 在 `symbio/src/plugins/<name>/` 下创建 `mod.rs` + `plugin.rs`。
2. 实现 [`Plugin`](../docs/reference/API_DESIGN.md) trait 的 `route()` 与 `traverse()`。
3. 在 `init.rs::create_root_plugin()` 中通过 `composite.add_instance(...)` 注册。
4. 在 `tauri/src/constants/pluginPaths.ts` 中添加路由常量。
5. 在 `tauri/src/services/` 下添加对应 TS 客户端。
6. 在自己插件目录下添加 `README.md`（高内聚），并在 `docs/README.md` 的"插件自包含文档"表格中登记。

完整教程见 [docs/how-to/PLUGIN_DEVELOPMENT_GUIDE.md](../docs/how-to/PLUGIN_DEVELOPMENT_GUIDE.md)。

---

## 6. Pull Request 流程

1. **Fork** 仓库，创建分支（`feat/xxx` / `fix/xxx` / `docs/xxx`）。
2. 提交信息推荐格式：
   ```
   <scope>: <summary>
   
   <details>
   ```
   scope 例：`agent` / `tauri/services` / `docs`。
3. 推送后通过 PR 提交，CI 必须全绿。
4. 至少 1 位 maintainer 审阅通过后可合入。
5. 涉及架构变更的 PR 必须在 `docs/CHANGELOG.md` 添加条目。

---

## 7. 报告 Bug

请使用 GitHub Issues，并包含：

- 复现步骤（最小可复现 demo）
- 实际行为 vs 预期行为
- 平台（Windows / macOS / Linux）+ Rust / Node 版本
- 关键日志（启用 `RUST_LOG=debug` 或 `VITE_LOG_LEVEL=debug`）

---

## 8. 社区准则

参见 [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)。

---

## 9. 许可证

提交即表示你同意本项目以 [MIT License](../LICENSE) 发布你的贡献。
