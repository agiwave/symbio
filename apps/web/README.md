---
kind: guide
id: apps/web/README
title: Symbio Web 交付物
status: active
created: 2026-11-28
---

# Symbio Web 交付物

纯浏览器运行的 Symbio 前端。与桌面端、CLI serve 共用 [`packages/ui`](../../packages/ui) 的 Vue 实现，
三者的差异只在 transport 注入：

- **desktop**（`apps/desktop`）：轻量启动器拉起 CLI serve，浏览器同源 HTTP 连本机后端；
- **cli serve**（`apps/cli`）：同源 HTTP 连本机 CLI 后端；
- **web**（本包）：browser 环境，可选经 HTTP 连接远端 `symbio-cli serve`；无后端时读类走
  `packages/ui/src/services/fallback.ts` 的兜底，页面仍可交互。

## 构建

```
npm run build   # 产物在 packages/ui/dist，纯静态可部署
```

Vue 实现不在这里维护：改界面去 `packages/ui/src`。
