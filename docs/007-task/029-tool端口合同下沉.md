---
kind: task
id: T-029-tool端口合同下沉
title: tool 端口合同下沉：B1–B5 迁 session，环只认端口
status: doing
track: arch
prio: now
created: 2026-11-02
parent: T-026-loop耦合审计
deps: []
where: "crates/symbio-plugin-api/src/port.rs, crates/symbio-plugin-session/src/{port.rs,capabilities.rs}, crates/symbio-plugin-loop/src/{port.rs,tools/tool_executor.rs}, e2e/cases/t99-tool-port-paths.mjs"
exit: B1–B5 按 [T-026] §一归属表落位：线上名解析 / 空闲超时 / 结果契约（B1/B2/B3）+ ToolDecls（B4）+ record_effect（B5）成为 tool 端口的 PortProvider 合同，执行体住 session 的 ToolPort；loop 的 execute_tool_async 只剩端口取用面；batch / parallel / gates / breaker 留环；装 / 不装 / 缺失三路径 e2e 等价，缺失响亮失败并落事实；EL-003 11 读点只降（实际 8，基线随降）；门禁全绿
---

# tool 端口合同下沉：B1–B5 迁 session，环只认端口

## 内容

按 [T-026] §四批次表执行 T-029（原 T-025 按审计重做）：loop 里 tool 世界的五样
语义（B1 线上名解析、B2 空闲超时、B3 结果契约、B4 ToolDecls、B5 record_effect）
下沉为 tool 端口的 `PortProvider` 合同；**落点 = session 插件**（顶层收集宿主，
不新建 `symbio-plugin-tool` 代理——各工具插件的 `Capability` 是执行提供者）。
`batch` / `parallel` / `gates` / `breaker` 是环机制，留 loop。

## 形状

- **plugin-api `port.rs`**：合同类型下沉——`ToolCallRequest` / `ToolCallOutcome` /
  `PendingPrompt` / `ToolCallExecutor`（`tool_execute` 返回执行器；默认 `None` =
  本端口无此合同，T-024 已立的形状）。
- **session `port.rs`（新）**：`ToolPort` 执行体——B1 解析失败不猜、按原名交路由
  拿 NotFound；B2 600s 空闲超时（`sink.progress().emitted()` 进展判据、`biased`
  select 三出口）；B3 `extract_result` / `PendingPrompt` / RESULT_CONTENT·RESULT_OUTPUT。
- **session `capabilities.rs`**：`collect_capabilities` 广播之后按「表里 ≥1 工具行」
  装 ToolPort 行（`<port:tool>:session`）——零工具 ⇒ 不装 ⇒ 环在调用点响亮失败。
- **loop `tool_executor.rs`**：`execute_tool_async` 薄壳化为端口取用面——按端口名读
  提供者、整份交调用现场、收回三元组；端口 / 合同缺席都出 `Error: `port.tool` 缺席`
  的响亮失败（与既有 route 错误同形）。

## 验证读数

- **单测**：session `port.test.rs` 迁 B1/B2/B3 行为测试；loop 侧保留
  `a_missing_tool_port_fails_loudly_on_the_result`（零工具 ⇒ 端口行缺席的响亮失败
  ——**该装配 e2e 不可达**，见下）。
- **e2e（t99）**：装 = 既有电池（t2/t3/t17 每次工具调用都经端口合同）；不装 =
  停 local/web/mcp/skill/agent，纯对话照常、工具面失去它们的工具、vdfs 底座工具
  仍在；缺失 = 同一装配强发停用工具（`web_search`）⇒ 诚实 NotFound 落成结果子节点
  （`meta.success=false`）+ 父节点终态 completed + 模型第二轮看到错误照常收尾。
- **EL-003**：11 → 8（`port` 三处全量读随执行体迁走），基线随降。
- **等价性**：强发停用工具的落库形状经 stash 对照——迁移前后**逐字同形**
  （父 `completed` + `meta.tool_name`；子 `failed` + `meta.success=false` + 错误正文；
  t6/t78 同族先例）。

## 裁决与偏差

- **「缺失 = 端口行缺席」在 e2e 不可达**：对话输入本身走 `vdfs/write` 收件箱，vdfs
  是不可停用底座且无条件贡献 9 个 `vdfs_*` 工具 ⇒ 能对话的装配必有工具行 ⇒ ToolPort
  行必装；停 vdfs = 无入口发起对话。故 e2e 的「缺失」取**可达形态**（调用的工具
  缺失，B1 收口分支），「零工具 ⇒ 行缺席」由单测钉——对 [T-026] §四「缺失路径」
  的这一解释读在此登记。
- **持久化视图的父节点不带合并 meta**（`success`/`failure_kind` 落在结果子节点）：
  落库镜像按 `into_messages` 重建原始节点，广播的父终态快照不进存储——迁移前后
  同形、非本批回归；「父终态快照带合并 meta」由 loop 单测钉。

## 边界

- 压缩链 / tokenizer 不随批（[T-026] 口径裁定一 / 二，D-297 终态）。
- B8（prompt / 对话视图归属）留 T-030 复核。
- `T-030` 不开工，等本批读数。

## 处理过程

1. 前置提交 `be46404c`：`capability_port_of` 同端口两行 provider 改为装载期硬红
   （`capability_port_check` 报 error 指名两者），172/172 全绿。
2. 合同面迁 plugin-api `port.rs`；session `port.rs` 承接执行体（B1/B2/B3 逐字迁，
   行为测试随迁 `port.test.rs`）；`capabilities.rs` 按「≥1 工具行」装 ToolPort 行；
   loop `tool_executor.rs` 薄壳化，测试夹具补 ToolPort 行 / 夹具执行器。
3. e2e t99 调试中撞上「落库父节点无 `success`、结果子节点 `failed`」的疑点——
   `git stash` 对照迁移前代码跑同一用例，**逐字同形**，定性为既有持久化契约
   （落库镜像按 `into_messages` 重建原始节点，非本批回归），断言改为与真实契约
   一致（`success` 断在结果子节点 meta，t6/t78 同族先例）。
4. EL-003 读数 11 → 8（`port` 三处全量读随执行体迁走），基线随降。
5. `cargo fmt` → `node scripts/gate.mjs --full` 172/172 全绿（e2e 96/96 含 t99、
   CI Rust 2617/2617、EL-003 8 读点）→ 提交（`Migration:` trailer）。读数基线随批
   上抬并各挂 `baseline-allow` 叙述：`ciRustTestsTotal` 2612→2617（−16 迁走 +20 落位
   +1 loop 新钉）、`e2eCases` 95→96（增 t99）。
