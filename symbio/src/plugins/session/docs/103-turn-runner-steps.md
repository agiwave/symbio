---
kind: design
id: DE-103-turn-runner-steps
title: `run_with_tools` 的 Step 施工图（DE-090 §1 步骤 1）
status: active
created: 2026-10-17
impl: done
---

# `run_with_tools` 的 Step 施工图（DE-090 §1 步骤 1）

> **这份文档的存在理由**：[DE-090 §1](../../../../../docs/003-design/090-过渡方案.md "DE-090-过渡方案") 步骤 1 要把 `turn_runner.rs` 的 768 行单函数
> （`run_with_tools`，696–1463）收敛成 Step 循环。没有这张映射表，300 行熔断线被突破的
> 原因**不是抽象错了，是缺图**——施工者会在函数里现场发明边界。表在这里，边界就是给定的。

## 1. 环的现状骨架

```text
run_with_tools
├─ 1 用户格 u-{turn}（turn × opened，载荷带 tier / budget_ms / 节点身份）
├─ 1a 开场白 op-{turn}（turn × asserted；空正文不落格）
├─ 1b 定稿答话（无请求路径）→ f-{turn} 或兜底
├─ 1c 闸门与首请求之间的中止检查 → 中止收束格
└─ loop { ← 唯一循环，无 break、无轮次上界
 ├─ Ask：base + exchange → llm.generate_turn
 ├─ Ok(rt)
 │ ├─ 无工具调用 ⇒ 2a 收束（f-{turn} completed / 兜底）
 │ ├─ 有工具调用但无 dispatch ⇒ 兜底
 │ ├─ Act：dispatch.dispatch(&rt) ⇒ 2c 产物格 a-{turn}-{n}
 │ ├─ 需要用户动作 ⇒ 2d 进展格 p-{turn}-{n}（status: pending）
 │ └─ 2e/2f 累加 exchange（工具交换 → 注入）
 ├─ Err(Aborted) ⇒ 3a 中止收束（f-{turn} aborted + terminated）
 └─ Err(e) ⇒ 3b 兜底（I3：到点必答）
 }
```

## 2. 五值 Step 的映射

| 段 | Step | 判据（纯函数可判） | 落的事实 |
|---|---|---|---|
| 1 | `Emit` | 恒真（用户消息先入格） | `u-{turn}` · `EVENT_USER_MESSAGE` · 载荷 `text/tier/budget_ms/id` |
| 1a | `Emit` | `opening_text` 非空 且 `opening_dialog` 在 | `op-{turn}` · `EVENT_ASSISTANT_OPENING` |
| 1b | `Emit` → `Halt` | `final_reply` 在（调用方已定稿，模型不参与） | `f-{turn}` · completed / 兜底（空答话） |
| 1c | `Halt` | `abort.is_aborted` | `f-{turn}` · aborted |
| loop 头 | `Ask` | 恒真（环的唯一模型调用点） | 无（流式帧走 sink 直通，不进事实） |
| 2a | `Halt` | `rt.tool_calls.is_empty` | `f-{turn}` · completed（`finish` 随载荷带出） |
| 2a 空答 | `Emit` → `Halt` | 答话取值为空（含 `finish=length` 的截断） | 兜底格 |
| 2b | `Emit` → `Halt` | 有工具调用但 `dispatch` 缺 | 兜底格（配置缺口，诚实回报） |
| 2c | `Emit` × N | 每条 `outcome`（过 `closure_gate(Artifact)` 闸） | `a-{turn}-{n}` · `EVENT_ARTIFACT_ADDED` · 首格带 `round` |
| 2d | `Emit` → `Halt` | `outcomes.any(needs_user_action)` | `p-{turn}-{n}` · `EVENT_ASSISTANT_PROGRESS` · `status: pending` |
| 2e/2f | （无 Step） | 纯内存状态推进 | 无（`exchange` / `rounds` / `tool_round` 是投影的原料，不是事实） |
| 3a | `Emit` → `Halt` | `Err(AdapterError::Aborted)` | `f-{turn}` · aborted + `terminated` |
| 3b | `Emit` → `Halt` | 其他 `Err` | 兜底格 |

**结论**：环里真正需要「决定下一步」的分支只有 **2a / 2b / 2c / 2d** 四处，
其余都是「入格 + 结束」。这就是 Step 循环要收敛的全部决策面——不是 768 行都要变成 Step。

## 3. 事件批序（重建的顺序锚）

重建按**发射序**（`seq` 由 `nodes_of` 统一发号），不是按语义分组。一轮的合法批序：

```text
u-{turn} → [op-{turn}] → { [a-{turn}-0 … a-{turn}-n] → [p-{turn}-m] }* → f-{turn}
```

三条硬约束：

1. **产物格必须在本轮中途产出入格**（`round` 载荷挂第一格）——只记在收束格的 `rounds[]`
 里会让重开会话后中途思考整体挪到全部工具卡片之后（实测会话 `fixverify01`）。
2. **进展格可能有多条**（恢复后又等待），事件 id 按本轮已有进展格数编号，从 0 起会撞
 幂等键。
3. **收束格全轮唯一**（`f-{turn}`），C2 不变量按它判；等待轮不收束。

## 4. 静息判定

「这一轮停下来了吗」只有三个判据，且都在**执行结果**上，不在模型回复上：

| 静息态 | 判据 | 落格 |
|---|---|---|
| 等用户动作 | `outcome.needs_user_action`（`ask_user` / 审批卡） | 进展格 `status: pending`，`TurnOutcome.awaits_user = true` |
| 工具在途 | 环内**同步** `dispatch.dispatch`，无在途态 | 不适用（v2 路径的工具是批同步） |
| 用户中止 | `abort.is_aborted`（1c 与 `Err(Aborted)` 两处） | 中止收束格 |

⚠️ **Step 化时必须守住的一点**：v2 没有「工具在途」这个静息态——环内批同步，表里那一行标着
「不适用」。把主循环侧的「等批次齐备」判据搬进运行器，等于把批同步改成异步在途，
那是**行为变更**而不是重构。v1 外壳留下的那份簿记与闸门的对应态已退场
（[D-073](../../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")）。

## 5. 失败分支映射（DE-014 §3 规则 e）

失败是事实，不是第六个 Step 值。四个失败分支各自落到哪：

| 分支 | 触发 | 落的事实 | `TurnOutcome` 字段 |
|---|---|---|---|
| 兜底 | 模型空答 / 请求异常 / 无 dispatch | 兜底格（`fell_back` 形状） | `fell_back: true` |
| 中止 | `AdapterError::Aborted` / 1c 检查 | 中止收束格（`terminated`） | `aborted: true` |
| 授权拒绝 | `closure_gate` 判拒 | **不落格**（本轮留未收束，C4 报得出）+ 落一条 `authz.denied` | `closure_denied: true` |
| 截断 | `finish.is_length` 且正文空 | 不提升思考为正文，草稿仍作思考事实入格 | `finish: Length` |

**判据**：兜底与中止**绝不复用彼此的格**——把「用户按了停止」记成「模型答不出」会抬高
兜底率，反之则让中止率失真。这条在 Step 化后必须保持。

## 6. 迁移顺序（切三刀，每刀一个提交）

| 刀 | 内容 | 判据 |
|---|---|---|
| **1a** | 本文档（施工图） | 文档评审通过 |
| **1b** | 抽出**纯决策**：`classify_round(&rt, has_dispatch) -> RoundDecision`（`Halt` / `Act` / `ConfigGap`），环体改成 `match classify_round(...)` | 单测钉四个分支；e2e t27 + 全量 e2e 零漂移 |
| **1c** | 抽出**收束**：2a 的答话取值（`text` → 截断 → `reasoning` → 桥上的思考）成为纯函数 `final_answer(&rt, &round_facts)` | 单测钉截断分支（ 13KB 草稿用例） |

三刀之后，环体只剩「Ask → match(decision) → 执行 → 推进」四段，768 行里属于**决策**的
部分全部可单测。剩下的搬迁（2c 产物批、2d 进展格）留给步骤 4 的行表一并做——
它们需要 `push` 的产物契约与 `policy.report` 的义务查询才能真正脱离本函数，提前搬只是换位置。

## 7. 进度

- **2026-10-17：1a / 1b / 1c 已落地（同一提交）**——`classify_round`（四组合）与
 `final_answer`（四条取值顺序）已抽出为纯函数，单测落在
 `turn_runner.test.rs::round_decision_tests`（3 例）。环体由此只剩
 「Ask → 决策 → 执行 → 推进」四段，768 行里属于**决策**的部分全部可单测。
 `truncated` 保留在调用点：兜底话术要区分「被截断」与「空答」，它不是取值逻辑。
- **剩余（不属于步骤 1）**：2c 产物批与 2d 进展格的搬迁依赖行表的义务查询（`policy.report` /
 `report_due`）与 `turn_exec` 的产物契约，随步骤 4 一起做——提前搬只是换位置，换不来可测性。
