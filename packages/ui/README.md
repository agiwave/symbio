---
kind: guide
id: packages/ui/README
title: Symbio 共享 Vue 前端
status: active
created: 2026-11-28
---

# Symbio 共享 Vue 前端

`symbio-cli serve`（浏览器内嵌 UI）与 `apps/web`（纯浏览器交付物）共用的 Vue 3 实现。
三端差异只在 transport 注入：desktop 走 Tauri IPC，cli serve 走同源 HTTP，web 走
browser HTTP / 离线兜底。

- `src/` — 三端共享的 Vue 源码（视图、组件、store、services）
- `dist/` — 唯一构建产物；Tauri `generate_context!`、web 静态部署与 CLI 编译期内嵌都读这一份
- `docs/` — 共享前端文档：`110-FRONTEND.md`（架构与机制）、`111-INTERACTION.md`（交互语义）

本目录是前端事实源；交付物壳（`apps/desktop`、`apps/web`、`apps/cli`）只做适配与入口装配，
不维护独立的前端源码、产物或文档。

## 开发

```
npm ci
npm run dev        # Vite dev server（默认 1420 端口）
npm run build      # 产出 dist/，三端共用
npm test           # Vitest
npm run typecheck
npm run lint
```

`apps/desktop` 与 `apps/web` 的 npm 脚本均委托到本目录，不在各自目录安装依赖。
