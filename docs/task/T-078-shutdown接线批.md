---
kind: task
id: T-078-shutdown接线批
title: ADR-033 残余清偿：进程退出与丢弃旧容器的收尾接线（`PluginStopReason::Shutdown` 从纸面能力变生产路径）
status: done
created: 2026-10-13
track: 插件生命周期
deps: []
prio: later
evidence: `symbio/src/plugins/composite/composite.test.rs` 三条探针测试（整树广播各恰一次 · 单家失败不拦其余 · 非 Shutdown 理由不广播）+ 接线点 `cli/src/main.rs`（四种用法的唯一出口 `client.shutdown()`）与 `tauri/src-tauri/src/main.rs`（`RunEvent::Exit`）；[ADR-033](../adr/ADR-033.md) 正文收口、`impl: done`；composite 单测 32/32 绿 · cli/tauri cargo check 绿 · clippy 零告警
exit: ① `symbio/src/symbio_core/plugin/mod.rs` 定义的 `PluginStopReason::Shutdown` 出现生产触发点（两个宿主入口：`cli/` 与 `tauri/src-tauri/` 的退出路径，整树 stop 的遍历 owner = `symbio/src/plugins/composite/registry.rs`）；② 判据 = Rust 集成测试：进程正常退出时全部已装配插件收到恰一次 `stop(Shutdown)` 且理由可读；③ [ADR-033](../adr/ADR-033.md) 正文「未完成（明确记录）」段据此收口
---

## 内容

[ADR-033](../adr/ADR-033.md) 明文记录：「进程退出」与
「丢弃旧容器」两类收尾**本期不接**——今天外部进程插件（mcp 子进程、telegram listener）的
优雅关闭只有停用/卸载两条用户动作，宿主退出不走钩子。按裁定，此类妥协残余属历史遗留，
由本任务接管。

## 处理过程

- 2026-10-13 登记（ADR 全量复核发现：`进程退出钩子 / 丢弃旧容器 / Shutdown` 关键词全 docs 只命中 ADR 自己）。
- 2026-10-15 闭批：遍历 owner = `PluginRegistry::shutdown_all`（对 `snapshot()` 广播
  `stop(Shutdown)`，单家失败记 `plugin_warn` 继续——与 ADR 的 `stop` 失败口径一致）；
  容器侧 `Composite::stop` 覆写：仅 `Shutdown` 委托 `shutdown_all`，`Disabled` /
  `Uninstalled` 是对单个插件的处置、不广播。调用点两处：**CLI**——`main` 重构为
  「各分支只产出退出码 + 唯一出口 `client.shutdown()`」（`run_*` 由值传递改
  `&mut` 借用；收尾散进各分支是第三种用法出现时最容易漏掉的形态）；**Tauri**——
  `.run(generate_context)` 改 `.build()` + run 回调，`RunEvent::Exit` 时
  `block_on` 整树收尾。判据为三条探针测试（挂载两个 `StopProbe` 到测试容器，
  记录 stop 理由与次数）。ADR-033 正文「未完成」段改为接线记录、`impl: done`；
  「丢弃旧容器」仍不接——仓库内没有「重建整棵树」的运行时路径，出现该路径前
  保持不接（已在 ADR 写明撤销条件）。composite 单测 32/32 绿 · cli 与
  tauri `cargo check` 绿 · clippy 零告警。
