# Symbio Agent：系统架构

> **定位**：本文档聚焦 **agent 插件内部**的模块划分、依赖关系、数据流与创建链路。
> 分形架构、核心 `Plugin` Trait、能力系统等**跨插件**设计以 [docs/explanation/ARCHITECTURE.md](../../../docs/explanation/ARCHITECTURE.md) 为准。
> 接口规范见 PRINCIPLES.md，数据规范见 COGNITION.md，进度追踪见 PLAN.md。
> 
> **最后更新**：2026-06-19（v44 存储层重构 + AgentEngine 合并）

---

## 1. 分层架构

> **重构说明（agent 降级为普通插件）**：`default_tool_manager.rs` 已外迁至
> `symbio_core::DefaultToolManager`（跨插件共享设施）。agent 插件与 local / web /
> mcp / skill 完全同构——唯一参与会话的方式是 `traverse(TRAVERSE_AVAILABLE_TOOLS)`
> 向会话贡献工具，是否贡献取决于 `ctx[AGENT_ID]` 是否存在。人格不再写入系统提示词，
> 改由 `agent_identity` 能力的**工具说明**承载。

```
┌─────────────────────────────────────────────────────────────────────┐
│                    应用层 (handlers/)                                │
│  chat.rs（仅子智能体会话执行入口）| get.rs | list.rs | config.rs   │
│  system_prompt.rs（仅渲染人格文本 build_persona）                   │
├─────────────────────────────────────────────────────────────────────┤
│                    能力层 (capabilities/)                            │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  身份能力：AgentIdentityTool (agent_identity) —— 人格载体    │   │
│  │  对话能力：AgentChatTool (agent_chat) —— 子智能体委托        │   │
│  ├──────────────────────────────────────────────────────────────┤   │
│  │  统一认知能力：AgentCognitionTool                            │   │
│  │    └── ops/ 目录：5 个操作，每个自注册（submit_cognition_op!） │   │
│  │    └── memory/     5 个操作（save/retrieve/graph_query/reflect/consolidate）│
│  │           delete 已废除 → save {confidence:0} 软删除立即生效    │
│  ├──────────────────────────────────────────────────────────────┤   │
│  │  特殊能力：AgentCreateTool (agent_create)                    │   │
│  └──────────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────────┤
│                    存储层 (store/)                                  │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  mindscape/  认知能力层（MindscapeScaffold）                 │   │
│  │    scaffold.rs | cognitive_feedback.rs                       │   │
│  ├──────────────────────────────────────────────────────────────┤   │
│  │  embedding/  语义搜索装饰器（EmbeddingStore）                │   │
│  ├──────────────────────────────────────────────────────────────┤   │
│  │  dir/ | file/ | memory/ | sqlite/  基础存储后端              │   │
│  └──────────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────────┤
│                    协议层 (core/)                                   │
│  AgentStore trait | CognitiveUnit | Config | CognitionContext       │
└─────────────────────────────────────────────────────────────────────┘
```

> **共享服务（已外迁）**：`EmbeddingService` trait 现位于 `symbio_core::providers::embedding`，
> 实现（`FastEmbedService` / `NoopEmbeddingService`）位于 `src/providers/embedding/`，
> 经对象工厂 `create_object::<dyn EmbeddingService>("fastembed", ctx)` 跨插件共享获取，
> 不再内置于本插件（避免插件间相互依赖）。

### 1.1 各层职责

| 层级 | 目录 | 职责 |
|------|------|------|
| **应用层** | `handlers/` | 请求路由、参数解析、响应组装、系统提示词构建、消息上下文构建 |
| **能力层** | `capabilities/` | LLM 可调用的工具能力（3个核心能力：chat/cognition/create） |
| **存储层** | `store/` | 统一存储架构：基础后端 + EmbeddingStore 装饰器 + Mindscape 认知层 |
| **协议层** | `core/` | trait 定义（`AgentStore`）、纯数据类型、错误类型、配置类型 |
| **共享服务（外迁）** | `symbio_core::providers::embedding` + `src/providers/embedding/` | 向量嵌入抽象与实现，经对象工厂跨插件共享（原 `embedding/` 已迁出） |

---

## 2. 存储层架构

存储层采用**分层装饰器**模式，每层独立子目录：

```
build_store(config, agent_dir)  ← 统一构建入口
  │
  ▼
MindscapeScaffold (store/mindscape/)  ← 认知能力层
  │  - 认知单元验证 + 语义去重
  │  - 认知反馈（belief 衰减 + 冲突检测）
  │  - 高级搜索（JSON filter + 自动 record_access）
  │
  ▼
EmbeddingStore (装饰器，实现位于 `src/providers/embedding/`，经对象工厂跨插件共享)  ← 语义搜索装饰器
  │  - 自动向量化（insert/update 时计算 embedding）
  │  - 向量相似度检索
  │  - 异步 embed 队列 + 冷启动降级
  │
  ▼
Inner Store (store/{dir|file|memory|sqlite}/)  ← 基础存储后端
     DirStorage | FileStorage | MemoryStorage | SqliteStorage
```

### 2.1 store/ 目录结构

```
store/
  mod.rs                    # 统一构建入口（build_store）
  dir/mod.rs                # DirStorage — 每个认知单元一个独立文件
  file/mod.rs               # FileStorage — 单文件存储（JSON/JSONL/YAML）
  memory/mod.rs             # MemoryStorage — 仅内存（测试用）
  sqlite/mod.rs             # SqliteStorage — SQLite + FTS5 全文检索
  mindscape/
    mod.rs                  # 工厂函数 + seed CU 加载
    scaffold.rs             # MindscapeScaffold — 种子 CU 初始化 + prop 校验 + CognitiveFeedback
    scaffold_tests.rs
    cognitive_feedback.rs   # 认知反馈（belief 衰减 + 攒批 flush）
    seed_cus.jsonl          # 核心种子 CU（四层认知体系）
```

### 2.2 构建机制

```rust
// 完整认知 store（inner + EmbeddingStore + MindscapeScaffold）
pub async fn build_store(config: &AgentConfig, agent_dir: &Path) -> Arc<dyn AgentStore>
```

> 统一构建入口 `build_store` 内部按 `config.storage_backend` 创建基础存储，叠加 `EmbeddingStore` 装饰器，再包裹 `MindscapeScaffold` 认知层。

**关键设计**：`AgentStore` 是唯一的存储接口 trait。`MindscapeScaffold` 和 `EmbeddingStore` 都实现 `AgentStore`，通过装饰器模式叠加能力。

---

## 3. 核心模块详解

### 3.1 core/ 协议层

| 文件 | 内容 | 说明 |
|------|------|------|
| `store.rs` | `AgentStore` trait, `FilterExpr`, `PageRequest`, `PageResult`, `StoreError` | 唯一的存储协议（含基础 CRUD + 认知层操作） |
| `traits.rs` | `CognitionContext` | 引擎构建上下文（agent_config + agent_dir） |
| `types.rs` | `cu_fields` 字段常量（ID/NAME/...）、`CuRef`/`generate_short_id` 等 | 核心字段常量与辅助类型 |
| `typed_unit.rs` | `CognitiveUnit` struct, `UnitMeta` | 类型安全的认知单元（`CognitiveUnitExt` 已于 v9.5 废除，方法并入 `CognitiveUnit`） |
| `error.rs` | `AgentError`, `AgentResult`, `ErrorLevel` | 错误类型 |
| `config.rs` | `AgentConfig`, `StorageBackendType`, `StorageFormat` | 配置类型 |

**`AgentStore` trait**：统一的存储接口（仅 CognitiveUnit 接口）：
- **基础 CRUD**：`get`/`insert`/`update`/`upsert`/`delete`/`query`/`semantic_search`/`count`
- **生命周期**：`record_access`/`cancel_background_tasks`/`shutdown`/`insert_batch`

### 3.2 store/mindscape/ 认知能力层

**MindscapeScaffold** — `AgentStore` 的认知层实现：

| 组件 | 职责 |
|------|------|
| `store` | `Arc<dyn AgentStore>` — 委托给 EmbeddingStore |
| `feedback` | `CognitiveFeedback` — belief 衰减 + 攒批 flush |
| `snapshot_cache` | COW 快照 — 验证用，写时失效 |

### 3.3 handlers/ 应用层

| 文件 | 职责 |
|------|------|
| `chat.rs` | **仅**子智能体会话执行入口（`agent_run` 派生）；校验智能体存在 → 统一管线收集工具 → 转交 `model/chat` |
| `system_prompt.rs` | 人格文本渲染（`build_persona`，从 identity CU 获取身份信息） |
| `get.rs` | 获取 Agent 信息 |
| `list.rs` | 列出 Agent |
| `config.rs` | Agent 配置管理 |

**人格文本分层**（由 `system_prompt.rs::build_persona` 渲染，供 `agent_identity` 工具说明嵌入）：

> 重构后人格**不再写入系统提示词**——顶层会话由 session 插件编排（session 组装
> `AGENTS.md` 环境级指令 + 时间上下文），agent 仅通过 `agent_identity` 能力的
> `description` 把人格随工具定义送达 LLM（每轮请求自动可见）。

```
人格文本 = build_persona(store)
   ├── 身份锚定：id="identity" 的 CU，全量注入
   ├── 行为规则：rule 类型，全量注入，按 confidence 降序
   └── 认知索引：其余 sys 级，摘要注入（受预算截断）
```

### 3.4 capabilities/ 能力层

#### 核心能力（4 个）

| 工具 | 名称 | 职责 | 操作数量 | 状态 |
|------|------|------|----------|------|
| `AgentIdentityTool` | agent_identity | **人格载体**：身份/规则/策略/预算随工具说明送达 LLM | — | ✅ 已实现 |
| `AgentChatTool` | agent_chat | 子智能体委托（`agent_run`） | — | ✅ 已实现 |
| `AgentCognitionTool` | agent_cognition | **统一认知体系** | 5 个操作（memory 域） | ✅ 全部实现 |
| `AgentCreateTool` | agent_create | 创建新的 Agent | — | ✅ 已实现 |

#### `agent_cognition` 操作体系

5 个操作分布在 `ops/memory/` 目录下，每个操作是独立的 `.rs` 文件，通过 `submit_cognition_op!` 宏自注册。

| 域 | 操作 | 文件 |
|------|------|------|
| memory | save | `ops/memory/save.rs`（**软删除也用它**：`{id, confidence:0}` 立即物理删除） |
| memory | retrieve | `ops/memory/retrieve.rs` |
| ~~memory~~ | ~~delete~~ | **已废除**（由 save 软删除取代） |
| memory | graph_query | `ops/memory/graph_query.rs` |
| memory | reflect | `ops/memory/reflect.rs` |
| memory | consolidate | `ops/memory/consolidate.rs`（周期任务：遗忘/合并/晋升） |

> **历史**：原计划在 reason/learn/plan/metacognition 4 个域共 26 个操作，当前实现仅落地 memory 域的 5 个核心操作。其他域操作将在后续版本按需补全（详见 ISSUES.md I-050）。

### 3.5 manager/ 管理器

| 组件 | 职责 |
|------|------|
| `AgentManager` | Agent 列表缓存、store 缓存（moka）、Agent CRUD、路径解析 |
| `AgentProfile` | Agent 身份信息（id, name, description, base_dir） |
| `create_agent` | 创建 Agent 的工具能力 |

**缓存策略**：
- `cached_agents`：`RwLock<HashMap<workdir_key, Vec<AgentProfile>>>`，按 workdir 分键
- `mindscapes`：`moka::Cache<String, Arc<dyn AgentStore>>`，最大 100，空闲超时 30 分钟

---

## 4. 数据流

### 4.1 对话流（子智能体会话）

> 顶层会话已不经过本插件（由 session 插件编排）。本插件只在**两种**情况下参与：
> 1. 会话选定智能体 → `traverse` 贡献智能体工具（含 `agent_identity` 人格载体）
> 2. 一次会话内部 `agent_run` 委托另一个智能体 → 走 `handlers/chat.rs` 起子会话

```
（会话选定智能体）
  │
  ▼
plugin.rs::traverse(TRAVERSE_AVAILABLE_TOOLS)
  │  ├── ctx[AGENT_ID] 存在 → 渲染人格（render_persona）
  │  ├── 不存在 → 早退，不贡献任何工具
  │  └── 注册 4 个能力（identity/chat/cognition/create）到 tool_manager
  │
（agent_run 委托子智能体）
  │
  ▼
capabilities/chat.rs::execute（AgentChatTool）
  │  ├── 校验目标智能体存在
  │  └── handlers/chat.rs
  │        ├── 统一管线收集工具（collect_capabilities）
  │        └── 转交 model/chat
```

### 4.2 认知存储流

```
LLM 调用 agent_cognition (operation: "memory.save", content: "...", type: "semantic")
  │
  ▼
AgentCognitionTool::execute()
  │
  ▼
dispatch_cognition() → ops::execute_op("memory.save", engine, params)
  │
  ▼
SaveOp::execute(engine, params)
  │
  ▼
MindscapeScaffold::upsert(&cu)
  │
  ├── engine.get(id) — 检查是否存在
  ├── 存在 → engine.update(&unit)  不存在 → engine.insert(&unit)
  └── invalidate_snapshot_cache()
```

---

## 5. 创建与缓存

### 5.1 创建链路

```
AgentPlugin::get_mindscape(workdir, agent_id)
  └── AgentManager::get_agent_engine(workdir, agent_id, config)
        ├── get_agent(workdir, agent_id)  → AgentProfile
        ├── CognitionContext::new(config, agent_dir)
        └── store::build_store(config, agent_dir)
            ├── 按 config.storage_backend 创建基础存储
            ├── EmbeddingStore::new(inner, embed_service)
            └── MindscapeScaffold::new_with_inner(store, ctx)
                  ├── init_metacognitive_units(&store)
                  └── CognitiveFeedback::new(store)
```

---

## 6. Active Memory 机制（已移除）

> **重构变化**：每轮自动注入的 `<active_memory>` 语义记忆片段已**移除**。
> 工作记忆不再由系统在每轮替 LLM 灌入上下文，而是由 LLM 主动调用 `agent_cognition`
> 的 `memory.retrieve`（`filter:{"semantic":"..."}`）按需回忆——这正是"自我进化"的方向：
> 由 LLM 自主决定何时回忆、何时固化，而非系统每轮做主。

旧的自动注入语义（供参考，已废止）：
1. 提取用户消息文本
2. 语义检索：`engine.semantic_search(text, limit=5)`
3. 过滤：排除 identity 单元和 rule 类型
4. 注入：作为用户消息的 `<active_memory>` 前缀
5. 兜底：若无匹配结果，整段不注入（零 Token 消耗）

---

## 7. 深入阅读

- [**PRINCIPLES.md**](./PRINCIPLES.md)：架构原则与质量标准
- [**COGNITION.md**](./COGNITION.md)：认知单元数据规范
- [**PLAN.md**](./PLAN.md)：执行计划与进度追踪
- [**TESTING.md**](./TESTING.md)：自动化测试体系
- [**ISSUES.md**](./ISSUES.md)：当前活跃问题清单
- [**CHANGELOG.md**](./CHANGELOG.md)：修复历史
