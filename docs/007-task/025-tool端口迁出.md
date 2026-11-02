---
kind: task
id: T-025-tool端口迁出
title: tool 端口迁出：只搬线上名解析 / 超时 / 结果契约，不建代理
status: todo
track: arch
prio: next
created: 2026-11-02
parent: T-001-能力全面拆分
deps: [T-023-世界端口注册面]
where: `crates/symbio-plugin-loop/src/tools/tool_executor.rs`, `crates/symbio-plugin-loop/src/tools/dispatch`, `crates/symbio-sdk/src/capability`, `crates/symbio-plugin-agent/src/host`
exit: `loop` 的 `execute_tool_async` 只剩环编排（`batch` / `parallel` / `gates` / 记账 / 幂等键）；线上名解析、空闲超时、结果契约（`RESULT_CONTENT` / `RESULT_OUTPUT` / `PendingPrompt` 的返回形状）搬进端口适配器（机制侧或既有工具收集宿主，不新建 `symbio-plugin-tool` 代理）；每个工具插件仍是执行者（`Capability::execute`）；装 / 不装工具插件时端到端行为逐字不变；停用工具插件 ⇒ 该工具 NotFound、不静默回退；`EL-003` 的 11 个工具路径全量读点只许降（低于 11 时同提交改基线），`EL-001` 的 16 行不变；迁移提交挂 `Migration: tool-port`，机制面新增行为 0
---

# tool 端口迁出：只搬线上名解析 / 超时 / 结果契约，不建代理

## 内容

`D-298` 的 M2：工具的世界实现**本来就在各工具插件**（`Capability::execute` 经 `tool` 族
动态收集）。`loop` 还要自己做的是三样 glue：

1. **线上名解析**：模型给的 `mcp__fs__read` → 能力名（`capability_resolve` + `tool_metas`）；
2. **超时**：`execute_tool_async` 里按空闲间隔轮询的那段；
3. **结果契约**：从 `PluginPayload` 解出 `RESULT_CONTENT` / `RESULT_OUTPUT` 与
   `PendingPrompt`（审批 / 交互）。

这三样搬出 `loop` 之后，`loop` 只按端口名 `tool` 调用适配器。**不建 `symbio-plugin-tool`
代理**：总执行者插件会同时违反「工具由各工具插件执行」与 `E-009` 的同一条理由；适配器
只做 glue，不拥有任何工具。

## 处理过程

1. 起手先定适配器落点（SDK 机制侧或既有工具收集宿主 `agent`），判据是「不新建 crate、
   不依赖兄弟插件、`loop` diff 只剩删除与绑定切换」；结论写进本任务的处理记录。
2. 先立反向用例：两个实现同时生效必须红（`EX-003`）；停用某工具插件 ⇒ 该工具
   `NotFound`，不是静默落到别的工具。
3. 同一提交删掉 `execute_tool_async` 的三样 glue，`loop` 只保留批调度、风险档并发、
   `gates` 与 `breaker`、`begin` / `settle` 记账。
4. 跑端到端与停用 / 启用工具插件的反向用例；`EL-003` 读数按实测只降不升，低于 11 时
   同提交改基线；不新开账本 / 脚本。
5. `T-014` 步 3 合流时复核「五类之外没有名字」与 `EL-001…003` 读数。

## 边界

- `batch` / `parallel` / `gates` / `breaker` 是**环的编排**，不随 tool 端口迁出。
- 不把工具执行搬进 `core` / `kernel` 或别的控制面来「满足」封闭集；执行者仍是工具插件。
- 不改 `PortDecl` 的 `concurrency` / `max_attempts`；迁出的是 glue，不是记账口径。
