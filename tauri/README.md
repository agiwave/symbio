---
kind: arch
id: tauri/README
title: Tauri 前端
status: active
created: 2026-09-09
---

# Tauri 前端

Symbio 桌面前端：Vue 3 + TypeScript + Tauri 2。

## 结构

```
tauri/
├── src/              # Vue3 应用（视图、组件、store、API 层）
├── src-tauri/        # Tauri 宿主（Rust）：只有 `route_v2` 家族 command
└── docs/             # 前端文档
    ├── 023-FRONTEND.md     # 前端架构与机制（代码结构、与后端通信）
    └── 024-INTERACTION.md  # 交互语义（用户看到什么、动作产生什么反馈）
```

## 宿主命令（src-tauri）

`src-tauri/` 只有 `route_v2` 家族的命令——宿主不含业务逻辑，请求一律直通插件树；
`gateway` 插件的 HTTP 入口是同一件事的另一条通道。

本文不维护命令表：命令面由 [当前事实表](../docs/006-ref/006-当前事实表.md "RE-006-当前事实表") §5.2 从代码投影，
逐条职责见 `docs/023-FRONTEND.md`，网关端点见 [路由契约](../docs/006-ref/003-ROUTES.md "RE-003-ROUTES") §Gateway 插件。

## 文档

- 前端架构与机制：`docs/023-FRONTEND.md`
- 交互语义（屏幕 / 动作 / 状态 / 流式时间轴）：`docs/024-INTERACTION.md`
- 后端总览：`../README.md`、`../docs/005-arch/005-SYSTEM_MAP.md`
