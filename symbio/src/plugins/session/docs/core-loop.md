# Session 核心循环（LLM × 工具）

> 范围：`symbio/src/plugins/session/` 中「调用大语言模型 + 执行工具」的会话主循环。
> 本文只回答一个问题：**主循环的结构是什么**。「压缩策略有哪些」见
> [`../README.md`](../README.md)（六大压缩策略）；节点状态与流的词汇见
> [`./node-state-streaming.md`](./node-state-streaming.md)；模块拆分与符号落点见
> [`./module-layout.md`](./module-layout.md)；「为什么这样设计」见
> [`docs/DECISIONS.md`](../../../../../docs/DECISIONS.md)。
> 代码位置一律按**符号名**检索——模块拆分后行号会漂移。

---

## 0. 一句话结论

主循环 = **前步骤**（请求解析 → 开会话 → `TurnState` → resume）+ **loop**（7 步）+
**唯一收尾点 `finish_turn`**。循环只能由 `return finish_turn(...)` 离开，没有旁路出口。

四个收口点 + 一个状态容器（§2）：

| 收口项 | 职责 | 唯一性 |
|---|---|---|
| `gate_turn` | 启动条件（在途工具是否齐备）+ 退出条件（软上限 / abort） | 判定点唯一 |
| `prepare_turn_inputs` | 系统提示词 + 工具（同机制、同时机）+ 请求级开销 + 压缩 + 请求视图 | 准备点唯一 |
| `apply_compaction` | 水位响应（自动压缩 70% → 水位提醒 55%，顺序写死） | 响应点唯一 |
| `finish_turn` | 软上限提示 + 增量落库 + Stop 钩子 + 返回语义 | 收尾点唯一 |
| `TurnState` | 跨轮可变状态（工具轮次 / 续写次数 / 落库锚点 / 在途集合 / 中止信号） | 状态容器唯一 |

---

## 1. 调用链全景

### 1.1 三层结构

```text
① 装配层   handle_chat_send_oneoff                orchestrator/entry.rs
             解析 session_id / mode / risk / provider
             用户消息落库 + 自动命名 + agent_id 解析
             chat_ctx 装配（WORKDIR / SESSION_ID / AGENT_ID / SESSION_HANDLE）
             collect_capabilities(traverse) → CAPABILITY_VISITOR
             （系统提示词与工具都在这一步收集；本层**不**拼提示词）
             构造 model_chat::Request → run_chat_loop_task
② 宿主层   run_chat_loop_task                     orchestrator/consume.rs
             WorkingGuard（panic 兜底）/ StopSignal / AbortGuard
             provider 解析 + RATE_LIMITER
             EventSink::direct(TranscriptSink) + AbortSignal::new()
             spawn → ③（子任务）
             select! 三臂：任务结束 / 1800s 看门狗 / 状态被接管（见 §6）
               → 收尾：定稿在途 / persist_failure / 结局字段
③ 循环层   run_chat_loop                          chat_loop.rs
             + gate_turn / finish_turn             chat_loop.rs
             + close_turn / settle_reasoning       chat_loop/turn.rs
             + prepare_turn_inputs / apply_compaction   chat_loop/inputs.rs
             + auto_compress_process / compress_with_snapshot_core
               / run_context_compact               chat_loop/compress.rs
             + persist_messages / open_chat_session / emit_streaming_start
               / fire_*_hook                        chat_loop/io.rs
             + process_tool_calls_async            tool_executor.rs
```

**职责边界**：③ 不持有终态落库职责——终态唯一落库点在 ② 的消费循环。后步骤**不存在**：
循环只能由 `return` 离开，所有出口都经 `finish_turn`。

### 1.2 主循环七步

**前步骤**（循环之前）

| # | 动作 |
|---|---|
| P1 | 解析 `model_chat::Request`，构造请求级快照 `TurnRequest::new(&req)`（默认值唯一真源是 `SessionConfig`） |
| P2 | `open_chat_session`（取 `SESSION_HANDLE`，缺则回退内存会话） |
| P3 | `TurnState` 初始化（含 `abort: AbortSignal`）+ `SessionContext` 构造 |
| P4 | **resume 处理**（`process_resume`）→ `Continue` / `Done` / `Err`，`Done`/`Err` 经 `finish_turn` 提前收尾 |

**循环体**（7 步）

| # | 步骤 | 内容 | 职责 |
|---|---|---|---|
| 1 | 加载本轮上下文 | `get_context_messages` 轮次窗口；首轮追加 `single_message` + `UserPromptSubmit` 钩子；推进 `last_saved` | 加载 |
| 2 | `gate_turn` | 启动条件 + 退出条件（收口 ①）；`Exit` → `finish_turn` | 判定 |
| 3 | `prepare_turn_inputs` | 系统提示词 + 工具（同函数、同时刻）+ `overhead_tokens` + `apply_compaction` + `build_request_view`（收口 ②③） | 准备 + 压缩 |
| 4 | abort 检查 + `execute_turn` | Turn 创建后的中止检查点；`ExecEnv::new(sink, abort)` 一次给全出口与中止；`provider.execute_turn` | 推理 |
| 5 | `settle_reasoning` | 推理产物定稿（子节点状态）→ `feedback_estimate` 校准 → `into_messages` 并入 `context.messages` | 收尾 |
| 6 | `close_turn` | 截断续写 / 主动压缩拦截 / 工具分发 / 父节点状态落库 / 停等判定（收口 ④ 的判定分支） | 收尾 + 判定 |
| 7 | `finalize_turn_root` | 封根 Turn——子树收敛之后发出**即将落库的那条节点** | 收尾 |

**`close_turn` 的分支**

| 分支 | 动作 | 返回 |
|---|---|---|
| 无工具 | `finish == length` 且无工具调用 → `length` 自动续写（≤ `MAX_CONTINUE_ROUNDS`）；续写耗尽 / 工具参数截断 → 会话级告警 | `NextTurn` / `Finish(Completed)` |
| 有工具 | `context_compact` 拦截分流（主动压缩） | — |
| 有工具 | `process_tool_calls_async`（**串行阻塞**，见 §4） | — |
| 有工具 | 父节点终态快照同步进内存镜像（整条替换，随 persist 落库） | — |
| 有工具 | `needs_user_action` → 停等 | `Finish(Completed)` |
| 有工具 | `tool_rounds += 1` | `NextTurn` |

### 1.3 职责边界（收口依据）

- **收尾三件套**（软上限提示 / 增量落库 / Stop 钩子）只在 `finish_turn` 出现一次；
  每个出口只负责构造 `TurnExit`。
- **压缩判定与执行**同处 `apply_compaction`，顺序由函数内代码行序保证；
  内核 `compress_with_snapshot_core` 是唯一实现。
- **提示词与工具**必须同机制、同时机收集：两者都经 `CapabilityVisitor` 注册、经 `traverse`
  汇集到同一个 visitor 的「会话输入」，来源与生命周期一致。
- **启动条件**是显式判据（`TurnState::in_flight_tools`），不是「同步阻塞的隐式结果」。

---

## 2. 收口点

### 2.1 收口 ①：启动条件 + 退出条件 → `gate_turn`

```rust
/// 单轮是否允许发起 LLM 请求（启动条件）以及是否必须退出（退出条件）。
/// 唯一调用点：主循环顶部。
enum Gate {
    Proceed,                  // 条件齐备 → 进入准备与推理
    WaitForTools,             // 工具结果未齐备（异步工具在途）→ 不唤醒
    Exit(TurnExit),           // 软上限 / abort / 不可恢复
}

fn gate_turn(req: &TurnRequest, turn: &TurnState) -> Gate
```

**启动条件的唯一判据**：取**运行时在途集合** `TurnState::in_flight_tools`——
非空 ⇒ 不唤醒；多个工具调用**全部**结束才唤醒主循环，不完整不唤醒。

**不是**「扫历史看每个 ToolCall 有没有结果子节点」：**「ToolCall 无结果」在本代码库是
合法状态**（交互模式审批中断时本批剩余 ToolCall 有意留空，由请求视图层 `flatten_chat_messages`
合成占位结果喂回模型），按历史判定会让 approve/reject 恢复后的续写被**永久挡住**。

> ⚠️ **必须配修复路径**：若某个工具永远等不到结果（进程被杀 / 工具任务 panic），
> 在途集合会永久非空 → 主循环永久挂起。`tool_executor::record_protocol_failure`
> 在写入时合成失败结果来维持「没有悬空 ToolCall」这条不变式。

退出条件在 `gate_turn` 内判定两处，语义不同：

- **显式软上限**（`req.max_tool_rounds`，`None` = 不限制）→ `TurnExit::MaxToolRounds`，
  先广播明确提示再退出，绝不静默；
- **循环顶部的中止检查点** → `TurnExit::AbortedAtBoundary`（**不冒泡** `Err(Aborted)`：
  上一轮 Turn 已定稿落库，冒泡会让消费循环的 `persist_failure` 把成功的 Turn
  误回滚为 Failed）。推理前后（在途 Turn 尚未定稿）的中止才走 `TurnExit::Aborted`。

### 2.2 收口 ②：提示词 / 工具 / 请求视图 → `prepare_turn_inputs`

**硬约束：系统提示词与工具信息必须同机制、同时机、始终一起收集——在每轮 Turn 开头、
循环内收集。**

```rust
/// 一次 LLM 请求所需的全部输入。**唯一收集点**：系统提示词与工具在同一函数内
/// 依次取得（同一个 CapabilityVisitor、同一个时刻），下游只读产物。
struct TurnInputs {
    root_id: String,                    // Turn 根节点 id（本步内创建）
    system_prompt: String,              // resolve_system_prompt 唯一真源
    tools: Vec<CapabilityMeta>,         // 含条件注入的 context_compact
    request_view: Vec<ChatMessage>,     // build_request_view 产物（不落库）
}

async fn prepare_turn_inputs(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    sink: &EventSink,
    turn: &mut TurnState,
    req: &TurnRequest,
) -> Result<TurnInputs, TurnExit>;
```

- `retention` **不进** `TurnInputs`——它只是 `build_request_view` 的一个入参，
  由 `tools` 就地派生后立即消费，暴露到边界只多一份需要维护的中间态。
- `overhead_tokens` 是函数内局部量：只被本步内的自动压缩阈值与水位提醒消费，
  **一轮只算一次**（`compression::estimate_overhead_with_tools(system_prompt, &tools)`
  复用已收集的工具清单，不让 visitor 注册表被查三遍）。

每轮收集本身**不触发 I/O**（`list_capability` / `list_system_prompts` 均为纯内存读
visitor 注册表），因此「每轮动态可调」是净收益：人格与工具集始终反映**当轮**注册表，
中途新增的能力（如 MCP 新连上服务端）下一轮即可见。

### 2.3 收口 ③：压缩 → `apply_compaction`

```rust
/// 收口 ③：上下文水位的**唯一响应点**——判定与执行同处一地，顺序写死。
///
/// 返回：是否需要在请求视图末尾注入一次性水位提醒。
async fn apply_compaction(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    sink: &EventSink,
    turn: &mut TurnState,
    req: &TurnRequest,
    overhead_tokens: usize,
) -> Result<bool, TurnExit> {
    // ① L1 自动语义压缩（70% 触发；就地改写 context.messages 并推进落库锚点）
    // ② 水位提醒（55% 触发；判**压缩之后**的真实水位，不落库，交视图层追加）
    …
}
```

**判定与执行同处一地**：`Auto`（70%）与 `Nudge`（55%）顺序相依——先按 70% 压缩，
再按压缩后的水位判 55% 提醒；主动压缩又由模型输出触发、决策点在 LLM 响应之后的
`close_turn`。两者都不可能进入「请求前」的纯决策。

**收口后的分工**：

- **水位响应**（auto + nudge）→ 唯一入口 `apply_compaction`；
- **执行内核** → `compress_with_snapshot_core`（唯一实现）；
- **收益护栏** → `compression::has_compaction_payoff`（门槛语义只有这一个出处）；
- **「何时压」仍有两个入口**（水位触发 / 模型主动）——这是产品语义决定的，不合并；
  被合并的是「怎么压」（1 个实现）与「水位怎么响应」（1 个入口）。

`auto_compress_process` 不带 `extra_hints` 参数（自动路径没有「用户」在环内提供
保留提示，该参数在本路径恒为 `None`）；`force` 保留——它是 `should_start_compression`
的公开契约且有单测背书。

### 2.4 收口 ④：退出 → `TurnExit` + `finish_turn`

```rust
/// 主循环的唯一退出原因。
enum TurnExit {
    Completed,                     // 正常收尾（无工具调用 / 待用户输入）
    AbortedAtBoundary,             // 循环顶部的中止检查点：不冒泡
    MaxToolRounds { max: usize },  // 显式软上限
    Aborted,                       // 用户中止且在途 Turn 未定稿 → 冒泡 Err(Aborted)
    Failed(PluginError),
    ResumeDone,                    // resume 已完成，无需进入循环
}

/// 唯一收尾点：Stop 钩子（幂等）+ 增量落库 + 返回。所有出口只做一件事：构造 TurnExit。
async fn finish_turn(
    orch: &ChatOrchestrator,
    context: &SessionContext,
    sink: &EventSink,
    turn: &TurnState,
    exit: TurnExit,
) -> Result<(), PluginError>;
```

- `StopSignal` 的幂等 + RAII 兜底保留：显式触发点携带准确的「本轮最后一条消息」，RAII 兜底
  只在显式触发全部未发生时生效（panic / 消费循环超时 / provider 解析失败）。
- `persist_messages` 在锚点已对齐时是 no-op（切片为空即返回），因此所有出口都可以
  无条件调用，不会重复落库。
- `TurnExit::Aborted` → `Err(PluginError::Aborted)`，消费循环的 ABORTED 分支语义不变。

### 2.5 `TurnState`：跨轮可变状态

```rust
struct TurnState {
    abort: AbortSignal,              // 用户中止信号（推理与工具共享同一份）
    tool_rounds: usize,              // 已完成的工具轮次（跨轮累加）
    continuation_count: u32,         // 长度截断自动续写次数（跨轮累加）
    nudged_this_request: bool,       // 水位提醒一次性标记
    last_saved: usize,               // 增量落库锚点
    in_flight_tools: HashSet<String>,// 已派发、尚未产出结果的工具调用 id（启动条件权威判据）
}
```

`close_turn` 的参数表因此收成 `(orchestrator, ctx, sink, context, turn, result, req)`。

---

## 3. 主循环骨架

```rust
pub async fn run_chat_loop(
    orchestrator: &ChatOrchestrator,
    ctx: Arc<dyn PluginInvokeRequest>,
    sink: EventSink,          // 事件唯一出口（见 §6）
    abort: AbortSignal,       // 中止唯一入口（见 §6）
) -> Result<(), PluginError> {
    // ── 前步骤 ①：请求解析与请求级配置快照 ──────────────────────────────
    let mut req: model_chat::Request = ctx.payload()?;
    let turn_req = TurnRequest::new(&req);          // 默认值唯一真源：SessionConfig

    // ── 前步骤 ②：会话引擎 ──────────────────────────────────────────────
    let session = open_chat_session(&ctx).await;

    // ── 前步骤 ③：轮次状态 + 上下文容器 ─────────────────────────────────
    let mut turn = TurnState { abort: abort.clone(), ..Default::default() };
    let mut single_message = req.single_message.take();
    let mut context = SessionContext { messages: Vec::new(), session };

    // ── 前步骤 ④：resume（一次性，可提前收尾）───────────────────────────
    if let Some(tr) = req.resume.take() {
        match resume::process_resume(…).await {
            Ok(ResumeOutcome::Continue) => {}
            Ok(ResumeOutcome::Done) =>
                return finish_turn(orch, &context, &sink, &turn, TurnExit::ResumeDone).await,
            Err(e) =>
                return finish_turn(orch, &context, &sink, &turn, TurnExit::Failed(e)).await,
        }
    }

    // ── 主循环 ──────────────────────────────────────────────────────────
    loop {
        // 步骤 1 · 加载本轮上下文（会话窗口 + 首轮追加用户消息 + 推进落库锚点）
        …

        // 步骤 2 · 闸门（收口 ①：启动条件 + 退出条件，唯一判定点）
        match gate_turn(&turn_req, &turn) {
            Gate::Proceed      => {}
            Gate::WaitForTools => return finish_turn(…, TurnExit::Completed).await,
            Gate::Exit(exit)   => return finish_turn(…, exit).await,
        }

        // 步骤 3 · 本轮输入（收口 ②③，唯一准备点）
        //   系统提示词 + 工具在同一函数内、同一时刻收集（同一 CapabilityVisitor）；
        //   派生 overhead（一轮一次）；压缩（apply_compaction）与视图重建亦在其内。
        let inputs = match prepare_turn_inputs(orch, &ctx, &mut context, &sink,
                                              &mut turn, &turn_req).await {
            Ok(v)            => v,
            Err(exit)        => return finish_turn(orch, &context, &sink, &turn, exit).await,
        };

        // 步骤 4 · 推理（Turn 创建后的中止检查点 + 唯一 LLM 发起处）
        if turn.abort.is_aborted() {
            return finish_turn(orch, &context, &sink, &turn, TurnExit::Aborted).await;
        }
        let env = ExecEnv::new(sink.clone(), turn.abort.clone());
        let result = orch.provider.execute_turn(&inputs.system_prompt, &inputs.request_view,
                                                &inputs.tools, &inputs.root_id, &env).await;
        let out = match result {
            Err(RetryWithoutContextId) => { /* 清 response_id + 移除半截 Streaming */ continue; }
            Err(Aborted)               => return finish_turn(…, TurnExit::Aborted).await,
            Err(e)                     => return finish_turn(…, TurnExit::Failed(e)).await,
            Ok(out)                    => out,
        };
        if turn.abort.is_aborted() {
            return finish_turn(orch, &context, &sink, &turn, TurnExit::Aborted).await;
        }

        // 步骤 5 · 推理收尾（定格子节点 → 校准估算 → 并入上下文）
        let result = settle_reasoning(orch, &mut context, &sink, &inputs.root_id, out).await;

        // 步骤 6 · 本轮结算 + 下一步判定（收口 ④）
        let flow = close_turn(orch, ctx.clone(), &sink, &mut context,
                              &mut turn, result, &turn_req).await;

        // 步骤 7 · 封根 Turn（子树收敛之后，全流程唯一一处）
        finalize_turn_root(&sink, &context, &inputs.root_id).await;

        match flow {
            TurnFlow::NextTurn     => {}
            TurnFlow::Finish(exit) => return finish_turn(orch, &context, &sink, &turn, exit).await,
        }
    }
    // ── 后步骤 ──────────────────────────────────────────────────────────
    // 无：循环只能由 return 离开，所有出口都经 finish_turn（唯一收尾点）。
}
```

---

## 4. 工具执行与启动条件

### 4.1 当前执行模型：串行、单点

`process_tool_calls_async` 在 `close_turn` 内**顺序 `await`** 每个工具
（`for tc in tool_calls`）。本批工具全部跑完才回到循环，因此回到 `gate_turn` 时
`TurnState::in_flight_tools` 恒为空。

`in_flight_tools` 仍作为**启动条件的权威判据**存在（§2.1）：`WaitForTools` 分支在当前模型下
恒不命中，语义上却已成立。

### 4.2 必须保留的语义

1. `interactive` 模式下「前一个工具待审批 / 失败 → 中止本批剩余」
   （`tool_executor.rs`）——顺序语义；
2. `record_protocol_failure` 为 id / name 非法的调用合成失败结果，
   保证「没有悬空 ToolCall」这一不变式；
3. 工具执行的超时判据是**出口的发射计数**（`EventSinkProgress`，挂在出口上——
   出口是「工具还活着」的唯一证据源，因为工具唯一的出方向动作就是 `emit`）：

   ```rust
   let progress = sink.progress();
   let mut seen = progress.emitted();
   loop {
       tokio::select! { biased;
           r = &mut route_fut => break r,               // 工具已返回 ⇒ 立即采纳
           _ = wait_tool_abort(abort) => return …,      // 用户中止
           _ = sleep(IDLE) => {
               let now = progress.emitted();
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
  请求，**不本地截断历史**（[`docs/DECISIONS.md`](../../../../../docs/DECISIONS.md) ADR-018）；
- **`compress_with_snapshot_core` 单一实现**（被动与主动共用内核）；
- **`StopSignal` 幂等 + RAII 兜底** 与 **`WorkingGuard` / `AbortGuard`** 的收敛保证（§6）；
- **`finalize_assistant_turn` 的 reasoning-only 分支**、**Turn 子树完整性守卫**
  （`find_turn_user_split_idx`，provider 400 的直接防线）、**`feedback_estimate` 两条
  不变式**、**`tool_executor` 的超时与失败信息性语义**；
- **消费循环（`orchestrator/consume.rs`）的 `select!` 三臂与 `persist_failure` 只作用于
  `failing_turn` 子树**（终态落库唯一入口，不误回滚上一轮）。

---

## 6. 执行期出口 / 信号双原语

> 本节描述**宿主层与执行层之间的协议**：`EventSink`（出）与 `AbortSignal`（入）
> 是两个语义单一的原语；`PluginChannel` 只作**纯跨进程传输**，不承担执行期协议。
> 决策与理由见 [`docs/DECISIONS.md`](../../../../../docs/DECISIONS.md) ADR-020。

### 6.1 原语

| 原语 | 方向 | 形状 | 位置 |
|---|---|---|---|
| `EventSink` | 出 | `Direct(Arc<dyn TranscriptWriter>)` \| `Null` | `symbio_core/exec/mod.rs` |
| `AbortSignal` | 入 | `Arc<AtomicBool>` + `CancellationToken` 合一 | `symbio_core/exec/mod.rs` |

- **`EventSink::Direct`** 直连转写唯一写入点（`TranscriptSink` → `Transcript::apply`），
  进程内**零 serde**；**`EventSink::Null`** 是「本次调用不产生可见事件」的**类型级**表达
  （上下文压缩、`resume` 重跑工具用它）。
- **`AbortSignal::abort()`** 是唯一置位入口，置位即唤醒（`CancellationToken`），因此**没有
  轮询**；`PluginChannel` 退回**纯跨进程传输**（前端实时面用），不承担执行期协议。

### 6.2 执行期链路

```text
parse_sse_stream → sink.apply(message) → TranscriptSink → Transcript::apply → 事件总线
（进程内调用不经 serde；后端段无「帧」中转的两跳纯开销）
```

### 6.3 隐式语义显式化

`AbortGuard::disarm` 注销登记时一并 `abort()`——把「通道被 drop ⇒ 对端读到关闭 ⇒ 视作
中止」这条隐式语义显式化（测试 `orchestrator.test.rs::disarm_aborts_the_signal` 锁定）。

### 6.4 保住的语义

- 转写仍只有**一个写入点**；`Warn` 仍是会话级状态（不进转写）——分派规则不变。
- 「中止不是失败」：会话结局是 `aborted`（不是 `completed`），在途根 Turn 定稿 `Aborted`
  （重试入口不消失）。
- `handle_abort` 的 3s 兜底判据（登记变 `None`）与语义保留。
- 工具经**自有** `PluginPayload::Session` 通道回传事件的能力完整保留（尚未收敛，见 §7）。

---

## 7. 工具侧收敛到同一出口

> `shell` / `agent_run` / `ask_user` / 交互审批都接到同一个出口（`EventSink`）上，而不是各自返回 `PluginPayload::Session` 再由 `execute_tool_async` 把帧解回来。

### 7.1 工具怎么拿到出口

出口与中止信号**经 `ExecEnv` 显式传递**（由分发点装配，见 §9），不给 `Capability::execute` 加隐性键：

```rust
// 工具侧（只读）：缺席 ⇒ 静默 / 永不中止
let sink = env.sink();
let abort = env.abort();
```

出口是**执行期**的，与「这次调用从哪条路径来」无关（同一个 `shell` 既可能被编排层调用
而有出口，也可能被 `route()` 直连而无出口 ⇒ 静默）；用 `env` 承载，工具不必记键名，
装配收口在 `invoke_capability` 一处（§9）。

### 7.2 「工具只声明意图，节点归编排层」

`ask_user` 与交互审批（`local/plugin.rs` 的 `confirm_prompt_payload`）**不构造节点**，
只返回载荷：

```rust
Ok(PluginPayload::new(&json!({
    "content": "需要确认：…",           // 卡片正文
    "failure_kind": "needs_approval",   // 编排层凭它收口为 WaitingUserAction
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
`is_pending()`。生产方（`local`）与消费方（`session`）读同一份常量，避免各自写字面量
导致「加一个等待用户的 kind 就漏改判定」（表现是交互模式下本批剩余工具照跑）。

它**不是**跨栈闭集（前端无同名常量镜像），因此不进 `protocol-mirror-audit` 的 C 组。

---

## 8. 流式工具接上出口

> `shell` 与 `agent_run` 的**流式增量**也走 `EventSink`；`execute_tool_async`
> 不转发节点。

### 8.1 两条流式工具

**`shell`（单进程内自足）**：pump 任务直接写出口，执行体不 spawn。

| 项 | 形态 |
|---|---|
| 分流 | **一条路径**：`env.sink()` / `env.abort()` / `SnapshotTarget::from_ctx` 三个读取，缺席即降级 |
| 执行体 | 直接 `await`（出口没有背压） |
| 增量 | `sink.apply(message)`（一条 `ChatMessage`）；**没有**「消费端还在吗」这种判断——没有可关闭的通道 |
| 返回值 | `Ok(Response{exit_code, output, risk_level})`——返回值就是结果 |

`SnapshotTarget`（`msg_id` + `tool_call_id` **同行**）承载「增量快照的节点身份」，
由编排层提供；**缺席 = 直连调用**（`route()`，如 MCP 网关）⇒ 不发增量。
工具**不得**自造节点身份——与 §7.2 同一条规则。

**`agent_run`（子会话转播）**：转播桥成为「子会话 → 父视图」的**唯一翻译点**
（`user` 角色过滤、`parent_id` 锚定、`Assistant → Tool` 角色改写、`Reset`/`Warn` 丢弃
全部在此一处完成），出口下游只做落地。转播桥的返回值是具名结局：

```rust
enum RelayOutcome {
    Done(String),                             // 子会话跑完，最终答复
    Pending { text, prompt, failure_kind },   // 待用户动作（只给载荷，不给节点）
    Failed(String),                           // 子会话以错误结束
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
invoke_capability(cap, ctx)                 ← 唯一「拆信封」点（§9）
  ├─ args = ctx.payload::<Value>() ?? Null
  ├─ env  = ExecEnv::from_request(&*ctx)    ← 出口 + 中止，缺席 ⇒ 静默 / 永不中止
  └─ cap.execute(args, &env, ctx)
        ├─ 读 ctx：RESULT_MSG_ID / TOOL_CALL_ID / WORKDIR…（只读真正需要的）
        ├─ env.sink().apply(message) ─► 转写唯一写入点（进程内，零 serde）
        └─ 返回 Value ────────────────► PluginPayload::Data
                                          └─ execute_tool_async
                                               └─ 结果节点 / 父终态（唯一写入者）
```

`PluginPayload::Session` 剩下的用途**只有跨进程**（`event_bus/subscribe`、vdfs 网关、
CLI / 前端客户端）；执行期走 `EventSink` 进程内直连，与之无关。前端实时面也只有一条通道：
`event_bus` 的 `kind = "vdfs"` 频道（信封 `{path, data?}`）；**不存在** `session/stream`
这类独立转写流，实时帧因此没有帧序号，也不做缺口检测。

---

## 9. 两个执行接口同形

### 9.1 形状：`ExecEnv` 具名化

数据面统一之后，**签名**也统一：新增
[`ExecEnv { sink, abort }`](../../../symbio_core/exec/mod.rs)——「一次带中止的流式执行」
的**出 / 入两个方向**，两处共用：

```text
Capability::execute(args, env, ctx)      -> Result<Value, PluginError>
ModelProvider::execute_turn(inputs, env) -> Result<TurnOutput, PluginError>
```

决策与理由见 [`docs/DECISIONS.md`](../../../../../docs/DECISIONS.md) ADR-021；
差别只在 `ctx`——工具**被路由、被注册**故需信封，模型执行不被路由故没有，不强行抹平。

### 9.2 唯一「拆信封」的地方

`invoke_capability(cap, ctx)`（`symbio_core/capability/mod.rs`）是**唯一**把
`Result<Value, _>` 装回 `PluginPayload` 的地方。所有分发路径都必须经它：

- `DefaultToolVisitor::invoke`（`symbio_core/capability/tools.rs`）
- `LocalPlugin::route` 的工具分支（`local/plugin.rs`）
- `WebPlugin::route` 的工具分支（`web/plugin.rs`）
- 装饰器 `PrefixedCapability`（`agent/host/scope.rs`）与 `SecureToolWrapper`
  （`local/plugin.rs`）**不拆不装**，`(args, env, ctx)` 原样透传。参数缺席（信封里没有
payload）⇒ `Value::Null`，仍由工具自己给出「缺少必填参数」的报错。

### 9.3 保住的语义

- `route()` 直连调用**自然静默**：`ExecEnv::from_request` 在没有 `EVENT_SINK` 时给出
  `Null` 出口、没有 `ABORT_SIGNAL` 时给出永不中止的信号。
- 工具仍然**不构造节点身份**：`failure_kind` + `prompt` 载荷契约不变，节点归
  `tool_executor`（§7.2）。
- 压缩路径仍然**出口静默**：`run_compression_llm` 收 `env`，调用方给
  `ExecEnv::new(EventSink::silent(), abort)`——「内部请求不产生可见事件」是类型上的选择。

---

## 10. SSE 增量解析

> 契约拆成「完整行」与「未结束行」两个方法，协议知识留在协议层；决策与理由见 [`docs/DECISIONS.md`](../../../../../docs/DECISIONS.md) ADR-022。

### 10.1 形状

新增 [`symbio_core/llm/sse.rs`](../../../symbio_core/llm/sse.rs)：

```text
trait SseLineParser {
    fn parse_line(&self, line: &str) -> Vec<ProtocolEvent>;              // 完整行
    fn open_partial_line(&self, head: &str) -> Option<Box<dyn PartialLineExtractor>>;  // 默认 None
}

trait PartialLineExtractor {
    fn push(&mut self, bytes: &str, out: &mut Vec<ProtocolEvent>);       // 只吃**新增**字节
}
```

- **默认 `None` 是合法降级**：不实现增量提取的协议一行代码都不用写，代价只是「等换行」。
- `push` 一次可以吐**多个**事件：同一块里可能一个字段收尾紧接下一个字段开头。
- `open_partial_line` 由 core **每行只问一次**：答 `None` 就记下、本行不再重试。

`utf8_chunk(buf, from)` 把「UTF-8 边界对齐」写进契约：被切断的多字节字符**不消费**，
留给下一次 `push`。SSE 分块由 TCP 决定，一个中文字符横跨两块是常态；若用
`from_utf8_lossy` 各替换出一个 U+FFFD，而完整行给出的是真字符，按前缀截断会吃字。

core 侧只做：按 `\n` 切行 → 交给 `parse_line` → 按前缀截断去重（`LineProgress`）
→ 尾巴交给提取器。

### 10.2 协议层：`JsonLineExtractor`

四个协议共用一个**逐字节推进**的 JSON 结构扫描器
（[`protocols/partial_json.rs`](../../../plugins/model/protocols/partial_json.rs)）。
它只做两件事：在 JSON 里定位「哪个字符串值」，以及按 JSON 转义规则解码它的增量；
「那个位置算内容还是推理」由协议实现 `PartialJsonSink` 决定。判据是**键路径全等**
（数组下标不占位）：

```text
openai_chat      ["choices","delta","content"]                          -> 内容
                 ["choices","delta","reasoning_content"]                -> 推理
                 ["choices","delta","tool_calls","function","arguments"]-> 参数(取最内层数组下标)
openai_responses ["type"] 先观察，再按它决定 ["delta"] 算什么
anthropic        ["type"] 必须是 content_block_delta，再看 ["delta", ...]
gemini           ["candidates","content","parts","text"]                -> 内容
```

`ModelProtocol` 以 `SseLineParser` 为**父 trait**，故 `BoundProvider::execute_turn` 直接把
协议实例交给 `parse_sse_stream`——core 与协议之间无闭包中转。

### 10.3 保住的语义

- **前缀截断去重机制不动**：`LineProgress` 按字段分别记账，完整行到达时按已发长度截断。
  增量文本「恰好是完整行文本的前缀」这条不变量**由协议层用同一套转义规则**保证。
- **`PARTIAL_LINE_MIN_BYTES = 256` 不动**：短尾巴等下一块补齐即可。
- **增量路径不产出** `Finish` / `Usage` / `Error` / `ResponseId`：半截 JSON 里这些
  字段的值不可信，它们只走完整行（由 `LineProgress::record` 丢弃非增量事件兜底）。
- **`[DONE]`、注释行、非 data 行不吐任何事件**（扫描器找不到 JSON 起点）。
- **协议判定「宁可漏，不可错」**：全等匹配、下标未知就放弃本次增量。错配的代价是
  前端多出内容且完整行截断会吃正文；不匹配的代价只是失去增量。

### 10.4 已知取舍

- 扫描器对**非法 JSON 转义**比 `serde_json` 宽松（分歧只在非法输入上，此时完整行
  路径本就不产出事件）；`output_index` 出现在 `delta` **之后**时本次增量被放弃
  （真实报文里它在前）；Gemini 的 `[{...},{...}]` 挤在一行非真实形状，增量路径不产出。

---

## 11. 工具调用协议封闭

> 工具名怎么发出去、怎么认回来，以及工具结果怎么读。三件事，共同点：它们都是
> 跨插件 / 跨层的**约定**，需要写成契约。名字归属的准入规则见
> [`docs/DECISIONS.md`](../../../../../docs/DECISIONS.md) ADR-023。

### 11.1 名字的线上形态：投影 + 查表，不反演

今天注册的能力名（`vdfs_read` / `shell` / `ask_user` / `todo_write` /
`codebase_search` / `web_search` / `http_request` / `heartbeat` / `agent_run` /
`mcp.<server>.<tool>`（带**点**）/ `agent_<safe_id>_<tool>`）里，**没有一个含 `/`**，
而真正违反协议字符集的是 MCP 的 `.`（OpenAI / Anthropic 只接受 `[A-Za-z0-9_-]`）。

因此**名字的线上形态由一对具名函数定义**（`symbio_core/capability/tool_name.rs`）：

```rust
pub fn to_wire(canonical: &str) -> String            // 字符集之外的字符一律 → "__"
pub fn resolve<'a>(wire: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str>
```

- 出方向：**一个具名函数**，字符集定义只有一份；
- 入方向：**查注册表**（`visitor.list_capability()` 的名字集合），不反演。**字面名优先**——
  `mcp__fs__read` 既可能是 `mcp.fs.read` 的线上形态、也可能本身就是这个名字，只有集合能回答；
- 解析失败 → **不猜**，按原样交给路由，由它给出诚实的 `NotFound`（反演猜错会调起**另一个工具**）；
- 命中判据是「解析成功」而不是「按线上名查表」——带非法字符的名字按线上名永远查不到，
  会白走一遍 `route` 回落。

**为什么在 core**：`to_wire`（`model` 出）与 `resolve`（`session` 入）是同一份契约的两半，
两个插件互相不可见，core 是唯一共同可见处——准入规则见
[`docs/DECISIONS.md`](../../../../../docs/DECISIONS.md) ADR-023。

### 11.2 工具结果：顺序即约定

`extract_result` **留在 `session/tool_executor.rs`**（唯一消费方是它自己），
判定顺序是**契约**（模块文档的表 + 两个字段名常量 + 用例钉住）：

```text
content 是字符串 → output 是字符串 → success 是布尔 → 顶层是字符串 → 整包 JSON
```

- 「**先命中者胜**」而不是「取第一个存在的键」：`{"content":[1,2],"output":"x"}`
  里 `content` 存在但不是字符串 ⇒ 继续走到 `output`；按「存在即取」会把数组原样丢给模型。
- 「结果文本本就没有契约」（工具返回任意 JSON，模型只消费一段文本）——这层适配是**显式**的。
- **控制流不得建立在这里**：要判「等待用户动作」用 `symbio_core::failure_kind`
  这个约定字段，不猜形状（§7.3）。

### 11.3 审批闸门：判两次，写一次

两条入口都真实存在（`route` 是注册表未命中的回落路径，`wrapper` 是注册表路径），故闸门
两处把守，但**判定逻辑**收成一个 `approval_gate`（两处传的都是工具真实描述，文案一致）。

### 11.4 MCP 侧的两条协议契约

- **协议字段名大小写**：`mcp/types.rs` 的协议类型带
  `#[serde(rename_all = "camelCase")]` 映射（MCP 规范用 camelCase）。缺失会让
  `initialize` 响应解析失败 ⇒ 该 server 的工具一个都不注册。
- **工具结果按载荷解析**：`McpToolCallResponse` 按规范载荷
  （`content` / `structuredContent` / `isError`）建模，不对 JSON-RPC 信封建模——
  否则全字段可选会「解析成功」并返回 `Value::Null`，正文永远渲染成字符串 `null`
  且不报错。`stdio.rs` / `http.rs` 只借它读 `isError`，其余原样返回；
  `capability.rs::execute` 把内容块压平成 `{"content": <文本>}` 再交出——
  **跨插件约定保持最窄**，会话层不需要认识 MCP 的「内容块数组」形状。

### 11.5 已知取舍

- **Gemini `functionResponse.name` 填的是 `tool_call_id`**（规范要求函数名；id 不是能力名，
  做线上形态换算无意义）。
- **MCP 工具名在前端显示为线上形态 `mcp__server__tool`**；前端不按 `mcp.` 前缀判断，只影响显示。
- **线上名投影不单射**（`a.b` 与 `a__b` 都得 `a__b`），由 `resolve` 的字面优先消解。

---

## 12. 发送方向的会话读取合并

> 本节覆盖 §1.1 的 **① 装配层**（`handle_chat_send_oneoff`）。

### 12.1 会话读取点

一次无工具调用的请求里，`load_session` 被触发的读取点：

| 读取点 | 读什么 | 次数 |
|---|---|---|
| `resolve_session_params` | `metadata` 的 mode / risk_level / provider_id | 1 |
| workdir 解析 | `metadata.workdir` | 1（`ctx[WORKDIR]` 缺失时） |
| `agent_id` 解析 | `metadata.agent_id` | 1 |
| `ensure_auto_title` | `metadata.title` + `messages` | 1 |
| `append_messages`（用户消息落库） | `messages`（读-改-写，必需） | 1 |
| `get_context_messages`（每轮） | `messages`（必需） | 1 |
| `persist_messages`（收尾落库） | `messages`（读-改-写，必需） | 1 |

前四个读的是**同一份元数据**，却各触发一次 `get_or_create_session`，而每次都把 **meta 与
消息两个文件整份读出**——即使调用方只需要 `metadata` 里的一个字符串。

### 12.2 请求级会话快照

`SessionSnapshot`（`orchestrator/entry.rs`）：`resolve_session_params` 的返回值带上
**本请求唯一一次**会话读取，后面所有元数据回退都从这一份读。

- `workdir`：只从 `params.meta_str("workdir")` 取，不另读；
- `agent_id`：**提前到派发前**解析（原在派发任务内），从同一份快照取；
- `ensure_auto_title`：快照里**已有标题**就整趟跳过（该函数的常见分支就是那次读取）。

### 12.3 为什么安全（等价性）

三个字段在本请求内**不会被本请求改写**（`append_messages` 只改 `messages`，
`ensure_auto_title` 只写 `metadata.title`），故派发前读一次与派发内读一次等价；共用一份
快照也比读三次更一致（分三次读可能取到被并发改动改写过的不同版本）。
`has_title` 的判据与 `ensure_auto_title` 内部的提前返回**逐字对齐**（`!s.trim().is_empty()`），
由用例 `has_title_matches_the_ensure_auto_title_criterion` 钉住——判错即「快照说已有
标题 → 跳过 → 实际从未命名」的静默丢命名。

### 12.4 会话读取次数

| | 新建会话 | 已有标题的会话 |
|---|---|---|
| `load_session` 次数 | **6** | **5** |

- 已省：`agent_id` 读取（两种场景）、已有标题时的 `ensure_auto_title`、`ctx[WORKDIR]`
  缺失时的 workdir。
- **没省**：新建会话上 `ensure_auto_title` 仍要读一次（需用刚落库的用户消息派生标题）。
- **不该省**：三处 `messages` 读取（两处写路径读-改-写、一处每轮上下文）须各取最新值。
- **不需要**：落库回包（把权威 `seq` 交回实时面，见
  [`vdfs-session-messages.md`](vdfs-session-messages.md) §3.4）不另读——`append_messages`
  直接把落库后的权威副本交回调用方。
