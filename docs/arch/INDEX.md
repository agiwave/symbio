# 架构登记索引（自动生成）

> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 生成，请勿手改。**
> `arch`（架构）的条目**在位存放**：文件本身就是登记，编号 = 仓根相对路径（去 `.md`），
> `title` 逐字等于正文首个 H1（RG-021 两面对账）；本页是投影。字段规范见 [SP-001](../spec/SP-001.md)，
> 机制见 [14 · 登记与汇总机制](../plan/14-登记与汇总机制.md)。

> **这个 kind 回答什么**：现在是什么样——系统结构 / 设计规范 / 模块机制

**计数**：active 46 · draft 0 · retired 0（共 46）

## 现行文档（active）
| 编号 | 标题 |
|---|---|
| [`cli/docs/architecture`](../../cli/docs/architecture.md) | 架构 |
| [`docs/arch/DATA_FLOW`](DATA_FLOW.md) | 数据流与调用链（DATA_FLOW） |
| [`docs/arch/OVERVIEW`](OVERVIEW.md) | Symbio 架构总览 |
| [`docs/arch/PROTOCOLS`](PROTOCOLS.md) | Symbio 协议规范 |
| [`docs/arch/SYSTEM_MAP`](SYSTEM_MAP.md) | Symbio 系统地图 |
| [`docs/arch/agent-directory-spec`](agent-directory-spec.md) | Agent 目录规范 v2 |
| [`docs/arch/frontend-ui-ux-plan`](frontend-ui-ux-plan.md) | 前端 UI/UX 提升规划（提案） |
| [`docs/arch/http-api-transport`](http-api-transport.md) | HTTP(S) 开放接口插件设计 |
| [`docs/arch/plugin-route-address`](plugin-route-address.md) | 插件路由地址规则 |
| [`docs/arch/session-input-addressing`](session-input-addressing.md) | 会话输入改走地址：`<会话地址>/inbox` + 三个动作 |
| [`docs/arch/third-party-plugin-spec`](third-party-plugin-spec.md) | 第三方插件体系与规范 |
| [`docs/arch/vdfs`](vdfs.md) | VDFS —— 虚拟动态文件系统规范 |
| [`docs/arch/vdfs-frontend`](vdfs-frontend.md) | VDFS 前端页面规范 |
| [`symbio/src/plugins/agent/README`](../../symbio/src/plugins/agent/README.md) | Agent 插件 |
| [`symbio/src/plugins/classify/README`](../../symbio/src/plugins/classify/README.md) | Classify 插件（对话面：判决） |
| [`symbio/src/plugins/compose/README`](../../symbio/src/plugins/compose/README.md) | Compose 插件（对话面：措辞） |
| [`symbio/src/plugins/composite/README`](../../symbio/src/plugins/composite/README.md) | Composite 插件 |
| [`symbio/src/plugins/event_bus/README`](../../symbio/src/plugins/event_bus/README.md) | Event Bus 插件 |
| [`symbio/src/plugins/gateway/README`](../../symbio/src/plugins/gateway/README.md) | Gateway 插件 |
| [`symbio/src/plugins/home/README`](../../symbio/src/plugins/home/README.md) | Home 插件 |
| [`symbio/src/plugins/hook/README`](../../symbio/src/plugins/hook/README.md) | Hook 插件 |
| [`symbio/src/plugins/local/README`](../../symbio/src/plugins/local/README.md) | Local 插件 |
| [`symbio/src/plugins/mcp/README`](../../symbio/src/plugins/mcp/README.md) | MCP 插件 |
| [`symbio/src/plugins/memory/README`](../../symbio/src/plugins/memory/README.md) | Memory 插件 |
| [`symbio/src/plugins/model/README`](../../symbio/src/plugins/model/README.md) | Model 插件 |
| [`symbio/src/plugins/plugin_manager/README`](../../symbio/src/plugins/plugin_manager/README.md) | Plugin_manager 插件（插件管理） |
| [`symbio/src/plugins/session/README`](../../symbio/src/plugins/session/README.md) | Unified Session & Memory Orchestration Architecture (会话与记忆系统化管理架构说明书) |
| [`symbio/src/plugins/session/docs/README`](../../symbio/src/plugins/session/docs/README.md) | Session 插件文档 |
| [`symbio/src/plugins/session/docs/context-compression-design`](../../symbio/src/plugins/session/docs/context-compression-design.md) | 会话上下文压缩设计 |
| [`symbio/src/plugins/session/docs/core-loop`](../../symbio/src/plugins/session/docs/core-loop.md) | Session 核心循环（LLM × 工具） |
| [`symbio/src/plugins/session/docs/heartbeat-mechanism`](../../symbio/src/plugins/session/docs/heartbeat-mechanism.md) | 会话心跳机制（heartbeat） |
| [`symbio/src/plugins/session/docs/module-layout`](../../symbio/src/plugins/session/docs/module-layout.md) | Session 插件模块分工 |
| [`symbio/src/plugins/session/docs/node-state-streaming`](../../symbio/src/plugins/session/docs/node-state-streaming.md) | 节点状态流（Node-State Streaming）—— 实时面即节点状态 |
| [`symbio/src/plugins/session/docs/perf`](../../symbio/src/plugins/session/docs/perf.md) | 会话 / 转写的前端性能设计 |
| [`symbio/src/plugins/session/docs/session-options`](../../symbio/src/plugins/session/docs/session-options.md) | 会话选项 = 会话配置表单 |
| [`symbio/src/plugins/session/docs/turn-tool-mechanisms`](../../symbio/src/plugins/session/docs/turn-tool-mechanisms.md) | 工具失败回传与 Turn 终态机制 |
| [`symbio/src/plugins/session/docs/vdfs-session-messages`](../../symbio/src/plugins/session/docs/vdfs-session-messages.md) | 会话消息的 VDFS 化 —— 转写即列表，流式即追加 |
| [`symbio/src/plugins/setting/README`](../../symbio/src/plugins/setting/README.md) | Setting 插件 |
| [`symbio/src/plugins/skill/README`](../../symbio/src/plugins/skill/README.md) | Skill 插件 |
| [`symbio/src/plugins/telegram/README`](../../symbio/src/plugins/telegram/README.md) | Telegram 插件 |
| [`symbio/src/plugins/vdfs/README`](../../symbio/src/plugins/vdfs/README.md) | VDFS 插件 |
| [`symbio/src/plugins/web/README`](../../symbio/src/plugins/web/README.md) | Web 插件 |
| [`symbio/src/symbio_core/README`](../../symbio/src/symbio_core/README.md) | symbio_core —— 内核契约层：命名与结构规范 |
| [`tauri/README`](../../tauri/README.md) | Tauri 前端 |
| [`tauri/docs/FRONTEND`](../../tauri/docs/FRONTEND.md) | 前端架构 |
| [`tauri/docs/INTERACTION`](../../tauri/docs/INTERACTION.md) | 前端交互面（用户视角） |

## 草案（draft）
（无）

## 废止（retired）
（无）
