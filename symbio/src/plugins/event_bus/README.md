# Event Bus 插件

进程内事件总线：聚合各插件发出的帧并向订阅连接广播（连接级 SSE 风格推送）。

## 路由

| Path | 说明 |
|------|------|
| `event_bus/subscribe` | 订阅事件流（长连接，帧式推送） |
| `event_bus/publish` | 发布事件帧 |

## 机制

- gateway 的 `/api/events`（SSE）经 event_bus 取得广播帧；tauri 宿主的前端推送同理。
- 帧 payload 与 PluginChannel 的 Session 帧同构，客户端按 `kind` 分发。

## 关联

- 入站网关：`../gateway/README.md`
- 通道协议：`docs/architecture/PROTOCOLS.md`
