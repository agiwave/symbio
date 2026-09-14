# Agent 插件

认知中心：管理 Agent 人格（CU 认知单元）。**不独占会话编排**——会话编排唯一入口是 session；agent 在会话选定智能体时经 `traverse` 贡献工具与人格。

## 职责

- Agent 实体 CRUD 与默认/激活管理
- 人格贡献：`traverse(agent/persona)` 返回人格提示词，session 组装 system prompt 时取用
- 工具贡献：`traverse(agent/available_tools)` 把该 Agent 关联的工具加入会话工具集
- 子插件 `MindscapeScaffold`：CU（认知单元）的存储与检索脚手架

## 路由

| Path | 说明 |
|------|------|
| `agent/chat` | 与 Agent 对话（经 session 编排） |
| `agent/list` / `agent/create` / `agent/update` / `agent/delete` | Agent 实体管理 |
| `agent/set_default` / `agent/set_active` / `agent/get_active` / `agent/available` | 默认与激活态管理 |

## 机制化原则

- **关系机制化**：哪些属性名是"关系"由 `prop` CU 决定（`RelationPropRegistry::from_prop_cus`），核心代码不硬编码关系清单。
- **展示机制化**：`kind` 类型清单、索引优先级由 `prop` CU 的 `is_a` 与 `priority` 派生。
- **单一事实来源**：同一份 `seed_cus.jsonl` 同时驱动"如何解析 CU"与"如何展示 CU"。

## 关联

- 会话编排：`../session/README.md`
- 实体提供者机制：`docs/design/entity-provider-mechanism.md`
