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

## 3. 提交前检查

**一条命令跑完全部**（后端含 `cli/` → 前端 → 4 项审计 → 事实文件）：

```bash
node scripts/gate.mjs                 # 全量
node scripts/gate.mjs --only=frontend # 单阶段（也有 --skip=）
node scripts/gate.mjs --fix           # 先自动格式化 / 重生成，再检查
node scripts/gate.mjs --ci            # 对齐 CI（cargo test --workspace）
```

覆盖：Rust 编译 / 单测 / clippy / rustfmt、TypeScript 类型检查、vitest、以及 4 项审计
（`grep-audit`、`style-audit`、`doc-link-audit`、`test-layout-audit`）与事实文件一致性。
完整输出落 `.workbuddy-ai/gate-logs/`；通过数低于基线会报错、高于基线提示更新 `BASELINE`。

⚠️ **要加检查就改 `scripts/gate.mjs`**——本文档、CI、记忆里都不再另立清单，两份清单必然漂移。

找不到某条机制 / 约定写在哪：`node scripts/doc-find.mjs <关键词>`（搜全仓 `*.md` 与源码
`//!` / `///`）。项目文档是下沉的，**知识只写一处**；发现缺文档就补那一处，不要把摘要抄到别处。

CI 流水线见 [.github/workflows/ci.yml](./.github/workflows/ci.yml)；提交信息规范由
`scripts/check-commit-msg.mjs` 判定，本机一次性挂上即可自动生效：

```bash
git config core.hooksPath scripts/git-hooks
```

### 本机操作陷阱（都踩过）

- ⚠️ **cargo / git 一律不接管道**（`| tail` / `| head`）：输出到 EOF 才刷 ⇒ 看着像卡死；且管道**吞掉退出码** ⇒ 失败的命令看起来是成功的。要看长输出就用 `gate.mjs`（它落日志、只信退出码）。
- ⚠️ **`git commit -F /c/Temp/msg.txt` 会失败**（Windows git 不认 MSYS 路径）⇒ 写 `-F "C:/Temp/msg.txt"`。
- ⚠️⚠️ **批量改名绝不用 `git rm` / `git mv`**：它们写索引，被 SIGTERM 中断后留下 0 字节 `.git/index.lock` 并写坏索引 ⇒ 上百文件误报「已删除」。正解 = 纯文件系统改名 + 最后一次 `git add -A`。**恢复**：`rm -f .git/index.lock` → `git checkout -- .`（索引损坏时 `reset --hard` 不可靠）。单个文件用 `git mv` 无妨。
- ⚠️ **无备份绝不 `git checkout -- .` / 大范围删除**。回滚先 `stash push` 或 `git diff > /c/Temp/x.patch`。
- ⚠️ **rustfmt 只用 `cargo fmt`**，绝不裸跑 `rustfmt`（工具链锁定版本不同 ⇒ 格式漂移）。
- ⚠️ vitest 4 的 `toBe(v, 'msg')` 只收 1 个参数（写两个参数静默失效）。
- ⚠️ `.workbuddy-ai/` 被 gitignore：**不要提交、不要删除**。仓库另有 `.workbuddy/`（旧 harness 遗留）——以 `.workbuddy-ai/` 为准。

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
  路由规则见 [docs/architecture/OVERVIEW.md](./docs/architecture/OVERVIEW.md)，
  请求全链路见 [docs/architecture/DATA_FLOW.md](./docs/architecture/DATA_FLOW.md)。

#### 测试文件布局（约定）

测试**独立成文件**，且与**被测试的实现文件同级**、同名加 `.test` 后缀：

```text
workdir.rs          实现
workdir.test.rs     它的测试（同级，不放子目录）
```

父文件末尾用 `#[path]` 指向同级测试文件：

```rust
#[cfg(test)]
#[path = "workdir.test.rs"]
mod tests;
```

要点：

- 布局由 `node scripts/test-layout-audit.mjs` 判定（已接入 `gate.mjs`），不靠人工记住。
- **一个实现文件对应一个测试文件**。不要写"一个测试文件同时测几个实现文件"，
  也不要写"几个测试文件测同一个实现文件"——测试跟着它测的那个实现走。
- **不要为测试文件单独建目录**。`X/tests.rs` 那种形态会让"模块"与"目录"两个概念
  混在一起；`#[path]` 相对声明它的文件解析，测试文件完全可以与实现同级。
- **例外**：模块文件是 `mod.rs` 的（如 `store/mod.rs`），测试放同级的 `tests.rs`。
- `#[path]` **不改变模块路径**（仍是 `X::tests`），故 `use super::*;` 语义与内联
  `mod tests { … }` 完全一致，拆分/搬移是纯文件操作，不涉及可见性调整。

> 存量代码里仍有大量内联 `#[cfg(test)] mod tests { … }`。**新增代码请按上述约定**；
> 存量按"改到哪个文件就顺手拆哪个"渐进处理，不做全仓一次性搬移。

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
2. 实现 [`Plugin`](./docs/architecture/PROTOCOLS.md) trait 的 `route()` 与 `traverse()`。
3. 在 `init.rs::create_root_plugin()` 中注册（挂载方式参考 `plugins/home/plugin.rs` 的默认挂载逻辑）。
4. 在 `tauri/src/constants/pluginPaths.ts` 中添加路由常量。
5. 在 `tauri/src/services/` 下添加对应 TS 客户端。
6. 在 `docs/CHANGELOG.md` 追加条目；如涉及路由/协议/配置变更，同步更新 `docs/reference/`（ROUTES / ERROR_CODES / CONFIGURATION）与 `docs/architecture/`（OVERVIEW / DATA_FLOW）对应内容。

完整教程见 [docs/guides/PLUGIN_DEVELOPMENT.md](./docs/guides/PLUGIN_DEVELOPMENT.md)。

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

提交即表示你同意本项目以 [MIT License](./LICENSE) 发布你的贡献。
