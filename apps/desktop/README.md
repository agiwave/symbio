---
kind: design
id: apps/desktop/README
title: 桌面交付物（Tauri 原生窗口）
status: active
created: 2026-09-09
impl: done
---

# Symbio 桌面交付物

桌面端是 **Tauri 原生窗口**（`tao` 窗口 + `wry` WebView），窗口加载本机
`symbio-cli serve` 提供的 UI 地址。后端 HTTP/WS、插件树、UI 内嵌全在 CLI 二进制里；
桌面、CLI serve 与 web 因此共享**同一条 HTTP/WS 传输路径**，没有只有桌面端才走的 IPC 层。

## 结构

```
apps/desktop/
├── Cargo.toml         # symbio-desktop：Tauri 原生窗口壳（不依赖插件树、无 IPC command）
├── build.rs           # tauri_build::build()
├── tauri.conf.json    # 窗口尺寸/图标；无 frontendDist（UI 来自 CLI serve）
├── capabilities/      # 最小权限：core:default（前端不需要 dialog/fs 权限）
├── icons/             # 窗口与打包图标
├── src/main.rs        # 拉起 CLI serve → 等 health 就绪 → 窗口加载本机地址 → 关窗时终止 CLI
├── package.json       # tauri CLI + Vue 开发/构建脚本
└── README.md
```

Vue 源码、文档与唯一构建产物都在 `packages/ui/`：`packages/ui/src` 是三端共享的实现，
`packages/ui/dist` 由 CLI 在编译期内嵌进二进制，同时供 `apps/web` 静态部署。
`packages/ui/docs/` 是三端共享的前端架构与交互语义文档。

## 运行与开发

```bash
# 1) 安装依赖（Tauri CLI 只在本壳层声明）
cd apps/desktop && npm install

# 2) 原生窗口 + 前端热更新（等价旧 tauri dev）：
#    Tauri 起 Vite（1420，HMR），窗口加载 1420，/api 由 Vite 代理到 CLI serve(9231)
npm run tauri dev

# 3) 直接运行（不依赖 Vite）：窗口加载 CLI 内嵌 UI
cargo run -p symbio-desktop
```

`npm run tauri dev` 时 Tauri CLI 设 `cfg(dev)`，窗口加载 Vite dev server，改 Vue 即时热更新；
直接运行二进制或 release 构建则加载 CLI serve 的本机地址（UI 来自 `packages/ui/dist` 的内嵌副本）。
`SYMBIO_DESKTOP_URL` 可显式覆盖窗口加载地址。

## 运行时行为

- **系统目录**：`SYMBIO_HOMEDIR` 显式指定优先；否则落在平台用户配置目录下的 `symbio/`
  （如 `%APPDATA%\symbio`、`~/.config/symbio`），不会写进用户当前工作目录。
- **CLI 二进制**：`SYMBIO_CLI` 显式指定优先；否则取同目录的 `symbio-cli`
  （Cargo workspace 把两者产物都放在同一个 `target/<profile>/`），再回落 PATH。
- **网络**：`serve` 默认只监听回环 `127.0.0.1`，窗口同源 HTTP 访问本机后端；
  全程不依赖外网，单机断网可用。
- **退出**：窗口销毁（`WindowEvent::Destroyed`）时终止 CLI 子进程，不留孤儿后端。

## 为什么窗口加载 HTTP 而不是 IPC

旧 Tauri 用 `invoke("route_v2")` 三兄弟经 IPC 调后端，那套分支只有桌面端走、难测试，
且要求桌面端自己嵌一份 UI。现在窗口只是「原生壳 + WebView 容器」：
- 前端统一 HTTP/WS（`packages/ui/src/services/plugin.ts`），三端完全同构；
- UI 由 CLI 编译期内嵌，桌面端不嵌第二份；
- 目录选择走后端 `/api/v1/host/directories`、文件选择走 `<input type="file">`，不依赖 dialog 插件。

代价是原生窗口的固有成本：系统 WebView 依赖（Windows WebView2、Linux WebKitGTK/GTK），
CI 需装对应前置；但这比旧 Tauri 轻——没有 IPC command、没有构建戳、没有权限模型配置。

## 文档

前端文档属于三端共享的 Vue 实现，owner 在 `packages/ui/docs/`：

- 前端架构与机制：`packages/ui/docs/110-FRONTEND.md`
- 交互语义（屏幕 / 动作 / 状态 / 流式时间轴）：`packages/ui/docs/111-INTERACTION.md`
- 后端总览：`../README.md`、`../../docs/003-design/004-系统地图.md`
