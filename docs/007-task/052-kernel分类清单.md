---
kind: task
id: T-052-kernel分类清单
title: kernel 分类清单：语法 / 冻结求解器 / 可替换实现（先清单，后搬迁）
status: todo
track: arch
prio: later
created: 2026-11-08
deps: []
where: "`crates/symbio-kernel/src`（`store/mod.rs` 144-225 行的 `MemoryStore`、`capability/mod.rs` 293-302 行的 `capability_invoke`、`exec/mod.rs`、`present/detail.rs` / `present/reading.rs`）；`crates/symbio-sdk/src`"
exit: 先出一张逐块分类清单（每块：符号 / 现址 / 类（语法 / 冻结求解器 / 可替换实现 / 契约 / 机制）/ 处置（留守 / 下沉 sdk）/ 理由），不改代码；清单经批准后才搬迁；判据是「任何能力都用同一份、且换实现就破坏语义」——能过可替换性检验的下沉 sdk，不能的把理由写死在清单里；`MemoryStore` / `capability_invoke` / `exec` / `present` 四块逐块有结论；环求解器不因「判据好看」搬走；搬迁后 `gate --full` 全绿
---

# kernel 分类清单

> 对应架构分析：**P2（裁决类）**。
> 架构批第 3 笔之一。裁决（用户既定，不再开讨论）：
> **kernel 承认含冻结求解器，但判据必须可执行。**
> 依据：P2 核实——kernel 生产面 43 文件 / 9000 行 / 256 个 `pub fn`，
> 而 `crates/symbio-sdk/src/lib.rs` 19 行仍写「没有一行实现」。

## 内容

一条实现留在 kernel，必须同时满足两件事：

1. **任何能力都用同一份**：它不是某个插件 / 某条能力的私有算法，而是所有能力共享的
   唯一求解过程；
2. **换实现就破坏语义**：把它换成第二种实现，系统语义立刻改变（不是「变慢」或
   「不方便」，而是不变量 / 闭集 / 配对规则被打破）。

通过可替换性检验（换成另一份实现对语义无影响）的实现下沉 `symbio-sdk`；不能通过的把
理由写死在清单里，不许用「它是内核所以留内核」当理由。

待审四块：

| 块 | 现址 | 必须回答 |
|---|---|---|
| `MemoryStore`（锁 / 去重 / seq） | `crates/symbio-kernel/src/store/mod.rs` 144-225 行 | 平凡基线还是语义强制？`RwLock` 串行化与 seq 无跳号是否构成「换实现就破坏语义」 |
| `capability_invoke` | `crates/symbio-kernel/src/capability/mod.rs` 293-302 行 | 「唯一拆信封点」是契约还是可替换便利 |
| `exec` | `crates/symbio-kernel/src/exec/mod.rs` | `ExecEnv::from_request` / 出口绑定是语法还是机制 |
| `present` / `detail` | `crates/symbio-kernel/src/present/detail.rs`、`present/reading.rs` | 呈现方言的类型面与默认实现各归哪类 |

## 是否阻塞横向扩展

**是（裁决类）**。kernel 归类判据失真时，新能力无法从判据判断该住哪一层；
四块（`MemoryStore` / `capability_invoke` / `exec` / `present`）必须在清单里各有结论，
先清单后搬迁，不许留与代码相反的判据。

## 处理过程

1. 逐块填上表：符号 / 现址 / 类 / 处置 / 理由，**先只出清单，不动代码**。
2. 清单交评审；批准后按「能过可替换性检验的下沉 sdk」逐块搬迁，每块一笔提交。
3. `decl/ring.rs` 一类冻结求解器若留守，在清单里写出它满足上面两条中的哪一条。
4. 搬迁批跑 `gate --full`，全绿后提交。
