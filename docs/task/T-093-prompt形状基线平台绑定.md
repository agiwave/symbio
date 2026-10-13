---
kind: task
id: T-093-prompt形状基线平台绑定
title: t46 形状基线把「平台命名的 shell 工具」（Windows `cmd` / Linux `sh`）钉进了判据——基线只能在捕获它的那个平台绿
status: todo
created: 2026-10-13
track: e2e
deps: []
prio: now
where: docs/ref/PROMPT_SHAPE.md（基线本体）+ scripts/prompt-shape-audit.mjs（PS-001 工具面判据）+ e2e/cases/t46-prompt-shape-baseline.mjs
exit: t46 在 Windows 与 Linux 上**同时**可绿（基线不再把平台命名的工具当结构判据，或按平台分别持有基线）；若为此改了 ADR-057 的判据口径，变更写进该 ADR
---

## 内容

CI 的 `e2e-check` 恒定红在 `t46-prompt-shape-baseline`（run 38372186256：48/50；run 38358236876：49/50），
而本机（Windows）跑整轮门禁是 50/50 全绿。红的形式是
`assertEq(a.status, 0)` ⇒ `prompt-shape-audit.mjs` 以 PS-001「形状与基线不一致」退出 1。

## 诊断（2026-10-13）

`docs/ref/PROMPT_SHAPE.md` 的判据 JSON 里，三个请求的工具面都是
`agent_run,cmd,content_search,todo_write,…`（工具数 21）——**其中 `cmd` 是 Windows 的 shell 工具名**。

`symbio/src/plugins/local/shell.rs:95-124` 的 `tool_name` 按 `target_os` 取值：
Windows = `cmd`，macOS = `sh`，Linux = `sh/bash`（取 `sh`）。于是 Linux 上
`diffShapes`（`scripts/prompt-shape-audit.mjs:214`）判「工具面：新增 [sh] 移除 [cmd]」⇒ PS-001 红。

旁证两条：① 基线自述「重生成于 2026-10-12」——在 Windows 上重生成；② 2026-10-08 的
CI run 37721749855 里 `e2e-check` 是**绿**的，与「那份基线此前是在 Linux 上捕获的」一致。

**结论**：t46 的基线是**平台绑定**的（它把「平台命名的工具」当成了结构判据），
因此「哪台机器最后重生成，CI 就在另一台机器上红」——这条守卫在 CI 上目前无法执行其职责。

## 处置方向（待定，属 ADR-057 的口径）

两条候选，需择一并写明理由：

1. **归一化平台变体**：`extractShape` 把已知的平台命名工具（`cmd` / `sh` / `zsh/sh`）映射到一个
   规范记号（如 `shell`），判据仍记「顺序 + 计数 + 其余名字」。改动最小，且**平台无关性由构造保证**
   （归一化后的形状在两平台上逐字段相同），本机即可验证；代价是基线不再记录「模型被告诉用哪个 shell」。
2. **按平台分别持有基线**：基线文件里以 `场景 + 平台` 为键存多份，各平台与自己的那份比。
   判据更强（每个平台的形状都被钉住），但**Linux 那份只能在 Linux 上捕获**，本机无法生成。

必要性论证（两条候选共有）：现状下 t46 在 CI 上**恒红且红得无信息**（报的是「形状漂移」，
真因是平台），这属于本仓反复记录的「守卫看起来在跑、其实判不了」形态；不修则
`e2e-check` 永远不可能绿。

## 不是本任务范围

- 不改 `shell.rs` 的工具名（那是**生产行为**：模型看到的工具名会变，且 e2e / 文档有引用）。
- 不追 `t36-autonomous-initiator` 的一次失败（run 38372186256，1m41s 超时形态）——它在更早的
  run 里是绿的，形态更像抖动；若再复现，另立任务。

## 处理过程

- 2026-10-13 登记：清偿 `v2` 分支 CI 全红时，逐 job 排查到 `e2e-check`；本地 e2e 50/50 全绿 ⇒
  定位为平台差异；读基线判据 JSON 见 `cmd`，对照 `shell.rs` 的 `target_os` 取值链得出上述结论。
