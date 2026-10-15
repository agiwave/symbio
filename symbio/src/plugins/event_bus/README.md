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

## 关联

- 入站网关：`../gateway/README.md`
- 通道协议：`docs/005-arch/004-PROTOCOLS.md`
