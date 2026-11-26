---
kind: release
id: RV-000-发布版本汇总
title: 发布版本总览
status: active
projection: true
lists: [release-entries=owner]
created: 2026-11-16
prefix: RV
label: 发布版本
desc: 发布说明——一篇一个已发布版本；active 条目即需要数据兼容的版本集合
summary.order: [active, draft, retired]
summary.active.title: 已发布版本（需要数据兼容）
summary.active.columns: [idLink, title]
summary.draft.title: 未发布版本（不要求兼容）
summary.draft.columns: [idLink, title]
summary.retired.title: 已停止兼容的版本
summary.retired.columns: [idLink, title, reason]
---

# 发布版本总览

> **文档类型：发布版本说明汇总** — 一篇文档记录一个发布版本。已发布版本列表由登记机制自动生成，本页不手抄版本表。登记与汇总规则见 [DE-017-登记与汇总机制](../003-design/017-登记与汇总机制.md)。

## 版本身份

- `id` 用 `RV-<三位序号>-<语义化版本>`，文件名 = 去前缀主干：`RV-001-0.1.0` 住 `001-0.1.0.md`。
- `title` 逐字等于正文首个 H1；正文用「发布内容 / 数据形状变更 / 兼容与迁移说明 / 验证证据」四节写清这一版。
- 目录不参与判定，`kind: release` 只来自 front-matter；汇总页自己带 `projection: true`，不计入版本条目数。

## 兼容口径

- `status: active`：已发布，且纳入数据兼容集合。
- `status: draft`：尚未发布，不要求兼容。
- `status: retired`：已停止兼容，`reason` 写明停止兼容的理由。
- 下方自动汇总区的 `active` 组既是**已发布版本列表**，也是**需要兼容的版本号清单**。数据形状改动只需兼容这张清单里的版本；不在清单中的版本数据修改不用做数据兼容。

当前清单为空：项目尚未发布任何版本，也没有需要兼容的版本。

## 自动清单

<!-- registry:summary begin -->
<!-- list:release-entries owner -->

> ⚠️ **本区块由 `node scripts/audits/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](../005-spec/001.md)，机制见 [DE-017 · 登记与汇总机制](../003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")。

`release`（发布版本）的条目**在位存放**：文件本身就是登记，`title` 逐字等于正文首个 H1（RG-021 两面对账）。

**计数**：active 0 · draft 0 · retired 0（共 0）

## 已发布版本（需要数据兼容）

（无）

## 未发布版本（不要求兼容）

（无）

## 已停止兼容的版本

（无）

<!-- registry:summary end -->
