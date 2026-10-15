---
kind: ref
id: RE-000-参考汇总
title: 参考总览
status: active
projection: true
lists: [ref-entries=owner]
created: 2026-10-14
prefix: RE
label: 参考
desc: 查数据——路由 / 错误码 / 配置等权威表
---

# 参考总览

参考是权威表的家：路由、错误码、配置。能由代码投影生成的（如 GATE_CODES、RE-006）不手改，手写的以本目录为准。

<!-- registry:summary begin -->
<!-- list:ref-entries owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](../008-spec/001.md)，机制见 [PL-011 · 登记与汇总机制](../004-plan/011-登记与汇总机制.md "PL-011-登记与汇总机制")。

`ref`（参考）的条目**在位存放**：文件本身就是登记，`title` 逐字等于正文首个 H1（RG-021 两面对账）。

**计数**：active 7 · draft 0 · retired 0（共 7）

## 现行文档（active）
| 编号 | 标题 |
|---|---|
| [`RE-001-CONFIGURATION`](001-CONFIGURATION.md) | Symbio 配置参考 |
| [`RE-002-ERROR_CODES`](002-ERROR_CODES.md) | Symbio 错误码参考 |
| [`RE-003-ROUTES`](003-ROUTES.md) | Symbio 路由参考 |
| [`RE-004-slo-calibration`](004-slo-calibration.md) | SLO 校准 · 首测（2026-09-30） |
| [`RE-005-e2e覆盖矩阵`](005-e2e覆盖矩阵.md) | e2e 覆盖矩阵 · 能力域 × 场景形态 |
| [`RE-006-当前事实表`](006-当前事实表.md) | Symbio 当前事实表（自动生成） |
| [`RE-007-清单归属总账`](007-清单归属总账.md) | Symbio 清单归属总账（自动生成） |

## 草案（draft）
（无）

## 废止（retired）
（无）

<!-- registry:summary end -->

