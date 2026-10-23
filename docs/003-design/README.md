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
summary.active.columns: [idLink, title:设计, impl]
summary.draft.title: 草案（draft）
summary.draft.columns: [idLink, title:设计, impl]
summary.retired.title: 废止（retired）
summary.retired.columns: [idLink, title:设计, reason]
board.columns: [impl]
---

# 设计总览

设计文档回答「产品与系统怎么设计」：**现状机制**（代码此刻的样子）、**目标形态**（要变成的样子）与**关键取舍**（为什么不是另一种）。三者写在**同一篇**里——同一件事不分散在两处。

- **时态在 `impl`，不在目录**：`done` = 与代码一致；`partial` = 已落一部分（残余在正文写明）；`planned` = 目标形态，尚未落地。落地只是把 `impl` 改一格，文档原地不动。
- **取舍跟着主题走**：架构决策（决策）折进所属设计的「关键决策」节，编号 `D-NNN` 保留为锚点；已退场或主题已消失的决策不迁移（`git log` 是历史）。
- **编号**：`0xx` 系统级设计（跨模块的机制与契约），`1xx` 组件级设计（就近放在该组件目录内）；`03x` 是其中的**能力阶梯**一组（`030` 总览 + `031`–`043` 十三阶，与 `verify/scenario_ladder.rs` 同源）。

## 怎么读这些设计

系统级设计不是一串并列话题，是**六个面**。按「先看什么」排：

- **总纲**——系统里存在什么、为什么这样设计：[核心架构](001-核心架构.md "DE-001-核心架构") · [架构总览](002-架构总览.md "DE-002-架构总览")
- **视图**——一次请求走哪些代码、谁挂在谁下面、代码怎么拆：[数据流与调用链](003-数据流与调用链.md "DE-003-数据流与调用链") · [系统地图](004-系统地图.md "DE-004-系统地图") · [模块架构](018-模块架构.md "DE-018-模块架构")
- **机制**——协议、地址、插件、资源访问、会话输入、两个对话环：[协议规范](005-协议规范.md "DE-005-协议规范") · [插件路由与地址](006-插件路由与地址.md "DE-006-插件路由与地址") · [第三方插件体系](007-第三方插件体系.md "DE-007-第三方插件体系") · [VDFS 规范](009-VDFS规范.md "DE-009-VDFS规范") · [会话输入与地址](012-会话输入与地址.md "DE-012-会话输入与地址") · [HTTP 开放接口](013-HTTP开放接口.md "DE-013-HTTP开放接口") · [单环一表](014-单环一表.md "DE-014-单环一表") · [义务环](015-义务环.md "DE-015-义务环")
- **界面**——资源页、前端机制化、体验目标：[VDFS 前端页面规范](010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") · [前端机制化](019-前端机制化.md "DE-019-前端机制化") · [前端 UI/UX](020-前端UI-UX.md "DE-020-前端UI-UX")
- **治理**——能力坐标系、落地与验证、登记机制、插件切分：[能力坐标系](008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系") · [工程落地与验证](016-工程落地与验证.md "DE-016-工程落地与验证") · [登记与汇总机制](017-登记与汇总机制.md "DE-017-登记与汇总机制") · [横向扩充与插件切分](021-横向扩充与插件切分.md "DE-021-横向扩充与插件切分")
- **阶梯**——13 阶能力一步步长出来的机制增量：[能力阶梯总览](030-能力阶梯总览.md "DE-030-能力阶梯总览") 起十三阶

**承重关系只有四条**（`deps`，机读）：VDFS 前端页面规范 ← VDFS 规范、义务环 ← 单环一表、
模块架构 ← 核心架构、前端机制化 ← 架构总览；阶梯内部另有逐阶依赖（S01 → … → S13）。
其余篇之间是**引用**而不是前置——一篇改了，不要求另一篇先改。这条区分要守住：
把引用写成 `deps`，「必须先有什么」这张图就不再有信息量。

**新增一篇设计时先回答「它落在哪个面」**——落不进任何一个面，通常说明它其实是两篇（总纲 + 细化），
或者它该写进某个模块的 `docs/` 而不是系统级目录。

<!-- registry:summary begin -->
<!-- list:design-entries owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](../005-spec/001.md)，机制见 [DE-017 · 登记与汇总机制](017-登记与汇总机制.md "DE-017-登记与汇总机制")。

**计数**：active 74 · draft 0 · retired 0（共 74）

## 现行设计（active）

| 编号 | 设计 | 落地 |
|---|---|---|
| [`DE-001-核心架构`](001-核心架构.md) | 核心架构：原语、Store、Projection、Actor | 与代码一致 |
| [`DE-002-架构总览`](002-架构总览.md) | 架构总览与目标形态 | 部分落地 |
| [`DE-003-数据流与调用链`](003-数据流与调用链.md) | 数据流与调用链（DATA_FLOW） | 与代码一致 |
| [`DE-004-系统地图`](004-系统地图.md) | Symbio 系统地图 | 与代码一致 |
| [`DE-005-协议规范`](005-协议规范.md) | Symbio 协议规范 | 与代码一致 |
| [`DE-006-插件路由与地址`](006-插件路由与地址.md) | 插件路由地址规则 | 与代码一致 |
| [`DE-007-第三方插件体系`](007-第三方插件体系.md) | 第三方插件体系与规范 | 与代码一致 |
| [`DE-008-能力注册表与能力坐标系`](008-能力注册表与能力坐标系.md) | 能力注册表与能力坐标系 | 与代码一致 |
| [`DE-009-VDFS规范`](009-VDFS规范.md) | VDFS —— 虚拟动态文件系统规范 | 与代码一致 |
| [`DE-010-VDFS前端页面规范`](010-VDFS前端页面规范.md) | VDFS 前端页面规范 | 与代码一致 |
| [`DE-011-Agent目录规范`](011-Agent目录规范.md) | Agent 目录规范 v2 | 部分落地 |
| [`DE-012-会话输入与地址`](012-会话输入与地址.md) | 会话输入改走地址：`<会话地址>/inbox` + 三个动作 | 与代码一致 |
| [`DE-013-HTTP开放接口`](013-HTTP开放接口.md) | HTTP(S) 开放接口插件设计 | 与代码一致 |
| [`DE-014-单环一表`](014-单环一表.md) | 单环一表收敛 | 与代码一致 |
| [`DE-015-义务环`](015-义务环.md) | 义务环与端口细粒度 | 部分落地 |
| [`DE-016-工程落地与验证`](016-工程落地与验证.md) | 工程落地与验证 | 与代码一致 |
| [`DE-017-登记与汇总机制`](017-登记与汇总机制.md) | 登记与汇总机制 | 与代码一致 |
| [`DE-018-模块架构`](018-模块架构.md) | 模块架构（施工视图） | 与代码一致 |
| [`DE-019-前端机制化`](019-前端机制化.md) | 前端机制化 | 部分落地 |
| [`DE-020-前端UI-UX`](020-前端UI-UX.md) | 前端 UI/UX 设计 | 目标形态 |
| [`DE-021-横向扩充与插件切分`](021-横向扩充与插件切分.md) | 横向扩充与大模块拆分 | 部分落地 |
| [`DE-030-能力阶梯总览`](030-能力阶梯总览.md) | 能力扩充阶梯 · 分步实施路线图 | 与代码一致 |
| [`DE-031-最小闭环`](031-最小闭环.md) | 最小闭环（对话基线） | 部分落地 |
| [`DE-032-工具调用与产物`](032-工具调用与产物.md) | 工具调用与产物 | 部分落地 |
| [`DE-033-多步任务与返工`](033-多步任务与返工.md) | 多步任务与返工 | 与代码一致 |
| [`DE-034-并发调度与租约`](034-并发调度与租约.md) | 并发调度与租约 | 目标形态 |
| [`DE-035-长会话与断点恢复`](035-长会话与断点恢复.md) | 长会话与断点恢复 | 与代码一致 |
| [`DE-036-长期记忆与语义检索`](036-长期记忆与语义检索.md) | 长期记忆与语义检索 | 部分落地 |
| [`DE-037-插话与实时打断`](037-插话与实时打断.md) | 插话与实时打断 | 与代码一致 |
| [`DE-038-多主体与对等承诺`](038-多主体与对等承诺.md) | 多主体与对等承诺 | 与代码一致 |
| [`DE-039-外部执行与熔断`](039-外部执行与熔断.md) | 外部执行与熔断 | 与代码一致 |
| [`DE-040-个人认知体系注入`](040-个人认知体系注入.md) | 个人认知体系注入 | 部分落地 |
| [`DE-041-技能编译与自我改进`](041-技能编译与自我改进.md) | 技能编译与自我改进 | 与代码一致 |
| [`DE-042-自主层与长期目标`](042-自主层与长期目标.md) | 自主层与长期目标 | 部分落地 |
| [`DE-043-多智能体社会`](043-多智能体社会.md) | 多智能体社会 | 部分落地 |
| [`DE-044-能力注册表建设`](044-能力注册表建设.md) | 能力注册表建设 | 与代码一致 |
| [`DE-101-context-compression-design`](../../symbio/src/plugins/session/docs/101-context-compression-design.md) | 会话上下文压缩设计 | 与代码一致 |
| [`DE-102-core-loop`](../../symbio/src/plugins/session/docs/102-core-loop.md) | 会话核心循环（LLM × 工具） | 与代码一致 |
| [`DE-103-turn-runner-steps`](../../symbio/src/plugins/session/docs/103-turn-runner-steps.md) | 016 · `run_with_tools` 的 Step 施工图（DE-014 步骤 1） | 与代码一致 |
| [`DE-104-module-layout`](../../symbio/src/plugins/session/docs/104-module-layout.md) | Session / Loop 插件模块分工 | 与代码一致 |
| [`DE-105-node-state-streaming`](../../symbio/src/plugins/session/docs/105-node-state-streaming.md) | 节点状态流（Node-State Streaming）—— 实时面即节点状态 | 与代码一致 |
| [`DE-106-perf`](../../symbio/src/plugins/session/docs/106-perf.md) | 会话 / 转写的前端性能设计 | 与代码一致 |
| [`DE-107-session-options`](../../symbio/src/plugins/session/docs/107-session-options.md) | 会话选项 = 会话配置表单 | 与代码一致 |
| [`DE-108-turn-tool-mechanisms`](../../symbio/src/plugins/session/docs/108-turn-tool-mechanisms.md) | 工具失败回传与 Turn 终态机制 | 与代码一致 |
| [`DE-109-vdfs-session-messages`](../../symbio/src/plugins/session/docs/109-vdfs-session-messages.md) | 会话消息的 VDFS 化 —— 转写即列表，流式即追加 | 与代码一致 |
| [`DE-110-FRONTEND`](../../tauri/docs/110-FRONTEND.md) | 前端架构 | 与代码一致 |
| [`DE-111-INTERACTION`](../../tauri/docs/111-INTERACTION.md) | 前端交互面（用户视角） | 与代码一致 |
| [`DE-112-architecture`](../../cli/docs/112-architecture.md) | 架构 | 与代码一致 |
| [`DE-113-heartbeat-mechanism`](../../symbio/src/plugins/heartbeat/docs/113-heartbeat-mechanism.md) | 会话心跳机制（heartbeat） | 与代码一致 |
| [`scripts/facts/README`](../../scripts/facts/README.md) | ''`scripts/facts/` —— 事实生成器目录'' | 与代码一致 |
| [`symbio/src/plugins/agent/README`](../../symbio/src/plugins/agent/README.md) | Agent 插件 | 与代码一致 |
| [`symbio/src/plugins/classify/README`](../../symbio/src/plugins/classify/README.md) | Classify 插件（对话面：判决） | 与代码一致 |
| [`symbio/src/plugins/compose/README`](../../symbio/src/plugins/compose/README.md) | Compose 插件（对话面：措辞） | 与代码一致 |
| [`symbio/src/plugins/composite/README`](../../symbio/src/plugins/composite/README.md) | Composite 插件 | 与代码一致 |
| [`symbio/src/plugins/event_bus/README`](../../symbio/src/plugins/event_bus/README.md) | Event Bus 插件 | 与代码一致 |
| [`symbio/src/plugins/gateway/README`](../../symbio/src/plugins/gateway/README.md) | Gateway 插件 | 与代码一致 |
| [`symbio/src/plugins/heartbeat/README`](../../symbio/src/plugins/heartbeat/README.md) | Heartbeat 插件（空闲会话的自主开轮调度） | 与代码一致 |
| [`symbio/src/plugins/home/README`](../../symbio/src/plugins/home/README.md) | Home 插件 | 与代码一致 |
| [`symbio/src/plugins/hook/README`](../../symbio/src/plugins/hook/README.md) | Hook 插件 | 与代码一致 |
| [`symbio/src/plugins/local/README`](../../symbio/src/plugins/local/README.md) | Local 插件 | 与代码一致 |
| [`symbio/src/plugins/loop/README`](../../symbio/src/plugins/loop/README.md) | Loop 插件（单环驱动器） | 与代码一致 |
| [`symbio/src/plugins/mcp/README`](../../symbio/src/plugins/mcp/README.md) | MCP 插件 | 与代码一致 |
| [`symbio/src/plugins/memory/README`](../../symbio/src/plugins/memory/README.md) | Memory 插件 | 与代码一致 |
| [`symbio/src/plugins/model/README`](../../symbio/src/plugins/model/README.md) | Model 插件 | 与代码一致 |
| [`symbio/src/plugins/plugin_manager/README`](../../symbio/src/plugins/plugin_manager/README.md) | Plugin_manager 插件（插件管理） | 与代码一致 |
| [`symbio/src/plugins/session/README`](../../symbio/src/plugins/session/README.md) | Unified Session & Memory Orchestration Architecture (会话与记忆系统化管理架构说明书) | 与代码一致 |
| [`symbio/src/plugins/session/docs/README`](../../symbio/src/plugins/session/docs/README.md) | Session 插件文档 | 与代码一致 |
| [`symbio/src/plugins/setting/README`](../../symbio/src/plugins/setting/README.md) | Setting 插件 | 与代码一致 |
| [`symbio/src/plugins/skill/README`](../../symbio/src/plugins/skill/README.md) | Skill 插件 | 与代码一致 |
| [`symbio/src/plugins/telegram/README`](../../symbio/src/plugins/telegram/README.md) | Telegram 插件 | 与代码一致 |
| [`symbio/src/plugins/vdfs/README`](../../symbio/src/plugins/vdfs/README.md) | VDFS 插件 | 与代码一致 |
| [`symbio/src/plugins/web/README`](../../symbio/src/plugins/web/README.md) | Web 插件 | 与代码一致 |
| [`symbio/src/symbio_core/README`](../../symbio/src/symbio_core/README.md) | symbio_core —— 内核契约层：命名与结构规范 | 与代码一致 |
| [`tauri/README`](../../tauri/README.md) | Tauri 前端 | 与代码一致 |

## 草案（draft）

（无）

## 废止（retired）

（无）

<!-- registry:summary end -->
