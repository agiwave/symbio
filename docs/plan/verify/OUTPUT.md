# 验证程序实测输出

> 环境：最初实测于 rustc 1.98.1 (48a229cea 2026-09-01)；当前判据环境为 `rust-toolchain.toml` 的 1.93.1——
> 由 `scripts/gate.d/55-verify.mjs`（CI job `verify-evidence`）每次 push 重新编译运行这 14 个程序、断言退出码，
> 并以 `--cfg feature="should_not_compile"` 断言那 3 个反例**必须**编译不过。
> **判据在那个阶段里，不在本文件**：本文只是某次实测的留存，陈旧不会让门禁变绿或变红——「快照不是事实来源」。
> 命令：`rustc --edition 2021 <file>.rs -o <out> && ./<out>` → 全部 exit 0，零 error 零 warning。
>
> **共 14 个程序。** 其中 3 个（`projection_purity` / `latency_gate` / `conation_minimal`）
> 额外带**编译期反向用例**：用
> `rustc --edition 2021 --check-cfg 'cfg(feature)' --cfg 'feature="should_not_compile"' <file>.rs -o sf.exe`
> 验证它们**必然编译失败**（退出码非 0）。这是"验证纪律第 4 条"的落地：
> **声称"编译期强制"的，必须附一段已知编译不过的代码**——没有它，
> 无法区分"真的强制"与"恰好没写错"。

## lower_bound.rs

```
═══ 职责合并检验（由冲突矩阵计算，非硬编码）═══

── 标准模型 ──
  Hold(持有+写门) ⊕ Derive(纯派生) → 可合并
  Hold(持有+写门) ⊕ Act(行为+副作用) → 不可合并（direct_write：Hold(持有+写门) 禁止 / Act(行为+副作用) 要求）
  Derive(纯派生) ⊕ Act(行为+副作用) → 不可合并（side_effect：Derive(纯派生) 禁止 / Act(行为+副作用) 要求）
  分组 1：{Hold(持有+写门), Derive(纯派生)}
  分组 2：{Act(行为+副作用)}
  不可再分分组数 = 2

── 反事实 A：Act 不需要 direct_write ──
  Hold(持有+写门) ⊕ Derive(纯派生) → 可合并
  Hold(持有+写门) ⊕ Act(行为+副作用) → 可合并
  Derive(纯派生) ⊕ Act(行为+副作用) → 不可合并（side_effect：Derive(纯派生) 禁止 / Act(行为+副作用) 要求）
  分组 1：{Hold(持有+写门), Derive(纯派生)}
  分组 2：{Act(行为+副作用)}
  不可再分分组数 = 2

── 反事实 B：Derive 不禁 side_effect ──
  Hold(持有+写门) ⊕ Derive(纯派生) → 可合并
  Hold(持有+写门) ⊕ Act(行为+副作用) → 不可合并（direct_write：Hold(持有+写门) 禁止 / Act(行为+副作用) 要求）
  Derive(纯派生) ⊕ Act(行为+副作用) → 可合并
  分组 1：{Hold(持有+写门), Derive(纯派生)}
  分组 2：{Act(行为+副作用)}
  不可再分分组数 = 2

── 反事实 C：两者同时关掉 ──
  Hold(持有+写门) ⊕ Derive(纯派生) → 可合并
  Hold(持有+写门) ⊕ Act(行为+副作用) → 可合并
  Derive(纯派生) ⊕ Act(行为+副作用) → 可合并
  分组 1：{Hold(持有+写门), Derive(纯派生), Act(行为+副作用)}
  不可再分分组数 = 1

═══ 反事实检验（判据是否真在参与计算）═══
  标准 Hold⊕Act 冲突 = true；反事实 A 下 = false
  标准 Derive⊕Act 冲突 = true；反事实 B 下 = false

═══ 下界：两种记账立场 ═══
  Act 计入原语        → 原语下界 = 2
  Act 计为一等概念    → 原语下界 = 1
  （本方案立场：Act 是「使用者」不是「构成者」，故取 1）

✅ 全部断言通过（含 4 条反向用例）：下界由计算得出，且判据对输入敏感。
```

## invariants.rs

```
═══ 三条不变量的可执行判据 ═══

健康日志（6 条事件）→ 违规 0 条
注入违规后 → 违规 4 条：
  I1SeqNotMonotonic { seq: 4, prev: 6 }
  I1DoubleFinal { turn: 1 }
  I2NoProvenance { seq: 8 }
  I3BudgetExceeded { seq: 9, cost_ms: 5000, budget: 1500 }

═══ 反向用例：关掉检查，违规必须消失 ═══
  关掉 I1 → 剩 2 条（少了 2 条）
  关掉 I2 → 剩 3 条（少了 1 条）
  关掉 I3 → 剩 3 条（少了 1 条）

✅ 全部断言通过（含 3 条反向用例）：三条不变量各自独立起作用。
```

## mechanism_growth.rs

```
═══ 元素数守恒审计 ═══
机制表（恒定）：10 项
场景数：20
越界键（需要新机制的）：0 个

机制种类数随场景数的变化：
  场景 1 个 → 用到机制 5 种
  场景 5 个 → 用到机制 5 种
  场景 10 个 → 用到机制 8 种
  场景 15 个 → 用到机制 9 种
  场景 20 个 → 用到机制 9 种
  全量场景 → 用到机制 9 种（机制表 10 项，未用 1 项为延迟启用）

反向用例：加入含 'wizard.mode' 的场景 → 越界键 1 个

✅ 全部断言通过（含 1 条反向用例）：机制数不随场景数增长。
```

## capability_frame.rs

```
═══ 能力坐标系完备性 ═══
能力条目：54
归类失败：0 条

覆盖矩阵（行=五维，列=五轴）：
               主动性        时域跨度        抽象层级      自我模型精度        自主程度  
  知             1           5           4           0           0  
  行             1           1           6           2           3  
  言             2           3           3           0           0  
  省             0           4           3           5           4  
  欲             3           0           0           1           3  
空格子：7 / 25（这些是坐标系预留的生长位，不是缺口）

各维条数：
  知：10
  行：13
  言：8
  省：16
  欲：7

═══ 伪高阶识别 ═══
  工具数量多 → 伪高阶 = true
  上下文更长 → 伪高阶 = true
  反应更快 → 伪高阶 = true
  支持更多模型 → 伪高阶 = true
  能跑很久 → 伪高阶 = true
  会写代码 → 伪高阶 = true
  校准（说到做到率） → 伪高阶 = false

反向用例：假维度 = true，假轴 = true，越界天梯 = true

✅ 全部断言通过（含 4 条反向用例）：五维完备、五轴有刻度、伪高阶可识别。
```

## termination.rs

```
═══ 终止性三前提 ═══
健康任务树 → 终止性成立
破坏前提 1（seq 重复）→ Some("前提 1 失效：seq 非严格单调")
破坏前提 2（返工超限）→ Some("前提 2 失效：返工次数超过上界 3")
破坏前提 3（1↔2 成环）→ Some("前提 3 失效：依赖图有环或存在悬空依赖")
  is_acyclic(环) = false
悬空依赖 → is_acyclic = false（悬空也必须判否）

═══ 反向用例：关掉前提，保证必须失效 ═══
  忽略环检测后 is_acyclic(环) = true（必须变 true，说明检查在起作用）

✅ 全部断言通过（含 2 条反向用例）：三前提各自独立必要。
```

## budget.rs

```
═══ 四层时延 = 同一 Actor 的四组 budget 取值 ═══
          反射 (<80ms)  budget =        80 ms   允许模型调用 = false
         快速 (~300ms)  budget =       300 ms   允许模型调用 = true
           深度 (秒–分钟)  budget =     60000 ms   允许模型调用 = true
           自主 (小时–天)  budget =  86400000 ms   允许模型调用 = true

═══ I3 到点必答 ═══
  反射层耗时 20ms  → Event("chat.assistant.final")
  反射层耗时 500ms → Fallback("chat.assistant.fallback")（超预算，走兜底）
  反射层耗时 500ms 且不强制 → Silent（静默超时，违规）

  违规判定：
    正常   → 违规 = false
    兜底   → 违规 = false（兜底是合法的，兜底话术本身也是事件，可审计）
    静默   → 违规 = true

═══ 反向用例 ═══
  把预算放宽到自主层（86400000ms）后，500ms 的耗时 → Event("chat.assistant.final")
  即：预算取值决定了是否被判定超时，契约对参数敏感

✅ 全部断言通过（含 1 条反向用例）：I3 到点必答成立。
```

## cognitive_fit.rs

> 用途：拿一整套外部「认知架构体系」当负载，压测 v2 骨架是否需要重构。
> 需求 = 一组（机制键, 取值）赋值向量；越界键 = 需要新增机制 = 重构。

```
═══ 认知架构体系 → v2 承载压力测试 ═══

── Q1/Q2：需求映射审计 ──
机制表（恒定）：10 项
认知体系需求：16 条
  C01 七类认知内容入库（知识/经验/技能/判断/策略/直觉/情绪）
  C02 内容密度金字塔（原文/要点/摘要/模式）
  C03 Level 0-4 通用性分级与过滤（读侧，与遗忘同构）
  C04 五层存储（原始/情景/语义/技能/元）
  C05 激活扩散检索（关系网络多跳）
  C06 向量索引 + 关系索引 + 时间分区
  C07 时间衰减 + 使用反馈调权
  C08 内外双轨动作空间（Internal / External）
  C09 系统 2 → 系统 1 技能编译
  C10 个人认知归属与隔离
  C11 跨主体借用他人认知（人 → 智能体）
  C12 动态学习：巩固（压缩 + 反事实）
  C13 动态学习：遗忘（投影排除，非物理删除）
  C14 认知注入上下文（检索结果进提示词）
  C15 认知置信度校准（元认知）
  C16 L1-L5 能力演进（自主层 + 子作用域）

越界键（= 必须新增机制 = 重构）：0 个
需扩展值域的需求：0 条 → []
认知体系用到的机制种类：10 / 10

── Q3.1：后台巩固是否污染在途视图 ──
  as-of=4 巩固前：["raw-4", "raw-3", "raw-2"]
  as-of=4 巩固后：["raw-4", "raw-3", "raw-2"]
  （错实现，忽略 as-of）：["consolidated-b", "consolidated-a", "raw-4"]
  as-of=6：["consolidated-b", "consolidated-a", "raw-4"]

── Q3.2：扩散检索的预算有界性（I3）──
  预算充足：nodes=19683 degraded=false used_ms=320
  预算不足：nodes=147 degraded=true used_ms=80
  （错实现，静默超时）：nodes=0 degraded=false
  放大预算后：degraded=false

── Q3.3：通用性过滤的可逆性（读侧 vs 写侧）──
  阈值=2        读侧 ["L2-个人经验", "L3-个人判断"]
  阈值=2        写侧 ["L2-个人经验", "L3-个人判断"]
  阈值放宽到 0  读侧 ["L0-通用常识", "L1-通用术语", "L2-个人经验", "L3-个人判断"]
  阈值放宽到 0  写侧 ["L2-个人经验", "L3-个人判断"]

✅ 全部断言通过（含 6 条反向用例）。
   结论：认知体系 16 条需求全部落在已有机制键上 → 承载无需重构；
   且无需扩展任何值域——通用性过滤与 §9.3 遗忘同构，归入读侧投影参数即可。
```

> **Q3.3 的读法**：阈值 = 2 时读侧与写侧结果完全一致（过滤效果不打折）；
> 阈值放宽到 0 时读侧能召回全部 4 条，写侧仍是 2 条——**写侧过滤不可逆，内容已永久丢失**。
> 这是"通用性过滤必须放读侧"的决定性理由，也是 S10 那处"架构改动"被复核推翻的依据。

## scenario_ladder.rs

> 用途：把 roadmap/ 里 13 阶能力阶梯的赋值向量集中审计，验证"从 S01 到 S13 全程 0 新增机制键"。

```
═══ 能力扩充阶梯审计（13 阶）═══

| 阶 | 场景 | 天梯 | 新增键 | 累计 |
|---|---|:-:|:-:|:-:|
| S01 | 最小闭环（对话基线） | T1 | 6 | 6 |
| S02 | 工具调用与产物 | T1-T2 | 0 | 6 |
| S03 | 多步任务与返工 | T2 | 1 | 7 |
| S04 | 并发调度与租约 | T3 | 0 | 7 |
| S05 | 长会话与断点恢复 | T3 | 1 | 8 |
| S06 | 长期记忆与语义检索 | T3 | 0 | 8 |
| S07 | 插话与实时打断 | T3-T4 | 0 | 8 |
| S08 | 多主体与对等承诺 | T4-T5 | 2 | 10 |
| S09 | 外部执行与熔断 | T5-T6 | 0 | 10 |
| S10 | 个人认知体系注入 | T4-T6 | 0 | 10 |
| S11 | 技能编译与自我改进 | T6 | 0 | 10 |
| S12 | 自主层与长期目标 | T7 | 0 | 10 |
| S13 | 多智能体社会 | T8 | 0 | 10 |

── Q1 越界键（= 需新增机制 = 重构）：0 个
── Q2 累计机制键：10 / 10（机制表恒定 10 项）
── Q4 需扩展值域的阶：[]（应为 0：任何非 0 都必须走 ADR 前置闸门）

后 5 阶（S09-S13）新增键全为 0：true
零新增键的阶数：9 / 13

── 反向用例 ──
  加一个需新机制的 S14 → 越界键 1 个
  加一个回退值等于运行值的 S99 → J2 违例 1 个
  删掉 S03 的 scope → 累计键 10 → 9

✅ 全部断言通过（含 4 条反向用例）。
   13 阶能力扩充 = 0 新增机制键 + 0 值域扩展（架构改动 0）；
   后 5 阶只取新值，不引入新键、不改任何已有键的语义。
```

## module_deps.rs

> 用途：回答"沿路线图扩充会不会长出复杂的模块间依赖"。分四层，本程序验证 L4（派生链）。

```
═══ 模块依赖 · 派生链与代数 ═══

── L4.1 派生图是否有环 ──
  健康日志：方向违例 0 条，有环 false
  坏日志  ：方向违例 1 条，有环 true
  弱检查器（只看有无溯源）对坏日志报 0 条

── L4.2 代数发散与代数上界 ──
  无上界  ：事件 13 条，最大代数 12
  上界=3  ：事件 4 条，最大代数 3
  上界=5  ：事件 6 条，最大代数 5

✅ 全部断言通过（含 2 条反向用例）。
   派生图无环由 I1 的 seq 单调保证（溯源边只能指向更早的 seq），不是由 I2 保证；
   但代数发散架构不禁止 —— 必须靠「代数上界」这个参数兜住，且要做成 CI 断言（J3 静默失效）。
```

> **关键读法**：「弱检查器报 0 条」是这条最重要的输出——
> 只检查"有没有溯源"（I2）抓不到环，**必须检查溯源方向**（来自 I1 的 seq 单调）。
> 所以派生图无环是 I1 的推论，不是 I2 的。

---

# 改进轮次新增的 5 个验证程序（本轮）

> 以下 5 个程序对应"按建议改进 plan3"的 5 项落地。
> 前 3 个把原有声称从"约定 / 事后断言"升级为"编译期强制 / 构造期约束 / 质量约束"；
> 第 4 个补上守恒审计的盲区；第 5 个把「欲」维度从分类学断言做成可执行形态。
> **全部实测编译运行通过；带 `should_not_compile` 的 3 个程序，其反向用例均以退出码 1 失败（符合预期）。**

## projection_purity.rs（改进一 · 投影纯度升级为编译期强制）

```
═══ 投影纯度 · 编译期强制（非约定、非 CI 扫描）═══

── 确定性 ──
  第 1 次: [(4, "user.message")]
  第 2 次: [(4, "user.message")]

── 与外部时钟无关（now 是参数，不是被读的 Clock）──
  now=0        → [(4, "user.message")]
  now=u64::MAX → [(4, "user.message")]

── 反向用例 A：预算真的在算吗 ──
  budget.tokens=2   → value=2 degraded=true
  budget.tokens=100 → value=4 degraded=false

── 反向用例 B：改事件，结论必须变 ──
  4 条事件 → [(4, "user.message")]
  5 条事件 → [(4, "user.message"), (5, "task.created")]

── 能力令牌 PureOnly ──
  令牌签发 → 认证投影可运行，值 = [(4, "user.message")]
  令牌收据 → hold-only

── 投影不产生写 ──
  投影前后 Store.write_count: 0 → 0

═══ 编译期强制的反向用例 ═══
  本文件的 `should_not_compile` feature 里放了三段代码：
    (a) 投影想把 &Store 当形参传进来（顺手写一笔）→ 必须编译失败
    (b) 外部模块伪造 PureOnly 令牌（访问私有字段）→ 必须编译失败
    (c) 投影在体内直接读系统时钟 → 【形状合法，编译得过】
        ——(c) 是刻意保留的边界：一级强制只守"句柄进不来"，
           守不住"内部调全局函数"，后者必须靠 CI 静态扫描（二级强制）。

✅ 全部断言通过（含 2 条数据反向用例 + 2 条编译期反向用例）
```

实测编译期反例（`--cfg 'feature="should_not_compile"'`，退出码 1）：

```
error[E0593]: closure is expected to take 3 arguments, but it takes 4 arguments   ← (a)
error[E0451]: field `_private` of struct `PureOnly` is private                    ← (b)
```

> **诚实边界**：反向用例 (c) 编译**通过**（只有 warning）。这暴露了一级强制的真实边界——
> 类型系统能守"句柄从签名进来"，守不住"函数体内调全局"。后者留给 CI 静态扫描（二级强制）。
> **把这条写出来，比假装"全都强制住了"更有价值。**

## latency_gate.rs（改进二 · 时延档位做成事前闸门）

```
═══ 时延闸门 · 事前约束（不是声明 + 事后断言）═══

── 四层预算 ──
  Reflex: budget_ms = 80
  Fast: budget_ms = 300
  Deep: budget_ms = 60000
  Autonomic: budget_ms = 86400000

── 档位 × 模型许可 ──
  反射: 模型=false 生成=false
  快速: 模型=true 生成=false
  深度: 模型=true 生成=true

── 按档位装配（事前闸门的结果）──
  Reflex: rules=true classifier=false llm=false
  Fast: rules=true classifier=true llm=false
  Deep: rules=true classifier=true llm=true

── 反向用例 A：事前闸门 vs 事后断言 ──
  旧做法（先调模型再兜底）: Fallback("chat.assistant.fallback")
  新做法（闸门）          : 反射档 has_llm = false → 不可能发生 2000ms 的模型调用

── 反向用例 B：改预算，结论必须变 ──
  500ms @ 反射(80ms)   → Fallback("chat.assistant.fallback")
  500ms @ 深度(60s)    → Event("chat.assistant.final")

── I3 静默超时判定 ──
  正常   Event("chat.assistant.final") → 违规=false
  兜底   Fallback("chat.assistant.fallback") → 违规=false
  静默   Silent → 违规=true

── 零运行时开销 ──
  size_of::<RuleOnly>()      = 0
  size_of::<ClassifyOnly>()  = 0
  size_of::<FullModel>()     = 0

✅ 全部断言通过（含 2 条数据反向用例 + 2 条编译期反向用例）
```

实测编译期反例（退出码 1）：

```
error[E0277]: the trait bound `RuleOnly: CanClassify` is not satisfied   ← (a) 反射档调分类器
error[E0308]: mismatched types                                           ← (b) 快速档调 LLM
```

> **关键读法**：`Reflex: llm=false` 不是"运行时判断"，而是**装配结果**——
> 反射档的 Actor 结构上就没有 LLM 字段。超时因此从"事后兜底"变成"结构上不可能"。

## consolidation_fidelity.rs（改进三 · 巩固补信息保真度下界）

```
═══ 巩固保真度 · 质量约束（max_gen 之外补 min_fidelity）═══

── 现象：只有 max_gen 时会发生什么 ──
  gen 0: 保真度 = 1.0000
  gen 1: 保真度 = 0.8006
  gen 2: 保真度 = 0.6188
  gen 3: 保真度 = 0.5060
  gen 4: 保真度 = 0.4143
  gen 5: 保真度 = 0.3825
  最终代数 = 5（≤ max_gen=5，形态合规）
  最终保真度 = 0.3825  ← 代数合规，但这条记忆还准吗？

── 加固：加 min_fidelity = 0.7 ──
  gen 0: 保真度 = 1.0000
  gen 1: 保真度 = 0.8006
  最终代数 = 1（**小于** max_gen=5 —— 质量约束先于形态约束生效）
  最终保真度 = 0.8006 ≥ 0.7

── 反向用例 A：改阈值，结论必须变 ──
  min_fidelity=0.9 → 实际代数 = 0
  min_fidelity=0.7 → 实际代数 = 1
  min_fidelity=0.5 → 实际代数 = 3
  min_fidelity=0.3 → 实际代数 = 10
  严格(0.9) = 0 代 < 宽松(0.3) = 10 代 → 阈值确实在参与计算

── 反向用例 B：min_fidelity 的平凡值 ──
  min_fidelity=0.0 → 代数 = 5（应等于纯 max_gen 的 5）

── 拒绝理由可区分（不静默）──
  拒绝：保真度 0.8006 < 下界 0.9500

✅ 全部断言通过（含 3 条反向用例 + 1 条平凡值用例）
```

> **关键读法**：`max_gen` 是**形态约束**（防无限推进），`min_fidelity` 是**质量约束**（防失真固化）。
> 只守前者会稳定产出"代数合规但已经失真"的记忆——两个指标都不报错，坏的是它们之间的差。
> 这与业界经验法则吻合：1 跳可缓存（阈值 0.7 → 停在 gen 1）、≥3 跳从源重推（阈值 0.5 → 停在 gen 3）。

## semantic_drift_audit.rs（改进四 · 语义漂移审计进 CI）

```
═══ 语义漂移审计 · 架构适应度函数 ═══
（四个维度：D1 值域滑移 / D2 语义边界漂移 / D3 平凡值滥用 / D4 表—码失配）

########## 第一步：负样本（必须全部报警）##########
  ✗ [D1 值域滑移] 场景「漂移样本-值域滑移」用了 `projection=semantic_dedup`，但权威参数表没有这个取值。
  ✗ [D1 新机制键] 场景「漂移样本-新机制键」用了参数表里没有的键 `memory.compress_algo`。
  ✗ [D2 语义边界漂移] `projection.param=consolidate:max_gen` 在不同场景里代表了不同东西：
      [("场景 S06", "最多巩固几代"), ("场景 S10", "记忆还剩多少可用信息")]
  ✗ [D3 平凡值滥用] `projection=eval` 出现占比 100% > 阈值 90%。
  → 四个检查器全部会响（不是摆设）✓

########## 第二步：正样本（必须零报警）##########
  D1 无漂移 ✓  D2 无漂移 ✓  D3 无漂移 ✓  D4 无漂移 ✓
  → 正样本零报警（无假阳性）✓

########## 第三步：D4 负样本（表—码失配）##########
  ✗ [D4 表—码失配] 代码里独有的键：[]；表里独有的键：["legacy_bus"]

✅ 全部断言通过（4 个检查器 × 各自的负样本 + 正样本无假阳性 + D4 负样本）
```

> **关键读法**：这个审计器补的是 `mechanism_growth.rs` / `scenario_ladder.rs` 的**盲区**。
> 那两个问"有没有发明新机制"；本审计问"**已有机制的语义有没有滑走**"。
> G7（巩固代数发散）正是后者——两个旧审计器都报合规，因为语义不在它们的度量里。
> **D3 的一个设计要点**：平凡值阈值必须**按参数分别配置**（`D3_ENFORCED`）——
> `scope=root` 在大多数场景就是正常形态，要求它"充分分化"是荒谬的。

## conation_minimal.rs（能力补强 · 「欲」的最小可执行形态）

```
═══ 「欲」维度的最小可执行形态 ═══
（E1 欲是事实 / E2 欲行分离可编译强制 / E3 欲有平凡值）

── E1：欲 = 一条事件 ──
  日志 = ["conation.expressed", "system.triggered"]
  投影得到未完成意图 = ["推进长期目标"]
  → 「欲」用的是**唯一原语 Store**，没有新增机制 ✓

── E2：欲 → 行的类型闸门 ──
  候选意图 approved = false（造出来一定是 false，不能伪造）
  过闸后：approved = true，产出任务 = ApprovedTask { from_seq: 1, goal: "推进长期目标" }
  过宽目标 → Err("goal too broad")
  无溯源的欲 → 能否构造候选意图：false

── E3：欲的平凡值 ──
  平凡值 enabled = false
  关闭欲后 approve → Err("conation disabled")
  → 关闭『欲』= 一键退回纯响应式，而系统其余部分完好 ✓

── 零运行时开销 ──
  size_of::<GateWarrant>() = 0

── 准入判定复核（02 §4 四问）──
  → 4 项「欲」能力全部落在**已有机制键的新取值**上，新增机制键 = 0 ✓

✅ 全部断言通过
```

实测编译期反例（退出码 1）：

```
error[E0451]: field `approved` of struct `CandidateIntent` is private      ← (a) 伪造"已批准"意图
error[E0451]: field `_private` of struct `GateWarrant` is private          ← (b) 伪造能力令牌
error[E0061]: this function takes 3 arguments but 2 arguments were supplied ← (c) 无令牌调 approve
```

> **⚠️ 一个必须记下的坑**：编译期反向用例**必须写在外部模块**里。
> `CandidateIntent.approved` 与 `GateWarrant._private` 都是私有字段，但**同模块内可访问**——
> 若把用例写在本文件主模块里，它反而会**编译通过**（笔者第一版即如此）。
> 正解：放进 `mod outsider { ... }`，才触发真正的可见性错误。这条已写入 README 验证纪律第 4 条。


---

## projection_purity.rs（投影纯度 · 编译期强制）

```
═══ 投影纯度 · 编译期强制（非约定、非 CI 扫描）═══

── 确定性 ──
  第 1 次: [(4, "user.message")]
  第 2 次: [(4, "user.message")]

── 与外部时钟无关（now 是参数，不是被读的 Clock）──
  now=0        → [(4, "user.message")]
  now=u64::MAX → [(4, "user.message")]

── 反向用例 A：预算真的在算吗 ──
  budget.tokens=2   → value=2 degraded=true
  budget.tokens=100 → value=4 degraded=false

── 反向用例 B：改事件，结论必须变 ──
  4 条事件 → [(4, "user.message")]
  5 条事件 → [(4, "user.message"), (5, "task.created")]

── 能力令牌 PureOnly ──
  令牌签发 → 认证投影可运行，值 = [(4, "user.message")]
  令牌收据 → hold-only

── 投影不产生写 ──
  投影前后 Store.write_count: 0 → 0

═══ 编译期强制的反向用例 ═══
  本文件的 `should_not_compile` feature 里放了三段代码：
    (a) 投影想把 &Store 当形参传进来（顺手写一笔）→ 必须编译失败
    (b) 外部模块伪造 PureOnly 令牌（访问私有字段）→ 必须编译失败
    (c) 投影在体内直接读系统时钟 → 【形状合法，编译得过】
        ——(c) 是刻意保留的边界：一级强制只守"句柄进不来"，
           守不住"内部调全局函数"，后者必须靠 CI 静态扫描（二级强制）。
  验证命令（(a)(b) 应当以非 0 退出）：
    rustc --edition 2021 --check-cfg 'cfg(feature)' \
          --cfg 'feature="should_not_compile"' projection_purity.rs -o should_fail.exe

✅ 全部断言通过（含 2 条数据反向用例 + 2 条编译期反向用例）
   结论：投影纯度的一级强制（句柄进不来）由类型系统保证；二级强制（无隐式全局依赖）
         必须留给 CI 静态扫描 —— 两级合起来才是完整的 J3 落地。
```

---

## latency_gate.rs（时延闸门 · 事前约束）

```
═══ 时延闸门 · 事前约束（不是声明 + 事后断言）═══

── 四层预算 ──
  Reflex: budget_ms = 80
  Fast: budget_ms = 300
  Deep: budget_ms = 60000
  Autonomic: budget_ms = 86400000

── 档位 × 模型许可 ──
  反射: 模型=false 生成=false
  快速: 模型=true 生成=false
  深度: 模型=true 生成=true

── 按档位装配（事前闸门的结果）──
  Reflex: rules=true classifier=false llm=false
  Fast: rules=true classifier=true llm=false
  Deep: rules=true classifier=true llm=true

── 反向用例 A：事前闸门 vs 事后断言 ──
  旧做法（先调模型再兜底）: Fallback("chat.assistant.fallback")
  新做法（闸门）          : 反射档 has_llm = false → 不可能发生 2000ms 的模型调用

── 反向用例 B：改预算，结论必须变 ──
  500ms @ 反射(80ms)   → Fallback("chat.assistant.fallback")
  500ms @ 深度(60s)    → Event("chat.assistant.final")

── I3 静默超时判定 ──
  正常   Event("chat.assistant.final") → 违规=false
  兜底   Fallback("chat.assistant.fallback") → 违规=false
  静默   Silent → 违规=true

── 零运行时开销 ──
  size_of::<RuleOnly>()      = 0
  size_of::<ClassifyOnly>()  = 0
  size_of::<FullModel>()     = 0

── 档位证据 ──
  for_tier(Deep) → tier = Deep

═══ 编译期强制的反向用例 ═══
  `should_not_compile` feature 里放了两段【必须编译失败】的代码：
    (a) 反射档令牌 RuleOnly 去调分类器 → 不满足 CanClassify
    (b) 快速档令牌 ClassifyOnly 去调 LLM → 不满足 FullModel
  验证命令（应当以非 0 退出）：
    rustc --edition 2021 --check-cfg 'cfg(feature)' \
          --cfg 'feature="should_not_compile"' latency_gate.rs -o should_fail.exe

✅ 全部断言通过（含 2 条数据反向用例 + 2 条编译期反向用例）
   结论："反射层不得调模型"由类型保证 —— 它是**构造期约束**，
         不是运行时断言。超时从"事后兜底"变成"结构上不可能"。
```

## consolidation_fidelity.rs（巩固保真度 · 质量约束）

```
═══ 巩固保真度 · 质量约束（max_gen 之外补 min_fidelity）═══

── 现象：只有 max_gen 时会发生什么 ──
  gen 0: 保真度 = 1.0000
  gen 1: 保真度 = 0.8006
  gen 2: 保真度 = 0.6188
  gen 3: 保真度 = 0.5060
  gen 4: 保真度 = 0.4143
  gen 5: 保真度 = 0.3825
  最终代数 = 5（≤ max_gen=5，形态合规）
  最终保真度 = 0.3825  ← 代数合规，但这条记忆还准吗？

── 加固：加 min_fidelity = 0.7 ──
  gen 0: 保真度 = 1.0000
  gen 1: 保真度 = 0.8006
  最终代数 = 1（**小于** max_gen=5 —— 质量约束先于形态约束生效）
  最终保真度 = 0.8006 ≥ 0.7

── 反向用例 A：改阈值，结论必须变 ──
  min_fidelity=0.9 → 实际代数 = 0
  min_fidelity=0.7 → 实际代数 = 1
  min_fidelity=0.5 → 实际代数 = 3
  min_fidelity=0.3 → 实际代数 = 10
  严格(0.9) = 0 代 < 宽松(0.3) = 10 代 → 阈值确实在参与计算

── 反向用例 B：min_fidelity 的平凡值 ──
  min_fidelity=0.0 → 代数 = 5（应等于纯 max_gen 的 5）

── 拒绝理由可区分（不静默）──
  拒绝：保真度 0.8006 < 下界 0.9500

── 溯源方向 ──
  gen0 seq=100 ← gen3 seq=103（溯源必须指向更早）
  溯源链末端 seq = 102 ≤ 起点 = 103 ✓

✅ 全部断言通过（含 3 条反向用例 + 1 条平凡值用例）
   结论：`max_gen` 只保证"不无限跑"；`min_fidelity` 保证"跑到还有用为止"。
         两者是**不同性质的约束**：前者是形态，后者是质量。只有前者时，
         系统会产出"代数合规但已经失真"的记忆 —— 这正是 G7 的真实危害。
```

## semantic_drift_audit.rs（语义漂移审计 · 适应度函数）

```
═══ 语义漂移审计 · 架构适应度函数 ═══
（四个维度：D1 值域滑移 / D2 语义边界漂移 / D3 平凡值滥用 / D4 表—码失配）

########## 第一步：负样本（必须全部报警）##########

── D1 · 值域滑移（负样本） ──
  ✗ [D1 值域滑移] 场景「漂移样本-值域滑移」用了 `projection.param=consolidate:min_fidelity=0.8`，但权威参数表没有这个取值。
    Because：参数表是『系统存在什么』的唯一清单。表外的取值 = 存在一个没登记的机制——
    它可能是合理的（那就该走 ADR 补进表），但绝不能**悄悄出现**。
  ✗ [D1 值域滑移] 场景「漂移样本-值域滑移」用了 `projection=semantic_dedup`，但权威参数表没有这个取值。
    Because：参数表是『系统存在什么』的唯一清单。表外的取值 = 存在一个没登记的机制——
    它可能是合理的（那就该走 ADR 补进表），但绝不能**悄悄出现**。

── D1 · 新机制键（负样本） ──
  ✗ [D1 新机制键] 场景「漂移样本-新机制键」用了参数表里没有的键 `memory.compress_algo`。
    Because：新机制键意味着系统多了一个『构成者』。J1 要求先证明它只是取值、不是机制。

── D2 · 语义边界漂移（负样本） ──
  ✗ [D2 语义边界漂移] `projection.param=consolidate:max_gen` 在不同场景里代表了不同东西：[("场景 S06", "最多巩固几代"), ("场景 S10", "记忆还剩多少可用信息")]
    Because：这是审计盲区的核心形态——**取值没变、值域也没变**，
    但『这个取值意味着什么』已经滑走了。G7 正是这么长出来的：
    `consolidate:max_gen` 一直在表里，值域也没扩展，
    可它的语义从『最多巩固几代』滑成了『记忆还剩多少可用信息』。

── D3 · 平凡值滥用（负样本） ──
  ✗ [D3 平凡值滥用] `projection=eval` 出现占比 100% > 阈值 90%。
    Because：J2 要求每个参数有平凡值（不启用该机制时系统仍完整运行）。
    但若平凡值成了**最常用档位**，说明这个机制在名义上启用、实际上没起作用——
    这是另一种静默失效。

  → 四个检查器全部会响（不是摆设）✓

########## 第二步：正样本（必须零报警）##########

── D1 · 值域（正样本） ──
  无漂移 ✓

── D2 · 语义（正样本） ──
  无漂移 ✓

── D3 · 平凡值（正样本，按参数配置阈值） ──
  无漂移 ✓

── D4 · 表—码（正样本） ──
  无漂移 ✓

  → 正样本零报警（无假阳性）✓

########## 第三步：D4 负样本（表—码失配）##########

── D4 · 表—码失配（负样本） ──
  ✗ [D4 表—码失配] 代码里独有的键：[]；表里独有的键：["legacy_bus"]
    Because：参数表称『唯一一份』。若代码用了表外的键、或表里的键从未被用，
    两者之一必然过期——而『唯一一份』这句话就不再成立。

########## 第四步：CI 闸门契约 ##########
  合成漂移样本共产生 6 条 Finding，全部**阻断发布**
  接入位置：`04 §2 CI 清单` 的 N7 之后，作为常驻适配度闸门。
  运行频率：每个 PR（D1/D4 静态可查）+ 每日（D2/D3 需事件采样）。

✅ 全部断言通过（4 个检查器 × 各自的负样本 + 正样本无假阳性 + D4 负样本）
   结论：这个审计器补上的，是 `mechanism_growth.rs` / `scenario_ladder.rs` 的**盲区**。
         那两个审计器问的是「有没有发明新机制」；
         本审计器问的是「**已有机制的语义有没有滑走**」。
         G7（巩固代数发散）正是后者——两个旧审计器都报合规，因为语义不在它们的度量里。
         这就是把'架构期望'编码成'可执行检查'的意义：
         凡是靠人复核对不出来的东西，必须有一个程序每天替你问一遍。
```

## conation_minimal.rs（「欲」的最小可执行形态）

```
═══ 「欲」维度的最小可执行形态 ═══
（E1 欲是事实 / E2 欲行分离可编译强制 / E3 欲有平凡值）

── E1：欲 = 一条事件 ──
  日志 = ["conation.expressed", "system.triggered"]
  投影得到未完成意图 = ["推进长期目标"]
  → 「欲」用的是**唯一原语 Store**，没有新增机制 ✓

── E2：欲 → 行的类型闸门 ──
  候选意图 approved = false（造出来一定是 false，不能伪造）
  过闸后：approved = true，产出任务 = ApprovedTask { from_seq: 1, goal: "推进长期目标" }
  过宽目标 → Err("goal too broad")
  无溯源的欲 → 能否构造候选意图：false

── E3：欲的平凡值 ──
  平凡值 enabled = false
  关闭欲后 approve → Err("conation disabled")
  → 关闭『欲』= 一键退回纯响应式，而系统其余部分完好 ✓

── 零运行时开销 ──
  size_of::<GateWarrant>() = 0

── 准入判定复核（02 §4 四问）──
  「价值偏好」→ projection.param=conation:pref（取值）
  「内在动机」→ projection=conation（新取值）
  「好奇与探索」→ event.entity=conation（同一取值）
  「目标自生成」→ event.entity=conation（新取值）
  → 4 项「欲」能力全部落在**已有机制键的新取值**上，新增机制键 = 0 ✓

✅ 全部断言通过
   结论：「欲」不需要新原语、新模块、新 trait。它需要的是三样东西：
         ① 一条事件（`conation.expressed`）—— 让'想要'成为可审计的事实；
         ② 一道类型闸门（`IntentGate` + `GateWarrant`）—— 让'想要'不能直接变成'做了'；
         ③ 一个平凡值（`enabled = false`）—— 让能动性可被一键关停。
         这三样合起来，就是『欲』的**最小可执行形态**。
         注意：它解决的是'欲如何被治理'，不是'欲从哪来'——后者是价值对齐问题，
         架构只保证'想要的都留痕、都必须过闸、都可关停'。
```
