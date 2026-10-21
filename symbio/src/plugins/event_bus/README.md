---
kind: arch
id: symbio/src/plugins/event_bus/README
title: Event Bus 插件
status: active
created: 2026-09-09
---

# Event Bus 插件

进程内事件总线：聚合各插件发出的帧并向订阅连接广播（连接级 SSE 风格推送）。

## 路由

清单见 `docs/006-ref/003-ROUTES.md` §Event Bus 插件（**权威**）：`event_bus/subscribe`、
`event_bus/ping`。

> `event_bus/publish` **不存在**：本插件是进程内帧广播（VDFS 变更、总线自身握手），
> 发布方在进程内直接调用 `EventBus`，不经路由。

## 机制

- 推送（VDFS 变更、`connected` 握手）经 event_bus 取得广播帧；前端 / CLI / tauri 宿主同理。
- 帧 payload 与 PluginChannel 的 Session 帧同构，客户端按 `kind` 分发（闭集与常量见
  `docs/005-arch/004-PROTOCOLS.md` §事件总线频道）。
- 本插件是**纯传输层**：不认识任何业务频道语义；会话域只有 `vdfs` 一条频道
  （见 `docs/005-arch/004-PROTOCOLS.md` §事件总线频道）。
- **慢消费者的兜底是一条指令，不是一个序号**：通道满时 `EventBus::try_publish` 丢帧但
  **保留订阅**，并补送一条 `{ "type": "resync" }`（刻意不带 `path`，故不会被解成变更）。
  **每个订阅者都必须认它**并按自己的作用域重读——判别走 core 的 `event_bus_resync_of`，
  不要手写一份（手写的那两份在 `cli` 与 telegram，而子智能体转播桥曾一份都没写，
  于是把「本轮结束」的兜底也丢了）。为什么不改成序号：`docs/010-adr/025.md`
  （顺序是**节点属性**，不是投递属性）。

## 关联

- 入站网关：`../gateway/README.md`
- 通道协议：`docs/005-arch/004-PROTOCOLS.md`
