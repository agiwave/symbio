# Symbio 项目结构与命名规范指南

> **文档类型：How-to guide / Reference（操作指南·参考）** — 目录、可见性、资源/脚本放置的权威约定。

本文件是 Symbio 代码库架构、目录结构与命名规范的**唯一权威约定**，适用于 Rust 后端（`symbio/` crate）、前端（`tauri/`）以及文档组织。所有新增代码与重构都应遵循本文档。

> 本文档由 2026-07 的架构/目录/命名专项审查整理而来，记录已落地的整改结论。

---

## 1. 架构分层原则

Symbio 采用**分形插件架构（Fractal Plugin Architecture）**，核心约束是**依赖方向单向、插件互相不可见**。

### 1.1 三层模型

| 层 | 路径 | 可见性 | 职责 |
|----|------|--------|------|
| **插件实现** | `symbio/src/plugins/*` | `mod`（私有） | 具体业务能力，通过 `Plugin` trait 暴露 |
| **核心抽象** | `symbio/src/symbio_core/*` | `pub` | `Plugin` trait、`InvokeRequest`、各类 **schema**、**服务 trait** |
| **服务实现** | `symbio/src/providers/*` | `pub(crate)` | `StorageService` 等通用基础设施的**具体实现** |

依赖方向：**`plugins` → `symbio_core`（trait/schemas）**，**`providers`（实现）→ `symbio_core::providers`（trait）**。反向依赖一律禁止。

### 1.2 插件互不可见

- 所有 `plugins/<name>` 子模块都是私有（`mod xxx;`），**插件之间不能直接 `use crate::plugins::<其它插件>::...`**。
- 插件之间的合法交互方式只有三种：
  1. **通用对象创建机制**：`submit_object_creator!` + 名称常量，按插件名查找构造函数；
  2. **`symbio_core` 公共接口**：`Plugin` trait / `InvokeRequest`，通过 trait object 交互；
  3. **`symbio_core` 共享设施**：跨插件复用的全局服务（见 1.3）。

### 1.3 共享设施放在 `symbio_core`

跨插件复用的全局服务（如事件发布、存储）统一定义为 `symbio_core` 中的**抽象 + 全局门面**，插件只依赖 `symbio_core`，不直接依赖其它插件模块：

- `symbio_core::event_bus::EventBus` —— 全局事件发布门面（`publish` / `try_publish`），由 `event_bus` 插件负责建立订阅连接，但**定义**在 `symbio_core`，供 `session`/`explorer` 等插件调用。
- `symbio_core::providers::StorageService` —— 存储抽象 trait，具体实现在 `src/providers/storage_service/`。

> **历史问题（已修复）**：`EventBus` 原定义在 `plugins/event_bus` 并被其它插件直接 `use`，违反"插件互不可见"。现已上移至 `symbio_core::event_bus`。

---

## 2. 目录结构约定

### 2.1 Rust crate 根（`symbio/symbio/`）

```
symbio/                      # 仓库根（monorepo 根）
├── symbio/                  # Rust 主 crate（注意：与仓库根同名嵌套）
│   ├── src/
│   │   ├── lib.rs           # 顶层可见性声明（见 §2.2）
│   │   ├── init.rs          # 引导/初始化入口（pub）
│   │   ├── symbio_core/     # 核心抽象（pub）
│   │   │   ├── schemas/     # 按业务域分组的请求/响应结构（见 §2.3）
│   │   │   ├── providers.rs # 服务 trait 抽象层（pub）
│   │   │   └── event_bus.rs # 跨插件共享设施
│   │   ├── plugins/         # 业务插件（私有）
│   │   └── providers/       # 通用服务实现（pub(crate)）
│   ├── resources/           # （规划中，尚未创建）运行时资源目录
│   └── Cargo.toml
├── tauri/                   # 前端（Vue + Tauri）
├── docs/                    # 项目文档（见 §2.5）
└── scripts/                 # 构建/测试脚本（.ps1/.sh）
```

### 2.2 顶层可见性（[`lib.rs`](file:///c:/Bing/agiwave/symbio/symbio/src/lib.rs)）

```rust
mod plugins;              // 私有
pub(crate) mod providers; // 实现层，不对外暴露
pub mod init;            // 引导入口
pub mod symbio_core;     // 核心抽象，对外
```

### 2.3 `symbio_core/schemas/` 按域分组

约 50+ 个 schema 文件**不再扁平堆放**，而是按业务域分子目录：

```
schemas/
├── mod.rs            # 声明各域模块 + 少量全局 re-export
├── common.rs         # 通用响应类型（SchemaResponse / SuccessResponse）
├── session/          # session_*.rs, chat_message.rs
├── memory/           # memory_*.rs
├── explorer/         # explorer_*.rs, home_reload.rs
├── agent/            # agent_config, skill*.rs, local_config
├── model/            # model_*.rs
├── mcp/              # mcp_*.rs
├── telegram/         # telegram_*.rs
├── setting/          # setting_*.rs
├── tools/            # tools_*.rs
├── work/             # work_*.rs
├── web/              # web_config, shell_execute
└── system/           # hook, events_trigger, config_get
```

引用方式：外部统一用 `crate::symbio_core::schemas::<domain>::<name>`。
前端 `tauri/src/schemas/` 已采用相同的按域分组，二者命名一一对应。

### 2.4 避免"单文件目录"与过深嵌套

- 一个目录若只含 1 个 `.rs` + 1 个 `tests.rs`，应**扁平化**到上一级（如 `manager/create_agent.rs` 直接放 `manager/`）。
- 源码树最大嵌套深度建议控制在 **4 层**以内（`src/plugins/agent/...` 下已出现过 7 层，需逐步收敛）。
- 模块级单元测试：用 `#[cfg(test)] mod tests;` 内联，或同目录 `tests.rs` 经 `mod tests;` 引入。

### 2.5 资源与数据文件

- **模型权重等二进制资源**：目前通过 `include_bytes!` 编译进 `providers/embedding` 实现模块（[`fastembed.rs`](file:///c:/Bing/agiwave/symbio/symbio/src/providers/embedding/fastembed.rs)），保留在 `src/providers/embedding/`。
  > 注：曾评估外置到 `resources/` 并运行时加载，但为保持离线内置语义，当前维持内置。若未来改为外置，路径解析逻辑应放在 `symbio_core::paths` 统一处理。
- **种子数据**（如 `normal_agent_units.jsonl`、`seed_cus.jsonl`、`seed_agents_data.json`）：与引用它的 `.rs` 同目录放置（被 `include_str!` 引用）。`bin/seed_agents_data.json` 之前混入 `bin/`，已迁移到 `plugins/agent/manager/`。
- **运行时加载的 prompt 文档**（如 `CREATE_AGENT_SKILL.md`）：被 `include_str!` 引用，必须与其 `.rs` 同目录，不得移动。

### 2.5.1 `bin/` 目录约束

- `src/bin/` **只放 binary 入口 `.rs`**（当前仅 `seed_agents.rs` 一个二进制入口）。
- 严禁在 `src/bin/` 下放置数据/资源文件（即使是被 `include_str!` 引用）。若 binary 需要数据，应**随引用源移动**或外置到 `resources/`。

### 2.5.2 `vendor/` 目录约束

- `vendor/` 整体在仓库根 `.gitignore` 中（`vendor/`），不污染 git 仓库。
- **不应在 `vendor/` 放置第三方项目的完整源码副本**（如 `vendor/qwen-code/`）。理由：
  - npm 依赖应通过 `package.json` 走 `node_modules/`，yarn/pnpm lockfile 管理版本。
  - 整棵 vendored 源码会污染本地工作目录、增大 IDE 索引负担、干扰 AI Agent 阅读。
  - symbio 自身代码不引用 `vendor/` 中任何文件 → 纯死代码。
- **审计要求**：`vendor/` 下任何子目录必须能被某条 symbio 代码或构建脚本引用，否则应删除并改用包管理器。
- 2026-07-07 审计：`vendor/qwen-code/`（QwenLM/qwen-code 整棵源码）→ 0 引用、0 npm 依赖关联 → 建议删除。

### 2.6 脚本位置

- 所有构建/测试脚本集中在仓库根 `scripts/`（`grep-audit.mjs`、`cargo-offline-refresh.mjs`、`test-capabilities.mjs` 等）。
- **脚本一律平台无关**：统一用 Node.js 编写（扩展名 `.mjs`），不使用 `.sh` / `.ps1` / `.bat`，
  以保证 Windows / macOS / Linux 上行为一致。
- 若出现 `tests/` 目录，则**只放 Rust 集成测试**（`.rs`），不放脚本。

### 2.7 文档组织

- **正式文档**放在 `docs/`（按 `architecture/`、`development/`、`ideas/`、`plugins/` 等子目录分类）。
- **源码树内只保留轻量 README**；禁止在 `src/` 下堆积与工程文档重复的长文档。
- 被代码 `include_str!` 依赖的文档（如 skill prompt）除外，必须跟随代码。

---

## 3. 命名规范

### 3.1 Rust

| 类别 | 约定 | 示例 |
|------|------|------|
| 文件 / 目录 | `snake_case` | `session_chat.rs`、`agent/manager/` |
| 模块 | `snake_case` | `event_bus` |
| 类型 / struct / enum / trait | `UpperCamelCase` | `EventBus`、`SessionStore` |
| 函数 / 方法 / 变量 | `snake_case` | `resolve_model_dir` |
| 常量 | `SCREAMING_SNAKE_CASE` | `PENDING_EVENTS_CAP` |
| 宏 | `snake_case` | `submit_object_creator!` |

**易错点**：类型名中每个单词首字母都应大写，`input` 作为单词的一部分也要大写：
- ❌ `Eventinput` / `Statusinput` → ✅ `EventInput` / `StatusInput`（已在 2026-07 修复）
- ❌ `BusEventinput` → ✅ `BusEventInput`

**缩写规范**：避免使用不透明缩写。
- ❌ `aus`（Agent Units）→ ✅ `agent_units`（`expert_agent_units.jsonl`、变量 `agent_units_str`、`write_units_to_dir`、`units` HashMap）
- 业界通用缩写（`cfg`、`tmp`、`id`）可保留。

**schema 文件命名**：统一 `snake_case`，按 `<domain>_<action>`（如 `session_chat`、`memory_store`）。同域文件归入 `schemas/<domain>/` 子目录。

### 3.2 前端（TypeScript / Vue）

| 类别 | 约定 | 示例 |
|------|------|------|
| 文件 / 目录 | `kebab-case` | `use-chat-connection.ts` |
| `.vue` 组件 | `PascalCase`（推荐统一）| `ChatInputArea.vue` |
| 变量 / 函数 | `camelCase` | `formatTime` |
| 类型 / 接口 | `PascalCase` | `ChatMessage`、`AgentProfile` |
| 常量 | 二选一：`UPPER_SNAKE_CASE` 或 `camelCase` | `KIND_SESSION` / `providerPresets` |

> `.vue` 文件目前 PascalCase 与 kebab-case 混用，建议逐步统一为 PascalCase（Vue 官方推荐）。
> 前端 `schemas/*.ts` 文件名与 Rust `schemas/<domain>/*.rs` 一一对应，新增 schema 时两端同步。

---

## 4. 配置文件约定

- `rustfmt.toml` / `clippy.toml`：**全仓库仅根目录一份**（已删除 `symbio/symbio/` 下的重复副本）。
  - `clippy.toml` 的 `msrv` 必须与根 `Cargo.toml` 的 `rust-version` 保持一致（当前 `1.91`）。
- 多 crate 共享的 lint/format 基线在根配置中定义，子 crate 不重复放置。

---

## 5. 测试组织约定

- **模块级单元测试**：`#[cfg(test)] mod tests;` 内联，或同目录 `tests.rs` 经 `mod tests;` 引入。
- **大型外部化测试文件**：若单文件测试体积极大，可用 `#[path = "xxx_tests.rs"]` 外置到同目录 sibling 文件，文件名以 `_tests.rs` 结尾（如 `typed_unit_tests.rs`、`scaffold_tests.rs`）。此类为**有意为之的例外**，不属于违规。
- **集成 / E2E 测试**：若需集成测试，放 `symbio/tests/`（`e2e_test.rs` 等），仅 `.rs`；当前仓库以模块内联测试为主，尚无独立 `tests/` 目录。
- 集成测试目录内不得放脚本——脚本统一在根 `scripts/`。

---

## 6. 已审查但保留的"看似冗余"模式

专项审查中发现的几处"重复/冗余"经核实为**合理的分层或单一机制**，不做合并，记录如下以免重复质疑：

### 6.1 `SessionStore` 与通用 `StorageService` 不合并

- `symbio_core::providers::StorageService`（`EntityStore`）是**底层、通用、字符串型**的实体存储（按 `category/id/manifest_file` 存原始字符串）。
- `plugins/session/store::SessionStore` 是**领域特定**的 `Session` 对象存储（load/save/delete/list `Session`，含 `session_dir` 压缩存档语义、按 `updated_at` 降序等）。
- 二者抽象层级不同：`SessionStore` 包裹领域类型与领域行为，`EntityStore` 不感知业务语义。强行让 `SessionStore` 复用 `EntityStore` 会丢失领域语义；且 `EntityStore` 实现层为 `pub(crate)`，插件本就不该直接依赖实现。故保留各自独立。

### 6.2 agent 存储注册是单一机制，非双套

- `submit_store_backend!` 宏**封装**了 `inventory::submit!`；`StoreBackendRegistry`（`OnceLock` 缓存）只是对 `inventory::iter::<StoreBackendEntry>` 的惰性缓存。
- 看似"inventory 自注册 + 手动 HashMap 注册"两套，实为**一套**（宏 + 缓存层）。不拆分、不合并。

### 6.3 源码树内文档保留策略

- `src/plugins/agent/docs/`（ARCHITECTURE/COGNITION/TESTING 等 7 个）与 `agent/README.md` 内部存在大量相对链接交叉引用，物理迁移会破坏链接。故保留在源码树内，仅在本文档固化"轻量 README + 长文档走 docs/"的约定；未来若迁移需同步更新 `docs/README.md`、根 `README.md` 的链接。
- 被 `include_str!` 引用的文档（如 `CREATE_AGENT_SKILL.md`、`*_agent_units.jsonl`）必须随代码同目录，不得移动。
