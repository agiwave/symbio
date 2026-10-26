---
kind: task
id: T-318-A2-kernel-子集与审计范围
title: Phase A2：core 内划 kernel 子集，门禁只对 kernel 生效
status: todo
created: 2026-10-26
track: impl
prio: next
deps: [T-317-A1-kernel-纯度与扩展局部性判据]
parent: T-316-内核边界与扩展局部性收口
where: symbio/src/symbio_core/ · scripts/core-shape-audit.mjs · scripts/core-export-audit.mjs
exit: 'core 内出现 `kernel/` 门面，只暴露 DE-023 §5 的语法与少量协议类型；形状与根出口审计范围收窄到 kernel；现有域行为零变化，全量门禁全绿。'
---

## 内容

不改行为，只改文件边界：

1. `kernel/` 只装 Event / Store / Projection / ring / container / seal 的语法面；
2. 对外一个 `Kernel` 门面 + 协议类型；
3. 现有域继续留在 core，但 `core-shape-audit` / `core-export-audit` 只审 `kernel/`；
4. 明确列出 kernel 文件集（供 T-317 的局部性夹具使用）。

## 处理过程

- 2026-10-26：立任务。文件集清单先与 DE-023 §5 对齐，再改审计范围，避免先改门禁后补边界。
