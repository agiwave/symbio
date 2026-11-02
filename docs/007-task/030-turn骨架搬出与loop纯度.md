---
kind: task
id: T-030-turn骨架搬出与loop纯度
title: turn 骨架搬出 session + loop 纯度常设门禁（含 B4/B5 补迁）
status: doing
track: arch
prio: now
created: 2026-11-02
parent: T-026-loop耦合审计
deps: []
where: "crates/symbio-plugin-loop/src/{chat_loop,turn_session.rs,provider.rs,turn_runner.rs,stage_events.rs,control_decl.rs,loop_rows.rs,port.rs}, crates/symbio-plugin-session/src, scripts/{loop-purity,plugin-entry}-audit.mjs"
exit: 两步提交，每步 e2e 96/96 逐字不变。Step 1 泛化批：B4 泛化留环（`ToolDecls`→`BehaviorDecls`，口径逐字）、B5 `record_effect` 留环签名随改；PORTS 迁 session 装配侧经 loop 入口注入（kernel `PortDecl` 零改动，级联/批次开关住装配载荷）；ROWS 六行参数化 `authorize/announce/dispatch/open/deny/await-{port}`（实例化后 16 行同值，EL-001 16→14 基线随降）；`control_decl` 配对参数化为「请求的发起端口」；纯度门禁落地（字面量面基线 0 绝禁、类型面基线钉存量只许降、压缩链 D-297 显式例外）。Step 2 搬家批：C 类 8 项迁 session（TurnState 格数只降，CS-010 重指向；EX-003 同提交删旧实现；门禁基线只降）；B8 复核落裁决；EL-002 1 不变、EL-003 8 只降；EX-007 零新增行
---

# turn 骨架搬出 session + loop 纯度常设门禁（含 B4/B5 补迁）

## 内容

[T-026] §四批次表的末批：C 类（turn / session 骨架）从 loop 搬 session，loop 只剩
D 类环机制 + 端口派发面；同时把「loop 纯度」立为常设结构门禁。**先报边界、经用户
确认后再落代码**——本文档即边界稿。

## 形状（逐项落点）

### C 类 8 项

| # | 现位置 | 落点 |
|---|---|---|
| C1 | `chat_loop/state.rs` `SessionContext`（消息列表 + LoopSession + 压缩寄存） | session（会话历史管理） |
| C2 | `chat_loop/state.rs` `TurnState` | session；格数不删（CS-010 只许降，读数对象随归属重指向） |
| C3 | `turn_session.rs` `LoopSession` | session（会话身份与事实包） |
| C4 | `provider.rs` `run_turn` 入口（`LOOP_RUN_TURN` 动作、装配 `ChatOrchestrator`） | session；loop 只被调用 |
| C5 | `turn_runner.rs`（begin/settle/close/`await_model`） | 会话半边（begin/settle/close、`chat.assistant.final`/`fallback` 落格）迁 session；`await_model` 的 poll 骨架是 D 类，留环并统一为 `await-{port}` 行动作 poll 在飞 future（去 model 名） |
| C6 | `chat_loop/{turn,wake,decide,delegate,compose}.rs`（轮首判决、唤醒 resume 幂、委派、措辞） | session |
| C7 | `stage_events.rs`（`turn.stage` 事实） | session |
| C8 | `control_decl.rs` 会话词汇（`turn.dispatch` 等） | 拆出迁 session；求解器声明主体（PTS-003 判据面）留 loop |

### B4 / B5 落点：泛化留环

| 项 | 现名 / 位置 | 裁决 |
|---|---|---|
| B4 | `ToolDecls`（`tool_executor/decls.rs`） | **(a) 泛化留环**：改名 `BehaviorDecls`（不用 `PortDecls`——与 kernel `PortDecl` 撞名）；`of_metas` / `behaviors_of` 的「等值再末段」口径逐字迁 |
| B5 | `record_effect`（`tool_executor/effects.rs`） | 名字无词根问题，留环；签名 `&BehaviorDecls`，逻辑逐字（模块自述「不认识任何工具」） |

- 消费者 `batch.rs` / `gates.rs` / `unfinished.rs` 同 crate 改名即用——零回读、零代理、
  零新 crate 边。
- **不选 (b) 迁 session**：① E-009 禁 loop→session 兄弟 crate 边；② 经端口 / 注册表
  回读 = 批首读数变运行时往返（时延源 + 判重面漂移）；③ 代理已被 [T-026] §四裁决
  排除。合同面（`EffectInput` / `ToolBehavior`）本就住 plugin-api，loop 内剩的是**读数
  与记账助手**——泛化留环即终态，登记为对 B 表「tool 端口合同」落点的偏差修订。
- 批首 `tool_metas` 读数随批编排（D 类）留环，sdk 函数 import 进纯度门禁基线白名单。

### ROWS 逐行表（16 行）

| 行 | 处置 |
|---|---|
| `authorize-tool` | 参数化 `authorize-{port}`：由装配侧端口属性实例化（仅 tool 声明授权级联，[T-026] §二「级联是端口属性」） |
| `policy.classify` / `policy.compress` / `policy.recall` / `policy.report` | 留环（无端口词；`policy.compress` 挂 D-297 显式例外注） |
| `prepare-turn` / `close-turn` / `continue-turn` | 留环（turn 非端口词；轮次选债行属 D 类） |
| `announce-model` | 参数化 `announce-{port}`（视图面行） |
| `dispatch-model` | 参数化 `dispatch-{port}` |
| `open-tools` / `deny-tools` | 参数化 `open-{port}` / `deny-{port}`（批次语义行；仅 tool 声明批次） |
| `dispatch-tool` | 参数化 `dispatch-{port}` |
| `await-model` / `await-tool` | 参数化 `await-{port}`（统一等待行；C5 的 poll 骨架去 model 名后的行侧形状） |
| `artifact.added`（归宿行） | 留环；`by: WAITER_TOOLS` 绑定名随 `await-{port}` 参数化 |

- 静态模板 14 条（4 policy + 3 turn + 6 端口模板 + 1 归宿），实例化后仍 16 行、
  行名同值（端口名由装配侧填入）⇒ 事实面 / e2e 逐字不变；`EL-001` 16 → 14，基线随降。

### PORTS 表与 `ports::` 常量如何从 loop 消失

- `ports::{MODEL,TOOL,KNOWN}` 常量已住 plugin-api（T-024/T-028 已做），**不迁**；
  消失的是 loop 内的**引用**：`port.rs` 的 `model_port`/`tool_port` 读数助手改通用
  `port_of(name)`（端口名由调用侧传，调用侧随 C4 已在 session）；`control_decl.rs`
  的 `str_field(e,"port")==Some(ports::…)` 配对（约 10 处）参数化为「请求的发起端口」。
- `PORTS`（`PortDecl` 装配配置 + 新增的级联 / 批次开关）迁 session 装配侧，经 loop
  公共入口载荷注入；kernel `PortDecl` 零改动（开关住装配载荷，不进内核数据面）；
  dispatch / reap / announce 收注入表。行表 `ROWS`（选债）是 D 类，留环。

### loop 纯度门禁（常设，非一次性清理）

新判据（`loop-purity-audit` 或并入 `plugin-entry-audit` E 系），形态与 `EL` / `CS`
棘轮同构——基线只许降、变宽即红，两面分工：

- **字面量面（step 1 清零，基线 0，绝禁）**：端口行名字面量、端口名字面量、
  `ports::` 常量引用、按端口名的分支（`== ports::MODEL` 一类）。
- **类型面（基线钉存量，只许降）**：`Model` / `Tool` / `Turn` 词根类型名全 crate 判，
  基线三族——tool_dispatch 批编排族（`ToolPortCall` / `ToolRisk` 等，owner D 类）、
  kernel 合同 import（`TurnToolCallInfo`）、C 类 Turn 族（step 2 搬走后降）。
- 测试与文档注释的豁免口径与 E-013 一致（测试另放行）。

### 两步提交

- **Step 1 泛化批**：B4/B5 泛化改名、PORTS 迁出注入、ROWS 六行参数化、
  `control_decl` 配对参数化、`port.rs` 助手 `port_of(name)`、`await_model` poll
  去 model 名、纯度门禁落地。e2e 96/96 逐字不变；EL-001 16→14、EL-003 只降。
- **Step 2 搬家批**：C1–C8 迁 session，EX-003 同提交删旧实现，CS-010 重指向，
  门禁基线只降（Turn 族清零）。
- 可拆理由：step 1 后 loop 已端口无感知，step 2 纯搬家（行为零差）；唯一交叉点
  `await_model`（step 1 改、step 2 搬，先改后搬合法）；回退单位是 git 提交，
  两批各自全绿各自可回退。

### 压缩链：D-297 显式例外

纯度门禁的「按能力名分支」判据为压缩链挂**显式例外**（owner D-297：压缩执行体
留控制面——触发与呈现）。重开条件写进例外注：①第二个长过程消费者；②触发改成
事件等值。合法动作是发起架构讨论重开 D-297，不在迁移批顺手下沉。

### B8 复核

`context/{prompt,conversation_view,providers,view}.rs` + `tool_executor.rs` 内
`build_user_prompt_message`：本批复核归属——随 C 搬 session，或另立 B 下沉批
（落点 = model 端口合同扩展或对话面草稿，D-293 形状）；结论落「裁决与偏差」。

## 验证读数（预期）

- e2e 96/96 逐字不变（两步各自全绿；行名实例化后同值，事实面不动）。
- EL-001：step 1 后 16 → 14（六行端口模板参数化，静态 Decl 数降；实例化行数
  不变）；EL-002 1 通道不变（派发通道留环）；EL-003 8 只降不升。
- CS-010：`TurnState` 格数只降（搬家不删格），读数对象随归属重指向。
- EX-003：loop 行净删、旧实现同提交删除（`Migration:` trailer）；EX-007 零新增行。

## 裁决与偏差

- **B4/B5 未随 T-029 迁走**：B 表原判 B1–B5 同批，实际只迁 B1–B3——B4/B5 的消费者
  是留环的 batch / gates / unfinished，单独迁会立刻造出「环经代理回读」的形状；
  补迁随本批（纯度门禁落地的前置），在此登记对平。
- B8 复核结论（随 C 搬或另立 B 批）施工后落本节。

## 边界

- 本批不动 D 类环机制（选债 / 派发 / 等待 / 配对 / 记账 / 预算租约 / 重放）。
- 压缩链 / tokenizer 不随批（[T-026] 口径裁定一 / 二，D-297 终态）。
- 边界经用户确认后才落代码；确认前的修订只改本文档。

## 处理过程

1. 对平 B 表账：查证 B4（`ToolDecls`）/ B5（`record_effect`）仍在 loop
   （`tool_executor/decls.rs` / `effects.rs`），消费者为留环的 batch / gates /
   unfinished；未随 T-028 或 T-029 迁走，登记补迁随本批。
2. C 类 8 项、PORTS / 常量去向、纯度门禁判据、D-297 例外、预期读数逐项落
   「形状」节——边界稿，待用户确认后施工。
3. 按用户裁决补两张表：B4/B5 落点（泛化留环，`BehaviorDecls`；不选迁 session 的
   三条理由）与 ROWS 逐行表（六行参数化、模板 14 / 实例化 16）；采纳两步提交
   （泛化批 → 搬家批），可拆理由落「两步提交」节。
