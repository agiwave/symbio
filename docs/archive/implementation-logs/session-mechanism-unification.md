# Session 插件运行机制全景 与 机制化 / 统一化 / 简化方案

> **【归档说明】** 本文档为机制审计与改造提案（含缺陷取证、实施状态、待办分期），属历史实施记录，
> 移入 `docs/archive/implementation-logs/`，本文不再更新。
> 现行机制描述见 `symbio/src/plugins/session/README.md` 与 `symbio/src/plugins/session/docs/turn-tool-mechanisms.md`。
> 已落地项的变更事实见 `docs/CHANGELOG.md`。

> 取证基线：`plugins/session/`（chat_loop 1553 / orchestrator 1296 / tool_executor 950 /
> chat_session 679 / compression 789 / resume 469 / context_window 478 / handlers 649 /
> plugin 800 / prompt 119 / tool_result_guard 224 / active 92）+ `symbio_core/`
> （model_provider 156 / turn 1071 / capability 267 / transport 183 / chat_pipeline 257 / tools 204）
> + `plugins/model/bound_provider.rs`。本文只做机制描述与改造提案，**未改动任何代码**。

---

## 一、整体运行机制

### 1.1 分层与职责

| 层 | 位置 | 职责 |
|---|---|---|
| 入口/路由 | `plugin.rs` route + `handlers.rs` | 会话 CRUD、配置、打开会话、实体摘要、崩溃清理 |
| 编排 | `orchestrator.rs` | 请求校验、能力收集、提示词装配、spawn 任务、帧消费、工作态收敛、失败降级 |
| 主循环 | `chat_loop.rs` | 轮次状态机：历史→视图→LLM→定格→持久化→工具→续轮 |
| 能力 | `tool_executor.rs` | 单工具/批量工具执行、流式合并、结果守卫、hook |
| 上下文治理 | `compression.rs` / `context_window.rs` / `tool_result_guard.rs` / `chat_session.rs` | 视图裁剪、骨架化、语义压缩、写入口守卫、存储裁剪 |
| 恢复 | `resume.rs` | 审批/拒绝/补参/回答/重试的删除-重建 |
| 模型网关 | `plugins/model` + `symbio_core/turn.rs` | 协议差异内化、HTTP/SSE、TurnOutput |

关键契约：`ChatOrchestrator { provider: Arc<dyn ModelProvider>, parent, context_limit }`。
Session 只认 `ModelProvider`，不认任何协议实现；协议差异全部内化在 model 插件。

### 1.2 一次发送的完整链路

```
前端 send → session route
  └ handle_chat_message
      1. 统一 session_id 解析/校验（新建会话落库 + 标题）
      2. resume 与 message 互斥校验；resume 需 is_working==false
      3. message 分支：允许覆盖旧请求（旧 rid 自然失效）
      4. is_working=true + 广播 busy + rid
      5. spawn:
           resolve_session_params（请求 > 会话 metadata > 默认）
           open_session_handle → SESSION_HANDLE 塞进 ctx
           collect_capabilities → take_errors 硬校验 → attach_capabilities
           prompt::build_system_prompt(workdir) → req.system_prompt
           组装 model_chat::Request（resume / ping / 普通三分支）
           run_chat_loop_task
```

`run_chat_loop_task` 做四件事：**WorkingGuard**（panic/异常退出兜底）→ **emit_streaming_start**
（创建 root Turn 节点 + 广播）→ **消费循环**（select! 帧 + 1800s 无帧超时）→ **收尾**（idle 广播 / abort）。

### 1.3 run_chat_loop 主循环（每轮）

```
0  resume 前置处理（RetryTurn / Retry / Approve / Reject / Supply / Answer）
1  取上下文：load_history ? session.get_context_messages() : clear + 仅本轮
2  首轮追加用户消息（落库 + 广播 + UserPromptSubmit hook）
3  解析系统提示词（优先级链，见 §4.1）
4  auto_compress 70% 水位 → compress_with_snapshot_core
5  解析 provider_id → 覆盖 orchestrator.provider
6  取 CAPABILITY_VISITOR 工具清单（provider 不支持 tools 则清空）
7  build_request_view（四步纯视图裁剪）+ 55% nudge
8  provider.execute_turn(...) → 消费循环收集 TurnOutput
9  RetryWithoutContextId → 清 response_id + 删 Streaming 节点 + continue
10 finalize_assistant_turn（状态定格）+ 落库 assistant 消息（复用流式子节点 id）
11 token 校准（provider usage vs count_raw）
12 无 tool_calls → 落库 + Stop hook + 退出
13 length 截断 → 纯文本自动续写（≤3）/ 工具参数截断（报错）
14 context_compact 拦截 → run_context_compact
15 process_tool_calls_async → 结果 extend + 父节点 update_messages + 落库
16 max_tool_rounds 软上限 / needs_user_action 暂停 → Stop hook + 退出
17 下一轮
```

### 1.4 上下文治理分层（已落地的不变式）

| 层 | 变换 | 是否落库 | 位置 |
|---|---|---|---|
| 写入口 | 工具结果守卫（8192 token 归档 + 头尾摘要） | **是** | `tool_result_guard` |
| 存储 | `max_messages` 轮数裁剪、历史工具对裁剪 | 是 | `chat_session` |
| 视图 | 内容节点淡化 → 老旧工具结果淡化 → 工具明细骨架化 → nudge | **否**（每轮重建） | `build_request_view` |
| 语义 | L2 快照压缩（LLM 摘要 + transcript 全文转存） | 是（`replace_messages`） | `compress_with_snapshot_core` |

铁律：**视图层绝不改动存储**，否则 `last_saved` 的切片语义（只追加 `messages[last_saved..]`）会错位。

---

## 二、LLM 调用 与 工具调用 的机制对照

| 维度 | LLM 调用 | 工具调用 |
|---|---|---|
| 入口 | `provider.execute_turn(sp, view, tools, root_id, chan, flag)` | `CapabilityVisitor.invoke` → 命中不了回退 `parent.route()` |
| 注册 | `register_model_provider`（唯一生效 provider） | `register_capability`（`CapabilityMeta` 含 `context_retention`） |
| 请求 | 强类型：`TurnOutput` + 协议内部 `ProtocolEvent` | `InvokeRequest` + `PluginPayload::{Data, Session}` |
| 流式 | `turn.rs` 统一 `emit_update/emit_status/emit_abort` | 自研帧解析：`type ∈ {data, complete, result, output, error}` 字符串匹配 |
| 输出 | `TurnOutput{text, reasoning, usage, finish_reason, child_ids}` | `(Vec<ChatMessage>, Vec<ChatMessage>)` 结果 + 父节点补丁 |
| 错误 | 五态 → `Aborted / NetworkError / Provider(context_id_invalid) / Provider(永久) / Provider(截断)` | 结果级 `success=false`（信息性，不冒泡）+ `PluginError` 仅路由失败 |
| 超时 | connect 10s / read-idle 180s / 整体 1800s / HTTP 重试 4 次 | 硬超时 600s / 流式空闲 180s |
| 持久化 | `append_messages`（复用流式 child id） | `append_messages`（结果）+ `update_messages`（父 ToolCall） |
| Hook | 无（PreCompact 属压缩路径） | PreToolUse / PostToolUse / PostToolUseFailure |
| Abort 检查 | 请求前 / SSE 解析中 / 退避 sleep | 帧 select! 的 `_ => continue`（**不解析控制帧**） |

**已经统一的部分**：两者共用同一个 `PluginChannel`/`PluginFrame`，最终都表达为
`StreamEvent::Update{message}` 的 `ChatMessage` 节点；工具子 agent 帧被过滤、顶层 Assistant
被重锚到 ToolCall 下——即"节点树 + parent_id"是统一表达。

**尚未统一的部分**：请求构造、流式解析、错误映射、超时、hook、持久化各自实现一遍；
`resume.rs` 又把"执行工具 → 生成结果 → 更新父节点 → 落库 → 广播"整条生命周期**第二次**实现。

---

## 三、出口清单（现状 10 条）

| # | 出口 | 落库 | Stop hook | Turn 终态 | 前端 |
|---|---|---|---|---|---|
| 1 | resume 后 Paused | — | ✅ | Completed | idle |
| 2 | 无新用户消息且非 resume | — | ❌ | （未建 Turn） | idle |
| 3 | LLM 错误冒泡 | `persist_failure` | ❌ | Failed | Error + idle |
| 4 | LLM 后 abort（ABORTED 冒泡） | `persist_failure` | ❌ | Failed(中止文案) | idle |
| 5 | 工具参数 length 截断 | `persist_failure` | ❌ | Failed | Error + idle |
| 6 | 无 tool_calls 正常收尾 | ✅ | ✅ | Completed | idle |
| 7 | `max_tool_rounds` 达上限 | ✅ | ✅ | Completed | Error + idle |
| 8 | `needs_user_action` 暂停 | ✅ | ✅ | Completed | idle |
| 9 | 消费循环 1800s 无帧 | `persist_failure` | ❌ | Failed | Error + idle |
| 10 | panic / 任务崩溃 | `WorkingGuard::drop` → `persist_failure` | ❌ | Failed | Error + idle |

**结论**：出口在"是否落库"上收敛良好（全部走 `append/update/replace` 三选一，且失败降级统一走
`persist_failure`），但在**是否触发 Stop hook** 与**是否冒泡**上不一致：3/4/5/9/10 都不触发 Stop。
另外出口 4 用字符串 `msg.contains("用户手动中止了本次回复")` 区分 abort 与真失败。

**修复后（本轮）**：Stop hook 由 `StopSignal` 兜底，10 条出口全部恰好触发一次。

| # | 出口 | Stop hook（修复后） | 触发方式 |
|---|---|---|---|
| 1 | resume 后 Paused | ✅ | 显式 `fire(messages)` |
| 2 | 无新用户消息且非 resume | ✅（原 ❌） | 显式 `fire(&[])` |
| 3 | LLM 错误冒泡 | ✅（原 ❌） | 显式 `fire(&context.messages)` |
| 4 | LLM 后 abort | ✅（原 ❌） | 显式 `fire(&context.messages)` |
| 5 | 工具参数 length 截断 | ✅（原 ❌） | 显式 `fire(&context.messages)` |
| 6 | 无 tool_calls 正常收尾 | ✅ | 显式 `fire(messages)` |
| 7 | `max_tool_rounds` 达上限 | ✅ | 显式 `fire(messages)` |
| 8 | `needs_user_action` 暂停 | ✅ | 显式 `fire(messages)` |
| 9 | 消费循环 1800s 无帧 | ✅（原 ❌） | 显式 `fire_fallback()` |
| 10 | panic / 任务崩溃 | ✅（原 ❌） | `StopSignal::drop` 兜底 |

出口 #3/4/5 虽在循环内提前 `return Err(..)`，但 `context.messages` 只是借用，故仍能给出准确的
`last_message`；真正只能兜底的是出口 #9（消费循环超时，chat_loop 任务可能仍在跑）与 #10（panic，
出口代码根本不执行）——两者都由 `WorkingGuard::drop → StopSignal::fire_fallback()` 覆盖
（`last_message` 为空 + 一条 warn 表明"显式触发点缺失"）。`StopSignal::drop` 是最后防线，
`fire_fallback` 与 `fire` 共用同一个 `fired: AtomicBool`，因此"恰好一次"是硬保证。
出口 #4 的字符串判别同步删除，改由 `PluginFrame::is_abort()` 的类型化错误码分派。

---

## 四、缺陷取证

### 4.1 系统提示词优先级链未作用于实际请求（已修复，逐行核验记录保留如下）

三处读取点、两个真源：

| 位置 | 读取的值 | 用途 |
|---|---|---|
| `chat_loop.rs:317-337` | `system_prompt_owned` → `system_prompt_for_request`：请求显式 → visitor prompts（`default` → provider_id → 首个）→ `"You are a helpful MODEL assistant."` | 仅 overhead 估算 + 压缩 |
| `chat_loop.rs:460-463` | `req.system_prompt.as_deref().unwrap_or("You are a helpful assistant.")` | **实际发给模型**（主循环唯一 `execute_turn`；length 自动续写靠 `continue` 回到顶部复用同一处） |
| `chat_loop.rs:1507` | 传入的 `system_prompt`（来自 337） | 压缩摘要请求 |

`req.system_prompt` 由 `orchestrator.rs:877-882` 填入：`prompt::build_system_prompt(workdir)`
（AGENTS.md 全局 + 工作区指令）非空则 `Some(base_prompt)`，空则 `None`。

推论（两种情形都坏）：

- **base_prompt 非空**：请求 = base_prompt，visitor 链被整条跳过 → 任何经
  `register_system_prompt` 注册的提示词都不生效。
- **base_prompt 为空**：请求 = 硬编码 `"You are a helpful assistant."`，而估算/压缩用的是
  visitor 提示词（或 `"You are a helpful MODEL assistant."`）→ **同一个请求里两个口径**，
  水位计算与压缩提示词都偏离真实发送内容。

`plugins/model/plugin.rs:703-712` 明确以 `provider_id` + `default` **双键**注册 provider 配置的
`system_prompt`，注释写着"消费侧提示词解析链的首选键"——但消费侧从不把它送进请求，
即该注册路径当前**只影响 token 估算，不影响模型行为**。

修复是单点：解析一次后 `req.system_prompt = Some(system_prompt_owned.clone())`（或直接让
461/759 读 `system_prompt_for_request`），三处口径自动归一。
注意分工：`prompt.rs` 声明"智能体人格走 `agent_identity` 工具说明"，故此 bug 的直接影响面是
**provider 级 system prompt 失效 + 水位口径分裂**，不是人格丢失。

### 4.2 错误类型靠字符串判别（已修复）

`orchestrator.rs:447` 用文案区分 abort；`bound_provider.rs:146` 用 `Provider("context_id_invalid")`
承载机器可读语义。缺少 `ProviderError` 枚举，文案一改即静默失效。

修复：`symbio_core::ErrorCode` 枚举成为分类真源——生产侧 `PluginError::code()`、传输侧
`to_frame()` 写 `code` 字段、消费侧 `PluginFrame::error_code()` / `is_abort()` 比较枚举值；
`orchestrator` 的 `is_abort = err_str.contains("用户主动中止")` 已删除，
`RETRY_WITHOUT_CONTEXT_ID` 判定同步改为类型化比较。往返一致性由
`error::tests::error_code_roundtrip_through_frame` 锁定。

### 4.3 工具流式帧的类型协议是隐式的

`tool_executor.rs:345-360` 的 `match m["type"]` 与 `extract_tool_stream_result`（resume.rs 又一份）
构成"没有类型定义的私有协议"，与 `symbio_core::turn::ProtocolEvent` 完全平行。

### 4.4 无新用户消息出口（#2）不触发 Stop hook（已修复）

与出口 #1/#6/#7/#8 不一致；若 Stop hook 承担"通知外部本轮结束"职责，则该路径漏触发。

修复见 §4.4 对应的 P0-3 落地（`StopSignal` 显式 `fire()` + RAII 兜底，恰好一次），
出口 #2/#9/#10 与 orchestrator 早期失败路径全部补齐。

### 4.5 任务 panic 细节只落 stderr，日志侧不可查（已修复）

`orchestrator` 对 `JoinError` 分支原先只 `format!("{e}")` 后直接发帧：面向前端不该泄漏
内部细节，但日志侧同样丢了 panic 消息与 `file:line:col`（默认 panic hook 只写 stderr，
不落结构化日志）。现拆两路：日志保留 `e.to_string()` 全量 + `is_panic()` 区分，
前端只回通用文案，并显式标注 `INTERNAL_ERROR` 错误码。

### 4.6 按字节预算截断字符串在 CJK 边界 panic（已修复）

多处"超长内容截断"用裸 `&s[..n]` / `String::truncate(n)`，`n` 落在多字节字符中间即 panic。
线上先炸的是 `tool_executor::args_summary`（日志路径 panic，会把 chat_loop 任务直接打挂）。
新增 `symbio_core::text::{floor_char_boundary, truncate_bytes}` 作为唯一截断入口，
替换 `tool_executor`、`shell`（流式累积 + 两处终态截断）、`web_fetch`、`mcp/http`、
`content_search`；其余切片点已逐一核验为边界安全（ASCII 定长前缀、`split`/`strip_*`
产出的字符边界、或按 `chars()` 计数）。

### 4.7 `extract_tool_stream_result` 等重复实现（部分核实后修正结论）

原判断"resume.rs 另有一份 `extract_tool_stream_result`、`plugin_traverse` 能力收集两处、
`needs_user_action` 判定两处"经 grep 复核已不成立：这些符号在当前代码树中不存在或只剩
单一定义（应为此前重构收敛的结果）。真正仍平行的是 `tool_executor` 内对工具流帧
`m["type"]` 的隐式协议匹配，属 §4.3 / P1-5 `InvocationEvent` 的范围，不在 P0 处理。

---

## 五、机制化 / 统一化 / 简化方案

### P0 低风险修正（行为等价或明确纠错）

1. ✅ **提示词单一真源**：`chat_loop::resolve_system_prompt` 在 `run_chat_loop` 顶部解析一次并
   写回 `req.system_prompt`，请求构造 / overhead / 压缩三处读同一值。修 §4.1 两个后果。
2. ◐ **错误分类去文案化**：`orchestrator` 的 `is_abort = err_str.contains("用户主动中止")` 已删除，
   改由 `PluginFrame::is_abort()` 比较类型化 `ErrorCode::Aborted`；往返一致性由单测锁定。
   完整的 `ProviderError` 枚举（`Aborted | Network | ContextIdInvalid | Permanent | Truncated |
   IdleTimeout`）作为 `execute_turn` 返回类型仍未做，与 P1-5 `InvocationEvent` 同批推进更划算，
   故未在本轮单独引入半成品。
3. ✅ **Stop hook 对称化**：`StopSignal`（显式 `fire` + `fire_fallback` + `Drop` 兜底）保证
   "一次请求生命周期恰好一次 Stop"，出口 #2/#9/#10 与 orchestrator 早期失败路径全部补齐。
4. ✅ **合并重复实现**：`extract_tool_stream_result` 与 `plugin_traverse` 在当前代码树中已不存在
   （`grep` 复核为 0 命中，见 §4.7），`needs_user_action` 仅一处定义；该项无需再动。

### P1 机制化（统一抽象）

5. **`CapabilityInvocation` 统一外部能力调用**（LLM 与工具是同一件事的两种实现）：

```rust
// symbio_core
pub enum InvocationEvent {
    Delta      { id: String, parent: Option<String>, kind: DeltaKind, text: String },
    Status     { id: String, status: MessageStatus, error: Option<String>, meta: Option<Value> },
    Result     { message: ChatMessage },          // 终态结果节点
    ParentPatch{ message: ChatMessage },          // 已有节点增量
    Control    (ControlFrame),                    // Abort / RetryWithoutContextId
}
pub struct InvocationOutput { pub events: Vec<ChatMessage>, pub patches: Vec<ChatMessage>,
                              pub usage: Option<Usage>, pub finish: FinishReason, pub error: Option<ProviderError> }

pub struct StreamPump<'a> { channel: &'a mut PluginChannel, ledger: &'mut TurnLedger, abort: &'a AtomicBool }
impl StreamPump<'_> { pub async fn run<S: Stream<Item = InvocationEvent>>(...) -> InvocationOutput }
```

   `StreamPump` 一次性承担：**转发前端 + 收集内存镜像（供 `persist_failure`）+ abort 检查点 +
   空闲超时 + 无帧日志**。LLM 侧把 `ProtocolEvent` 映射进 `InvocationEvent`；工具侧把
   `PluginFrame::Data` 解析进 `InvocationEvent`（解析器注册到 capability 元数据，不再是 match 字符串）。
   → 消灭 §4.3、消灭 `collected_ai_messages` 手写 upsert、工具路径自动获得 abort 响应。

6. **工具生命周期单一状态机**：`ToolRun { origin: Fresh | Resume(ResumeAction) }`
   共用 prepare → PreToolUse → invoke → pump → guard → 结果节点 + 父补丁。
   `resume.rs` 从 469 行降到"定位节点 + 计算 args + 复用 ToolRun"，删除第二套落库/广播代码。

7. **`context_compact` 内建工具化**：以 `InternalTool` trait 进入同一分派表（签名可拿到
   `&ChatOrchestrator` 与 `&mut TurnState`），删除 `chat_loop.rs:618-660` 的字符串拦截特例；
   门控（`enable_compact_tool`）与 meta 由注册处单点表达。

8. **`TurnLedger` 统一持久化锚点**：封装 `context.messages` 与 `last_saved`，暴露
   `append()` / `patch()` / `replace()` 三个意图，`replace` 自动重锚 `last_saved`
   （现在手工散在 3 处：压缩后、RetryWithoutContextId 后、resume 后），并统一
   `persist → 失败可见化 → 内存镜像合并（status/meta/error 三字段）` 一段逻辑。

9. **`TokenBudget` 单点预算**：`{ limit, overhead, used, watermark }` 每轮算一次，
   压缩阈值（70%）、nudge（55%）、迟滞（1.15）、视图裁剪、校准全部读它。
   现在常量散在 `compression.rs` 四处 + `orchestrator.context_limit` + `chat_loop` 内联判断。

### P2 结构简化

10. **拆分 `chat_loop.rs`（1553 行）**：`turn_loop.rs`（骨架）/ `compact.rs`（自动+手动压缩）/
    `session_store.rs`（`open_chat_session` + `FallbackChatSession`）/ `hooks.rs`。
11. **`TurnState` 参数对象**：把 `context / last_saved / is_aborted / token_cal / tool_rounds /
    nudged_this_request / provider` 收进一个结构，消掉 `process_tool_calls_async` 的
    `#[allow(clippy::too_many_arguments)]`（8 参数）。
12. **Turn 开-闭同处**：`emit_streaming_start` 在 orchestrator、`finalize_assistant_turn` 在
    chat_loop、落库在 chat_loop、失败降级在 orchestrator——四处分散。收敛为一个
    `TurnFrame { open(), finalize(), fail() }`，由主循环独占调用。

### 必须保持的兼容边界

- 前端协议：`StreamEvent::{Update,Delete,Error,Abort,Status,Title}` 与节点类型树
  （Turn/Reasoning/Text/ToolCall/Tool/UserPrompt）+ `parent_id` 不变式 + `seq` 有序 + append push-only。
- 崩溃恢复：`cleanup_crashed_sessions` 把 `Streaming` 降为 Failed，但**必须保留** `WaitingUserAction`。
- 视图不落库（`last_saved` 切片依赖存储全量）。
- 压缩失败不冒泡（发生在 Turn 创建之前）。
- 工具失败信息性、不中断会话。
- 不设 `max_tool_rounds` 硬默认（65535 软上限），长对话靠视图裁剪 + 压缩控制。

### 建议落地顺序

| 阶段 | 内容 | 风险 | 收益 |
|---|---|---|---|
| 1 | P0 全部（1–4） | 低（含 1 处行为纠错） | 提示词真正生效；错误分派不再靠文案 |
| 2 | P1-8 `TurnLedger` + P1-9 `TokenBudget` | 低 | 消除 3 处手工重锚与阈值散落 |
| 3 | P1-5 `StreamPump` + `InvocationEvent` | 中（触核心） | LLM/工具/压缩三路流式统一，工具获得 abort |
| 4 | P1-6 ToolRun 合并 resume、P1-7 内建工具化 | 中 | 删除约 300 行重复生命周期 |
| 5 | P2 拆分与参数对象 | 低（纯搬迁） | 单文件认知负荷下降 |

---

## 六、实施状态（本轮已落地）

| 项 | 内容 | 落点 | 验证 |
|---|---|---|---|
| P0-1 | 提示词单一真源：`resolve_system_prompt()` 在 `run_chat_loop` 顶部解析一次并写回 `req.system_prompt`，请求构造 / overhead 估算 / 压缩摘要三处读同一值 | `chat_loop.rs` | `chat_loop::tests` 6 个用例（显式优先、default 优先 provider、provider 匹配、保序取首个、无 visitor 兜底、空注册兜底） |
| P0-3 | Stop hook 对称化：`StopSignal` 显式 `fire()` + RAII 兜底，保证一次请求生命周期恰好触发一次；出口 #2（无新用户消息）、#9（消费循环空闲超时）、#10（panic / 任务崩溃）及 orchestrator 早期失败（缺 `CAPABILITY_VISITOR` / 无 provider）全部补齐 | `chat_loop.rs`、`orchestrator.rs::fail_before_loop` | `chat_loop::stop_signal_tests` 6 个用例（显式幂等、空转录、兜底、兜底幂等、Drop 兜底、无父插件静默） |
| — | UTF-8 截断加固：新增 `symbio_core::text`（`floor_char_boundary` / `truncate_bytes`），替换所有按字节预算截断字符串的裸 `&s[..n]` 与 `String::truncate(n)` | `text.rs`、`tool_executor.rs`、`shell.rs`、`web_fetch.rs`、`mcp/http.rs`、`content_search.rs` | `args_summary` 多字节回归测试 + `text` 模块单测；全量 `cargo test --lib` 313 passed |

未动的审计项（P0-2 `ProviderError` 枚举、P0-4 重复实现合并、P1 / P2）仍按上表顺序待排期。
