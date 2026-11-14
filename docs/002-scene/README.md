---
kind: scene
id: SC-000-场景汇总
title: 场景总览
status: active
projection: true
lists: [scene-entries=owner]
created: 2026-10-14
prefix: SC
label: 场景
desc: 用户拿它做什么——角色、情境与用户判据
---

# 场景总览

场景是产品层：用户拿它做什么。用户判据与边界的唯一账本在这里（目标与设计只留指针）——补判据、销缺口、改边界只改场景文档。

<!-- registry:summary begin -->
<!-- list:scene-entries owner -->

> ⚠️ **本区块由 `node scripts/audits/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](../005-spec/001.md)，机制见 [DE-017 · 登记与汇总机制](../003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")。

`scene`（场景）的条目**在位存放**：文件本身就是登记，`title` 逐字等于正文首个 H1（RG-021 两面对账）。

**计数**：active 1 · draft 1 · retired 0（共 2）

## 现行文档（active）

| 编号 | 标题 |
|---|---|
| [`SC-001-场景总册`](001-场景总册.md) | 场景总册 |

## 草案（draft）

| 编号 | 标题 |
|---|---|
| [`SC-014-未承诺场景草案`](014-未承诺场景草案.md) | 未承诺场景草案 |

## 废止（retired）

（无）

<!-- registry:summary end -->

