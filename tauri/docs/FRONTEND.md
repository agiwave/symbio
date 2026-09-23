# 前端架构

> 前端 UI/UX 需求与设计详见 `docs/design/frontend-ui-ux-prd.md` 与 `docs/design/frontend-ui-ux-design.md`（根 docs/）。本文只写前端代码结构与机制。

## 技术栈

- Vue 3（Composition API）+ TypeScript + Vite
- Tauri 2 桌面宿主

## 与后端通信

前端不直接感知 14 个插件，只通过 Tauri command 与插件树对话：

```
Vue 组件 → API 层 (invoke) → route_v2 / route_v2_send / route_v2_close → 插件树
```

- `route_v2`：通用同步调用（PATH/PAYLOAD/SESSION_ID），等价 gateway 的 `POST /api/route`。
- `route_v2_send`：向活动会话发送用户输入，接收流式帧。
- `route_v2_close`：关闭会话连接。

## 关键约定

- **发言是入队，不是立即上屏**：发送只是往会话收件箱写一条（后端 `<sid>/inbox`），消息进入消息列表的时机由**后端消费**决定（落库后发权威帧）。因此前端不做乐观回显——"已发出"与"已处理"在界面上必须可区分；等待期由 working 状态 + 等待骨架（`sessionLive.needsTypingRow`）表达。
- **流式渲染**：会话视图按帧 `kind` 增量渲染（文本增量、工具调用、状态帧）。
- **会话中断可见性**：遵循"存储层全量 / 会话层不过滤 Failed / 请求视图层附加中断说明"三层分工（见 `../symbio/src/plugins/session/README.md` 与 `../symbio/src/plugins/session/docs/turn-tool-mechanisms.md`）。
- **VDFS 驱动**：Agent / Model / MCP 等一切资源的管理界面都只有一个协议、一套形状 —— 虚拟文件系统（`vdfs/*`，地址 `<根>/<插件名>/…`）。列表项与详情同为 `VdfsNode`，变更事件只有 `kind = 'vdfs'` 一条频道；规范见 `docs/design/vdfs.md`。
