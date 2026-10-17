---
kind: task
id: T-157-ActorSpec-pattern死字段接线
title: ActorSpec.pattern 生产零读取——三模式是数据存在、行为未分叉，Decider 语义长在三处特判里
status: todo
created: 2026-10-17
track: plan
prio: now
parent: T-156-架构机制化与旧代码清理批
where: symbio/src/symbio_core/actors/spec.rs · symbio/src/plugins/session/chat_loop/state.rs · symbio/src/plugins/session/chat_loop/decide.rs · symbio/src/plugins/session/turn_runner.rs · symbio/src/plugins/classify/
exit: `ActorSpec.pattern` 在生产代码有真实读方且决定走哪条决策路径；Decider 语义的三处实现（classify 规则短路 / turn_runner run_reflex / orchestrator 的 "ping" 特判）在判定处读 spec 而非各自 if；core 形状棘轮不升（不加 trait、不加方法，只让已有字段被读）；cargo test + clippy + 门禁全绿
signal: grep '.pattern' 在 symbio/src 只命中测试断言（actors/mod.test.rs:23,1095,1725 · store/wal.test.rs:206 · chat_loop/state.test.rs:250,273,289）与 vdfs 的 req.pattern（无关域）——ActorSpec.pattern 字段在生产代码**零行为读取**；行为分叉实际由 LatencyTier 驱动（v2_exec.rs 的 run_reflex vs run_with_tools）。Decider（规则决策）语义在系统里有三份独立实现且都不读该字段：classify/rules.rs 规则短路（独立插件）· turn_runner.rs:1503 run_reflex 技能命中快路（内联）· orchestrator/entry.rs:324-339 "ping" 消息特判（硬编码 if）。config.rs:347/361 与 chat_loop/inputs.rs:209 的注释已把 skill_fast_path 描述为「S11 §4 的 actor.pattern」——语义已认领、字段未接线
---

## 内容

父任务：[T-156-架构机制化与旧代码清理批](156-架构机制化与旧代码清理批.md)
依据：[PL-001 §4](../004-plan/001-核心架构.md "PL-001-核心架构")（Actor 三模式 = 一等概念，`pattern` 是 F3 冻结锚点的一部分）·
判据 J1（能用已有参数新取值表达的，就是数据不是机制）· J3（静默失效 → 必须强制）

**问题**：`ActorSpec.pattern` 是 F3 冻结字段集的成员（`actors/spec.rs:8-15` 的
`Pattern::{RuleOnly, ClassifyOnly, FullModel}` 与 `for_principal` 派生），但**生产代码没有任何
一处读它来决定行为**。实际的档位分叉由 `LatencyTier` 驱动。

后果不是「少一个功能」，而是**「Decider（规则决策）」语义在系统里长了三份互不相识的实现**：

| 位置 | 形态 | 读 pattern 吗 |
|---|---|:-:|
| `classify/rules.rs` 规则短路 | 独立插件（问候/致谢/确认/空输入全等匹配） | ✗ |
| `turn_runner.rs::run_reflex` 技能命中快路 | 内联函数（逐字命中技能 ⇒ 零模型调用） | ✗ |
| `orchestrator/entry.rs` "ping" 特判 | 硬编码 `if`（健康检查伪装成用户消息） | ✗ |

三者都属于同一个判据函数的输出（[PL-001 §4](../004-plan/001-核心架构.md "PL-001-核心架构")：
`可写成确定性规则 → Decider`），却各自实现了自己的入口。**这正是「机制化不够、特殊化偏多」的
教科书案例：抽象给了，但它是死的；活的需求各自长出了自己的形状。**

**本条要买到的性质**：新增一种规则决策（新的快路 / 新的健康检查形态）⇒ 落成**一条 ActorSpec 行
的 pattern 取值**，而不是在流程里再加一个 `if`。

## 处理过程

- **不改 F1–F6**：`Pattern` 三值闭集与 `ActorSpec` 五字段集一个字不动。本条只是让**已有字段被读**，
  属于收窄而非变宽（`core-shape-audit` 的 traits/methods/defaults 三格只降不升）。
- **三处特判逐个判定**：`ping` 特判（健康检查不落库不 mark_activity）最接近纯 Decider；
  classify 规则短路已有插件边界，判定是否只需在**调用点**读 spec 而非重构插件；
  `run_reflex` 已由 `skill_fast_path` 开关门控（`config.rs:347` 明确写了它是 pattern 的语义载体），
  接线成本最低。
- **平凡值必须仍完整运行**（J2）：`pattern` 缺省（`ActorSpec::trivial`）取 `Decider`，
  缺省路径的行为不得改变。
- 若勘察后结论是「三处不该合、pattern 本就不该做分叉点」，那也是合法结论——但必须**成文为 ADR
  并从 F3 论证里显式划界**，而不是让字段继续躺着（J3：静默失效不可接受）。
