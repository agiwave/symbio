---
kind: design
id: apps/desktop/README
title: Tauri 前端
status: active
created: 2026-09-09
impl: done
---

# Tauri 前端

Symbio 桌面前端：Vue 3 + TypeScript + Tauri 2。

## 结构

```
apps/desktop/
├── src-tauri/   # Tauri 宿主（Rust）：只有 `route_v2` 家族 command
└── package.json # 薄壳脚本：Vue 开发/构建委托 packages/ui，tauri 用本目录 src-tauri
```

Vue 源码、文档与唯一构建产物都在 `packages/ui/`：`packages/ui/src` 是三端共享的实现，
`packages/ui/dist` 同时供 Tauri `generate_context!`、`apps/web` 静态部署与 CLI 编译期内嵌，
`packages/ui/docs/` 是三端共享的前端架构与交互语义文档。本目录只保留 Tauri 壳与委托脚本。

## 宿主命令（src-tauri）

`src-tauri/` 只有 `route_v2` 家族的命令——宿主不含业务逻辑，请求一律直通插件树；
`gateway` 插件的 HTTP 入口是同一件事的另一条通道。

本文不维护命令表：命令面由 [当前事实表](../../docs/004-ref/006-当前事实表.md "RE-006-当前事实表") §5.2 从代码投影，
逐条职责见 `packages/ui/docs/110-FRONTEND.md`，网关端点见 [路由契约](../../docs/004-ref/003-ROUTES.md "RE-003-ROUTES") §Gateway 插件。

## 文档

前端文档属于三端共享的 Vue 实现，owner 在 `packages/ui/docs/`：

- 前端架构与机制：`packages/ui/docs/110-FRONTEND.md`
- 交互语义（屏幕 / 动作 / 状态 / 流式时间轴）：`packages/ui/docs/111-INTERACTION.md`
- 后端总览：`../README.md`、`../docs/003-design/004-系统地图.md`
