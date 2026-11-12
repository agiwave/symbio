---
kind: task
id: T-373-内核Decl_tier移除
title: 内核 Decl.tier 移除——一个零读取的冻结面字段，28 处构造点只是噪声
status: todo
track: arch
prio: now
created: 2026-11-12
parent: T-372-残余转任务SPRINT
where: crates/symbio-kernel/src/decl/ring.rs、crates/symbio-kernel/src/decl/mod.rs、各插件的 decl*.rs
exit: `Decl.tier` 不存在；`LatencyTier` 仍被 `ActorDecl.mode` 使用；CS-009 structFields 616 → 615 且基线同步；`gate --full --ci` 绿
---

# 内核 Decl.tier 移除——一个零读取的冻结面字段，28 处构造点只是噪声

## 内容

### 现场

[DE-048-端口机制裁决](../003-design/048-端口机制裁决.md) 的 `D-315` 末段是这么写的：

> **未收口（如实记）。** 本条其余裁决已随 P2 批落地…**只剩 `Decl.tier`**（每行都设，
> `D-413` 搬行表时原样带走）；它不移除之前本条不称「全部兑现」。

实测这个字段的状态：

| 事实 | 读数 |
|---|---|
| 定义 | `crates/symbio-kernel/src/decl/ring.rs:125` —— `pub tier: LatencyTier,`，文档注释只有一句「这一行动作允许的时延档」 |
| 内核内的读取 | **0 处**（`grep -n tier ring.rs` 只命中定义那一行；`owed` / `keys_of` / `may_open_debts` / 依赖图都不读它） |
| 外部的读取 | **0 处**（全仓 `.tier` 的其它命中都是报表行 `TierRow.tier` 与 `slo` 的 `by_tier` 局部变量，与 `Decl` 无关） |
| 构造点 | **28 处**（11 处生产 + 17 处测试），全在赋值 `LatencyTier::Reflex / Fast / Deep` |
| 占的判据格 | `arch-shape-audit` 的 CS-009 `structFields`（现 616 / 基线 617） |

也就是说：它是**冻结面上唯一一个零消费者的字段**。每加一行 `Decl` 就要多写一行
`tier: LatencyTier::Reflex`，而这一行对任何判决都不产生影响——`D-306` 之后「谁执行、能不能
执行、什么时候欠」全由 `owes` / `until` / `unless` / `reads` 派生，没有一处看时延档。

### 判定

**删字段，不删类型。** 三条依据：

1. **同一条事实已经由 `ActorDecl.mode` 声明过。** 时延档是同一条执行体的属性（`mode:
   LatencyTier`），`Decl` 再声明一遍是「同一件事写两处」——两处迟早不一致，而不一致时没有
   任何判据会红（这正是 `D-251` / `D-455` 的坏形状：重复声明本身不是错，**不一致且无人判**
   才是）。
2. **`DE-008` §2.3 已经把时延归到 `actor.budget_ms`**：「分层回答的是『我多快能回应』，
   这是表达问题，不是结构问题——它是 `actor.budget_ms` 的取值」。行表不是它的 owner。
3. **删字段降 `structFields`，方向对。** CS-009 是「只许降」，冻结面清零是它的目标；
   `D-315` 也把「移除 `tier`」写成兑现条件，不是新决定。

`LatencyTier` 类型本身保留：`ActorDecl.mode`（`crates/symbio-kernel/src/decl/mod.rs:201`）
仍在用，`budget_ms()` / `as_str()` / `parse` 三个方法也仍有消费方。

### 施工点

1. `ring.rs`：删字段与它的文档注释；`use crate::LatencyTier` 若变空则删。
2. 11 处生产构造：`classify` / `conation` / `glossary` / `loop::vocab` / `memory::episodic`（3 处）/
   `session::declarations`（2 处）/ `skill` / `tool::declarations` —— 逐处删 `tier:` 行，
   并清掉各文件因此变成未使用的 `LatencyTier` 导入。
3. 17 处测试构造：同上（测试夹具里的 `tier` 也是纯噪声）。
4. `scripts/arch-shape-audit.mjs` 的 `CORE_DATA_BASELINE`：`structFields` 616 → 615。
   同一条提示还报 `enumVariants 256/257` 已收到基线下方——**同批一起同步到 256**：
   留着旧数字等于把余量白留给下一次（脚本自己的提示语）。

### 判据

- `grep -rn "pub tier" crates/symbio-kernel/src/decl/` 0 处；
- `grep -rn "tier: LatencyTier::" crates/` 0 处；
- `node scripts/arch-shape-audit.mjs` 的 CS-009 读 `615/615`、`256/256`，且不再打
  「数据面已收到基线下方」的提示；
- `cargo test -p symbio-kernel --lib` 与 `node scripts/gate.mjs --full --ci` 绿。

### 不做什么

- 不删 `LatencyTier`（`ActorDecl.mode` 还在用）；
- 不把时延档改成从 `ActorDecl.mode` 派生进 `Decl` —— 那是把刚删掉的重复声明换个方向再加一次；
- 不为这个字段写兼容层 / 弃用期：零消费者，没有兼容对象。

## 处理过程

- 2026-11-12：登记。判定依据是「零读取」这条实测事实 + `D-315` 的兑现条件；不新立决策，
  裁决已有的，本条只做施工与记录收口。
