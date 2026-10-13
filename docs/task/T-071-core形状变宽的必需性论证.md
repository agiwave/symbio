---
kind: task
id: T-071-core形状变宽的必需性论证
title: core 形状棘轮亮红（fn 73>72 / defaults 25>24）：逐格补「必需」论证 ADR 或回退
status: todo
created: 2026-10-13
track: quality
deps: []
prio: now
exit: 两格各有一处归宿：要么写一条 ADR 逐条论证「这次变宽必需」（并在允许账本旁写 core-shape-allow <格>: ADR-NNN <理由>，判据见 [ADR-055](../adr/ADR-055.md) 与 scripts/core-shape-audit.mjs 的 CS-001/CS-002），要么回退变宽的成员。验证：node scripts/core-shape-audit.mjs 零违规；--verbose 输出与 ADR 论证逐格对上
---

## 内容

`node scripts/core-shape-audit.mjs` 现行两格亮红（实测 fn 成员总数 73 > 基线 72、
带默认体的成员数 25 > 基线 24）——**symbio_core 的公共形状只许收窄**（[ADR-055](../adr/ADR-055.md)），
变宽必须挂得住一条逐格论证的 ADR（`08 §0` 纪律 3，登记条目 [SP-007](../spec/SP-007.md)）。

背景事实（biǎn 还须逐格确认）：变宽发生在 megabatch T-066 相关提交
（`0569f411` 一族）之后的树上，与本批 e2e / 文档工作无关（已在 `67bb143c` 干净
worktree 复现同样两格红）。处置只能二选一，不许动人：

1. **论证必需**：逐成员说明为什么必须出现在 core 公共面 + 为什么收窄不了；
   ADR 旁挂 `core-shape-allow fn: ADR-NNN <理由>` / `core-shape-allow defaults: ADR-NNN <理由>`;
2. **回退**：把这两格变宽的成员收窄回去（下沉到唯一依赖方模块）。

## 处理过程

- 2026-10-13 登记：跑 docs,facts 门禁时发现两格红，经干净 worktree 比对确认为
  存量欠账（非本批引入），按「形状债务必须有归宿」登记。
