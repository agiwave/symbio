---
kind: task
id: T-261-await行语义裁决
title: await-model / await-tools 从未被问过——是 Wait 折叠的声明位，还是行表里的装饰行
status: todo
created: 2026-10-23
track: plan
prio: next
deps: []
parent: T-255-义务环收口批
where: docs/003-design/015-义务环.md · symbio/src/plugins/loop/loop_rows.rs · symbio/src/symbio_core/decl/ring.rs
exit: '一条 `### D-NNN` 写明 `by: None` 且从未被任何 `owes()` 直调点问过的行（今天：`await-model` / `await-tools`）属于哪一类，并给出**机器可判**的依据：要么「Wait 折叠的声明位，合法，判据允许存在但要求 `until` 与对应 `dispatch-*` 的出口同名」，要么「装饰行，应删」，并落成一条装载期检查或审计判据。'
---

## 内容

**事实**：`await-model` / `await-tools` 两行 `by: None`，且全仓**没有任何** `owes()`
直调点问过它们（13 处直调点逐条核对，见 T-260）。

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
