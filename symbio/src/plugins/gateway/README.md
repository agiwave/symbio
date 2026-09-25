# Gateway 插件

入站网关：把 HTTP/WebSocket 客户端请求适配为插件路由调用，是外部接入 Symbio 的统一入口（与 Tauri 宿主的 route_v2 同构）。

## 路由

端点清单见 `docs/reference/ROUTES.md` §Gateway 插件（**权威**，含鉴权与只读白名单）。
三类端点：`POST /api/v1/invoke`（对齐 `route_v2`）、`WS /api/v1/ws`（首帧即请求体，
此后双向 `PluginFrame`）、`GET /api/v1/health`。本插件自身接口**恒走进程内
（in-process）**，前端不经 HTTP 访问（`gateway/status` 除外）。

> 「进程内」= 不经 HTTP/WS 传输、由宿主直接调用。这与已删除的 `PluginPayload::Native`
> 变体**无关**——那曾是协议层的一个死变体（见 `docs/architecture/PROTOCOLS.md` §载荷协议）。

## 机制

- **交付分类**：`classify_payload` 把 `PluginPayload` 的 3 态分成「一次性」（`Data` /
  `Empty`）与「长连接」（`Session`）——**两个入口共用它**，各自只保留真差异（HTTP 把
  `Session` 折叠为最后一帧、WS 双向转发；`Empty` 在 HTTP 写 `null`、在 WS 直接关连接）。
  从前这段分类在两个入口各写一遍，漏改一处就出现「HTTP 能调、WS 不能调」这类不对称。
- **协议转换**：HTTP body → `InvokeRequest`（PATH/payload/SESSION_ID），响应 → `InvokeResponse<PluginPayload>`。
- **流式**：WS 场景下把 `Session(PluginChannel)` 帧逐条推送（不用 SSE——会话需双向通道）。
- **鉴权与错误**：错误码遵循 `docs/reference/ERROR_CODES.md`。

## 关联

- 设计文档：`docs/design/http-api-transport.md`
- 数据流转：`docs/architecture/DATA_FLOW.md`
