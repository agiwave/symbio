# Agent 插件

认知中心：管理 Agent 人格（CU 认知单元）。**不独占会话编排**——会话编排唯一入口是 session；agent 在会话选定智能体时经 `traverse` 贡献工具与人格。

## 职责

- Bundle 资产管理：作为 `.vdfs/agent` 挂载点提供 bundle 的浏览 / 整包导入 / 导出 / 删除，目录自管（`BundleStore`：工作区级 + 全局级双层）
- 人格贡献：经 `CapabilityVisitor::register_system_prompt` 注册两段——**人格**（bundle 的提示词 / 技能 / MCP 装配）与**智能体记忆**（bundle 根下的 `AGENTS.md`）。系统提示词只有这一个注册通道，注册的每一段都按顺序送达模型，不存在「取一个」的竞争；拼接与消费在 session 侧
- 工具贡献：`traverse(agent/available_tools)` 把 `agent_run`（子智能体委托）加入会话工具集
- 选项贡献：`traverse(agent/available_options)` 提供会话页的「智能体」选择项

## 记忆（本插件的那个作用域）

```text
{bundle 目录}/AGENTS.md                         记忆本体
.vdfs/agent/<bundle id>/AGENTS.md               可编辑地址
```

记忆**不是 bundle 的条目**：它不参与 `classify_item_path` 的白名单（那条白名单描述的是 `prompts/` `skills/` `mcps/` 三个约定能力目录），也不计入概览的条目计数。它是 bundle 根下的一个普通文件，与工作区级 / 会话级的 `AGENTS.md` **同名同语义**——放哪个作用域就管哪个作用域。

机制（读写、两道容量闸门、片段排版、VDFS 节点）全在 `symbio_core::memory`，本插件只提供「落位 + 标题 + 地址 + 空提示 + 闸门取值」。**不可删除**：要清空就写入空内容。

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

- 会话编排与系统提示词的拼接 / 消费：`../session/README.md`
- 另两层记忆：`../work/README.md`（工作区）、`../session/README.md`（会话）
- 记忆内核（三层共用）：`symbio_core::memory`
- VDFS 机制（`.vdfs/agent` 挂载点由本插件自持 `impl VdfsProvider`）：`docs/design/vdfs.md` §13.4
