# Symbio Agent：自动化测试体系

> **定位**：Agent 功能开发的自动化测试规范、流程和最佳实践。
> 不包含：架构细节（见 ARCHITECTURE.md）、数据规范（见 COGNITION.md）。
> **最后更新**：2026-06-19（v44 系统性扫描）

---

## 1. 测试体系概述

### 1.1 测试分层架构

```
┌─────────────────────────────────────────────────────────────────────┐
│          E2E 测试层 (tests/e2e_test.rs)【规划中，尚未落地】         │
│  完整对话流程验证、推理能力评估、LLM 响应质量验证                   │
├─────────────────────────────────────────────────────────────────────┤
│                 高阶能力测试层 (capabilities/ ops 文件内联)         │
│  memory 域 5 个操作（save/retrieve/graph_query/reflect/consolidate）│
│  测试以内联 #[cfg(test)] 形式随 ops 实现文件一起维护                 │
├─────────────────────────────────────────────────────────────────────┤
│                    单元测试层 (src/lib.rs 内联)                    │
│  存储后端：DirStorage/FileStorage/MemoryStorage/SQLite              │
└─────────────────────────────────────────────────────────────────────┘
```

### 1.2 现有测试资源

| 资源 | 位置 | 说明 |
|------|------|------|
| **单元测试** | `src/plugins/agent/store/*.rs` | 存储后端测试，全部通过 |
| **高阶能力测试** | `src/plugins/agent/capabilities/`、`ops/` 各文件内联 | 以 `#[cfg(test)]` 模块内联，或同目录 `tests.rs` / `*_tests.rs` |
| **认知层测试** | `store/mindscape/scaffold_tests.rs`、`store/mindscape/cognitive_feedback.rs` | 认知层 CRUD/快照/搜索、belief 衰减/冲突检测（测试内联） |
| **E2E 测试** | `tests/e2e_test.rs` | **规划中，尚未落地** |
| **Embedding 测试** | `src/providers/embedding/` | embedding 已迁出 `store/embedding/`，测试随新位置维护 |

---

## 2. 单元测试 (存储层)

### 2.1 测试范围

| 模块 | 测试文件 | 测试内容 |
|------|----------|----------|
| DirStorage | `src/plugins/agent/store/dir.rs` | 目录模式读写 |
| FileStorage | `src/plugins/agent/store/file.rs` | 单文件 JSON/YAML |
| MemoryStorage | `src/plugins/agent/store/memory.rs` | 内存存储 |
| SQLiteStorage | `src/plugins/agent/store/sqlite.rs` | SQLite 后端 |

### 2.2 运行单元测试

```bash
# 运行所有单元测试
cargo test

# 仅运行 agent store 模块测试
cargo test -- plugins::agent::store

# 运行特定存储后端测试
cargo test -- plugins::agent::store::dir
cargo test -- plugins::agent::store::memory
```

### 2.3 存储层基础测试（历史存档）

> 以下仅展示 4 个存储后端的基础测试子集。完整测试统计见 §2.4。

```
running 6 tests
test plugins::agent::store::memory::tests::test_memory_storage ... ok
test plugins::agent::store::file::tests::test_file_storage_json ... ok
test plugins::agent::store::file::tests::test_file_storage_yaml ... ok
test plugins::agent::store::dir::tests::test_dir_storage_single_file_yaml ... ok
test plugins::agent::store::dir::tests::test_dir_storage_directory_mode ... ok
test plugins::agent::store::sqlite::tests::test_sqlite_storage ... ok

test result: ok. 6 passed; 0 failed
```

### 2.4 全量测试统计（v44）

| 测试分类 | 数量 | 位置 |
|----------|------|------|
| 存储后端（Dir/File/Memory/SQLite） | 6 | `store/{dir,file,memory,sqlite}/tests.rs` |
| EmbeddingStore（向量索引/量化/召回率） | 若干 | `src/providers/embedding/`（已迁出 `store/embedding/`） |
| MindscapeScaffold（认知层 CRUD/快照/搜索） | 23+ | `store/mindscape/scaffold_tests.rs` |
| CognitiveFeedback（belief 衰减/冲突检测） | 24+ | `store/mindscape/cognitive_feedback.rs`（测试内联） |
| CognitiveUnit（类型/关系/序列化） | 20+ | `core/typed_unit_tests.rs` |
| Ops 操作层（memory 域 5 个操作） | 若干 | `capabilities/ops/` 各文件内联 |
| Handler 层（chat ContextBuilder + get/list/config） | 16+ | `handlers/{chat,get,list,config}` 内联 |
| 其他（types/embedding/config） | 若干 | 各模块内联 |
| **总计** | **以 `cargo test --lib plugins::agent` 实时输出为准** | |

> 测试持续增长。运行 `cargo test --lib plugins::agent` 查看最新结果。

---

## 3. 高阶能力测试

### 3.1 核心能力模块（v43 统一认知工具）

| 模块 | 文件 | 功能 | 测试重点 |
|------|------|------|----------|
| **Cognition** | `capabilities/cognition.rs` | 统一认知体系 | 5 个操作（memory 域），通过 `operation` 分发 |
| **Chat** | `capabilities/chat.rs` | 对话能力 | 消息处理 |
| **Create** | `manager/create_agent.rs` | 创建智能体 | Agent 创建 |

### 3.2 `agent_cognition` 操作体系

所有操作通过 `submit_cognition_op!` 宏自注册到全局 `OpRegistry`，新增 op 无需修改任何中间模块。

**当前已实现 op（5 个，全部在 memory 域）**：

| 域 | 操作 | 文件 | 状态 |
|------|------|------|------|
| memory | save（**软删除也用它**：`{id, confidence:0}` 立即物理删除）| `ops/memory/save.rs` | ✅ |
| memory | retrieve | `ops/memory/retrieve.rs` | ✅ |
| memory | graph_query | `ops/memory/graph_query.rs` | ✅ |
| memory | reflect | `ops/memory/reflect.rs` | ✅ |
| memory | consolidate（周期任务：遗忘/合并/晋升）| `ops/memory/consolidate.rs` | ✅ |


> **历史**：原计划在 reason/learn/plan/metacognition 4 个域共 26 个操作（CHANGELOG.md v40-v46），当前落地为 memory 域 5 个核心操作。其他域操作按需补全（ISSUES.md I-050）。

**LLM 调用方式**（扁平 JSON，不需要嵌套 params）：
```json
{"operation": "memory.save", "content": "...", "is_a": ["fact"], "confidence": 0.9}
{"operation": "memory.save", "id": "cu_xxx", "confidence": 0}  // 软删除立即生效
{"operation": "memory.consolidate", "dry_run": true}
```

### 3.3 运行高阶能力测试

```bash
# 运行所有高阶能力测试（测试随 ops 实现文件内联，无独立 capability_test target）
cargo test --lib plugins::agent::capabilities

# 运行特定内存域操作测试
cargo test --lib plugins::agent::capabilities -- test_memory_save
cargo test --lib plugins::agent::capabilities -- test_memory_save_batch
cargo test --lib plugins::agent::capabilities -- test_memory_retrieve
cargo test --lib plugins::agent::capabilities -- test_memory_soft_delete
cargo test --lib plugins::agent::capabilities -- test_memory_graph_query
cargo test --lib plugins::agent::capabilities -- test_memory_reflect
cargo test --lib plugins::agent::capabilities -- test_memory_consolidate
```

### 3.4 高阶能力测试用例

| 测试名称 | 测试能力 | 描述 |
|----------|----------|------|
| `test_memory_save` | memory.save | 验证单条记忆保存 |
| `test_memory_save_batch` | memory.save_batch | 验证批量记忆保存 |
| `test_memory_retrieve` | memory.retrieve | 验证语义检索功能 |
| `test_memory_soft_delete` | memory.save (`{id, confidence:0}`) | 验证软删除立即生效 |
| `test_memory_graph_query` | memory.graph_query | 验证图结构查询 |
| `test_memory_reflect` | memory.reflect | 验证反思→CU 生成 |
| `test_memory_consolidate` | memory.consolidate | 验证自动遗忘/合并 |
| ~~`test_memory_delete`~~ | ~~memory.delete~~ | **已废除**（由 soft delete 取代）|
| ~~`test_memory_list`~~ | ~~memory.list~~ | **未实现**（合并到 retrieve）|

> **注意**：reason./learn./plan./metacognition. 各域操作（如 reason.search_evidence、learn.extract、plan.decompose、metacognition.introspect 等）**尚未实现**，仅为路线图目标（ISSUES.md I-050），相关测试暂不存在。

---

## 4. 端到端测试 (E2E)

### 4.1 E2E 测试结构

**测试文件**：`tests/e2e_test.rs` —— **⚠️ 规划中，尚未落地**。当前仓库中不存在该文件，以下结构为设计目标，请勿直接运行。

**核心组件（规划）**：
- `E2ETestTool`：模拟完整对话流程
- `VerificationTool`：使用第二个 LLM 验证响应质量

### 4.2 运行 E2E 测试

> 该测试尚未实现，下列命令暂不可用：

```bash
# （规划中）运行所有 E2E 测试（需要 LLM API）
# cargo test --test e2e_test

# （规划中）运行特定测试
# cargo test --test e2e_test -- test_reasoning_causal_inference
```

**当前可行的端到端验证方式**：通过 Tauri 桌面端（`tauri/`）或宿主直接调用 `route()`；单元测试使用 `cargo test --lib`。

### 4.3 E2E 测试用例

| 测试名称 | 测试能力 | 描述 |
|----------|----------|------|
| `test_reasoning_causal_inference` | 因果推理 | 验证多链条因果分析能力 |
| `test_reasoning_counterfactual` | 反事实推理 | 假设变更的影响分析 |
| `test_reasoning_chain_effect` | 连锁反应 | 多层因果传导分析 |
| `test_abstraction_hierarchy` | 抽象层次 | 多层次结构推理 |
| `test_temporal_reasoning` | 时序推理 | 因果时序关系分析 |
| `test_analogical_reasoning` | 类比推理 | 跨领域知识迁移 |
| `test_systems_thinking` | 系统思维 | 系统反馈循环分析 |
| `test_uncertainty_reasoning` | 不确定性推理 | 概率和模糊性处理 |
| `test_metacognition` | 元认知 | 自我反思和认知监控 |
| `test_memory_store` | 记忆存储 | 认知单元持久化 |
| `test_memory_recall` | 记忆召回 | 语义记忆检索 |
| `test_memory_lifecycle` | 记忆生命周期 | 记忆的创建和演化 |
| `test_empathy` | 共情能力 | 情感理解和回应 |
| `test_goal_prioritize` | 目标优先级 | 多目标排序和资源分配 |
| `test_attention_focus` | 注意力聚焦 | 关键信息识别 |
| `test_motivation_query` | 动机分析 | 行为背后的动机推断 |
| `test_multi_perspective` | 多视角思维 | 不同立场分析 |

### 4.4 E2E 测试原理

```rust
// 1. E2ETestTool 发送消息并收集响应
let response = tool.chat("test_name", prompt).await;

// 2. VerificationTool 使用第二个 LLM 验证质量
let result = verifier.verify_response(
    "test_name",
    prompt,
    &response,
    &[
        "识别出多个因果链条（至少2个）",
        "包含蒸腾作用、遮荫效应、热岛效应等关键因素",
        // ... 更多验证点
    ]
).await;

// 3. 验证结果
assert!(result.passed || result.score >= 70.0);
```

**验证协议**：
```
VERDICT: PASSED 或 FAILED
SCORE: 0-100 的数字
REASON: 一句话理由
```

---

## 5. 推理能力专项测试

### 5.1 推理测试 Agent

| 属性 | 值 |
|------|-----|
| **ID** | `reasoning_tester` |
| **定位** | 专门用于测试推理和元认知能力 |
| **知识库路径** | `~/.symbio/agents/reasoning_tester/` |

### 5.2 知识库结构

**事实知识** (fact)：
| ID | 名称 | 描述 |
|:---|:---|:---|
| rain | 降雨 | 一种气象现象，水滴从云中降落 |
| wet_ground | 地面变湿 | 地表被水覆盖的状态 |
| evaporation | 蒸发 | 液体转变为气体的过程 |
| cloud_form | 云层形成 | 水蒸气凝结形成云 |
| fire | 火焰 | 燃烧产生的可见光和热量 |
| smoke | 烟雾 | 燃烧产生的气体和颗粒物混合物 |

**因果规则** (rule)：
| ID | 前提 | 结果 | 置信度 |
|:---|:---|:---|:---:|
| cause_rain_wet | rain | wet_ground | 0.95 |
| cause_wet_evap | wet_ground | evaporation | 0.90 |
| cause_evap_cloud | evaporation | cloud_form | 0.90 |
| cause_cloud_rain | cloud_form | rain | 0.95 |
| cause_fire_smoke | fire | smoke | 0.95 |

### 5.3 运行推理测试

> 当前仓库没有独立 `cli` 二进制（`symbio/src/bin/` 下只有 `seed_agents.rs`），下列 `cargo run --release --bin cli` 命令无效。

**当前可行的推理测试方式**：
- 通过 Tauri 桌面端（`tauri/`）或宿主直接调用 `route()` 进行对话/查询验证；
- 通过 `cargo test --lib` 运行 memory 域 5 个操作的内联单元测试；
- 推理能力（reason.* 等）属于路线图目标，尚未实现，无法在该 agent 上直接验证。

---

## 6. CLI 与端到端验证（现状说明）

> **重要更正**：当前仓库**没有独立的 `cli` 二进制**。`symbio/src/bin/` 下仅有 `seed_agents.rs`，因此所有 `cargo run --release --bin cli ...` 命令均无效（本节原 6.1–6.4 的 CLI 示例已废弃）。

### 6.1 当前可行的端到端 / 集成验证方式

- **桌面端**：通过 Tauri 桌面端（`tauri/`）进行完整的对话与能力调用验证。
- **宿主调用**：在宿主程序中直接调用 agent 插件的 `route()` 入口，驱动 memory 域 5 个操作（save/retrieve/graph_query/reflect/consolidate）及 chat 等能力。
- **单元测试**：`cargo test --lib` 运行随实现文件内联的 `#[cfg(test)]` 测试，覆盖存储层、认知层与 memory 域操作。
- **指定工作目录访问 agent**：通过宿主/桌面端的 `workdir` 配置访问项目工作目录下的 agent（如 `c:\Bing\agiwave\symbio\.symbio\agents\tester`），而非已不存在的 `--workdir` CLI 参数。

### 6.2 可用 Agent（数据路径参考）

| Agent ID | 说明 | 数据路径 |
|----------|------|----------|
| normal | 普通助手 | `~/.symbio/agents/normal/` |
| deep_thinker | 深度思考者 | `~/.symbio/agents/deep_thinker/` |
| code_expert | 代码专家 | `~/.symbio/agents/code_expert/` |
| reasoning_tester | 推理测试专家 | `~/.symbio/agents/reasoning_tester/` |
| **tester** | **测试用智能体** | **`<workdir>/.symbio/agents/tester/`** |

> 以上数据路径可用于宿主/桌面端配置，但**无法**通过 `cli` 命令行访问。

### 6.3 能力链验证要点（替代原 CLI 测试）

**验证重点**：确认完整的 LLM → 能力调用 → 能力执行 → 结果返回 → LLM 使用结果链路（需在宿主/桌面端中观察）。

| 验证项 | 成功标准 |
|--------|----------|
| **能力调用** | 日志中显示 `[Tool] Execution started: invoke_capability` |
| **能力执行** | 日志中显示 `[Tool] FINISHED: invoke_capability` 且有返回结果 |
| **结果正确性** | 返回结果符合预期功能（如查询返回正确数据，存储返回确认） |
| **LLM 使用结果** | LLM 回答基于能力返回结果，包含相关信息 |

> 原 6.3/6.4 的 `cargo run --release --bin cli ...` 测试示例、`./test_capabilities.ps1` / `./test_capability_chain.ps1` 脚本及 reason/learn/plan/metacognition 相关测试示例均已废弃——这些域操作尚未实现。

---

## 7. 测试报告模板

### 7.1 能力测试报告（v41 统一认知工具）

```markdown
## 能力测试报告

### 测试信息
- **测试日期**: 2026-06-19
- **测试 Agent**: tester
- **工作目录**: c:\Bing\agiwave\symbio

### 测试结果
| 操作 | 调用状态 | 执行状态 | 结果正确性 | LLM使用结果 | 总体评价 |
|------|---------|---------|-----------|------------|----------|
| agent_cognition (memory.save) | ✅ | ✅ | ✅ | ✅ | 通过 |
| agent_cognition (memory.retrieve) | ✅ | ✅ | ✅ | ✅ | 通过 |
| agent_cognition (memory.save + soft delete) | ✅ | ✅ | ✅ | ✅ | 通过 |
| agent_cognition (memory.graph_query) | ✅ | ✅ | ✅ | ✅ | 通过 |
| agent_cognition (memory.reflect) | ✅ | ✅ | ✅ | ✅ | 通过 |
| agent_cognition (memory.consolidate) | ✅ | ✅ | ✅ | ✅ | 通过 |

> **注意**：reason./learn./plan./metacognition. 各域操作（如 reason.search_evidence、learn.extract、plan.decompose、metacognition.introspect 等）**尚未实现**，不在此报告范围内。

### 关键发现
- ✅ memory 域 5 个操作通过 inventory 自注册机制正确注册
- ✅ LLM 只需理解 `operation` 字段，其他参数自由传入
- ✅ CognitionRequest 通过 serde flatten 透传，cognition 层不感知参数细节
- ✅ 参数缺失时 op 返回使用提示（含范例），LLM 可自动修正
```

---

## 8. 历史记录

> 以下为历史记录，部分早期操作数（26/27）已随架构收敛为当前 memory 域 5 个操作。

### 8.1 v43（2026-06-19）

- Token 优化：移除 build_schema() 中的冗余参数信息，节省约 1000-1500 tokens/请求
- 语义化：graph_query 子操作使用语义化名称（neighbors, path, subgraph, infer, by_type, bridges）
- 合并操作：删除 reason.graph_traverse，合并到 memory.graph_query，操作数量从 27 个减少到 26 个
- 文档同步：更新所有文档，确保与实现一致

### 8.2 v41（2026-06-19）

- 26 个操作通过 `submit_cognition_op!` 宏自注册，消除分组 `register_ops()` 函数
- `CognitionRequest` 简化为 `operation` + `params`（serde flatten），不感知具体参数
- Schema 简化为只有 `operation` 属性 + `additionalProperties: true`
- LLM 通过扁平 JSON 传参，缺失参数时 op 返回使用提示

### 8.3 v40（2026-06-18）

- 5 个旧 capability 文件删除，26 个操作拆分为独立 `.rs` 文件
- `plugin.rs` 的 `AGENT_CAPABILITY_IDS` 从 7 个精简为 3 个

### 8.3 v39（2026-06-18）

- 统一认知工具 `agent_cognition`，使用 `operation: "domain.action"` 两层命名

### 8.4 v31（2026-06-18）

- `agent_reason` 9→2 种推理操作
- `agent_learn` 12→8 种操作
- `agent_metacognition` 7→3 种类型

---

## 9. 深入阅读

- [**ARCHITECTURE.md**](./ARCHITECTURE.md)：系统架构
- [**PLAN.md**](./PLAN.md)：执行计划与进度追踪
