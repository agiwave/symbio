---
lists: [doc-boundaries=owner, kinds=owner]
kind: guide
id: docs/README
title: Symbio 文档中心
status: active
created: 2026-07-20
---

<!-- doc-link-allow D-004: 本文定义「现行文档不写变更史」这条规矩及其判据边界，需引用反例措辞界定禁止范围 -->
# Symbio 文档中心

> **文档驱动开发**：本文档体系是 Symbio 项目的「单一事实来源」。

## 三种文档，两个时态

| 你在写什么 | 去哪 | 允许写过程吗 |
|---|---|---|
| 要达成什么、谁在用、系统怎么设计、必须成立什么、怎么用 | `001-goal` / `002-scene` / `003-design` / `005-spec` / `006-guide` | **不** |
| 权威数据表（路由 / 错误码 / 配置 / 当前事实） | `004-ref` | 不 |
| 要做什么、做到哪一步 | `007-task` | **是（唯一）** |

两类时态用两个字段表达，不用两个目录：

- **`status`** 回答「这篇文档还算不算数」（`draft` / `active` / `retired`）；
- **`impl`** 回答「它离代码有多远」（`done` / `partial` / `planned` / `none`）——只有 `design` 必填。

**落地不改身份**：一篇目标设计实现之后，只把 `impl` 从 `planned` 改成 `done`，文档原地不动。
不存在「做完要从 plan 搬进 arch」这种迁移。

## 文档登记：每篇文档都是登记条目

全仓的活跃 `.md` 都在**同一套登记机制**里（机制见 [DE-017](003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")，字段见 [SP-001](005-spec/001.md)）：
每篇文档开头是 front-matter，**文件本身就是条目**，没有第二处登记表。

 ---
 kind: design
 id: DE-009-VDFS规范
 title: VDFS —— 虚拟动态文件系统规范
 status: active
 created: 2026-09-13
 impl: done
 ---

- **`kind` 回答「这篇文档在回答什么问题」**——七类，各有自己的目录与汇总页。每类的形状
 （编号前缀 / 状态机 / 汇总版式）由**该 kind 目录 `README.md` 的 meta 自描述**，机制不内建
 任何类型清单；下面的总账块由脚本投影生成：

<!-- registry:summary begin -->
<!-- list:kinds owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 条目各是一个单文档：条目类**在位存放**（文件即条目，kind 只来自 front-matter，目录不参与判定）；
> 各 kind 的形状与汇总版式由其 README meta 自描述；机制见 [DE-017 · 登记与汇总机制](003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")，字段规范见 [SP-001](005-spec/001.md)。

| 类别 | 身份 | 回答什么 | 条目数 | 状态分布 | 分解 |
|---|---|---|---|---|---|
| [goal · 目标](001-goal/README.md "GO-000-目标汇总") | 编号前缀 `GO-` | 要达成什么——产品 / 技术 / 工程三个大目标，各挂可度量判据 | 3 | draft 0 · active 3 · retired 0 | — |
| [scene · 场景](002-scene/README.md "SC-000-场景汇总") | 编号前缀 `SC-` | 用户拿它做什么——角色、情境与用户判据 | 18 | draft 5 · active 13 · retired 0 | — |
| [design · 设计](003-design/README.md "DE-000-设计总览") | 编号前缀 `DE-` | 产品与系统怎么设计——现状、目标形态与关键取舍（每篇的时态由 impl 标注） | 74 | draft 0 · active 73 · retired 1 | — |
| [ref · 参考](004-ref/README.md "RE-000-参考汇总") | 编号前缀 `RE-` | 查数据——路由 / 错误码 / 配置等权威表 | 10 | draft 0 · active 10 · retired 0 | — |
| [spec · 规范](005-spec/README.md "SP-000-规范汇总") | 编号前缀 `SP-` | 必须成立的条文——机制与判据的规范面 | 32 | draft 0 · active 27 · retired 5 | — |
| [guide · 指南](006-guide/README.md "GU-000-指南汇总") | 编号前缀 `GU-` | 怎么用——上手 / 贡献 / 排障 | 14 | draft 0 · active 14 · retired 0 | — |
| [task · 任务](007-task/README.md "T-000-任务汇总") | 编号前缀 `T-` | 要做什么、做到哪一步——可勾销的最小工作单元，收口即删（只留 git 历史） | 13 | todo 12 · doing 1 · done 0 · wont 0 | 11 条分解边 |

<!-- registry:summary end -->

- **编号规范全仓一套**：细则见 [SP-001](005-spec/001.md "SP-001")（id 形态 / 文件名 = 去前缀主干 /
 前缀与 kind 对账 / 导航文档豁免），本页不复述——写错会被 RG-003 / RG-022 判红。
 设计文档的分区（`0xx` 系统级、`1xx` 组件级）见 [SP-032](005-spec/032.md "SP-032")。

### 四层怎么衔接（goal → design → spec → task）

| 层 | kind | 回答什么 | 依赖指向 |
|---|---|---|---|
| 目标 | `goal` | 要达成什么、违反了哪条判据会变红 | 链根，无上游 |
| 设计 | `design` | 怎么设计、为什么这样选（含被否决的方案） | 上游目标；跨篇关系写在正文，不建依赖边 |
| 规范 | `spec` | 必须成立什么条文、由哪条判据守 | 由设计推出；条文只引 `DE-` 编号，不复述设计 |
| 任务 | `task` | 现在做什么、做到哪一步 | 前置任务（`deps`）；大任务用 `parent` 分解出子任务 |

`scene` 与 `guide` **不在这条链上**：场景是用户视角的**验收面**（目标说「要什么」，场景说「用户看到什么」），
指南是**使用面**。两者都不是「为了达成某个目标而必须经过的环节」，所以不参与依赖链。

**任务不回头被引用**：`task` 是过程记录，只在 `007-task/` 内部互相引用。
其它文档、源码注释、提交信息之外的地方**不得出现 `T-` 编号**——过程不是事实（判据 D-016）。

- **机读关系只有 `deps` 与 `parent` 两个字段**：`deps` 答「必须先有什么」，`parent` 答「是谁的一部分」（只许同 kind）。
 两者的值域与判据（RG-006 / RG-008 / RG-024 / RG-025）在 SP-001 与 [DE-017](003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")，本页不复述。
- **`id` 是身份，`status` / `impl` 是状态**：字段语义见 SP-001；各 kind 的状态词由该 kind README meta 声明。
- **目录不参与判定**：`kind` 只来自 front-matter；文档搬去别的目录不会破机制，只需把 `id` 与它一起改（判据会拦）。
 **模块文档就地登记**（`symbio/src/plugins/*/README.md` 与模块 `docs/` 不搬进 `docs/`）——「就近放置」优先于「按类归堆」。
- **引用只写编号**：文档之间互相引用时只写 `[[编号]]`（如 `[[SP-001]]`；要显示文字写 `[[编号|显示文字]]`），
 **地址既不手写也不维护**——由 `node scripts/gen-doc-refs.mjs` 从登记解析生成，每次门禁重算全仓地址。
 判据三条：编号不存在判红（RF-001）、地址漂移自动重算（RF-002）、**指向已登记文档的手工地址判红**（RF-003）。
 编号是身份（稳定），地址是位置（会变）。
- **汇总页是投影**：各 kind 的汇总块住本 kind 目录 `README.md` 的 `<!-- registry:summary -->` 标记区，跨 kind 总账块住本页，
 都由 `node scripts/registry-audit.mjs --write` 生成（门禁自动重跑）；查询看 `node scripts/registry-board.mjs`。

## 文档下沉原则

文档**就近放置、单一来源**：

- **单模块文档**放在该模块目录内（`README.md` + 可选 `docs/`），只写该模块自身机制，不重复系统级内容；
- **系统级文档**（本目录）只保留**跨模块**的核心逻辑、边界与约定，涉及单模块内部实现时**引用**模块文档，不复制细节；
- **历史**一律进 `archive/`（归档不进登记）：已废止的机制、一次性评审、已落地的实施方案、已完成的决策记录——**git log 是变更史**。

下面这些由 `node scripts/doc-link-audit.mjs` 机械判定（任一命中即失败）：**站内链接必须有效**、
**过程文档不得滞留活跃目录**（D-002）、**活跃文档不得超过 1000 行**（D-003）、**正文不得写变更史**（D-004——历史归 `git log`）、
**反引号里指路的文件路径必须存在**（D-006）、**链接的 `#锚点` 必须指向目标文件真实存在的标题**（D-007）、
**裸 `R<数字>` 必须是风险表定义行的首列、或紧邻「风险」的限定引用**（D-008）、
**非 task 文档与源码注释不得引用 `T-` 编号**（D-016）、
**证据锚点指到的文件 / 符号 / 行号必须存在**（D-017——`<文件>:<行>` / `<文件>::<符号>`）。

行数上限的作用不是控制篇幅，而是给「职责失守」一个不会误报的信号：超限即按下面这张表处置，没有豁免。
D-004 判**措辞标记**而不判**叙事**（`曾经` / `已改为` / `修订（` / `后记（` / `初版` 等；不判裸数字箭头与叙事式历史）：
守卫一旦开始误报，下场是被整体豁免，比没有还糟。豁免分**行内**（`<!-- doc-link-allow D-00X: 理由 -->` 写在本行或前一行）
与**全文**（文件首行非空内容，理由写明它买断的是什么）两档。

**文档文件名不含空格**（用 `-` 或直接相连）。D-001 不做 URL 解码，写成 `%20` 的链接会被判为失效。

文档是**下沉**的，所以「某条约定写在哪」要靠检索而不是靠记：`node scripts/doc-find.mjs <关键词>`
会同时搜 `*.md` 与源码里的 `//!` / `///`（相当一部分机制就写在模块文档注释里）。**知识只写一处**。

## 文档职责边界（一个事实只有一个 owner）

**重复描述 = 双倍维护成本。** 同一事实写两处，改代码时就要改两处，且两处必然漂移。
因此每类事实只有一个 owner，其余位置一律**引用**，不复述：

<!-- list:doc-boundaries owner -->

| 事实 | 唯一 owner | 其它文档 |
|---|---|---|
| 现在是什么（插件 / 挂载点 / 路由 / 工具 / 规模 / 每条能力的用户可见语义） | [RE-006-当前事实表](004-ref/006-当前事实表.md)（代码生成，能力矩阵在 §2） | **不手抄**，需要就引用 |
| 路由**契约**（调用方 / 返回类型 / 备注 / 退役索引） | [RE-003-ROUTES](004-ref/003-ROUTES.md) | 模块 `README.md` 不抄路由表 |
| 路由**一句话职责** | [RE-006-当前事实表](004-ref/006-当前事实表.md) §2.3（代码投影） | RE-003 不另写「用途」列 |
| 错误码 | [RE-002-ERROR_CODES](004-ref/002-ERROR_CODES.md) | PROTOCOLS 不再列错误码表 |
| 配置项 | [RE-001-CONFIGURATION](004-ref/001-CONFIGURATION.md) | 模块 `README.md` 只写机制 + 指向它 |
| 判据码（各守卫的编号地址） | [ref/GATE_CODES.md](./004-ref/GATE_CODES.md)（脚本声明生成） | 守卫脚本只写自己判什么，不抄全表 |
| 出厂 prompt 的**形状基线** | [ref/PROMPT_SHAPE.md](./004-ref/PROMPT_SHAPE.md)（真实链路生成，不手改） | 口径在 [DE-102](../symbio/src/plugins/session/docs/102-core-loop.md "DE-102-core-loop") |
| 协议线上形状（帧 / 载荷 / 通道 / 线格式） | [design/005-协议规范.md](003-design/005-协议规范.md "DE-005-协议规范") | — |
| 系统拓扑（谁挂在谁下面） | [design/004-系统地图.md](003-design/004-系统地图.md "DE-004-系统地图") | 其它图只画自己那一层 |
| 排障链路（一次请求经过哪些代码） | [design/003-数据流与调用链.md](003-design/003-数据流与调用链.md "DE-003-数据流与调用链") | — |
| **为什么这样设计（取舍与被否决的方案）** | 该主题的**设计文档**里的「关键决策」节（`### D-NNN`） | 其它文档只引用 `D-` 编号 |
| 模块内部机制 | 该模块 `README.md` | 系统级文档不复述 |
| 模块深度设计与不变量 | 该模块 `docs/` | — |
| **要达成什么（产品 / 技术 / 工程三个大目标与可度量判据）** | [goal/001-产品目标.md](001-goal/001-产品目标.md "GO-001-产品目标") | 其余文档只引用编号，不抄判据数字 |
| **用户拿它做什么（角色 / 场景 / 用户判据）** | [SC-000-场景汇总](002-scene/README.md) | 其余文档只引用编号 |
| **必须成立的条文与它的判据** | [spec/](005-spec/README.md "SP-000-规范汇总") 下的单文档（`## 规范` + `## 判据`） | 条文不复述，只引 `SP-` 编号 |
| 条目级登记（任务 / 规范的编号、状态、优先级、依赖、处理过程） | `docs/<kind>/` 下的**单文档**（汇总页由脚本生成） | 各处不手抄条目清单 |
| 变更历史（改了什么、何时改的） | `git log` | **现行文档不写变更史** |
| 一次性评审 / 迁移记录 / 体检报告 / 已完成的决策 | `docs/archive/` | 活文档不保留过程日志 |

**变更时只改 owner 那一处**：结构事实重跑 `node scripts/gen-facts.mjs`（门禁自动重生成并暂存）；
跨模块架构改 [002-架构总览.md](003-design/002-架构总览.md "DE-002-架构总览") + [004-系统地图.md](003-design/004-系统地图.md "DE-004-系统地图") 各自己那一层；
常见问题进 [故障排查指南](006-guide/007-TROUBLESHOOTING.md "GU-007-TROUBLESHOOTING")。

### 这张表有人守吗

**「一个事实只有一个 owner」由判据守着，不靠记性**（`scripts/facts.test.mjs`，随门禁跑）：
**文件内**一个封闭集只在一处成表、**跨文档**非 owner 文档的表里含 ≥3 个成员即判红（散文点名不算）、
**owner 收留**——真源加了东西而表没跟上判红。集合声明只有一份，住在 [`scripts/facts/_sets.mjs`](../scripts/facts/_sets.mjs)，
owner 从各文档 front-matter 的 `lists: [<id>=owner]` 读（与 [RE-007-清单归属总账](004-ref/007-清单归属总账.md "RE-007-清单归属总账") 同一份输入）。
**新增一节的纪律**：写下新节时先问**它覆盖了谁**——覆盖了就要同时删掉旧 owner 或把它改成一行指路。

### 两条硬规则

1. **现行文档只描述「现在是什么」与「要变成什么」**，不写「先做哪批、做到第几步、评审怎么说的」。
 追溯走 `git log -S<符号>` / `git log --grep=<词>`；要保留「为什么否决 A 选 B」的结论，
 写进该主题设计文档的「关键决策」节——**那才是它的 owner**。
2. **代码注释与文档各有边界**：注释写「读这个文件需要知道的不变量与陷阱」，文档写「机制与取舍」。
 **注释不复述文档，文档不复述注释**（签名、方法清单、字段表以代码为准）。详见 [CONTRIBUTING.md §4](../CONTRIBUTING.md "CONTRIBUTING")。

## 快速导航

| 我想... | 查阅 |
|---------|------|
| 核对「现在是什么」（插件 × 挂载点 × 路由 × 工具，自动生成） | [RE-006-当前事实表](004-ref/006-当前事实表.md) |
| 了解系统全貌 / 架构设计 | [001-核心架构.md](003-design/001-核心架构.md "DE-001-核心架构") · [002-架构总览.md](003-design/002-架构总览.md "DE-002-架构总览") · [004-系统地图.md](003-design/004-系统地图.md "DE-004-系统地图") |
| 查看协议规范 / 追踪请求全链路 | [005-协议规范.md](003-design/005-协议规范.md "DE-005-协议规范") · [003-数据流与调用链.md](003-design/003-数据流与调用链.md "DE-003-数据流与调用链") |
| 理解控制面 / 义务环 | [001-核心架构.md](003-design/001-核心架构.md "DE-001-核心架构") §4.3–§4.4 · [015-义务环.md](003-design/015-义务环.md "DE-015-义务环") · [090-过渡方案.md](003-design/090-过渡方案.md "DE-090-过渡方案") |
| 查找路由路径 / 理解错误码 / 配置系统 | [RE-003-ROUTES](004-ref/003-ROUTES.md) · [RE-002-ERROR_CODES](004-ref/002-ERROR_CODES.md) · [RE-001-CONFIGURATION](004-ref/001-CONFIGURATION.md) |
| 给门禁加一条判据、要挑不撞的号 | [GATE_CODES.md](./004-ref/GATE_CODES.md) |
| 快速上手 / 开发新插件 / 排查问题 | [GU-006-QUICK_START](006-guide/006-QUICK_START.md) · [GU-005-PLUGIN_DEVELOPMENT](006-guide/005-PLUGIN_DEVELOPMENT.md) · [GU-007-TROUBLESHOOTING](006-guide/007-TROUBLESHOOTING.md) |
| 看**全项目登记总账**（七类 kind × 条目数 × 状态分布） | 本页上方总账块（生成）；查询台 `node scripts/registry-board.mjs` |
| 了解平台**要达成什么** / **用户拿它做什么** | [GO-001 · 产品目标](001-goal/001-产品目标.md "GO-001-产品目标") · [GO-002 · 技术目标](001-goal/002-技术目标.md "GO-002-技术目标") · [SC-000 · 场景总览](002-scene/README.md "SC-000-场景汇总") |
| 看**目标形态**与尚未落地的设计 | `node scripts/registry-board.mjs --kind=design --impl=planned` |
| 看**某条纪律与它的判据** | [spec/](005-spec/README.md "SP-000-规范汇总")（每条 `## 规范` + `## 判据`） |
| 查「某条约定写在哪」 | `node scripts/doc-find.mjs <词>`（全仓 md + Rust 文档注释检索） |

## 文档放在哪

| 位置 | 放什么 |
|------|--------|
| `docs/001-goal/` | 三个大目标（产品 / 技术 / 工程）与可度量判据 |
| `docs/002-scene/` | 角色、情境与用户判据 |
| `docs/003-design/` | **设计**：系统级 0xx（架构 / 机制 / 协议 / 目标形态）+ `verify/`（随设计的可编译验证程序） |
| `docs/004-ref/` | 权威参考页（含生成物 GATE_CODES / PROMPT_SHAPE / CURRENT） |
| `docs/005-spec/` | 规范条文与判据（每条一个单文档，`## 规范` + `## 判据`） |
| `docs/006-guide/` | 上手 / 插件开发 / 排障 |
| `docs/007-task/` | 在办任务（过程类；完成后由提交者删除，`git log` 保留） |
| `docs/archive/` | 历史：已废止机制、一次性评审、已落地的实施方案、已退场的决策记录；**不进登记** |
| 模块目录 | `symbio/src/plugins/<plugin>/README.md`（+ `docs/`）、`tauri/`、`cli/`、`e2e/`——就近放置，同一套登记机制管 |

**加一篇文档 = 建文件 + 写 front-matter**（`kind` / `id` / `title` / `status` / `created`，设计再加 `impl`），
文件名按编号规范起，`id` 与文件名一致；不需要改任何清单：汇总页由脚本重生成。

## 模块文档地图

| 模块 | 文档 | 一句话职责 |
|------|------|-----------|
| symbio_core | [symbio/src/symbio_core/README](../symbio/src/symbio_core/README.md) | 内核契约层：命名与结构规范 + 域清单 |
| session | [symbio/src/plugins/session/README](../symbio/src/plugins/session/README.md) | 会话编排唯一入口：工具循环、提示词组装、上下文压缩 |
| loop | [symbio/src/plugins/loop/README](../symbio/src/plugins/loop/README.md) | 单环驱动器：行表、事实层、执行体装配 |
| model | [symbio/src/plugins/model/README](../symbio/src/plugins/model/README.md) | 无状态单轮 LLM 网关（execute_turn），多协议适配 |
| agent | [symbio/src/plugins/agent/README](../symbio/src/plugins/agent/README.md) | 智能体域唯一所有者：agent 目录库、子树装配、子智能体委托 |
| mcp | [symbio/src/plugins/mcp/README](../symbio/src/plugins/mcp/README.md) | MCP 外部工具接入与能力注册 |
| skill | [symbio/src/plugins/skill/README](../symbio/src/plugins/skill/README.md) | 技能脚本发现与装载 |
| local | [symbio/src/plugins/local/README](../symbio/src/plugins/local/README.md) | 本地原生工具（shell / 内容与语义搜索 / 任务清单） |
| vdfs | [symbio/src/plugins/vdfs/README](../symbio/src/plugins/vdfs/README.md) | 文件系统本身：`vdfs/*` 协议入口 + `vdfs_*` LLM 工具 |
| web | [symbio/src/plugins/web/README](../symbio/src/plugins/web/README.md) | 网页抓取 / 搜索工具 |
| home | [symbio/src/plugins/home/README](../symbio/src/plugins/home/README.md) | 根插件：持应用级状态，构造 worker(Composite) |
| composite | [symbio/src/plugins/composite/README](../symbio/src/plugins/composite/README.md) | 子插件容器：扫描目录 + 路径合并分发 |
| memory | [symbio/src/plugins/memory/README](../symbio/src/plugins/memory/README.md) | 记忆的两个作用域 |
| setting | [symbio/src/plugins/setting/README](../symbio/src/plugins/setting/README.md) | 智能体自身的信息设置 |
| gateway | [symbio/src/plugins/gateway/README](../symbio/src/plugins/gateway/README.md) | HTTP/WS 入站网关 |
| plugin_manager | [symbio/src/plugins/plugin_manager/README](../symbio/src/plugins/plugin_manager/README.md) | 插件管理入口 |
| hook | [symbio/src/plugins/hook/README](../symbio/src/plugins/hook/README.md) | 生命周期钩子 |
| event_bus | [symbio/src/plugins/event_bus/README](../symbio/src/plugins/event_bus/README.md) | 进程内事件总线 |
| heartbeat | [symbio/src/plugins/heartbeat/README](../symbio/src/plugins/heartbeat/README.md) | 空闲会话的自主开轮调度 |
| tauri 前端 | [tauri/README](../tauri/README.md) → [DE-110-FRONTEND](../tauri/docs/110-FRONTEND.md) | Vue3 + Tauri2 桌面前端 |
| 门禁与脚本 | [scripts/facts/README](../scripts/facts/README.md) | 事实生成机制：每份 `.mjs` 投影出一份代码事实 |

> **维护原则**：本文是文档体系的入口；结构性变更（新增/删除目录、职责改属）先改此文件。
