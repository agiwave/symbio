# Agent 智能体插件

> **核心职责**：将 AI 的"交互场域 (Session)"与"认知主体 (Agent)"解耦。作为系统唯一的持久化认知核心，负责管理 7D 认知结构与物理记忆。

---

## 1. 核心概念：7D 认知模型

每个 Agent 智能体都拥有其独特的人格特征，维护了一个长期的、稳定的七维（7D）心智模型：

| 维度 | 名称 | 说明 |
|------|------|------|
| Knowledge | 事实 | 客观存在的事实性认知 |
| Experience | 经验 | 历史积累的经验 |
| Skill | 技能 | 可调用的专业能力 |
| Judgment | 判断 | 做出判断的标准 |
| Strategy | 策略 | 解决问题的思维策略 |
| Intuition | 直觉 | 基于经验的快速判断能力 |
| Emotion | 情感 | 影响认知的情绪状态和偏好（**v18 状态**：仅作为 `kind` 标识注册到 seed_cus，引擎**未实现**情感状态的读写/检索/调整逻辑；LLM 端的 prompt 集成是未来工作） |

> **底层支撑**：上述 7D 模型是业务层面的"心理学范式"。底层数据的物理存储、向量检索和动态演化统一由内置的 **Mindscape 引擎** 负责。

---

## 2. 存储结构

### 2.1 支持的存储格式

| 格式 | 类型 | 说明 | 推荐场景 |
|------|------|------|----------|
| **目录模式** | 目录 | 多个 `.yaml` 文件，每文件一个 CognitiveUnit | 复杂智能体，多人协作 |
| **单一文件模式** | 文件 | 单个 `.yaml`/`.json`/`.jsonl` 文件 | 简单智能体，快速原型 |

### 2.2 目录结构

```
~/.symbio/
├── agents/
│   ├── {agent_id}/                    # 目录模式
│   │   ├── identity.yaml              # 核心身份定义
│   │   └── *.yaml                     # 其他认知单元
│   └── {agent_id}.yaml                # 文件模式（YAML）
│   └── {agent_id}.jsonl                # 文件模式（JSONL）
└── config/
    └── global_memory.md               # 全局跨人格共享规范

{workdir}/.symbio/
└── agents/                            # 项目特定Agent（优先于全局）
```

### 2.3 文件格式

**目录模式 - identity.yaml**：
```yaml
id: identity
is_a: fact
level: sys
name: 普通助手
description: 一个平衡、直接且专业的通用AI助手
```

**文件模式 - JSONL**：
```json
{"id":"identity","is_a":"fact","level":"sys","name":"全栈架构师","description":"我是 Architect & Coder"}
{"id":"c5d6e7f8","is_a":"rule","level":"sys","description":"【代码标准】在提供代码时，必须优先考虑健壮性。"}
```

### 2.4 字段说明

| 字段 | 说明 |
|------|------|
| `id` | 唯一标识符（身份单元固定为 `"identity"`） |
| `is_a` | 类型关系，表示此认知单元属于哪种类型 |
| `level` | 作用域，`core`=元认知级，`sys`=系统级（始终加载），`msg`=消息级 |
| `name` | 显示名称 |
| `description` | 详细描述 |

---

## 3. 核心 API

### 3.1 基础管理（Handler 路由）

| 路由 | 说明 |
|------|------|
| `agent/list` | 获取所有可用的人格配置列表 |
| `agent/get` | 获取指定 Agent 的基础配置 |
| `agent/create` | 创建智能体（含种子认知数据） |
| `agent/delete` | 删除智能体（物理目录 + 缓存清理，幂等） |
| `agent/chat` | 核心对话入口（含上下文构建、系统提示词生成） |
| `agent/config/get` | 获取存储与引擎相关配置 |
| `agent/config/set` | 修改配置（存储后端、格式等） |

### 3.2 认知引擎（LLM 工具调用）

认知操作通过 `agent_cognition` 统一工具调用，使用 `operation: "域.操作"` 两层命名：

| 工具 | 说明 |
|------|------|
| `agent_chat` | 对话能力 |
| `agent_cognition` | 统一认知工具（当前落地 `memory` 域 5 个操作） |
| `agent_create` | 创建智能体 |

**`agent_cognition` 操作列表**：

> 当前仅实现 `memory` 域的操作，由 `submit_cognition_op!` 宏自注册到全局 `OpRegistry`。`reason` / `learn` / `plan` / `metacognition` 为路线图目标，尚未落地。

| 域 | 操作 | 说明 |
|------|------|------|
| memory | save, retrieve, graph_query, reflect, consolidate | 记忆管理（保存 / 检索 / 关系图谱查询 / 反思 / 合并整理） |

---

## 4. 工具调用详解

### 4.1 agent_cognition(memory.save) - 保存认知单元

**请求**：
```json
{
  "operation": "memory.save",
  "content": "始终遵循安全协议",
  "type": "rule",
  "confidence": 0.9
}
```

**参数说明**：
- `operation`（必需）：操作标识，格式 `域.操作名`
- `content`（必需）：记忆内容
- `type`：记忆类型（semantic/fact/rule/experience/skill 等），默认 general
- `confidence`：置信度 0-1，默认 0.5
- `id`：自定义 ID（可选，不提供时系统自动生成）
- `tags`：标签列表
- `related`：关联单元 ID 列表

**使用示例**：
```json
// 保存规则
{"operation": "memory.save", "content": "代码标准：优先考虑健壮性", "type": "rule"}

// 保存事实
{"operation": "memory.save", "content": "今天学习了 Rust 所有权", "type": "fact", "confidence": 0.8}
```

### 4.2 agent_cognition(memory.retrieve) - 查询认知单元

**请求**：
```json
{
  "operation": "memory.retrieve",
  "query": "代码质量规则",
  "limit": 5
}
```

**参数说明**：
- `operation`（必需）：操作标识
- `query`：语义搜索关键词
- `id`：按 ID 精确获取
- `is_a`：按类型过滤
- `limit`：返回数量，默认 5

**查询示例**：
```json
// 语义搜索
{"operation": "memory.retrieve", "query": "代码质量规则", "limit": 3}

// 按 ID 获取
{"operation": "memory.retrieve", "id": "c5d6e7f8"}

// 按类型查询
{"operation": "memory.retrieve", "is_a": "rule", "limit": 10}
```

---

## 5. 任务分解与状态追踪

### 5.1 任务类型

| 类型 | is_a 值 | 说明 |
|------|---------|------|
| 任务 | `task` | 可分解的独立工作单元 |
| 子任务 | `subtask` | 从属于父任务的分解单元 |

### 5.2 任务状态

| 状态 | ID | 说明 |
|------|-----|------|
| 等待中 | `pending` | 等待执行 |
| 进行中 | `in_progress` | 正在执行 |
| 已完成 | `completed` | 成功完成 |
| 被阻塞 | `blocked` | 因依赖未完成而等待 |
| 已取消 | `cancelled` | 被取消 |

### 5.3 任务属性

| 属性 | 说明 |
|------|------|
| `task_status` | 当前状态 |
| `parent_task` | 父任务 ID |
| `subtasks` | 子任务 ID 列表 |
| `dependencies` | 依赖任务 ID 列表 |
| `priority` | 优先级 |
| `result` | 执行结果 |

### 5.4 使用示例

**创建父任务**：
```json
{
  "unit": {
    "is_a": "task",
    "name": "完成系统架构设计",
    "task_status": "pending",
    "description": "设计整个系统的架构方案"
  }
}
```

**创建子任务**：
```json
{
  "unit": {
    "is_a": "subtask",
    "parent_task": "t1",
    "name": "设计数据库方案",
    "task_status": "pending",
    "dependencies": []
  }
}
```

**更新任务状态**：
```json
{
  "unit": {
    "id": "t2",
    "is_a": "subtask",
    "task_status": "completed",
    "result": "采用 PostgreSQL + Redis 方案"
  }
}
```

---

## 6. 创建智能体指南

### 6.1 手动创建（目录模式）

1. 创建目录：`~/.symbio/agents/{agent_id}/`
2. 创建 `identity.yaml`：
```yaml
id: identity
is_a: fact
level: sys
name: 我的智能体
description: 这是我的自定义智能体描述。
```

### 6.2 手动创建（JSONL 文件模式）

创建 `~/.symbio/agents/{agent_id}.jsonl`：
```json
{"id":"identity","is_a":"fact","level":"sys","name":"我的智能体","description":"这是我的自定义智能体描述。"}
{"id":"a1b2c3d4","is_a":"rule","level":"sys","description":"始终遵循安全协议"}
```

### 6.3 通过 API 创建

```json
{
  "id": "my_agent",
  "path": "/path/to/agent/directory",
  "is_global": false
}
```

**参数说明**：
- `id`：智能体唯一标识符
- `path`：源目录或文件路径
- `is_global`：是否安装为全局智能体
- `format`：可选，指定格式（`yaml`, `json`, `jsonl`）

---

## 7. 深入阅读

- [**COGNITION.md**](./docs/COGNITION.md)：认知体系的类型定义、属性规范、验证规则
- [**ARCHITECTURE.md**](./docs/explanation/ARCHITECTURE.md)：底层 Mindscape 引擎的架构原理与演进蓝图
