---
kind: design
id: DE-000-设计总览
title: 设计总览
status: active
projection: true
lists: [design-entries=owner]
created: 2026-10-14
prefix: DE
label: 设计
desc: 产品与系统怎么设计——现状、目标形态与关键取舍（每篇的时态由 impl 标注）

titleEqualsH1: false
required: [impl]
optional: [reason, supersedes]
closedStatus: active
requiresDepsDone: [active]
retiredStates: [retired]
status.draft:
status.active:
status.retired.require: [reason]
impl.done: 与代码一致
impl.partial: 部分落地
impl.planned: 目标形态
impl.none: 未落地
summary.order: [active, draft, retired]
summary.active.title: 现行设计（active）
summary.active.columns: [id, title:设计, impl]
summary.draft.title: 草案（draft）
summary.draft.columns: [id, title:设计, impl]
summary.retired.title: 废止（retired）
summary.retired.columns: [id, title:设计, reason]
board.columns: [impl]
---

# 设计总览

设计文档回答「产品与系统怎么设计」：**现状机制**（代码此刻的样子）、**目标形态**（要变成的样子）与**关键取舍**（为什么不是另一种）。三者写在**同一篇**里——同一件事不分散在两处。

- **时态在 `impl`，不在目录**：`done` = 与代码一致；`partial` = 已落一部分（残余在正文写明）；`planned` = 目标形态，尚未落地。落地只是把 `impl` 改一格，文档原地不动。
- **取舍跟着主题走**：架构决策（ADR）折进所属设计的「关键决策」节，编号 `D-NNN` 保留为锚点；已退场或主题已消失的决策不迁移（`git log` 是历史）。
- **编号**：`0xx` 系统级设计（跨模块的机制与契约），`1xx` 组件级设计（就近放在该组件目录内）。

<!-- registry:summary begin -->
<!-- list:design-entries owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](../005-spec/001.md)，机制见 [DE-017 · 登记与汇总机制](017-登记与汇总机制.md "DE-017-登记与汇总机制")。

**计数**：active 59 · draft 0 · retired 0（共 59）

## 现行设计（active）

| 编号 | 设计 | 落地 |
|---|---|---|
| `DE-001-核心架构` | 核心架构：原语、Store、Projection、Actor | 与代码一致 |
| `DE-002-架构总览` | 架构总览与目标形态 | 部分落地 |
| `DE-003-数据流与调用链` | 数据流与调用链（DATA_FLOW） | 与代码一致 |
| `DE-004-系统地图` | Symbio 系统地图 | 与代码一致 |
| `DE-005-协议规范` | Symbio 协议规范 | 与代码一致 |
| `DE-006-插件路由与地址` | 插件路由地址规则 | 与代码一致 |
| `DE-007-第三方插件体系` | 第三方插件体系与规范 | 与代码一致 |
| `DE-008-能力注册表与能力坐标系` | 能力注册表与能力坐标系 | 与代码一致 |
| `DE-009-VDFS规范` | VDFS —— 虚拟动态文件系统规范 | 与代码一致 |
| `DE-010-VDFS前端页面规范` | VDFS 前端页面规范 | 与代码一致 |
| `DE-011-Agent目录规范` | Agent 目录规范 v2 | 部分落地 |
| `DE-012-会话输入与地址` | 会话输入改走地址：`<会话地址>/inbox` + 三个动作 | 与代码一致 |
| `DE-013-HTTP开放接口` | HTTP(S) 开放接口插件设计 | 与代码一致 |
| `DE-014-单环一表` | 单环一表收敛 | 与代码一致 |
| `DE-015-义务环` | 义务环与端口细粒度 | 部分落地 |
| `DE-016-工程落地与验证` | 工程落地与验证 | 与代码一致 |
| `DE-017-登记与汇总机制` | 登记与汇总机制 | 与代码一致 |
| `DE-018-模块架构` | 模块架构（施工视图） | 与代码一致 |
| `DE-019-前端机制化` | 前端机制化 | 部分落地 |
| `DE-020-前端UI-UX` | 前端 UI/UX 设计 | 目标形态 |
| `DE-021-横向扩充与插件切分` | 横向扩充与大模块拆分 | 部分落地 |
| `DE-101-context-compression-design` | 会话上下文压缩设计 | 与代码一致 |
| `DE-102-core-loop` | 会话核心循环（LLM × 工具） | 与代码一致 |
| `DE-103-turn-runner-steps` | 016 · `run_with_tools` 的 Step 施工图（DE-014 步骤 1） | 与代码一致 |
| `DE-104-module-layout` | Session / Loop 插件模块分工 | 与代码一致 |
| `DE-105-node-state-streaming` | 节点状态流（Node-State Streaming）—— 实时面即节点状态 | 与代码一致 |
| `DE-106-perf` | 会话 / 转写的前端性能设计 | 与代码一致 |
| `DE-107-session-options` | 会话选项 = 会话配置表单 | 与代码一致 |
| `DE-108-turn-tool-mechanisms` | 工具失败回传与 Turn 终态机制 | 与代码一致 |
| `DE-109-vdfs-session-messages` | 会话消息的 VDFS 化 —— 转写即列表，流式即追加 | 与代码一致 |
| `DE-110-FRONTEND` | 前端架构 | 与代码一致 |
| `DE-111-INTERACTION` | 前端交互面（用户视角） | 与代码一致 |
| `DE-112-architecture` | 架构 | 与代码一致 |
| `DE-113-heartbeat-mechanism` | 会话心跳机制（heartbeat） | 与代码一致 |
| `scripts/facts/README` | ''`scripts/facts/` —— 事实生成器目录'' | 与代码一致 |
| `symbio/src/plugins/agent/README` | Agent 插件 | 与代码一致 |
| `symbio/src/plugins/classify/README` | Classify 插件（对话面：判决） | 与代码一致 |
| `symbio/src/plugins/compose/README` | Compose 插件（对话面：措辞） | 与代码一致 |
| `symbio/src/plugins/composite/README` | Composite 插件 | 与代码一致 |
| `symbio/src/plugins/event_bus/README` | Event Bus 插件 | 与代码一致 |
| `symbio/src/plugins/gateway/README` | Gateway 插件 | 与代码一致 |
| `symbio/src/plugins/heartbeat/README` | Heartbeat 插件（空闲会话的自主开轮调度） | 与代码一致 |
| `symbio/src/plugins/home/README` | Home 插件 | 与代码一致 |
| `symbio/src/plugins/hook/README` | Hook 插件 | 与代码一致 |
| `symbio/src/plugins/local/README` | Local 插件 | 与代码一致 |
| `symbio/src/plugins/loop/README` | Loop 插件（单环驱动器） | 与代码一致 |
| `symbio/src/plugins/mcp/README` | MCP 插件 | 与代码一致 |
| `symbio/src/plugins/memory/README` | Memory 插件 | 与代码一致 |
| `symbio/src/plugins/model/README` | Model 插件 | 与代码一致 |
| `symbio/src/plugins/plugin_manager/README` | Plugin_manager 插件（插件管理） | 与代码一致 |
| `symbio/src/plugins/session/README` | Unified Session & Memory Orchestration Architecture (会话与记忆系统化管理架构说明书) | 与代码一致 |
| `symbio/src/plugins/session/docs/README` | Session 插件文档 | 与代码一致 |
| `symbio/src/plugins/setting/README` | Setting 插件 | 与代码一致 |
| `symbio/src/plugins/skill/README` | Skill 插件 | 与代码一致 |
| `symbio/src/plugins/telegram/README` | Telegram 插件 | 与代码一致 |
| `symbio/src/plugins/vdfs/README` | VDFS 插件 | 与代码一致 |
| `symbio/src/plugins/web/README` | Web 插件 | 与代码一致 |
| `symbio/src/symbio_core/README` | symbio_core —— 内核契约层：命名与结构规范 | 与代码一致 |
| `tauri/README` | Tauri 前端 | 与代码一致 |

## 草案（draft）

（无）

## 废止（retired）

（无）

<!-- registry:summary end -->
