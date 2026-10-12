---
kind: spec
id: WVR-03
title: dead-code R-002 契约登记存量（8 处）
status: active
created: 2026-10-11
scope: scripts/dead-code-audit.mjs 的 R-002 死码承认账本（终局基线 8，见 docs/plan/04-工程落地.md §3.1）
clears: [W2]
---

## 规范

**形态：契约登记（非待清）**——协议契约键 / 词表常量先于接线，消费方是 `PROTOCOLS.md` 字段表与前端镜像（Rust 侧无调用点）。

本条只记「该豁免存在」与「谁来清」；逐条理由归账本注释。清除任务：[`W2`](../task/W2.md)（已闭环——复核后本条保留为契约登记形态）。

## 判据

`scripts/dead-code-audit.mjs` 的 R-002 棘轮：存量不得超终局基线 8，超了即红。
