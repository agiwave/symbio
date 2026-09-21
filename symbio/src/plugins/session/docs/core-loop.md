# Session 核心循环（LLM × 工具）梳理与收口设计

> 状态：**批次 A/B/C 已落地**（`cargo check --tests` 通过）；批次 D（异步工具并发）待授权；
> **批次 E（执行期出口/信号双原语，§8）、批次 F（工具侧收敛到同一出口，§9）、
> 批次 G（流式工具接上出口、执行期通道归零，§10）与批次 H（两个执行接口签名统一，§11）
> 已落地**。
> 这四批改的是**宿主层与执行层之间的协议**（`PluginChannel` 双职责拆为
> `EventSink` + `AbortSignal`，工具不再把通道当事件流，执行层不再有帧循环，
> `Capability::execute` 与 `ModelProvider::execute_turn` 收成同一形状），
> 不改本文 §2 的四个收口点，但 §1.1 的 ② 与 §3 代码骨架已按其更新。
> 代码位置：其后又做了模块拆分（S2），`chat_loop.rs` **2000 → 434 行**，拆出
> `chat_loop/{state,inputs,turn,compress,io}.rs`（见 [`./module-layout.md`](./module-layout.md) §4.2）。
> 本文的"文件:行"只作**历史取证**，请按**符号名**检索。
> 范围：`symbio/src/plugins/session/` 中"调用大语言模型 + 执行工具"的会话主循环
> 审计基线 commit：`88a6800`（设计稿）→ 实施基线 `5c60806`（本设计文档首次提交）
> 上游文档：`docs/archive/implementation-logs/session-mechanism-audit.md`（批次 A–D 已落地，已归档）、
> `../README.md`（六大压缩策略）
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

收敛方向：**四个收口点**（`gate_turn` / `prepare_turn_inputs` / `apply_compaction` /
`finish_turn`）+ 一个 `TurnState`，把 10 个出口压到 1 个，把 23 个动作压到 6 步。

> **实施后的实际账（相对初稿预估的更正）**：初稿预估"生产代码 −150 ~ −250 行"，
> **未达成**。实测 `chat_loop.rs` 代码行 **+136**（新增 362 / 删除 226；另有注释
> +175 行）。原因是收口本身要付出"新类型 + 新函数"的成本：
> `TurnState` / `TurnRequest` / `TurnExit` / `Gate` / `TurnResult` 五个新类型
> 与 `gate_turn` / `finish_turn` / `settle_reasoning` / `prepare_turn_inputs` /
> `apply_compaction` 五个新函数，都是从原来"内联在循环体里"变成"有名字、可单测"。
>
> 真正的收益不在总行数，而在**局部复杂度**：
> - `run_chat_loop` 本体：**407 → 248 行**；
> - 其中 **loop 循环体：320 → 171 行（−47%）**；
> - 收尾三件套的独立出现点：**12 → 1**；
> - 压缩判定点：**3 → 1**；提示词/工具收集点：**3 → 1**；启动/退出判定点：**4+ → 1**。
>
> 即：**"决策声明点"与"收尾点"大幅收敛，代价是引入了一层显式的类型与函数边界。**
> 若后续仍要求净减行数，应从注释密度与 `close_turn`（仍 300+ 行）入手，而非回退收口。
>
> 另：初稿的 `plan_compaction` 收口点**并入 `apply_compaction`**（纯决策拆分不成立，
> 见 §2.3）；批次 C 的 `tool_results_complete` 收敛项**已撤销**（不存在三份重复实现，
> 见 §2.1 更正说明）。

---

## 1. 现状：调用链全景

### 1.1 三层结构

```text
① 装配层   handle_chat_send_oneoff                orchestrator.rs:887
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
             select! 三臂：任务结束 / 1800s 看门狗 / 状态被接管（见 §8）
               → 收尾：定稿在途 / persist_failure / 结局字段
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
| P6 | `AbortSignal` + `SessionContext` 构造 | 366–372 |
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
| 有工具 | 父节点终态快照同步进内存镜像（整条替换，随 persist 落库） | 958–980 |
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
（`context` / `sink` / `abort` / `tool_rounds` / `last_saved` / `continuation_count` /
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
/// 本轮是否还有在途工具（已派发、尚未产出结果）。
/// 非空 ⇒ 不唤醒：多个工具调用**全部**结束才唤醒主循环，不完整不唤醒。
///
/// 判据取**运行时在途集合**（`TurnState::in_flight_tools`），不是"扫历史看
/// ToolCall 有没有结果子节点"——见下方更正说明。
fn gate_turn(req: &TurnRequest, turn: &TurnState) -> Gate {
    if !turn.in_flight_tools.is_empty() {
        return Gate::WaitForTools;
    }
    …
}
```

> ### ⚠️ 更正（实施批次 A 时发现，原设计判断有误）
>
> 本节初稿把启动判据写成 `tool_results_complete(messages)`——「历史中每个 ToolCall
> 都有配对结果子节点」，并称"同一语义当前已有三份独立实现"。**两条都不成立**：
>
> 1. **「ToolCall 无结果」在本代码库是合法状态**，不能当作"未就绪"。
>    交互模式下工具批被用户审批中断时，本批剩余 ToolCall 会被**有意留空**，由请求
>    视图层 `flatten_chat_messages`（`plugins/model/message_builder.rs:188`）合成占位
>    tool 结果（"[工具 X 的执行被中断，未产生结果。]"）喂回模型。按历史判定会让
>    approve/reject 恢复后的续写被**永久挡住**。
> 2. **不存在三份重复实现**。真正"ToolCall 是否有结果子节点"的实现只有一处——
>    `message_builder::find_tool_result`（`message_builder.rs:294`），且它在 model
>    插件、属请求扁平化职责。原设计点名的三处是**三个不同谓词**：
>    - `compression::find_compress_split_point` 的尾部检查（compression.rs:118–138）
>      ——"**末节点**是否为未配对 ToolCall"，只看尾巴一格；
>    - `chat_session::drop_orphan_messages`（chat_session.rs:189）——"`parent_id`
>      是否存在于本批集合"，父子存在性不动点，与工具无关；
>    - `resume::process_retry_turn` 的 BFS（resume.rs:111–120）——收集子孙节点，
>      是树遍历而非配对判定。
>
> 因此**批次 C 取消该收敛项**（无重复可收敛），启动条件改用运行时集合。
>
> ⚠️ **必须配修复路径**：若某个工具永远等不到结果（进程被杀 / 工具任务 panic），
> 在途集合会永久非空 → 主循环永久挂起。当前靠
> `tool_executor::record_protocol_failure`（tool_executor.rs:501）在写入时合成失败结果来
> 维持不变式，异步化之后必须保留这条路径，并补一个超时兜底
> （超时后为悬空 ToolCall 合成失败结果，再从在途集合移除）。

### 2.2 收口 ②：提示词 / 工具 / 请求视图 → `prepare_turn_inputs`

**硬约束：系统提示词与工具信息必须同机制、同时机、始终一起收集。**
两者都是经 `CapabilityVisitor` 注册、经 `traverse` 汇集到同一个 visitor 上的
「会话输入」，来源与生命周期完全一致；把它们拆成两处、两个时机收集，会让
"这一轮模型看到的人格"与"这一轮模型能调的工具"出现不一致窗口。

**时机：保持现状（每轮 Turn 开头，在循环内）**，理由与权衡如下。

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
    ctx: &Arc<dyn InvokeRequest>,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    turn: &mut TurnState,
    req: &TurnRequest,
) -> Result<TurnInputs, TurnExit>;
```

> **实施记录（批次 B 已落地）**：`retention` 不进入 `TurnInputs`——它只是
> `build_request_view` 的一个入参，由 `tools` 就地派生后立即消费，暴露到边界反而
> 多一份需要维护的中间态。`overhead_tokens` 同理：它只被本步内的 ③（自动压缩阈值）
> 与 ④（水位提醒）消费，故保留为函数内局部量（**仍然只算一次**，这是收口目标本身）。
> `plan: CompactionPlan` 留待批次 C 引入。
>
> 配套改动：`compression::estimate_overhead_with_tools(system_prompt, &tools)` 从
> `estimate_request_overhead` 中拆出（后者保留为「自取工具 + 转交」的便捷包装），
> 使主循环能把已收集的工具清单直接复用，而不是让 visitor 注册表被查三遍。

收口的三块：`resolve_system_prompt` + `list_capability`（**必须成对，同一时刻**）、
`retention` 解析、`estimate_request_overhead`（当前一轮最多算 3 次：
nudge / auto_compress / 压缩核心各一次 → 收口后只算一次并复用）。

#### 时机权衡（明确记录，不擅自改变现状）

| | 循环外收集一次（前步骤） | **循环内每轮收集（现状，本轮保持）** |
|---|---|---|
| 每轮可动态调整人格 / 工具 | ✗ | ✓ |
| 与 `traverse` 收集通道的语义一致 | 部分（只反映请求开始时刻的注册表） | ✓（每轮反映当轮注册表） |
| 每轮开销 | 省（一次 `list_capability` / `list_system_prompts`） | 每轮一次 visitor 查询 |
| 中途新增工具（如 MCP 新连上服务端）可见性 | 本轮不可见 | 下一轮即可见 |

**判断**：`list_capability` / `list_system_prompts` 都是纯内存读 visitor 注册表，
不触发 I/O；相比之下每轮省下的这点开销相对一次 LLM 推理可忽略。
**因此"每轮动态可调"这个能力净收益为正，本轮保留在循环内。**

> 何时才考虑提升到前步骤：若将来出现①单轮工具轮次极多（如数百轮）导致
> visitor 查询成为可观开销，或②明确要求"一次请求内人格与工具集冻结"以消除
> 中途变更带来的不可复现性——届时再单独评估，**且必须两者一起提升**，
> 不允许只提升其一。

### 2.3 收口 ③：压缩 → `CompactionPlan`

```rust
/// 收口 ③：上下文水位的**唯一响应点**——判定与执行同处一地，顺序写死。
///
/// 返回：是否需要在请求视图末尾注入一次性水位提醒。
async fn apply_compaction(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn InvokeRequest>,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    turn: &mut TurnState,
    req: &TurnRequest,
    overhead_tokens: usize,
) -> Result<bool, TurnExit> {
    // ① L1 自动语义压缩（70% 触发；就地改写 context.messages 并推进落库锚点）
    // ② 水位提醒（55% 触发；判**压缩之后**的真实水位，不落库，交视图层追加）
    …
}
```

> **与初稿的差异（实施后修正）**：初稿设计为 `plan_compaction()`（纯决策）+
> `apply_compaction()`（执行）两个函数，并枚举
> `CompactionPlan { None, Nudge, Auto, Manual }`。实施时发现该拆分**不成立**：
>
> 1. `Auto` 与 `Nudge` 不是并列选项，而是**有顺序依赖的两步**——先按 70% 压缩，
>    再按压缩后的水位判 55% 提醒。纯决策函数无法表达"决策 → 执行 → 再决策"的链，
>    只能退化为两次调用（等于没收口）；而若把两个判定都提前到压缩之前，提醒就会
>    基于压缩前的高水位触发，出现"刚压完立刻提醒"的自相矛盾。
> 2. `Manual`（模型主动调 `context_compact`）由**模型输出**触发，决策点在 LLM 响应
>    之后的 `close_turn` 工具拦截处，物理上不可能进入"请求前"的 plan。
>
> 故收敛为**单个 `apply_compaction`**：判定与执行同处一地，两步顺序由函数内部的
> 代码行序保证（收口前该顺序只由主循环体的行序隐含保证，一次重排就能无声破坏）。

- 现状：判定在 `chat_loop.rs:498`（auto）、`chat_loop.rs:536`（nudge）、`chat_loop.rs:835`（manual）
  三处，阈值在 compression.rs 三处。
- 收口后：
  - **水位响应**（auto + nudge）→ 唯一入口 `apply_compaction`；
  - **执行内核** → `compress_with_snapshot_core`（唯一实现 ✓，保持不动）；
  - **收益护栏** → `compression::has_compaction_payoff`（原先 auto / manual 两处
    各写一遍 `tokens < MIN_COMPACT_TOKENS`，门槛语义有两个出处）；
  - **"何时压"仍有两个入口**（水位触发 / 模型主动）——这是产品语义决定的，不强行
    合并；被合并的是"怎么压"（1 个实现）与"水位怎么响应"（1 个入口）。
  - 顺带：`auto_compress_process` 的 `extra_hints` 参数删除（自动路径没有"用户"
    在环内提供保留提示，该参数在本路径恒为 `None`），参数 8 → 7，`#[allow(clippy::too_many_arguments)]`
    随之去掉。`force` 保留——它是 `should_start_compression` 的公开契约且有单测背书。

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

## 3. 落地后的代码骨架

> 以下为**批次 A/B/C 落地后**的实际形态（已与代码对齐，非设计稿）。
> 模块拆分（S2）后各符号的落点：`run_chat_loop` / `gate_turn` / `finish_turn` / `TurnFlow`
> 在 `chat_loop.rs`；`TurnRequest` / `TurnState` / `TurnExit` / `Gate` / `TurnResult` /
> `ChatOrchestrator` 在 `chat_loop/state.rs`；`prepare_turn_inputs` / `apply_compaction`
> 在 `chat_loop/inputs.rs`；`close_turn` / `settle_reasoning` 在 `chat_loop/turn.rs`。

```rust
pub async fn run_chat_loop(
    orchestrator: &ChatOrchestrator,
    ctx: Arc<dyn InvokeRequest>,
    sink: EventSink,          // 事件唯一出口（见 §8）
    abort: AbortSignal,       // 中止唯一入口（见 §8）
) -> Result<(), PluginError> {
    // ── 前步骤 ①：请求解析与请求级配置快照 ──────────────────────────────
    let mut req: model_chat::Request = ctx.payload()?;
    let turn_req = TurnRequest::new(&req);          // unwrap_or(cfg) 只求值一次

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
            Gate::WaitForTools => return finish_turn(…, TurnExit::Completed).await,   // 级别 2 改为等待唤醒
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

        // 步骤 4 · 推理
        let result = orch.provider.execute_turn(&inputs.system_prompt, &inputs.request_view,
                                               &inputs.tools, &inputs.root_id,
                                               &sink, &turn.abort).await;
        let out = match result {
            Err(RetryWithoutContextId) => { /* 清 response_id + 半截 Streaming */ continue; }
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
        match close_turn(orch, ctx.clone(), &sink, &mut context,
                         &mut turn, result, &turn_req).await {
            TurnFlow::NextTurn        => {}
            TurnFlow::Finish(exit)    => return finish_turn(orch, &context, &sink, &turn, exit).await,
        }
    }
    // ── 后步骤 ──────────────────────────────────────────────────────────
    // 无：循环只能由 return 离开，所有出口都经 finish_turn（唯一收尾点）。
}
```

对照用户要的形状：

```text
前步骤：parse 请求 → 开会话 → TurnState → resume
loop
  步骤1  加载本轮上下文
  步骤2  gate_turn            启动条件 + 退出条件（收口①）
  步骤3  prepare_turn_inputs  提示词/工具收集 + 开销 + 压缩 + 视图（收口②③）
  步骤4  execute_turn         LLM 调用
  步骤5  settle_reasoning     推理产物并入上下文
  步骤6  close_turn           工具分发 + 落库 + 下一步判定（收口④）
end loop
后步骤：finish_turn（唯一出口，被所有 return 复用）
```

---

## 4. 异步工具调用：启动条件与唤醒

对应用户第 3 条。分两级落地，**级别 1 零行为变更，级别 2 是真正的异步化**。

### 级别 1 · 显式化启动条件（低风险）—— ✅ 已随批次 A 落地

- 把"工具结果齐备"从隐式（`process_tool_calls_async` 同步等待）变为**显式判据**
  `TurnState::in_flight_tools`（运行时在途集合），在 `gate_turn` 里作为启动条件。
- 执行模型**不变**（工具仍在 `close_turn` 里跑完才回循环），因此
  `gate_turn` 恒返回 `Proceed`，行为等价。
- 收益：判据单点、可读、为级别 2 铺路。
- ⚠️ 初稿此处写"顺带收敛 3 份重复实现"——经核实**不成立**，已在 §2.1 的更正说明中撤回。

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
  `TurnState::in_flight_tools`——只要还有工具未产出结果就返回 `WaitForTools`，
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
  4. 父节点终态快照（`parent_updates` 整条替换进内存镜像）与结果子节点的写入顺序。

> 建议：级别 2 单独一个批次，且先在 `auto` 模式上启用，`interactive` 保持串行。
> 若只想要"结构清晰"而不想动执行模型，**只做级别 1** 也能拿到第 1、2、4 条的全部收益。

---

## 5. 分步实施计划（每批独立可交付、可回滚）

| 批次 | 内容 | 行为变更 | 风险 |
|---|---|---|---|
| **A** 骨架分层 | `TurnState` 替代 14 参数；`finish_turn` 收口 12 个出口；`gate_turn` 收口启动/退出条件（恒 `Proceed`，级别 1） | 无 | 中：需覆盖 abort / 软上限 / resume / 待用户输入 4 类出口的回归测试 |
| **B** 输入收口 | 单一 `prepare_turn_inputs`：**系统提示词与工具同函数、同时刻收集**（同一 `CapabilityVisitor`），再派生 `retention` / `overhead_tokens`（一轮只算一次）与请求视图 | 无（时机保持循环内，见 §2.2 权衡） | 低 |
| **C** 压缩收口 | `apply_compaction` 合并水位响应（auto 70% + nudge 55%，顺序写死）；`has_compaction_payoff` 收敛收益护栏；`auto_compress_process` 去掉恒为 `None` 的 `extra_hints` | 无（阈值与文案逐字不变） | 低 |
| **D** 异步工具（级别 2） | `settle_turn` spawn 工具 + 通知唤醒；`interactive` 保持串行 | **有**：工具并发执行 | 高：需真实并发场景验证 + 超时/失败路径回归 |

> **批次 C 的收敛项已缩减**（相对初稿）：`tool_results_complete` 那一项**取消**——
> 经核实不存在"三份重复实现"，详见 §2.1 更正说明。`plan_compaction` 那一项**并入
> `apply_compaction`**——纯决策拆分不成立，详见 §2.3 实施记录。

每批完成后必跑（见 `./perf.md` 与项目门禁）：

```bash
# 工作区根 = D:\Bing\symbio\symbio（仓库根无 Cargo.toml）；cli/ 是独立 workspace，两处各跑一遍
cargo check --tests
cargo test --lib          # 基线 2026-09-15：528 → 2026-09-16：532 passed
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check  # ⚠️ 必须用 cargo fmt（rust-toolchain.toml 锁 1.93.1），
                            #    不要用裸 rustfmt（会走 default toolchain，版本漂移）
# 前端（若涉及）
node node_modules/vue-tsc/bin/vue-tsc.js --noEmit -p tsconfig.json
node node_modules/vitest/vitest.mjs run    # 基线 18 文件 / 146 用例
node scripts/grep-audit.mjs && node scripts/style-audit.mjs
```

> ⚠️ **跑 cargo 门禁别接 `| tail -N`**：管道会缓冲到进程结束才输出，3–4 分钟的构建
> 全程零输出，与"卡死"无法区分（2026-09-16 那次"卡死 6m23s"即此假象）。
> 改为 `> 日志文件 2>&1` 重定向，后台跑，等完成通知再读日志。
>
> 📌 **存量问题（不属本设计范围）**：`cargo fmt --all -- --check` 在 HEAD 有 **9 处**偏差
> —— `plugin.rs` 5 / `store/mod.rs` 1 / `store/tests.rs` 1 / `symbio_core/vdfs/host.rs` 2。
> 即 CI 的 fmt 门禁在改动前就是红的；`cargo fmt --all` 一条命令可修，宜作独立 commit。

**建议先 `git commit` 当前状态再动刀**（本文基线 `88a6800`，工作树干净）。

---

## 6. 不要动的部分（明确保留）

| 机制 | 保留理由 |
|---|---|
| 压缩四层（L0 守卫 / L1 自动 / L2 主动 / 输入超限预判） | 每层对应一种真实故障。**注意预判层只保留"跳过注定失败的请求"，不再本地截断历史**（该兜底已删除，见 `node-state-streaming.md` §6 S20.8 与 `docs/DECISIONS.md` ADR-018） |
| `compress_with_snapshot_core` 单一实现 | 被动与主动共用内核已是正确收敛，只缺"判定"收口 |
| `StopSignal` 幂等 + RAII 兜底 | "一个请求生命周期内 Stop 恰好一次"由生命周期保证，不是人工记忆 |
| `WorkingGuard` / `AbortGuard` | panic / 提前 return 下 `is_working` 与**中止信号登记**的收敛保证（`AbortGuard::disarm` 注销登记**并置位信号**，显式承担历史上「通道 drop ⇒ 中止」的隐式语义，见 §8） |
| `finalize_assistant_turn` 的 reasoning-only 分支 | 防前端双份文本（真实事故） |
| Turn 子树完整性守卫（`find_turn_user_split_idx`） | provider 400 的直接防线 |
| `feedback_estimate` 两条不变式 | 破坏任一条都会静默劣化水位判定 |
| 消费循环（`orchestrator/consume.rs`）的 `select!` 三臂与 `persist_failure` 作用域收窄 | 终态落库唯一入口，189-Turn 误回滚事故的修复 |
| `tool_executor` 的三级超时与失败信息性语义 | 防挂死 + 失败必须回传模型继续 |

---

## 7. 验收标准（批次 A/B/C 已按实测值填写）

> 说明：本表初稿写的是**设计意图**下的目标值，其中若干项（如"`return` ≤ 2"）
> 事后看是**过度约束**——真正的目标是"收尾动作只出现一次"，不是"字面上只有一个
> `return`"。下表已改为**可机械核对且已实测**的口径。

| 批次 | 验收项 | 实测 |
|---|---|---|
| A | 收尾三件套（软上限提示 / 增量落库 / Stop 钩子）在 `chat_loop.rs` 内的**独立出现点** = 1 | ✅ `fire_stop_hook` 全文件仅 1 处调用（`finish_turn` 内） |
| A | `run_chat_loop` 内所有 `return` 均为 `return finish_turn(...)`（无旁路出口） | ✅ 11 处 `return`，全部经 `finish_turn` |
| A | `close_turn` 参数 ≤ 7（clippy 阈值内，无需 `#[allow]`） | ✅ 7 个；`#[allow(clippy::too_many_arguments)]` 已摘除 |
| A | 启动条件与退出条件集中在单一函数 | ✅ `gate_turn`（`Gate::{Proceed, WaitForTools, Exit}`） |
| B | 主路径 `estimate_request_overhead` 调用次数 = 0（改用 `estimate_overhead_with_tools`，一轮 1 次） | ✅ 仅剩压缩摘要请求预判 1 处（不同 system prompt，属另一请求） |
| B | `resolve_system_prompt` 与 `list_capability` 同函数、同一时刻（源码相邻） | ✅ `prepare_turn_inputs` ① |
| B | 请求视图入口唯一 | ✅ `build_request_view` 全文件仅 1 处调用 |
| C | `should_emit_context_nudge` 调用点 = 1 | ✅ `apply_compaction` 内 |
| C | `should_start_compression` 生产调用点 = 1 | ✅ `auto_compress_process` 内（被 `apply_compaction` 独占调用） |
| C | `tokens < MIN_COMPACT_TOKENS` 字面比较 = 0 | ✅ 统一走 `has_compaction_payoff` |
| 全程 | `cargo check --tests` 零告警 | ✅ 12.65s / 12.80s |
| 全程 | `cargo clippy --all-targets -- -D warnings` 零告警 | ✅ 48.5s（顺带修掉 `plugin.rs:1103` 一处**存量** `needless_borrow`） |
| 全程 | `cargo test --lib` 用例数 ≥ 528 且 0 failed | ✅ 532 passed / 0 failed（107.55s / 109.93s） |
| 全程 | `cargo fmt --all -- --check`（改动文件零偏差） | ✅ `chat_loop.rs` / `compression.rs` 零偏差；`plugin.rs` 仅剩 5 处**存量**偏差（见上方 📌） |
| D | 多工具调用总耗时 ≈ max(单个) 而非 sum；`interactive` 顺序语义回归 | ⏸ 未开始（批次 D 待授权） |

---

## 8. 批次 E：执行期出口/信号双原语（已落地）

> 这一批不动 §2 的四个收口点，改的是**宿主层与执行层之间的协议**：
> `PluginChannel` 曾经**同时**承担「出（事件）」「入（中止）」两种职责，本批把它拆成
> 两个语义单一的原语。决策与理由见 [`docs/DECISIONS.md`](../../../../../docs/DECISIONS.md) ADR-019。

### 8.1 原语

| 原语 | 方向 | 形状 | 位置 |
|---|---|---|---|
| `EventSink` | 出 | `Direct(Arc<dyn TranscriptWriter>)` \| `Null` | `symbio_core/exec.rs` |
| `AbortSignal` | 入 | `Arc<AtomicBool>` + `CancellationToken` 合一 | `symbio_core/exec.rs` |

- **`EventSink::Direct`** 直连转写唯一写入点（`TranscriptSink` → `Transcript::apply`），
  进程内**零 serde**；**`EventSink::Null`** 是「本次调用不产生可见事件」的**类型级**表达
  （上下文压缩、`resume` 重跑工具用它，取代历史上的「哑通道 + drain task」）。
- **`AbortSignal::abort()`** 是唯一置位入口，置位即唤醒（`CancellationToken`），
  因此**不再有任何轮询**——历史上并存的「Abort 帧 / 100ms 标志位轮询 / `cancel_token`」
  三源收敛为一条。`PluginChannel` 退回**纯跨进程传输**（前端实时面用），不再承担执行期协议。

### 8.2 链路对比

```text
改造前：parse_sse_stream → emit_append(serde#1) → PluginFrame → 消费循环
        from_value::<NodeOp>(serde#2) → Transcript::apply → publish_frame(serde#3) → 前端

改造后：parse_sse_stream → sink.emit(NodeOp) → TranscriptSink → Transcript::apply → 前端
        （后端段两跳纯开销消失；进程内调用不再付进程外的 serde 代价）
```

### 8.3 宿主层的连带简化

| 项 | 改造前 | 改造后 |
|---|---|---|
| 消费循环 | 嵌套双层 `spawn` + `join` + keepalive sender + 每帧 `from_value` 分派 | 一次 `spawn` + `select!` 三臂（任务结束 / 1800s 看门狗 / 状态被接管） |
| 压缩静默 | 哑 sender + drain task + `mem::replace(rx)` 所有权交换 | `EventSink::silent()` |
| `resume` 重跑工具静默 | 临时 `PluginChannel::pair(64)` + drain task | `EventSink::silent()` |
| 中止 | 投 Abort 帧 + 收帧 + 置位标志（三源并存） | `AbortSignal::abort()` 一次调用 |
| 中止登记 | `ai_control_tx: Option<Sender>` | `abort_signal: Option<AbortSignal>`（`AbortGuard` 管登记） |

**`AbortGuard::disarm` 注销登记时一并 `abort()`**——这是把历史上「通道被 drop ⇒ 对端
读到关闭 ⇒ 视作中止」这条**隐式**语义显式化。它由测试直接锁定
（`orchestrator.test.rs::disarm_aborts_the_signal`），不靠读代码。

### 8.4 保住的语义（未变）

- 转写仍只有**一个写入点**；`Warn` 仍是会话级状态（不进转写）——分派规则逐字未变，
  只是从消费循环搬到了 `TranscriptSink`。
- 「中止不是失败」：会话结局是 `aborted`（不是 `completed`），在途根 Turn 定稿 `Aborted`
  （重试入口不消失）。
- `handle_abort` 的 3s 兜底判据（登记变 `None`）与语义保留。
- 工具经**自有** `PluginPayload::Session` 通道回传事件的能力完整保留（尚未收敛，见 §9）。

### 8.5 验证

- `node scripts/gate.mjs` **32/32**；`cargo test --lib` **811 passed / 0 failed**。
- **CLI 端到端**（`--provider LMStudio`，本地模型）：单条流式 / `--repl` 多轮 /
  工具调用（`vdfs_list` 两轮）/ 流式工具（`cmd`）四场景全部 EXIT=0。
- 中止路径 CLI 无入口（CLI 只在收到会话节点 `outcome == aborted` 时渲染，不主动发起），
  改由插件级端到端用例锁定：`orchestrator.test.rs::handle_abort_signals_registered_turn_and_converges`。

---

## 9. 批次 F：工具侧收敛到同一出口（已落地）

> 批次 E 把**出/入两个方向**从通道里拆了出来，但**工具**还没接上去：`shell` /
> `agent_run` / `ask_user` / 交互审批仍然返回 `PluginPayload::Session`，由
> `execute_tool_async` 把帧解回来、改 id、再转发一遍。本批把工具接到同一个出口上。

### 9.1 工具怎么拿到出口

出口与中止信号**经 `ctx` 传递**（键 `symbio_core::EVENT_SINK` / `ABORT_SIGNAL`），
不给 `Capability::execute` 加参数：

```rust
// 编排层（execute_tool_async）：发起工具前写入
tool_ctx.set(crate::symbio_core::EVENT_SINK, sink.clone());
tool_ctx.set(crate::symbio_core::ABORT_SIGNAL, abort.clone());

// 工具侧（只读）：缺席 ⇒ 静默 / 永不中止
let sink = EventSink::of(&*ctx);      // §11 后：改由分发点装配成 env，工具读 env.sink()
let abort = AbortSignal::of(&*ctx);   // §11 后：同上，工具读 env.abort()
```

**为什么走 `ctx` 而不是加参数**（**本批的历史理由，已被 §11 修订**）：`execute(ctx)`
的入参是**请求信封**（`PATH` / `trace_id` / `payload` / 会话上下文），而出口是
**执行期**的，与「这次调用从哪条路径来」无关——同一个 `shell` 既可能被编排层调用
（有出口），也可能被 `route()` 直接调用（无出口 ⇒ 静默）。用 `ctx` 承载就不必为
「有没有出口」造第二条调用路径。

> **§11 的修订**：这个**目标**（不为「有没有出口」造第二条调用路径）保留，但**手段**
> 换成了「显式 `env` 参数 + 缺席降级」——因为把出口藏在 `ctx` 里让每个工具都得自己
> 记键名，且与 `execute_turn` 的显式参数形态分裂。现在工具侧读的是 `env.sink()` /
> `env.abort()`，装配收口在 `invoke_capability` 一处。

### 9.2 「工具只声明意图，节点归编排层」

`ask_user` 与交互审批（`local/plugin.rs` 的 `confirm_prompt_payload`）**不再构造
节点**，只返回载荷：

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

**这条规则消灭的是一类 bug，不是一个分支**：历史上工具自造一个随机 id 的节点，
消费方再改成 `result_msg_id` 重播一遍 ⇒ 同一个逻辑节点在前端有两个 id ⇒ 重复审批卡，
且 resume 只删得掉一个（后端 messages 仅一份），另一个永远留在前端 store。
现在 `PendingPrompt` **没有 id 字段**，那类 bug 在类型层面不成立。

### 9.3 `failure_kind` 成为共享闭集

`symbio_core::capability::failure_kind` 收口了这组词（`ERROR` / `NEEDS_APPROVAL` /
`NEEDS_INTERACTION` / `PERMISSION_DENIED` / `TOOL_UNAVAILABLE`），判据只有一条
`is_pending()`。此前生产方（`local`）与消费方（`session`）各写各的字面量，
而编排层里已有一处硬编码判定——加一个「等待用户」的 kind 就会漏改它，
表现是**交互模式下本批剩余工具照跑**（用户本该逐个处理却收到一堆并发审批）。

它**不是**跨栈闭集（前端无同名常量镜像），因此不进 `protocol-mirror-audit` 的 C 组。

### 9.4 本批的账

| 项 | 变化 |
|---|---|
| `local/plugin.rs::emit_confirm_prompt` | 建通道 → 发帧 → drop tx → 返回 rx；**改为** `confirm_prompt_payload`（纯返回值，无 async、无通道） |
| `local/ask_user.rs::execute` | 同上；删掉 `ChatMessage` / `PluginChannel` / `PluginFrame` / `session_chat_response` 五个导入 |
| `tool_executor.rs` | `execute_tool_async` 第三返回值 `Option<ChatMessage>` → `Option<PendingPrompt>`；新增 `pending_prompt_from` / `pending_from_message` / `build_user_prompt_message` |
| 双份审批检查点 | `LocalPlugin::route` 与 `SecureToolWrapper::execute` 两处仍各判一次（属独立问题，本批未合） |

### 9.5 验证

- `node scripts/gate.mjs` **32/32**；`cargo test --lib` **811 passed / 0 failed**。
- 新增 6 个用例：`pending_prompt_from_only_accepts_the_pending_kinds`（**反面**：
  信息性 kind 不得判为等待用户，否则自动模式会话永久挂起）、
  `build_user_prompt_message_uses_the_orchestrator_owned_ids`、
  `pending_from_message_extracts_payload_not_identity`（`tool_executor.test.rs`）；
  `interactive_mode_returns_pending_intent_not_a_channel`、
  `auto_mode_returns_continuable_error_not_pending`、`too_few_options_is_rejected`
  （新建 `local/ask_user.test.rs`）。

### 9.6 未收敛（已由批次 G 完成）

`shell` 与 `agent_run` 的**流式增量**当时仍走 `PluginPayload::Session`。它们的自造
复杂度在注释里写着：「executor 是先拿到 Session(rx) 返回值后才开始消费通道，因此
执行体必须 spawn 到后台，否则输出帧超过通道容量（64）时 pump 阻塞在 send、
execute() 不返回、executor 不消费 → **死锁**」。→ **§10**：两条工具接上出口后，
`PluginChannel::pair(64)`、`cancel_token` 克隆、「有 `RESULT_MSG_ID` 才走流式」的
分支与 `execute_tool_async` 的 `Session` 分支整块消失。

---

## 10. 批次 G：流式工具接上出口，执行期通道归零（已落地）

> 批次 F 把**审批/提问**接到了同一出口，但**流式增量**还没接：`shell` 与 `agent_run`
> 仍返回 `PluginPayload::Session`，`execute_tool_async` 里还有一整段帧循环在替它们
> 转发节点。本批是这一系列**最大的一笔删除**。

### 10.1 两条流式工具的改法

**`shell`（单进程内自足）**：pump 任务直接写出口，执行体不再 spawn。

| 收口前 | 现在 |
|---|---|
| `Capability::execute` 判 `RESULT_MSG_ID`/`TOOL_CALL_ID` 是否齐备 → 分流「流式 / 非流式」两条路径 | **一条路径**：`env.sink()` / `env.abort()` / `SnapshotTarget::from_ctx` 三个读取，缺席即降级（§11 前读的是 `EventSink::of` / `AbortSignal::of`，等价） |
| 执行体 `tokio::spawn` 到后台（否则通道满 ⇒ 死锁） | 直接 `await`（出口没有背压） |
| 增量走 `tx.send(PluginFrame::Data(NodeOp…))`，`send` 失败置 `consumer_gone` | `sink.emit(NodeOp::Upsert{…})`；**没有**「消费端还在吗」这种判断——没有可关闭的通道 |
| 末尾发哨兵帧 `{"content": full}` | `Ok(Response{exit_code, output, risk_level})`——返回值就是结果 |
| `execute_inner`（非流式等待式路径） | 删除 |

`SnapshotTarget`（`msg_id` + `tool_call_id` **同行**）承载「增量快照的节点身份」，
由编排层提供；**缺席 = 直连调用**（`route()`，如 MCP 网关）⇒ 不发增量。工具不得
自造节点身份——与 §9.2 同一条规则。

**`agent_run`（子会话转播）**：转播桥成为「子会话 → 父视图」的**唯一翻译点**。

收口前翻译散在**两处**——转播桥原样转发节点，`tool_executor` 再对每一帧做二次分派
（`user` 角色过滤、`parent_id` 锚定、`Assistant → Tool` 角色改写、`Reset`/`Warn` 丢弃）。
两处翻译 ⇒ 规则会漂移；而且执行层必须为「工具可以返回事件流」这件事付出整段帧循环
的代价。现在翻译只剩转播桥一处，出口下游只做落地。

转播桥的返回值从「帧」变成具名结局：

```rust
enum RelayOutcome {
    Done(String),                             // 子会话跑完，最终答复
    Pending { text, prompt, failure_kind },   // 待用户动作（只给载荷，不给节点）
    Failed(String),                           // 子会话以错误结束
}
```

`Pending` 与批次 F 的规则同源：**工具只声明意图**。子会话冒泡的审批节点**不落出口**，
只把 `text` / `prompt`（本工具的续跑参数）/ `failure_kind` 回传——节点由
`process_tool_calls_async` 以统一 id 构造。若这里也落一个（子会话自己的 id），
批次 F 刚消灭的「同一逻辑节点两个 id」会从这个入口重新长回来。

### 10.2 顺带修掉的语义回退：总时长上限 → 空闲上限

`agent_run` 收口前经通道回传，**逃过了** `execute_tool_async` 的 600s 总时长上限
（它落在 180s 空闲上限的那条循环里）。接上出口后它被内联等待，就会被总时长误杀——
一个跑十几分钟但**一直在发事件**的子智能体，与一个真的挂死的工具，在「总时长」
这一维上完全同形。

因此本批把判据换成**出口的发射计数**（`EventSinkProgress`，挂在出口上——出口本来
就是「工具还活着」的唯一证据源，因为工具唯一的出方向动作就是 `emit`）：

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

于是同一个数字覆盖两种情形，且**不再需要为长任务开特例**：不发事件的工具
（`web_search` / MCP / `codebase_search`…）读数恒为 0，退化为「总时长上限」——
与历史口径一致；会发事件的工具只在**真的沉默** 600s 后被杀。历史并存的两个魔法数
（600s 总时长 + 180s 流式空闲）收成一个。

### 10.3 本批的账

| 项 | 变化 |
|---|---|
| `tool_executor.rs::execute_tool_async` | **删除 `PluginPayload::Session` 分支**（含 ~150 行 `NodeOp` 转发块、`user` 角色过滤、`parent_id` 锚定、`Assistant→Tool` 改写、`Reset`/`Warn` 分派）；删 `TOOL_STREAM_IDLE_TIMEOUT_SECS` / `emit_tool_update` / `pending_from_message` |
| `local/shell.rs` | 删 `execute_inner`、`PluginChannel`/`PluginFrame`/`mpsc`/`CancellationToken` 导入、`consumer_gone` 标志、哨兵帧、后台 `spawn`；新增 `SnapshotTarget` |
| `agent/host/subagent.rs` | 删 `PluginChannel::pair(512)`、`send_frame`、错误帧转播、`META_PROMPT`；转播桥改签名（`sink` + `abort` + `tool_call_id`）并返回 `RelayOutcome`；新增 `abort.cancelled()` 臂（出口没有「对端关闭」这种隐式中止信号） |
| `symbio_core/exec.rs` | 新增 `EventSinkProgress`（出口的发射计数），`EventSink::Direct` 携带它 |
| `local/shell.rs` 测试 | 拆到 `local/shell.test.rs`（内联 `mod tests` 棘轮 **53 → 52**） |

### 10.4 保住的语义（未变）

- **增量是累积全量快照**（`role=tool` + `status=Streaming`），节流 120ms；
- **子会话节点锚定**：顶层节点 `parent_id → tool_call_id`、`Assistant → Tool`；
- **子会话的 `user` 消息不透传**（内容已见于 ToolCall 请求参数，且临时 `role=user`
  节点会在前端获得必然失败的"编辑"入口）；
- **子会话的 `Reset` / `Warn` 不透传**（作用域不同；且出口的 `Direct` 实现会把 `Warn`
  分派到**本会话**节点上——透传等于把子会话告警记到父会话头上）；
- **待审批的优先级**：错误 > 待审批 > 文本。

### 10.5 验证

- `node scripts/gate.mjs` **32/32**；`cargo test --lib` **817 passed / 0 failed**。
- CLI 端到端（`--provider LMStudio`，`.symbio/model/LMstudio`）：普通流式对话、
  交互模式 `cmd` 流式、交互模式 `ask_user`、自动模式 `ask_user`，全部 `EXIT=0`。
- 新增用例：`local/shell.test.rs` **9 个**（pump 三态：有身份成帧 / 无身份不发 /
  静默不发；execute 四态：Data 返回 / 直连静默 / 大量输出不阻塞 / 参数非法报错；
  ctx 读取口径两态）、`exec.test.rs::sink_progress_counts_every_emit`、
  `tool_executor.test.rs::subagent_pending_payload_is_recognized_as_pending_intent`
  （**跨文件契约**：`agent_run` 的待审批载荷必须被编排层认出来——两边各自演化时
  这条契约最容易静默断掉，表现是审批卡直接消失且不报错）。

### 10.6 收口后的链路

工具执行不再有任何「帧」的概念（**§11 后签名已换成 `(args, env, ctx)`**，
下图是批次 G 收口时的形态）：

```
工具 execute(ctx)
  ├─ 读 ctx：EVENT_SINK / ABORT_SIGNAL / (RESULT_MSG_ID, TOOL_CALL_ID)
  ├─ sink.emit(NodeOp) ─────────────► 转写唯一写入点（进程内，零 serde）
  └─ 返回 PluginPayload::Data ──────► execute_tool_async
                                        └─ 结果节点 / 父终态（唯一写入者）
```

批次 H（§11）之后同一张图变成：

```
invoke_capability(cap, ctx)                 ← 唯一「拆信封」点
  ├─ args = ctx.payload::<Value>() ?? Null
  ├─ env  = ExecEnv::from_request(&*ctx)    ← 出口 + 中止，缺席 ⇒ 静默 / 永不中止
  └─ cap.execute(args, &env, ctx)
        ├─ 读 ctx：RESULT_MSG_ID / TOOL_CALL_ID / WORKDIR…（只读真正需要的）
        ├─ env.sink().emit(NodeOp) ───► 转写唯一写入点（进程内，零 serde）
        └─ 返回 Value ────────────────► PluginPayload::Data
                                          └─ execute_tool_async
                                               └─ 结果节点 / 父终态（唯一写入者）
```

`PluginPayload::Session` 剩下的用途**只有跨进程**：`session/stream`（前端实时面）、
`event_bus/subscribe`、vdfs 网关、CLI/前端客户端。执行期与它再无关系。

---

## 11. 批次 H：两个执行接口签名统一（已落地）

### 11.1 问题：数据面统一了，控制面没有

E/F/G 三批把**数据面**（通道 → 出口/信号）收干净了，但**签名**没动：

| | `Capability::execute`（改前） | `ModelProvider::execute_turn`（改前） |
|---|---|---|
| 出口/中止怎么到 | 藏在 `ctx` 的两个无名键里 | **显式参数** `sink` / `abort` |
| 参数怎么到 | `ctx.payload::<Value>()?`（无类型，各工具自读） | 显式类型化参数 |
| 返回值 | `PluginPayload`（4 变体，工具只用 `Data`） | `TurnOutput`（类型化） |
| 信封 | `ctx` 把「路由信封」与「执行期上下文」混在一个键值袋 | 无 |

后果是**每个工具都得记住「我该读哪些键」**：`ctx.payload()` 拿参数、
`ctx.get(EVENT_SINK)` 拿出口、`ctx.get(ABORT_SIGNAL)` 拿中止、`ctx.get(WORKDIR)`
拿工作目录——这四件事混在同一个袋子里，看不出哪两个是执行期必备、哪两个是可选上下文。
而且「同一个执行期概念有两种到达方式」（一处经 ctx、一处经显式参数），
新人改一处必然漏另一处。

### 11.2 形状：`ExecEnv` 具名化，两个接口同形

新增 [`ExecEnv { sink, abort }`](../../../symbio_core/exec.rs)——「一次带中止的流式执行」
的**出/入两个方向**，两处共用：

```text
Capability::execute(args, env, ctx)      -> Result<Value, PluginError>
ModelProvider::execute_turn(inputs, env) -> Result<TurnOutput, PluginError>
```

差别只剩 `ctx`，而这是**真实差异**：工具是**被路由、被注册**的（要转发
`session/chat/send`、要解析 VDFS 挂载），所以还需要信封；模型执行不被路由，
也就没有信封。不强行抹平。

### 11.3 唯一「拆信封」的地方

`invoke_capability(cap, ctx)`（`symbio_core/capability.rs`）是**唯一**把
`Result<Value, _>` 装回 `PluginPayload` 的地方。所有分发路径都必须经它：

- `DefaultToolVisitor::invoke`（`symbio_core/tools.rs`）
- `LocalPlugin::route` 的工具分支（`local/plugin.rs`）
- `WebPlugin::route` 的工具分支（`web/plugin.rs`）
- 装饰器 `PrefixedCapability`（`agent/host/scope.rs`）与 `SecureToolWrapper`
  （`local/plugin.rs`）**不拆不装**，`(args, env, ctx)` 原样透传

```text
invoke_capability(cap, ctx)
  ├─ args = ctx.payload::<Value>().unwrap_or(Value::Null)
  ├─ env  = ExecEnv::from_request(&*ctx)   // 缺席 ⇒ 静默 / 永不中止
  └─ cap.execute(args, &env, ctx).map(PluginPayload::new)
```

参数缺席（信封里没有 payload）⇒ `Value::Null`：与收口前各工具自己的
`unwrap_or(Value::Null)` 口径逐字一致，仍由工具自己给出「缺少必填参数」的报错。

### 11.4 本批的账

- **24 个 `impl Capability` 换签名**：其中 2 个用 `env`（`shell` / `agent_run`
  会发增量），22 个 `_env`（不发射，前缀即文档）；`ctx` 只在真正需要的工具里保留
  （VDFS 工具用它调 provider、`agent_run` 读会话身份、`ask_user` 读 `MODE`）。
- **删掉的重复**：`confirm_prompt_payload` 的返回从 `InvokeResponse<PluginPayload>`
  收成 `Result<Value, _>`；`execute_skill` 同理；`PluginPayload::new(&json!{...})`
  这个包装在 24 个工具里**逐处消失**。
- **`subagent.rs` 改读 `args` 而非 `ctx.payload()`**：它原先是「签名换了但体内还回读
  信封」的唯一残留——正是本批要消灭的那种耦合。
- **`execute_turn` 参数 7 → 6**，两处 `#[allow(clippy::too_many_arguments)]` 随之删除
  （阈值 7，已不再触发）。

### 11.5 保住的语义（未变）

- `route()` 直连调用仍然**自然静默**：`ExecEnv::from_request` 在没有 `EVENT_SINK`
  时给出 `Null` 出口、在没有 `ABORT_SIGNAL` 时给出永不中止的信号——与历史一致。
- 工具仍然**不构造节点身份**：`failure_kind` + `prompt` 载荷的契约逐字不变，
  节点归 `tool_executor`（§9.2）。
- 压缩路径仍然**出口静默**：`run_compression_llm` 改为收 `env`，调用方给
  `ExecEnv::new(EventSink::silent(), abort)`——「内部请求不产生可见事件」仍是
  类型上的选择。

### 11.6 验证

- `node scripts/gate.mjs` **32 / 32**；`cargo test --lib` **817 passed / 0 failed**。
- CLI 端到端（`--provider LMStudio`，4 场景）：普通流式对话、交互模式 `cmd`、
  交互模式 `ask_user`、自动模式，全部 `EXIT=0`。
- 用例数**不变**（817）：本批只改签名，`ask_user.test.rs` / `shell.test.rs` 的
  调用形态随签名改写，断言内容与覆盖点逐一保留。

---

## 附：证据索引（文件:行）

> ⚠️ 以下行号指向**审计基线 `88a6800`**（收口前），用于复核 §1 的"现状"描述；
> 收口后行号已整体位移，S2 拆分后 `chat_loop.rs` 的符号又分散到 `chat_loop/*.rs`，
> 因此**一律按符号名检索**（如 `prepare_turn_inputs` / `apply_compaction`）。
> 新增条目请只写文件名（不写行号）。

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
