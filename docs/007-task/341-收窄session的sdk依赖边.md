---
kind: task
id: T-341-收窄session的sdk依赖边
title: 收窄 session 的 sdk 依赖边
status: doing
track: arch
prio: next
parent: T-340-session退役线
created: 2026-11-11
---

# 收窄 session 的 sdk 依赖边

## 内容

D-451 的**边一**：把 `scripts/exemptions/plugin-session-sdk.json` 的 **82** 条符号收窄到 0，
然后删掉这份豁免。

**它是一条读数，不是施工段**（`D-456`）：82 条里 67 条住在边二（装配）/ 边三（存储·转写）
要搬的代码里，它们的退场是那两条边的**结果**。本任务因此只负责两件事：① 摘掉那些「不住在
边二/边三 代码里、且出口已经能说清」的独立残余（15 条「两处都没有」的）；② 每次边二/边三
动完，把读数重新量一遍并核对账本。

**每摘一条符号，只允许三种走法**（都是仓内已有的正路，不新造通道）：

1. **用法本来该消失**：那条读口在被裁决撤掉之后已无人读——删用法（本批摘掉的 15 条
   余量就是这个形态的存量）；
2. **由 owner 提供**：session 只是持有与透传的，改由声明方 / owner 给出口
   （`D-427` 把能力开关真源搬回能力包、`D-428` 把「是不是同一能力的副本」搬回声明面，
   都是这条路）；
3. **经门面转出**：符号本来就该给插件用，走 `symbio-plugin-api` 的 `pub use`
   （`D-443` 之后门面只收生产依赖面，所以这一步要先回答「谁在消费」）。

**不许的走法**：把符号改名 / 换个形状再留在账上；在 session 里再抄一份折法（那是第二条
路径）；为了少一条符号去调宽 `plugin-api`（那只是把依赖搬家，边还在）。

where: `scripts/exemptions/plugin-session-sdk.json`、`crates/symbio-plugin-session/src/`、
`crates/symbio-plugin-api/src/lib.rs`、`crates/symbio-sdk/src/`。

exit: 豁免账 82 → 0；`scripts/exemptions/plugin-session-sdk.json` 删除；
`session-retire-audit` 与它的 10 条回归**同批从 `30-docs.mjs` 撤下**（它的输入没了，
留着会变成一条永远红的守卫）；`plugin-entry-audit` 的 E-009 对 session 只剩「没有这条边」
这一种读法。

## 处理过程

- 2026-11-11：登记。起点 101 条（D-451 校准后的真实分母）。第一批建议从 `reason` 里
  已经点明「只是持有与透传」的那几族入手（`AgentScope` / `agent_perms_of` /
  `stamp_branches` / `META_BRANCH` / `DriveEntry` / `DriveTarget` /
  `declared_drive_binding` / `ViewUnavailable`），它们各自都有 owner 侧的落点。
- 2026-11-11（同轮量账）：101 条里按「实现 / 测试」两分，**7 条只在测试里出现**
  （`PromptMessage` / `VDFS_PARENT_ADDR` / `budget_exceeded` / `commitment_events` /
  `production_matrix` / `vdfs_context` / `verify`）。这是风险最低的第一批：测试里的依赖
  通常是一条**为验证而生的读口**，不是生产路径。
- **首个目标：`PromptMessage`。** 它只剩一处（`store/wal.test.rs` 的 `verify_prompt_fidelity`
  用例），而 `symbio_sdk::projection::prompt_fidelity::verify` **全仓只有这一个消费方**——
  一个公共 sdk 出口只有别家测试在读，正是 `D-443` 拒绝的「为测试面开口」。
  入场券（两条选一，都要先答）：① 把 `verify` 的入参从 `&[PromptMessage]` 换成
  `&[TranscriptEntry]`（折法留在 sdk，调用方不再需要模型消息类型）；② 或把这段校验
  搬进 session 的测试、`verify` 从 sdk 删除（先确认没有第二个消费方、并按
  `arch-dead-code` 的棘轮下调基线）。**不许**为了摘符号在 session 里再抄一份折法。
- 2026-11-11（第一条真实收窄）：**`PromptMessage` 退场，101 → 100。** 走的是第 1 种走法
  的一个变体：用法本身没错，但**判据的入参面比它的比较面宽**，于是那条用法只为「凑一个
  类型」而存在。裁决落成 `D-452`：`verify` 收 `&[TranscriptEntry]`（它只比 role / text /
  tool 三格），`exclude_from_context` 的过滤仍留调用方（那是组包规则）。同批改了那句
  假注释——它声称「用投影条目会少验 `tool_call_id`」，而 `verify` 从不读那个字段。
  session 侧只剩 `Vec<_>` 一次 `cloned()`；`SYMBOL_BASELINE` 101 → 100。
  **下一步仍按同一形态找**：先问「判据 / 读口真的比了这么多格吗」，再问「谁在消费」——
  只被测试用、且替身比判据宽的符号，往往是同一种病的下一格。
- 2026-11-11（同批第二批）：**三条只在测试里出现的符号改走门面，100 → 97**（D-453）。
  `VDFS_PARENT_ADDR` / `vdfs_context` / `production_matrix` 在生产里本来就走门面（容器写
  上下文、子智能体折请求、回复闸与升格闸查授权表），会话的测试是在模拟容器，直取 sdk
  本体等于给同一件事留了第二条取字的路。改的是 `use` 与调用前缀，行为零变化（416 单测绿）。
  剩下真正「没有门面落点」的测试依赖（`verify` / `budget_exceeded` / `commitment_events`）
  不搬：它们读的是 core 的判据与成格口，硬搬只是把判据搬走，随边一闭合一起处理。
- 2026-11-11（同批第三批，**校准**）：**97 → 83**（D-454）。开工前先按路径重算了一遍账，
  发现 14 条**记错了 crate**：13 条 `symbio_kernel` 的词汇（`ChatNode` / `NodeRole` /
  `NodeStatus` / `NodeKind` / `MessageContent` / `Parts` / `Text` / `User` / `Assistant` /
  `Tool` / `System` / `chat_node` / `from_legacy`）与一条方法名 `at`。成因是两把尺的取法不同：
  E-009 按路径判、SR-001 按名字判，于是「名字在源码里出现」被当成了 sdk 用法。清账的同批把
  取法合成一份（`scripts/gate.d/_shared.mjs` 的 `sdkSymbolsIn`），SR-001 改路径级，并补了
  「名字在、路径不在 ⇒ 红」「`Type::method()` 记类型」两组回归。**这不是进度**：真实依赖面
  一直是 83，这一步只是把分母改对，并堵上那 14 个名字「随便从 sdk 取」的冻结口子。
- 2026-11-11（同批第四批）：**82**（D-455）。`budget_exceeded` 退场：会话的自主层验收原先
  自己递一个预算去调谓词（`budget_exceeded(&snapshot, Some(86.4M))`），而生产的预算读法
  只认 `turn.dispatch` 上的 `DeclaresBudget` 角色——那条用例自称钉「预算从哪来」，实际一次
  都没跑过那一跳。改走 `check_all`（生产读侧同一入口）后，判据自己从事实里读预算，用例
  反而更强（新增一档「同一耗时、更小预算 ⇒ 违规」证明预算取自事实而非常量）。
- 2026-11-11（同批）：**任务定性改了**（D-456）。`--breakdown` 量出 82 条的住处，67 条在
  边二 / 边三 的代码里 ⇒ 本任务不再被当成「先做完才能开边二/三」的前置阶段，而是**读数**。
  它剩下的独立残余是 15 条「两处都没有」的（命令面 6 / 统计读侧 5 / 配置 2 / 进度 1 /
  投影 1）——每摘一条仍要回答「谁消费」。这批**一条都没摘**：它们的摘法还没写出来，
  按 D-451 第 3 条只登记不开工。
