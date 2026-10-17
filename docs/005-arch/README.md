---
kind: arch
id: AR-000-架构汇总
title: 架构总览
status: active
projection: true
lists: [arch-entries=owner]
created: 2026-10-14
prefix: AR
label: 架构
desc: 现在是什么样——系统结构 / 设计规范 / 模块机制
---

# 架构总览

架构描述「现在是什么样」：系统结构、模块机制、设计规范。结构事实的自动投影另有 RE-006 当前事实表（由代码生成）——这里只放人写的机制说明。

<!-- registry:summary begin -->
<!-- list:arch-entries owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](../008-spec/001.md)，机制见 [PL-011 · 登记与汇总机制](../004-plan/011-登记与汇总机制.md "PL-011-登记与汇总机制")。

`arch`（架构）的条目**在位存放**：文件本身就是登记，`title` 逐字等于正文首个 H1（RG-021 两面对账）。

**计数**：active 48 · draft 0 · retired 0（共 48）

## 现行文档（active）

| 编号 | 标题 |
|---|---|
| [`AR-001-architecture`](../../cli/docs/001-architecture.md) | 架构 |
| [`AR-002-DATA_FLOW`](002-DATA_FLOW.md) | 数据流与调用链（DATA_FLOW） |
| [`AR-003-OVERVIEW`](003-OVERVIEW.md) | Symbio 架构总览 |
| [`AR-004-PROTOCOLS`](004-PROTOCOLS.md) | Symbio 协议规范 |
| [`AR-005-SYSTEM_MAP`](005-SYSTEM_MAP.md) | Symbio 系统地图 |
| [`AR-006-agent-directory-spec`](006-agent-directory-spec.md) | Agent 目录规范 v2 |
| [`AR-007-frontend-ui-ux-plan`](007-frontend-ui-ux-plan.md) | 前端 UI/UX 提升规划（提案） |
| [`AR-008-http-api-transport`](008-http-api-transport.md) | HTTP(S) 开放接口插件设计 |
| [`AR-009-plugin-route-address`](009-plugin-route-address.md) | 插件路由地址规则 |
| [`AR-010-session-input-addressing`](010-session-input-addressing.md) | 会话输入改走地址：`<会话地址>/inbox` + 三个动作 |
| [`AR-011-third-party-plugin-spec`](011-third-party-plugin-spec.md) | 第三方插件体系与规范 |
| [`AR-012-vdfs-frontend`](012-vdfs-frontend.md) | VDFS 前端页面规范 |
| [`AR-013-vdfs`](013-vdfs.md) | VDFS —— 虚拟动态文件系统规范 |
| [`AR-014-context-compression-design`](../../symbio/src/plugins/session/docs/014-context-compression-design.md) | 会话上下文压缩设计 |
| [`AR-015-core-loop`](../../symbio/src/plugins/session/docs/015-core-loop.md) | Session 核心循环（LLM × 工具） |
| [`AR-016-heartbeat-mechanism`](../../symbio/src/plugins/session/docs/016-heartbeat-mechanism.md) | 会话心跳机制（heartbeat） |
| [`AR-016-turn-runner-steps`](../../symbio/src/plugins/session/docs/016-turn-runner-steps.md) | 016 · `run_with_tools` 的 Step 施工图（PL-014 步骤 1） |
| [`AR-017-module-layout`](../../symbio/src/plugins/session/docs/017-module-layout.md) | Session 插件模块分工 |
| [`AR-018-node-state-streaming`](../../symbio/src/plugins/session/docs/018-node-state-streaming.md) | 节点状态流（Node-State Streaming）—— 实时面即节点状态 |
| [`AR-019-perf`](../../symbio/src/plugins/session/docs/019-perf.md) | 会话 / 转写的前端性能设计 |
| [`AR-020-session-options`](../../symbio/src/plugins/session/docs/020-session-options.md) | 会话选项 = 会话配置表单 |
| [`AR-021-turn-tool-mechanisms`](../../symbio/src/plugins/session/docs/021-turn-tool-mechanisms.md) | 工具失败回传与 Turn 终态机制 |
| [`AR-022-vdfs-session-messages`](../../symbio/src/plugins/session/docs/022-vdfs-session-messages.md) | 会话消息的 VDFS 化 —— 转写即列表，流式即追加 |
| [`AR-023-FRONTEND`](../../tauri/docs/023-FRONTEND.md) | 前端架构 |
| [`AR-024-INTERACTION`](../../tauri/docs/024-INTERACTION.md) | 前端交互面（用户视角） |
| [`scripts/facts/README`](../../scripts/facts/README.md) | `scripts/facts/` —— 事实生成器目录 |
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
| [`symbio/src/plugins/setting/README`](../../symbio/src/plugins/setting/README.md) | Setting 插件 |
| [`symbio/src/plugins/skill/README`](../../symbio/src/plugins/skill/README.md) | Skill 插件 |
| [`symbio/src/plugins/telegram/README`](../../symbio/src/plugins/telegram/README.md) | Telegram 插件 |
| [`symbio/src/plugins/vdfs/README`](../../symbio/src/plugins/vdfs/README.md) | VDFS 插件 |
| [`symbio/src/plugins/web/README`](../../symbio/src/plugins/web/README.md) | Web 插件 |
| [`symbio/src/symbio_core/README`](../../symbio/src/symbio_core/README.md) | symbio_core —— 内核契约层：命名与结构规范 |
| [`tauri/README`](../../tauri/README.md) | Tauri 前端 |

## 草案（draft）

（无）

## 废止（retired）

（无）

<!-- registry:summary end -->

