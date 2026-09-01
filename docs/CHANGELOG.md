# 功能更新记录

> **文档类型：Reference（参考）** — 按时间倒序的功能/修复事实记录。

> **重要：当前形态声明（2026-07-06 起）**
>
> 本仓库当前形态是 **Rust 核心库 + Tauri (Vue 3) 桌面前端 + E2E CLI**。
> - `tauri/` 目录是活跃维护中的桌面前端（一等公民），与核心库版本同步演进。
> - `symbio/src/bin/seed_agents.rs` 提供批量灌入种子 Agent 的 CLI（当前唯一二进制入口）。
> - 早期"Tauri 时代 = 临时形态"的说法已被否决：前端自 2026-06 多会话体系改造后恢复为**一等公民**。
>
> 下方按日期倒序记录**与代码同步**的功能/修复条目。
> 早期"Tauri 时代相关"已统一迁到 `docs/archive/design_docs/HISTORY_AND_REVIEWS.md`，本文件**不再追加**前端 UI 变更。

---

## 2026-09-01: 全量依赖升级（Rust / Node / 前端 / CI）

**Rust**（保留精确版本，symbio/Cargo.toml）：thiserror 2.0.20、dirs 6、notify 8.2、fastembed 6.0.2、dashmap 6.2.1、which 8.0.6、reqwest 0.13.4、time 0.3.55、rusqlite 0.37 + tokio-rusqlite 0.7 配套锁定等。代码适配：

- tokio-rusqlite 0.7 移除 `Error::Other`，`call` 闭包直接返回 `Result<R, E>`，10 处调用点改写并显式标注 `rusqlite::Error`
- time 0.3.55 deprecated `format_description::parse`，改用 `parse_borrowed::<2>`（chat.rs / system_prompt.rs）
- tauri/src-tauri 旧 Cargo.lock 与 rusqlite 0.37 冲突（links = "sqlite3"），重新生成

**前端**（tauri/package.json）：vite 8、@vitejs/plugin-vue 6、vitest 4、pinia 4、vue-router 5、marked 18、katex 0.18、mermaid 11、milkdown 7.22、@tauri-apps/* 2.11。配置适配：

- vite 8（rolldown 内核）不支持对象形式 `manualChunks`，改为函数形式（vite.config.ts）
- vite 8 不再内置 esbuild，`minify: 'esbuild'` 改为 `'oxc'`
- typescript 保持 5.9：TS 7（native）与 vue-tsc 3.3 不兼容（实测 ERR_PACKAGE_PATH_NOT_EXPORTED）

**CI**：node 20→22（20 已 EOL）、checkout v5 / setup-node v5 / cache v4（消除 Node 20 deprecation 警告），release.yml 同步升级。

**验证**（2026-09-01）：

- `cargo fmt --check` / `cargo clippy -D warnings`：0 error
- `cargo test --workspace`：355 passed
- tauri/src-tauri `cargo check`：通过
- `npx vue-tsc --noEmit` / `npm test`（18 passed）/ `npm run build`：通过

---

## 2026-09-01: 连接测试 ping 请求修复 + CI 三处门禁修复

**一、模型连接测试报 400（`max_tokens must be greater than 2`）**

- 根因：`handle_ping` 探活请求硬编码 `"max_tokens": 1`，GLM 的 OpenAI 兼容网关要求 `max_tokens > 2`。
- 修复：三个协议（`openai_chat` / `anthropic_messages` / `gemini_api`）的 ping 请求统一调大到 16。

**二、GitHub CI 持续失败（三个 job 各一处根因）**

| job | 根因 | 修复 |
|---|---|---|
| rust-checks | CI 与本地 rustfmt 版本漂移导致 `fmt --check` 失败；rustfmt.toml 含 5 个 nightly-only 选项被 stable 静默忽略 | `rust-toolchain.toml` 锁定 `channel = "1.93.1"`；CI 改用 `actions-rust-lang/setup-rust-toolchain@v1` 读取该文件；清理 nightly-only 选项 |
| frontend-checks | `setup-node` 的 `cache: 'npm'` 在仓库根目录找不到 lock 文件（实际在 `tauri/`） | 增加 `cache-dependency-path: tauri/package-lock.json` |
| security-check | `cargo audit` 报 RUSTSEC-2025-0068：`serde_yml` 不维护且有 soundness 问题 | 全量替换为维护中的 fork `serde_yaml_ng`（API 兼容，9 文件 20 处） |

**验证**（2026-09-01）：

- `cargo fmt --all -- --check`：0 diff
- `cargo clippy --workspace --all-targets -- -D warnings`：0 error
- `cargo test --workspace`：355 passed / 0 failed
- `bash scripts/grep_audit.sh`：0 errors
- `npx vue-tsc --noEmit`：0 错误；`npm test`：18 passed
- Cargo.lock 已确认无 `serde_yml` 残留

---

## 2026-09-01: Model Provider"测试连接"路由修复

**Bug**：在 Model Provider 添加新模型时点击"测试连接"，报错
`Composite: 路径 'model_providers/test' 无法识别或子插件未挂载`。

**根因**：前端 `ModelProvidersView.vue` 的 `handleTest` 硬编码调用了 `model_providers/test`，
但该路径从未存在——Model Provider 管理路由的正确前缀是 `worker/model/providers/*`
（见 `tauri/src/services/modelProviders.ts` 的 `MODEL_PROVIDERS_PATH` 常量），
且后端此前**没有**独立的"测试连接"路由（只有 `providers/set` 会在保存时顺带校验）。

**修复（前后端联动）**：

| 端 | 改动 |
|---|---|
| 后端 schema | `symbio_core/schemas/model/model_providers.rs` 新增 `model_providers_test` 模块（Request 含 `provider` + `skip_validation`，Response 空） |
| 后端路由 | `plugins/model/plugin.rs` 新增 `providers/test`——复用 `validate_provider`（无副作用校验），**不写注册表、不落盘**，因此未保存的草稿配置也能直接测试；失败返回 `ValidationError("连接测试失败: {err}")` |
| 前端 schema | `tauri/src/schemas/model_providers.ts` 新增 `ModelProvidersTest` namespace |
| 前端 service | `tauri/src/services/modelProviders.ts` 新增 `testModelProvider()`（走 `worker/model/providers/test`） |
| 前端视图 | `ModelProvidersView.vue` 的 `handleTest` 改用 `testModelProvider`，移除 `model_providers/test` 硬编码与不再使用的 `callPlugin` 导入 |

**验证**（2026-09-01）：

- `cargo test --lib`：355 passed / 0 failed
- `cargo clippy --all-targets -- -D warnings`：0 error
- `cargo fmt --all -- --check`：0 diff
- `npx vue-tsc --noEmit`：0 错误
- `npm test`（vitest）：18 passed

**附带收益**：`providers/test` 作为无副作用路由，也是后续在设置表单中"实时校验"（输入即测）的稳定后端锚点。

---

## 2026-09-01: 全库质量门禁回归修复 + 前端测试基建

**背景**：项目级质量审计发现三处"门禁失真"：
1. `cargo clippy --lib` 实际存在 **12 个 warning**（文档声称 0）；
2. `cargo fmt --check` 存在 **83 处历史偏差**（CI 的 `--check` 门禁实际从未在当前 toolchain 下通过）；
3. 前端**零测试**（CI frontend job 仅有 vue-tsc + 空 sanity check）。

**变更**：

### 一、Rust 侧（业务行为零变化）

| 修复 | 位置 |
|---|---|
| `redundant_closure` ×3 | `symbio_core/event_bus.rs`（×2）、`providers/embedding/fastembed.rs` |
| `collapsible_else_if` | `plugins/agent/handlers/system_prompt.rs` |
| `unnecessary_if_let`（`.flatten()`） | `plugins/local/codebase_search.rs` |
| `manual_div_ceil` | `plugins/local/file_read.rs` |
| `let_and_return` | `plugins/mcp/http.rs` |
| `doc_overindented_list_items` | `plugins/model/message_builder.rs` |
| `unnecessary_cast` | `plugins/session/heartbeat.rs` |
| `ptr_arg`（`&mut Vec` → `&mut [T]`） | `plugins/skill/plugin.rs` |
| `derivable_impls`（`#[derive(Default)]` + `#[default]`） | `symbio_core/schemas/mcp/mcp_config.rs` |
| `doc_lazy_continuation` | `symbio_core/schemas/session/session_chat.rs` |
| `field_reassign_with_default` ×4（**测试代码**，`--all-targets` 门禁） | `plugins/mcp/http/tests.rs`（Default 赋值改结构体初始化语法） |
| 死代码清理 | `plugins/agent/core/mod.rs::query_relation_names`（零调用）；`plugins/agent/capabilities/mod.rs` 3 个 v10 预留死常量 |
| `cargo fmt --all` | 全库 83 处历史偏差统一，fmt 门禁恢复有效 |

### 二、前端测试基建（从 0 到 1）

| 项 | 说明 |
|---|---|
| 引入 `vitest@2.1.9`（devDependency） | 与 vite 6 对齐的稳定版本 |
| 新增 `tauri/vitest.config.ts` | `@` alias 与 vite.config 对齐；node 环境（纯逻辑层） |
| 新增 18 个单元测试 | `src/utils/__tests__/time.spec.ts`（6）+ `message.spec.ts`（12，多模态文本提取 / 消息键稳定性） |
| `package.json` | 新增 `test` / `test:watch` script |
| CI（`.github/workflows/ci.yml`） | frontend-checks job 新增 `npm test` 门禁 |

**验证**（2026-09-01）：

- `cargo test --lib`：355 passed / 0 failed
- `cargo clippy --lib -- -D warnings`：0 warning
- `cargo clippy --all-targets -- -D warnings`（CI 同款，含测试代码）：0 error（另修复 `plugins/mcp/http/tests.rs` 4 处 `field_reassign_with_default`）
- `cargo fmt --all -- --check`：0 diff
- `npm test`：18 passed（vitest）
- `npx vue-tsc --noEmit`：0 错误

**设计决策记录**：审计中评估的"认知内核与 Agent 插件壳解耦"（CognitionService trait 化 / crate 拆分）经确认**不采纳**——认知与智能体是一一对应的共生关系，`agent` 插件的"认知中心"内聚形态是设计使然。详见 `symbio/src/plugins/agent/docs/CHANGELOG.md` 同日条目。

---

## 2026-07-06: 项目级文档系统性同步 + 历史文档清理

**背景**：用户两轮反馈：
1. "项目文档与代码实现存在系统性脱节"——`docs/archive/proj/` 下历史规划文件、`docs/archive/design_docs/` 中早期提案、以及 `docs/README.md` 中插件文档列表与实际不符。
2. "继续，注意删除历史过期文档信息或者文档，确保所有文档保持最新"——既然能通过新方案覆盖过期文档，应**直接删除**而非保留横幅标注。

**变更**：

### 一、新增权威改进方案

1. **新增 `docs/archive/proj/IMPROVEMENT_PLAN_2026.md`**（**权威改进方案**）
   - 基于 2026-07 当前代码状态的项目级下一阶段改进计划
   - 10 个改进方向（P0-P3）：文档脱节修复 / Plugin Channel 跨进程 / MCP 成熟化 / Skill 实战化 / E2E CI 化 / 可观测性 / HNSW ANN / 前后端类型同步 / 外部插件 / 移动端
   - 季度路线图（2026 Q3 / Q3-Q4 / 2027 Q1-Q2 / 2027+）
   - 取代 `PLAN.yml` / `TASK_INDEX.md` 作为项目要做的事的**唯一权威来源**

### 二、`docs/README.md` 修复

- §3 `design_docs/`：删除 `ARCHITECTURE_IMPROVEMENT.md` / `MODEL_CHAT_REDESIGN.md` / `COMPARISON_WITH_QWEN_CODE.md` 链接（已删除）
- §5 agent 插件文档列表：移除不存在的 `PROMPT_ARCHITECTURE.md` / `OPERATIONS.md` / `CODE_ANALYSIS_REPORT.md` 引用
- §6 "早期与产品向文档"：移除 `docs/archive/proj/` 引用（已清理为只剩 `IMPROVEMENT_PLAN_2026.md`）
- 添加"§6.1 现行改进方案"小节，链接到新的 `IMPROVEMENT_PLAN_2026.md`

### 三、删除过期历史文档（用户要求"删除"而非"加横幅"）

| 删除文件 | 原因 |
|---------|------|
| `docs/archive/design_docs/ARCHITECTURE_IMPROVEMENT.md` | Skill/Subagent/Hook 提案已**全部落地**（plugins/skill/, agent_create+agent_chat 组合, plugins/hook/） |
| `docs/archive/design_docs/MODEL_CHAT_REDESIGN.md` | 扁平消息树设计已落地（`chat_message.rs` + Tauri `MessageNode.vue`） |
| `docs/archive/design_docs/COMPARISON_WITH_QWEN_CODE.md` | 报告差异**多数已通过新增/重写插件弥合**，继续保留会持续误导 |
| `docs/archive/proj/PLAN.yml` | 2026-03 早期规划，与当前代码严重脱节 |
| `docs/archive/proj/TASK_INDEX.md` | 2026-03 早期任务索引，24 个任务多数不适用 |
| `docs/archive/proj/tasks/T001-project-infrastructure.md` | 早期任务 |
| `docs/archive/proj/tasks/T002-markdown-editor.md` | 早期任务 |
| `docs/archive/proj/tasks/T004-docker-environment.md` | 早期任务 |
| `docs/archive/proj/tasks/T010-rnaseq-template.md` | 早期任务 |
| `docs/archive/proj/MODEL_CHAT_IMPLEMENTATION_PLAN.md` | Phase 1-4 已落地，Phase 5 部分落地 |
| `docs/archive/proj/MODEL_CHAT_IMPROVEMENT_PLAN.md` | 已落地且**内容存在事实性错误**（声称的 `mcp/` / `hooks/` / `subagent/` / `workflow/` / `hmemory/` / `checkpoint/` 目录**实际不存在**） |

### 四、其他修复

- `docs/archive/design_docs/HISTORY_AND_REVIEWS.md`：
  - 修复"当前形态"错误描述（原称"纯 Rust 核心库 + E2E CLI"，但前端已于 2026-06 恢复为 Tauri）
  - 新增"2026-06 — Tauri 桌面前端恢复（当前形态）"里程碑小节
- `docs/archive/proj/IMPROVEMENT_PLAN_2026.md`：
  - 移除 §5 中指向已删除文件的引用
  - §2.1 改为"✅ 已完成"，列出全部已完成的删除项

**影响**：
- ✅ `docs/` 中无任何**事实性误导**文档
- ✅ `docs/archive/design_docs/` 仅保留 `HISTORY_AND_REVIEWS.md`（关键里程碑回顾）
- ✅ `docs/archive/proj/` 仅保留 `IMPROVEMENT_PLAN_2026.md`（项目级改进方案）
- ✅ "项目要做的事"有了单一权威来源（`IMPROVEMENT_PLAN_2026.md`）
- ✅ 关键决策轨迹仍在 `HISTORY_AND_REVIEWS.md` 中可追溯

**未改动**：
- 业务插件自包含文档（`symbio/src/plugins/agent/docs/`）保持不变
- `docs/explanation/*` / `docs/reference/*` / `docs/how-to/*` 权威文档保持不变
- `docs/CHANGELOG.md` 本文件**追加**本节记录
- `docs/ideas/*` 创意文档已整体移入 `docs/archive/ideas/`（属于产品方向探索，不在主文档树清理范围）

---

## 2026-06-15: 文档系统性更新

**背景**：上一轮代码与文档脱节（Tauri / Vue 引用遍布 `docs/`，但代码已剥离前端），用户要求按当前代码系统性更新项目文档。

**变更**：

1. **根 `README.md` 全面重写**
   - 移除 Tauri / Vue 全部引用；
   - 明确项目当前形态为"纯 Rust 核心库 + E2E CLI"；
   - 新增插件清单、能力路由示例、CLI 用法、快速开始、最小工作流。

2. **`docs/README.md` 文档中心索引重建**
   - 重新组织为"核心架构设计 / 开发构建 / 设计草案 / 插件自包含 / 历史参考"五段；
   - 标注哪些文档"权威"、哪些"仅作历史参考"。

3. **`docs/explanation/*` 与 `docs/reference/*` 三份文档全部更新**
   - `ARCHITECTURE.md`：补充分形路由树示意、插件清单、内核模块表；
   - `OPERATION_MECHANISM.md`：移除 Vue EventHandler / useChatEventHandler 等前端细节；
   - `API_DESIGN.md`：聚焦 V3.0 上下文注入版的 `Plugin` Trait 与 `PluginPayload` 4 态。

4. **`docs/how-to/*` 三份文档全部更新**
   - `DEVELOPMENT_GUIDE.md`：聚焦机制化、Trait 抽象、Agent 子系统规范；
   - `BUILD_GUIDE.md`：移除 `pnpm tauri dev` 等前端命令，补 `cargo` 命令与排错；
   - `PLUGIN_DEVELOPMENT_GUIDE.md`：以 `weather` 插件为例演示完整链路。

5. **`docs/archive/design_docs/HISTORY_AND_REVIEWS.md` 重写**
   - 按 v0.1.x / v0.1.5+ / v8 / v9 / v9.1 五段回顾关键里程碑；
   - 总结"机制化 vs 硬编码 / 文档代码同源 / identity 本质 / 前端剥离"四条经验。

6. **未改动文件**：
   - 业务插件自包含文档（`symbio/src/plugins/agent/docs/`）保持不变；
   - 本文件下方历史记录按"历史参考"原样保留。

---

## 2026-07-06: MCP 插件重构——对齐系统工具机制 + 清理误删

**背景**：用户两次反馈纠正早期对 MCP 插件的错误理解：
1. "前端并不负责任何 MCP 的调用，前端只是配置"——纠正了之前把 MCP 客户端实现归到前端的错误方向。
2. "call_tool / discover / list_tools 不是被调用的，系统的工具有现成机制（参考 web 插件等），所以 mcp 插件不会主动被调用的"——纠正了"为 MCP 单独设计一套调用 API"的过度设计。

**结论**：
- **后端**承担 MCP **配置管理**（CRUD）+ **客户端 transport**（stdio / http）
- **前端**仅做配置 UI（CRUD）
- MCP 工具通过 **系统统一的 `Capability` trait + `traverse` + `tool_manager` 机制**集成到 agent——与 `web` 插件完全对齐

**变更**：

### 一、恢复 + 完善后端 MCP 客户端

1. **恢复 `mcp/stdio.rs` + `mcp/http.rs` + `mcp/types.rs`**（误删纠正）
   - `mcp/stdio.rs`：stdio transport（每次调用临时 spawn 子进程 + kill）
   - `mcp/http.rs`：http transport（每次调用新建短连接）
   - `mcp/types.rs`：JSON-RPC 2.0 协议层类型（`JsonRpcRequest` / `JsonRpcResponse` / `McpTool` / `McpToolCallResponse` / `McpInitializeResponse` 等）

2. **新建 `mcp/manager.rs`** —— 无状态 transport 路由器
   - `discover_tools(name, config)`：按 `transport_type` 路由到 stdio / http + 应用 `include_tools` / `exclude_tools` 过滤
   - `call_tool(name, config, tool_name, args)`：同上
   - **不维护**"激活集合"等运行时状态——是否可见由 `McpConfig.servers[name].enabled` 决定

3. **新建 `mcp/capability.rs`** —— `McpToolCapability`
   - 把单个 MCP 工具包装为标准 `Capability`（`meta()` + `execute(ctx)`）
   - 命名规则：`mcp.<server_name>.<tool_name>` 三段式
   - 分类：`CapabilityCategory::Mcp`（新增变体）

### 二、集成系统工具机制

4. **改造 `McpPlugin::traverse`**（参考 `WebPlugin::traverse`）
   - 每次 `parent.traverse(TRAVERSE_AVAILABLE_TOOLS)` 时遍历 `McpConfig.servers` 中 `enabled=true` 的项
   - 对每个 server 调 `McpManager::discover_tools` 动态发现工具
   - 把每个工具构造为 `McpToolCapability` 注册到 `ctx.get(CAPABILITY_MANAGER)`
   - agent 通过 `tool_manager.invoke("mcp.<server>.<tool>", ctx)` 调用（与 `web_search` 等一致）

### 三、配置层统一 + 持久化

5. **升级 `mcp_config::McpServerConfig`** 为完整版
   - 新增 `transport_type`（Stdio / Http / Sse）
   - 新增 `url`（http/sse 必填）
   - 新增 `include_tools` / `exclude_tools`（白/黑名单过滤）
   - 持久化路径不变：`~/.symbio/plugins/mcps/<name>/server.json`

6. **更新 `servers/set` 校验**：按 `transport_type` 校验必填字段（stdio → command；http/sse → url）

### 四、清理过度抽象

7. **删除 5 个多余 schema**：
   - `mcp_call_tool` / `mcp_discover` / `mcp_list_tools` / `mcp_register` / `mcp_unregister`
   - 这些功能通过 `Capability` trait + `tool_manager` 机制实现，不再需要单独 schema

8. **删除 `McpManager` 的过度抽象**：
   - 移除 `register` / `unregister` / `is_active` / `active_servers` 集合
   - 移除 `tools_to_capabilities` / `list_capabilities` / `shared_manager` / `register_result_message` 等辅助
   - 移除 `types::stdio_command_args_env` 等内部辅助

### 五、Skill 插件清理

9. **删除未使用的 `load_budget` / `estimate_tokens` 方法**（之前为了"预留"留下但实际未使用）

### 六、文档同步

10. **`docs/archive/proj/IMPROVEMENT_PLAN_2026.md` §2.3** 重写：反映"前端只做配置 + 后端实现 transport + 系统工具机制集成"的正确方向
11. **`mcp_servers.rs` 注释** 修正：删除"前端 tauri 端处理"的错误描述

**验证**：
- `cargo check`：✅ 零错误零警告
- `cargo test --lib`：✅ 233 tests passed

---

## 2026-04-04: LLM 配置改进

**新增功能**:

1. **LM Studio 支持**
   - 在 LLM 提供商下拉列表新增 "LM Studio" 选项
   - 默认 API 地址: `http://localhost:1234/v1`
   - 模型列表为空，支持用户手动输入模型名称

2. **模型名称可输入**
   - 将模型选择框从 `<select>` 改为 `<input list="models-list">`
   - 支持从预设列表选择（有建议列表）
   - 支持手动输入任意模型名称（无限制）
   - 适用于 LM Studio 等动态加载模型的场景

**修改文件**:

- `src/components/SettingsPage.vue`
  - 新增 LM Studio 提供商预设
  - 模型输入框改用 datalist 实现可选择可输入
  - 优化样式和交互逻辑

**技术说明**:

- 使用 HTML5 `<datalist>` 元素实现自动完成输入框
- LM Studio 的 `models` 设为空数组，因为模型是动态加载的
- 后端 token.rs 的 `get_model_config()` 会自动为未知模型使用默认配置

## 2026-04-04: 启动流程修复

**问题**:

- App 启动时直接进入导航页面，跳过了工作区选择页面
- 原因：`loadWorkspaceState()` 在检测到有效工作区路径时直接设置 `workspaceReady = true`

**修复**:

1. **始终显示欢迎页面**
   - 修改 `loadWorkspaceState()` 逻辑，无论是否有有效工作区路径，都设置 `workspaceReady = false`
   - 每次启动都从欢迎页面开始

2. **新增"继续上次工作区"功能**
   - 如果有上次使用的工作区路径，显示蓝色的"继续"按钮
   - 按钮显示路径信息，方便用户确认
   - 用户可以选择：
     - 点击"继续上次工作区"快速进入
     - 点击"浏览目录"选择新的工作区
     - 从最近使用列表中选择

**修改文件**:

- `src/views/HomeView.vue`
  - 修改 `loadWorkspaceState()` 函数逻辑
  - 新增 `continueLastWorkspace()` 函数
  - 模板中添加条件渲染的"继续"按钮
  - 添加 `.continue-btn` 样式

**技术说明**:

- 欢迎页面通过 `v-if="!workspaceReady"` 控制显示
- 工作区路径保存在后端配置文件中
- 最近使用列表最多保存 5 条记录

## 2026-04-04: 文件读写工具权限修复

**问题**:

- AI 对话中调用文件读取工具时提示："路径解析后超出允许范围"
- 例如读取 `Cargo.toml` 失败，即使文件在工作区内

**原因分析**:

1. `SecurityPolicy` 在初始化时使用 `std::env::current_dir()` 作为默认工作区
2. 实际工作区路径通过 `get_workdir()` 动态获取（从 work 插件）
3. **问题**：`SecurityPolicy` 的工作区路径从未被更新，导致安全检查使用错误的路径
4. `file_read.rs` 中的路径验证逻辑没有考虑 `workspace_only` 设置，即使 `workspace_only = false`（默认值）也强制检查

**修复方案**:

1. **修改 `file_read.rs` 路径验证逻辑**
   - 添加 `workspace_only` 条件判断
   - 当 `workspace_only = false` 时，允许读取任意系统文件（禁止路径除外）
   - 当 `workspace_only = true` 时，只允许读取工作区内的文件

2. **动态更新 `SecurityPolicy` 工作区路径**
   - 在 `ToolsPlugin` 结构体中保存 `security` 引用
   - 在每次工具调用前，通过 `update_workspace_dir()` 更新工作区路径
   - 确保安全检查使用最新的、正确的工作区路径

**修改文件**:

- `src-tauri/src/plugins/agent/tools/file_read.rs`
  - 第 91 行：添加 `if self.security.workspace_only` 条件
  - 只有限制模式下才强制检查工作区路径

- `src-tauri/src/plugins/agent/tools/plugin.rs`
  - 结构体新增 `security: Arc<SecurityPolicy>` 字段
  - 构造函数保存 `security` 引用
  - `invoke()` 方法在工具调用前更新工作区路径（两处：新格式和旧格式）

**权限策略**:

- **读取**：默认允许读取系统文件（除禁止路径外），`workspace_only = true` 时只读工作区
- **写入**：始终只允许写入工作区内（`is_path_allowed_for_write` 逻辑不变）
- **禁止路径**：包含 `..` 的路径、配置的 `forbidden_paths` 始终拒绝

## 2026-04-04: AI 对话流式显示修复

**问题**:

- AI 对话在调用 Tool 后，最终显示的回复内容为空
- 流式过程中 Tool 调用显示正常，但完成后消息内容为空字符串

**原因分析**:

1. 后端 Model 插件在处理工具调用循环时，`final_content` 只在**没有工具调用**时被赋值
2. 当有工具调用时，代码执行 `break` 跳出循环的逻辑不会触发，`final_content` 保持空字符串
3. 后端最终返回 `done: true` 时，`content` 字段为空
4. 前端收到空内容后，`else if (streamingContent.value)` 条件不满足，不创建消息或创建空消息

**关键代码位置** (`plugin.rs`):

```rust
// 第 825-829 行：没有工具调用时才赋值 final_content
if tool_calls.is_empty() {
    final_content = stream_content;
    break;
}

// 第 959 行：返回最终结果时 final_content 为空
yield StreamChunk {
    data: json!({
        "content": final_content,  // 这里为空！
        "done": true
    }),
    ...
}
```

**修复方案**:

1. **添加 `last_stream_content` 变量**
   - 在外层循环声明 `let mut last_stream_content = String::new()`
   - 用于保存每次迭代的流式内容

2. **初始化 `stream_content` 时使用上次内容**
   - 在流式请求开始时，如果 `last_stream_content` 非空，则使用它初始化 `stream_content`
   - 确保多轮工具调用时内容连续

3. **工具调用完成后保存内容**
   - 在所有工具调用完成后，从消息历史中提取 assistant 消息的 content
   - 将其赋值给 `final_content`，确保最终返回的内容非空

**修改文件**:

- `src-tauri/src/plugins/agent/openai/plugin.rs`
  - 第 681 行：新增 `last_stream_content` 变量
  - 第 742-747 行：修改 `stream_content` 初始化逻辑
  - 第 924-930 行：工具调用完成后从消息历史提取 content

**技术说明**:

- 后端使用 `async_stream::stream!` 宏实现流式返回
- 工具调用循环最多执行 255 次（防止无限循环）
- 每次循环都会累积 `stream_content`，最终需要正确传递给前端
- 前端使用 `streamingContent` 响应式变量接收流式内容，完成后创建消息对象

## 2026-04-19: 工具工作区目录统一修复

**问题**:

- 部分工具（`file_edit`、`glob_search`、`content_search）使用启动时的`current_dir()` 作为工作区目录
- 打开新工作区后，这些工具仍然使用旧的目录，而不是新的工作区
- 不同工具之间工作区目录不同步

**原因分析**:

1. `ToolsPlugin::new()` 中创建工具时，传入 `std::env::current_dir()` 作为默认工作区
2. `FileReadTool`/`FileWriteTool`/`ShellTool` 通过 `Arc<SecurityPolicy>` 获取工作区（**动态更新**）
3. `FileEditTool`/`GlobSearchTool`/`ContentSearchTool` 持有独立的 `Arc<RwLock<PathBuf>>`（**固定不变**）
4. 虽然 `invoke()` 前会调用 `security.update_workspace_dir()`，但只更新了 `SecurityPolicy`，没有更新三个工具的独立引用

**修复方案**:

采用**共享 `Arc<SecurityPolicy>`** 方案，让所有工具都通过 `SecurityPolicy` 获取工作区目录：

1. **修改 `FileEditTool`**
   - 结构体：`workspace_dir: Arc<RwLock<PathBuf>>` → `security: Arc<SecurityPolicy>`
   - 构造函数：`new(workspace_dir)` → `new(security)`
   - 使用：`self.workspace_dir.read().await` → `self.security.get_workspace_dir().await`

2. **修改 `GlobSearchTool`**
   - 同上，改为持有 `Arc<SecurityPolicy>`

3. **修改 `ContentSearchTool`**
   - 同上，改为持有 `Arc<SecurityPolicy>`

4. **修改 `ToolsPlugin::new()`**
   - 所有工具创建时都传入 `Arc::clone(&security)`
   - 确保所有工具共享同一个 `SecurityPolicy` 实例

**修改文件**:

- `src-tauri/src/plugins/agent/tools/file_edit.rs`
  - 结构体和构造函数修改
  - 两处 `workspace_dir` 使用改为通过 `security` 获取

- `src-tauri/src/plugins/agent/tools/glob_search.rs`
  - 结构体和构造函数修改
  - `workspace_dir` 使用改为通过 `security` 获取

- `src-tauri/src/plugins/agent/tools/content_search.rs`
  - 结构体和构造函数修改
  - 两处 `workspace_dir` 使用改为通过 `security` 获取

- `src-tauri/src/plugins/agent/tools/plugin.rs`
  - 工具创建时统一使用 `Arc::clone(&security)`

**架构优势**:

- ✅ **单一数据源**：所有工具的工作区目录来自同一个 `SecurityPolicy`
- ✅ **自动同步**：调用 `security.update_workspace_dir()` 后，所有工具立即生效
- ✅ **代码简化**：减少重复的 `Arc<RwLock<PathBuf>>` 管理
- ✅ **易于扩展**：新增工具只需传入 `Arc<SecurityPolicy>` 即可

**当前目录切换说明**:

- 现在项目**已使用** `std::env::set_current_dir()` 在打开工作区后切换进程当前目录
- 切换时机：
  1. **应用启动时**：根据配置文件中的 `workdir` 切换当前目录
  2. **用户选择新工作区时**：调用 `set_workspace` 后立即切换当前目录
- 所有工具通过显式路径拼接（`workspace_dir.join(path)`）或命令参数（`cmd.current_dir()`）使用工作区
- 当前目录切换是**额外的便利功能**，工具仍然通过 `SecurityPolicy` 获取工作区目录，确保安全

## 2026-04-04: Windows 路径规范化修复

**问题**:

- 在 Windows 上，`tokio::fs::canonicalize()` 返回带有 `\\?\` 前缀的路径（如 `\\?\C:\Bing\agiwave\symbio\docs`）
- 而 `workspace_dir` 没有这个前缀（如 `C:\Bing\agiwave\symbio`）
- 导致 `starts_with` 比较失败，即使路径实际上在工作区内

**原因分析**:

- Windows 的 `canonicalize` 返回 UNC 格式路径（`\\?\` 前缀）
- 直接字符串或路径比较会失败，因为前缀不一致

**修复方案**:

在 `policy.rs` 中添加路径规范化辅助函数：

```rust
/// 规范化路径用于比较
/// 在 Windows 上，canonicalize 返回带有 `\\?\` 前缀的路径，需要统一处理
pub fn normalize_path_for_comparison(path: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();
    // 移除 Windows UNC 路径前缀（如 \\?\）
    if path_str.starts_with("\\\\?\\") {
        PathBuf::from(&path_str[4..])
    } else {
        path.to_path_buf()
    }
}

/// 检查路径是否以另一个路径为前缀（规范化后比较）
pub fn path_starts_with_normalized(base: &Path, prefix: &Path) -> bool {
    let normalized_base = normalize_path_for_comparison(base);
    let normalized_prefix = normalize_path_for_comparison(prefix);
    normalized_base.starts_with(&normalized_prefix)
}
```

**修改文件**:

- `src-tauri/src/plugins/agent/tools/policy.rs` - 添加路径规范化函数，修改 `is_path_allowed` 和 `is_path_allowed_for_write`
- `src-tauri/src/plugins/agent/tools/file_edit.rs` - 使用 `path_starts_with_normalized`
- `src-tauri/src/plugins/agent/tools/file_read.rs` - 使用 `path_starts_with_normalized`
- `src-tauri/src/plugins/agent/tools/glob_search.rs` - 使用 `path_starts_with_normalized` 和 `normalize_path_for_comparison`
- `src-tauri/src/plugins/agent/tools/content_search.rs` - 使用 `path_starts_with_normalized`

**影响范围**:

- 所有使用 `canonicalize` 后进行路径比较的工具都得到修复
- 包括：`file_edit`、`file_read`、`glob_search`、`content_search`
- `SecurityPolicy` 中的路径验证也得到修复
