---
kind: ref
id: RE-008-判据码登记表
title: 判据码登记表（自动生成）
status: active
created: 2026-09-20
lists: [gate-codes=owner]
---

# 判据码登记表（自动生成）

> ⚠️ **本表由 `scripts/facts/gate-codes.mjs` 从各脚本的 `// @ns` / `// @codes` 声明生成，请勿手改。**
> 改了声明不用手动重跑——门禁会**自动重新生成并暂存**（`scripts/gen-facts.mjs`，见 `scripts/gate.d/60-facts.mjs`）。
>
> 判据码是每条机械判定的**地址**：豁免注释、门禁日志、文档指认都指向它。
> 每条判据**判什么**归声明它的那个脚本的头注释；撞号与漏登记由
> [`scripts/gate-codes-audit.mjs`](../../scripts/gate-codes-audit.mjs) 判（GC-001–006）。

<!-- list:gate-codes owner -->

## 1. 命名空间

| 前缀 | 归属 | 声明脚本 | 已用号段 | 下一个可用号 |
|---|---|---|---|---|
| **C** | core 出口 | `scripts/core-export-audit.mjs` | C-001–003 | `C-004` |
| **CS** | core 形状 | `scripts/core-shape-audit.mjs` | CS-001–002 | `CS-003` |
| **D** | 文档正文与链接 | `scripts/doc-link-audit.mjs` · `scripts/doc-symbol-audit.mjs` | D-001–011 | `D-012` |
| **DC** | 文档计数对账（文档里的少数几个数 vs 代码） | `scripts/doc-count-audit.mjs` | DC-001–004 | `DC-005` |
| **E** | 插件入口与门面 | `scripts/plugin-entry-audit.mjs` | E-001–009、E-011–012 | `E-013` |
| **GC** | 判据码命名空间本身 | `scripts/gate-codes-audit.mjs` | GC-001–006 | `GC-007` |
| **GW** | 门禁接线（测试文件与门禁名单的对账） | `scripts/gate-wiring-audit.mjs` | GW-001–005 | `GW-006` |
| **LWF** | 列表写入口事实 | `scripts/list-write-fact-audit.mjs` | LWF-001–002 | `LWF-003` |
| **M** | 机制表 | `scripts/mechanism-audit.mjs` | M-001–008 | `M-009` |
| **N** | core 命名 | `scripts/core-naming-audit.mjs` | N-001–005 | `N-006` |
| **NDC** | 主体直连 | `scripts/no-direct-call-audit.mjs` | NDC-001–002 | `NDC-003` |
| **PN** | 健壮性登记（每一处 panic 都是显式事实） | `scripts/panic-audit.mjs` | PN-001–003 | `PN-004` |
| **PS** | prompt 形状基线 | `scripts/prompt-shape-audit.mjs` | PS-001–003 | `PS-004` |
| **R** | 死代码 | `scripts/dead-code-audit.mjs` | R-001–002 | `R-003` |
| **RF** | 文档引用（编号 → 地址由机制生成） | `scripts/gen-doc-refs.mjs` | RF-001–004 | `RF-005` |
| **RG** | 登记条目校验（registry 单文档的字段 / 状态 / 依赖 / 判据指认 / 分解） | `scripts/registry-lib.mjs` | RG-001–029 | `RG-030` |
| **S** | 源码 grep 形态 | `scripts/grep-audit.mjs` | S-001–003、S-006–012 | `S-013` |
| **SX** | SLO 出口表 vs session/stats 读数列（用户可见出口） | `scripts/slo-exit-audit.mjs` | SX-001–003 | `SX-004` |

## 2. 逐码归属

| 判据码 | 声明脚本 |
|---|---|
| **C-001** | `scripts/core-export-audit.mjs` |
| **C-002** | `scripts/core-export-audit.mjs` |
| **C-003** | `scripts/core-export-audit.mjs` |
| **CS-001** | `scripts/core-shape-audit.mjs` |
| **CS-002** | `scripts/core-shape-audit.mjs` |
| **D-001** | `scripts/doc-link-audit.mjs` |
| **D-002** | `scripts/doc-link-audit.mjs` |
| **D-003** | `scripts/doc-link-audit.mjs` |
| **D-004** | `scripts/doc-link-audit.mjs` |
| **D-005** | `scripts/doc-symbol-audit.mjs` |
| **D-006** | `scripts/doc-link-audit.mjs` |
| **D-007** | `scripts/doc-link-audit.mjs` |
| **D-008** | `scripts/doc-link-audit.mjs` |
| **D-009** | `scripts/doc-symbol-audit.mjs` |
| **D-010** | `scripts/doc-link-audit.mjs` |
| **D-011** | `scripts/doc-symbol-audit.mjs` |
| **DC-001** | `scripts/doc-count-audit.mjs` |
| **DC-002** | `scripts/doc-count-audit.mjs` |
| **DC-003** | `scripts/doc-count-audit.mjs` |
| **DC-004** | `scripts/doc-count-audit.mjs` |
| **E-001** | `scripts/plugin-entry-audit.mjs` |
| **E-002** | `scripts/plugin-entry-audit.mjs` |
| **E-003** | `scripts/plugin-entry-audit.mjs` |
| **E-004** | `scripts/plugin-entry-audit.mjs` |
| **E-005** | `scripts/plugin-entry-audit.mjs` |
| **E-006** | `scripts/plugin-entry-audit.mjs` |
| **E-007** | `scripts/plugin-entry-audit.mjs` |
| **E-008** | `scripts/plugin-entry-audit.mjs` |
| **E-009** | `scripts/plugin-entry-audit.mjs` |
| **E-011** | `scripts/plugin-entry-audit.mjs` |
| **E-012** | `scripts/plugin-entry-audit.mjs` |
| **GC-001** | `scripts/gate-codes-audit.mjs` |
| **GC-002** | `scripts/gate-codes-audit.mjs` |
| **GC-003** | `scripts/gate-codes-audit.mjs` |
| **GC-004** | `scripts/gate-codes-audit.mjs` |
| **GC-005** | `scripts/gate-codes-audit.mjs` |
| **GC-006** | `scripts/gate-codes-audit.mjs` |
| **GW-001** | `scripts/gate-wiring-audit.mjs` |
| **GW-002** | `scripts/gate-wiring-audit.mjs` |
| **GW-003** | `scripts/gate-wiring-audit.mjs` |
| **GW-004** | `scripts/gate-wiring-audit.mjs` |
| **GW-005** | `scripts/gate-wiring-audit.mjs` |
| **LWF-001** | `scripts/list-write-fact-audit.mjs` |
| **LWF-002** | `scripts/list-write-fact-audit.mjs` |
| **M-001** | `scripts/mechanism-audit.mjs` |
| **M-002** | `scripts/mechanism-audit.mjs` |
| **M-003** | `scripts/mechanism-audit.mjs` |
| **M-004** | `scripts/mechanism-audit.mjs` |
| **M-005** | `scripts/mechanism-audit.mjs` |
| **M-006** | `scripts/mechanism-audit.mjs` |
| **M-007** | `scripts/mechanism-audit.mjs` |
| **M-008** | `scripts/mechanism-audit.mjs` |
| **N-001** | `scripts/core-naming-audit.mjs` |
| **N-002** | `scripts/core-naming-audit.mjs` |
| **N-003** | `scripts/core-naming-audit.mjs` |
| **N-004** | `scripts/core-naming-audit.mjs` |
| **N-005** | `scripts/core-naming-audit.mjs` |
| **NDC-001** | `scripts/no-direct-call-audit.mjs` |
| **NDC-002** | `scripts/no-direct-call-audit.mjs` |
| **PN-001** | `scripts/panic-audit.mjs` |
| **PN-002** | `scripts/panic-audit.mjs` |
| **PN-003** | `scripts/panic-audit.mjs` |
| **PS-001** | `scripts/prompt-shape-audit.mjs` |
| **PS-002** | `scripts/prompt-shape-audit.mjs` |
| **PS-003** | `scripts/prompt-shape-audit.mjs` |
| **R-001** | `scripts/dead-code-audit.mjs` |
| **R-002** | `scripts/dead-code-audit.mjs` |
| **RF-001** | `scripts/gen-doc-refs.mjs` |
| **RF-002** | `scripts/gen-doc-refs.mjs` |
| **RF-003** | `scripts/gen-doc-refs.mjs` |
| **RF-004** | `scripts/gen-doc-refs.mjs` |
| **RG-001** | `scripts/registry-lib.mjs` |
| **RG-002** | `scripts/registry-lib.mjs` |
| **RG-003** | `scripts/registry-lib.mjs` |
| **RG-004** | `scripts/registry-lib.mjs` |
| **RG-005** | `scripts/registry-lib.mjs` |
| **RG-006** | `scripts/registry-lib.mjs` |
| **RG-007** | `scripts/registry-lib.mjs` |
| **RG-008** | `scripts/registry-lib.mjs` |
| **RG-009** | `scripts/registry-lib.mjs` |
| **RG-010** | `scripts/registry-lib.mjs` |
| **RG-011** | `scripts/registry-lib.mjs` |
| **RG-012** | `scripts/registry-lib.mjs` |
| **RG-013** | `scripts/registry-lib.mjs` |
| **RG-014** | `scripts/registry-lib.mjs` |
| **RG-015** | `scripts/registry-lib.mjs` |
| **RG-016** | `scripts/registry-lib.mjs` |
| **RG-017** | `scripts/registry-lib.mjs` |
| **RG-018** | `scripts/registry-lib.mjs` |
| **RG-019** | `scripts/registry-lib.mjs` |
| **RG-020** | `scripts/registry-lib.mjs` |
| **RG-021** | `scripts/registry-lib.mjs` |
| **RG-022** | `scripts/registry-lib.mjs` |
| **RG-023** | `scripts/registry-lib.mjs` |
| **RG-024** | `scripts/registry-lib.mjs` |
| **RG-025** | `scripts/registry-lib.mjs` |
| **RG-026** | `scripts/registry-lib.mjs` |
| **RG-027** | `scripts/registry-lib.mjs` |
| **RG-028** | `scripts/registry-lib.mjs` |
| **RG-029** | `scripts/registry-lib.mjs` |
| **S-001** | `scripts/grep-audit.mjs` |
| **S-002** | `scripts/grep-audit.mjs` |
| **S-003** | `scripts/grep-audit.mjs` |
| **S-006** | `scripts/grep-audit.mjs` |
| **S-007** | `scripts/grep-audit.mjs` |
| **S-008** | `scripts/grep-audit.mjs` |
| **S-009** | `scripts/grep-audit.mjs` |
| **S-010** | `scripts/grep-audit.mjs` |
| **S-011** | `scripts/grep-audit.mjs` |
| **S-012** | `scripts/grep-audit.mjs` |
| **SX-001** | `scripts/slo-exit-audit.mjs` |
| **SX-002** | `scripts/slo-exit-audit.mjs` |
| **SX-003** | `scripts/slo-exit-audit.mjs` |
