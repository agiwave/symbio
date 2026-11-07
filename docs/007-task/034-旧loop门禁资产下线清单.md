---
kind: task
id: T-034-旧loop门禁资产下线清单
title: 旧 loop 门禁资产：下线未执行、判据仍在线
status: doing
track: arch
prio: now
created: 2026-11-04
deps: []
where: "现状核对：旧 loop 门禁资产的下线计划与 gate 实际接线"
exit: "逐项标出现状（仍在线 / 已改指 / 已不存在）与接线处；结论为不执行下线，判据面维持"
---

# 旧 loop 门禁资产：下线未执行、判据仍在线

## 内容

### 结论

下线计划**未执行，并已反转**。旧 `symbio-plugin-loop` 源码整体删除后，判据没有跟着删除或撤线，
而是继续在线上运行、改指新 crate。当前 gate 判据面 181 项；下列脚本全部存在并由
`scripts/gate.d/30-docs.mjs` 接线。

## 现状

| 资产 | 现状 | 接线 / 说明 |
|---|---|---|
| `scripts/control-plane-audit.mjs` | 在线 | `30-docs.mjs` 的 `GUARDS`；机制面指纹基线已随新环重算（D-360 / D-366） |
| `scripts/baseline-scope-audit.mjs` | 在线 | 本体由 `gate.d/36-baseline-scope.mjs` spawn，`GUARDS` 与 `LIBS` 各一处 |
| `scripts/turn-root-audit.mjs` | 在线 | `30-docs.mjs` 的 `GUARDS`；turn 词根存量只许降 |
| `scripts/engine-ledger-audit.mjs` | 在线 | `30-docs.mjs` 的 `GUARDS` |
| `scripts/extension-locality-audit.mjs` | 在线 | `30-docs.mjs` 的 `GUARDS`（EX-002） |
| `scripts/capability-migration-audit.mjs` | 在线 | `30-docs.mjs` 的 `GUARDS`（EX-003 / EX-007） |
| `scripts/loop-purity-audit.mjs` | 在线 | `30-docs.mjs` 的 `GUARDS`（LP-001…005） |
| `scripts/obligation-ring-accept.mjs` | 在线 | `30-docs.mjs` 的 `GUARDS` 与 `LIBS` |
| `scripts/facts-boundary-audit.mjs` | 在线 | `30-docs.mjs` 的 `GUARDS`（FB-002） |

`docs/005-spec/033.md` 与 `docs/005-spec/README.md` 里的判据表继续有效；`loop-purity-baseline.json`、
`turn-root-baseline.json`、`scripts/baselines/control-plane.json` 均按新 crate 重算，
`_shared.mjs` 的测试总数按实际读数维护。旧 crate 的文件路径不再是任何判据的锚点。

## 处理过程

本档是清点与现状核对记录，未改动任何门禁脚本或基线。
