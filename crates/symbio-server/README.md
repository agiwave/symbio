---
kind: design
id: crates/symbio-server/README
title: Server 与共享运行时
status: active
created: 2026-11-24
impl: partial
---

# Server 与共享运行时

`symbio-server` 是 Symbio 的共享运行时宿主与浏览器服务库。CLI、Tauri 和 `symbio-cli
serve` 都经 `RuntimeHost` 创建同一棵插件树；Server 额外提供静态 Vue 资源与宿主目录
浏览 API。本 crate 不发布独立二进制。

## 边界

- `symbio-http` 拥有传输、鉴权与连接机制；本 crate 的 `Ingress` 决定 Server 的公开面。
- 宿主目录 API 只列目录，不能读取文件，也不替代物理 VDFS。
- 浏览器只走 HTTP/WS；Tauri IPC 只存在于 embedded 环境。
- homedir / rootdir 是系统数据根，workdir 是会话元数据，二者不互换。

## 运行

```text
symbio-cli serve --bind 127.0.0.1 --port 9231 --homedir <系统根> --frontend-dist tauri/dist
```

非回环监听必须提供令牌；浏览器 WebSocket 通过 `bearer.<token>` 子协议携带同一个令牌。

## 浏览器接入安全

- 默认只允许同源 HTTP/WS。回环免令牌模式仍校验 `Host` 与 `Origin`，拒绝跨站页面和
  Host 重绑定；非回环监听必须设置 `--token`。
- 显式跨源访问用 `--allowed-origin <精确 Origin>`（或 `SYMBIO_SERVER_ALLOWED_ORIGIN`），
  必须同时设置令牌；不接受通配 Origin。普通 HTTP 使用 `Authorization: Bearer <token>`。
- `POST /api/v1/invoke` 只接受 `Content-Type: application/json`；静态资源、宿主目录与
  `/api/v1/events`、`/api/v1/ws` 共用这套传输鉴权。
- 请求读取和单次响应写入分别限制为 15 秒；停机给连接 20 秒退出，超时后才强制结束。
- `home/reload` 持运行时写锁，取消并等待其他 SSE/WS 连接退出后再重载。WebSocket 发起
  重载时保留自身连接以避免自等待，重载响应发送后即关闭。

宿主目录入口是 `GET /api/v1/host/runtime` 与
`GET /api/v1/host/directories?path=<Server 路径>`；两者都要求鉴权，且只返回目录信息。

一会话一 worker 的生产绑定已落到 `RuntimeHost`：会话作用域请求经 durable inbox 派发到
独立 worker 进程，worker 的事件回传控制面总线。独立进程不宣称隔离；容器落点与子会话
spawn 尚未完成，目标拓扑见 [DE-060](../../docs/003-design/060-Server运行时与会话执行.md)。
