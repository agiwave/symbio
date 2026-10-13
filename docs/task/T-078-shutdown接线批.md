---
kind: task
id: T-078-shutdown接线批
title: ADR-033 残余清偿：进程退出与丢弃旧容器的收尾接线（`PluginStopReason::Shutdown` 从纸面能力变生产路径）
status: todo
created: 2026-10-13
track: 插件生命周期
deps: []
prio: later
exit: ① `symbio/src/symbio_core/plugin/mod.rs` 定义的 `PluginStopReason::Shutdown` 出现生产触发点（两个宿主入口：`cli/` 与 `tauri/src-tauri/` 的退出路径，整树 stop 的遍历 owner = `symbio/src/plugins/composite/registry.rs`）；② 判据 = Rust 集成测试：进程正常退出时全部已装配插件收到恰一次 `stop(Shutdown)` 且理由可读；③ [ADR-033](../adr/ADR-033.md) 正文「未完成（明确记录）」段据此收口
---

## 内容

[ADR-033](../adr/ADR-033.md) 明文记录：「进程退出」与
「丢弃旧容器」两类收尾**本期不接**——今天外部进程插件（mcp 子进程、telegram listener）的
优雅关闭只有停用/卸载两条用户动作，宿主退出不走钩子。按裁定，此类妥协残余属历史遗留，
由本任务接管。

## 处理过程

- 2026-10-13 登记（ADR 全量复核发现：`进程退出钩子 / 丢弃旧容器 / Shutdown` 关键词全 docs 只命中 ADR 自己）。
