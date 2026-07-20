# Symbio Agent：认知单元数据规范

> **定位**：定义认知单元（CU）的属性、值类型、关系、类型层级、ID 规范、验证规则。
> 不包含：架构细节（见 ARCHITECTURE.md）、接口规范（见 PRINCIPLES.md）、测试与操作流程（见 TESTING.md）。

---

## 1. 认知单元定义

### 1.1 什么是认知单元

**认知单元 (CognitiveUnit) 是 `serde_json::Map<String, Value>` 的强类型薄包装**。

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CognitiveUnit {
    #[serde(flatten)]
    data: Map<String, Value>,
}
```

> 早期（v7 及之前）`CognitiveUnit = serde_json::Value`，存在 200+ 处调用点
> 失去编译期类型保护、字段拼写错误只能运行时发现等问题。
> v8 起统一为 `CognitiveUnit` struct：字段拼写错误编译期拒绝，
> id/name/description 强类型，扩展属性通过方法访问（`unit.name()` / `unit.set_name(...)`）。
> v9.4 进一步统一为 `Map<String, Value>` 薄包装，所有字段平等存储在 `data` 中。

### 1.2 认知单元结构

```
┌─────────────────────────────────────────────────────────────┐
│                      认知单元 (JSON Object)                  │
├─────────────────────────────────────────────────────────────┤
│  属性名1: 属性值1                                            │
│  属性名2: 属性值2                                            │
│  属性名3: 属性值3                                            │
└─────────────────────────────────────────────────────────────┘
```

> **JSON 序列化形态**：每个关系是**顶层独立字段**
> （`{"is_a":[...],"causes":[...]}`），不嵌套为 `{"relations":{...}}`。
> `relations: HashMap` 仅是内存表示。

**最小示例**：

```json
{"id": "a1b2c3d4", "is_a": ["fact"], "description": "地球是圆的", "confidence": 0.95}
```

---

## 2. 属性名规范

### 2.1 属性名格式

属性名直接采用 **prop CU 的 `id`**（如 `id`、`is_a`、`name`、`description`、`causes`）：

- **id**：必需，prop CU 的唯一标识符（即 JSON 键名）
- **来源**：运行时由 `RelationPropRegistry`（或更广泛的 `PropRegistry`）决定某个键名是否合法
- **无 `name::` 前缀**：JSON 键名不带命名空间

> **演进说明**：早期设计稿曾考虑过 `[{prop_cu_name}::]{prop_cu_id}` 双段命名，
> 允许同一 prop CU 拥有多个等价键名（如 `is_a` / `is_a::type`）。
> 考虑到 LLM 写 JSON 时复杂度、双重命名空间带来的解析开销，
> 以及 prop CU 自身已经支持 `name` 字段作为展示名（不参与存储键），
> 决定**直接采用 prop CU 的 `id` 作为 JSON 键名**。
> 若运行时需要"prop 多别名"，可通过 prop CU 自身的 `name` 字段做展示层映射。

### 2.2 简写规则

JSON 键名直接就是 prop CU 的 `id`，没有完整/简写之分。prop CU 自身的 `name`
是可选的展示名（人类可读），与 JSON 键名解耦：

```json
// 键名 = prop CU id（运行时注册）
{"id": "abc", "is_a": ["fact"], "name": "地球形状", "description": "地球是圆的"}

// prop CU 自身的"展示名"（不参与 JSON 键名）
{"id": "is_a", "name": "类型继承", "is_a": ["relation"], "prop_value_is_a": "cu[]", "level": "core"}
```

### 2.3 标准属性列表

| 属性名（prop id） | 类型约束 | 必需 | 说明 |
|-------------------|----------|------|------|
| `id` | string | 是 | 唯一标识符 |
| `is_a` | cu[] | 是 | 类型关系，支持多重继承 |
| `name` | string | 否 | 名称 |
| `description` | string | 否 | 描述文本 |
| `content` | string | 否 | 内容文本 |
| `confidence` | number | 否 | 置信度 0.0-1.0（**`0` 触发软删除**） |
| `meta_belief` | number | 否 | 元认知信念度 |
| `priority` | number | 否 | 系统提示词候选池优先级（`≤20` 入候选池；`>20` 排除） |

**内部字段**（以 `_ext_` 开头，**永不向 LLM 暴露**）：
- `_ext_version`：版本号
- `_ext_embedding`：向量嵌入
- `_ext_created_at` / `_ext_updated_at` / `_ext_last_access`：时间戳
- `_ext_memory_strength` / `_ext_access_count`：记忆相关

### 2.4 核心关系列表（v9 起由 prop 驱动）

| 关系名 | 类型约束 | 说明 |
|--------|----------|------|
| `is_a` | cu[] | 类型继承：定义认知单元的类型分类 |
| `has` | cu[] | 包含关系：拥有或包含的关联 |
| `part_of` | cu[] | 组成关系：整体与部分的关联 |
| `causes` | cu[] | 因果关系：原因与结果的关联 |
| `depends` | cu[] | 依赖关系：对其他单元的依赖 |
| `similar` | cu[] | 相似关系：概念间的相似性 |
| `opposite` | cu[] | 对立关系：概念间的对立或矛盾 |
| `related` | cu[] | 泛化关联：不确定关系 |

> 上表是**系统预声明的常用关系**（`RelationPropRegistry::default_relation_prop_registry()`
> 提供 8 种关系作为种子）。**运行时可通过声明新的 `is_a: ["relation"]` CU 来注册新关系**
> （如 `influences` / `cures` / `mentions` ...），
> 核心代码无需任何改动。

### 2.5 属性名示例

```json
// 简写格式（单类型）
{"id": "a1b2c3d4", "is_a": ["fact"], "name": "地球形状", "description": "地球是圆的"}

// 简写格式（多类型，支持多重继承）
{"id": "zhangsan", "is_a": ["person", "employee", "football_player"], "name": "张三"}

// 多种关系属性并存
{"id": "apple", "is_a": ["fruit"], "part_of": ["plant"], "related": ["food", "healthy"]}

// 自定义关系（运行时注册）
{"id": "aspirin", "is_a": ["fact"], "cures": ["headache"]}
```

---

## 3. 属性值规范

### 3.1 属性值类型

认知单元的属性值分为五种类型：

| 类型 | 说明 | 约束 |
|------|------|------|
| `string` | 普通字符串 | 值必须是字符串 |
| `number` | 数字 | 值必须是数字 |
| `boolean` | 布尔 | 值必须是 true/false |
| `cu` | 引用单个 CU | 值必须是已存在的 CU ID |
| `cu[]` | 引用多个 CU | 值必须是 CU ID 数组 |

### 3.2 string 类型

普通文本字符串，可以是任意文字。

```json
{"name": "地球是圆的"}
{"description": "这是一个客观事实"}
{"content": "地球是一个近似球形的椭球体"}
```

### 3.3 number 类型

数值类型，支持浮点数和整数。

```json
{"confidence": 0.95}
{"priority": 42}
{"score": 3.14159}
```

### 3.4 cu 类型（引用单个认知单元）

值是另一个认知单元的 ID，用于建立认知单元之间的关联关系。

```
值格式: [{cu_name}::]{cu_id}
```

- **{cu_id}**：必需，认知单元的唯一标识符
- **{cu_name}**：可选，用于提高可读性的名称前缀
- **::**：分隔符（当提供 cu_name 时才使用）

**示例**：

```json
{"is_a": "fact"}
{"parent": "task::t12345"}
```

### 3.5 cu[] 类型（引用多个认知单元）

值是多个认知单元 ID 组成的数组。

```
数组元素格式: [{cu_name}::]{cu_id}
```

**示例**：

```json
{"is_a": ["fact", "knowledge"]}
{"related": ["science", "astronomy"]}
{"depends": ["task::t1", "task::t2"]}
```

### 3.6 引用解析逻辑

系统通过 `parse_cu_ref` 函数解析引用，返回带可选别名的 `CuRef` 结构：

```rust
#[derive(Debug, Clone)]
pub struct CuRef<'a> {
    pub id: &'a str,
    pub name: Option<&'a str>,
}

pub fn parse_cu_ref(ref_str: &str) -> CuRef<'_> {
    if let Some(colon_pos) = ref_str.find("::") {
        if colon_pos > 0 && colon_pos + 2 < ref_str.len() {
            let name = &ref_str[..colon_pos];
            let id = &ref_str[colon_pos + 2..];
            if !id.contains('/') && !id.contains('\\') {
                return CuRef { id, name: Some(name) };
            }
        }
    }
    CuRef { id: ref_str, name: None }
}
```

- **有 `::`**：拆分 `(name, id)`，name 仅为展示
- **无 `::`**：原串即为 id，name 为 None
- **包含路径分隔符**（`/` 或 `\`）：视为非法引用，原样返回（`name=None`）

---

## 4. 属性定义认知单元 (prop)

### 4.1 什么是属性定义认知单元

**属性定义认知单元**（简称 prop）是一种特殊类型的认知单元，用于定义认知单元中可使用的属性名称及其值类型约束。

每个属性名（如 `id`、`is_a`、`name`）在系统中都对应一个属性定义认知单元。

### 4.2 prop 的特征

属性定义认知单元与普通认知单元的最大区别在于，它**必须**包含 `prop_value_is_a` 属性：

```json
{"id": "id", "name": "id", "description": "唯一标识符",
 "is_a": ["prop"], "prop_value_is_a": "string", "level": "core"}
```

### 4.3 prop_value_is_a

`prop_value_is_a` 是属性定义认知单元的核心属性，用于定义该属性对应的属性值类型约束。

| 值 | 说明 |
|----|------|
| `string` | 属性值必须是字符串类型 |
| `number` | 属性值必须是数字类型 |
| `boolean` | 属性值必须是布尔类型 |
| `cu` | 属性值必须是单个认知单元引用 |
| `cu[]` | 属性值必须是认知单元引用数组 |

当一个认知单元作为属性被使用时，其属性值必须符合该属性定义认知单元中
`prop_value_is_a` 指定的类型。

### 4.4 系统内置属性定义认知单元

```json
// 属性定义根类型
{"id": "prop", "name": "prop", "description": "属性定义类型", "is_a": ["cu"], "level": "core"}
{"id": "relation", "name": "relation", "description": "关系定义", "is_a": ["prop"], "level": "core"}

// 基础属性定义
{"id": "id",          "name": "id",          "prop_value_is_a": "string", "is_a": ["prop"],     "level": "core"}
{"id": "name",        "name": "name",        "prop_value_is_a": "string", "is_a": ["prop"],     "level": "core"}
{"id": "description", "name": "description", "prop_value_is_a": "string", "is_a": ["prop"],     "level": "core"}
{"id": "content",     "name": "content",     "prop_value_is_a": "string", "is_a": ["prop"],     "level": "core"}
{"id": "level",       "name": "level",       "prop_value_is_a": "string", "is_a": ["prop"],     "level": "core"}
{"id": "confidence",  "name": "confidence",  "prop_value_is_a": "number", "is_a": ["prop"],     "level": "core"}

// 核心关系定义
{"id": "is_a",     "name": "is_a",     "prop_value_is_a": "cu[]", "is_a": ["relation"], "level": "core"}
{"id": "has",      "name": "has",      "prop_value_is_a": "cu[]", "is_a": ["relation"], "level": "core"}
{"id": "part_of",  "name": "part_of",  "prop_value_is_a": "cu[]", "is_a": ["relation"], "level": "core"}
{"id": "causes",   "name": "causes",   "prop_value_is_a": "cu[]", "is_a": ["relation"], "level": "core"}
{"id": "depends",  "name": "depends",  "prop_value_is_a": "cu[]", "is_a": ["relation"], "level": "core"}
{"id": "similar",  "name": "similar",  "prop_value_is_a": "cu[]", "is_a": ["relation"], "level": "core"}
{"id": "opposite", "name": "opposite", "prop_value_is_a": "cu[]", "is_a": ["relation"], "level": "core"}
{"id": "related",  "name": "related",  "prop_value_is_a": "cu[]", "is_a": ["relation"], "level": "core"}
```

### 4.5 关系由 prop 驱动（v9 关系机制化）

关系**判定完全由 prop CU 派生**，不靠硬编码关系名。规则：

> **一个属性名是关系 ⟺ 存在 prop CU `p` 满足**
> 1. `p.is_a` 含 `relation`（或 `relation` 子类型）
> 2. `p.prop_value_is_a` ∈ {`cu`, `cu[]`}
> 3. `p.id` == 该属性名

系统在启动时扫描 `prop` CU 集合构建 `RelationPropRegistry`。
运行时新增关系（如 `cures` / `mentions` / `influences`）只需声明一个 prop CU，
核心代码无需任何改动。

**示例：注册新关系 `cures`**

```json
// 仅需声明一个 prop CU，不需要改任何代码
{"id": "cures", "name": "cures", "description": "治愈关系",
 "is_a": ["relation"], "prop_value_is_a": "cu[]", "level": "core"}

// 之后所有 CU 即可使用
{"id": "aspirin", "is_a": ["fact"], "cures": ["headache"]}
```

`is_a` 仅为最常用的关系，提供 `is_a()` / `is_type()` / `add_type()` 等便捷访问器。

### 4.6 属性定义与属性使用的对应关系

| 属性定义单元 | prop_value_is_a | 属性使用示例 |
|-------------|-----------------|-------------|
| `id::id` | string | `"id": "a1b2c3d4"` |
| `is_a::is_a` | cu[] | `"is_a": ["fact"]` |
| `name::name` | string | `"name": "地球形状"` |
| `description::description` | string | `"description": "地球是圆的"` |
| `confidence::confidence` | number | `"confidence": 0.95` |
| `related::related` | cu[] | `"related": ["science", "astronomy"]` |

---

## 5. 认知类型层级

### 5.1 类型继承树

```
cu (CognitiveUnit 根类型)
├── prop —— 属性定义（如 id、name、description）
│   ├── relation —— 关系类型（如 is_a、causes、depends）
│   └── meta —— 元认知属性命名空间（meta_belief / meta_reflection / ...）
└── kind —— 认知类型根
    ├── fact — 事实知识
    ├── experience — 经验经历
    ├── skill — 技能能力
    ├── judgment — 判断标准
    ├── strategy — 策略方法
    ├── rule — 系统规则（强制执行的行为约束）
    ├── intuition — 直觉能力
    ├── emotion — 情感偏好
    └── tag — 标签
```

> **注：`identity` 不是一种类型**。
> 它是每个 agent 必备的、id 固定为 `"identity"` 的**一条**特殊认知单元
> （典型 `is_a: ["fact"]`），承担"自我定位"职责。
> 识别身份必须用 `id == "identity"`，不能用 `is_a == "identity"`。

### 5.2 标准认知类型

| 类型 ID | 说明 | 提示词行为 |
|---------|------|-----------|
| `fact` | 事实知识 | 摘要索引 |
| `experience` | 经验经历 | 摘要索引 |
| `skill` | 技能能力 | 摘要索引 |
| `judgment` | 判断标准 | 摘要索引 |
| `strategy` | 策略方法 | 摘要索引 |
| `rule` | 系统规则 | **全量注入**（不可打折） |
| `intuition` | 直觉能力 | 摘要索引 |
| `emotion` | 情感偏好 | 摘要索引（**v18 状态**：仅注册 kind 标识，引擎**未实现**情感读写/调整；LLM 端 prompt 集成是未来工作） |
| `tag` | 标签分类 | 不进入提示词 |

> **提示词注入身份说明**：每 agent 必备的、id 固定为 `"identity"` 的 CU
> （典型 `is_a: ["fact"]`）会被**全量注入**到提示词中作为"自我认知基石"。
> `identity` 是**单条 CU 的固定 id**，不是一种类型。

> **展示与优先级机制化（v9.1）**：类型清单、索引优先级**完全由 prop CU 派生**，
> 不再 `match is_a` 硬编码：
> - 类型清单 = `is_a` 含 `kind` 的 prop CU 集合
> - 优先级 = prop CU 的 `priority` 属性（缺省 100 = 不进入系统提示词；同 kind 内比较）
>
> 同样的 prop CU 数据同时驱动"如何解析 CU"和"如何展示 CU"——单一事实来源。

---

## 6. 层级系统

| 层级 | 说明 | 提示词行为 | 操作限制 |
|------|------|-----------|----------|
| `core` | 内核级：系统核心定义（cu、prop、kind 等） | 永不进入提示词 | 不可修改、不可删除 |
| `sys` | 系统级：持久化认知 | 进入系统提示词或摘要索引 | 可修改 |
| `msg` | 消息级：临时上下文 | 按需注入激活记忆 | 临时 |

---

## 7. 元认知机制

### 7.1 定位澄清：`meta` 是 prop 命名空间，不是业务 kind

> **重要（v10+ 修订）**：`meta` 当前的 `is_a` 是 `["prop"]`，**不是** `["kind"]`。
>
> 历史上 `meta` 曾被错误地挂在 `kind` 树下，导致以下三类语义不一致：
> 1. `meta_*` 这 5 个 prop 自身 `is_a: ["meta"]`——意味着 prop 同时是 kind 的实例，违反"prop / kind 互不交叉"的设计原则
> 2. `meta` kind 在运行时**从未被实例化**——grep 整个代码库，没有任何 `is_type("meta")` 或 `FilterExpr::is_a("meta")` 的业务调用
> 3. `meta_belief` / `meta_reflection` 等字段实际是**横切属性**（cross-cutting concerns）——附加到任何类型的 CU 上，而非仅限 `meta` kind
>
> 修订方案：把 `meta` 与 `relation` 对齐，作为 `prop` 的子类型：
>
> ```json
> {"id":"meta","name":"meta","description":"元认知属性命名空间","is_a":["prop"],"level":"core"}
> ```
>
> 修订后链路自洽：`meta_belief` → `meta` → `prop` → `cu` 一路到底，与 `is_a` → `relation` → `prop` → `cu` 平行。

### 7.2 元认知属性 prop

`meta` 命名空间下注册 5 个横切属性 prop，可附加到任何类型的 CU 上：

| 属性名 | 类型 | 说明 |
|--------|------|------|
| `meta_belief` | number | 信念度：对自身知识的信任程度 |
| `meta_conflict` | cu[] | 冲突检测：识别知识矛盾 |
| `meta_learning` | string | 学习策略：获取新知识的方法 |
| `meta_reflection` | string | 反思记录：对思考过程的监控 |
| `meta_adaptation` | cu[] | 适应调整：根据反馈修正认知 |

**作用范围**：`meta_*` 字段是**横切属性**，允许附加在 `fact` / `skill` / `rule` / `strategy` 等任何业务 kind 的 CU 上，无类型限制。

**运行时机制**（已实现）：
- `meta_belief`：`CognitiveFeedback::on_units_retrieved` 在 CU 被检索时自动累加，攒批 flush（见 [cognitive_feedback.rs](file:///c:/Bing/agiwave/symbio/symbio/src/plugins/agent/store/mindscape/cognitive_feedback.rs)）

**未实现**（v26+ 待办）：
- `meta_conflict`：自动冲突检测写入路径在 v26-N2 重构时移除，目前是"死字段"，需后续能力补全
- `meta_reflection` / `meta_learning` / `meta_adaptation`：仅作为 LLM 主动写入的语义通道，引擎不主动触发

### 7.3 使用模式

**正确用法**：元认知字段附加到业务类型的 CU 上：

```json
// 一条 fact 携带信念度与反思
{"id": "cu_abc12345", "is_a": ["fact"],
 "description": "地球是圆的",
 "confidence": 0.95,
 "meta_belief": 0.85,
 "meta_reflection": "经过多方资料交叉验证"}

// 一条 rule 携带冲突标记
{"id": "rule_no_eval", "is_a": ["rule"],
 "description": "禁止使用 eval 执行外部输入",
 "level": "sys",
 "meta_conflict": ["rule_dynamic_exec"]}
```

**错误用法**（**禁止**）：不要再写 `is_a: ["meta"]` 的 CU——`meta` 是 prop 子类，不是 kind 子类，没有"元认知 CU"这种业务类型。

### 7.4 与 `relation` 的对称性

| 维度 | `relation` | `meta` |
|------|------------|--------|
| `is_a` | `["prop"]` | `["prop"]` |
| 角色 | "该 prop 是关系（值约束 cu/cu[]）" | "该 prop 是元认知字段" |
| 识别方法 | `is_relation_prop()`：`is_a` 含 `relation` + `prop_value_is_a ∈ {cu, cu[]}` | `is_meta_prop()`：`is_a` 含 `meta` |
| 索引能力 | `FilterExpr::is_a("relation")` 查所有关系 prop | `FilterExpr::is_a("meta")` 查所有元认知 prop |
| 启用查询 | 已在 `core::query_relation_names` 实现 | 待实现（`query_meta_props` 备用） |

---

## 8. 典型示例

```json
// 系统内置核心 CU
{"id":"cu","name":"cu","description":"认知单元根类型","level":"core"}
{"id":"prop","name":"prop","description":"属性定义类型","is_a":["cu"]}
{"id":"relation","name":"relation","description":"关系定义","is_a":["prop"]}
{"id":"kind","name":"kind","description":"认知类型根","is_a":["cu"]}
{"id":"meta","name":"meta","description":"元认知属性命名空间","is_a":["prop"]}

// 业务认知单元
{"id":"a1b2c3d4","is_a":["fact"],"description":"地球是圆的","confidence":0.95}
{"id":"b2c3d4e5","is_a":["rule"],"description":"代码必须遵循安全协议","priority":200}

// 多重继承示例
{"id":"zhangsan","is_a":["person","employee","football_player"],"name":"张三"}

// 关系使用示例
{"id":"apple","is_a":["fruit"],"part_of":["plant"],"related":["food"]}

// 元认知字段使用示例（附加在业务 CU 上，非独立类型）
{"id":"cu_fact_001","is_a":["fact"],"description":"地球是圆的","meta_belief":0.9,"meta_reflection":"已完成事实验证"}
```

---

## 9. 核心设计原则

### 9.1 简单性原则
- **四层结构**：cu → prop → rel → 具体定义
- **无冗余属性**：通过 `is_a` 和 `prop_value_is_a` 表达所有关系
- **语义清晰**：每个属性和类型都有明确的语义定义

### 9.2 自洽性原则
- **`is_a`**：表示继承关系，支持多重继承
- **`prop_value_is_a`**：表示属性值的类型约束
- **自举闭环**：`prop.is_a = ["cu"]`，`kind.is_a = ["cu"]`，形成完整的定义闭环

### 9.3 强约束原则
- **属性名验证**：所有属性名必须是已定义的 `prop`
- **属性值验证**：属性值必须符合对应的 `prop_value_is_a` 约束
- **继承链验证**：`is_a` 必须引用存在的认知单元，且无循环

### 9.4 机制优先原则
- **知识表示机制**：通过认知单元表达知识
- **推理机制**：基于关系的图遍历与逻辑推导
- **元认知机制**：自我监控、反思与调整
- **关系机制化（v9）**：关系判定完全由 prop 派生，运行时可扩展
- **展示机制化（v9.1）**：类型清单 / 显示名 / 优先级完全由 prop 派生，运行时可扩展

---

## 10. 四层认知体系

### 10.1 结构图示

```
┌─────────────────────────────────────────────────────────────┐
│ Layer 1: 根类型 (cu)                                         │
│ {"id":"cu","name":"cu","level":"core"}                       │
└─────────────────────────────────────────────────────────────┘
                                │
              ┌─────────────────┴─────────────────┐
              ▼                                   ▼
┌─────────────────────────────────────┐ ┌───────────────────────────────────┐
│ Layer 2: 元定义类型                  │ │ Layer 2: 元定义类型                │
│ • prop（属性定义）                   │ │ • kind（认知类型根）                │
│   - id, name, description           │ │   - fact, skill, experience...     │
└─────────────────────────────────────┘ └───────────────────────────────────┘
              │                                   │
              ▼                                   │
┌─────────────────────────────────────┐           │
│ Layer 3: 关系定义 (relation)         │           │
│  + 元认知属性 (meta)                 │           │
│ • relation: is_a, has, part_of...   │           │
│ • meta: meta_belief, meta_reflection│           │
│   meta_conflict, meta_learning...   │           │
└─────────────────────────────────────┘           │
                                                  │
                                                  ▼
                              ┌───────────────────────────────────┐
                              │ Layer 4: 业务认知单元（携带元认知字段）│
                              │ • {"is_a":["fact"],"description":...}│
                              │ • {"is_a":["fact"],"meta_belief":...}│
                              └───────────────────────────────────┘
```

### 10.2 自举闭环

- `cu` 是根（`level=core`）
- `prop` 是属性定义类型（`is_a=["cu"]`）
- `relation` 是关系定义类型（`is_a=["prop"]`）
- `meta` 是元认知属性命名空间（`is_a=["prop"]`，与 `relation` 平行）
- `kind` 是认知类型根（`is_a=["cu"]`）
- 所有具体属性（`id`/`is_a`/`name`...）和关系（`is_a`/`has`/`causes`...）都是 prop CU
- **核心代码不维护任何硬编码的属性 / 关系 / 类型名清单**——全部由 prop 集合派生

---

## 11. 验证规则

### 11.1 属性名合法性验证

```
规则：所有属性名必须是已定义的 prop
✓ {"id": "abc", "name": "测试", "is_a": ["fact"]}
✗ {"id": "abc", "unknown_prop": "value"}
```

### 11.2 属性值类型验证

```
规则：属性值必须符合其 prop_value_is_a 约束
✓ {"is_a": ["fact"]}
✓ {"is_a": ["person", "employee"]}
✓ {"confidence": 0.8}
✗ {"confidence": "high"}
```

### 11.3 关系引用验证

```
规则：关系属性的值必须引用已存在的认知单元
✓ {"is_a": ["fact"]}            ← "fact" 是已存在的类型
✗ {"is_a": ["unknown_type"]}    ← "unknown_type" 不存在
```

### 11.4 继承链循环验证

```
规则：cu[] 关系字段（is_a / causes / depends / similar / opposite / related / ...）
     引用必须存在且无循环
✗ a 的 is_a 含 b，且 b 的 is_a 含 a  ← 循环，禁止
```

> 循环检测机制化（v9）：检测 `prop_value_is_a` 为 `cu` / `cu[]` 的**所有关系**
> （不再仅限 `is_a`）。

### 11.5 引用格式验证

```
规则：如果属性为关联类属性，则属性值必须是 [{cu_name}::]{cu_id} 格式
✓ {"related": ["fact::f123", "skill::s456"]}
✓ {"related": ["science", "astronomy"]}
```

### 11.6 schema 元数据保护

> 已废弃旧的 `level = core` 概念。当前保护机制是：
> `is_a` 包含 `kind` / `prop` / `meta` / `relation` / `cu` 的 CU（即 seed_cus.jsonl 定义的 schema 元数据）
> 不可被 `memory.save` / `memory.consolidate` / `belief_buffer` 修改（系统启动时从 seed_cus.jsonl 重新加载以恢复）。

---

## 12. ID 规范

### 12.1 ID 生成规则

| 类型 | 格式 | 示例 |
|------|------|------|
| 系统内置 | 语义化单词 | `cu`, `prop`, `relation`, `kind`, `fact` |
| 用户创建 | 8 位短 GUID | `a1b2c3d4`, `f3e4d5c6` |
| 带类型前缀 | `{type}::{id}` | `skill::code_review`, `fact::abc123` |

### 12.2 ID 生成实现

```rust
pub fn generate_short_id() -> String {
    uuid::Uuid::new_v4().as_simple().to_string()[..8].to_string()
}
```

---

## 13. 高阶 AI 能力体系

> 系统向 LLM 暴露的六大核心能力模块。下表中"已实现"标记 ✅ 的能力
> 可在当前版本调用；其他为路线图目标，尚未实现。

### 13.1 能力体系架构

```
                invoke_capability
                       │
        ┌──────────────┼──────────────┐
        ▼              ▼              ▼
   知识管理层       关系推理层       学习归纳层
   • query         • path          • induce
   • store         • conflict      • analogy
   • delete                         • general
        │              │              │
        └──────────────┼──────────────┘
                       ▼
        ┌──────────────┼──────────────┐
        ▼              ▼              ▼
   决策规划层       元认知层        记忆管理层
   • plan          • reflect       • recall
   • eval          • belief        • consolidate
   • predict       • detect        • forget
```

### 13.2 能力清单

| 层 | 能力 | 状态 | 说明 |
|----|------|------|------|
| 知识管理 | `query_knowledge` | ✅ | 语义检索知识 |
| 知识管理 | `store_knowledge` | ✅ | 存储/更新知识（**软删除：`{id, confidence:0}` 立即删除**） |
| 关系推理 | `find_relation_path` | ✅ | 关系路径查找（支持跨关系类型） |
| 关系推理 | `detect_conflicts` | ✅ | 知识冲突检测 |
| 学习归纳 | `induce_pattern` | ⏳ | 从案例中归纳模式 |
| 学习归纳 | `analogy_reason` | ⏳ | 类比推理 |
| 学习归纳 | `generalize_rule` | ⏳ | 从具体到一般 |
| 决策规划 | `plan_action` | ⏳ | 行动计划生成 |
| 决策规划 | `evaluate_options` | ⏳ | 选项评估 |
| 决策规划 | `predict_outcome` | ⏳ | 结果预测 |
| 元认知 | `reflect_reasoning` | ✅ | 推理过程反思 |
| 元认知 | `evaluate_belief` | ✅ | 信念度评估 |
| 元认知 | `detect_bias` | ⏳ | 认知偏差检测 |
| 记忆管理 | `recall_memory` | ⏳ | 记忆回忆 |
| 记忆管理 | `consolidate_memory` | ⏳ | 记忆巩固 |
| 记忆管理 | `forget_memory` | ⏳ | 记忆遗忘 |

**关系路径格式**（紧凑格式，Token 效率高）：

```
apple -- {is_a} -- fruit -- {part_of} -- plant
```

**关系路径格式**（详细格式）：

```json
{
  "steps": [
    {"from": "apple", "relation": "is_a", "to": "fruit"},
    {"from": "fruit", "relation": "part_of", "to": "plant"}
  ],
  "description": "apple -- {is_a} -- fruit -- {part_of} -- plant"
}
```

### 13.3 高阶 AI 实现标志

当系统具备以下能力时，可视为实现了高阶人工智能：

| 能力维度 | 具体表现 | 对应能力 |
|---------|---------|---------|
| **推理能力** | 能进行传递性推理、类比推理 | `find_relation_path`, `analogy_reason` |
| **学习能力** | 能从经验中归纳模式和规则 | `induce_pattern`, `generalize_rule` |
| **决策能力** | 能制定计划并评估选项 | `plan_action`, `evaluate_options` |
| **元认知能力** | 能反思自身推理过程 | `reflect_reasoning`, `evaluate_belief` |
| **记忆能力** | 能管理长期记忆并适时回忆 | `recall_memory`, `consolidate_memory` |

---

## 14. 深入阅读

- [**PRINCIPLES.md**](./PRINCIPLES.md)：架构原则与质量标准
- [**ARCHITECTURE.md**](./ARCHITECTURE.md)：系统架构与模块设计
- [**TESTING.md**](./TESTING.md)：测试体系（含 op 标准操作流程）
