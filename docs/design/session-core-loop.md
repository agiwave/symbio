# Session 核心循环（LLM × 工具）梳理与收口设计

> 状态：梳理 + 设计（**待授权实施**）
> 范围：`symbio/src/plugins/session/` 中"调用大语言模型 + 执行工具"的会话主循环
> 审计基线 commit：`88a6800`
> 上游文档：`docs/architecture/session-mechanism-audit.md`（批次 A–D 已落地）、
> `symbio/src/plugins/session/README.md`（六大压缩策略）
> 本文不重复"压缩策略有哪些"，只回答一个问题：**主循环的结构是否收口**。

---

## 0. 一句话结论

主循环的**内核是健全的**（压缩四层、Turn 子树的完整性守卫、失败可恢复、Stop 恰好一次
这四件事都有真实故障背书，不能删）。但它的**控制流没有收口**：

- 循环体里平铺了 **23 个动作、6 类职责**（加载 / 检查 / 准备 / 推理 / 收尾 / 判定下一步），
  没有"前步骤 → loop → 后步骤"的分层；
- **10 个 `return` 出口**（另加 `close_turn` 内 2 个）各自重写一遍"Stop 钩子 + 落库 + 返回"，
  退出逻辑无单点；
- **压缩判定散在 3 处**（自动 / 水位提醒 / 主动工具拦截），三个阈值各判一次；
- **提示词与工具准备每轮重算**，`estimate_request_overhead` 一轮最多算 3 次；
- **不存在"启动条件"**——"工具结果齐备"是靠 `process_tool_calls_async` 同步阻塞
  隐式保证的，不是一个可读、可测的判据，因此**无法支持异步工具调用的唤醒语义**。

收敛方向：**四个收口点**（`gate_turn` / `prepare_turn_inputs` / `plan_compaction` /
`finish_turn`）+ 一个 `TurnState`，把 10 个出口压到 1 个，把 23 个动作压到 4 步。
预计 `chat_loop.rs` 生产代码 **−150 ~ −250 行**，且**决策声明点从 12 处降到 4 处**。

---

## 1. 现状：调用链全景

### 1.1 三层结构

```text
① 装配层   handle_chat_send_oneoff                orchestrator.rs:887
             解析 session_id / mode / risk / provider
             用户消息落库 + 自动命名 + agent_id 解析
             chat_ctx 装配（WORKDIR / SESSION_ID / AGENT_ID / SESSION_HANDLE）
             collect_capabilities(traverse) → CAPABILITY_VISITOR
             build_system_prompt（AGENTS.md 基础提示词）
             构造 model_chat::Request → run_chat_loop_task
② 宿主层   run_chat_loop_task                     orchestrator.rs:363
             WorkingGuard（panic 兜底）/ StopSignal / AiControlGuard
             provider 解析 + RATE_LIMITER + PluginChannel::pair
             spawn → ③（子任务）
             消费循环（orchestrator.rs:551）：收帧 → merge_message_patch
               → emit_message_patch / VDFS 变更 / persist_failure
③ 循环层   run_chat_loop                          chat_loop.rs:324   ← 本文对象
             + close_turn                         chat_loop.rs:748
             + auto_compress_process              chat_loop.rs:1145
             + compress_with_snapshot_core        chat_loop.rs:1231
             + run_context_compact                chat_loop.rs:1477
             + process_tool_calls_async           tool_executor.rs:568
```

**职责边界已经清楚**（③ 不再持有终态落库职责，终态唯一落库点在②的消费循环）；
问题只在 ③ 的内部组织。

### 1.2 核心 loop 逐行地图

**前步骤**（循环之前，`chat_loop.rs:329–409`）

| # | 动作 | 位置 |
|---|---|---|
| P1 | 解析 `model_chat::Request` | 329 |
| P2 | 配置快照：`max_tool_rounds` / `auto_compress` / `enable_compact_tool` | 341–351 |
| P3 | 计数器初始化：`tool_rounds` / `continuation_count` / `nudged_this_request` | 353–357 |
| P4 | `open_chat_session`（取 `SESSION_HANDLE`，缺则回退内存会话） | 359 |
| P5 | fade 参数（`fade_activate_rounds` / `fade_keep_recent_turns`） | 364–365 |
| P6 | `abort_flag` + `SessionContext` 构造 | 366–372 |
| P7 | **resume 处理**（`process_resume`）→ `Continue` / `Done` / `Err` | 385–409 |

**循环体**（`chat_loop.rs:411–730`）

| # | 动作 | 位置 | 职责类别 |
|---|---|---|---|
| L1 | `get_context_messages` 加载历史（轮次窗口） | 417–426 | 加载 |
| L2 | 首轮追加 `single_message` + `UserPromptSubmit` 钩子 | 431–442 | 准备 |
| L3 | `last_saved` 落库锚点 | 444 | 收尾 |
| L4 | 软上限检查 → **出口 ①** | 448–465 | 退出 |
| L5 | abort 检查 → **出口 ②** | 467–472 | 退出 |
| L6 | 日志 `TURN n START` | 474–480 | — |
| L7 | abort 检查（与 L5 重复）→ **出口 ③** | 482–485 | 退出 |
| L8 | `resolve_system_prompt`（每轮重算） | 491–497 | 准备 |
| L9 | `auto_compress_process`（**压缩入口 ①**） | 498–526 | 压缩 |
| L10 | 水位提醒（nudge）判定：55% + 请求级一次性 | 536–550 | 压缩 |
| L11 | `emit_streaming_start`（建 Turn 根节点） | 552–553 | 准备 |
| L12 | `list_capability()` 汇集工具 + 条件注入 `context_compact`（每轮重拉） | 555–565 | 准备 |
| L13 | `retention` 解析 + `build_request_view`（**请求视图唯一入口**） | 577–605 | 准备 |
| L14 | abort 检查 → **出口 ④** | 615–618 | 退出 |
| L15 | `provider.execute_turn` | 620–630 | 推理 |
| L16 | 错误分支：`RetryWithoutContextId` → `continue`；`Aborted` → **出口 ⑤**；其他 → **出口 ⑥** | 632–665 | 退出 |
| L17 | abort 检查 → **出口 ⑦** | 667–674 | 退出 |
| L18 | 取出 `out` 的字段（`finish` / `usage` / `had_tool` / 子节点 id） | 676–683 | 收尾 |
| L19 | `finalize_assistant_turn`（Turn 收尾状态机） | 685–687 | 收尾 |
| L20 | `feedback_estimate`（用量反馈校准 token 估算） | 690 | 收尾 |
| L21 | `into_messages` → 追加进 `context.messages` | 692–696 | 收尾 |
| L22 | `finish == length` 打标 | 699–707 | 收尾 |
| L23 | `close_turn(...)` → `Finish`（**出口 ⑧**）/ `NextTurn` | 709–729 | 收尾 + 判定 |

**后步骤**：**不存在**。循环只能由 `return` 离开，收尾动作分散在 L3/L23 与 `close_turn` 内部。

**`close_turn`**（`chat_loop.rs:748–1016`，14 个参数）

| 分支 | 动作 | 位置 |
|---|---|---|
| 无工具 | `length` 自动续写（≤3 次）→ `NextTurn`；续写耗尽 / 参数截断 → 报错 | 767–813 |
| 无工具 | `persist_messages` + `fire_stop_hook` → **出口 ⑨** | 805–812 |
| 有工具 | `context_compact` 拦截分流（**压缩入口 ②**） | 821–942 |
| 有工具 | `process_tool_calls_async`（**串行阻塞**，tool_executor.rs:590） | 944–951 |
| 有工具 | 父节点状态 `update_messages` | 958–980 |
| 有工具 | `persist_messages` | 982 |
| 有工具 | `needs_user_action` → `fire_stop_hook` → **出口 ⑩** | 988–1004 |
| 有工具 | `tool_rounds += 1` → `NextTurn` | 1008–1015 |

### 1.3 与四条标准的差距

#### 差距 1 · 结构上没有"前 / 中 / 后"分层

循环体 23 个动作里，只有 1 个是真正的推理（L15），其余 22 个是准备与收尾。
用户要的形状是：

```text
前步骤1 / 前步骤2 / …
loop
  步骤1 / 步骤2 / …
end loop
后步骤1 / …
```

现状是"所有步骤都在 loop 里，且前/后步骤各占一半"。

#### 差距 2 · 四处未收口

| 应收口项 | 现状（声明点） | 具体问题 |
|---|---|---|
| **压缩** | `should_start_compression`（compression.rs:259）+ `should_emit_context_nudge`（compression.rs:309）+ `MIN_COMPACT_TOKENS`（compression.rs:28）+ `find_compress_split_point`（compression.rs:111）；调用点在 `chat_loop.rs:498`、`chat_loop.rs:536`、`chat_loop.rs:835` | 内核已收口（`compress_with_snapshot_core` 是唯一实现 ✓），但**判定**分散在 3 个入口、阈值各判一次；`find_compress_split_point` 与 `find_turn_user_split_idx` 是两个"切分点"函数 |
| **提示词 / 工具准备** | `resolve_system_prompt`（chat_loop.rs:298，循环内 L491 调用）+ `list_capability`（循环内 L555）+ `retention` 解析（循环内 L581）+ `estimate_request_overhead`（compression.rs:239，被 3 处调用） | 同一请求内 system_prompt / tools 恒定，却每轮重算；overhead 一轮最多算 3 次（nudge / auto_compress / 压缩核心） |
| **启动条件检查** | **不存在** | 循环无条件进入；"工具结果齐备"由 `process_tool_calls_async` 同步阻塞隐式保证 |
| **退出循环** | 10 个出口（chat_loop.rs:400/405/462/470/483/617/656/661/672/727）+ `close_turn` 内 2 个（806/1002） | 每个出口各自重写 `fire_stop_hook` + `persist_messages` + `return`；新增出口必须记得补齐三件套 |

#### 差距 3 · 工具执行同步串行，不支持异步唤醒

`process_tool_calls_async` 在 `close_turn` 内**顺序 `await`** 每个工具
（tool_executor.rs:590 `for tc in tool_calls`）。多个工具调用必须全部跑完才回到循环，
因此"启动条件"隐式成立但**不可见、不可测、无法改造成异步**。

#### 差距 4 · `close_turn` 的 14 个参数是"循环体被机械搬出"的痕迹

`#[allow(clippy::too_many_arguments)]`（chat_loop.rs:747）掩盖了参数全部来自循环作用域
（`context` / `channel` / `abort_flag` / `tool_rounds` / `last_saved` / `continuation_count` /
`nudged_this_request` …）。这说明它本质仍是循环体的一部分，只是被搬成了函数。

---

## 2. 收口点设计

### 2.1 收口 ①：启动条件 + 退出条件 → `gate_turn`

```rust
/// 单轮是否允许发起 LLM 请求（启动条件）以及是否必须退出（退出条件）。
/// 唯一调用点：主循环顶部。
enum Gate {
    Proceed,                  // 条件齐备 → 进入准备与推理
    WaitForTools,             // 工具结果未齐备（异步工具在途）→ 不唤醒
    Exit(TurnExit),           // 软上限 / abort / 不可恢复
}

fn gate_turn(
    req: &TurnRequest,        // 请求级快照（configured_max_tool_rounds 等）
    context: &SessionContext,
    turn: &TurnState,
    abort: &AtomicBool,
) -> Gate
```

**启动条件的唯一判据**（对应用户第 3 条）：

```rust
/// 历史中每个 ToolCall 是否都已有配对结果子节点。
/// 这是"可以唤醒主循环"的充要条件——多个工具调用全部结束才唤醒，
/// 任一未完成都不唤醒。
pub fn tool_results_complete(messages: &[ChatMessage]) -> bool {
    let mut pending: HashSet<&str> = messages
        .iter()
        .filter(|m| m.msg_type == Some(MessageType::ToolCall))
        .map(|m| m.id.as_str())
        .collect();
    for m in messages {
        if let Some(pid) = m.parent_id.as_deref() {
            pending.remove(pid);
        }
    }
    pending.is_empty()
}
```

> **同一语义当前已有三份独立实现**，收口后应全部改为引用本判据：
> - `compression::find_compress_split_point` 的"尾部未配对 ToolCall 检查"
>   （compression.rs:118–138）——只查尾部，是本判据的弱化版；
> - `chat_session::drop_orphan_messages`（chat_session.rs:189）——反向（剔孤儿）；
> - `resume::process_retry_turn` 的 BFS 子节点收集（resume.rs:111–120）。
>
> ⚠️ **必须配修复路径**：若某个 ToolCall 永远等不到结果（进程被杀 / 工具任务 panic），
> `tool_results_complete` 会永久为假 → 主循环永久挂起。当前靠
> `tool_executor::record_protocol_failure`（tool_executor.rs:501）在写入时合成失败结果来
> 维持不变式，异步化之后必须保留这条路径，并补一个超时兜底
> （超时后为悬空 ToolCall 合成失败结果，再放行）。

### 2.2 收口 ②：提示词 / 工具 / 请求视图 → `TurnInputs`

```rust
/// 一次 LLM 请求所需的全部输入。
struct TurnInputs {
    system_prompt: String,
    tools: Vec<CapabilityMeta>,                        // 含条件注入的 context_compact
    retention: HashMap<String, ToolContextRetention>,  // 由 tools 派生，不单独维护
    request_view: Vec<ChatMessage>,                    // build_request_view 产物
    overhead_tokens: usize,                            // system_prompt + tools，只算一次
    plan: CompactionPlan,                              // 收口 ③ 的产物
}

/// 请求内恒定部分（前步骤算一次）
struct RequestInvariants { system_prompt: String, tools: Vec<CapabilityMeta>, retention: …, overhead_tokens: usize }

async fn prepare_invariants(ctx: &TurnCtx, context: &SessionContext) -> RequestInvariants;
/// 每轮重建部分（依赖 tool_rounds 与消息历史，必须每轮算）
async fn prepare_turn_inputs(ctx: &TurnCtx, inv: &RequestInvariants, context: &SessionContext, turn: &TurnState) -> TurnInputs;
```

- `resolve_system_prompt` / `list_capability` / `retention` 解析 / `estimate_request_overhead`
  → 进 `prepare_invariants`（**前步骤**，一次）。
- `build_request_view` / nudge 注入 / 压缩判定 → 进 `prepare_turn_inputs`（**每轮**，因为
  它们依赖 `tool_rounds` 与当前消息历史，无法提升）。

> ⚠️ 把 `tools` 提升到前步骤是**行为等价性待确认项**：`list_capability()` 读的是编排器一次性
> 装配的 `CAPABILITY_VISITOR`，正常路径下请求中途不会变；但若 MCP 在会话中途连上新服务端，
> 现状能下一轮就看见新工具、提升后看不见。实施时二选一：
> ① 确认 visitor 请求内不可变 → 直接提升；
> ② 保守起见只提升 `system_prompt` 与 `overhead_tokens`（这两者确定不变），`tools` 仍每轮取。

### 2.3 收口 ③：压缩 → `CompactionPlan`

```rust
/// 本轮压缩决策（唯一产出点）。
enum CompactionPlan {
    None,                                                  // 不动
    Nudge,                                                 // 55%：视图末尾注入提醒（不落库）
    Auto { msg: ChatMessage, keep: Vec<ChatMessage> },     // 70%：被动语义压缩
    Manual { split_idx: usize, hints: Option<String> },    // 主动 context_compact（工具拦截）
}

fn plan_compaction(
    messages: &[ChatMessage],
    limit: usize,
    overhead: usize,
    flags: CompactionFlags,   // auto_compress / nudge_enabled / nudged_this_request / force
) -> CompactionPlan;

/// 执行（唯一实现，三个入口共用）
async fn apply_compaction(plan: CompactionPlan, …) -> CompactionOutcome;
```

- 现状：判定在 `chat_loop.rs:498`（auto）、`chat_loop.rs:536`（nudge）、`chat_loop.rs:835`（manual）
  三处，阈值在 compression.rs 三处。
- 收口后：**一个 `plan_compaction()` 决策 + 一个 `apply_compaction()` 执行**；
  自动 / 主动工具 / 紧急兜底（`emergency_tail_compression`）只是 plan 的不同来源。
- `compress_with_snapshot_core`（唯一实现 ✓）与 `emergency_tail_compression`（唯一实现 ✓）
  保持不动，只把"什么时候调用它们"合并到一处。

### 2.4 收口 ④：退出 → `TurnExit` + `finish_turn`

```rust
/// 主循环的唯一退出原因。
enum TurnExit {
    Completed,                     // 正常收尾（无工具调用 / 待用户输入）
    MaxToolRounds { max: usize },  // 显式软上限
    Aborted,                       // 用户中止 → 冒泡 Err(Aborted)
    LlmFailed(PluginError),
    ResumeDone,                    // resume 已完成，无需进入循环
}

/// 唯一收尾点：Stop 钩子（幂等）+ 增量落库 + 返回。所有出口只做一件事：构造 TurnExit。
async fn finish_turn(
    orch: &ChatOrchestrator,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    turn: &TurnState,
    exit: TurnExit,
) -> Result<(), PluginError>;
```

- 10 个出口 + `close_turn` 内 2 个 → **1 个**。
- `StopSignal` 的幂等 + RAII 兜底**保留不动**（它已保证"恰好一次"）；`finish_turn` 只是把
  显式触发点变成单点，让 RAII 兜底退化为真正的"异常兜底"而非常规路径。
- `TurnExit::Aborted` → `Err(PluginError::Aborted)`，消费循环的 ABORTED 分支语义不变。

### 2.5 顺带：`TurnState` 取代 14 个参数

```rust
/// 跨轮累加的可变状态（原散落在循环作用域 + close_turn 参数表）。
struct TurnState {
    tool_rounds: usize,
    continuation_count: u32,
    nudged_this_request: bool,
    last_saved: usize,
}
```

`close_turn` 的 14 个参数 → `(orch, ctx, channel, context, turn, out)` 6 个。
`#[allow(clippy::too_many_arguments)]` 可以摘掉。

---

## 3. 目标代码骨架

```rust
pub async fn run_chat_loop(
    orch: &ChatOrchestrator,
    ctx: Arc<dyn InvokeRequest>,
    mut channel: PluginChannel,
) -> Result<(), PluginError> {
    // ── 前步骤 ────────────────────────────────────────────────
    let req   = TurnRequest::parse(&ctx)?;                    // P1–P2 请求 + 配置快照
    let mut context = SessionContext::new(open_chat_session(&ctx).await);  // P4
    let mut turn = TurnState::new(&req);                      // P3 / P5 / P6

    // P7 resume：一次性，可提前收尾
    if let Some(exit) = handle_resume(orch, &ctx, &mut channel, &mut context, &req).await? {
        return finish_turn(orch, &mut context, &mut channel, &turn, exit).await;
    }

    let inv = prepare_invariants(&ctx, &context).await;       // 收口②：提示词/工具/overhead

    // ── 主循环 ────────────────────────────────────────────────
    loop {
        // 步骤 1 · 启动条件 + 退出条件（收口①）
        match gate_turn(&req, &context, &turn, &abort_flag) {
            Gate::Exit(exit)      => return finish_turn(orch, &mut context, &mut channel, &turn, exit).await,
            Gate::WaitForTools    => { wait_all_tools_done(&context).await; continue; }
            Gate::Proceed         => {}
        }

        // 步骤 2 · 本轮输入（收口②③）：压缩决策 + 视图重建 + nudge
        let inputs = prepare_turn_inputs(&ctx, &inv, &mut context, &mut channel, &turn).await?;

        // 步骤 3 · 推理
        let out = match execute_turn(orch, &ctx, &inputs, &mut channel, &abort_flag).await {
            Ok(out) => out,
            Err(StepError::RetryWithoutContextId) => { reset_context_ids(&mut context).await; continue; }
            Err(StepError::Fatal(e)) => {
                return finish_turn(orch, &mut context, &mut channel, &turn, TurnExit::from(e)).await
            }
        };

        // 步骤 4 · 收尾 + 下一步判定（收口③④）
        match settle_turn(orch, &ctx, &mut context, &mut channel, &mut turn, out).await? {
            Flow::Next(exit) => return finish_turn(orch, &mut context, &mut channel, &turn, exit).await,
            Flow::Continue   => continue,
        }
    }
    // ── 后步骤 ────────────────────────────────────────────────
    // 无：循环只能由 return 离开，所有出口都经 finish_turn。
}
```

对照用户要的形状：

```text
前步骤：parse 请求 → 开会话 → TurnState → resume → prepare_invariants
loop
  步骤1  gate_turn            启动条件 + 退出条件
  步骤2  prepare_turn_inputs  压缩决策 + 提示词/工具/视图准备
  步骤3  execute_turn         LLM 调用
  步骤4  settle_turn          工具分发 + 落库 + 下一步判定
end loop
后步骤：finish_turn（唯一出口，被所有 return 复用）
```

---

## 4. 异步工具调用：启动条件与唤醒

对应用户第 3 条。分两级落地，**级别 1 零行为变更，级别 2 是真正的异步化**。

### 级别 1 · 显式化启动条件（低风险，建议先做）

- 把"工具结果齐备"从隐式（`process_tool_calls_async` 同步等待）变为**显式判据**
  `tool_results_complete()`，在 `gate_turn` 里作为启动条件。
- 执行模型**不变**（工具仍在 `settle_turn` 里跑完才回循环），因此
  `gate_turn` 恒返回 `Proceed`，行为等价。
- 收益：判据单点、可单测、为级别 2 铺路；顺带收敛 3 份重复实现。

### 级别 2 · 工具执行移出 `settle_turn`，改由"全部完成"事件唤醒

```text
loop {
    match gate_turn(...) {
        WaitForTools => await all_tools_done.notified(),   // 部分完成不唤醒
        Proceed      => { …推理… }
        Exit(e)      => return finish_turn(e),
    }
    // settle_turn 只做三件事：
    //   ① spawn 每个工具（各自独立子任务，完成后写结果子节点 + 计数/通知）
    //   ② 落库当前进度（persist_messages）
    //   ③ 判定：全部完成 → 循环回 gate_turn 放行；未完成 → 回 gate_turn 走 WaitForTools
}
```

- **唤醒语义**：`settle_turn` 派发 N 个工具后立即回到循环；`gate_turn` 检查
  `tool_results_complete()`——只要还有 ToolCall 没有结果子节点就返回 `WaitForTools`，
  主循环挂起在通知上；**最后一个工具写入结果时通知唤醒**，此时判据才为真。
  这正是"多个工具调用结束后会唤醒主循环，不完整是不唤醒"。
- 收益：多工具调用**并发执行**（不再串行等待）；等待期主循环可响应 abort。
- **必须保留的语义**（否则会回归）：
  1. `interactive` 模式下"前一个工具待审批/失败 → 中止本批剩余"（tool_executor.rs:594–617）
     ——顺序语义与并发互斥，`interactive` 必须继续走串行路径；
  2. `record_protocol_failure`（tool_executor.rs:501）为 id/name 非法的调用合成失败结果，
     保证"没有悬空 ToolCall"这一不变式；
  3. `TOOL_STREAM_IDLE_TIMEOUT_SECS`（180s）与 `TOOL_EXEC_HARD_TIMEOUT_SECS`（600s）
     两级超时，避免单个工具挂死导致 `WaitForTools` 永不满足；
  4. 父节点状态补丁（`parent_updates` → `update_messages`）与结果子节点的写入顺序。

> 建议：级别 2 单独一个批次，且先在 `auto` 模式上启用，`interactive` 保持串行。
> 若只想要"结构清晰"而不想动执行模型，**只做级别 1** 也能拿到第 1、2、4 条的全部收益。

---

## 5. 分步实施计划（每批独立可交付、可回滚）

| 批次 | 内容 | 行为变更 | 风险 |
|---|---|---|---|
| **A** 骨架分层 | `TurnState` 替代 14 参数；`finish_turn` 收口 12 个出口；`gate_turn` 收口启动/退出条件（恒 `Proceed`，级别 1） | 无 | 中：需覆盖 abort / 软上限 / resume / 待用户输入 4 类出口的回归测试 |
| **B** 输入收口 | `prepare_invariants` + `prepare_turn_inputs`；`overhead_tokens` 只算一次；`retention` 从 `tools` 派生 | 无（`tools` 是否提升见 §2.2 的待确认项） | 低 |
| **C** 压缩收口 | `plan_compaction` 合并 3 个判定点；`apply_compaction` 合并调用；`tool_results_complete` 收敛 3 份重复实现 | 无（阈值与文案逐字不变） | 低 |
| **D** 异步工具（级别 2） | `settle_turn` spawn 工具 + 通知唤醒；`interactive` 保持串行 | **有**：工具并发执行 | 高：需真实并发场景验证 + 超时/失败路径回归 |

每批完成后必跑（见 `docs/design/session-perf.md` 与项目门禁）：

```bash
# 工作区根 = D:\Bing\symbio\symbio（仓库根无 Cargo.toml）；cli/ 是独立 workspace，两处各跑一遍
cargo check --tests
cargo test --lib          # 基线 2026-09-15：528 passed
cargo clippy --all-targets -- -D warnings
# 前端（若涉及）
node node_modules/vue-tsc/bin/vue-tsc.js --noEmit -p tsconfig.json
node node_modules/vitest/vitest.mjs run    # 基线 18 文件 / 146 用例
node scripts/grep-audit.mjs && node scripts/style-audit.mjs
```

**建议先 `git commit` 当前状态再动刀**（本文基线 `88a6800`，工作树干净）。

---

## 6. 不要动的部分（明确保留）

| 机制 | 保留理由 |
|---|---|
| 压缩四层（L0 守卫 / L1 自动 / L2 主动 / 本地紧急兜底） | 每层对应一种真实故障，合并会造成"超限即死锁"回归 |
| `compress_with_snapshot_core` 单一实现 | 被动与主动共用内核已是正确收敛，只缺"判定"收口 |
| `StopSignal` 幂等 + RAII 兜底 | "一个请求生命周期内 Stop 恰好一次"由生命周期保证，不是人工记忆 |
| `WorkingGuard` / `AiControlGuard` | panic / 提前 return 下 `is_working` 与 `ai_control_tx` 的收敛保证 |
| `finalize_assistant_turn` 的 reasoning-only 分支 | 防前端双份文本（真实事故） |
| Turn 子树完整性守卫（`find_turn_user_split_idx`） | provider 400 的直接防线 |
| `feedback_estimate` 两条不变式 | 破坏任一条都会静默劣化水位判定 |
| 消费循环（orchestrator.rs:551）的帧合并与 `persist_failure` 作用域收窄 | 终态落库唯一入口，189-Turn 误回滚事故的修复 |
| `tool_executor` 的三级超时与失败信息性语义 | 防挂死 + 失败必须回传模型继续 |

---

## 7. 验收标准

| 批次 | 可机械核对的验收项 |
|---|---|
| A | `run_chat_loop` 内 `return` 语句数 ≤ 2（`finish_turn` 调用点）；`close_turn` 参数 ≤ 6；`#[allow(clippy::too_many_arguments)]` 在 `chat_loop.rs` 内为 0；新增出口测试覆盖 4 类 `TurnExit` |
| B | `estimate_request_overhead` 在 `run_chat_loop` 单轮路径上的调用次数 = 1；`resolve_system_prompt` 调用次数 = 1/请求 |
| C | `should_start_compression` / `should_emit_context_nudge` 的调用点各 = 1（且都在 `plan_compaction` 内）；`tool_results_complete` 被 `find_compress_split_point` / `drop_orphan_messages` / `resume` 引用 |
| D | 多工具调用场景下工具执行总耗时 ≈ max(单个耗时) 而非 sum；`interactive` 模式顺序语义回归测试通过 |
| 全程 | `cargo test --lib` 用例数 ≥ 528 且 0 failed；`clippy --all-targets -- -D warnings` 零告警 |

---

## 附：证据索引（文件:行）

- `chat_loop.rs:324` — `run_chat_loop` 入口
- `chat_loop.rs:385–409` — resume 前步骤（含出口 ①②）
- `chat_loop.rs:411–730` — 主循环体（23 个动作 / 10 个出口）
- `chat_loop.rs:467` / `:482` — 相邻的两次 abort 检查（重复）
- `chat_loop.rs:491` — `resolve_system_prompt` 每轮重算
- `chat_loop.rs:555` — `list_capability()` 每轮重拉
- `chat_loop.rs:577–605` — `build_request_view` 调用点
- `chat_loop.rs:748` — `close_turn` 定义（14 参数）
- `chat_loop.rs:767–813` — 无工具收尾 + 出口 ⑨
- `chat_loop.rs:821–942` — `context_compact` 拦截（压缩入口 ②）
- `chat_loop.rs:988–1004` — `needs_user_action` 出口 ⑩
- `chat_loop.rs:1145–1205` — `auto_compress_process`（压缩入口 ①）
- `chat_loop.rs:1231–1458` — `compress_with_snapshot_core`（唯一实现 ✓）
- `chat_loop.rs:1477–1524` — `run_context_compact`
- `compression.rs:111` — `find_compress_split_point`（尾部未配对检查）
- `compression.rs:183` — `find_turn_user_split_idx`（第二个切分点函数）
- `compression.rs:239` — `estimate_request_overhead`
- `compression.rs:259` / `:309` — 两个阈值判定
- `compression.rs:542` — `build_request_view`（请求视图唯一入口 ✓）
- `tool_executor.rs:568` — `process_tool_calls_async`
- `tool_executor.rs:590` — 串行 `for tc in tool_calls`
- `tool_executor.rs:594–617` — interactive 模式顺序语义
- `tool_executor.rs:501` — `record_protocol_failure`（悬空 ToolCall 的修复路径）
- `chat_session.rs:189` — `drop_orphan_messages`
- `resume.rs:111–120` — BFS 子节点收集
- `orchestrator.rs:363` — `run_chat_loop_task`
- `orchestrator.rs:551` — 消费循环
- `orchestrator.rs:887` — `handle_chat_send_oneoff`（会话唯一编排入口）
