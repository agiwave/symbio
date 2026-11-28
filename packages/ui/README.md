---
kind: guide
id: packages/ui/README
title: Symbio 共享 Vue 前端
status: active
created: 2026-11-28
---

# Symbio 共享 Vue 前端

桌面启动器、`symbio-cli serve` 与 `apps/web` 三端共用的 Vue 3 实现。三端跑在
**同一套普通浏览器环境**里，transport 完全一致：同源或远端 Server 的 HTTP/WS；
web 端在无后端时读类走离线兜底。桌面端由启动器拉起本机 CLI serve，浏览器同源访问本机后端。

- `src/` — 三端共享的 Vue 源码（视图、组件、store、services）
- `dist/` — 唯一构建产物；web 静态部署与 CLI 编译期内嵌都读这一份；桌面启动器拉起的 CLI serve 提供同一份产物
- `docs/` — 共享前端文档：`110-FRONTEND.md`（架构与机制）、`111-INTERACTION.md`（交互语义）

本目录是前端事实源；交付物壳（`apps/desktop`、`apps/web`、`apps/cli`）只做适配与入口装配，
不维护独立的前端源码、产物或文档。

## 开发

```
npm ci
npm run dev        # Vite dev server（1420 端口，带 HMR 热更新）
npm run build      # 产出 dist/，三端共用
npm test           # Vitest
npm run typecheck
npm run lint
```

### 原生窗口开发（HMR）：`npm run tauri dev`

在 `apps/desktop` 下执行（需先 `npm install` 装 Tauri CLI）：

```bash
cd apps/desktop && npm install
npm run tauri dev
```

Tauri 会起 Vite dev server（1420，HMR），拉起 CLI serve（9231），并打开原生窗口加载 1420；
`vite.config.ts` 把 `/api`（含 WS）代理到 `SYMBIO_SERVER_PORT`（缺省 9231）。
改 Vue 即时热更新，后端逻辑改动需重启 CLI（Rust 无 HMR）。

### 纯浏览器开发（无原生窗口）

前端热更新与后端是两条独立进程，开两个终端：

```bash
# 终端 A：前端（改 .vue 即时热更新，浏览器打开 http://localhost:1420）
npm run dev

# 终端 B：后端 Server（默认 127.0.0.1:9231）
cargo run -p symbio-cli -- serve
```

浏览器地址是 `http://localhost:1420`，同样通过 Vite 代理走 `/api`。

`apps/desktop` 与 `apps/web` 的 npm 脚本均委托到本目录，不在各自目录安装依赖。
