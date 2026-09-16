# Session 核心循环（LLM × 工具）梳理与收口设计

> 状态：**批次 A/B/C 已落地**（`cargo check --tests` 通过）；批次 D（异步工具并发）待授权
> 范围：`symbio/src/plugins/session/` 中"调用大语言模型 + 执行工具"的会话主循环
> 审计基线 commit：`88a6800`（设计稿）→ 实施基线 `5c60806`（本设计文档首次提交）
> 上游文档：`./mechanism-audit.md`（批次 A–D 已落地）、
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

> 以下为**批次 A/B/C 落地后**的实际形态（已与 `chat_loop.rs` 对齐，非设计稿）。

```rust
pub async fn run_chat_loop(
    orchestrator: &ChatOrchestrator,
    ctx: Arc<dyn InvokeRequest>,
    mut channel: PluginChannel,
) -> Result<(), PluginError> {
    // ── 前步骤 ①：请求解析与请求级配置快照 ──────────────────────────────
    let mut req: model_chat::Request = ctx.payload()?;
    let turn_req = TurnRequest::new(&req);          // unwrap_or(cfg) 只求值一次

    // ── 前步骤 ②：会话引擎 ──────────────────────────────────────────────
    let session = open_chat_session(&ctx).await;

    // ── 前步骤 ③：轮次状态 + 上下文容器 ─────────────────────────────────
    let mut turn = TurnState { abort_flag: …, ..Default::default() };
    let mut single_message = req.single_message.take();
    let mut context = SessionContext { messages: Vec::new(), session };

    // ── 前步骤 ④：resume（一次性，可提前收尾）───────────────────────────
    if let Some(tr) = req.resume.take() {
        match resume::process_resume(…).await {
            Ok(ResumeOutcome::Continue) => {}
            Ok(ResumeOutcome::Done) =>
                return finish_turn(orch, &context, &channel, &turn, TurnExit::ResumeDone).await,
            Err(e) =>
                return finish_turn(orch, &context, &channel, &turn, TurnExit::Failed(e)).await,
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
        let inputs = match prepare_turn_inputs(orch, &ctx, &mut context, &mut channel,
                                              &mut turn, &turn_req).await {
            Ok(v)            => v,
            Err(exit)        => return finish_turn(orch, &context, &channel, &turn, exit).await,
        };

        // 步骤 4 · 推理
        let result = orch.provider.execute_turn(&inputs.system_prompt, &inputs.request_view,
                                               &inputs.tools, &inputs.root_id,
                                               &mut channel, &turn.abort_flag).await;
        let out = match result {
            Err(RetryWithoutContextId) => { /* 清 response_id + 半截 Streaming */ continue; }
            Err(Aborted)               => return finish_turn(…, TurnExit::Aborted).await,
            Err(e)                     => return finish_turn(…, TurnExit::Failed(e)).await,
            Ok(out)                    => out,
        };
        if turn.abort_flag.load(SeqCst) {
            return finish_turn(orch, &context, &channel, &turn, TurnExit::Aborted).await;
        }

        // 步骤 5 · 推理收尾（定格子节点 → 校准估算 → 并入上下文）
        let result = settle_reasoning(orch, &mut context, &channel, &inputs.root_id, out).await;

        // 步骤 6 · 本轮结算 + 下一步判定（收口 ④）
        match close_turn(orch, ctx.clone(), &mut channel, &mut context,
                         &mut turn, result, &turn_req).await {
            TurnFlow::NextTurn        => {}
            TurnFlow::Finish(exit)    => return finish_turn(orch, &context, &channel, &turn, exit).await,
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
  4. 父节点状态补丁（`parent_updates` → `update_messages`）与结果子节点的写入顺序。

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

## 附：证据索引（文件:行）

> ⚠️ 以下行号指向**审计基线 `88a6800`**（收口前），用于复核 §1 的"现状"描述；
> 收口后行号已整体位移，请按符号名检索（如 `prepare_turn_inputs` / `apply_compaction`）。

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
