---
kind: spec
id: WVR-01
title: C-003 根出口单消费方豁免账本（6 条）
status: active
created: 2026-10-11
scope: scripts/core-export-audit.mjs 的 WAIVERS 账本（ActorSpec / EVENT_SESSION_RECONCILED / turnstate / SloLatencyView / FallbackRateView / mutations_latest_snapshot）
clears: [W1]
---

## 规范

**形态：真·单消费方根导出豁免**（README §4 四问的「接缝」档）：core 内没有第二个生产模块在用，但也下沉不了（C-002 禁插件深引 core 域目录 / 类型行为无处安放）。

本条只记「该豁免存在」与「谁来清」两件事；逐条豁免的**理由**不在这里（owner = 账本注释）。清除任务：[`W1`](../task/W1.md)（已闭环——修的是账本口径，18 条缩到 6 条；余下 6 条是真豁免，形态见上）。

## 判据

`scripts/core-export-audit.mjs` 的 C-003 棘轮（单消费方根导出按格计数）；`WAIVERS` 账本长度即豁免存量，缩编与否由 `W1` 的出口判据核对。
