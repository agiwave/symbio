# Event Bus 插件

进程内事件总线：聚合各插件发出的帧并向订阅连接广播（连接级 SSE 风格推送）。

## 路由

清单见 `docs/reference/ROUTES.md` §Event Bus 插件（**权威**）：`event_bus/subscribe`、
`event_bus/ping`、`event_bus/pending/snapshot`。

> `event_bus/publish` **不存在**：本插件是进程内帧广播（会话流式增量、VDFS 变更等），
> 发布方在进程内直接调用 `EventBus`，不经路由。

## 机制

- 前端推送（会话流式、VDFS 变更）经 event_bus 取得广播帧；tauri 宿主同理。
- 帧 payload 与 PluginChannel 的 Session 帧同构，客户端按 `kind` 分发（闭集与常量见
  `docs/architecture/PROTOCOLS.md` §事件总线频道）。

## 关联

- 入站网关：`../gateway/README.md`
- 通道协议：`docs/architecture/PROTOCOLS.md`
