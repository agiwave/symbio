# 工具错误回传与 Turn 终态机制设计

> **【归档说明】** 本文档为历史实施记录，已于重构后移入 docs/archive/implementation-logs/。
> 机制现状（精简版）见 `symbio/src/plugins/session/docs/turn-tool-mechanisms.md`，本文不再更新。
> 状态：已实施（修复 A / 修复 B / 修复 A 边界 / 2.6 "继续会话"中断可见性均落地）
> 关联：`docs/archive/implementation-logs/model-session-refactor.md`（Model/Session 插件分工总纲，已归档）
> 调查基线：commit `4876853`（audit-session 收官后）

本文档回答两个机制问题：

1. **工具调用失败为什么不应该（也不需要）中断会话 loop**——现状全景梳理；
2. **Turn 的两种终态（正常完成 / 异常）如何持久化为节点状态并在前端体现、可重试**——现状梳理 + 一处重大差距（用户手动中止）的修复设计。

---

## 1. 机制一：工具失败是"信息性"的，loop 不中断

### 1.1 设计原则

工具失败（如 FileRead 发现文件不存在）本质上是 **LLM 可感知、可自我修正的环境信息**，不是系统故障。因此：

- 失败结果必须以**合法 Tool 消息**的身份留在上下文中喂回 LLM；
- 会话 loop 绝不因工具失败而中断、停摆或退出；
- 只有"需要用户决策/输入"的工具（confirm / ask_user）才通过 `WaitingUserAction` 暂停 loop——那是正常语义，不是失败。

### 1.2 全链路现状（以 FileRead 文件不存在为例）

```
LLM 发起 tool_call(file_read)
  → tool_executor::process_tool_calls_async 分发到 local/file_read.rs
  → 插件返回 Err(PluginError::InternalError("无法解析路径: ..."))
  → 归一为错误文本：execute_tool_async 内 (format!("Error: {e}"), false, None)
  → build_tool_message 构造 Tool 消息（success=false）
  → 【关键定格】tool_msg.status = Completed（tool_executor.rs L611-618）
  → 父 ToolCall 节点标 Completed + error=错误文本 + meta.failure_kind="error"（L684-709）
  → 两条消息进入 context.messages + persist_messages 落库
  → 下一轮 LLM 请求看到 "Error: ..."，自我修正（换路径/告知用户）
  → loop 继续下一轮，从未中断
```

### 1.3 三条防线（为什么必须这样设计）

| # | 防线 | 位置 | 若违反的后果 |
|---|------|------|--------------|
| 1 | **失败结果定格 Completed** | tool_executor.rs `if !success && tool_msg.msg_type != UserPrompt { tool_msg.status = Completed }` | 工具失败若落库为 Failed：请求视图会推导 `success=false`（Anthropic is_error），且 Failed 状态污染 Turn 级异常语义——前端会把该工具节点渲染为错误条/重试入口，而 loop 实际并未中断（详见 2.6 的分层） |
| 2 | **父 ToolCall 同标 Completed + meta.failure_kind="error"** | tool_executor.rs 父节点更新段 | 同上——Failed 属于 Turn 级状态机（2.1），工具节点标 Failed 会让 RetryTurn 子树删除、前端终态渲染把一个正常轮次误判为失败；错误信息降级为父节点 `error` 字段承载（仅展示用） |
| 3 | **Failed 仅用于 Turn 根节点** | persist_failure / message_builder | 会话层不再过滤 Failed（2.6，上下文必须完整真实）；请求视图层对 Failed 的动态呈现（中断说明）也只针对 Turn 根节点——工具节点若标 Failed 会触发 is_error 映射并破坏重试入口契约 |

### 1.4 extract_result 的双层错误捕获

工具返回 `Ok(payload)` 但 payload 内 `success=false` 时，extract_result（tool_executor.rs）同样转为错误文本——即"业务失败"与"插件异常"对 LLM 完全同构，LLM 无需理解两套失败协议。

### 1.5 已清理的历史遗留（修复 B）

`needs_user_action` 检测块内存在一段给 `Failed + failure_kind` 父 ToolCall 打 `recoverable` 标记的代码（原 chat_loop.rs L813-844）。**不可达**：

- auto 模式：工具失败父节点标 Completed（信息性策略），永远不产生 Failed；
- manual 模式：同上（1.3 防线 2 无模式区分）；
- user_prompt 暂停路径：父节点状态是 WaitingUserAction，不是 Failed。

该块假设的"Failed 父节点"仅存在于信息性策略引入之前的旧设计。已整体删除，`needs_user_action` 检测保留（它负责的"暂停等用户输入"是活跃语义）。

---

## 2. 机制二：Turn 只有两种终态——Completed / Failed

### 2.1 状态机全景

```
                    ┌─────────────────────────────────────────┐
                    │            Turn（根节点）                │
                    └─────────────────────────────────────────┘
   Pending ──▶ Streaming ──┬──▶ Completed（finalize_assistant_turn 定稿）
                           └──▶ Failed（persist_failure 降级）
                                     │
                                     ▼
                     前端渲染 ⚠ 错误条 + 重试按钮
                     （error 字段承载原因；RetryTurn 删除-重建）
```

- `MessageStatus` 五态：Pending / Streaming / **WaitingUserAction** / Completed / Failed（chat_message.rs）。
- `ChatMessage.error` 字段：**仅当 status == Failed 时存在**（schema 注释约定），由 persist_failure 写入根 Turn。
- WaitingUserAction 是**合法中间态**（不是异常）：崩溃恢复时被 cleanup_crashed_sessions 特意保留，作为 resume 锚点。

### 2.2 三条"收尾"路径（谁负责给在途 Turn 终态）

| 路径 | 触发 | 收尾动作 | 落点 |
|------|------|----------|------|
| **正常完成** | LLM 流结束 → finalize_assistant_turn | Turn 及子树定稿 Completed，persist_messages 切片落库 | chat_loop.rs |
| **异常（LLM/编排层）** | run_chat_loop 返回 Err → 消费循环 PluginFrame::Error 分支 | `persist_failure`：最后一个根级 Turn 标 Failed + error，子树在途节点定稿 Completed，广播 changed，再 broadcast_error_with_idle（复位 is_working） | orchestrator.rs 消费循环 |
| **崩溃兜底** | spawn join 失败 / panic | WorkingGuard::drop → persist_failure("会话处理异常中断，请重试") + Error + idle | orchestrator.rs |

**persist_failure 精确语义**（orchestrator.rs，是本机制的枢纽）：

1. 定位 `collected`（消费循环维护的流式快照）中**最后一个根级 Turn**（msg_type=turn 且 parent_id=None）；找不到则仅广播会话级错误（错误发生在 Turn 创建前）。
2. 作用域收窄为该 Turn 的传递闭包子树（subtree_of）——历史轮次绝不受影响。
3. 内存镜像定稿：根 Turn → Failed + error（error 已存在则不覆盖）；子节点 → None/Streaming/Pending 定稿 Completed，**绝不挂 error**（工具在本地执行不会产生 API 错误，避免错误刷屏）。
4. 载入整会话合并（不覆盖历史），三种情况：
   - 已落库的根 Turn 即使是 Completed 也**强制回滚 Failed**（防 finalize 后才抛错的时序）；
   - collected 有而存储没有的（崩溃时刚流式出来）→ 补写；
   - 子节点父缺失 → 丢弃孤儿，不污染前端。
5. 仅广播状态真正变化的 changed 列表（Update 事件），前端据 Failed + error 渲染错误条与重试按钮。

### 2.3 重试语义（RetryTurn：删除-重建）

前端对 Failed Turn 的重试 = `chat/send` + `resume:{target_id, action:"retry_turn"}`：

1. resume.rs process_retry_turn 定位 Failed Turn → BFS 收集全部子孙 → replace_messages 删除；
2. 逐个广播 Delete 事件（前端移除错误子树）；
3. 返回 Continue → chat_loop 从存储重载历史（用户原消息仍在）→ 发起全新 LLM 请求 → 生成全新 Turn 子树。

**关键设计**：重试不"续写"失败请求，而是删除重建——失败子树从存储中整棵移除后发起全新请求，语义干净、无残留半截内容。与"继续会话"（2.6）互不干扰：用户点重试走删除-重建；用户选择继续对话，失败子树保留在上下文中、由请求视图层附加中断说明。

### 2.4 重大差距（已修复）：用户手动中止

**修复前行为**：`handle_abort` 发送 abort 帧 → run_chat_loop 的 `send_request` 返回 `Err(PluginError::Aborted)` → chat_loop.rs L497-503 **直接 `return Ok(())`**——不走 finalize、不走 persist_failure。后果：

- 在途 Turn（已 Streaming、前端正在渲染流式动画）**不落库、无 Failed 状态、无 error**；
- 刷新后该 Turn 凭空消失（前端幽灵节点）；
- 无重试入口——用户中止后想重问只能重新输入完整内容。

这与"异常（含用户手动中止）应持久化为节点状态、前端可体现且重试"直接冲突。用户手动中止被明确列入异常终态清单。

**修复设计（实施内容）**：

- **chat_loop.rs**：`send_request` 后的两个 abort 逃逸点（`Err(PluginError::Aborted)` 分支、`abort_flag` 复查分支）由 `return Ok(())` 改为 `return Err(PluginError::Aborted)`，让中止走既有 Error 帧链路。
  - **仅这两处冒泡**：它们位于 `send_request` 之后——本轮 Turn 已 Streaming、必须收尾；
  - **turn 循环顶部的两个 abort 检查点不冒泡**（L294/L307）：处于轮次边界，上一轮已定稿落库，冒泡会把已成功的 Turn 错误回滚成 Failed。
- **orchestrator.rs 消费循环**：`PluginFrame::Error` 分支识别 ABORTED 错误码（`{"code": "ABORTED"}`，error.rs 已提供），改走专用收尾：
  1. `persist_failure(&state, &session_id, &collected_ai_messages, "用户手动中止了本次回复")`——在途 Turn 落库为 Failed + error，子节点定稿 Completed，广播 changed；
  2. `break` 退出消费循环，复用循环后的统一收尾：清 `ai_control_tx`（让 handle_abort 的 3s 轮询立即感知、免等兜底）→ 复位 is_working → `guard.done` → 广播 Status idle（**不广播业务 Error 事件**——中止不是错误，避免前端弹错误提示；与 handle_abort 的 Abort 事件语义互补）。**必须 break 而非 return**：清理块位于 while 循环之后，`return` 会跳过 `ai_control_tx = None`，handle_abort 将等满 3s 才收敛。
- **为什么必须在消费循环侧持久化**：run_chat_loop 拿不到 `collected_ai_messages`（流式快照由消费循环的 Data 帧合并逻辑维护，persist_failure 消费它）——chat_loop 侧无材料可写。
- **与 handle_abort 的协作**：handle_abort 广播 `StreamEvent::Abort`（前端清除流式 UI）+ idle；本修复在其之前让在途 Turn 先落库为 Failed——前端收到 changed 的 Update(failed) 后渲染错误条 + 重试，Abort 事件只负责清理流式动画，两者不冲突。
- **重试入口语义**：中止的 Turn 与 LLM 失败的 Turn 完全同构（Failed + error），RetryTurn 删除-重建直接复用——用户点重试即重新发起该请求。

**边界规则：压缩阶段的中止/失败不冒泡（修复 A 补充）**

`auto_compress_process`（chat_loop.rs）发生在本轮 Turn 创建**之前**（调用点在 emit_streaming_start 之前）。若压缩阶段的 `Err(PluginError::Aborted)`（或限流、流中断、网关错误）向上冒泡，消费循环 Error 帧分支会调用 persist_failure——而 `collected` 中最后一个根级 Turn 是**上一轮已成功定稿的 Turn**，其"已落库 Completed 强制回滚 Failed"逻辑会误回滚成功 Turn。因此：

- **压缩阶段一切失败就地降级**：回滚到未压缩历史（`context.messages = original_messages`），返回 `Ok(None)` 让主循环继续创建本轮 Turn——压缩是优化，不是本轮对话的前置条件；
- **新增 execute_turn 前 abort 检查点**：压缩失败降级后 abort_flag 仍为 true（abort 帧已被压缩请求消费），若不拦截，execute_turn 会发起一次多余的 LLM 请求。该检查点位于 Turn 已创建（emit_streaming_start）之后，故冒泡 `Err(PluginError::Aborted)` → 消费循环 ABORTED 分支 → persist_failure 把**本轮** Turn 收尾为 Failed（persist_failure 按 failing_turn 子树收窄，不波及上一轮）；
- **turn 循环顶部原有两个检查点维持不冒泡**（L294/L307）：处于轮次边界，上一轮已定稿落库。

**冒泡判定原则**（修复 A 的统一口径）：Turn 已创建 → 冒泡（必须收尾为 Failed）；Turn 创建前（轮次边界 / 压缩阶段）→ 不冒泡（就地降级或静默重入）。

### 2.5 各异常来源与终态对照表（修复后）

| 异常来源 | 在途 Turn 终态 | error 文案 | 前端重试 |
|----------|----------------|-----------|---------|
| LLM 网关错误 / 流中断 | Failed（persist_failure） | 具体错误信息 | ✅ |
| LLM 限流 | Failed（persist_failure） | 限流提示 | ✅ |
| 编排层失败（provider 缺失等） | Failed（persist_failure） | 具体错误信息 | ✅ |
| **用户手动中止** | **Failed（persist_failure）** | **用户手动中止了本次回复** | **✅（本次修复）** |
| 进程崩溃 / panic | Failed（WorkingGuard drop） | 会话处理异常中断，请重试 | ✅ |
| 崩溃重启（历史残留 Streaming） | Failed（cleanup_crashed_sessions） | 会话因重启中断 | ✅ |
| 工具执行失败 | **不影响 Turn**（信息性，见机制一） | —（父节点 error 字段） | ❌（loop 不中断） |
| 工具待用户输入 | WaitingUserAction（合法中间态，非异常） | — | 恢复走 approve/reject/answer |

### 2.6 "继续会话"中断可见性（用户选择不重试）

机制二的前提是"异常持久化为 Turn 终态 + 提供重试"，但用户也可以**不重试、直接继续对话**——正如生活体验中"用户打断了模型说话，然后继续"。此时模型必须在上下文中看到中断现场，否则思维链断裂：半截输出凭空消失，模型无从知晓"话说了一半"。

**设计原则**（与持久化/过滤的分工一致）：

1. 会话持久化反映整个会话**真实完整的状况**（含错误/中断及原因）；
2. 过滤/压缩是在完整真实基础上做的**平衡裁剪**，两者不矛盾——裁剪下移到请求视图层，存储永远真实；
3. 失败始终发生在 **Turn 层面**：思考/正文/工具调用的异常中断，重试的都是这个 Turn。

**三层分工**：

| 层 | 职责 | 位置 |
|----|------|------|
| **存储层** | 单一事实源：所有节点（含 Failed 与半截输出）原样持久化，不做任何裁剪 | orchestrator persist_failure 等 |
| **会话层** | `get_context_messages` **不再过滤 Failed**——失败 Turn 整树保留进上下文。persist_failure 只把根 Turn 标 Failed、半截子节点定稿 Completed，因此整树保留后自然连贯；原有的孤儿过滤退化为安全网 | chat_session.rs（Persistent / Ephemeral 两处） |
| **请求视图层** | 按 status 动态补齐（build_request_view 每轮从存储重建，天然幂等）：<br>① **中断说明**——Failed 根 Turn 聚合后向 content 附加说明段落（携带 error 原因），形成「半截输出 → [本轮回复被中断（原因：…）…] → 用户新消息」序列；<br>② **占位工具结果**——ToolCall 无对应结果（中止打断工具执行）时合成占位 tool 结果（含工具名 + "执行被中断"），避免请求包出现「有 tool_call 无 tool_result」触发 provider 400；<br>③ **success 推导**——status=Failed 的工具结果推导 `NativeMessage.success=Some(false)`，触发 Anthropic tool_result 的 `is_error=true` | message_builder.rs `flatten_chat_messages` |

**顺带修复的两个既有缺陷**：

1. **跨轮工具失败结果丢失**：build_tool_message 把失败工具结果标 Failed，此前被会话层过滤剔除——模型跨轮看不到工具失败原因（违背机制一"失败是信息性"的精神）。过滤移除后整树保留，且经 success 推导携带 is_error，失败信息真正到达模型。
2. **中止打断工具执行的 400 风险**：ToolCall 已持久化但结果未产生，请求包会出现"有 tool_call 无 tool_result"——占位结果合成修复。

**与重试的互不干扰**：用户点重试 → RetryTurn 删除-重建（2.3），删除整棵失败子树后全新请求，不受中断说明影响；用户继续对话 → 失败子树保留 + 中断说明呈现。两种选择各自语义干净，共用同一份真实存储。

**压缩协作**：压缩阶段（build_request_view / sliding_window）在完整会话上做平衡裁剪，失败 Turn 的裁剪遵循与其他 Turn 相同的规则（按 User 边界切片、fade/骨架化），不因 Failed 而特殊处理——中断说明在请求视图重建时按 status 重新附加，天然幂等。

---

## 3. 实施记录

| 项 | 文件 | 内容 |
|----|------|------|
| 修复 A-1 | chat_loop.rs | `Err(PluginError::Aborted)` 分支与 `abort_flag` 复查分支（send_request 后）：`return Ok(())` → `return Err(PluginError::Aborted)`，注释说明冒泡原因与轮次边界检查点不冒泡的区分 |
| 修复 A-2 | orchestrator.rs | 消费循环 Error 帧分支：解析 meta.code，`ABORTED` → persist_failure("用户手动中止了本次回复") + guard.done=true + is_working 复位 + 广播 idle（不广播业务 Error） |
| 修复 A-边界 | chat_loop.rs | auto_compress_process 一切错误就地降级（回滚未压缩历史 + 返回 Ok(None)，不再向消费循环冒泡，防止误回滚上一轮成功 Turn）；execute_turn 前 abort 检查点（Turn 已创建后冒泡 Err(Aborted)，走 ABORTED 分支收尾本轮） |
| 修复 B | chat_loop.rs | 删除 needs_user_action 块内不可达的 recoverable 标记代码（Failed + failure_kind 父节点在信息性策略下永不产生） |
| 2.6-会话层 | chat_session.rs | Persistent / Ephemeral 的 get_context_messages 移除 Failed 过滤（失败 Turn 整树保留，孤儿过滤退化为安全网）；删除 MessageStatus 冗余导入 |
| 2.6-视图层 | message_builder.rs | flatten_chat_messages：① Failed 根 Turn 附加中断说明（含 error 原因）；② 无结果 ToolCall 合成占位 tool 结果（先于孤儿剔除）；③ Failed 工具结果推导 success=Some(false) |
| 2.6-测试 | message_builder.rs | 新增 4 测试：failed_turn_visible_with_interrupt_note / failed_turn_without_children_survives_with_note / interrupted_tool_call_gets_placeholder_result / failed_tool_result_sets_success_false |

回归：cargo check 零警告；cargo test --lib 全绿（基线 227 + 新增 4 = 231）。

---

## 4. context_compact 主动压缩机制：三问题分析与修复

> 现场证据：会话 `mtqm3mlgpyyg4cmlf6q`（GLM provider，code 1214 400 错误）。
> 修复后语义适用于**被动压缩**（auto_compress 链路共用同一快照/消息构造代码）与**主动压缩工具**（context_compact）两条路径。

### 4.1 问题 1：主动压缩后悬空 tool_call → provider 400

**现象**：会话出现 parent 悬空的 ToolCall 节点（`call_8e281709` 的 parent `1bcba853` 不存在于消息列表），请求包里 Tool 结果失去父节点，GLM 返回 400。

**根因链**：主动压缩切分函数 `find_turn_tool_call_split_idx` 切在**本 Turn 首个 ToolCall**的下标：

- 待压缩历史 = `[.., 切分点)`，把**当前用户指令 + Turn 根节点**一起压进快照；
- 保留区 = `[切分点..]` 只剩 ToolCall 及其结果子节点——它们的 parent（Turn 根）已进快照，parent 链断裂；
- 快照正文被渲染为一条 Text 消息，Turn 根不复存在 → 悬空。

讽刺的是，代码注释声称"保留区：当前用户指令 + 进行中 Turn（含本批 ToolCall）及之后的一切"——**实现与注释意图自相矛盾**。

**修复**（切分点前移）：

- 函数改名 `find_turn_tool_call_split_idx` → `find_turn_user_split_idx`，语义改为**定位当前用户指令**：先找 `id == root_id` 的 Turn 根下标，再向前回扫最近的根级 User 消息（`role=User && parent_id=None`）；
- 保留区 = `[用户指令, Turn, 子节点...]`，Turn 子树 parent 链天然完整，任务原文不进快照（与原注释意图一致）；
- 回退规则：Turn 之前无根级 User → 切在 Turn 根下标（子树仍完整）；Turn 不存在 → 返回 0（run_context_compact 以 `split==0` 视为中止）；
- 安全性依据：① nudge 水位提醒为请求级注入、**不落库**（chat_loop nudge 注释），回扫不会误命中注入的 User 消息；② 压缩后快照本身是 User 角色，二次压缩时回扫命中快照 → 切点 0 → 中止，不会把快照自身当待压缩历史；③ provider 侧连续 User 消息安全（anthropic_messages 有连续同角色合并，OpenAI/GLM 协议接受）。

### 4.2 问题 2：压缩后首条消息不是 user → provider 400（code 1214）

**现象**：压缩后会话首条是 assistant 快照（`7d7a0efe`，post_tokens=5376），GLM 要求对话以 user 开头，返回 400 code 1214。多数 provider 有同类约束（Anthropic 首条必须 user，OpenAI 强依赖）。

**根因**：两条压缩路径的快照构造均硬编码 `role: Some(MessageRole::Assistant)`：

- 被动路径 chat_loop.rs（原 L1193，`id: root_id.to_string()`）；
- 主动路径 chat_loop.rs（原 L1346，`id: short_id()`）。

**修复**：两处均改为 `role: Some(MessageRole::User)`，快照内容文本不变（`[CONTEXT SNAPSHOT — 压缩的历史记忆，基于它继续任务]...`）。语义上快照是"给模型的记忆交接书"，以 user 口吻交代上下文也是协议更自然的表达。

### 4.3 问题 3：远未到上下文限额即触发压缩

**触发链拆解**（修复后保留此分析作为调参依据，机制本身未改）：

| 环节 | 机制 | 阈值 | 说明 |
|------|------|------|------|
| 硬触发 | `should_start_compression` | `context_limit × 0.7` | current（含 overhead）≥ 阈值即触发 |
| 引导触发 | `should_emit_context_nudge` → 请求级注入提醒 → 模型调用 context_compact | `context_limit × 0.55` | 提醒诱导模型"提前"压缩——这是"远未到限额就压缩"的直接来源 |
| force 直通 | context_compact 工具调用 / `/compact` 指令 | 无 | `force=true` 跳过阈值与迟滞判断 |
| 迟滞保护 | 距上次快照增长 < `post_tokens × 1.15` | 拦截 | 仅拦硬触发，不拦 force |
| 最小收益 | `MIN_COMPACT_TOKENS = 4000` | 拦截 | 待压缩历史 < 4000 tok 放弃 |

**水位口径误区**：现场 `before_tokens=68848` 看似远超限额，但它只是**切分点前待压缩历史的估算**，不是总水位；真实总水位 = 全部消息估算 + 请求级 overhead（system prompt + 工具定义 schema，插件场景可达数万 token）。触发时真实水位可能确实越过了 55%/70% 线，只是用户从 `before_tokens` 数字上看不出来。

**修复决策**：`enable_compact_tool` 默认关闭——机制存在缺陷与调参争议时，最稳妥的是收口为显式 opt-in。默认关闭后 0.55 nudge 引导与 force 直通链路整体不生效，问题 3 随之消失；需要主动压缩的用户在插件配置显式开启，并应理解 nudge 会引导模型在 55% 水位即自行调用压缩（属预期行为）。

### 4.4 修复 A：enable_compact_tool 默认关闭（5 处落点）

| 落点 | 文件 | 内容 |
|------|------|------|
| 1 | chat_loop.rs DIAG 日志 | `req.enable_compact_tool.unwrap_or(false)` |
| 2 | chat_loop.rs 有效值（原 L180） | 同上 + 注释说明与 session_config 默认一致 |
| 3 | session_config.rs | `default_enable_compact_tool() -> false`（serde 默认，Default impl 同源） |
| 4 | plugin.rs UI schema | `"default": false` + description"默认关闭，需手动开启" |
| 5 | README.md | 配置示例 `enable_compact_tool: false` + 注释 |

配套确认：orchestrator.rs 三分支均传 `Some(session_cfg.enable_compact_tool)`，serde 默认值改动直接生效于持久化/临时/新建全部会话形态。

### 4.5 回归

- cargo check 零警告；
- compression 模块 21 测试全绿（含新 `test_turn_split_idx_starts_at_user_instruction` / `test_turn_split_idx_fallback_and_abort`，取代旧 `test_turn_tool_call_split_idx_scoped_to_current_turn`）；
- 全量 `cargo test` 232 passed / 0 failed。
