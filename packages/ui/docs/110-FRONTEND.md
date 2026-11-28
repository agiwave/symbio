---
kind: design
id: DE-110-FRONTEND
title: 前端架构
status: active
created: 2026-09-09
impl: done
---

# 前端架构

> 本文是**三端共享 Vue 实现**（desktop / cli serve / web）的前端架构 owner，住在
> `packages/ui/docs/`，不挂在某一个交付物壳下面。
> 前端 UI/UX 需求与设计方案详见 `docs/003-design/020-前端UI-UX.md`（根 docs/）。
> 用户视角的交互语义（屏幕、动作、反馈、流式时间轴）见 `packages/ui/docs/111-INTERACTION.md`。
> 本文只写前端代码结构与机制。

## 分层的边界：薄的是决策层，不是适配层

「薄前端」在本仓指**业务决策与事实源都不在这一层**：标题、图标、可用动作、统计量由后端随
`vdfs/node` 下发，前端只按 `NodeKind` 登记表渲染。它**不**指「TS 只做渲染」——传输选择
（http / ws / gateway）、帧合并、重连与重同步、协议归一、地址口径换算都在
`packages/ui/src/services/` 与 `packages/ui/src/composables/`；这些文件的规模**不在此复述**，唯一权威是
[当前事实表 §5.1](../../../docs/004-ref/006-当前事实表.md "RE-006-当前事实表")（`packages/ui/src` 实现与测试分列），
会随代码变化由脚本重算。`packages/ui/src/services/` 与 `packages/ui/src/composables/` 只承担协议适配、状态桥接与展示编排，
是前端分层允许的例外，不是业务逻辑泄漏；反过来，看到 `stores/sessions.ts` 长就断言「业务跑前端了」也是读错层。

## 技术栈

- Vue 3（Composition API）+ TypeScript + Vite
- 普通浏览器运行时（三端统一，无 WebView / IPC 分支）

## 与后端通信

Vue 实现只有一份（`packages/ui/src`），三端 transport 完全一致：

- desktop：启动器拉起本机 `symbio-cli serve --browser`，浏览器同源 HTTP → CLI 进程内插件树，UI 由二进制编译期内嵌；
- cli serve：同源 HTTP → `/api/v1/invoke` → CLI 进程内插件树，UI 默认二进制内嵌（`--frontend-dist` 可覆盖）；
- web：browser 环境默认无后端，读类走 `packages/ui/src/services/fallback.ts` 兜底；显式配置后才经 HTTP 连 `symbio-cli serve`。

前端不直接感知插件树的具体形状；走到后端时，三条通道装的是同一份请求：

```
Vue 组件 → API 层（browser fetch / WebSocket） → HTTP/WS 入口 → 网关路由 → 插件树
```

一次性调用走 `POST /api/v1/invoke`，持久会话走 `WS /api/v1/ws`（首帧为 PluginMessageWire，
其后双向转发 PluginFrame）。两路与 gateway 的路由入口等价（端点见
[003-ROUTES.md §Gateway 插件](../../../docs/004-ref/003-ROUTES.md "RE-003-ROUTES")）。

## 关键约定

- **发言是入队，不是立即上屏**：发送只是往会话收件箱写一条（后端 `<sid>/inbox`），消息进入消息列表的时机由**后端消费**决定（落库后发权威帧）。因此前端不做乐观回显——"已发出"与"已处理"在界面上必须可区分；等待期由 working 状态 + 等待骨架（`sessionLive.needsTypingRow`）表达。
- **流式渲染**：会话视图按帧 `kind` 增量渲染（文本增量、工具调用、状态帧）。
- **会话中断可见性**：遵循"存储层全量 / 会话层不过滤 Failed / 请求视图层附加中断说明"三层分工（见 `../crates/symbio-plugin-session/README.md` 与 `../crates/symbio-plugin-session/src/docs/108-turn-tool-mechanisms.md`）。
- **VDFS 驱动**：Agent / Model / MCP 等一切资源的管理界面都只有一个协议、一套形状 —— 虚拟文件系统（`vdfs/*`，地址 `<根>/<插件名>/…`）。列表项与详情同为 `VdfsNode`，变更事件只有 `kind = 'vdfs'` 一条频道；规范见 `docs/003-design/009-VDFS规范.md`。
