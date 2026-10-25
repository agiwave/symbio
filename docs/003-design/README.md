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

设计文档回答「产品与系统怎么设计」：**目标形态**（要变成的样子）、**现行机制**（代码此刻的样子）与**关键取舍**（为什么不是另一种）。三者尽量写在**同一篇**里——同一件事不分散在两处。

## 权威顺序

1. **目标态**：[核心架构](001-核心架构.md "DE-001-核心架构") 是系统存在什么、控制面怎么跑、Agent 怎么组织的**唯一 owner**，专题细则只展开它。任何「目标应该长什么样」的裁决，只在目标文档里改写。
2. **现状路径**：[过渡方案](090-过渡方案.md "DE-090-过渡方案") 只回答「从现在到目标怎么走」，不反向定义目标。
3. **历史**：已退场文档保留原号与决策锚点；已完成的一次性实施方案与评审进 `docs/archive/`，不参与现行设计。

- **时态在 `impl`，不在目录**：`done` = 与代码一致；`partial` = 已落一部分（残余在正文写明）；`planned` = 目标形态，尚未落地。落地只是把 `impl` 改一格，文档原地不动。
- **取舍跟着主题走**：架构决策折进所属设计的「关键决策」节，编号 `D-NNN` 保留为锚点；已退场或主题已消失的决策不迁移（`git log` 是历史）。
- **一个事实只有一个 owner**：目标、路径、验证、登记各有归属（见下方边界表），不在多篇里复述同一事实。

## 编号与命名

- 文件名 = `NNN-主题.md`，**编号即身份**（稳定）；改名或改号会打断锚点与链接，故本规则用于新文档，既有编号不重排。
- `00x`：系统级基础契约与机制（跨模块）。
- `01x`–`02x`：系统级专题（Agent 目录、会话输入、HTTP、义务环、模块架构、前端等）。
- `03x`：能力阶梯——`030` 总览 + `031`–`043` 十三阶，与 `verify/scenario_ladder.rs` 同源。
- `09x`：过渡与历史——`090` 过渡方案；已退场文档保留原号。
- `1xx`：组件级设计，就近放在该组件目录内（如 `symbio/src/plugins/session/docs/101-…`）。
- 标题写**主题名**：不加英文别名、不写「施工视图」「对话基线」一类阶段后缀、不重复 `Symbio` 前缀；`title` 逐字等于首个 H1（RG-021）。

## 怎么读：六个面 + 过渡

系统级设计不是一串并列话题，是**六个面**。按「先看什么」排：

### 一、总纲——系统里存在什么、为什么这样设计

| 文档 | 回答什么 |
|---|---|
| [核心架构](001-核心架构.md "DE-001-核心架构") | 一个原语、一个签名约束、一个使用者概念、三条不变量与 N 行参数；控制面的一环 / 一表 / 一源；Agent 的组织形态 |
| [架构总览](002-架构总览.md "DE-002-架构总览") | 判断架构改动是否成立的原则、目录分层、`Plugin` 契约与「哪一层可妥协」的标尺 |

### 二、视图——一次请求走哪些代码、代码怎么拆

| 文档 | 回答什么 |
|---|---|
| [数据流与调用链](003-数据流与调用链.md "DE-003-数据流与调用链") | 一次请求从入口到出口经过哪些代码（含排障锚点） |
| [系统地图](004-系统地图.md "DE-004-系统地图") | 运行期拓扑一图看完 |
| [模块架构](018-模块架构.md "DE-018-模块架构") | 代码包怎么拆、依赖方向与模块不变量 |

### 三、机制——协议、地址、插件、资源访问、会话输入、义务环

| 文档 | 回答什么 |
|---|---|
| [协议规范](005-协议规范.md "DE-005-协议规范") | 帧 / 载荷 / 通道 / 线格式的准确规格 |
| [插件路由与地址](006-插件路由与地址.md "DE-006-插件路由与地址") | `route` / `traverse` 两条寻址通道的地址规则 |
| [第三方插件体系](007-第三方插件体系.md "DE-007-第三方插件体系") | 外部插件怎么打包、装载与接入 |
| [VDFS 虚拟动态文件系统规范](009-VDFS规范.md "DE-009-VDFS规范") | 纯接口、统一文件系统与容器拓扑 |
| [会话输入与地址](012-会话输入与地址.md "DE-012-会话输入与地址") | 会话输入落在哪：`<会话地址>/inbox` + 三个动作 |
| [HTTP 开放接口](013-HTTP开放接口.md "DE-013-HTTP开放接口") | HTTP/WS 网关插件的端点、配置与调用链 |
| [义务环](015-义务环.md "DE-015-义务环") | 债、行表、端口粒度与装载检查的机制细则 |

### 四、界面——资源页、协议呈现、体验目标

| 文档 | 回答什么 |
|---|---|
| [VDFS 前端页面规范](010-VDFS前端页面规范.md "DE-010-VDFS前端页面规范") | 页面 = 路由宿主；VDFS 地址驱动的界面与文件布局 |
| [前端机制化](019-前端机制化.md "DE-019-前端机制化") | 协议收敛到节点树 / `kind` / `principal`，前端只按机制分派渲染 |
| [前端 UI/UX](020-前端UI-UX.md "DE-020-前端UI-UX") | 体验目标与交互形态 |

### 五、能力与阶梯——能力能长到哪去、怎么一步步长出来

| 文档 | 回答什么 |
|---|---|
| [能力注册表与能力坐标系](008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系") | 能力坐标系（五维 / 五轴 / 天梯）与注册表形状（行族、写读、三问裁决，见该文第九节） |
| [能力阶梯总览](030-能力阶梯总览.md "DE-030-能力阶梯总览") | 13 阶路线图、逐阶前置与验收判据 |

各阶的机制增量：

| 阶 | 文档 | 增量 |
|:-:|---|---|
| S01 | [最小闭环](031-最小闭环.md "DE-031-最小闭环") | 对话基线：一轮怎么走通 |
| S02 | [工具调用与产物](032-工具调用与产物.md "DE-032-工具调用与产物") | 工具清单、调用与产物落点 |
| S03 | [多步任务与返工](033-多步任务与返工.md "DE-033-多步任务与返工") | 任务表、依赖、返工与收束 |
| S04 | [并发调度与租约](034-并发调度与租约.md "DE-034-并发调度与租约") | 并发集合与租约 |
| S05 | [长会话与断点恢复](035-长会话与断点恢复.md "DE-035-长会话与断点恢复") | 压缩、归档与恢复 |
| S06 | [长期记忆与语义检索](036-长期记忆与语义检索.md "DE-036-长期记忆与语义检索") | 记忆投影与召回 |
| S07 | [插话与实时打断](037-插话与实时打断.md "DE-037-插话与实时打断") | 外部事件注入与打断 |
| S08 | [多主体与对等承诺](038-多主体与对等承诺.md "DE-038-多主体与对等承诺") | 多主体、承诺与信任 |
| S09 | [外部执行与熔断](039-外部执行与熔断.md "DE-039-外部执行与熔断") | 外部执行、隔离与熔断 |
| S10 | [个人认知体系注入](040-个人认知体系注入.md "DE-040-个人认知体系注入") | 人格、价值与知识注入 |
| S11 | [技能编译与自我改进](041-技能编译与自我改进.md "DE-041-技能编译与自我改进") | 技能编译与回退 |
| S12 | [自主层与长期目标](042-自主层与长期目标.md "DE-042-自主层与长期目标") | 自主发起与长期目标 |
| S13 | [多智能体社会](043-多智能体社会.md "DE-043-多智能体社会") | 多智能体协作与社会结构 |

### 六、工程与治理——怎么验证、怎么登记、怎么扩充

| 文档 | 回答什么 |
|---|---|
| [工程落地与验证](016-工程落地与验证.md "DE-016-工程落地与验证") | 什么算「验证过」；判据与验证程序的 owner |
| [登记与汇总机制](017-登记与汇总机制.md "DE-017-登记与汇总机制") | 文档即登记、汇总块生成与状态机 |
| [横向扩充与插件切分](021-横向扩充与插件切分.md "DE-021-横向扩充与插件切分") | 加一行 / 一格 / 一臂的真实成本与模块级拆分 |

### 过渡与历史（不是目标设计的一部分）

| 文档 | 回答什么 |
|---|---|
| [过渡方案与现状差距](090-过渡方案.md "DE-090-过渡方案") | 现状 → 目标的六步路径、临时形状与已知差距；差距关闭即删条目 |
| [单环一表收敛](014-单环一表.md "DE-014-单环一表")（retired） | 保留 D-064…D-128 的决策锚点与历史裁决；目标形状已并入 DE-001 / DE-015 |
| `docs/archive/` | 已完成的实施方案、一次性评审、已退场机制；**不进登记**，`git log` 是变更史 |

## 文档边界（防重复）

| 事实 | owner | 其他文档怎么写 |
|---|---|---|
| 系统存在什么 / 控制面 / Agent 组织 | [DE-001](001-核心架构.md "DE-001-核心架构") | 只引用，不复述形状 |
| 原则、分层、可妥协标尺 | [DE-002](002-架构总览.md "DE-002-架构总览") | 决策只写「违反哪条原则」 |
| 一次请求经过哪些代码 | [DE-003](003-数据流与调用链.md "DE-003-数据流与调用链") | 排障与调用链细节指回该文 |
| 运行期拓扑 | [DE-004](004-系统地图.md "DE-004-系统地图") | 只画自己那一层的局部图 |
| 代码包拆分与依赖 | [DE-018](018-模块架构.md "DE-018-模块架构") | 模块 README 只写自有机制 |
| 线格式 / 帧 / 载荷 | [DE-005](005-协议规范.md "DE-005-协议规范") | 只引用字段名，不重抄结构 |
| 地址规则 | [DE-006](006-插件路由与地址.md "DE-006-插件路由与地址") | 路由细节指回该文 |
| 债、行表、端口细则 | [DE-015](015-义务环.md "DE-015-义务环") | 主文档只给形状，细节指回该文 |
| 能力坐标与注册表形状 | [DE-008](008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系") | 能力名册只在 §6.1 维护 |
| 能力阶梯与各阶增量 | [DE-030](030-能力阶梯总览.md "DE-030-能力阶梯总览")–[DE-043](043-多智能体社会.md "DE-043-多智能体社会") | 各阶只写本阶增量 |
| 验证判据与脚本 | [DE-016](016-工程落地与验证.md "DE-016-工程落地与验证") | 文档只写「由哪条判据守」 |
| 文档登记机制 | [DE-017](017-登记与汇总机制.md "DE-017-登记与汇总机制") | front-matter 即登记，不另建表 |
| 从现在到目标怎么走 | [DE-090](090-过渡方案.md "DE-090-过渡方案") | 目标文档不写施工步骤 |
| 已退场目标 / 一次性方案 | [DE-014](014-单环一表.md "DE-014-单环一表") / `docs/archive/` | 现行文档只引用锚点，不复述旧形状 |

**承重关系**（`deps`，机读）：VDFS 前端页面规范 ← VDFS 规范、义务环 ← 核心架构、
模块架构 ← 核心架构、前端机制化 ← 架构原则、过渡方案 ← 核心架构 / 义务环；
阶梯内部另有逐阶依赖（S01 → … → S13）。
其余篇之间是**引用**而不是前置——一篇改了，不要求另一篇先改。这条区分要守住：
把引用写成 `deps`，「必须先有什么」这张图就不再有信息量。

**新增一篇设计时先回答「它落在哪个面」**——落不进任何一个面，通常说明它其实是两篇（总纲 + 细化），
或者它该写进某个模块的 `docs/` 而不是系统级目录。已完成的实施方案不新增设计条目，进 `docs/archive/`。

> 下方生成块是**全 kind 登记投影**（含组件级 `1xx` 与各模块 `README`）；上面的六面导读只覆盖
> 系统级 `0xx`–`09x`。

<!-- registry:summary begin -->
<!-- list:design-entries owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](../005-spec/001.md)，机制见 [DE-017 · 登记与汇总机制](017-登记与汇总机制.md "DE-017-登记与汇总机制")。

**计数**：active 73 · draft 0 · retired 1（共 74）

## 现行设计（active）

| 编号 | 设计 | 落地 |
|---|---|---|
| [`DE-001-核心架构`](001-核心架构.md) | 核心架构：事实、Actor 与控制环 | 与代码一致 |
| [`DE-002-架构总览`](002-架构总览.md) | 架构总览：原则与分层 | 部分落地 |
| [`DE-003-数据流与调用链`](003-数据流与调用链.md) | 数据流与调用链 | 与代码一致 |
| [`DE-004-系统地图`](004-系统地图.md) | 系统地图 | 与代码一致 |
| [`DE-005-协议规范`](005-协议规范.md) | 协议规范 | 与代码一致 |
| [`DE-006-插件路由与地址`](006-插件路由与地址.md) | 插件路由与地址 | 与代码一致 |
| [`DE-007-第三方插件体系`](007-第三方插件体系.md) | 第三方插件体系 | 与代码一致 |
| [`DE-008-能力注册表与能力坐标系`](008-能力注册表与能力坐标系.md) | 能力注册表与能力坐标系 | 与代码一致 |
| [`DE-009-VDFS规范`](009-VDFS规范.md) | VDFS 虚拟动态文件系统规范 | 与代码一致 |
| [`DE-010-VDFS前端页面规范`](010-VDFS前端页面规范.md) | VDFS 前端页面规范 | 与代码一致 |
| [`DE-011-Agent目录规范`](011-Agent目录规范.md) | Agent 目录规范 | 部分落地 |
| [`DE-012-会话输入与地址`](012-会话输入与地址.md) | 会话输入与地址 | 与代码一致 |
| [`DE-013-HTTP开放接口`](013-HTTP开放接口.md) | HTTP 开放接口 | 与代码一致 |
| [`DE-015-义务环`](015-义务环.md) | 义务环 | 部分落地 |
| [`DE-016-工程落地与验证`](016-工程落地与验证.md) | 工程落地与验证 | 与代码一致 |
| [`DE-017-登记与汇总机制`](017-登记与汇总机制.md) | 登记与汇总机制 | 与代码一致 |
| [`DE-018-模块架构`](018-模块架构.md) | 模块架构 | 与代码一致 |
| [`DE-019-前端机制化`](019-前端机制化.md) | 前端机制化 | 部分落地 |
| [`DE-020-前端UI-UX`](020-前端UI-UX.md) | 前端 UI/UX | 部分落地 |
| [`DE-021-横向扩充与插件切分`](021-横向扩充与插件切分.md) | 横向扩充与插件切分 | 部分落地 |
| [`DE-030-能力阶梯总览`](030-能力阶梯总览.md) | 能力阶梯总览 | 与代码一致 |
| [`DE-031-最小闭环`](031-最小闭环.md) | 最小闭环 | 部分落地 |
| [`DE-032-工具调用与产物`](032-工具调用与产物.md) | 工具调用与产物 | 部分落地 |
| [`DE-033-多步任务与返工`](033-多步任务与返工.md) | 多步任务与返工 | 与代码一致 |
| [`DE-034-并发调度与租约`](034-并发调度与租约.md) | 并发调度与租约 | 与代码一致 |
| [`DE-035-长会话与断点恢复`](035-长会话与断点恢复.md) | 长会话与断点恢复 | 与代码一致 |
| [`DE-036-长期记忆与语义检索`](036-长期记忆与语义检索.md) | 长期记忆与语义检索 | 部分落地 |
| [`DE-037-插话与实时打断`](037-插话与实时打断.md) | 插话与实时打断 | 与代码一致 |
| [`DE-038-多主体与对等承诺`](038-多主体与对等承诺.md) | 多主体与对等承诺 | 与代码一致 |
| [`DE-039-外部执行与熔断`](039-外部执行与熔断.md) | 外部执行与熔断 | 与代码一致 |
| [`DE-040-个人认知体系注入`](040-个人认知体系注入.md) | 个人认知体系注入 | 部分落地 |
| [`DE-041-技能编译与自我改进`](041-技能编译与自我改进.md) | 技能编译与自我改进 | 与代码一致 |
| [`DE-042-自主层与长期目标`](042-自主层与长期目标.md) | 自主层与长期目标 | 部分落地 |
| [`DE-043-多智能体社会`](043-多智能体社会.md) | 多智能体社会 | 部分落地 |
| [`DE-090-过渡方案`](090-过渡方案.md) | 过渡方案与现状差距 | 部分落地 |
| [`DE-101-context-compression-design`](../../symbio/src/plugins/session/docs/101-context-compression-design.md) | 会话上下文压缩设计 | 与代码一致 |
| [`DE-102-core-loop`](../../symbio/src/plugins/session/docs/102-core-loop.md) | 会话核心循环（LLM × 工具） | 与代码一致 |
| [`DE-103-turn-runner-steps`](../../symbio/src/plugins/session/docs/103-turn-runner-steps.md) | `run_with_tools` 的 Step 施工图（DE-090 §1 步骤 1） | 与代码一致 |
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

| 编号 | 设计 | 理由 |
|---|---|---|
| [`DE-014-单环一表`](014-单环一表.md) | 单环一表收敛 | 目标形状已并入 DE-001 §4.3 / §4.4 与 DE-015，施工六步并入 DE-090 §1，本文正文（目标设计 / 落地六步 / 场景验证）随退场删除；决策锚点 D-064…D-128 保留原址作为宿主——它们仍约束现行代码，且代码引用最多，搬家不增加信息（D-145 与本篇头注）。 |

<!-- registry:summary end -->
