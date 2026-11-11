---
kind: task
id: T-345-wire命名收进模型插件
title: wire 命名收进模型插件
status: todo
track: arch
prio: next
created: 2026-11-11
parent: T-342-吸收v3机制面SPRINT
deps: [T-343-端口集搬到装配期收集]
---

# wire 命名收进模型插件

## 内容

`D-444` 已判「端口身份与 wire 别名分离、消歧在请求成形前」，但落差在执行位置：v2 的
wire 命名规则摊在 4 处生产调用点（`plugin-model/src/types.rs`、`session/plugin/turn.rs`、
`plugin-tool/src/port.rs`、门面转发）。v3 把它收进 `plugin-model/src/tool_projection.rs`：
身份与别名分开、映射一一、**协议名称转换归模型插件**。

本任务搬的是**位置**不是判据：v2 的碰撞处理更硬（两个身份映到同一 wire ⇒ 整组退出本轮
暴露集合 + `collisions` 报因，`turn.rs::exposed_tools`），这一条保留；搬的是「谁知道协议
字符集」这件事——会话只交暴露集合的交集，不由它算别名。

where: `crates/symbio-plugin-model/src/tool_projection.rs`（新建）、
`crates/symbio-plugin-session/src/plugin/turn.rs`、`crates/symbio-plugin-tool/src/port.rs`、
`crates/symbio-plugin-model/src/types.rs`、`crates/symbio-plugin-api/src/lib.rs`（门面）。

exit: `capability_to_wire` 的生产调用点只剩模型插件的投影模块（门面转发不计）；
`session` 不再调用它，只读投影交来的别名表；碰撞反例（整组退出 + 报因）仍有用例；
`cargo test -p symbio-plugin-model -p symbio-plugin-session -p symbio-plugin-tool` 全绿。

## 处理过程

- 2026-11-11：立条目。排在 `T-343` 之后：暴露集合与有效绑定同源，先把端口集换载体，
  再谈「谁算别名」，否则投影要同时读两种载体。
