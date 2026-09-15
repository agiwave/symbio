# Agent 插件

认知中心：管理 Agent 人格（CU 认知单元）。**不独占会话编排**——会话编排唯一入口是 session；agent 在会话选定智能体时经 `traverse` 贡献工具与人格。

## 职责

- Bundle 资产管理：作为 `.vdfs/agent` 挂载点提供 bundle 的浏览 / 整包导入 / 导出 / 删除，目录自管（`BundleStore`：工作区级 + 全局级双层）
- 人格贡献：`traverse(agent/persona)` 返回人格提示词，session 组装 system prompt 时取用
- 工具贡献：`traverse(agent/available_tools)` 把 `agent_run`（子智能体委托）加入会话工具集
- 选项贡献：`traverse(agent/available_options)` 提供会话页的「智能体」选择项

## 路由

**agent 插件没有任何自有路由**：`route()` 直接返回 `NotFound` 并指引到 VDFS。
历史上的 `agent/entities/*`（S11）与 `agent/bundle/*`（S12 / S13）全部下线，
bundle 及其内部（提示词 / 技能 / MCP）一律经
`.vdfs/agent/<id>/<子类别标签>/<相对路径>` 寻址。

## 机制化原则

- **关系机制化**：哪些属性名是"关系"由 `prop` CU 决定（`RelationPropRegistry::from_prop_cus`），核心代码不硬编码关系清单。
- **展示机制化**：`kind` 类型清单、索引优先级由 `prop` CU 的 `is_a` 与 `priority` 派生。
- **单一事实来源**：同一份 `seed_cus.jsonl` 同时驱动"如何解析 CU"与"如何展示 CU"。

## 关联

- 会话编排：`../session/README.md`
- VDFS 机制（`.vdfs/agent` 挂载点由本插件自持 `impl VdfsProvider`）：`docs/design/vdfs.md` §13.4
