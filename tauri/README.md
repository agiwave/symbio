# Tauri 前端

Symbio 桌面前端：Vue 3 + TypeScript + Tauri 2。

## 结构

```
tauri/
├── src/              # Vue3 应用（视图、组件、store、API 层）
├── src-tauri/        # Tauri 宿主（Rust）：仅 3 个 command
└── docs/             # 前端文档
    └── FRONTEND.md   # 前端架构与机制
```

## Tauri 宿主命令（src-tauri）

| Command | 说明 |
|---------|------|
| `route_v2` | 同步路由调用 → 后端插件树 |
| `route_v2_send` | 向活动会话发送流式帧 |
| `route_v2_close` | 关闭会话连接 |

宿主不含业务逻辑：所有路由经 `route_v2` 直通插件树，与 gateway 的 `/api/route` 同构。

## 文档

- 前端架构与机制：`docs/FRONTEND.md`
- 后端总览：`../README.md`、`../docs/SYSTEM_MAP.md`
