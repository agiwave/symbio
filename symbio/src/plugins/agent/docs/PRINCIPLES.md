# Symbio Agent：架构原则与质量标准

> **定位**：系统的设计哲学、质量标准和接口设计规范。所有架构决策的依据。
> 不包含：架构细节（见 ARCHITECTURE.md）、数据规范（见 COGNITION.md）、测试与操作流程（见 TESTING.md）。

---

## 1. 核心设计哲学

### 1.1 简单性
- **类型三层**: cu → prop/kind → 具体定义，保持最小化
- **无冗余属性**: 通过 `relations`（含 `is_a`）和 `prop_value_is_a` 表达所有关系
- **强约束**: 验证属性名和属性值合法性，不验证属性完整性

### 1.2 自洽性
- **语义一致**: `is_a` 表继承，`prop_value_is_a` 表属性值类型
- **自举闭环**: `prop.is_a = "cu"`, `kind.is_a = "cu"`，形成完整定义
- **关系机制化（v9）**: 所有关系（is_a / has / causes / depends...）通过
  `relations: HashMap<String, Vec<String>>` **统一在内存中表示**（仅作内存模型），
  **实际存储为顶层独立字段**（JSON 中是 `{"is_a":[...],"causes":[...]}`，**不**嵌套
  为 `{"relations":{...}}`）。`is_a` 与 `causes` / `depends` 等所有关系
  **结构层面完全等同**，无任何特殊化处理；关系判定**完全由 `prop` CU 派生**
  （`RelationPropRegistry::from_prop_cus`）——任何字段名只要对应一个
  `is_a:["relation"]` + `prop_value_is_a ∈ {cu, cu[]}` 的 prop CU 即为关系。
  新增关系只需声明 prop CU，核心代码无需改动
- **运行时可刷新的关系注册表（v10）**: `SharedRelationRegistry` 用
  `Arc<RwLock<RelationPropRegistry>>` 包装，支持：
  - `register(name)` 增量添加
  - `refresh_from_props(props)` 全量从 prop CU 重建
  - 读路径走 `read()` 锁（高并发不阻塞）
  - 写路径走 `write()` 锁（短临界区）
  调用方（`cu_from_json` / `FilterExpr::Relation`）持有 `SharedRelationRegistry`，
  即可在 prop 集合变化时"换芯"而无需重新构造。
- **展示机制化（v9.1）**: 类型清单、索引优先级也完全由 prop CU 派生。
  架构级智能体的"认知体系概览"不再 `match is_a` 硬编码：
  - 类型清单 = `is_a` 含 `kind` 的 prop CU 集合
  - 优先级 = prop CU 的 `priority` 属性（缺省 10；同 kind 内比较）
  - 关系清单 = `RelationPropRegistry`

  同样的 prop CU 数据同时驱动"如何解析 CU"和"如何展示 CU"——单一事实来源

### 1.3 分离原则
- **业务层**: Knowledge、Experience、Skill、Judgment 等认知维度
- **引擎层**: CognitiveUnit 的 Schema-free 存储、高维向量检索
- **能力层**: 供 LLM 调用的推理、元认知、查询等工具
- **协议层 (core/)**: 仅 trait 定义和纯数据类型，不含实现

---

## 2. 接口设计原则

### 2.1 基于能力而非机制

```rust
// ✅ 好的设计：基于能力（CognitiveUnit 统一接口）
trait AgentStore {
    async fn semantic_search(&self, query_text: &str, ...) -> Result<Vec<SearchResult>, StoreError>;
    async fn query(&self, filter: &FilterExpr, page: &PageRequest) -> Result<PageResult, StoreError>;
}
// ❌ 不好的设计：基于机制（JSON 碎片接口）
trait VectorStore {
    async fn search_by_embedding(&self, ...) -> Vec<SearchResult>;
}
```

### 2.2 接口最小化
- 每个 trait 不超过 8 个方法
- 默认方法实现批量操作（如 `insert_batch`）
- 不出现在任何接口定义中但实际需要再添加

### 2.3 扩展性通过变体而非接口
- 新能力通过新枚举变体添加，不改现有接口签名
- `core/` 中的接口变更需经过 [§5 检查清单](#5-core-接口变更检查清单)

### 2.4 命名规范
- 能力工具名以 `Tool` 结尾（`AgentQueryTool`, `AgentStoreTool`）
- 模块名使用全小写蛇形

---

## 3. 稳定性检查清单

在修改任何 `core/` 接口前，必须逐项确认：

| # | 检查项 | 通过标准 |
|---|--------|----------|
| 1 | **方法必要性** | 是否有现成接口可委托？若无，才考虑新增 |
| 2 | **能力抽象** | 参数是否绑定实现机制？必须基于能力而非机制 |
| 3 | **变体扩展优先** | 新能力是否可通过变体扩展？优先扩展变体而非新增方法 |

**决策规则**：3 项全部 ✓ → 可以修改 core/；任何一项 ✗ → **停止**，返回重新设计

---

## 4. 反模式警示

### 反模式 1：接口绑定实现机制

**特征**：接口参数包含具体实现技术的细节

```rust
// ❌ 反模式：绑定 OWL 实现机制
trait OWLReasoner {
    fn infer_types(&self, unit_id: &str) -> Vec<String>;
}

// ✅ 正确：抽象推理能力
trait Reasoner {
    fn capabilities(&self) -> Vec<String>;
    fn reason(&self, query: ReasonQuery) -> ReasonResult;
}
```

**危害**：实现变更 → 接口断裂 → 所有调用方需修改

### 反模式 2：过度设计接口

**特征**：接口方法过多，或包含不必要的批量操作

**危害**：接口臃肿 → 实现者负担重 → 接口难以稳定

### 反模式 3：贫血接口

**特征**：接口过于简单，调用方需要组合多个接口才能完成常见任务

**危害**：接口过于底层 → 调用方代码复杂 → 隐藏的模块间耦合

### 反模式 4：循环依赖

**特征**：模块 A 依赖模块 B，模块 B 也依赖模块 A

**危害**：无法独立测试和部署 → 编译时间增加 → 难以维护

### 反模式 5：能力模糊

**特征**：接口名称使用实现机制而非业务能力

```rust
// ❌ 反模式：名称暴露实现
trait VectorIndex { ... }
trait GraphStore { ... }

// ✅ 正确：名称描述能力
trait IAgentStore { ... }
```

---

## 5. 质量标准

### 5.1 简单性标准
- 每个模块职责单一，不超过 5 个公开方法
- 没有重复代码，相似逻辑抽象为共享函数
- 命名清晰，无歧义

### 5.2 稳定性标准
- 接口参数基于能力而非实现机制
- 新功能通过新实现添加，不修改现有接口

### 5.3 合理性标准
- 模块划分符合高内聚原则
- 依赖关系清晰，无循环依赖
- 错误处理一致，错误类型语义明确

### 5.4 架构复杂度度量

| 指标 | 当前值 | 警戒线 | 行动 |
|------|--------|--------|------|
| 模块数量 | 6 | ≤ 10 | 监控 |
| 接口数量 | 3 | ≤ 8 | 监控 |
| 平均接口方法数 | 5 | ≤ 6 | 监控 |
| 循环依赖数 | 0 | = 0 | ✅ 保持 |

---

## 6. 修改 core/ 的流程

1. **提出修改**：在 issue 中描述需要解决的问题
2. **设计评审**：检查是否符合稳定性原则（§3 检查清单）
3. **实施**：修改 core/ 和受影响模块
4. **验证**：确保所有模块编译通过
5. **文档更新**：同步更新相关文档

---

## 7. 术语表

| 术语 | 定义 |
|------|------|
| **CognitiveUnit** | 认知单元，系统的基本数据单元，本质是 JSON 对象 |
| **IAgentStore** | 存储协议，定义认知单元的 CRUD + 认知层操作 |
| **MindscapeScaffold** | 认知能力层，实现 AgentStore trait，提供验证/反馈/去重 |
| **EmbeddingStore** | 嵌入存储，包装底层存储并自动处理向量化 |

---

## 8. 深入阅读

- [**ARCHITECTURE.md**](./ARCHITECTURE.md)：系统架构与模块设计
- [**COGNITION.md**](./COGNITION.md)：认知单元数据规范
- [**TESTING.md**](./TESTING.md)：测试体系（含 op 操作手册）
- [**PLAN.md**](./PLAN.md)：执行计划与进度追踪