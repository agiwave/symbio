---
kind: task
id: T-157-ActorSpec-pattern死字段接线
title: ActorSpec.pattern 生产零读取——三模式是数据存在、行为未分叉，Decider 语义长在三处特判里
status: done
created: 2026-10-17
track: plan
prio: now
parent: T-156-架构机制化与旧代码清理批
where: symbio/src/symbio_core/actors/spec.rs · symbio/src/plugins/session/chat_loop/state.rs · symbio/src/plugins/session/chat_loop/decide.rs · symbio/src/plugins/session/turn_runner.rs · symbio/src/plugins/classify/
exit: `ActorSpec.pattern` 在生产代码有真实读方且决定走哪条决策路径；Decider 语义的三处实现（classify 规则短路 / turn_runner run_reflex / orchestrator 的 "ping" 特判）在判定处读 spec 而非各自 if；core 形状棘轮不升（不加 trait、不加方法，只让已有字段被读）；cargo test + clippy + 门禁全绿
evidence: 走「三处不该合、pattern 本就不该做分叉点」这条出口，成文 [ADR-079](../010-adr/079.md "ADR-079")。核出的关键事实是**同名两物**：core 的 `ActorSpec.pattern` 是主体分类（`for_principal` 从授权谓词派生：持 `reply.*` ⇒ `Reasoner`），S11 §4 参数表里的 `actor.pattern` 是逐轮开关（落地为 `SessionConfig::skill_fast_path`）。裁决一给出「接不了」的证明：凡会跑模型的轮次主体都持 `reply.*` ⇒ 派生恒为 `Reasoner` ⇒ 接成分派闸门会把快路永久关死；裁决二把三处特判归到「输入形状」（礼节 / 逐字命中 / 健康探针），彼此不互斥、判据函数没有公分母；裁决三做 F3 划界（冻结的是形状与闭集，不是「必须有分派读方」）。判据：`chat_loop/state.test.rs::the_session_actor_is_a_reasoner_which_is_why_pattern_cannot_dispatch`（分类的反证）+ 既有 `e2e/cases/t41-skill-fast-path.mjs`（谁把 pattern 接成分派点，它立刻红）。同批订正四处自称「S11 §4 的 `actor.pattern`」的注释（`config.rs` 两处、`chat_session.rs`、`chat_loop/inputs.rs`）——「假机制感」的真正来源是这些注释，不是字段本身。
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

- 2026-10-19 **勘察与裁决（取出口的第二条）**。先按任务给的线索逐处核，核出来的第一件事是
  「两个 `pattern` 不是同一个东西」：
  - core 的 `ActorSpec.pattern` 由 `Pattern::for_principal` 从**授权谓词**派生（`reply.*` ⇒ Reasoner、
    `produce.artifact` ⇒ Translator、其余 ⇒ Decider），生产构造点三处、生产读方零处；
  - S11 §4 的 `actor.pattern` 是 capability-assign **参数**（值 `decider` / `reasoner`），落地形态是
    `SessionConfig::skill_fast_path`，生产读方在 `chat_loop/inputs.rs` 的 `fast_armed`。
  代码注释把后者写成「S11 §4 的 `actor.pattern`」，读者于是以为前者就是它——**字段躺着不是缺陷，
  注释许诺了一个它不管的机制才是**（J3）。
- **为什么「接线」不成立**（不是没做，是接不了）：派生函数决定了信息量。`session/main` 与它的子主体
  都持 `reply.*` ⇒ 派生恒为 `Reasoner`；把 `pattern` 接成「走不走模型」的闸门，等于让快路永不命中
  （`t41-skill-fast-path` 当场红）。反过来，凡派生 `Decider` 的主体（仲裁者 / 验证者 / 规划者）本来
  就不跑模型，那条分派早已由**类型**给出（`run_reflex` 只收 `RuleOnly` 令牌）——运行期字段在那里
  也没有事可做。
- **三处特判的共性不在主体**：礼节判定（classify 插件）· 技能逐字命中（`run_reflex`）· 健康探针
  （`entry.rs` 的 `ping`）判的都是**输入形状**，同一个主体上一句与下一句就能切换；三者彼此也不互斥。
  合成一条「按 pattern 分派」的链会把三种没有公分母的判据压成一条。
- **落地**：ADR-079 三节裁决 + 四处注释订正 + 一条判据（`session/main ⇒ Reasoner`）。`Pattern` 三值与
  `ActorSpec` 五字段一个字没动，core 形状棘轮三格不变。**三处特判保持原样**——「不许长第四个」的
  守卫归 [T-163](163-插件层特判审计守卫.md "T-163-插件层特判审计守卫")，本条只划界。
