---
kind: task
id: T-025-tool端口迁出
title: tool 端口迁出：声明与提供者归 `symbio-plugin-tool`，工具仍由各工具插件执行
status: todo
track: arch
prio: next
created: 2026-11-02
parent: T-001-能力全面拆分
deps: [T-023-世界端口注册面]
where: `crates/symbio-plugin-tool/**`, `crates/symbio-plugin-loop/src/tools/tool_executor.rs`, `crates/symbio-plugin-loop/src/tools/dispatch`, `crates/symbio-sdk/src/capability`, `crates/symbio-plugin-agent/src/host`
exit: `tool` 端口的声明与提供者归独立插件 `symbio-plugin-tool`（端口名 `tool`、单行 `<port:tool>:tool`，不持有任何工具）；线上名解析、空闲超时、结果契约（`RESULT_CONTENT` / `RESULT_OUTPUT` / `PendingPrompt` 的返回形状）住在该插件；每个工具插件仍是执行者（`Capability::execute`）；`loop` 的执行缝只认通用 `PortInput` / `PortOutcome` / `PortExecutor`；装 / 不装工具插件时端到端行为逐字不变；停用工具插件 ⇒ 该工具 NotFound、不静默回退；`EL-003` 的 11 个工具路径全量读点只许降（低于 11 时同提交改基线），`EL-001` 的 16 行不变；迁移提交挂 `Migration: tool-port`，机制面新增行为 0（[DE-048](../003-design/048-端口机制裁决.md "DE-048-端口机制裁决") D-303）
---

# tool 端口迁出：声明与提供者归 `symbio-plugin-tool`，工具仍由各工具插件执行

## 内容

`D-298` 的 M2：工具的世界实现**本来就在各工具插件**（`Capability::execute` 经 `tool` 族
动态收集）。`loop` 还要自己做的是三样 glue：

1. **线上名解析**：模型给的 `mcp__fs__read` → 能力名（`capability_resolve` + `tool_metas`）；
2. **超时**：`execute_tool_async` 里按空闲间隔轮询的那段；
3. **结果契约**：从 `PluginPayload` 解出 `RESULT_CONTENT` / `RESULT_OUTPUT` 与
   `PendingPrompt`（审批 / 交互）。

这三样与端口声明一起归独立插件 `symbio-plugin-tool`（裁决见
[DE-048](../003-design/048-端口机制裁决.md "DE-048-端口机制裁决") D-303）。插件只做 glue / 适配，
**不拥有任何工具**：每个工具仍是各工具插件的能力行，经能力表动态执行；它不依赖任何
兄弟插件（`E-009`），停用它只让 `tool` 端口缺席、不改变工具执行者。

## 处理过程

1. 新建 `crates/symbio-plugin-tool`：端口 `tool` 的 `PortDecl`（`before = sent`，
   租约 / 重试 / 并发沿用既有口径）+ `port_rows` 单行 `<port:tool>:tool` + `PortExecutor`
   执行体；`loop` 的执行缝换成 `PortInput` / `PortOutcome` / `PortExecutor`。
2. 反向用例：两个实现同时生效必须红（`EX-003`）；停用该插件 ⇒ 端口缺席、结果响亮失败，
   不是静默落到别的工具。
3. 执行缝只有一份：`loop` 不再持有 `ToolCallRequest` / `ToolCallExecutor` 一类具名合同。
4. 跑端到端与停用 / 启用该插件的反向用例；`EL-003` 读数按实测只降不升，低于 11 时
   同提交改基线；不新开账本 / 脚本。
5. `T-014` 步 3 合流时复核「五类之外没有名字」与 `EL-001…003` 读数。

## 边界

- `batch` / `parallel` / `gates` / `breaker` 是**环的编排**，不随 tool 端口迁出。
- 不把工具执行搬进 `core` / `kernel` 或别的控制面来「满足」封闭集；执行者仍是工具插件。
  本插件只提供端口，**不执行工具**（执行仍经各工具插件的能力行）。
- 不改 `PortDecl` 的 `concurrency` / `max_attempts`；迁出的是声明与 glue，不是记账口径。
