# 判据码登记表（自动生成）

> ⚠️ **本表由 `scripts/gen-gate-codes.mjs` 从各脚本的 `// @ns` / `// @codes` 声明生成，请勿手改。**
> 改了声明不用手动重跑——门禁会**自动重新生成并暂存**（见 `scripts/gate.d/60-facts.mjs`）。
>
> 判据码是每条机械判定的**地址**：豁免注释、门禁日志、文档指认都指向它。
> 每条判据**判什么**归声明它的那个脚本的头注释；撞号与漏登记由
> [`scripts/gate-codes-audit.mjs`](../../scripts/gate-codes-audit.mjs) 判（GC-001–006）。

## 1. 命名空间

| 前缀 | 归属 | 声明脚本 | 已用号段 | 下一个可用号 |
|---|---|---|---|---|
| **C** | core 出口 | `scripts/core-export-audit.mjs` | C-001–003 | `C-004` |
| **D** | 文档正文与链接 | `scripts/doc-link-audit.mjs` · `scripts/doc-symbol-audit.mjs` | D-001–009 | `D-010` |
| **E** | 插件入口与门面 | `scripts/plugin-entry-audit.mjs` | E-001–009、E-011 | `E-012` |
| **GC** | 判据码命名空间本身 | `scripts/gate-codes-audit.mjs` | GC-001–006 | `GC-007` |
| **GW** | 门禁接线（测试文件与门禁名单的对账） | `scripts/gate-wiring-audit.mjs` | GW-001–003 | `GW-004` |
| **M** | 机制表 | `scripts/mechanism-audit.mjs` | M-001–007 | `M-008` |
| **N** | core 命名 | `scripts/core-naming-audit.mjs` | N-001–005 | `N-006` |
| **NDC** | 主体直连 | `scripts/no-direct-call-audit.mjs` | NDC-001–002 | `NDC-003` |
| **R** | 死代码 | `scripts/dead-code-audit.mjs` | R-001–002 | `R-003` |
| **S** | 源码 grep 形态 | `scripts/grep-audit.mjs` | S-001–003、S-006–012 | `S-013` |

## 2. 逐码归属

| 判据码 | 声明脚本 |
|---|---|
| **C-001** | `scripts/core-export-audit.mjs` |
| **C-002** | `scripts/core-export-audit.mjs` |
| **C-003** | `scripts/core-export-audit.mjs` |
| **D-001** | `scripts/doc-link-audit.mjs` |
| **D-002** | `scripts/doc-link-audit.mjs` |
| **D-003** | `scripts/doc-link-audit.mjs` |
| **D-004** | `scripts/doc-link-audit.mjs` |
| **D-005** | `scripts/doc-symbol-audit.mjs` |
| **D-006** | `scripts/doc-link-audit.mjs` |
| **D-007** | `scripts/doc-link-audit.mjs` |
| **D-008** | `scripts/doc-link-audit.mjs` |
| **D-009** | `scripts/doc-symbol-audit.mjs` |
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
| **GC-001** | `scripts/gate-codes-audit.mjs` |
| **GC-002** | `scripts/gate-codes-audit.mjs` |
| **GC-003** | `scripts/gate-codes-audit.mjs` |
| **GC-004** | `scripts/gate-codes-audit.mjs` |
| **GC-005** | `scripts/gate-codes-audit.mjs` |
| **GC-006** | `scripts/gate-codes-audit.mjs` |
| **GW-001** | `scripts/gate-wiring-audit.mjs` |
| **GW-002** | `scripts/gate-wiring-audit.mjs` |
| **GW-003** | `scripts/gate-wiring-audit.mjs` |
| **M-001** | `scripts/mechanism-audit.mjs` |
| **M-002** | `scripts/mechanism-audit.mjs` |
| **M-003** | `scripts/mechanism-audit.mjs` |
| **M-004** | `scripts/mechanism-audit.mjs` |
| **M-005** | `scripts/mechanism-audit.mjs` |
| **M-006** | `scripts/mechanism-audit.mjs` |
| **M-007** | `scripts/mechanism-audit.mjs` |
| **N-001** | `scripts/core-naming-audit.mjs` |
| **N-002** | `scripts/core-naming-audit.mjs` |
| **N-003** | `scripts/core-naming-audit.mjs` |
| **N-004** | `scripts/core-naming-audit.mjs` |
| **N-005** | `scripts/core-naming-audit.mjs` |
| **NDC-001** | `scripts/no-direct-call-audit.mjs` |
| **NDC-002** | `scripts/no-direct-call-audit.mjs` |
| **R-001** | `scripts/dead-code-audit.mjs` |
| **R-002** | `scripts/dead-code-audit.mjs` |
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
