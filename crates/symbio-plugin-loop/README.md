---
kind: design
id: crates/symbio-plugin-loop/README
title: Loop 插件（纯机制环）
status: active
created: 2026-10-19
impl: done
---

# Loop 插件（纯机制环）

> **旧编排（幕机 / 入口 / 阶段事件 / 行表阶段号）已随 B2 退场（D-318）。**
> 本目录的现行内容是**纯机制环**：选债 / 派发 / 等待 / 配对 / 记账 / 预算与租约 / 重放。
> 机制形状与裁决见 [DE-048-端口机制裁决](../../docs/003-design/048-端口机制裁决.md)、[DE-051-机制形状规格](../../docs/003-design/051-机制形状规格.md)；
> 旧编排的落点与去向见 [DE-052-决策正文证据锚点冻结表](../../docs/003-design/052-决策正文证据锚点冻结表.md)。

## 现行入口

| 文件 | 职责 |
|---|---|
| [`src/ring/mod.rs`](src/ring/mod.rs) | 五类事实、事实投影、债表与驱动入口 |
| [`src/drive.rs`](src/drive.rs) | 驱动执行者：把环交给宿主的通用驱动契约 |
| [`src/plugin.rs`](src/plugin.rs) | 注册入口：交出驱动执行者 |
| [`src/vocab.rs`](src/vocab.rs) | 端口名；全部从行数据读入，代码里没有字面量 |

## 边界

本 crate **对外零导出**：宿主不认识环的任何类型。环经 `plugin_api::drive` 的通用契约
与宿主交互，能力束与可见域是**调用参数**，不是注册面（DE-051）。环本身不持有账本 /
预算 / 执行体注册表，不生成身份、不读序号、不认阶段。
