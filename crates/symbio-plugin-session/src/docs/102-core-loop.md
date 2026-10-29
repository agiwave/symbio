---
kind: design
id: DE-102-core-loop
title: 会话核心循环（LLM × 工具）
status: active
created: 2026-09-16
impl: done
---

# 会话核心循环（LLM × 工具）

> **目标形状**见 [DE-001 §4.3 / §4.4](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
> 与 [DE-015](../../../../docs/003-design/015-义务环.md "DE-015-义务环")。
> 本文只写**当前实现**，不复述目标裁决。
>
> 范围：主循环本体住在 `crates/symbio-plugin-loop/src/`（驱动器插件）；一轮的**装配与收尾**
> 留在 `crates/symbio-plugin-session/src/orchestrator/`（存储与生命周期）。
> 本文只回答一个问题：**主循环的结构是什么**。「压缩策略有哪些」见 [`../README.md`](../../README.md "crates/symbio-plugin-session/README")（五大压缩策略）；节点状态与流见 [`./018-node-state-streaming.md`](105-node-state-streaming.md "DE-105-node-state-streaming")。
> 模块拆分见 [`./017-module-layout.md`](104-module-layout.md "DE-104-module-layout")；驱动器形状见 [`../../loop/README.md`](../../../symbio-plugin-loop/README.md "crates/symbio-plugin-loop/README")；设计理由见 [`000-架构决策汇总.md`](../../../../docs/003-design/README.md "DE-000-设计总览")。
> 代码位置一律按**符号名**检索——模块拆分后行号会漂移；下文路径未写插件前缀时，①/② 在 `crates/symbio-plugin-session/src/`，③/④ 在 `crates/symbio-plugin-loop/src/`。

---

## 0. 一句话结论

主循环 = **前步骤**（请求解析 → 开会话 → `TurnState` → resume）+ **闸门** + **唯一一个环**
（`turn_exec::drive`：开幕 → 跑幕 → 收幕 → 接幕）+ **唯一收尾点 `finish_turn`**。
`chat_loop::run_chat_loop` 本身**没有环**；所有出口都经 `return finish_turn(...)`，没有旁路。

**幕**不是独立的驱动单位：它的三段（幕首 / 幕内 / 收幕）都在 `turn_exec::drive` 里，幕首与
收幕是那个唯一循环体的两端（`open_stage` / `close_stage`）。**「还开不开下一幕」读事实**
（`turn_next_round`），不由任何返回值决定——再开一段是一笔 `continue-turn` 债（`D-222` 裁决 2），
由同一个环按 `prio` 选中；那一行的出口（新落的 `round` 格）同时就是下一幕的锚与 `now`
（判据 12：一幕一个锚）。控制面因此只有一个 `loop {`（`D-222` 裁决 5 / 判据 8）。
模型往返与工具批次**也是这张表上的行**（`announce-model` → `dispatch-model` → `await-model`，
`open-tools` → `authorize-tool` → `dispatch-tools` → `await-tools`），幕内没有第二段小循环（§4.1）。

五个收口点 + 一个状态容器（§2）：

| 收口项 | 职责 | 唯一性 |
|---|---|---|
| `gate_turn` | 退出条件（只剩中止边界），见 §2.1 | 判定点唯一 |
| `collect_reads` / `prepare_request_sections` | 环前纯读一次取好系统提示词 + 工具（同机制、同时机）+ 请求级开销；轮首那几行跑完后由接缝行 `prepare-turn` 重建请求视图（压缩 / 判决 / 召回 / 快路的出口格是它的前件） | 收集点唯一 |
| `run_compress`（行 `policy.compress` 的执行体） | 水位响应（70% 阈值触发的语义压缩）：判定与执行同处这一执行体，由轮首派发点选中（[DE-015](../../../../docs/003-design/015-义务环.md "DE-015-义务环") 的 D-159） | 响应点唯一 |
| `turn_exec::drive` | 幕首 / 幕内 / 收幕三段与接幕判据：一个循环体、一张绑定表，`Ctx` 按步借、按步还，见 §1.2 | 控制面唯一的环 |
| `finish_turn` | 增量落库 + Stop 钩子 + 返回语义 | 收尾点唯一 |
| `TurnState` | 跨幕可变状态（请求轮次 / 落库锚点 / 静默时钟种子 / 中止信号 / 读侧产物） | 状态容器唯一 |

---

## 1. 调用链全景

### 1.1 四层结构

```text
① 装配层 start_turn session: orchestrator/entry.rs
 收件箱消费者取批后调进来（或恢复动作直接调）；解析 session_id / mode / risk / provider
 用户消息落库（Carrier::Runner）+ 自动命名 + chat_ctx / TurnDispatch 装配
 collect_capabilities(traverse) → CAPABILITY_VISITOR（本层**不**拼提示词）
 派发 loop/<sid> run_turn，结局由 session: orchestrator/consume.rs 等处消费
② 宿主层 run_turn（动作体） loop: provider.rs
 取 TURN_DISPATCH（授权三条）→ LoopSession + ChatOrchestrator；sink / abort 随交付包传入 → await ③
 结局序列化为 TurnReport 交回；session 侧 select! 看门狗 / 接管三臂在 ①
③ 循环层 run_chat_loop + gate_turn / finish_turn chat_loop.rs（本层没有环）
 + turn_exec::drive（唯一的环：幕首 open_stage → 循环体 → 收幕 close_stage → 接幕）turn_exec/drive.rs
 + continue-turn 的开幕半边 begin_drive / continue_drive chat_loop/drive.rs
 + collect_reads / run_compress ／ 接缝与执行体 run_seam / run_report chat_loop/inputs.rs · turn_exec.rs
 + settle_reasoning / persist_turn chat_loop/turn.rs
 + auto_compress_process / retry_compaction / compress_with_snapshot_core context/pipeline.rs
 + 会话侧读写经动作面 session_port.rs
④ 执行体层 turn_exec 的各行动作 turn_exec.rs
 + turn_runner/{prepare,stream,batch,boundary,settle}（各行的执行体动作在这里）turn_runner/
 + SessionDispatchPort::dispatch tool_dispatch.rs（三动作：open / authorize / dispatch）
 + process_tool_calls_async tools/tool_executor.rs
```

**职责边界**：③ 不持有终态落库职责——终态唯一落库点在 session 收尾（② 的消费循环与
`transcript/publish.rs`）。④ 只负责**这一轮**的事实与产物，不决定会话接下来跑不跑
（那一格由 ③ 的环读 `turn_next_round` 折出，`D-222` 裁决 2）。
后步骤**不存在**：循环只能由 `return` 离开，所有出口都经 `finish_turn`。

### 1.2 主循环：一个环，三段

**前步骤**（环之前）

| # | 动作 |
|---|---|
| P1 | 解析 `model_chat::Request`，构造请求级快照 `TurnRequest::new(&req)`（默认值唯一真源是 `SessionConfig`） |
| P2 | `open_chat_session`（取 `SESSION_HANDLE`，缺则回退内存会话） |
| P3 | `TurnState` 初始化（含 `abort: ExecAbortSignal`）+ `SessionContext` 构造 |
| P4 | **resume 处理**（`process_wake`）→ `Woken` / `NewTurn` / `Done` / `Err`，`Done`/`Err` 经 `finish_turn` 提前收尾 |
| P5 | `gate_turn`（收口 ①）：`Exit` → `finish_turn` |

**环**（`turn_exec::drive`；`chat_loop` 只调用它一次）

| # | 动作 | 内容 | 段 |
|---|---|---|---|
| 1 | 幕界判读 | `retiring` 有值（上一幕已收束、`continue-turn` 未跑）时判中止；`continue-turn` 落地后幕锚变了，旧幕才让位 | 幕间 |
| 2 | `open_stage` | 上下文 → 开一幕的格（`begin_drive`：唤醒格 / 收租 / 配置 / 幕锚 / 轮次格）→ `collect_reads` → 定根 → 装配 → `open_turn`；续幕的格已由 `continue-turn` 落过，不再落第二份 | 幕首 |
| 3 | 选债 → 一个动作 → 读账本 → 五道守卫 | 轮首四行 / 接缝 / 汇报 / 模型往返 / 工具批次同在**一张绑定表**上，按 `prio` 选最小可还债 | 幕内 |
| 4 | `close_stage` | 归类 → 派生事实 → 镜像并入 → `settle_reasoning` → `persist_turn` → `finalize_turn_root`（封根，全流程唯一一处） | 收幕 |
| 5 | 接幕判据 | `turn_next_round` 有值 ⇒ 留下旧幕等 `continue-turn`，否则 `DriveEnd::Completed` | 幕间 |

三段（**不是三笔账**——幕不是端口，`turn` 没有一行可以欠）：

| 段 | 入口 | 做什么 |
|---|---|---|
| 幕首 | `open_stage` | `collect_reads`（提示词 + 工具 + `overhead_tokens`）+ 定根 + 装配 + `open_turn`。接缝行 `prepare-turn`（`run_seam`）随后跑压缩 / 判决 / 召回 / 快路与 `build_request_view`。判成答话的一轮**在环内就地收束**（这一幕没有输入也没有往返） |
| 幕内 | 循环体 | 每圈一笔债：一个执行体、一次账本对账、五道守卫。模型往返经 `SessionDispatchPort` 三动作分发，中止检查点在运行器内部 |
| 收幕 | `close_stage` | **就地收好产出**：镜像并入 → 中止复查 → `settle_reasoning`（定稿 + 校准 + 并入上下文）→ `persist_turn`（增量落库）→ `finalize_turn_root`（封根） |

> **为什么三段不各是一笔债**：一次幕的借用面是**跨段**的（`&mut TurnState` +
> `&mut SessionContext` 在整幕期间都在手上），做成行就要给每一段各找一个「什么算欠」的
> 前件，而那些前件只能是「上一段行了没有」——那就是把 Rust 的借用面升格成调度结构。
> 幕是信封上的分区（`D-222` §二），不是推进单位；三段的次序由代码行序说，不由 `prio` 说。
>
> 幕**内**那些**本来就以事实为前件**的东西相反：轮首四行、接缝、汇报、模型往返与工具批次
> 各有自己的视图（`turn_head` / `report_due` / 端口），它们全是同一张表上的行，由同一个环
> 按 `prio` 选中。

**`persist_turn` 的分支**（落库是**写投影**，不是债）

| 条件 | 动作 |
|---|---|
| `finish == length` 且本幕轮次未到 `MAX_STAGE_ROUNDS` | 截断文本已在上下文里，下一幕由 `continue-turn` 续写（轮次由 `turn_next_round` 给出，见第 5 步）；增量落库 |
| `finish == length` 且轮次已到上限 | 会话级告警（提高单次输出预算 / 缩小任务范围），绝不静默结束；增量落库 |
| 其余 | 增量落库 |

> **这里没有工具执行，也没有压缩拦截**：工具跑在 `turn_exec::drive` 的内环里（§4.1），
> 到收幕时本轮工具早已产出结果；水位响应在轮首派发点选中的
> `policy.compress`（接缝段）。
> `persist_turn` 因此不读 `TurnOutput`——它只读 `ModelFinishReason`。

### 1.3 职责边界（收口依据）

- **收尾三件套**（增量落库 / Stop 钩子 / 返回语义）只在 `finish_turn` 出现一次；
 每个出口只负责构造 `TurnExit`。
- **压缩判定与执行**同处一个执行体（`run_compress` = 行 `policy.compress`）；它与召回的先后由
 `prio` 表达，不由派发点的代码行序保证（[DE-015](../../../../docs/003-design/015-义务环.md "DE-015-义务环")）；
 内核 `compress_with_snapshot_core` 是唯一实现。
- **提示词与工具**必须同机制、同时机收集：两者都经 `Registry`（收集口）注册、经 `traverse`
 汇集到同一个 visitor 的「会话输入」，来源与生命周期一致。
- **「不完整不唤醒」由行表的前件守住**：下一笔 `announce-model` 只在上一批结果全部接进
  `exchange` 之后才成立（§4.1），不在闸门里判在途工具。
- **工具执行**归运行器（`turn_exec::drive` 的内环经 `SessionDispatchPort`
 分发）；`persist_turn` 只做「增量落库 + 截断告警」，不承担工具执行（见 §4.1）。
- **流向不由返回值决定**（`D-222` 裁决 2）：幕停下之后是收工还是再开一幕，读的是
 `turn_next_round`（收束格的 `reason` 与本幕轮次的合成视图）。执行体交回的只有一件事——
 「这一幕跑完了没有」；接幕由 `continue-turn` 那一行按同一条视图开账。

---

## 2. 收口点

### 2.1 收口 ①：退出条件 → `gate_turn`

```rust
/// 本轮是否必须退出（退出条件）。唯一调用点：主循环顶部。
enum Gate {
 Proceed, // 放行 → 进入准备与推理
 Exit(TurnExit), // 中止边界
}

fn gate_turn(turn: &TurnState) -> Gate
```

**形状只有两态**（放行 / 退出）：「工具没跑完」不是退出条件——工具在
`turn_exec::drive` 的内环里一批批跑完（§4.1），不存在横跨轮次的在途集合，
于是这一态与它的簿记一起退场（裁决与被删符号名册见
[D-073](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")）。

「不完整不唤醒」没有废弃，它换了承担者：**结果接进 `exchange` 之后才可能开出下一笔
`announce-model`**（`await-tools` 那一行的出口，§4.1）。因此——

> ⚠️ **「历史中所有 ToolCall 都有结果子节点」从来不是判据**：**「ToolCall 无结果」在本代码库是
> 合法状态**（交互模式审批中断时本批剩余 ToolCall 有意留空，由请求视图层 `flatten_chat_messages`
> 合成占位结果喂回模型），按历史判定会让 approve/reject 恢复后的续写被**永久挡住**。
> 真正会悬空的只有一种：id / name 非法的调用——`record_protocol_failure` 在写入时合成失败结果
> 兜住它（§4.2），不需要主循环侧的等待。

退出条件只剩**一处**（`gate_turn`，环之前）：

- **环的幕界守卫** → `DriveEnd::Boundary` → `TurnExit::AbortedAtBoundary`（**不冒泡**
 `Err(Aborted)`：上一轮 Turn 已定稿落库，冒泡会让消费循环的 `persist_failure` 把已成功的 Turn
 误标为 `Aborted`）。判据是 `retiring.is_some() && abort`——`retiring` 是「上一幕已收束、
 `continue-turn` 未跑」的唯一标记，只有那一圈才是幕界。整次驱动的**第一圈**中止由运行器
 在发第一个请求前落中止收束格（`abort_before_first_request`），不是闸门；推理前后
 （在途 Turn 尚未定稿）的中止才走 `TurnExit::Aborted`。

> 闸门**不判工具轮数**：`max_tool_rounds` 那条软上限整条退役——它判的是 `TurnState` 上一个
> 恒 0 的计数（§4.1；裁决与逐符号名册见 [D-074](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")）。**真要这道上限，
> 位置在 `await-tools` 收尾之后、下一次 `announce-model` 开账之前**：那里有真实的 `tool_round`，
> 而要新写的不只是判定，还有「到上限时这一批怎么收」的收束语义。

### 2.2 收口 ②：提示词 / 工具 / 请求视图 → `collect_reads` + `prepare_request_sections`

**硬约束：系统提示词与工具信息必须同机制、同时机、始终一起收集——在每轮 Turn 开头、
循环内收集。**

```rust
/// 环前的**纯读**：不选债、不落格。
struct TurnReads {
    system_prompt: String,          // resolve_system_prompt 唯一真源
    tools: Vec<CapabilityMeta>,     // 本轮到模型的完整工具清单
}

async fn collect_reads(
    ctx: &Arc<dyn PluginInvokeRequest>,
    req: &TurnRequest,
    turn: &mut TurnState,
) -> TurnReads;

/// 接缝行的装配：轮首那几行跑完之后、任何请求之前。
async fn prepare_request_sections(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    sink: &ExecEventSink,
    turn: &mut TurnState,
    root_id: &str,                  // 幕首分配（`begin` 的输入）
    fresh_root: bool,               // 续写幕复用原根，不重发骨架
    tools: &[CapabilityMeta],
) -> Result<Vec<ChatNode>, ActorError>;  // 请求视图（不落库）
```

- **两段分开的原因**：提示词与工具是纯读，必须早于草稿（`begin` 要用工具清单建）；
  请求视图却依赖轮首那几行寄存的值（召回寄存的段、快路寄存的正文），只能在
  接缝里建。`root_id` 也是幕首定的——「定 id」与「把骨架发给流式桥」是两件事。
- `retention` **不进**请求视图的返回值——它只是 `build_request_view` 的一个入参，
  由 `tools` 就地派生后立即消费，暴露到边界只多一份需要维护的中间态。
- `overhead_tokens` 由 `collect_reads` 一轮算一次并寄存进 `TurnState`，
 **一轮只算一次**（`context::estimate_overhead_with_tools(system_prompt, &tools)`
 复用已收集的工具清单，不让 visitor 注册表被查三遍）。

每轮收集本身**不触发 I/O**（`Registry::list` 的工具行 / 提示词行读取均为纯内存读
visitor 注册表），因此「每轮动态可调」是净收益：人格与工具集始终反映**当轮**注册表，
中途新增的能力（如 MCP 新连上服务端）下一轮即可。

### 2.3 收口 ③：压缩 → 行 `policy.compress` 的执行体

```rust
/// 收口 ③：上下文水位的**唯一响应点**——判定与执行同处这一执行体。
pub(crate) async fn run_compress(
 orchestrator: &ChatOrchestrator,
 ctx: &Arc<dyn PluginInvokeRequest>,
 store: &Arc<dyn Store<Event = Event>>, // 出口那一格由它自己落
 context: &mut SessionContext,
 turn: &mut TurnState,
) -> Result<(), ActorError> {
 // ① 阈值触发的语义压缩（70%；就地改写 context.messages 并推进落库锚点）
 …
}
```

**判定与执行同处一地**：水位判定与压缩执行都在这一个执行体里，压缩之后水位即变，下一轮按新
水位判。**「什么时候轮到它」也不再是代码行序**：它与判决、召回、快路同处唯一那张绑定表，由
`ring::drive` 按 `prio` 选（判决 10 → 压缩 30 → 召回 40 → 快路 50 → 接缝 52），出口那一格
（`turn.stage{stage=compress}`）由执行体自己落
（形状与理由见 [DE-015](../../../../docs/003-design/015-义务环.md "DE-015-义务环")）。

**收口后的分工**：

- **水位响应** → 唯一执行体 `run_compress`（行 `policy.compress`，由轮首派发点选中）；
- **执行内核** → `compress_with_snapshot_core`（唯一实现）；
- **收益护栏** → `context::has_compaction_payoff`（门槛语义只有这一个出处）；
- **「何时压」是一个入口两种触发**：阈值自动（`threshold`）与用户点失败节点重试
 （`retry`，`force`）。被合并的是「怎么压」（1 个实现）与「水位怎么响应」（1 个入口）；
 模型主动调用的压缩工具不存在（裁决见 [D-072](../../README.md "crates/symbio-plugin-session/README")）。

`force` 是 `should_start_compression` 的公开契约且有单测背书——它越过迟滞并绕过熔断，
只由 `retry_compaction` 传入。

### 2.4 收口 ④：退出 → `TurnExit` + `finish_turn`

```rust
/// 主循环的唯一退出原因。
enum TurnExit {
 Completed, // 正常收尾（无工具调用 / 待用户输入）
 AbortedAtBoundary, // 循环顶部的中止检查点：不冒泡
 Aborted, // 用户中止且在途 Turn 未定稿 → 冒泡 Err(Aborted)
 Failed(PluginError),
 ResumeDone, // resume 已完成，无需进入循环
}

/// 唯一收尾点：Stop 钩子（幂等）+ 增量落库 + 返回。所有出口只做一件事：构造 TurnExit。
async fn finish_turn(
 orch: &ChatOrchestrator,
 context: &SessionContext,
 sink: &ExecEventSink,
 turn: &TurnState,
 exit: TurnExit,
) -> Result<, PluginError>;
```

- `StopSignal` 的幂等 + RAII 兜底保留：显式触发点携带准确的「本轮最后一条消息」，RAII 兜底
 只在显式触发全部未发生时生效（panic / 消费循环超时 / provider 解析失败）。
- `persist_messages` 在锚点已对齐时是 no-op（切片为空即返回），因此所有出口都可以
 无条件调用，不会重复落库。
- `TurnExecExit::Aborted(partial)` → `TurnExit::Aborted(partial)` → `TurnReport { status: Aborted,
 partial }`，消费循环的 ABORTED 分支语义不变（半截产出由 loop 私有的退出类型携带，
 不住 `PluginError`——D-095）。

### 2.5 `TurnState`：跨轮可变状态

```rust
struct TurnState {
 abort: ExecAbortSignal, // 用户中止信号（推理与工具共享同一份）
 request_round: usize, // 本次运行已发出的 LLM 请求数 = 「这是第几幕」
 // ——由 `continue-turn` 的开幕动作从事实写回（`turn_next_round` 给出下一幕轮次），
 // 不是自己往上加的计数；⚠️ **不是**工具轮数（见 §4.1）
 last_saved: usize, // 增量落库锚点
 drive_seq: u64, // 本幕的锚：新轮 = 用户格 seq；续幕 = 新落的 `round` 格
 now_ms: i64, // 本幕的 `now`：幕锚那条事实的 ts（判据 12）
 // 其余是本轮的读侧产物：input_utterance / recall_view / skill_route / skill_hits /
 // context_sections / opening_text / opening_dialog / head_answer / head_reflex
 // （各自由对应的行在幕内填一次，收幕交给归类与落库）
}
```

`persist_turn` 的入参因此是 `(sink, context, turn, finish)`——它不需要编排器与请求快照，
只读本轮的 `ModelFinishReason`。

---

## 3. 主循环骨架

```rust
pub async fn run_chat_loop(
    orchestrator: Arc<ChatOrchestrator>,
    ctx: Arc<dyn PluginInvokeRequest>,
    sink: ExecEventSink,      // 事件唯一出口（见 §6）
    abort: ExecAbortSignal,   // 中止唯一入口（见 §6）
) -> TurnReport {
    // ── 前步骤 ①：请求解析与请求级配置快照 ──────────────────────────────
    let mut req: chat_request::Request = ctx.payload()?;
    let turn_req = TurnRequest::new(&req, …); // 默认值唯一真源：SessionConfig

    // ── 前步骤 ②：会话句柄（随交付包进来，不在这里开会话）─────────────────
    let session = orchestrator.session.clone();

    // ── 前步骤 ③：轮次状态 + 上下文容器 ─────────────────────────────────
    let mut turn = TurnState { abort: abort.clone(), ..Default::default() };
    let mut single_message = req.single_message.take();
    let mut context = SessionContext { messages: Vec::new(), session, consolidated: None };
    // 本轮输入锚定在 `single_message` 被消费之前（判决与委派段共用这一份）。
    turn.input_utterance = …;

    // ── 前步骤 ④：resume（一次性，可提前收尾）───────────────────────────
    if let Some(tr) = req.resume.take() {
        match process_wake(…).await {
            Ok(Woken { kind, root_id, .. }) => {
                turn.resume_root = Some(root_id);
                next_drive_resume = Some(kind);
            }
            Ok(NewTurn { retried_root }) => turn.retry_of = Some(retried_root),
            Ok(Done) => return finish_turn(&context, &sink, &turn, TurnExit::ResumeDone).await,
            Err(e) => return finish_turn(&context, &sink, &turn, TurnExit::Failed(e)).await,
        }
    }

    // ── 前步骤 ⑤：闸门（收口 ①：退出条件，唯一判定点）────────────────────
    if let Gate::Exit(exit) = gate_turn(&turn) {
        return finish_turn(&context, &sink, &turn, exit).await;
    }

    // ── 唯一一个环（`D-222` 裁决 5）：开幕 → 跑幕 → 收幕 → 接幕 ──────────
    //
    // 环在 `turn_exec::drive` 里：`open_stage` 开一幕；循环体按 `prio` 选一笔债、
    // 跑一个动作、读账本、过五道守卫；`close_stage` 收幕；收幕之后读 `turn_next_round`
    // 决定接不接下一幕。本文件不判「下一步是哪一步」，只把结局映射成 `TurnExit`。
    let driven = super::turn_exec::drive(
        super::turn_exec::DriveStart::Control(Box::new(super::turn_exec::Control {
            orchestrator, orchestrator_arc: &orchestrator_arc, ctx: &ctx, sink: &sink,
            context: &mut context, turn: &mut turn, req: &turn_req,
            single_message: single_message.take(), resume_kind: next_drive_resume.take(),
        })),
        super::loop_rows::ACTORS,
    ).await;
    let exit = match driven {
        // 判决判成「就此答话」：收束格由编排层就地落（`land_answered_turn`）。
        Ok(DriveEnd::Answered) => { land_answered_turn(…).await; land_inputs(…, "closed")?; TurnExit::Completed }
        // 幕间中止：上一幕已收束、下一幕没开（同一轮不落第二格收束）。
        Ok(DriveEnd::Boundary) => TurnExit::AbortedAtBoundary,
        Ok(DriveEnd::Completed) => TurnExit::Completed,
        // 夹具入口的结局只在测试里出现：控制面交下去的一定是 `Control`。
        Ok(DriveEnd::Bare(_) | DriveEnd::Running(_)) => TurnExit::Failed(内部错误(…)),
        Err(TurnExecExit::Aborted(partial)) => TurnExit::Aborted(partial),
        Err(TurnExecExit::Failed(e)) => TurnExit::Failed(e),
    };
    finish_turn(&context, &sink, &turn, exit).await
    // ── 后步骤 ──────────────────────────────────────────────────────────
    // 无：所有出口都经 `finish_turn`（唯一收尾点）。
}
```

---

## 4. 工具执行与批次边界

### 4.1 当前执行模型：往返与工具批次都是行上的债

**幕内没有第二段循环。** 一次「请求 → 工具批 → 再请求」的接力由同一张表上的行完成，
每圈一笔债、一个执行体（§1.2 的幕内段）：

```text
announce-model  60  宣布一笔往返：落 request(port=model)
dispatch-model  66  relay 执行体派出这一笔，落 sent
await-model     71  waiter 执行体等应答，落 response / failed
open-tools      67  按响应里的 calls 逐条落 request(port=tool)
authorize-tool   0  逐笔判权限：落 authz.granted / authz.denied
dispatch-tools  68  courier 执行体放行的落 sent
deny-tools      69  被拒的落 failed（收束格拒绝）
await-tools     72  批次收尾：产物格 / 等待进展格 / 交换累加 / 轮内注入
```

**「不完整不唤醒」由表本身守住**：下一笔 `announce-model` 的前件是视图 `next_round_trip`，
而它只在上一批的交换都接进 `exchange` 之后才成立（`await-tools` 的出口）——结果没接进
`exchange` 就没有下一次请求，主循环侧因此不需要一份在途工具集合（§2.1）。

阶段实现按 `turn_runner/` 切六件：`prepare`（草稿与提示词）→ `stream`（一次往返、批次收尾）→
`batch`（产物格）→ `boundary`（`classify_round` / `final_answer` 两个纯函数）→
`settle`（收束格：final / fallback / exhausted / aborted）→ `facts`（共享的视图读取）。
阶段之间只传 `Prepared`，不传第二份事实。

**这里数的是请求，不是工具轮。** `TurnState` 上那一格叫 `request_round`（本轮已发出的 LLM 请求数，
续写时才 +1）；工具轮数从**已落事实**折出：本幕带非空 `calls` 的 `response(port=model)` 条数
（[`report_due`](../../../symbio-core/src/decl/mod.rs) v6 的 `tool_rounds`，与 `Prepared` 里的活计数器
`tool_round` 同一个读数——前者的写方就是模型应答本身）。静默时长与配额同样由
`report_due` 算出，四格一起构成汇报行的 `due`。**按工具轮写的判据在这一层恒假**，而这类旋钮的默认值恰好也是「不限制」——
恒假与没配长得一样，没有信号能区分（裁决见 [D-074](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")）。

`persist_turn` 里**没有**工具分发路径：v1 外壳那段 `process_tool_calls_async` 分支的进入条件
（`TurnOutput.tool_calls` 非空）在 v2 恒不成立，随 [DE-014 步骤 5](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表") 删除；
`persist_turn` 只读 `ModelFinishReason`，工具的真实执行点只有 `await-tools` 那一行接的批次。

### 4.2 必须保留的语义

1. `interactive` 模式下「前一个工具待审批 / 失败 → 中止本批剩余」
 （`tools/tool_executor.rs`）——顺序语义；
2. `record_protocol_failure` 为 id / name 非法的调用合成失败结果，
 保证「没有悬空 ToolCall」这一不变式；
3. 工具执行的超时判据是**出口的发射计数**（`ExecEventSinkProgress`，挂在出口上——
 出口是「工具还活着」的唯一证据源，因为工具唯一的出方向动作就是 `emit`）：

 ```rust
 let progress = sink.progress;
 let mut seen = progress.emitted;
 loop {
 tokio::select! { biased;
 r = &mut route_fut => break r, // 工具已返回 ⇒ 立即采纳
 _ = wait_tool_abort(abort) => return …, // 用户中止
 _ = sleep(IDLE) => {
 let now = progress.emitted;
 if now != seen { seen = now; continue; } // 有进展 ⇒ 重新计时
 return …挂死…
 }
 }
 }
 ```

 于是同一个数字覆盖两种情形：不发事件的工具（`web_search` / MCP /
 `codebase_search`…）读数恒为 0，退化为「总时长上限」；会发事件的工具只在
 **真的沉默** `TOOL_EXEC_IDLE_TIMEOUT_SECS`（600s）后被杀。
4. 父节点终态快照（整条替换进内存镜像）与结果子节点的写入顺序。

---

## 5. 不得改动的机制（不变量）

- **压缩四层**（L0 守卫 / L1 自动 / L2 主动 / 输入超限预判）——预判层只跳过注定失败的
 请求，**不本地截断历史**（[`crates/symbio-plugin-session/README.md`](../../README.md "crates/symbio-plugin-session/README")）；
- **`compress_with_snapshot_core` 单一实现**（被动与主动共用内核）；
- **`StopSignal` 幂等 + RAII 兜底** 与 **`WorkingGuard` / `AbortGuard`** 的收敛保证（§6）；
- **`finalize_assistant_turn` 的 reasoning-only 分支**、**Turn 子树完整性守卫**
 （`find_compress_split_point`，provider 400 的直接防线）、**`feedback_estimate` 两条
 不变式**、**`tool_executor` 的超时与失败信息性语义**；
- **消费循环（`orchestrator/consume.rs`）的 `select!` 三臂与 `persist_failure` 只作用于
 `failing_turn` 子树**（终态落库唯一入口，不误回滚上一轮）。

---

## 6. 执行期出口 / 信号双原语

> 本节描述**宿主层与执行层之间的协议**：`ExecEventSink`（出）与 `ExecAbortSignal`（入）
> 是两个语义单一的原语；`PluginChannel` 只作**纯跨进程传输**，不承担执行期协议。
> 决策与理由见 [`docs/003-design/001-核心架构.md`](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")。

### 6.1 原语

| 原语 | 方向 | 形状 | 位置 |
|---|---|---|---|
| `ExecEventSink` | 出 | `Direct(Arc<dyn TranscriptWriter>)` \| `Null` | `symbio_core/exec/mod.rs` |
| `ExecAbortSignal` | 入 | `Arc<AtomicBool>` + `CancellationToken` 合一 | `symbio_core/exec/mod.rs` |

- **`ExecEventSink::Direct`** 直连转写唯一写入点（`TranscriptSink` → `Transcript::apply`），
 进程内**零 serde**；**`ExecEventSink::Null`** 是「本次调用不产生可见事件」的**类型级**表达
 （上下文压缩、`resume` 重跑工具用它）。
- **`ExecAbortSignal::abort`** 是唯一置位入口，置位即唤醒（`CancellationToken`），因此**没有
 轮询**；`PluginChannel` 退回**纯跨进程传输**（前端实时面用），不承担执行期协议。

### 6.2 执行期链路

```text
parse_sse_stream → sink.apply(message) → TranscriptSink → Transcript::apply → 事件总线
（进程内调用不经 serde；后端段无「帧」中转的两跳纯开销）
```

### 6.3 隐式语义显式化

`AbortGuard::disarm` 注销登记时一并 `abort`——把「通道被 drop ⇒ 对端读到关闭 ⇒ 视作
中止」这条隐式语义显式化（测试 `orchestrator.test.rs::disarm_aborts_the_signal` 锁定）。

### 6.4 保住的语义

- 转写仍只有**一个写入点**；`Warn` 仍是会话级状态（不进转写）——分派规则不变。
- 「中止不是失败」：会话结局是 `aborted`（不是 `completed`），在途根 Turn 定稿 `Aborted`
 （重试入口不消失）。
- `handle_abort` 的 3s 兜底判据（登记变 `None`）与语义保留。
- 工具的事件出口**已收敛到 `ExecEventSink`**（§7 / §8）：`shell` / `agent_run` / `ask_user` / 交互审批都写同一个出口，`execute_tool_async` 不再有流式分支、不再把帧解回来；`PluginPayload::Session` 只剩**跨进程**用途（`event_bus/subscribe`、vdfs 网关、CLI / 前端客户端），与执行期无关。

---

## 7. 工具侧收敛到同一出口

> `shell` / `agent_run` / `ask_user` / 交互审批都接到同一个出口（`ExecEventSink`）上，而不是各自返回 `PluginPayload::Session` 再由 `execute_tool_async` 把帧解回来。

### 7.1 工具怎么拿到出口

出口与中止信号**经 `ExecEnv` 显式传递**（由分发点装配，见 §9），不给 `Capability::execute` 加隐性键：

```rust
// 工具侧（只读）：缺席 ⇒ 静默 / 永不中止
let sink = env.sink;
let abort = env.abort;
```

出口是**执行期**的，与「这次调用从哪条路径来」无关（同一个 `shell` 既可能被编排层调用
而有出口，也可能被 `route` 直连而无出口 ⇒ 静默）；用 `env` 承载，工具不必记键名，
装配收口在 `capability_invoke` 一处（§9）。

### 7.2 「工具只声明意图，节点归编排层」

`ask_user` 与交互审批（`local/plugin.rs` 的 `confirm_prompt_payload`）**不构造节点**，
只返回载荷：

```rust
Ok(PluginPayload::new(&json!({
 "content": "需要确认：…", // 卡片正文
 "failure_kind": "needs_approval", // 编排层凭它收口为 WaitingUserAction
 "prompt": { "kind": "confirm", … }, // 卡片形状由工具决定
})))
```

节点由 `process_tool_calls_async` 用 `build_user_prompt_message` 构造
（`id = result_msg_id`、`parent_id = tool_call_id`）——**本文件是工具结果节点的
唯一写入者**，普通结果与等待用户都出自它。

**这条规则消灭的是一类 bug**：工具自造节点会让同一逻辑节点在前端有两个 id（重复审批卡、
resume 只删得掉一个）；`PendingPrompt` **没有 id 字段**，那类 bug 在类型层面不成立。

### 7.3 `failure_kind` 是共享闭集

`symbio_core::capability::failure_kind` 收口了这组词（`ERROR` / `NEEDS_APPROVAL` /
`NEEDS_INTERACTION` / `PERMISSION_DENIED` / `TOOL_UNAVAILABLE`），判据只有一条
`is_pending`。生产方（`local`）与消费方（`session`）读同一份常量，避免各自写字面量
导致「加一个等待用户的 kind 就漏改判定」（表现是交互模式下本批剩余工具照跑）。

它**不是**跨栈闭集（前端无同名常量镜像），因此不进 `protocol-mirror-audit` 的 C 组。

---

## 8. 流式工具接上出口

> `shell` 与 `agent_run` 的**流式增量**也走 `ExecEventSink`；`execute_tool_async`
> 不转发节点。

### 8.1 两条流式工具

**`shell`（单进程内自足）**：pump 任务直接写出口，执行体不 spawn。

| 项 | 形态 |
|---|---|
| 分流 | **一条路径**：`env.sink` / `env.abort` / `SnapshotTarget::from_ctx` 三个读取，缺席即降级 |
| 执行体 | 直接 `await`（出口没有背压） |
| 增量 | `sink.apply(message)`（一条 `ChatNode`）；**没有**「消费端还在吗」这种判断——没有可关闭的通道 |
| 返回值 | `Ok(Response{exit_code, output, risk_level})`——返回值就是结果 |

`SnapshotTarget`（`msg_id` + `tool_call_id` **同行**）承载「增量快照的节点身份」，
由编排层提供；**缺席 = 直连调用**（`route`，如 MCP 网关）⇒ 不发增量。
工具**不得**自造节点身份——与 §7.2 同一条规则。

**`agent_run`（子会话转播）**：转播桥成为「子会话 → 父视图」的**唯一翻译点**
（`user` 角色过滤、`parent_id` 锚定、`Assistant → Tool` 角色改写、`Reset`/`Warn` 丢弃
全部在此一处完成），出口下游只做落地。转播桥的返回值是具名结局：

```rust
enum RelayOutcome {
 Done(String), // 子会话跑完，最终答复
 Pending { text, prompt, failure_kind }, // 待用户动作（只给载荷，不给节点）
 Failed(String), // 子会话以错误结束
}
```

`Pending` 与 §7.2 的规则同源：**工具只声明意图**。子会话冒泡的审批节点**不落出口**，
只把 `text` / `prompt` / `failure_kind` 回传——节点由 `process_tool_calls_async`
以统一 id 构造。

### 8.2 保住的语义

- **增量是累积全量快照**（`role=tool` + `status=Streaming`），节流 120ms；
- **子会话节点锚定**：顶层节点 `parent_id → tool_call_id`、`Assistant → Tool`；
- **子会话的 `user` 消息不透传**（内容已见于 ToolCall 请求参数，临时 `role=user` 节点还会在
 前端获得必然失败的「编辑」入口）；
- **子会话的 `Reset` / `Warn` 不透传**（作用域不同；出口的 `Direct` 实现会把 `Warn` 分派到
 **本会话**节点上——透传等于把子会话告警记到父会话头上）；
- **待审批的优先级**：错误 > 待审批 > 文本。

### 8.3 执行期链路

```text
capability_invoke(cap, ctx) ← 唯一「拆信封」点（§9）
 ├─ args = ctx.payload::<Value> ?? Null
 ├─ env = ExecEnv::from_request(&*ctx) ← 出口 + 中止，缺席 ⇒ 静默 / 永不中止
 └─ cap.execute(args, &env, ctx)
 ├─ 读 ctx：RESULT_MSG_ID / TOOL_CALL_ID / WORKDIR…（只读真正需要的）
 ├─ env.sink.apply(message) ─► 转写唯一写入点（进程内，零 serde）
 └─ 返回 Value ────────────────► PluginPayload::Data
 └─ execute_tool_async
 └─ 结果节点 / 父终态（唯一写入者）
```

`PluginPayload::Session` 剩下的用途**只有跨进程**（`event_bus/subscribe`、vdfs 网关、
CLI / 前端客户端）；执行期走 `ExecEventSink` 进程内直连，与之无关。前端实时面也只有一条通道：
`event_bus` 的 `kind = "vdfs"` 频道（信封 `{path, data?}`）；**不存在** `session/stream`
这类独立转写流，实时帧因此没有帧序号，也不做缺口检测。

---

## 9. 两个执行接口同形

### 9.1 形状：`ExecEnv` 具名化

数据面统一之后，**签名**也统一：新增
[`ExecEnv { sink, abort }`](../../../symbio-core/src/exec/mod.rs)——「一次带中止的流式执行」
的**出 / 入两个方向**，两处共用：

```text
Capability::execute(args, env, ctx)
 -> Result<Value, PluginError>

ModelProvider::round_trip(system_prompt, messages, tools, root_id, env)
 -> Result<TurnOutput, PluginError>
```

两者都把「出口 + 中止」收在同一个 `env` 形参里（不是两个平铺参数，因此不会出现
「给了出口忘了给中止」）；差别只在 `ctx`——工具**被路由、被注册**故需信封，
模型执行不被路由故没有，不强行抹平。决策与理由见
[`docs/003-design/001-核心架构.md`](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")。

### 9.2 唯一「拆信封」的地方

`capability_invoke(cap, ctx)`（`symbio_core/capability/mod.rs`）是**唯一**把
`Result<Value, _>` 装回 `PluginPayload` 的地方。所有分发路径都必须经它：

- `DefaultToolVisitor`（`providers/collectors/tool_visitor.rs`）的 `Registry::get`：按名取行，再把行交给它
- `LocalPlugin::route` 的工具分支（`local/plugin.rs`）
- `WebPlugin::route` 的工具分支（`web/plugin.rs`）
- 装饰器 `PrefixedCapability`（`agent/host/scope.rs`）与 `SecureToolWrapper`
 （`local/plugin.rs`）**不拆不装**，`(args, env, ctx)` 原样透传。参数缺席（信封里没有
payload）⇒ `Value::Null`，仍由工具自己给出「缺少必填参数」的报错。

### 9.3 保住的语义

- `route` 直连调用**自然静默**：`ExecEnv::from_request` 在没有 `EVENT_SINK` 时给出
 `Null` 出口、没有 `ABORT_SIGNAL` 时给出永不中止的信号。
- 工具仍然**不构造节点身份**：`failure_kind` + `prompt` 载荷契约不变，节点归
 `tool_executor`（§7.2）。
- 压缩路径仍然**出口静默**：`run_compression_llm` 收 `env`，调用方给
 `ExecEnv::new(ExecEventSink::silent, abort)`——「内部请求不产生可见事件」是类型上的选择。

---

## 10. SSE 增量解析

> 契约拆成「完整行」与「未结束行」两个方法，协议知识留在协议层；决策与理由见 [`docs/003-design/005-协议规范.md`](../../../../docs/003-design/005-协议规范.md "DE-005-协议规范")。

### 10.1 形状

新增 [`crates/symbio-plugin-model/src/protocols/sse.rs`](../../../symbio-plugin-model/src/protocols/sse.rs)：

```text
trait SseLineParser {
 fn parse_line(&self, line: &str) -> Vec<ProtocolEvent>; // 完整行
 fn open_partial_line(&self, head: &str) -> Option<Box<dyn PartialLineExtractor>>; // 默认 None
}

trait PartialLineExtractor {
 fn push(&mut self, bytes: &str, out: &mut Vec<ProtocolEvent>); // 只吃**新增**字节
}
```

- **默认 `None` 是合法降级**：不实现增量提取的协议一行代码都不用写，代价只是「等换行」。
- `push` 一次可以吐**多个**事件：同一块里可能一个字段收尾紧接下一个字段开头。
- `open_partial_line` 由流循环**每行只问一次**：答 `None` 就记下、本行不再重试。

`utf8_chunk(buf, from)` 把「UTF-8 边界对齐」写进契约：被切断的多字节字符**不消费**，
留给下一次 `push`。SSE 分块由 TCP 决定，一个中文字符横跨两块是常态；若用
`from_utf8_lossy` 各替换出一个 U+FFFD，而完整行给出的是真字符，按前缀截断会吃字。

流循环侧（[`crates/symbio-plugin-model/src/stream.rs`](../../../symbio-plugin-model/src/stream.rs)）只做：按 `\n` 切行
→ 交给 `parse_line` → 按前缀截断去重（`LineProgress`）→ 尾巴交给提取器。

### 10.2 协议层：`JsonLineExtractor`

四个协议共用一个**逐字节推进**的 JSON 结构扫描器
（[`protocols/partial_json.rs`](../../../symbio-plugin-model/src/protocols/partial_json.rs)）。
它只做两件事：在 JSON 里定位「哪个字符串值」，以及按 JSON 转义规则解码它的增量；
「那个位置算内容还是推理」由协议实现 `PartialJsonSink` 决定。判据是**键路径全等**
（数组下标不占位）：

```text
openai_chat ["choices","delta","content"] -> 内容
 ["choices","delta","reasoning_content"] -> 推理
 ["choices","delta","tool_calls","function","arguments"]-> 参数(取最内层数组下标)
openai_responses ["type"] 先观察，再按它决定 ["delta"] 算什么
anthropic ["type"] 必须是 content_block_delta，再看 ["delta", ...]
gemini ["candidates","content","parts","text"] -> 内容
```

`ModelProtocol` 以 `SseLineParser` 为**父 trait**，故 `BoundProvider::round_trip` 直接把
协议实例交给 `parse_sse_stream`——流循环与协议之间无闭包中转。

### 10.3 保住的语义

- **前缀截断去重机制不动**：`LineProgress` 按字段分别记账，完整行到达时按已发长度截断。
 增量文本「恰好是完整行文本的前缀」这条不变量**由协议层用同一套转义规则**保证。
- **`PARTIAL_LINE_MIN_BYTES = 256` 不动**：短尾巴等下一块补齐即可。**增量路径不产出**
 `Finish` / `Usage` / `Error` / `ResponseId`：半截 JSON 里这些字段的值不可信，它们只走完整行
 （由 `LineProgress::record` 丢弃非增量事件兜底）。
- **`[DONE]`、注释行、非 data 行不吐任何事件**（扫描器找不到 JSON 起点）。
- **协议判定「宁可漏，不可错」**：全等匹配、下标未知就放弃本次增量。错配的代价是
 前端多出内容且完整行截断会吃正文；不匹配的代价只是失去增量。

### 10.4 已知取舍

- 扫描器对**非法 JSON 转义**比 `serde_json` 宽松（分歧只在非法输入上，此时完整行路径本就不产出事件）；
 `output_index` 出现在 `delta` **之后**时本次增量被放弃（真实报文里它在前）；Gemini 的 `[{...},{...}]` 挤在一行非真实形状，增量路径不产出。

---

## 11. 工具调用协议封闭

> 工具名怎么发出去、怎么认回来，以及工具结果怎么读。三件事的共同点：它们都是跨插件 / 跨层的
> **约定**，需要写成契约。名字归属的准入规则见
> [`docs/003-design/001-核心架构.md`](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")。

### 11.1 名字的线上形态：投影 + 查表，不反演

今天注册的能力名（`vdfs_read` / `shell` / `ask_user` / `todo_write` / `codebase_search` / `web_search` /
`http_request` / `heartbeat` / `agent_run` / `mcp.<server>.<tool>`（带**点**）/ `agent_<safe_id>_<tool>`）里
**没有一个含 `/`**，真正违反协议字符集的是 MCP 的 `.`（OpenAI / Anthropic 只接受 `[A-Za-z0-9_-]`）。
因此名字的线上形态由**一对具名函数**定义（`symbio_core/capability/tool_name.rs`）——`capability_to_wire`
（`model` 出）与 `capability_resolve`（`session` 入）是同一份契约的两半，两个插件互相不可见，core 是唯一共同可见处：

```rust
pub fn capability_to_wire(canonical: &str) -> String // 字符集之外的字符一律 → "__"
pub fn capability_resolve<'a>(wire: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str>
```

- 出方向：**一个具名函数**，字符集定义只有一份；入方向：**查注册表**
 （`visitor.list(family::TOOL)` 工具行的名字集合），不反演。**字面名优先**——`mcp__fs__read` 既可能是
 `mcp.fs.read` 的线上形态、也可能本身就是这个名字，只有集合能回答；
- 解析失败 → **不猜**，按原样交给路由，由它给出诚实的 `NotFound`（反演猜错会调起**另一个工具**）。
 命中判据是「解析成功」而不是「按线上名查表」——带非法字符的名字按线上名永远查不到。

### 11.2 工具结果：顺序即约定

`extract_result` **留在 `session/tools/tool_executor.rs`**（唯一消费方是它自己），判定顺序是
**契约**（模块文档的表 + 两个字段名常量 + 用例钉住）：

```text
content 是字符串 → output 是字符串 → success 是布尔 → 顶层是字符串 → 整包 JSON
```

- 「**先命中者胜**」而不是「取第一个存在的键」：`{"content":[1,2],"output":"x"}` 里 `content`
 存在但不是字符串 ⇒ 继续走到 `output`；按「存在即取」会把数组原样丢给模型。
- 「结果文本本就没有契约」（工具返回任意 JSON，模型只消费一段文本）——这层适配是**显式**的。
- **控制流不得建立在这里**：要判「等待用户动作」用 `symbio_core::failure_kind` 这个约定字段，不猜形状（§7.3）。

### 11.3 审批闸门：判两次，写一次

两条入口都真实存在（`route` 是注册表未命中的回落路径，`wrapper` 是注册表路径），故闸门两处把守，
但**判定逻辑**收成一个 `approval_gate`（两处传的都是工具真实描述，文案一致）。

### 11.4 MCP 侧的两条协议契约

- **协议字段名大小写**：`mcp/types.rs` 的协议类型带 `#[serde(rename_all = "camelCase")]`
 映射（MCP 规范用 camelCase）。缺失会让 `initialize` 响应解析失败 ⇒ 该 server 的工具一个都不注册。
- **工具结果按载荷解析**：`McpToolCallResponse` 按规范载荷（`content` / `structuredContent` /
 `isError`）建模，不对 JSON-RPC 信封建模——否则全字段可选会「解析成功」并返回 `Value::Null`，
 正文永远渲染成字符串 `null` 且不报错。`stdio.rs` / `http.rs` 只借它读 `isError`，其余原样返回；
 `capability.rs::execute` 把内容块压平成 `{"content": <文本>}` 再交出——**跨插件约定保持最窄**，
 会话层不需要认识 MCP 的「内容块数组」形状。

### 11.5 已知取舍

- **Gemini `functionResponse.name` 填的是 `tool_call_id`**（规范要求函数名；id 不是能力名，做线上形态换算无意义）。
- **MCP 工具名在前端显示为线上形态 `mcp__server__tool`**；前端不按 `mcp.` 前缀判断，只影响显示。
- **线上名投影不单射**（`a.b` 与 `a__b` 都得 `a__b`），由 `resolve` 的字面优先消解。

---

## 12. 发送方向的会话读取合并

> 本节覆盖 §1.1 的 **① 装配层**（`start_turn`）。

### 12.1 读取点与请求级快照

`SessionSnapshot`（`orchestrator/entry.rs`）让 `resolve_session_params` 带上**本请求唯一一次**
元数据读取，后续所有元数据回退（mode / risk_level / provider_id / workdir / agent_id）都从这一份取：
分四次读不只是多 I/O，还会取到被并发改动改写过的不同版本（workdir 取自改动前、agent_id 取自改动后）。

| 读取点 | 读什么 | 次数 |
|---|---|---|
| `resolve_session_params` | `metadata`（mode / risk_level / provider_id / workdir / agent_id） | 1（唯一一次元数据读） |
| `ensure_auto_title` | 快照已有标题 ⇒ 整趟跳过；否则读 `messages` 派生标题 | 0 / 1 |
| `append_messages` / `persist_messages` | `messages`（读-改-写，必需） | 各 1 |
| `get_context_messages`（每轮） | `messages`（必需） | 1 |

### 12.2 为什么安全（等价性）

上述字段在本请求内**不会被本请求改写**（`append_messages` 只改 `messages`，`ensure_auto_title`
只写 `metadata.title`），故派发前读一次与派发内读一次等价。`has_title` 的判据与
`ensure_auto_title` 内部的提前返回**逐字对齐**（`!s.trim.is_empty`），由用例
`has_title_matches_the_ensure_auto_title_criterion` 钉住——判错即「快照说已有标题 → 跳过 →
实际从未命名」的静默丢命名。

### 12.3 读取次数

`load_session` 次数：新建会话 **6**、已有标题的会话 **5**。组成 = 一次元数据读取 + 三处
`messages` 读取（两处写路径读-改-写、一处每轮上下文）+ 新建会话上 `ensure_auto_title` 的那一次。
三处 `messages` 读取**不该省**（须各取最新值）；落库回包（把权威 `seq` 交回实时面，见
[`022-vdfs-session-messages.md`](109-vdfs-session-messages.md "DE-109-vdfs-session-messages") §3.4）
不另读——`append_messages` 直接把落库后的权威副本交回调用方。
