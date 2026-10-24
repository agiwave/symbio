---
kind: task
id: T-261-await行语义裁决
title: await-model / await-tools 从未被问过——是 Wait 折叠的声明位，还是行表里的装饰行
status: done
created: 2026-10-23
track: plan
prio: next
deps: []
parent: T-255-义务环收口批
where: docs/003-design/015-义务环.md · symbio/src/plugins/loop/loop_rows.rs · symbio/src/plugins/loop/turn_exec.rs
exit: '一条 `### D-NNN` 写明 `by: None` 且从未被任何 `owes()` 直调点问过的行（今天：`await-model` / `await-tools`）属于哪一类，并给出**机器可判**的依据：要么「Wait 折叠的声明位，合法，判据允许存在但要求 `until` 与对应 `dispatch-*` 的出口同名」，要么「装饰行，应删」，并落成一条装载期检查或审计判据。'
evidence: 'DE-015 新增 `### D-130`：裁决为**声明位、保留**，但它的成立要求两半——① 形状对偶，用例 `every_await_row_is_the_dual_of_its_dispatch_row`（`by: None` / 开账条件 = `dispatch-*` 的 `sent` 出口 / 出口是往返终态且同端口 / 反向「每条 `dispatch-*` 都有 `await-*`」）；② 一个生产读者，新增 `loop_rows::waiting` 并把 `drive_steps` 的 `None` 分支从一律 `no_progress` 拆成 `waiting`（有已派出未收清的往返）与 `no_progress`（真的没事）。`waiting` 的三种形状由用例 `waiting_means_a_sent_request_without_a_terminal` 钉住（含反例：只落 `request` 不算、`lease` 到期仍在等、终态落了就不在等）。`cargo test -p symbio --lib` 2110 passed。'
---

## 内容

**事实**：`await-model` / `await-tools` 两行 `by: None`，且全仓**没有任何** `owes()`
直调点问过它们（12 处直调点逐条核对，见 T-260）。

按 D-114「Wait 与 Ask 折叠」，`by: None` 就是「这笔账不由引擎执行」的声明位，
**本身合法**。但一条**从没被问过**的行，与一条「问过但不派发」的行，
在审计上不可区分——这正是行表长出装饰行的入口，而 DE-015 §3.4 第 7 条
（`OrphanFact`）只防「事实名没人管」，不防「行没人问」。

**两个候选**（裁决时挑一个，理由写进 ADR）：

1. **承认它是声明位，并给它一条可判的形状**：`by: None` 的行必须满足
   「`owes` 的来源 = 某条 `dispatch-*` 的 `before`（即 `sent`），且 `until` 与那条
   `dispatch-*` 的出口同名」。这样它不是装饰行，而是「派发行的对偶」，可机械核对。
2. **判它冗余**：`dispatch-<port>` 的 `until` 已经包含 `sent`，`await-<port>` 提供的
   信息（`sent` 之后等应答）在债公式里是否真的独立成立？若不能独立成立，删掉。

**为什么不能就这么留着**：GO-002 T1 说「架构足够简洁」。行表是控制面的全部，
多一条没有消费者的行，读者就得为它建立一套解释。要么它有对偶关系（可判），
要么它不该在。

## 处理过程

- 2026-10-23：登记。发现于 CLI 实测后的行表核对（`02-验证清单.md` 的 E3/P3）。
- 2026-10-23：裁决 = **候选 1，但加了候选 1 原本没有的第二半**。原候选只要求
  「形状可判」，而形状可判只能证明它不是**乱写的**，证明不了它不是**装饰的**——
  一条形状完全正确、却没有任何读者的行，仍然只是行表里的注脚。所以判据做成两半：
  形状对偶（用例）+ 生产读者（`waiting` 停机理由）。
- 2026-10-23：为什么形状判据是**用例**而不是装载期检查——夹具刻意只写一行
  （`ring.test.rs` 的 `await-x` 用例正是在验单行形状），装载期强制对偶会把「最小夹具」
  判成非法，而 `ROWS` 是 `&'static` 常量，对它跑用例与装载期判**等价**。
- 2026-10-23：`waiting` **按形状认、不按行名认**（开账条件 = `sent`），因为派发器不认
  行名（D-118 决策 1）；按 `await-` 前缀匹配会让判据依赖命名约定，改名即静默失效。
- 2026-10-23：`drive_steps` 走到 `waiting` 分支需要**真实中断**（进程在等应答时被杀），
  in-process 造不出来 ⇒ 这条接线的实测归 T-262 第 ④ 项（只有 `sent` 的日志重启后判死）。
