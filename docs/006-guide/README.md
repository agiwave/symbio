---
kind: guide
id: GU-000-指南汇总
title: 指南总览
status: active
projection: true
lists: [guide-entries=owner]
created: 2026-10-14
prefix: GU
label: 指南
desc: 怎么用——上手 / 贡献 / 排障
---

# 指南总览

指南回答「怎么用」：上手、贡献、排障。它描述操作步骤，不承载结构事实——事实与机制各归各的 owner，指南只指路。

<!-- registry:summary begin -->
<!-- list:guide-entries owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](../005-spec/001.md)，机制见 [DE-017 · 登记与汇总机制](../003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")。

`guide`（指南）的条目**在位存放**：文件本身就是登记，`title` 逐字等于正文首个 H1（RG-021 两面对账）。

**计数**：active 15 · draft 0 · retired 0（共 15）

## 现行文档（active）

| 编号 | 标题 |
|---|---|
| [`.github/workflows/README`](../../.github/workflows/README.md) | CI/CD 配置说明 |
| [`CODE_OF_CONDUCT`](../../CODE_OF_CONDUCT.md) | Contributor Covenant Code of Conduct |
| [`CONTRIBUTING`](../../CONTRIBUTING.md) | Contributing to Symbio |
| [`GU-001-QUICKSTART_CN`](../../.github/workflows/001-QUICKSTART_CN.md) | 🚀 自动构建指南 |
| [`GU-002-SECRETS_SETUP`](../../.github/workflows/002-SECRETS_SETUP.md) | GitHub Actions 密钥配置指南 |
| [`GU-003-building`](../../cli/docs/003-building.md) | 构建 |
| [`GU-004-usage`](../../cli/docs/004-usage.md) | 用法 |
| [`GU-005-PLUGIN_DEVELOPMENT`](005-PLUGIN_DEVELOPMENT.md) | Symbio 插件开发指南 |
| [`GU-006-QUICK_START`](006-QUICK_START.md) | Symbio 快速上手 |
| [`GU-007-TROUBLESHOOTING`](007-TROUBLESHOOTING.md) | Symbio 故障排查指南 |
| [`README`](../../README.md) | Symbio |
| [`cli/README`](../../cli/README.md) | symbio-cli |
| [`cli/docs/README`](../../cli/docs/README.md) | symbio-cli 文档中心 |
| [`docs/README`](../README.md) | Symbio 文档中心 |
| [`e2e/README`](../../e2e/README.md) | e2e 测试环境（CLI 模式） |

## 草案（draft）

（无）

## 废止（retired）

（无）

<!-- registry:summary end -->

