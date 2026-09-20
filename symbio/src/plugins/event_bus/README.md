# Event Bus 插件

进程内事件总线：聚合各插件发出的帧并向订阅连接广播（连接级 SSE 风格推送）。

## 路由

清单见 `docs/reference/ROUTES.md` §Event Bus 插件（**权威**）：`event_bus/subscribe`、
`event_bus/ping`、`event_bus/pending/snapshot`。

> `event_bus/publish` **不存在**：本插件是进程内帧广播（VDFS 变更、总线自身握手），
> 发布方在进程内直接调用 `EventBus`，不经路由。

## 机制

- 推送（VDFS 变更、`connected` 握手）经 event_bus 取得广播帧；前端 / CLI / tauri 宿主同理。
- 帧 payload 与 PluginChannel 的 Session 帧同构，客户端按 `kind` 分发（闭集与常量见
  `docs/architecture/PROTOCOLS.md` §事件总线频道）。
- 本插件是**纯传输层**：不认识任何业务频道语义。曾经由它承载的会话事件频道
  （`kind = "session"`）已随「状态即节点属性」一并废除，会话域只剩 `vdfs` 一条频道。

## 关联

- 入站网关：`../gateway/README.md`
- 通道协议：`docs/architecture/PROTOCOLS.md`
