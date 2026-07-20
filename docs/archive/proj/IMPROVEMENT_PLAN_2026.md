# Symbio 项目级改进方案（2026 Q3 之后）

> **文档定位**：基于 **2026-07 当前代码状态** 制定的下一阶段项目级改进计划。
> 区别于：
> - `docs/architecture/*` — 描述"系统现在怎么运作"（事实）
> - `docs/design_docs/HISTORY_AND_REVIEWS.md` — 关键里程碑与历史复盘
> - `docs/agent/docs/PLAN.md` — Agent 插件内部待办（**0 issues, 234 tests**）
>
> 本文档专注于**项目级**待办——涉及多个插件、需要统一设计、影响系统演进方向的工作。
>
> > 早期提案（Skill/Subagent/Hook 提案、Qwen Code 对比、AI 聊天红/蓝设计/实施/对比）
> > 已于 2026-07 项目级文档同步中**统一删除**（内容已由代码实现或本方案覆盖）。
> > 关键决策轨迹仍保留在 `docs/design_docs/HISTORY_AND_REVIEWS.md` 中。

---

## 1. 当前状态画像（2026-07）

### 1.1 形态

- **后端**：`symbio/` Rust 核心库 + 6+ 业务插件
- **前端**：`tauri/` Vue 3 + TypeScript 桌面前端（一等公民）
- **CLI**：`symbio/src/bin/cli.rs` E2E 命令行
- **插件清单**：`home` / `composite` / `agent` / `session` / `model` / `local` / `web` / `skill` / `mcp` / `telegram` / `explorer` / `setting` / `hook`

### 1.2 Agent 插件健康度

| 维度 | 状态 | 备注 |
|------|------|------|
| 单元测试 | 🟢 234 通过 | v47-v54 多轮修复 |
| Clippy | 🟢 0 warnings | |
| ISSUES | 🟢 0 活跃问题 | 详见 `agent/docs/ISSUES.md` |
| 核心能力 | 🟢 3 个 capability | chat / cognition(5 op) / create |
| 存储后端 | 🟢 4 种 | dir / file / memory / sqlite |
| 认知体系 | 🟢 关系/展示机制化 | 由 prop CU 派生，无硬编码 |

### 1.3 核心设计原则（已贯彻）

- **分形插件架构**：容器与叶子插件实现同一 `Plugin` Trait
- **机制化优先**：关系/类型/行为均由 `seed_cus.jsonl` 派生
- **路径即路由**：`/agent/chat`、`/model/chat` 等
- **PluginPayload 4 态**：Empty / Data / Native / Session
- **PluginFrame 2 通道**：Data / Error

### 1.4 已实现的关键能力

下列能力在 2026-07 已落地，作为"现状"参照：

| 能力 | 当前实现 |
|------|----------|
| Skill 系统 | ✅ `plugins/skill/` 已在仓库中；详见 §2.4 |
| Subagent 子智能体 | ✅ 通过 `agent_create` + `agent_chat` 组合实现（无需新插件） |
| Hook 扩展机制 | ✅ `plugins/hook/` 已在仓库中 |
| 扁平消息树模型 | ✅ `symbio_core/schemas/chat_message.rs` + Tauri 端 `MessageNode.vue` |
| LLM 协议后端流适配器 | ✅ `plugins/model/protocols/{openai,anthropic,gemini}_*.rs` |
| 思考模式（Prompt 调整） | ✅ 由 `model` 插件协议层处理 |
| MCP 协议集成 | ✅ `plugins/mcp/` 已在仓库中 |
| Agent 输出风格 | ✅ `AgentConfig` 包含风格字段 |

> **结论**：当前已具备一个**相对完整的 LLM Agent 桌面应用**所需的全部基础能力。
> 后续改进应聚焦**真实未实现或薄弱的环节**（见 §2），而非重复已落地的能力。

---

## 2. 项目级改进方向（按优先级）

### 2.1 P0：文档系统性脱节修复（**2026-07-06 已完成**）

**问题**：历史提案文档与实际代码之间存在系统性脱节。`docs/proj/MODEL_CHAT_IMPROVEMENT_PLAN.md` 错误地宣称"所有功能已实施"且引用不存在的目录；`docs/design_docs/COMPARISON_WITH_QWEN_CODE.md` 仍说 Symbio 缺少 Skills/Subagents/Hooks（实际已落地）；`docs/README.md` 引用了不存在的 agent 文档；`docs/proj/PLAN.yml` 与 `TASK_INDEX.md` 是 2026-03 早期规划，已不适用。

**已完成项**（2026-07-06）：
1. ✅ 创建 `docs/proj/IMPROVEMENT_PLAN_2026.md`（**本文档**）作为项目要做的事的**唯一权威来源**
2. ✅ 修复 `docs/README.md` 中 agent 文档列表（移除不存在的 `PROMPT_ARCHITECTURE.md` / `OPERATIONS.md` / `CODE_ANALYSIS_REPORT.md`）
3. ✅ **删除**所有过时的历史文档（详见 CHANGELOG §"2026-07-06 项目级文档系统性同步"）：
   - `docs/design_docs/ARCHITECTURE_IMPROVEMENT.md`（提案已落地）
   - `docs/design_docs/MODEL_CHAT_REDESIGN.md`（设计已落地）
   - `docs/design_docs/COMPARISON_WITH_QWEN_CODE.md`（对比已过时）
   - `docs/proj/PLAN.yml`（2026-03 早期规划）
   - `docs/proj/TASK_INDEX.md`（2026-03 早期任务索引）
   - `docs/proj/tasks/T00x-*.md`（早期任务文件）
   - `docs/proj/MODEL_CHAT_IMPLEMENTATION_PLAN.md`（已落地）
   - `docs/proj/MODEL_CHAT_IMPROVEMENT_PLAN.md`（已落地且内容错误）
4. ✅ 修复 `docs/design_docs/HISTORY_AND_REVIEWS.md` 中"当前形态"错误描述（前端已于 2026-06 恢复为 Tauri）
5. ✅ `docs/CHANGELOG.md` 头部添加"当前形态声明"

---

### 2.2 ~~P0：Plugin Channel 跨进程边界~~ （**已移除**）

> **2026-07-06 决策**：移除本节。
>
> 原计划定义 `SessionAdapter` trait（`InProcessAdapter` + `TauriEventAdapter`），
> 让流式响应能在 Tauri host 模式下经 IPC 投递。
>
> **否决理由**：
> 1. **方向不当**：`PluginChannel` 是分形插件架构的**内部机制**，与具体 host（Tauri / CLI / 未来其他 host）耦合会污染核心抽象
> 2. **已有等价方案**：当前 Tauri 模式下，`plugins/model/chat` 等流式响应通过 session 插件的中转 + Tauri `emit_event` 实现，路径清晰
> 3. **YAGNI**：未来若有其他 host 需求，再评估针对性方案，不必预先抽象
>
> **新增替代**（P1-1 MCP / P1-2 Skill）：当需要为多 host 共享协议层时，**优先在协议层抽象**（如 MCP transports），而非在 Plugin Channel 抽象。

---

### 2.3 P1：MCP 桥接成熟化

**现状（2026-07-06 重构）**：

- **后端**（`symbio/src/plugins/mcp/`）：承担**配置管理**（`servers/list` / `servers/get` / `servers/set` / `servers/delete`）+ **MCP 客户端 transport**（stdio / http，按需 lazy 加载）
- **前端**（`tauri`）：**仅**负责 MCP Server 的配置管理（CRUD UI），不实现任何 transport
- **系统工具机制集成**（与 `plugins/web` 对齐）：
  - 每次 `parent.traverse(TRAVERSE_AVAILABLE_TOOLS)` 时，`McpPlugin` 遍历 `McpConfig.servers` 中 `enabled=true` 的项
  - 对每个 server 调 `McpManager::discover_tools` 动态发现工具
  - 把每个工具包装为 `McpToolCapability`（实现 `Capability` trait）注册到 `ctx.get(CAPABILITY_MANAGER)`
  - agent 通过 `tool_manager.invoke("mcp.<server>.<tool>", ctx)` 调用

**模块结构**（后端）：

```
symbio/src/plugins/mcp/
├── mod.rs         # 模块注册 + 职责划分文档
├── plugin.rs      # McpPlugin：traverse（动态注册）+ route（CRUD + 持久化）
├── manager.rs     # McpManager：无状态 transport 路由器
├── stdio.rs       # stdio transport（每次 spawn 子进程）
├── http.rs        # http transport（每次新建短连接）
├── types.rs       # JSON-RPC 协议层类型 + 配置 re-export
└── capability.rs  # McpToolCapability：把单个 MCP 工具包装为标准 Capability
```

**关键设计**：

- **Lazy 加载**：stdio 每次调用都 spawn 新的子进程、调用后立即 kill；http 每次都新建短连接。**不持有长连接 / 进程 / 连接池**——这是"有的时候才集成"的核心语义。
- **激活控制**：`McpConfig.servers[name].enabled` 是唯一开关。`traverse` 时只注册 `enabled=true` 的 server；不维护独立的"激活集合"。
- **工具命名**：三段式 `mcp.<server_name>.<tool_name>`，避免不同 server 之间的同名工具冲突。

**已完成**：

1. ✅ 后端 MCP 客户端实现（stdio + http transport）—— 全部功能由后端承担
2. ✅ 工具机制集成（与 web 插件一致）—— `traverse` 动态注册
3. ✅ 配置 CRUD 与持久化（`~/.symbio/plugins/mcps/<name>/server.json`）
4. ✅ `CapabilityCategory::Mcp` 分类
5. ✅ 删除 5 个多余 schema（`mcp_call_tool` / `mcp_discover` / `mcp_list_tools` / `mcp_register` / `mcp_unregister`）——这些功能通过 `Capability` trait + tool_manager 实现，不需要单独的 schema

**后续方向**：

- 短期：stdio transport 健壮性（spawn 超时 / kill 兜底 / 错误重试）
- 中期：HTTP transport 连接池 + SSE 长连接支持
- 长期：MCP Resource / Prompt 协议支持（仅在 transport 层扩展，不影响 `McpToolCapability` 抽象）

---

### 2.4 P1：Skill 系统实战化

**现状**：`plugins/skill/` 已在仓库。

**待完善**：
- `SKILL.md` 格式规范与解析器
- 技能发现（`/home/skills/list`）与显式/隐式触发
- 技能市场（用户分享/下载技能）
- 技能加载的 token 预算控制

**价值**：让"非程序员用户扩展 Agent 能力"成为可能。

---

### 2.5 P1：端到端 CI 化

**现状**：CI 跑 `cargo test` 但不跑 E2E。

**待完善**：
- E2E 测试拆为"无 LLM"（CI 必跑）+ "含 LLM"（nightly）两套
- 端到端 CLI 子命令覆盖所有核心插件
- 失败时自动上报完整 plugin 路由链

**价值**：减少回归风险，加快发布节奏。

---

### 2.6 P2：可观测性系统化

**现状**：Agent 插件已有 `MetricsSink` trait，但未接入 tracing / Prometheus。

**待完善**：
- `model` 插件的协议层 span（按协议/模型分组）
- `session` 插件的消息持久化延迟
- `agent` 插件的 mindscape 操作计数
- 统一导出 Prometheus 格式

**价值**：生产环境的可调试性与可维护性。

---

### 2.7 P2：HNSW ANN 接入（认知检索性能）

**现状**：EmbeddingStore 用桶索引近邻扫描，规模上去后会变慢。

**待完善**：
- 评估 HNSW（hnsw-rs）/ ScaNN 等 Rust 生态 ANN 库
- 替换近邻扫描（保持 API 不变）
- 10K+ CU 规模下的基准测试

**价值**：支撑更大规模的 agent 认知库。

---

### 2.8 P2：前端 Tauri 与 symbio_core 类型同步

**现状**：`tauri/src/schemas/`（部分已迁）与 `symbio/src/symbio_core/schemas/` 之间存在手动同步。

**待完善**：
- 评估 ts-rs / specta 等 Rust→TS 类型导出方案
- 选定后统一为单一类型源
- 移除手动维护的 TS 类型副本

**价值**：消除前后端类型漂移。

---

### 2.9 P3：插件外部动态加载

**现状**：所有插件与库同 crate 编译（in-tree 形式）。

**待完善**：
- `PluginFactoryRegistry::register_external(Arc<dyn PluginFactory>)` 公开 API
- 评估 `abi_stable` / `wasmtime` 方案
- `plugin_provider: "external:weather.wasm"` 配置协议
- 进程隔离与 IPC

**价值**：第三方插件生态基础。

---

### 2.10 P3：移动端/Headless 模式

**现状**：当前为桌面应用。

**待完善**：
- symbio core 的 wasm target 编译
- Headless CLI 的 JSON 输出协议
- Telegram 插件的增强（目前已有基础）

**价值**：覆盖更多使用场景（CI/CD、远程控制、移动端）。

---

## 3. 改进推进原则

### 3.1 机制化优先

> 任何"如何分类"、"如何展示"、"如何选择"的逻辑，**必须**由数据（prop CU / config / seed）驱动，不得在核心代码中硬编码 `match`。
> 这条原则自 v8 起贯彻，新增代码必须遵守。

### 3.2 文档代码同源

> 业务概念**优先用代码表达**（trait / 数据 / 测试），文档作为外部描述。
> 任何对核心机制的修改必须同步：单元测试 + 插件自包含文档 + `CHANGELOG.md`。

### 3.3 路径即契约

> 任何能力通过 `route("xxx", …)` 表达。
> 禁止出现"特殊插件路径"或绕过路由的隐式调用。

### 3.4 增量可回滚

> 改进以 PR 粒度推进，每个 PR 独立可回滚。
> 涉及 `core/` 接口的变更需经过 PRINCIPLES.md §3 检查清单。

---

## 4. 改进路线图

### 2026 Q3（立即）

- **2.1 文档系统性脱节修复**（✅ 已完成）
- **2.3 MCP 桥接成熟化**（开始）
- **2.4 Skill 系统实战化**（开始）
- **2.5 E2E CI 化**（开始）

### 2026 Q3-Q4

- **2.3 MCP 桥接成熟化**
- **2.4 Skill 系统实战化**
- **2.5 E2E CI 化**

### 2027 Q1-Q2

- **2.6 可观测性系统化**
- **2.7 HNSW ANN 接入**
- **2.8 前后端类型同步**

### 2027+（远期）

- **2.9 外部插件动态加载**
- **2.10 移动端/Headless**

---

## 5. 与现有文档的关系

| 文档 | 关系 |
|------|------|
| `architecture/ARCHITECTURE.md` | 上游：系统当前怎么运作（事实） |
| `architecture/OPERATION_MECHANISM.md` | 上游：关键流程示例 |
| `architecture/API_DESIGN.md` | 上游：API/Schema 设计 |
| `development/DEVELOPMENT_GUIDE.md` | 上游：开发规范 |
| `development/BUILD_GUIDE.md` | 上游：构建/部署规范 |
| `development/PLUGIN_DEVELOPMENT_GUIDE.md` | 上游：插件开发实战 |
| `design_docs/HISTORY_AND_REVIEWS.md` | 平行：关键里程碑与历史复盘 |
| `agent/docs/ARCHITECTURE.md` | 平行：Agent 插件架构 |
| `agent/docs/COGNITION.md` | 平行：认知单元数据规范 |
| `agent/docs/PRINCIPLES.md` | 平行：Agent 插件设计原则 |
| `agent/docs/TESTING.md` | 平行：测试体系 |
| `agent/docs/PLAN.md` | 平行：Agent 内部待办（已稳定） |
| `agent/docs/ISSUES.md` | 平行：Agent 活跃问题（当前 0 个） |
| `agent/docs/CHANGELOG.md` | 平行：Agent 变更日志 |
| `proj/IMPROVEMENT_PLAN_2026.md` | **本文件**：下一阶段项目级改进 |
| `CHANGELOG.md` | 平行：项目变更日志 |

---

## 6. 深入阅读

- `docs/README.md` — 文档中心索引
- `docs/CHANGELOG.md` — 项目变更日志
- `symbio/src/plugins/agent/docs/PLAN.md` — Agent 内部待办
- `docs/development/PLUGIN_DEVELOPMENT_GUIDE.md` — 插件开发实战
