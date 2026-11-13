---
kind: design
id: DE-110-FRONTEND
title: 前端架构
status: active
created: 2026-09-09
impl: done
---

# 前端架构

> 前端 UI/UX 需求与设计方案详见 `docs/003-design/020-前端UI-UX.md`（根 docs/）。
> 用户视角的交互语义（屏幕、动作、反馈、流式时间轴）见 `tauri/docs/111-INTERACTION.md`。
> 本文只写前端代码结构与机制。

## 分层的边界：薄的是决策层，不是适配层

「薄前端」在本仓指**业务决策与事实源都不在这一层**：标题、图标、可用动作、统计量由后端随
`vdfs/node` 下发，前端只按 `NodeKind` 登记表渲染。它**不**指「TS 只做渲染」——传输选择
（native / http / gateway）、帧合并、重连与重同步、协议归一、地址口径换算都在
`src/services/` 与 `src/composables/`，合计约 2.7 万行（规模以 `docs/004-ref/006-当前事实表.md`
§5.1 为准）。这些有 `D-015` / `D-070` / `DE-020 §10-11` 的裁决，属**有裁决的例外**，不是业务
逻辑泄漏；反过来，看到 `stores/sessions.ts` 长就断言「业务跑前端了」也是读错层。

## 技术栈

- Vue 3（Composition API）+ TypeScript + Vite
- Tauri 2 桌面宿主

## 与后端通信

前端不直接感知插件树的具体形状，只通过 Tauri command 与插件树对话：

```
Vue 组件 → API 层 (invoke) → route_v2 家族 → 插件树
```

- `route_v2`：通用同步调用（PATH/PAYLOAD/SESSION_ID），与 gateway 的 HTTP 入口等价（端点见 [003-ROUTES.md §Gateway 插件](../../docs/004-ref/003-ROUTES.md "RE-003-ROUTES")）。
- `route_v2_send`：向活动会话发送用户输入，接收流式帧。
- `route_v2_close`：关闭会话连接。

## 关键约定

- **发言是入队，不是立即上屏**：发送只是往会话收件箱写一条（后端 `<sid>/inbox`），消息进入消息列表的时机由**后端消费**决定（落库后发权威帧）。因此前端不做乐观回显——"已发出"与"已处理"在界面上必须可区分；等待期由 working 状态 + 等待骨架（`sessionLive.needsTypingRow`）表达。
- **流式渲染**：会话视图按帧 `kind` 增量渲染（文本增量、工具调用、状态帧）。
- **会话中断可见性**：遵循"存储层全量 / 会话层不过滤 Failed / 请求视图层附加中断说明"三层分工（见 `../crates/symbio-plugin-session/README.md` 与 `../crates/symbio-plugin-session/src/docs/108-turn-tool-mechanisms.md`）。
- **VDFS 驱动**：Agent / Model / MCP 等一切资源的管理界面都只有一个协议、一套形状 —— 虚拟文件系统（`vdfs/*`，地址 `<根>/<插件名>/…`）。列表项与详情同为 `VdfsNode`，变更事件只有 `kind = 'vdfs'` 一条频道；规范见 `docs/003-design/009-VDFS规范.md`。
