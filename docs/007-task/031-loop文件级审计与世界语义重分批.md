---
kind: task
id: T-031-loop文件级审计与世界语义重分批
title: loop 文件级审计：工具域 / turn 骨架全面归属重审与 T-030 分批修订案
status: doing
track: arch
prio: now
created: 2026-11-02
parent: T-026-loop耦合审计
deps: []
where: "零代码改动；审计对象 crates/symbio-plugin-loop/src/{tool_dispatch.rs,tools.rs,tools/**,turn_exec.rs,turn_exec/**,turn_runner.rs,turn_runner/**,turn_session.rs}，联动 chat_loop/**、context/**、stage_events.rs、provider.rs；对照 crates/symbio-plugin-session/src/{port.rs,capabilities.rs,orchestrator/consume.rs}"
exit: 六项输出（清单 / 为什么在 loop / 目标落点 / T-029 重审与 M3 评估 / T-030 修订与新分批 / ≤30 行结论）全部落本文档并经用户裁决；A 批边界（M3 合同「不做什么」/ `tools/**` 迁出清单 / exit / 回退点）已落 §六，等 M3 裁决（turn 为废弃概念、不搬 session，turn 全量审计优先，见 [T-032]）；裁决前不动工具域代码、不入库；T-030 Step 1 未提交改动（B4/B5 改名）保留工作区不回退不入库，已另存命名 stash 与补丁备份（§6.5）
---

# loop 文件级审计：工具域 / turn 骨架全面归属重审与 T-030 分批修订案

## 内容

用户裁决暂停 T-030 Step 1 落地，指出 loop 仍有大量世界端口特例（`tool_dispatch.rs`、
`tools.rs`、`tools/**`、`turn_exec.rs`、`turn_runner.rs`、`turn_session.rs` 及对应目录），
[T-026] 的 C 类 8 项覆盖不足。本文档做文件级审计：逐文件给清单（行数 / 职责 / 公开面 /
依赖 / 状态所有权 / 调用方 / 测试）、归因（为什么在 loop）、目标落点、T-029 重审（含
M3 薄 tool 端口 provider 评估）、与 [T-030] 的关系及新分批计划。零代码改动。

## 结论（先给，≤30 行口径同回复）

1. 审计范围（生产代码）约 **10,900 行**：工具域 ~4,300（tool_dispatch 1621 + tools/** ~2,700），
   turn 骨架 ~6,600（turn_exec 3,824 + turn_runner 3,311 + turn_session 122）。联动已分类
   未搬的 chat_loop ~3,600、context ~1,700、stage_events 495。loop 全 crate 非测试约 2.0 万行，
   真环机制（loop_rows / port / control_decl 求解器主体 / tokenizer / report_exec）约 5,500。
2. [T-026] 的主要误判：把 `tools/dispatch` 批编排族 + `tool_dispatch` 整体归 D 类「环的
   编排」——其中 ChatNode 广播、风险档并发、熔断、结果守卫、未执行收口、模型可见清单
   全是**工具 / 模型世界语义**，不是端口无关的环机制。
3. T-029 方向没错（合同下沉、不建代理），但合同面**太薄**：`PortProvider` 上的 `execute`
   （拟议方法，尚未落地）只覆盖
   单笔 glue（名解析 / 超时 / 结果契约），批级工具世界无合同可去，被上述误判扣在 loop。
4. T-030 Step 1 **被修订不替代**：泛化（PORTS 注入 / ROWS 参数化 / 纯度门禁）后移——门禁
   类型面基线在工具世界未搬走前钉住大量 tool 词根，形同虚设。新顺序：批 A（M3 工具批
   形状合同 + tools/** 迁出）→ 批 B（turn 骨架搬 session，C 类扩容）→ 批 C（泛化 + 纯度
   门禁，类型面基线直接归零）。

## 一、清单（逐文件）

行数为生产代码行（测试另注）。依赖栏：K=kernel、A=plugin-api、S=sdk、Ses=session、
M=model、T=tool。「状态所有权」指 TurnState / LoopSession / SessionContext 的读写点。

### 工具域

| 文件 | 行 | 职责 | 公开面 | 依赖 | 状态所有权 | 调用方 | 测试 |
|---|---|---|---|---|---|---|---|
| `tool_dispatch.rs` | 1621 | 工具批的环侧半边：`ToolDispatch` trait（`open`/`authorize`/`begin`/`finish`/`wake`，:287-350）、批不变量、助手轮消息补建、`DispatchOutcome` 映射、`batch_call_id`/`latest_model_response`（按模型响应配批，:396-425） | `pub(crate)`：`ToolDispatch`、`PortOutput`、`DerivedFacts` 等 | K（`TurnToolCallInfo`）、A、S；`ports::TOOL/MODEL` 字面量 :404-419、:530 | 写 TurnState 草稿（批寄存）、读 SessionContext 转写 | turn_runner/{stream,batch}、turn_exec/drive | tool_dispatch.test.rs 545 |
| `tools.rs` | 43 | 工具域门面：三子模块 re-export（自称「session 域内」，:1-14） | `process_tool_calls_async`、`is_unanswered`、`NOT_EXECUTED`/`ABORTED`、`CircuitBreaker` | — | — | tool_dispatch.rs:67-68 | — |
| `tools/dispatch.rs` | 95 | 批处理三阶段分层（gates→batch→unfinished）+ `emit_parent_finalized`（ChatNode 父终态广播，:57-99） | 阶段 re-export | K（exec sink、`failure_kind`）、S（`ChatNode`/`NodeStatus`） | 读写 SessionContext.messages | tools/tool_executor | — |
| `tools/dispatch/gates.rs` | 488 | 批首闸门读数：mode、熔断、自声明位、主体与控制面结论、本批留底、同参台账 | `GateReadings`、`first_refusal`、`refuse`、`GateContext`、`Refusal`、`CircuitBreaker` | `port_of(PORTS,"tool")` :145（字面量） | 读 TurnState / 会话阈值 | batch.rs | gates.test.rs 212 |
| `tools/dispatch/batch.rs` | 1011 | 逐条执行：协议失败 / 宿主闸 / 声明闸 / 钩子拦下 / 正常执行四出口；`emit_tool_running`（ChatNode Streaming 广播，:112-156） | 段内函数 | K、A（hooks/EffectInput）、S（ChatNode） | 读写 SessionContext、TurnState | process_tool_calls_async | （tool_executor.test.rs 覆盖） |
| `tools/dispatch/parallel.rs` | 109 | 风险档 → 并行/串行策略 | `policy_for_port`、`ExecPolicy`、`ToolRisks` | — | — | batch.rs | parallel.test.rs 82 |
| `tools/dispatch/unfinished.rs` | 135 | 未执行收口：`NOT_EXECUTED`/`ABORTED` 成因词 + 结果子节点落法（:36-78） | `is_unanswered`、`NOT_EXECUTED`、`ABORTED` | S（ChatNode） | 写转写 | batch.rs、tools.rs 门面 | （同上） |
| `tools/tool_executor.rs` | 223 | T-029 后薄壳：`execute_tool_async` 经 `port::tool_port` 取 `ToolCallExecutor.execute`（:79-126）；`process_tool_calls_async` 编排入口 | `process_tool_calls_async` 等 | A（PortProvider） | — | tools.rs 门面 → tool_dispatch | tool_executor.test.rs 1670 |
| `tools/tool_executor/decls.rs` | 66 | `BehaviorDecls`：模型可见工具清单构造（名字等值再末段）——**model 请求形状** | `BehaviorDecls`、`of_metas`、`behaviors_of` | K（CapabilityMeta） | — | chat_loop/inputs（清单进请求） | decls.test.rs 159 |
| `tools/tool_executor/effects.rs` | 62 | `record_effect`：工具效果落格 | `record_effect` | A（`EffectInput`） | 写事实轨 | batch.rs | effects.test.rs 100 |
| `tools/tool_executor/breaker.rs` | 114 | 熔断（同参连续失败阈值 / 冷却） | `CircuitBreaker` | — | — | gates.rs | （gates.test.rs 覆盖） |
| `tools/tool_result_guard.rs`（已迁出 → `crates/symbio-plugin-tool/src/result_guard.rs`） | 175 | 结果体积上限与归档滚动（`guard_result`） | `guard_result`、`DEFAULT_TOKEN_CAP` | — | 执行体持有存档目录（P1b / D-308） | tool 端口 `finish` | result_guard.test.rs 134 |

### turn 骨架

| 文件 | 行 | 职责 | 依赖与状态所有权 | 调用链 | 测试 |
|---|---|---|---|---|---|
| `turn_exec.rs` | 1649 | 轮次执行体的声明与绑定表（ACTORS）、`Ctx` 面（head/草稿/请求句柄）；子模块 re-export | K（ActorSpec/ChatNode）、S（LlmAdapter）、LoopSession | 被 turn_exec/drive 消费 | turn_exec.test.rs 2909 |
| `turn_exec/drive.rs` | 890 | **环体**：幕首 / 幕内选债循环 / 收幕三段；幕接续 | 读写 SessionContext、TurnState；经 TurnRunner 驱动模型往返 | provider.rs run_chat_loop → 此处 | （drive.test.rs 于 chat_loop） |
| `turn_exec/call.rs` | 301 | 开轮：建草稿、**取 `port.model`**（:68-78）、构造 TurnInput | LoopSession、model 端口 | drive 幕首 | — |
| `turn_exec/assemble.rs` | 370 | 装配：事实源（`session.facts()`）、补落寄存（`DRAIN_PARKED`，:176-187）、流式桥、`Assembled` | LoopSession | drive 幕首 | — |
| `turn_exec/persist.rs` | 396 | 收幕落库：派生事实、收束贡献、淘汰回路、记忆巩固、`ExecTurnResult`（:86-204） | LoopSession、TurnState | drive 收幕 | — |
| `turn_exec/outcome.rs` | 218 | 轮末归类（`Settled` → 结果族） | TurnState | persist | — |
| `turn_runner.rs` | 1028 | 轮次运行器：`Prepared` 草稿、`begin_model`/`await_model`（:612-685）、`close_round` | LoopSession、Store、LlmAdapter、ToolDispatch | turn_exec/drive | turn_runner.test.rs 5211 |
| `turn_runner/stream.rs` | 1045 | 模型流式路径：`announce_round`（:292-314）、`poll_flight`（:327-373，await-model 的实现后端）、`settle_flight`（:390-510）、response/failed 落格（:142-176） | `port::begin`（:76-92）、ports::MODEL | turn_runner::await_model | （turn_runner.test.rs 覆盖） |
| `turn_runner/settle.rs` | 269 | 收束格构造：`chat.assistant.fallback`（:102-149）、`final`（:174-197）、中止 / 耗尽 | Store、TurnOutcome | stream/persist | （同上） |
| `turn_runner/boundary.rs` | 47 | `RoundDecision`（Halt/Act/ConfigGap）纯判断 | LlmTurn | settle_flight | （同上） |
| `turn_runner/prepare.rs` | 432 | 接缝行执行体（prepare-turn）：压缩入格、水位、请求视图、定档、首响 | SessionContext、tokenizer | drive 选债 | （同上） |
| `turn_runner/facts.rs` | 428 | 轮事实视图读数 | Store | 多处 | （同上） |
| `turn_runner/batch.rs` | 90 | 工具批寄存与收尾（对接 ToolDispatch） | TurnState | stream/drive | （同上） |
| `turn_session.rs` | 122 | `LoopSession`：TurnDispatch 收窄句柄（spec/facts/SessionPort，run_op 唯一动作出口） | S（TurnSpec）、SessionPort | 构造点唯一 provider.rs | — |

### 联动（已分类未搬，本轮只复核不改判）

`chat_loop.rs` 771 + `chat_loop/{state 557, inputs 593, drive 493, decide 287, wake 310, delegate 228,
compose 346, turn 136, io 113}`——`SessionContext`（state.rs:123-150）、`TurnState` 18 格
（state.rs:183-329）、`ChatOrchestrator`（state.rs:372+）定义地；`context.rs` 344 +
`context/{pipeline 996, prompt 162, conversation_view 101, providers 56, view 62}`（B8）；
`stage_events.rs` 495；`provider.rs` 101（run_turn 入口装配）。外部边界：session 侧
`orchestrator/consume.rs:367-425` 构造 TurnDispatch 派发 `loop/<sid>` run_turn；
`capabilities.rs:109-141` collect_capabilities 组装 `ToolPort`；session `port.rs:25-220`
为 `port.tool` 的 PortProvider（名解析 / 空闲超时 / 中止）。e2e：仓库根 `e2e/`，
由 `scripts/e2e-order.mjs` / `e2e-concurrency.mjs` 驱动，96 用例。

## 二、为什么在 loop（逐项 file:line 证据）

1. **历史路径**：loop 是从会话引擎整体拆出的「执行引擎」（turn_session.rs:4-10：搬家前
   驱动器直接持 `Arc<PersistentChatSession>`）；tools.rs:1 自称「session 域内」却住在
   loop——按引擎切、未按世界语义切。
2. **共享状态**：TurnState / SessionContext / ChatOrchestrator 定义在 loop（state.rs:123 /
   183 / 372），工具批直接广播 ChatNode（batch.rs:112-156）、模型流式直接写草稿
   （stream.rs:390-510）——状态在哪，世界代码跟到哪。
3. **E-009 规避**：session 是装配宿主又是派发方；批执行若不在 loop，需要 session→loop
   回读或代理，两案均被 D-298 否决，于是 glue 与编排一起留置。
4. **端口合同缺口（主因）**：T-029 只下沉单笔 glue（session port.rs:71-99 / 182-220）；
   批级语义（ChatNode 广播、风险档、熔断、守卫、未执行收口、模型可见清单）无合同面可去
   ——D-298 M2 把它们裁成「环的编排」留 loop（DE-045 D-298，`tools/dispatch` 的
   batch/parallel/gates 与 breaker 是环的编排），[T-026] §一 D 类沿用。**该归类是本轮
   主要修订对象**：它们消费 ChatNode / 风险档 / 熔断这些工具与模型世界词汇，不是
   `(port,id,attempt)` 记账那种端口无关机制。
5. **测试耦合**：turn_runner.test.rs 5211、turn_exec.test.rs 2909、loop_rows.test.rs 3017、
   tool_executor.test.rs 1670——全经 test_support 在 crate 内驱动全栈，「行为零差」切批
   时自然整体留置。
6. **装配表位置**：六张注册表与 LOOP_OWNER 在 loop（loop_rows.rs:1396-1425）；
   `chat.assistant.*` / `turn.stage` / `artifact.added` 等会话词汇由 loop 交表
   （control_decl.rs EVENTS/FACTS），消费这些词汇的执行体（settle / batch / unfinished）
   留同 crate 才编译——声明面与执行面互锁。

## 三、目标落点（三标：tool-specific / turn-specific / 真环）

> **口径失效提示**：`[DE-047]` 生效后，下表中凡出现「批形状合同」「按模型响应配批」
> 「批执行循环 → 环」的措辞均**作废**（X-1 / X-5 / X-6 / X-7）。归属按 `[DE-047]` §2 判定表，
> 本文只保留审计事实（file:line 与行数）。

| 文件 / 模块 | 标 | 去向 |
|---|---|---|
| `tool_dispatch.rs`：ToolDispatch trait、批不变量、wake/finish 记账 | 真环（参数化后） | 留环，统一 `await-{port}` 批 join |
| `tool_dispatch.rs`：助手轮消息补建、DispatchOutcome 映射、`call_signature` 按模型响应配批 | tool/turn 特例 | tool 批形状合同（见 §四 M3） |
| `tools/tool_executor.rs` 薄壳 | tool 特例 | 随批形状下沉，环内消失 |
| `tools/tool_executor/decls.rs`（BehaviorDecls） | tool 特例（实为 **model 请求形状**） | model 端口合同扩展（T-024 形状）或 plugin-api |
| `tools/tool_executor/effects.rs` | tool 特例 | plugin-api 合同面（`EffectInput` 已在彼处） |
| `tools/tool_executor/breaker.rs` | tool 特例 | tool 批形状合同（熔断读数） |
| `tools/dispatch/gates.rs` | 拆 | mode / 自声明位 / 主体结论 → session 决策；熔断 / 同参台账 → tool 批形状；`port_of(name)` 参数化留环消费 |
| `tools/dispatch/batch.rs` | 拆 | ChatNode 广播 / 父终态 → session 转写面（经 sink 回调）；批执行循环 → 环 |
| `tools/dispatch/parallel.rs` | tool 特例 | tool 批形状合同（风险档读数） |
| `tools/dispatch/unfinished.rs` | turn 特例 | session（结果子节点落法） |
| `tools/tool_result_guard.rs` | tool 特例 | tool 批形状合同（或 session 存档面，随裁决） |
| `turn_exec.rs` + `{call,assemble,persist,outcome}` | turn 特例 | session 骨架 |
| `turn_exec/drive.rs` 环体 | 真环 | 留环（选债循环本体；不按行名分支，经绑定表派发） |
| `turn_runner.rs` + `{prepare,facts,batch,settle,boundary}` | turn 特例 | session（settle 落 final/fallback；boundary 的 Halt/Act/ConfigGap 随判决面复核归属） |
| `turn_runner/stream.rs` | 拆 | poll 骨架 → 真环（`await-{port}` 统一等待）；模型流式桥 / 半截正文 → model 端口合同 |
| `turn_session.rs` | turn 特例 | session（C3 原判不变） |
| `chat_loop/**`、`stage_events.rs`、`provider.rs` 入口 | turn 特例 | session（C 类原判不变） |
| `context/**` | B8 | 随 C 搬 session（对话面草稿）；prompt 渲染形状复核 model 合同 |
| `loop_rows.rs`、`port.rs`、control_decl 求解器主体、`tokenizer.rs`、`report_exec.rs` | 真环 | 留 |

## 四、T-029 重审与 M3 薄 tool 端口 provider 评估

> **M3 整体作废 → `[DE-047]` X-1 / X-9**：下面第 151–159 行的「批形状合同 + 推荐 ③」不再成立。
> 批不是环概念，模型响应不带调用，调用由 decl 依据事实逐笔欠出；工具端口只保留单笔
> `ToolCallExecutor`。M1 / M2 的结论（合同下沉、每工具插件仍是执行者、不建代理、
> 缺失响亮失败）原样有效。

- **不判错的部分**：合同下沉方向、每工具插件仍是执行者（`Capability::execute`）、不建
  `symbio-plugin-tool` 代理、缺失响亮失败——与 D-298 判据一致，不回退。
- **判错的部分**：合同面停在单笔 glue。session 既是装配宿主义是 tool glue owner
  （session port.rs:25-220），而批级工具世界（~2,700 行 tools/ + tool_dispatch 配批半边）
  因「环的编排」归类留在 loop——归类的依据（批调度 = 环）只对**记账骨架**成立，对
  ChatNode / 风险档 / 熔断 / 守卫 / 清单不成立。[T-029] 按该归类执行无误，错在归类本身。
- **M3 薄 tool 端口 provider**（用户点名复核）：形状 = 把合同从「单笔 execute」扩到
  **批形状**——open 批输入（`TurnToolCallInfo` 列表 + 风险读数口 + 结果广播回调经
  `ExecEventSink` 回 session）、批首闸门读数（熔断 / 同参台账）、并发策略、结果守卫；
  **不接管**选债（authorize/dispatch/await 行留环）、授权判据（风险 vs 阈值比较留环
  authorizer）、重试（租约 / attempt 留环 port.rs）。落点三选：
  ① session 现有 ToolPort 扩面——零新 crate，但 session 持有 ChatNode 广播，越装配宿主之界；
  ② 各工具插件直接提供——D-298 已否的「总执行者」变体，且每插件重复批 glue；
  ③ **薄 provider 层（机制侧）**——批形状合同住 plugin-api，实现由工具收集宿主装配，
    与 D-298 M2 原文「端口适配器（机制侧），不是插件」一致。**推荐 ③**。
- 结论：T-029 不回退；其「落点 session」限缩为 B1–B3 单笔 glue 的过渡落点；批形状作为
  M3 补件重开任务（挂 `Core-Change: D-298` 修订）。

## 五、与 T-030 的关系：修订，不替代

- Step 1 方向（PORTS 注入 / ROWS 参数化 / `port_of(name)` / await-{port} / 纯度门禁）全部
  保留，但**后移**：门禁类型面基线在工具世界未搬走前要钉 `ToolDispatch`/`ToolPortCall`/
  `TurnToolCallInfo` 等大量词根，「只许降」变成空转。先清世界语义，再泛化，类型面基线
  才能一步归零（对应用户约束 2 的精神：C 类 Turn 基线归零而非只降）。
- 未提交的 B4/B5 改名（`BehaviorDecls`）保留工作区，随批 A 入库——名字与批 A 中 decls.rs
  的迁移落点不冲突（改名先行、迁移随后）。

### 新分批计划（草案，待裁决）

| 批 | 内容 | 验证读数 |
|---|---|---|
| T-031（本档） | 审计落档，零代码 | registry-audit 绿 |
| 批 A（M3 补件） | plugin-api 批形状合同 + `tools/**` 与 `tool_dispatch` 工具特例半边迁出（落点按 §三）+ BehaviorDecls 迁 model 合同 + B4/B5 改名入库 | e2e 96/96 逐字不变；EL-003 8 只降；EX-003 净删 loop 行；`Core-Change: D-298` |
<!-- 作废 → [DE-047] X-1 / X-9：批 A 的「plugin-api 批形状合同」不成立；批合同整体收回（单独一笔），
     `tools/**` 迁移按 `[DE-047]` §2.1 判定表重排，不与合同处置混笔。 -->
| 批 B（C 类扩容搬家） | turn_exec/{call,assemble,persist,outcome}、turn_runner/{prepare,facts,batch,settle,boundary}、stream 拆分（poll 骨架留环）、chat_loop/**、stage_events、context（B8 复核）迁 session | e2e 96/96；CS-010 重指向格数只降；EX-007 零新增行；EX-003 净删 |
| 批 C（泛化收口） | PORTS 迁出注入、ROWS 六行参数化、`port_of(name)`、await-{port} 统一、纯度门禁落地（字面量面基线 0、类型面基线归零） | e2e 96/96；EL-001 16→14；EL-002 1 不变；EL-003 只降 |

- 顺序 A→B→C 的理由：批 A 后 loop 的 tool 词根只剩环记账骨架；批 B 后 Turn 族清零；
  批 C 的参数化不再需要为留环世界代码保留过渡基线——三批各自全绿、各自可回退
  （回退单位是 git 提交）。
- 每批一批一提交、`Migration:` trailer、门禁 `node scripts/gate.mjs --full` 全绿、
  e2e 逐字不变。

## 六、批 A 边界（M3 合同草案，待裁决）

> **§6 裁决（2026-11-02，`[DE-047]`，覆盖本节原草案）**：
>
> 1. **批不是环概念。** 环的单位只有债——一次端口调用一笔债（`request` / `sent` / `response` /
>    `failed` / `retry` + 配对、幂等键、重放、预算租约）；不存在「批循环」，也不存在
>    「一个模型响应包一批调用」。
> 2. **模型响应不带调用。** 模型端口只产事实，响应行的形状由模型端口自己的契约定。
> 3. **调用由 decl 欠出。** 声明方（行为 / 能力插件里的 decl）依据事实欠 `request(port=<工具端口>)`；
>    N 次调用 = N 笔债，由 `ring::drive` 逐步推进。
> 4. **世界语义归 provider。** 执行、并发、gate 读数、熔断、结果守卫、广播、超时全部是工具端口
>    provider 的内部实现；环不读 gate，也不解析任何模型输出形状。
>
> 因此：§6.1「M3 批形状合同」**作废**（X-1 / X-9）；§6.3 的按批归属表**作废**，改按
> `[DE-047]` §2.1 判定表执行；§6.2 第 5 条**作废**（见下）。

本节只备裁决用，**不动代码、不入库**。

### 6.1 M3 合同「做什么」

> **作废 → `[DE-047]` X-1 / X-9。** 下列四条（open 批输入、批首闸门读数、并发策略、结果守卫）
> 都不构成合同：其中 gate 读数与并发策略属 provider 内部，批输入/出参随批合同整体收回。
> 工具端口只保留单笔 `ToolCallRequest → ToolCallOutcome`。

批形状合同落机制侧（plugin-api），实现由工具收集宿主装配：

- open 批输入：`TurnToolCallInfo` 列表 + 风险读数口 + 结果广播回调（经 `ExecEventSink` 回 session）；
- 批首闸门读数：熔断状态 / 同参台账；
- 并发策略：风险档 → 并行 / 串行；
- 结果守卫：体积上限与归档滚动。

### 6.2 M3 合同「不做什么」

1. 不建 `symbio-plugin-tool` 执行代理，也不让每个工具插件重复批 glue（D-298 已否）。
2. 不接管选债：`authorize` / `dispatch` / `await-{port}` 行留环。
3. 不接管授权判据：风险 vs 阈值比较留环 authorizer。
4. 不接管重试：租约 / attempt 留环 `port.rs`。
5. ~~不扩 session 现有 `ToolPort` 去持有 ChatNode 广播（越装配宿主之界）；广播经 sink 回调回 session。~~
   <!-- 作废 → [DE-047] X-2：广播是 provider 内部执行策略，既不必「扩 ToolPort 持有」，
        也不经 sink 回调回 session；环不接触广播形状（I-5）。 -->
6. 不搬 turn 骨架（批 B / [T-032] 的事）。
7. 不动压缩链 / tokenizer（D-297）。
8. 不新增生产面 DTO / 前端展示；turn 词根只许降（T0 基线）。
9. 不为实现困难改合同形状（AGENTS：架构只认方案为源）。

### 6.3 `tools/**` 迁出清单（批 A）

> **作废 → `[DE-047]` §2.1。** 下表按「批」归属（批形状 / 批尾收口 / 按响应配批），与 I-1 / I-4
> 冲突；`tools/**` 的处置以判定表为准：`tool_executor` / `result_guard` / `decls` / `effects`
> 移交 provider，`dispatch/{batch,gates,parallel,unfinished}` 与 `breaker` 作废，
> `dispatch.rs` 作废，`tools.rs` 删除，`tool_dispatch.rs` 改写为三笔单笔债。

| 文件 | 行 | 处置 | 去向 |
|---|---|---|---|
| `tools/tool_executor.rs` | 223 | 迁出 | 环内消失（薄壳随批形状下沉） |
| `tools/tool_executor/decls.rs` | 66 | 迁出 | model 合同（T-024 形状）或 plugin-api（实为 model 请求形状） |
| `tools/tool_executor/effects.rs` | 62 | 迁出 | plugin-api 合同面（`EffectInput` 已在彼处） |
| `tools/tool_executor/breaker.rs` | 114 | 迁出 | tool 批形状（熔断读数） |
| `tools/tool_result_guard.rs` | 170 | 迁出 | tool 批形状（或 session 存档面，随裁决） |
| `tools/dispatch/gates.rs` | 488 | 拆 | mode / 自声明位 / 主体结论 → session；熔断 / 同参台账 → 批形状；`port_of(name)` 参数化留环 |
| `tools/dispatch/batch.rs` | 1011 | 拆 | ChatNode 广播 / 父终态 → session（经 sink）；批执行循环 → 环 |
| `tools/dispatch/parallel.rs` | 109 | 迁出 | tool 批形状（风险档读数） |
| `tools/dispatch/unfinished.rs` | 135 | 迁出 | session（结果子节点落法） |
| `tools/dispatch.rs` | 95 | 拆 | `emit_parent_finalized` → session；三段分层留环 |
| `tools.rs` | 43 | 随迁 | 门面 re-export 随子模块走，迁完只剩环侧或删 |
| `tool_dispatch.rs` | 1621 | 拆 | `ToolDispatch` / 批不变量 / wake·finish 记账留环（参数化）；助手轮补建 / `DispatchOutcome` 映射 / 按模型响应配批 → 批形状 |

留环：`loop_rows.rs`、`port.rs`、`control_decl` 求解器主体、`tokenizer.rs`、`report_exec.rs`。

### 6.4 exit（批 A 验收）

1. e2e 96/96 逐字不变。
2. EL-003 8 只降；EL-001 16 / EL-002 1 不变。
3. `capability-migration-audit` 绿：补件笔带 `Core-Change: D-298`；迁移笔带 `Migration:`，机制面零新增、控制面净删旧实现。
4. `turn-root-audit` 五面只降不增（T0 基线）。
5. `gate --full` 174/174；`cargo fmt --check` 干净。
6. loop 的工具世界词根随迁出净减，读数落档。

### 6.5 回退点与保护

- **回退单位 = git 提交**：批 A 首提交的父提交（当前 `1082d1e0`，T0）即整批回退点；批内补件 / 迁移各一笔，回退到父提交即整批撤销。
- **B4/B5 WIP 备份**：命名 stash `B4B5-WIP-backup 2026-11-02`（保留工作区不动）+ 补丁 `%TEMP%\b4b5-wip-20261102.patch`；批 A 开工前不改 `tools/**`。
- **裁决前冻结**：A 批不入库；M3 裁决落地后才开第一笔。

## 边界

- 本审计零代码；裁决前批 A/B/C 均不开工。
- 批 B 方向经 [T-032] 重裁：turn 为废弃概念、不搬 session——原「turn 骨架搬 session」作废，由 [T-032] §四 的替换批承接。
- [T-030] 档暂不改状态，待本档裁决时一并改档或关闭重开。
- B8（context 归属）与 boundary.rs（Halt/Act/ConfigGap）的最终落点在批 B 复核，
  本档只给倾向。
- D-297（压缩链留控制面）不受本审计影响；批 A 不动压缩链 / tokenizer。

## 处理过程

1. 盘库存：loop 全文件行数清单（Shell 统计），圈定审计范围 ~10,900 行 + 联动 ~5,800 行。
2. 三路并行勘察（工具域 / turn 骨架 / 状态与 session 侧），关键 file:line 均经子代理与
   主会话双核对；M3 出处核对 DE-045 D-298（M1/M2 定义）与 [T-025] 原文。
3. 归因六项、落点三标、T-029 重审、新分批逐项落 §二–§五；结论与回复 ≤30 行同口径。
4. 待办：用户裁决本档六项输出后，批 A 起按 §五顺序开工；[T-030] 届时改档。
