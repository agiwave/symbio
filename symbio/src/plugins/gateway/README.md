# Gateway 插件

入站网关：把 HTTP/WebSocket 客户端请求适配为插件路由调用，是外部接入 Symbio 的统一入口（与 Tauri 宿主的 route_v2 同构）。

## 路由（HTTP 端点）

| 端点 | 说明 |
|------|------|
| `POST /api/route` | 同步路由调用（对齐 route_v2） |
| `GET /api/ws` | WebSocket 双向会话（对齐 route_v2_send/route_v2_close） |
| `GET /api/events` | SSE 事件流（经 event_bus 广播） |

## 机制

- **协议转换**：HTTP body → `InvokeRequest`（PATH/PAYLOAD/SESSION_ID），响应 → `InvokeResponse<PluginPayload>`。
- **流式**：WS/SSE 场景下把 `Session(PluginChannel)` 帧逐条推送。
- **鉴权与错误**：错误码遵循 `docs/reference/ERROR_CODES.md`；完整端点清单见 `docs/reference/ROUTES.md`。

## 关联

- 设计文档：`docs/design/http-api-transport.md`（已实现落地）
- 数据流转：`docs/architecture/DATA_FLOW.md`
