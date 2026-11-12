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

D-451 的**边一**：把 `scripts/exemptions/plugin-session-sdk.json` 的 **81** 条符号**只许降**
地管住——**不设归零**（`D-460`）。

**它是一条读数，不是施工段**（`D-456`）：账上 67 条住在边二（装配）/ 边三（存储·转写）
要搬的代码里，但存储与地址面都随身份 `session` 留下（`D-458` / `D-459`），所以那 67 条也
不会因为「搬走」而消失。本任务因此负责两件事：① 摘掉那些**入口只为凑一个形状而存在**的
符号（`D-452` / `D-453` / `D-455` 那三种形态）；② 每次动过代码，把读数重新量一遍并核对账本。

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

exit: **不再是「82 → 0 + 删豁免」**（`D-460`：82 条里 15 条是其它插件也在用的 core 词汇、
67 条随数据面留下，归零只在「会话不再使用 core 词汇」时可能，而那要么是抄类型、要么是会话
不写字）。终态改为：

- **只许降**：条数 ≤ `SYMBOL_BASELINE`（`SRA-002` 守），每摘一条同批下调；
- **零余量**：账上每条都真被用（`SRA-001` 守），用法消失的条目当场摘掉；
- **不得新增**：新用法即红（`E-009` 守）；
- 豁免文件**随身份保留**，`2027-06-30` 是复核日，不是删除日。

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
  E-009 按路径判、SRA-001 按名字判，于是「名字在源码里出现」被当成了 sdk 用法。清账的同批把
  取法合成一份（`scripts/gate.d/_shared.mjs` 的 `sdkSymbolsIn`），SRA-001 改路径级，并补了
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
- 2026-11-11（同批）：**本条账本的读数不变（82）**，但它的地位变了（D-458）。边三改判为
  验收条件之后，边一不再「等边三」，它只是边二（施工）与「能力面归零」这件事的读数。
  `submit_fact_source_opener` 仍在账上——会话仍登记那一个开面，`SRA-004` 的绊线盯的是
  「登记点恰好一家」，不是让这条符号退场。
- 2026-11-11（同批）：**本任务的 exit 改了**（D-460）。开工前逐条查了 15 条「两处都没有」的
  符号，发现它们**全部**在会话之外有生产消费方（`ReportDraft` 12 处、`SKILL_TAG` 12 处、
  `commitment_events` 11 处、`ViewUnavailable` 11 处、`ApprovalGrants` 9 处、
  `bind_inheriting` 8 处、`llm_removed_frame` 8 处……）——它们不是孤儿，是 **core 的词汇**，
  会话只是使用者之一。加上 67 条随数据面 / 地址面留下（D-458 / D-459），「归零」写不出来。
  所以豁免从「到期删除的欠条」改为**冻结账**：`retire` 改成「随身份保留（D-460）」、
  `expires` 改成复核日、`SRA-003` 从「必须写删除动作」改为「写明可兑现的处置（删除，或
  保留 / 冻结 + 依据决策号）」。**三条压力一条没松**：零余量 / 只许降 / 不得新增。
  顺带修掉 `reason` 开头那句对已删任务号 `T-034` 的引用（`D-016` 禁止）。
- 2026-11-12（**边一第二格真实收窄**）：**82 → 81**，`commitment_events` 退场，走的是
  走法 3（经门面转出）。`D-453` 当年把它与 `verify` / `budget_exceeded` 一起判「不搬」，
  理由是**没有门面落点**；`D-276` 之后门面补上了
  `pub use symbio_sdk::actors::commitment_events`，生产写侧 `agent/host/behavior.rs`
  本来就走它。**谁在消费**：会话 `stats.test.rs` 的承诺列 / 声誉列夹具模拟的是同一个成格口
  （注释自己写着「写侧改名，这两条一起红」），取字就该与写侧同源。三处夹具都走门面之后，
  账上这一条零命中，`SRA-001` 逼着当场摘掉；`SYMBOL_BASELINE` 82 → 81 与 `SRA-002`
  回归用例（83/82 → 82/81）同批下调，守卫 23 条回归全绿。
  `commitment_records` / `SKILL_TAG` 仍直取 sdk，不动——它们不在这条走法上。
