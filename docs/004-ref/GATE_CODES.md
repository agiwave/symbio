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
| **AO** | Agent 归属表对账（模块文档 ↔ 装配清单 ↔ 插件目录） | `scripts/agent-ownership-audit.mjs` | AO-001–006 | `AO-007` |
| **C** | core 出口 | `scripts/core-export-audit.mjs` | C-001–005 | `C-006` |
| **CP** | 控制面终态（唯一循环体 / 唯一选债口 / 守卫同处 / 机制层无域名 / 收束格单写方 / 机制面冻结与端口闭集 / 判据集闭集 / 声明面封闭） | `scripts/control-plane-audit.mjs` | CP-001–008 | `CP-009` |
| **CS** | core 形状 | `scripts/core-shape-audit.mjs` · `scripts/shared-event-vocab-audit.mjs` | CS-001–010 | `CS-011` |
| **D** | 文档正文与链接 | `scripts/doc-link-audit.mjs` · `scripts/doc-symbol-audit.mjs` | D-001–014、D-016–020 | `D-021` |
| **DC** | 文档计数对账（文档与源码注释里的少数几个数 vs 代码） | `scripts/doc-count-audit.mjs` | DC-001–005 | `DC-006` |
| **DS** | 文档小节寻址（一级编号唯一递增 + §N 引用可解析） | `scripts/doc-section-audit.mjs` | DS-001–002 | `DS-003` |
| **E** | 插件入口与门面 | `scripts/plugin-entry-audit.mjs` | E-001–009、E-011–013 | `E-014` |
| **EOL** | 行尾（工作树 / 入库 / 声明三面一致） | `scripts/eol-audit.mjs` | EOL-001–003 | `EOL-004` |
| **EX** | 能力全面拆分（名册处置与迁移边界） | `scripts/capability-ledger-audit.mjs` · `scripts/capability-migration-audit.mjs` · `scripts/extension-locality-audit.mjs` · `scripts/extension-vocab-audit.mjs` · `scripts/uninstall-edge-audit.mjs` | EX-002–004、EX-006–008 | `EX-009` |
| **FB** | 事实轨边界 | `scripts/facts-boundary-audit.mjs` | FB-001–002 | `FB-003` |
| **GC** | 判据码命名空间本身 | `scripts/gate-codes-audit.mjs` | GC-001–006 | `GC-007` |
| **GW** | 门禁接线（测试文件与门禁名单的对账） | `scripts/gate-wiring-audit.mjs` | GW-001–006 | `GW-007` |
| **HS** | 扩展层插件宿主面 | `scripts/plugin-surface-audit.mjs` | HS-001–004 | `HS-005` |
| **IM** | 内部请求模型调用（档位闸门的覆盖范围） | `scripts/internal-model-audit.mjs` | IM-001–004 | `IM-005` |
| **INJ** | 异常注入矩阵 | `scripts/injection-matrix-audit.mjs` | INJ-001–003 | `INJ-004` |
| **KB** | 前端分支（形态 / 资源类型） | `scripts/kind-branch-audit.mjs` | KB-001–002 | `KB-003` |
| **LWF** | 列表写入口事实 | `scripts/list-write-fact-audit.mjs` | LWF-001–002 | `LWF-003` |
| **M** | 机制表 | `scripts/mechanism-audit.mjs` | M-001–010 | `M-011` |
| **ME** | 机制强制等级清单 | `scripts/mechanism-enforcement-audit.mjs` | ME-001–005 | `ME-006` |
| **N** | core 命名 | `scripts/core-naming-audit.mjs` | N-001–005 | `N-006` |
| **NDC** | 主体直连 | `scripts/no-direct-call-audit.mjs` | NDC-001–002 | `NDC-003` |
| **OB** | 义务环派发面（调度旁路与幂等点问的两类计数棘轮） | `scripts/obligation-bypass-audit.mjs` | OB-001–004 | `OB-005` |
| **PN** | 健壮性登记（每一处 panic 都是显式事实） | `scripts/panic-audit.mjs` | PN-001–003 | `PN-004` |
| **PP** | 投影纯净性 | `scripts/projection-purity.mjs` | PP-001–005 | `PP-006` |
| **PS** | prompt 形状基线 | `scripts/prompt-shape-audit.mjs` | PS-001–003 | `PS-004` |
| **PTS** | 端口面（协议名闭集 / 引擎对端口无感 / 求解器字节基线 / 协议格写方唯一 / 契约 7 的重试限定） | `scripts/port-surface-audit.mjs` | PTS-001–006 | `PTS-007` |
| **R** | 死代码 | `scripts/dead-code-audit.mjs` | R-001–003 | `R-004` |
| **RF** | 文档引用（编号 → 地址由机制生成） | `scripts/gen-doc-refs.mjs` | RF-001–004 | `RF-005` |
| **RG** | 登记条目校验（registry 单文档的字段 / 状态 / 依赖 / 判据指认 / 分解） | `scripts/registry-lib.mjs` | RG-001–027、RG-029–033 | `RG-034` |
| **S** | 源码 grep 形态 | `scripts/grep-audit.mjs` | S-001–003、S-006–017 | `S-018` |
| **SC** | 插件层特判（工具名 / 供应商差异） | `scripts/plugin-special-case-audit.mjs` | SC-001–006 | `SC-007` |
| **SPC** | 规范执行方覆盖 | `scripts/spec-coverage-audit.mjs` | SPC-001–003 | `SPC-004` |
| **SX** | SLO 出口表 vs session/stats 读数列（用户可见出口） | `scripts/slo-exit-audit.mjs` | SX-001–003 | `SX-004` |
| **TD** | 工具自声明行为位（声明面基线对账） | `scripts/tool-decls-audit.mjs` | TD-001–002 | `TD-003` |
| **V** | core 分域 | `scripts/core-vocab-audit.mjs` | V-001–002 | `V-003` |

## 2. 逐码归属

| 判据码 | 声明脚本 |
|---|---|
| **AO-001** | `scripts/agent-ownership-audit.mjs` |
| **AO-002** | `scripts/agent-ownership-audit.mjs` |
| **AO-003** | `scripts/agent-ownership-audit.mjs` |
| **AO-004** | `scripts/agent-ownership-audit.mjs` |
| **AO-005** | `scripts/agent-ownership-audit.mjs` |
| **AO-006** | `scripts/agent-ownership-audit.mjs` |
| **C-001** | `scripts/core-export-audit.mjs` |
| **C-002** | `scripts/core-export-audit.mjs` |
| **C-003** | `scripts/core-export-audit.mjs` |
| **C-004** | `scripts/core-export-audit.mjs` |
| **C-005** | `scripts/core-export-audit.mjs` |
| **CP-001** | `scripts/control-plane-audit.mjs` |
| **CP-002** | `scripts/control-plane-audit.mjs` |
| **CP-003** | `scripts/control-plane-audit.mjs` |
| **CP-004** | `scripts/control-plane-audit.mjs` |
| **CP-005** | `scripts/control-plane-audit.mjs` |
| **CP-006** | `scripts/control-plane-audit.mjs` |
| **CP-007** | `scripts/control-plane-audit.mjs` |
| **CP-008** | `scripts/control-plane-audit.mjs` |
| **CS-001** | `scripts/core-shape-audit.mjs` |
| **CS-002** | `scripts/core-shape-audit.mjs` |
| **CS-003** | `scripts/core-shape-audit.mjs` |
| **CS-004** | `scripts/core-shape-audit.mjs` |
| **CS-005** | `scripts/core-shape-audit.mjs` |
| **CS-006** | `scripts/core-shape-audit.mjs` |
| **CS-007** | `scripts/shared-event-vocab-audit.mjs` |
| **CS-008** | `scripts/core-shape-audit.mjs` |
| **CS-009** | `scripts/core-shape-audit.mjs` |
| **CS-010** | `scripts/core-shape-audit.mjs` |
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
| **D-012** | `scripts/doc-symbol-audit.mjs` |
| **D-013** | `scripts/doc-link-audit.mjs` |
| **D-014** | `scripts/doc-symbol-audit.mjs` |
| **D-016** | `scripts/doc-link-audit.mjs` |
| **D-017** | `scripts/doc-link-audit.mjs` |
| **D-018** | `scripts/doc-link-audit.mjs` |
| **D-019** | `scripts/doc-link-audit.mjs` |
| **D-020** | `scripts/doc-symbol-audit.mjs` |
| **DC-001** | `scripts/doc-count-audit.mjs` |
| **DC-002** | `scripts/doc-count-audit.mjs` |
| **DC-003** | `scripts/doc-count-audit.mjs` |
| **DC-004** | `scripts/doc-count-audit.mjs` |
| **DC-005** | `scripts/doc-count-audit.mjs` |
| **DS-001** | `scripts/doc-section-audit.mjs` |
| **DS-002** | `scripts/doc-section-audit.mjs` |
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
| **E-013** | `scripts/plugin-entry-audit.mjs` |
| **EOL-001** | `scripts/eol-audit.mjs` |
| **EOL-002** | `scripts/eol-audit.mjs` |
| **EOL-003** | `scripts/eol-audit.mjs` |
| **EX-002** | `scripts/extension-locality-audit.mjs` |
| **EX-003** | `scripts/capability-migration-audit.mjs` |
| **EX-004** | `scripts/uninstall-edge-audit.mjs` |
| **EX-006** | `scripts/capability-ledger-audit.mjs` |
| **EX-007** | `scripts/capability-migration-audit.mjs` |
| **EX-008** | `scripts/extension-vocab-audit.mjs` |
| **FB-001** | `scripts/facts-boundary-audit.mjs` |
| **FB-002** | `scripts/facts-boundary-audit.mjs` |
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
| **GW-006** | `scripts/gate-wiring-audit.mjs` |
| **HS-001** | `scripts/plugin-surface-audit.mjs` |
| **HS-002** | `scripts/plugin-surface-audit.mjs` |
| **HS-003** | `scripts/plugin-surface-audit.mjs` |
| **HS-004** | `scripts/plugin-surface-audit.mjs` |
| **IM-001** | `scripts/internal-model-audit.mjs` |
| **IM-002** | `scripts/internal-model-audit.mjs` |
| **IM-003** | `scripts/internal-model-audit.mjs` |
| **IM-004** | `scripts/internal-model-audit.mjs` |
| **INJ-001** | `scripts/injection-matrix-audit.mjs` |
| **INJ-002** | `scripts/injection-matrix-audit.mjs` |
| **INJ-003** | `scripts/injection-matrix-audit.mjs` |
| **KB-001** | `scripts/kind-branch-audit.mjs` |
| **KB-002** | `scripts/kind-branch-audit.mjs` |
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
| **M-009** | `scripts/mechanism-audit.mjs` |
| **M-010** | `scripts/mechanism-audit.mjs` |
| **ME-001** | `scripts/mechanism-enforcement-audit.mjs` |
| **ME-002** | `scripts/mechanism-enforcement-audit.mjs` |
| **ME-003** | `scripts/mechanism-enforcement-audit.mjs` |
| **ME-004** | `scripts/mechanism-enforcement-audit.mjs` |
| **ME-005** | `scripts/mechanism-enforcement-audit.mjs` |
| **N-001** | `scripts/core-naming-audit.mjs` |
| **N-002** | `scripts/core-naming-audit.mjs` |
| **N-003** | `scripts/core-naming-audit.mjs` |
| **N-004** | `scripts/core-naming-audit.mjs` |
| **N-005** | `scripts/core-naming-audit.mjs` |
| **NDC-001** | `scripts/no-direct-call-audit.mjs` |
| **NDC-002** | `scripts/no-direct-call-audit.mjs` |
| **OB-001** | `scripts/obligation-bypass-audit.mjs` |
| **OB-002** | `scripts/obligation-bypass-audit.mjs` |
| **OB-003** | `scripts/obligation-bypass-audit.mjs` |
| **OB-004** | `scripts/obligation-bypass-audit.mjs` |
| **PN-001** | `scripts/panic-audit.mjs` |
| **PN-002** | `scripts/panic-audit.mjs` |
| **PN-003** | `scripts/panic-audit.mjs` |
| **PP-001** | `scripts/projection-purity.mjs` |
| **PP-002** | `scripts/projection-purity.mjs` |
| **PP-003** | `scripts/projection-purity.mjs` |
| **PP-004** | `scripts/projection-purity.mjs` |
| **PP-005** | `scripts/projection-purity.mjs` |
| **PS-001** | `scripts/prompt-shape-audit.mjs` |
| **PS-002** | `scripts/prompt-shape-audit.mjs` |
| **PS-003** | `scripts/prompt-shape-audit.mjs` |
| **PTS-001** | `scripts/port-surface-audit.mjs` |
| **PTS-002** | `scripts/port-surface-audit.mjs` |
| **PTS-003** | `scripts/port-surface-audit.mjs` |
| **PTS-004** | `scripts/port-surface-audit.mjs` |
| **PTS-005** | `scripts/port-surface-audit.mjs` |
| **PTS-006** | `scripts/port-surface-audit.mjs` |
| **R-001** | `scripts/dead-code-audit.mjs` |
| **R-002** | `scripts/dead-code-audit.mjs` |
| **R-003** | `scripts/dead-code-audit.mjs` |
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
| **RG-029** | `scripts/registry-lib.mjs` |
| **RG-030** | `scripts/registry-lib.mjs` |
| **RG-031** | `scripts/registry-lib.mjs` |
| **RG-032** | `scripts/registry-lib.mjs` |
| **RG-033** | `scripts/registry-lib.mjs` |
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
| **S-013** | `scripts/grep-audit.mjs` |
| **S-014** | `scripts/grep-audit.mjs` |
| **S-015** | `scripts/grep-audit.mjs` |
| **S-016** | `scripts/grep-audit.mjs` |
| **S-017** | `scripts/grep-audit.mjs` |
| **SC-001** | `scripts/plugin-special-case-audit.mjs` |
| **SC-002** | `scripts/plugin-special-case-audit.mjs` |
| **SC-003** | `scripts/plugin-special-case-audit.mjs` |
| **SC-004** | `scripts/plugin-special-case-audit.mjs` |
| **SC-005** | `scripts/plugin-special-case-audit.mjs` |
| **SC-006** | `scripts/plugin-special-case-audit.mjs` |
| **SPC-001** | `scripts/spec-coverage-audit.mjs` |
| **SPC-002** | `scripts/spec-coverage-audit.mjs` |
| **SPC-003** | `scripts/spec-coverage-audit.mjs` |
| **SX-001** | `scripts/slo-exit-audit.mjs` |
| **SX-002** | `scripts/slo-exit-audit.mjs` |
| **SX-003** | `scripts/slo-exit-audit.mjs` |
| **TD-001** | `scripts/tool-decls-audit.mjs` |
| **TD-002** | `scripts/tool-decls-audit.mjs` |
| **V-001** | `scripts/core-vocab-audit.mjs` |
| **V-002** | `scripts/core-vocab-audit.mjs` |
