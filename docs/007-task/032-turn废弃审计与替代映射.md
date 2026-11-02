---
kind: task
id: T-032-turn废弃审计与替代映射
title: turn 废弃审计：全量清单、逐项替代映射与分批拆除计划（含展示层）
status: doing
track: arch
prio: now
created: 2026-11-02
parent: T-031-loop文件级审计与世界语义重分批
deps: []
where: "零代码改动；审计范围：loop 的 turn 机制面（turn_exec/**、turn_runner/**、turn_session.rs、chat_loop 判决/唤醒、TurnState）、sdk turn 域、session 消费面（orchestrator/worker_progress/config）、前端与 DTO（tauri/src、session changes 投影、流式帧）、e2e 断言面"
exit: 五项输出（turn 全量清单 / 逐项替代或删除 / 行为保持含无 turn 转写形状 / 分批计划与 M3·A/B/C 顺序 / ≤30 行结论）全部落本文档，且 §五 T0 冻结门禁可审稿（判据面·规则 T0-1…6·基线载体·首批提交清单）经用户裁决；裁决前不动任何代码、不提交；T-031 的 A 批边界继续暂停
---

# turn 废弃审计：全量清单、逐项替代映射与分批拆除计划（含展示层）

## 内容

用户两条裁决（优先级高于 [T-031] 的 A 批边界）：

1. **turn 是废弃概念，不是搬到 session**：model + tool 合并时代才有 turn；端口独立后
   model / tool 各自是独立债务，turn 不应再是机制状态。
2. **展示层也不需要 turn**：不留 turn 投影或兼容视图——转写按**平铺时间线**（消息、
   模型响应、工具调用都是独立 item，不按 turn 分组，不显示 turn stage / round 计数）；
   前端、API / DTO、快照测试、转写消费方全部纳入清单；纯度门禁连前端 / DTO 的 turn
   词根一起禁，不留展示豁免；现有 e2e / 前端若断言 turn 分组，改为断言时间线顺序与
   内容，不改行为。

本文档零代码：只给清单、替代映射、行为保持口径与分批计划。

## 结论（≤30 行口径同回复）

1. turn 机制面合计约 **8,600 行生产代码**：loop 编排 6,600（turn_exec 3,824 +
   turn_runner 3,311 + turn_session 122）+ 判决/唤醒 597（chat_loop/{decide,wake}）+
   TurnState 147 格 + control_decl 的 turn 事实/视图 ~300；外加 sdk turn 域、session
   消费面（consume/worker_progress/config）与前端 / DTO 展示面。
2. turn 的机制职责可全部由**既有机制**承接：身份=用户格 seq（scope 锚）；债务编组键
   =(port, id, attempt)（D-138 已立）；收束=视图推导（scope 无未清债）；进度=投影。
   绑定表的主张本来就是「行欠债由事实判」——turn 只是历史直线编排（幕首/幕内/收幕）
   的载体。
3. 逐项映射已给全（§二）：`turn.stage` 删（派生）；`turn_head`→scope_head；
   `close-turn`→finalize（判据改视图推导）；TurnState 18 格 6 删 11 替代 1 保留；
   turn_exec/turn_runner 直线编排删（环体直连端口行）；`Entity::Turn` 枚举改名
   `Scope` 属内核语法面，**单列需裁决**。
4. 展示层：DTO `turn` 字段、「第 N 轮」展示、流式帧 `meta.turn`、根级骨架（每轮一
   turn 节点）全拆；转写改平铺时间线（§三给形状示例）。NodeKind 旧词 `turn`→
   skeleton 折叠是**数据迁移通道**（读存量），不是展示保留，存量清迁后删。
5. 分批：**T0 冻结**（turn 词根门禁只许降，全链含前端/DTO）→ **A 批 M3 不变**（工具
   批形状，端口独立前半）→ **B 批改向**（原「turn 骨架搬 session」作废；改 turn 替换：
   B1 事实/视图 scope 化 → B2 编排删除 → B3 展示拆除）→ **C 批泛化收口**（PORTS/ROWS
   参数化 + 纯度门禁 turn/tool 词根归零）。e2e：断言 final/fallback 与节点终态的不动；
   断言 turn 分组的改时间线顺序断言，行为不变。

## 一、turn 全量清单（file:line + 行数）

### 1. 类型（机制状态）

| 类型 | 定义点 | 行数 | 说明 |
|---|---|---|---|
| `TurnState` | chat_loop/state.rs:183-329 | 147 | 18 格轮次可变状态（§二逐格映射） |
| `SessionContext` | chat_loop/state.rs:123-150 | 28 | messages + LoopSession + consolidated 寄存 |
| `ChatOrchestrator` | chat_loop/state.rs:372+ | ~60 | 环驱动器句柄 |
| `TurnInput` | turn_runner.rs:70-81 | 12 | turn/user_seq/drive_seq/abort/window_turns/actor |
| `TurnRunner` / `Prepared` / `ModelRound` / `ModelInFlight` | turn_runner.rs + stream.rs | ~300 | 轮次运行器与在飞模型草稿 |
| `Settled` / `ExecTurnResult` / `TurnExecExit` | turn_exec.rs:116-143 | — | 轮末归类与产出 |
| `RoundDecision` | turn_runner/boundary.rs:9-30 | 47 | Halt/Act/ConfigGap 纯判断 |
| `LoopSession` | turn_session.rs | 122 | TurnDispatch 收窄句柄（run_op 动作面） |
| `TurnSpec` / `TurnDispatch` / `TurnReport` | sdk turn/dispatch.rs:29-128 | ~190 | 轮次纯值输入 / 派发载荷 / 结局值 |
| `WorkerProgress` | sdk turn/facts.rs:57-75 | — | id/title/in_flight/**rounds**/steps（投影） |

### 2. 事实与格位（control_decl.rs）

- `Entity::Turn` 事件（EVENTS :102-185）：`user.message`（Opened + DeclaresBudget，
  host 写口）、`turn.stage`、`turn.supplemented`、`session.config`、`turn.dispatch`
  （DeclaresBudget）、`authz.denied`（StopsTurn）、`authz.granted`、`chat.assistant.`
  `{final（FinalizesTurn）, fallback, reported, opening, progress}`、`turn.branch` /
  `turn.branch.selected`。
- turn 词根视图（:1389-1509）：`turn_head`（读 stage/config/协议五格/final/fallback）、
  `turn_stages`、`turn_closed`、`turn.recalled`；切片 `turn_slice`（按轮收窄）；
  `final_unique_per_turn`。
- FactRole：`FinalizesTurn` / `StopsTurn` 挂在 final / authz.denied 上（内核角色名）。

### 3. 行（loop_rows.rs ROWS）

轮次语义行：`prepare-turn`(52，接缝，出口 `turn.stage{inputs}`)、`report`(53，D-163
按已关闭 `response(port=model)` 计数)、`close-turn`(55，收束格写点)、`continue-turn`
(56，续幕)；外围能力行 classify/compress/recall/notes/skill(10-50) 是行为不是 turn
状态（保留）；端口行族（60-72）归 C 批参数化。

### 4. 函数与执行体

- turn_exec 族：`assemble_turn` / `open_turn`（call.rs:49-78，取 port.model）/
  `persist_turn`（persist.rs:86-204）/ `classify_turn` / `drive`（drive.rs:7-16 幕首/
  幕内/收幕直线编排）。
- turn_runner 族：`begin_model` / `await_model`（turn_runner.rs:612-685，stream.rs 为
  实现后端 :292-510）/ `close_round` / `poll_flight` / `settle_flight` / `settle.rs`
  收束格构造（final :174-197 / fallback :102-149）。
- chat_loop 判决与唤醒：`decide_turn`（decide.rs:73-82）、`wake_batch`（wake.rs，
  批唤醒重建）；abort 轮边界检查（turn_runner.rs:93-108；chat_loop.rs:297-308「产出
  无论完成/失败/被中断都入格」）。

### 5. 测试

loop：turn_runner.test.rs 5211、turn_exec.test.rs 2909、chat_loop.test 族、
loop_rows.test.rs 3017（部分）、tool_dispatch.test.rs 545（部分）。session：
consume.test.rs（turn_node 夹具 :77-109）、config.test.rs（progress_min_rounds）。
sdk：turn 域单测。**e2e**（96 用例，e2e/ 由 e2e-order.mjs 驱动）：helpers.mjs:950
断言 `chat.assistant.final/fallback`（事件名不变则不动）；t10/t11/t16/t18 断言节点
终态集 `TERMINAL`（completed/failed/aborted/waiting_user_action，不涉 turn 词根）；
t11:116 注释级「本轮终态」表述。

### 6. UI / DTO / 转写消费方（本轮新增范围）

| 位置 | 证据 | 内容 |
|---|---|---|
| session changes DTO | tauri/src/services/sessionChanges.ts:10,34 | 行结构 `{path,address,op,label,turn,changes}`——DTO 带 turn 序号 |
| 「第 N 轮」展示 | tauri/src/components/session/SessionChangesList.vue:39 | UI 显示轮数 |
| 流式帧 meta | tauri/src/services/__tests__/toolRoundLiveFrames.spec.ts:22,77 | 根级骨架 `meta.turn`（后端 llm_emit_message / session frames 产出） |
| 根级骨架 = turn 分组 | useChatConnection.ts:235-240,294,464 | 「会话节点 = 正在跑的这一轮」；messageTree 按根骨架挂子节点 |
| NodeKind 旧词折叠 | tauri/src/schemas/chat_node.ts:202-212；registry/messageTypes、messageRenderers | 存量转写 `type:'turn'` 折叠进 skeleton（读侧数据迁移通道） |
| WorkerProgress.rounds | sdk turn/facts.rs:57-75；session worker_progress.rs:14,51 | rounds=role=user 消息条数（投影字段） |
| 快照/组件测试 | ChatMainPanel.changes.spec.ts:71、SessionChangesList.spec.ts:33,47、sessionTranscriptSync.spec.ts:203、toolRoundLiveFrames.spec.ts 全文件 | 断言 turn 字段 / meta.turn |
| 配置项 | session config.rs:192,220-221 | `progress_max_per_turn` / `progress_min_rounds`（行为参数，B3 复核改名） |

## 二、逐项替代或删除

| 项 | 裁决 | 替代物 |
|---|---|---|
| `turn.stage` | **删** | 派生视图：prepare 的产物（压缩入格、水位、请求视图、定档、首响）各自已是事实，「就绪」由视图推导 |
| `turn_head` | 替代 | `scope_head`：切窗键改用户格 seq（T-030 参数化口径延续） |
| `turn_stages` / `turn_closed` / `turn_slice` / `final_unique_per_turn` | 删 / 替代 | stages 删（事实没了）；closed→派生（scope 无未清端口债）；slice→`scope_slice`；unique→per_scope |
| `turn.dispatch` / `turn.supplemented` / `turn.branch(+selected)` | 改名 | `scope.dispatch`（DeclaresBudget 保留）/ `scope.supplemented` / `scope.branch(+selected)` |
| `close-turn` 行 | 替代 | `finalize` 行：判据改视图推导（scope 内无未清债 ∧ 终稿已交），不再由「轮次结束」触发 |
| `continue-turn` 行 | 改名 | `continue`：新 scope 开格（D-222 裁决 2 语义不变） |
| `prepare-turn` 行 | 改名 | 接缝行（`prepare-scope` 或去名词化，B1 定）；出口 turn.stage 删 |
| `turn_exec`（直线编排） | **删** | 幕首/收幕直线代码退场：开 scope = user.message 落格后行自然欠债；环体（drive 选债循环）保留直连端口行 |
| `turn_runner` | **删（拆）** | poll 骨架归 `await-{port}`（C 批参数化）；settle 收束格构造归 finalize 行执行体；Prepared 草稿格消解进事实/寄存 |
| `turn_session`（LoopSession） | 改义 | session 身份句柄（spec+facts+SessionPort）；run_op 动作面保留，去 turn 名 |
| `TurnState` 18 格 | 逐格 | abort→StopSignal（信号非状态）；request_round→删（scope 内已关闭 `response(port=model)` 计数，D-163 同口径）；last_saved→删（事实源保证）；input_utterance→user.message 事实；recall_view→memory 能力包投影（D-264）；context_sections→请求视图事实；opening_text/opening_dialog→chat.assistant.opening 事实；head_answer→派生（收束格）；request_overhead→预算声明格；turn_no→删（日志可数）；user_seq→**保留**（scope 锚）；drive_seq→删（机制内部计数）；now_ms→删（now 是输入，D-115 判据 1，取事实 ts）；facts→删（scope_slice 现读）；resume_root/resume_old_child→scope.branch 事实；retry_of→(port,id,attempt) 键（D-138） |
| `RoundDecision`（decide.rs 判决） | **删** | 判决进行判据：模型响应含未答工具调用 = `open-{port}` 行 owes；无债 = 环停（收束行不欠）；ConfigGap = authorize 拒绝路径 |
| `wake_batch`（wake.rs 唤醒） | 替代 | `await-{port}` 租约/重建（port.rs reap 已是恢复点） |
| `Entity::Turn`（内核枚举） | **需裁决** | 两案：a) 改名 `Scope`（内核事件语法一格，属「内核语法面」改动，挂 Core-Change 走架构讨论）；b) 保留成员仅语义重释。**推荐 a**：名字是机制面承诺，b 会留下说谎的名字 |
| `FinalizesTurn` / `StopsTurn` 角色 | 随上项 | `FinalizesScope` / `StopsScope` |
| DTO `turn` 字段 + 「第 N 轮」 | **删** | changes 投影去 turn 列；UI 不显示轮计数 |
| 流式帧 `meta.turn` | **删** | 帧 meta 去字段（后端 llm_emit 面 + 前端 spec 同批） |
| 根级骨架（turn 节点分组） | **删** | 平铺时间线（§三形状）；工具调用与其结果子节点的**调用关系父子保留**（那是调用树，不是 turn 分组） |
| NodeKind 旧词 `turn`→skeleton 折叠 | 迁移通道 | 读存量数据的折叠保留至存量清迁，随后删通道（不是展示豁免） |
| `WorkerProgress.rounds` | **删** | 投影字段删（不展示轮计数）；worker_progress.rs 投影口径随之收窄 |
| `progress_max_per_turn` / `progress_min_rounds` | B3 复核 | 行为参数（每 scope 进展上限 / 最少模型往返数才汇报），语义保留、名字去 turn/round 词根（如 progress_min_responses） |

## 三、行为保持

- **e2e**：断言 `chat.assistant.final/fallback`（helpers.mjs:950）与节点终态集的用例
  **不动**（事件名不变，仅格位 scope 化）；显式依赖 turn 分组的断言改**时间线顺序与
  内容**断言（t11:116 类「本轮终态」表述改「自上次 user 消息以来的终态」），行为零差。
- **UI / 转写（无 turn 的转写形状）**：

```jsonc
// 时间线 = 按 seq 平铺的 item 流；无分组轴、无 stage、无 round 计数。
// 「一轮」只是读取侧临时切窗（user.message → final），不进存储、不进 DTO、不进前端状态。
[
  { "seq": 101, "kind": "message",   "role": "user",      "text": "…" },
  { "seq": 102, "kind": "notice",    "what": "opening",   "text": "…" },
  { "seq": 103, "kind": "response",  "port": "model",     "id": "m-7", "attempt": 1 },
  { "seq": 104, "kind": "request",   "port": "tool",      "id": "t-3", "name": "web_search" },
  { "seq": 105, "kind": "response",  "port": "tool",      "id": "t-3" },
  { "seq": 106, "kind": "message",   "role": "assistant", "text": "…", "status": "final" }
]
```

- **取消**：abort 链路保持「产出无论完成/失败/被中断都入格」（chat_loop.rs:297-308）；
  StopSignal→端口 abort（abort_round_trip）+ authz.denied；轮边界检查改端口批边界；
  终态 `aborted` 语义不变（t7 断言不动）。
- **重试**：(port, id, attempt) 幂等键 + 租约 reap 已 turn 无关——不动。
- **并发**：并发单位 = 端口批（e2e-concurrency 不动）。
- **未执行收口**：批 join 关账，`NOT_EXECUTED` / `ABORTED` 成因词表不变。
- **汇报**：report 行 due 判据本就按事实计数（D-163），改 scope 口径即可。

## 四、分批计划（与 M3 / A / B / C 顺序）

| 批 | 内容 | 验证读数 |
|---|---|---|
| T0 冻结（即刻可提） | turn 词根门禁（`turn-root-audit.mjs`）：覆盖 loop / sdk / session / 前端 / DTO，基线钉存量**只许降**；禁新增 turn 概念 | gate --full 全绿；基线数入档 |
| A 批 = M3（不变，[T-031] §六未落，随裁决一并落） | 工具批形状合同 + tools/** 迁出 + B4/B5 入库 | e2e 96/96；EL-003 8 只降；EL-001 16 不动 |
| B 批（改向：原「turn 骨架搬 session」作废） | B1 事实/视图 scope 化（改名 + Entity 裁决落地 + e2e 断言面迁移）；B2 编排删除（turn_exec/turn_runner 直线编排退场、TurnState 消解、判决/唤醒进行判据）；B3 展示拆除（DTO/帧 meta/根骨架/「第 N 轮」/rounds 投影/快照测试更新） | 每步 e2e 96/96（断言面按批更新、行为零差）；CS-010 TurnState 格数只降归零；EX-003 净删 |
| C 批（泛化收口，吸收原 [T-030] Step 1） | PORTS 迁出注入、ROWS 参数化、`port_of(name)`、`await-{port}` 统一、纯度门禁全量（turn 词根 0、tool 词根归零、字面量面 0） | e2e 96/96；EL-001 16→14；EL-003 只降 |

- 顺序理由：A 先立端口独立的前半（工具批形状），B 的「判决进行判据」才有稳定的端口
  行可依；C 在世界语义与 turn 清零后泛化，类型面基线一步归零，无空转。
- [T-030] 由本档承接关闭（Step 1 泛化并入 C、Step 2 搬家作废）；[T-031] 的批 B 定义
  由本档 §四 替换。B4/B5 未提交改动仍留工作区，随 A 批入库。

## 五、T0 冻结门禁（可审稿）

### 5.1 判据面（只冻 turn 词根，不含展示豁免）

扫的是**标识符与字面量**，不扫注释正文（注释由 D-016 管）与 `docs/**`（文档是「为什么」
的 owner，允许叙述历史）。五个面各自有生产 / 测试两组计数，**生产面严格冻结、测试面只许降**：

| 面 | 扫什么 | 范围 | 首跑实测（生产 / 测试） |
|---|---|---|---|
| F1 类型/函数 | `Turn[A-Z]…`、`turn_[a-z_]…`、`TURN_[A-Z]…`、裸 `turn` 标识符 | `crates/**/*.rs` | 1508 / 1303（732 文件） |
| F2 事实/视图/行名 | `"turn.<x>"` 字面量、`Entity::Turn`、`FinalizesTurn`/`StopsTurn`、`turn_head`/`turn_closed`/`turn_slice`、`close-turn`/`prepare-turn`/`continue-turn` 行名 | `crates/**/*.{rs,yml}`（含 PLUGIN.yml 与生成物） | 193 / 564（764 文件） |
| F3 DTO | Rust 命令层 `turn:` / `turn?` 字段 | `crates/symbio/**/*.rs` | 0 / 0（3 文件） |
| F4 前端 | `.ts/.vue` 中 `turn` 标识符、`meta.turn` | `tauri/src/**`（含 `__tests__`） | 4 / 12（190 文件） |
| F5 迁移通道 | NodeKind 旧词折叠项 | `tauri/src/schemas/chat_node.ts` | 1 / 0（1 文件） |

初测（594 / ≥326 / 59）是 Grep 行数下限且含注释；脚本口径是**去注释后的匹配次数**，故
首版基线以脚本实测为准（合计 3585 处，见 `scripts/turn-root-baseline.json`）。
F3 为 0 是实测事实：`crates/symbio` 命令层的 `turn` 字段已清完，判据保留以防回潮。

### 5.2 判据规则（T0-1 … T0-6）

- **T0-1 生产面零新增**：生产代码出现任何 turn 词根标识符 / 字面量 / 行名 = 红，
  F3 / F4 与 F1 / F2 同判——**展示层没有豁免行**。
- **T0-2 棘轮只降**：任一面计数 > 基线 = 红；< 基线时**同批下调基线文件**，否则红
  （基线不随下降而下调 = 空转，照 `vitest.config.ts:74-76` 的同一条纪律）。
- **T0-3 迁移通道唯一且显式**：`chat_node.ts` 的 `turn: NODE_KIND_SKELETON` 与其读取
  分支作为**存量数据迁移通道**，须带 `// turn-migration-allow T-032: 存量清迁后删`，
  基线计 1，只许降到 0；除这一条外无任何 allow 行。
- **T0-4 待裁决内核格单列**：`Entity::Turn` 与两个角色名挂一条 `pending-decision`
  登记（指向本档 §二 裁决项），计数进 F2、裁决后随批删除；它不是豁免，不能新增引用。
- **T0-5 空扫必须红**：任一面扫到 0 文件或 0 匹配要报错（防「扫错目录以为全绿」）。
- **T0-6 allow 必须指向存在**：allow / pending 条目所指的行不存在 = 红（防陈旧条目）。

### 5.3 基线载体

`scripts/turn-root-baseline.json`：`{ faces: {F1..F5: {prod, test}}, allow: [...],
pending: [...] }`，由脚本读写（`--write-baseline`）。**不挂 `gate.d/_shared.mjs` 的
`BASELINE`**：那份是测试总数 / 用例数一类的向上棘轮，而 turn 词根是「必须等于基线」
的两向判据（高于 = 新增红，低于 = 收窄未下调红），装进「只增不减」的容器会判反。

- `allow`：唯一一条，`F5` / `chat_node.ts` / `turn:\s*NODE_KIND_SKELETON`，含理由与
  期限；`F5` 之外的 `allow` 一律红。
- `pending`：`Entity::Turn`、`FinalizesTurn`、`StopsTurn` 三条，理由「随 Entity 裁决
  改名」——是**待裁决登记**不是豁免（裁决后随批删除，登记即消失）。

### 5.4 首批提交清单（T0 一个提交）

1. `scripts/turn-root-audit.mjs`（五面扫描 + T0-1…6）；
2. `scripts/turn-root-audit.test.mjs`（形态照 `engine-ledger-audit.test.mjs`：改输入 →
   结论必须变，含零新增判红、基线下降未下调判红、空扫判红、陈旧 allow 判红）；
3. `scripts/turn-root-baseline.json`（首跑实测值）；
4. 挂载：`scripts/gate.d/30-docs.mjs` 的 `GUARDS` 增 `'turn-root-audit'`（静态审计的家，
   不新建阶段文件）。`GUARDS` 一项产生**两步**（回归测试 + 本体 `--strict`），故门禁
   物化项数 172 → 174（gate 汇总口径；稿中「173」只数了本体一步）；
5. 本档 §五入正文；A 批边界（[T-031] §六）**仍不落**。
验证：`node scripts/turn-root-audit.mjs` 绿、`.test.mjs` 绿、`node scripts/gate.mjs --full`
全绿；e2e / EL 读数不动（门禁不触代码）；回退点 = 该提交本身。

### 5.5 与三张档的关系

- **T-032（本档，伞）**：T0 是伞下第一批，只立门禁不动代码；A/B/C 仍待三项裁决。
- **T-030**：由本档承接**关闭**——Step 1 泛化并入 C 批，Step 2「搬 turn 骨架进 session」
  **作废**（turn 不搬）；未提交的 B4/B5 改名留工作区，随 A 批入库。
- **T-031 批 B 作废**：其「turn 骨架搬 session」整批撤销，改由本档 B1–B3 承接；
  T-031 仅保留 §一–§四 的审计与 A 批（工具域）定义。
- **与 A 批顺序**：T0 可与 A 批裁决并行入库（T0 零代码依赖）；A 批开工仍等 M3 裁决。

## 边界

- 本档零代码、零提交；A 批边界（[T-031] §六）继续暂停，待 M3 与本档一并裁决。
- `Entity::Turn` 改名属内核语法面（「内核语法与门面机制面」纪律）：**单列裁决项**，
  裁决前 B1 不开工；若判 b)（保留重释），门外按 T0-4 挂 `pending-decision` 登记
  并在裁决后删除，**不设展示/内核豁免行**。
- 存量转写数据（`type:'turn'` 节点）迁移通道的清迁时点在 B3 后单列，不阻塞本档。
- D-297 压缩链、tokenizer、report 行判据（D-163）不动语义，仅口径词根随批改。

## 处理过程

1. 两条用户裁决入档；T-031 exit 字段已注记 A 批暂停、本档优先。
2. 三路勘察（loop 机制面 / loop 外消费方 / 展示层）+ 主会话定向 grep（e2e 断言面、
   前端 turn 词根、session 投影），file:line 双核对。
3. 逐项映射、无 turn 转写形状、分批计划落 §二–§四；结论与回复 ≤30 行同口径。
4. 待办：用户裁决（M3 + Entity 方案 + 分批顺序）后，T0 门禁先行提交，A 批边界补落
   [T-031] §六 再开工。
