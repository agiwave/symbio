# 会话上下文压缩设计（v2）

> 目标：**保持与大模型的会话持续不中断**——上下文永不因膨胀而失败，且压缩后模型仍能无缝继续工作。
>
> 四个核心问题：
> 1. 多轮会话轮次太多后，如何有效压缩或淡化？
> 2. 单轮响应内工具调用次数太多后，如何有效压缩或淡化？
> 3. 单轮工具调用与多轮会话之间的优先关系及取舍？
> 4. 主动压缩机制（启用工具，由模型自己决定时机）？

---

## 0. 总原则

1. **确定性优先**：能用纯本地规则解决的（截断、存档、骨架化）绝不花 LLM 调用；语义压缩（LLM 摘要）只做兜底。
2. **一切可回溯**：任何被压缩的内容必须先存档（全文落盘），上下文中留下存档路径脚注，模型可用 `local/file_read` 取回。
3. **配对永不离散**：`ToolCall`（assistant 请求）与其 `Tool` 结果子节点必须同生共死，任何切分点不得拆开它们，否则请求包非法。
4. **失败必回滚**：语义压缩失败时回滚到原始上下文，绝不让 `[压缩指令]` 残留，绝不让压缩失败阻断对话。
5. **单一计量**：所有层统一使用 `CalibratedTokenizer`（启发式 × provider 用量反馈校准），禁止裸 `len()/4`（对中文严重失准）。

---

## 1. 分层机制总览

| 层 | 名称 | 作用域 | 手段 | 成本 |
|---|---|---|---|---|
| L0 | 单条工具结果守卫 | 单条 Tool 结果 | > 8192 tok → 存档 + head/tail 摘要 | 0 |
| L1 | 单消息行数脱水 | 单条超长消息 | > 15 行 → 存档 + 末 10 行 + 路径脚注 | 0 |
| L2 | 轮内老化淡化（fade） | 单轮内历史工具结果 | > 40 工具轮激活，老结果压到 2k tok | 0 |
| L3 | 窗口骨架化 | 请求组装期 | > 15 个 ToolCall 明细 → 占位符 | 0 |
| L4 | 存储清理 | 持久层 | append 时物理删除老 ToolCall 及存档 | 0 |
| L5 | 全局快照（被动） | 跨轮历史 | ≥ 70% 上限 → LLM 摘要替换 | 1 次调用 |
| L6 | 主动压缩工具 | 轮内/跨轮 | 模型调用 `context_compact` | 1 次调用 |

L0–L4 是确定性机制，常开；L5 是被动兜底；L6 是主动手段。

---

## 2. 计量基础（修复项）

```rust
// compression.rs
pub fn estimate_message_tokens(m: &ChatMessage) -> usize;
pub fn estimate_context_tokens(messages: &[ChatMessage], overhead_tokens: usize) -> usize;
```

- 每条消息：`content.to_text()` + `name`（ToolCall 节点的参数 JSON 在 content 里）逐条 tokenizer 计数，再 + 每消息固定 8 tok 结构开销（role/框架）。
- `overhead_tokens`：system prompt + 工具定义（name/description/schema）的估算，由调用方传入——**压缩触发判断必须计入这部分**，否则阈值虚高。
- 有效上限 = `max_context_tokens - reserved_tokens`。

---

## 3. 目标一：多轮会话膨胀 → 全局快照（L5）

**触发**：`estimate_context_tokens ≥ 70% × 有效上限`，且通过迟滞检查（见下）。

**切分**（`find_compress_split_point`）：
- **只在 User 边界切分**（按字符量累计到 70% 处的最近 User 消息）；
- 保留最近 30%（`COMPRESSION_PRESERVE_THRESHOLD`）为原文明细；
- 待压缩部分 < 5%（`MIN_COMPRESSION_FRACTION`）不压缩；
- **禁止全量压缩**：旧版"末尾是 Assistant 就压缩全部"分支已移除——它违背 30% 保留语义，且可能把进行中的 tool_calls 压成孤儿。若尾部存在未配对的 ToolCall（无结果子节点），本轮放弃压缩（返回 0）。

**执行**（`auto_compress_process`）：
1. `PreCompact` hook → 压缩前**完整历史转存** `transcript_<ts>_<n>.json`（可回溯原则，路径记入快照 meta）；
2. 上下文临时替换为 `[压缩指令]` → `send_compression_request` 生成摘要；
3. **快照校验**：从输出中提取 `<state_snapshot>...</state_snapshot>`；缺失则附带纠正指令重试一次；仍失败则把纯文本摘要包裹为快照（有总比无好）；
4. 新上下文 = `[快照(Assistant, meta.compacted=true, post_tokens=N)] + 保留部分` → `replace_messages` 持久化；
5. 失败（除 Aborted/RateLimited 外）→ 回滚原始上下文，警告后继续对话。

**迟滞**：快照消息 meta 记录压缩后的估算 `post_tokens`；再次触发前若当前估算 `< post_tokens × 1.15`（再增长不足 15%），跳过——防止"压缩后仍超限 → 每轮重复摘要调用"的循环。

---

## 4. 目标二：单轮工具洪峰 → 三道防线

单轮内工具轮次不受硬上限（用户诉求），膨胀由以下机制消化：

1. **L0 守卫**（每条结果写进上下文前）：单条 > 8192 tok → 存档 + head/tail（前 60% / 后 40%，尾部多为结论/错误更关键）。这是"语义上限"，与物理 1MB 上限互补。
2. **L2 fade**（每轮请求前）：单轮工具轮次 > 40 后激活，把"最近 12 个 user turn"之外的工具结果压到 2k tok；**绝不改动 assistant 文本/推理**（保护思维链）。
3. **L6 主动压缩**：模型自知处于阶段间隙时调用 `context_compact`（见 §6），在轮内做语义压缩——这是对"巨型单轮"最有效的手段，因为 L5 的 User 边界切分在单轮场景下找不到切分点。

兜底：L3 窗口骨架化保证请求组装期无论历史多大都能发出。

---

## 5. 目标三：优先关系与取舍

决策顺序（从便宜到昂贵、从精准到模糊）：

| 情形 | 首选机制 | 理由 |
|---|---|---|
| 单条工具结果超大 | L0 | 确定性、零成本、精准定位 |
| 轮内老工具结果堆积 | L2 fade | 确定性、保护近期与思维链 |
| 模型判断阶段完成/历史冗余 | L6 主动压缩 | 语义最优 + 时机最优（模型自知） |
| 跨轮历史膨胀逼近上限 | L5 快照 | 被动兜底，保证不中断 |
| 请求组装期仍超限 | L3 骨架化 | 最后防线，保请求可发 |

**不变量**（任何机制都不得破坏）：
- ToolCall/Tool 配对完整；
- 当前轮的用户指令原文保留（在保留区或快照中）；
- assistant 思维链尽量保留；
- 压缩失败必回滚。

**阈值关系**：L0/L2 常开；55% 水位提醒（L6 引导）；70% 强制快照（L5）；L3 永远兜底。

---

## 6. 目标四：主动压缩（`context_compact` 工具）

**工具定义**：

```json
{
  "name": "context_compact",
  "description": "Compact conversation history: distill older messages into a structured
    state snapshot and keep only recent context. Call this when you have just finished
    a major subtask, when the context is filled with intermediate outputs you no longer
    need, or when a system note warns that context usage is high. The session continues
    seamlessly from the snapshot.",
  "parameters": { "hints": "string — 关键事实/计划/约束，必须保留进快照" }
}
```

**执行语义**（`run_context_compact`，在 chat_loop 工具执行点拦截，不进 CapabilityManager）：

1. 定位锚点：最后一个**真实** User 消息（跳过 `[system note]` 水位提醒）；
2. 切分点：`split ≤ 当前轮首个 ToolCall 所在 Turn 的起始下标`——**进行中的 Turn（含本轮全部 tool_calls）必须整体保留**，其结果在压缩后才追加，配对不破坏；
3. 待压缩历史 < 4000 tok → 返回"无需压缩"（避免浪费一次 LLM 调用）；
4. 复用 L5 的请求/校验/回滚链路生成快照（hints 追加进压缩提示词）；
5. 新上下文 = `[快照] + [当前用户指令] + [进行中 Turn 及之后]` → `replace_messages`；
6. 工具结果（`build_tool_message`）："已压缩：N 条消息 → 快照 + K 条保留（估算 A → B tokens）。请基于快照继续任务。"

**水位提醒（nudge）**：每轮请求前估算用量，≥ 55% 且本轮未提醒时，注入一条 User 角色消息（meta `kind=context_nudge`）："上下文即将达到上限，如处于阶段间隙请调用 context_compact"。一次请求生命周期内最多一次；压缩成功后重置。模型由此在**自己选定的安全时机**主动压缩，而非被 70% 硬触发。

**护栏**：每轮最多一次压缩；失败回滚并返回"压缩已跳过"；全程支持 abort。

---

## 7. 常量集中表

| 常量 | 值 | 位置 | 说明 |
|---|---|---|---|
| `DEFAULT_TOOL_RESULT_TOKEN_CAP` | 8192 | tool_result_guard.rs | L0 单条预算 |
| 压缩行数阈值 | 15 | session/compress.rs | L1 脱水阈值 |
| `FADE_ACTIVATE_ROUNDS` / `FADE_KEEP_RECENT_TURNS` | 40 / 12 | chat_loop.rs | L2 激活/保留 |
| ToolCall 明细窗口 | 15 | session/context.rs | L3 骨架化 |
| `COMPRESSION_TOKEN_THRESHOLD` | 0.7 | compression.rs | L5 触发 |
| `COMPRESSION_PRESERVE_THRESHOLD` | 0.3 | compression.rs | L5 保留比例 |
| `MIN_COMPRESSION_FRACTION` | 0.05 | compression.rs | L5 最小压缩量 |
| `CONTEXT_NUDGE_THRESHOLD` | 0.55 | compression.rs | L6 水位提醒 |
| `COMPACT_HYSTERESIS_FACTOR` | 1.15 | compression.rs | 迟滞再增长系数 |
| `MIN_COMPACT_TOKENS` | 4000 | compression.rs | L6 最小压缩量 |
| `max_messages` | 500 | session 配置 | 存储队列上限 |

---

## 8. 维护原则

1. **单一职责**：每层只处理自己作用域内的问题，跨层行为（如 L0 与 L2 的预算差异）必须在常量表中有注释。
2. **不变量优先**：ToolCall/Tool 配对、User 边界切分、失败回滚——任何新压缩机制必须先证明不破坏这三条。
3. **可观测性**：每次压缩记录 `[DIAG]` 日志（触发原因、前后估算、消息数），便于线上排查"为什么压缩了/为什么没压缩"。
4. **测试锚点**：切分点函数、快照提取、token 估算（含中文用例）、迟滞判断必须有单元测试。
