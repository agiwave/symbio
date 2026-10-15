---
kind: task
id: T-071-core形状变宽的必需性论证
title: core 形状棘轮亮红（fn 73>72 / defaults 25>24）：逐格补「必需」论证 ADR 或回退
status: done
created: 2026-10-13
track: quality
deps: []
prio: now
exit: 两格各有一处归宿：要么写一条 ADR 逐条论证「这次变宽必需」（并在允许账本旁写 core-shape-allow <格>: ADR-NNN <理由>，判据见 [ADR-055](../010-adr/055.md) 与 scripts/core-shape-audit.mjs 的 CS-001/CS-002），要么回退变宽的成员。验证：node scripts/core-shape-audit.mjs 零违规；--verbose 输出与 ADR 论证逐格对上
evidence: 处置取「论证必需」（变宽成员收窄不可行：`EventEnvelope::fallback_timestamp` 由泛型唯一写入口 `WalStore::append` 调用，全仓仅 `Event` 一个实现，下沉即 ADR-062 明确否决的方案）。① 逐格论证落 [ADR-062](../010-adr/062.md) 新增「形状论证（core 形状棘轮 CS-002）」节——`methods` +1：写入口对事件类型泛型（`Store::Event: EventEnvelope`），泛型触达信封自身 ts 的唯一通道是 trait 成员（与 `assign_seq` 同纪律）；`defaults` +1：默认 no-op 体是无时刻概念信封的平凡值（README §3 J2），去掉反让每个实现方手写同一空函数、公共面更宽。② `scripts/core-shape-audit.mjs` 基线 `methods 72→73` / `defaults 24→25`，旁挂两条 `core-shape-allow …: ADR-062 <理由>`，文件头冻结值同步 73/25。验证：core-shape-audit 零违规（CS-002 报「放宽 2 格，引用的 ADR 全部能打开」）· `--verbose` 逐 trait 显示 `EventEnvelope 4/1`（与论证的成员一一对上）· core-shape-audit.test.mjs 13/13 · `gate --only=baseline` 5/5（两格报「已登记豁免」非红）· registry-audit 零违规 · doc-link-audit 1408 链接 0 失效
---

## 内容

`node scripts/core-shape-audit.mjs` 现行两格亮红（实测 fn 成员总数 73 > 基线 72、
带默认体的成员数 25 > 基线 24）——**symbio_core 的公共形状只许收窄**（[ADR-055](../010-adr/055.md)），
变宽必须挂得住一条逐格论证的 ADR（`08 §0` 纪律 3，登记条目 [SP-007](../008-spec/007.md)）。

背景事实（还须逐格确认）：变宽发生在 megabatch T-066 相关提交
（`0569f411` 一族）之后的树上，与本批 e2e / 文档工作无关（已在 `67bb143c` 干净
worktree 复现同样两格红）。处置只能二选一，不许动人：

1. **论证必需**：逐成员说明为什么必须出现在 core 公共面 + 为什么收窄不了；
   ADR 旁挂 `core-shape-allow fn: ADR-NNN <理由>` / `core-shape-allow defaults: ADR-NNN <理由>`;
2. **回退**：把这两格变宽的成员收窄回去（下沉到唯一依赖方模块）。

## 处理过程

- 2026-10-13 登记：跑 docs,facts 门禁时发现两格红，经干净 worktree 比对确认为
  存量欠账（非本批引入），按「形状债务必须有归宿」登记。
- 2026-10-13 清偿（处置取选项 1）：以基线提交 `b5dae269` 的 worktree 与当前树逐 trait 对账，
  定位变宽成员 = `EventEnvelope` `3/0 → 4/1`（`symbio/src/symbio_core/event/mod.rs`），
  来源提交 `a4e5f7de`（ADR-062 加 `fallback_timestamp`）。核实「收窄不可行」的三条结构事实：
  该成员在 `store/wal.rs:467` 由泛型写入口 `WalStore::append` 调用；`Store` 的事件类型是
  关联类型 `type Event: EventEnvelope`（写入口看不见字段）；全仓仅 `Event` 一个实现。
  ⇒ 逐格论证写入 [ADR-062](../010-adr/062.md)「形状论证」节 + 脚本基线旁挂两条
  `core-shape-allow … ADR-062`，并同步文件头冻结值。门禁全绿（见 front-matter `evidence`）。
