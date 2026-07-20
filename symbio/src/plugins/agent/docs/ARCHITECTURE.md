# Symbio Agent：系统架构

> **定位**：本文档聚焦 **agent 插件内部**的模块划分、依赖关系、数据流与创建链路。
> 分形架构、核心 `Plugin` Trait、能力系统等**跨插件**设计以 [docs/explanation/ARCHITECTURE.md](../../../docs/explanation/ARCHITECTURE.md) 为准。
> 接口规范见 PRINCIPLES.md，数据规范见 COGNITION.md，进度追踪见 PLAN.md。
> 
> **最后更新**：2026-06-19（v44 存储层重构 + AgentEngine 合并）

---

## 1. 分层架构

```
┌─────────────────────────────────────────────────────────────────────┐
│                    应用层 (handlers/)                                │
│  chat.rs（含 ContextBuilder） | get.rs | list.rs | config.rs       │
│  system_prompt.rs | default_tool_manager.rs                         │
├─────────────────────────────────────────────────────────────────────┤
│                    能力层 (capabilities/)                            │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  对话能力：AgentChatTool (agent_chat)                        │   │
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
| `chat.rs` | 对话处理 + ContextBuilder（消息上下文构建） |
| `system_prompt.rs` | 系统提示词构建（从 identity CU 获取身份信息） |
| `get.rs` | 获取 Agent 信息 |
| `list.rs` | 列出 Agent |
| `config.rs` | Agent 配置管理 |

**系统提示词分层**（由 `handlers/chat.rs` + `system_prompt.rs` 构建）：
```
系统提示词 = 全局指令 + 工作区指令 + 心智认知

1. 全局指令：~/.symbio/AGENTS.md
2. 工作区指令：{workdir}/AGENTS.md
3. 心智认知：system_prompt::build(store)
   ├── 身份锚定：id="identity" 的 CU，全量注入
   ├── 行为规则：rule 类型，全量注入，按 confidence 降序
   └── 认知索引：其余 sys 级，摘要注入
```

### 3.4 capabilities/ 能力层

#### 核心能力（3 个）

| 工具 | 名称 | 职责 | 操作数量 | 状态 |
|------|------|------|----------|------|
| `AgentChatTool` | agent_chat | 与智能体对话 | — | ✅ 已实现 |
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

### 4.1 对话流

```
用户消息
  │
  ▼
handlers/chat.rs
  │  ├── 获取工具列表（traverse → 注册 capabilities）
  │
  ├── 构建系统提示词
  │     ├── load_system_agents_md()      → ~/.symbio/AGENTS.md
  │     ├── load_workspace_agents_md()   → {workdir}/AGENTS.md
  │     └── system_prompt::build(store)  → 从 identity CU 获取身份信息
  │
  ├── 消息上下文构建（ContextBuilder，内置于 chat.rs）
  │     ├── 语义检索：engine.semantic_search(user_text, limit=5)
  │     ├── 时间上下文：当前时间 + 工作区
  │     ├── 任务上下文：engine.query(FilterExpr::is_a("strategy")) + engine.query(FilterExpr::is_a("skill"))
  │     └── 注入：作为 msg.prompt 前缀
  │
  └── 路由到 Model 服务（parent.route("model/chat")）
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

## 6. Active Memory 机制

每轮用户消息到达时（`handlers/chat.rs` 中的 `ContextBuilder`）：
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
