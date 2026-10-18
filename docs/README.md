---
lists: [doc-boundaries=owner, kinds=owner]
kind: guide
id: docs/README
title: Symbio 文档中心
status: active
created: 2026-07-20
---

<!-- doc-link-allow D-004: 本文定义「现行文档不写变更史」这条规矩及其判据边界，需引用反例措辞（「曾经…已改为…」「五条 → 七条」）界定禁止范围 -->
# Symbio 文档中心

> **文档驱动开发**：本文档体系是 Symbio 项目的"单一事实来源"。

## 文档登记：每篇文档都是登记条目

全仓的活跃 `.md` 都在**同一套登记机制**里（机制见 [plan/PL-011](004-plan/011-登记与汇总机制.md "PL-011-登记与汇总机制")，字段见
[SP-001](008-spec/001.md)）：每篇文档开头是五行 front-matter，**文件本身就是条目**，没有第二处登记表。

    ---
    kind: arch
    id: AR-013-vdfs
    title: VDFS —— 虚拟动态文件系统规范
    status: active
    created: 2026-09-13
    ---

- **`kind` 回答「这篇文档在回答什么问题」**——十类，各有自己的目录与汇总页。每类的形状
  （编号前缀 / 状态机 / 汇总版式）由**该 kind 目录 `README.md` 的 meta 自描述**，机制不内建
  任何类型清单；下面的总账块（跨 kind 一览）由脚本投影生成：

<!-- registry:summary begin -->
<!-- list:kinds owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 条目各是一个单文档：条目类**在位存放**（文件即条目，kind 只来自 front-matter，目录不参与判定）；
> 各 kind 的形状与汇总版式由其 README meta 自描述；机制见 [PL-011 · 登记与汇总机制](004-plan/011-登记与汇总机制.md "PL-011-登记与汇总机制")，字段规范见 [SP-001](008-spec/001.md)。

| 类别 | 身份 | 回答什么 | 条目数 | 状态分布 | 分解 |
|---|---|---|---|---|---|
| [goal · 目标](001-goal/README.md "GO-000-目标汇总") | 编号前缀 `GO-` | 要达成什么——价值主张与可度量的验收判据 | 6 | draft 0 · active 6 · retired 0 | 3 条分解边 |
| [scene · 场景](002-scene/README.md "SC-000-场景汇总") | 编号前缀 `SC-` | 用户拿它做什么——角色、情境与用户判据 | 18 | draft 5 · active 13 · retired 0 | — |
| [plan · 计划](004-plan/README.md "PL-000-计划汇总") | 编号前缀 `PL-` | 要变成什么样——目标架构与实施方案 | 14 | draft 0 · active 14 · retired 0 | — |
| [arch · 架构](005-arch/README.md "AR-000-架构汇总") | 编号前缀 `AR-` | 现在是什么样——系统结构 / 设计规范 / 模块机制 | 49 | draft 0 · active 49 · retired 0 | — |
| [ref · 参考](006-ref/README.md "RE-000-参考汇总") | 编号前缀 `RE-` | 查数据——路由 / 错误码 / 配置等权威表 | 8 | draft 0 · active 8 · retired 0 | — |
| [guide · 指南](007-guide/README.md "GU-000-指南汇总") | 编号前缀 `GU-` | 怎么用——上手 / 贡献 / 排障 | 14 | draft 0 · active 14 · retired 0 | — |
| [spec · 规范](008-spec/README.md "SP-000-规范汇总") | 编号前缀 `SP-` | 必须成立的条文——机制与判据的规范面 | 31 | draft 0 · active 31 · retired 0 | — |
| [task · 任务](009-task/README.md "T-000-任务汇总") | 编号前缀 `T-` | 要做什么、做到哪一步——阶段任务 / SPRINT 任务分解子任务，可勾销的最小工作单元 | 164 | todo 7 · doing 2 · done 152 · wont 3 | 58 条分解边 |
| [adr · 架构决策](010-adr/README.md "ADR-000-架构决策汇总") | 编号前缀 `ADR-` | 为什么这样设计——决策、理由与被否决的方案 | 76 | active 66 · superseded 7 · withdrawn 3 | — |

<!-- registry:summary end -->

- **编号规范全仓一套**：细则（id 形态 / 文件名 = 去前缀主干 / 前缀与 kind 对账 / 导航文档
  发编号的豁免）见 [SP-001](008-spec/001.md "SP-001")，本页不复述——写错会被
  RG-003 / RG-022 / RG-023 判红。

### 目标分层与依赖链（goal / plan / task 怎么衔接）

这三个 kind 是**同一条链上的三层**，不是三个平铺的分类。层界不清，分工就含糊——
所以三层怎么衔接**只在本节解释一次**：各层文档只指针到这里，不各自复述。

| 层 | kind | 依赖指向 |
|---|---|---|
| 目标 | `goal` | 链根，无上游；判据的 owner 在各判据脚本 |
| 计划 | `plan` | 上游计划与目标（各篇正文的「上游」块） |
| 任务 | `task` | 前置任务（`deps`）；阶段任务 / SPRINT 任务用 `parent` 分解出子任务（可多级） |

> 原「阶段」层（`stage` kind，ST-001…013）已于 2026-10-16 废除：阶段属**过程类**文档，
> 整体并入任务体系——路线图 = [T-120](009-task/120-路线图总览.md "T-120-路线图总览")，
> 13 份阶段文档 = T-121–133（T-120 的子任务），各阶实现任务挂对应阶段任务名下（`parent`）。

- **机读关系有两个字段，各答一个问题**（字段规范见 [SP-001](008-spec/001.md "SP-001")）：
  `deps` 答「必须先有什么」（前置义务），`parent` 答「是谁的一部分」（分解归属，只许同 kind）。
  两者的值域、强制性与判据（RG-006 / RG-008 / RG-024 / RG-025）都在 SP-001 与
  [plan/PL-011 · 状态流转](004-plan/011-登记与汇总机制.md "PL-011-登记与汇总机制")，本页不复述。
- **判据只引用、不复制**：下层文档引用上层的判据编号（目标的可度量判据、阶段的验收断言），
  不把数字抄进自己正文——「同一个事实写了多遍」是文档腐化的起点。

- **`id` 是文档的身份，`status` 是唯一的状态处**：字段语义、状态机与两面对账判据见
  [SP-001](008-spec/001.md "SP-001")；各 kind 的状态词由该 kind README meta 的 `status.*` 声明。
- **目录不参与判定**：`kind` 只来自 front-matter，文档搬去别的目录不会破机制，只需把 `id`
  与它一起改（判据会拦）。**模块文档就地登记**（`symbio/src/plugins/*/README.md` 与模块 `docs/`
  不搬进 `docs/`）——「就近放置」优先于「按类归堆」。
- **引用只写编号**：文档之间互相引用时只写 `[[编号]]`（如 `[[SP-001]]`、`[[ADR-047]]`；
  要显示文字写 `[[编号|显示文字]]`），**地址既不手写也不维护**——
  由 `node scripts/gen-doc-refs.mjs` 从登记解析生成，并且每次门禁**重算全仓已展开的地址**
  （文档搬走后入链自愈）。判据三条：编号不存在判红（RF-001）、
  地址漂移自动重算（RF-002）、**指向已登记文档的手工地址判红**（RF-003——那种地址机制搬不动）。
  编号是身份（稳定），地址是位置（会变）。
- **汇总页是投影**：各 kind 的汇总块住在本 kind 目录 `README.md` 的 `<!-- registry:summary -->`
  标记区，跨 kind 总账块住在本页（上方），都由 `node scripts/registry-audit.mjs --write`
  生成（门禁自动重跑），**标记区之外手写、标记区之内不要手改**。查询看 `node scripts/registry-board.mjs`。

## 文档下沉原则

文档**就近放置、单一来源**：

- **单模块文档**放在该模块目录内（`README.md` + 可选 `docs/`），只写该模块自身机制，不重复系统级内容；
- **系统级文档**（本目录）只保留**跨模块**的核心逻辑、边界与约定，涉及单模块内部实现时**引用**模块文档，不复制细节；
- **历史实施记录**一律进 `archive/`（归档不进登记），现行文档只描述当前行为。

下面这些由 `node scripts/doc-link-audit.mjs` 机械判定（任一命中即失败）：**站内链接必须有效**、
**过程文档不得滞留活跃目录**（D-002）、**活跃文档不得超过 800 行**（D-003）、**正文不得写
变更史**（D-004——历史归 `git log`）、**反引号里指路的文件路径必须存在**（D-006）、
**链接的 `#锚点` 必须指向目标文件真实存在的标题**（D-007，含纯锚点 `#x`）、
**裸 `R<数字>` 必须是风险表定义行的首列、或紧邻「风险」的限定引用**（D-008）。
行数上限的作用不是控制篇幅，而是给「职责失守」一个不会误报的信号：超限即按下面这张表
处置，没有豁免。

D-004 判**措辞标记**而不判**叙事**，边界在此写明（免得读到「写了历史却没被拦」时以为守卫失灵）：
判**词表与带量词的计数箭头**——`曾经` / `已改为` / `修订（` / `后记（` / `初版` /
`五条 → 七条`（箭头两侧都带量词才判）；**不判**裸数字箭头（`56 → 46`，04 §3.1 的状态表合法
记着测试数增减）与叙事式历史（「一度掉到 20/42」）。这条边界不是疏漏，是同一条实测教训：
只收「不会出现在正常论述里的措辞」，因为**守卫一旦开始误报，下场是被整体豁免，比没有还糟**。
豁免分两档（与 D-006 同口径）：**行内**（`<!-- doc-link-allow D-004: 理由 -->` 写在本行或
前一行）只豁免该行；**全文**须为文件首行非空内容，且理由写明它买断的是什么
（如 [SP-028](008-spec/028.md) 那篇豁免登记，`## 处理过程` 是唯一的过程信息）。

D-006 是 D-001 的盲区补丁：D-001 只认 Markdown 链接语法（方括号文字 + 圆括号目标），
而本仓正文**更常**用行内反引号指路（「详见 `docs/005-arch/013-vdfs.md`」）——这类写法 D-001
一个字都看不见。它是启发式判定（缩写式引用与示意性路径在文本上与真断链不可分），故留
豁免出口 `<!-- doc-link-allow D-006: 理由 -->`：**行内**（本行或前一行）只豁免该行，
**全文**须为文件首行非空内容。

D-007 是 D-001 的**另一处**盲区：D-001 判定前会把 `#…` 整段丢掉，所以「文件在、
锚点指向的标题不在」一直无人守（ADR 拆成索引 + 分域正文后锚点面翻了数倍，暴露出
3 条早已存在的失效）。它是精确判定（标题存在与否没有中间态），**不给豁免**。

D-008 判**形态**而不判**解析**：本仓的风险编号出过一次**撞号**——`04` 风险登记表里的
那条，与从 `feat` 分支并入的另一篇文档里的**同号**编号各指一事（后者那篇不在本仓，
引用悬空却落进了前者的编号空间）。两个同号编号**都能解析**，解析会把**错误的含义**判成
通过——「守卫报 0 不等于没有坏链，只等于它看不见」。机械可判的只有「这个编号是定义、
是限定引用、还是裸的」，而撞号正是裸编号的产物。精确判定，**不给豁免**：处置只有两种
（写成 `| Rn |` 表行首列 / 紧邻「风险」二字），都不需要解释。

**文档文件名不含空格**（用 `-` 或直接相连，如 `008-工具轮v2化实施方案.md`、`008.md`）。D-001 不做 URL 解码，所以按 Markdown 惯例写成
`%20` 的链接（`./10-工具轮%20v2%20化.md`）会被判为**失效**——报错信息里路径看着
"对"，实际是转义后的串与磁盘名不相等。带空格的名字只在**标题**里出现，不在文件名里。

文档是**下沉**的，所以「某条约定写在哪」要靠检索而不是靠记：
`node scripts/doc-find.mjs <关键词>` 会同时搜 `*.md` 与源码里的 `//!` / `///`
（相当一部分机制就写在模块文档注释里，如 `providers::memory`）。
**知识只写一处**——不要以摘要形式复制到别处，复制必然漂移。

示例：会话上下文压缩的 L0-L6 分层总览在 [session/docs/014-context-compression-design.md](../symbio/src/plugins/session/docs/014-context-compression-design.md "AR-014-context-compression-design")，各层阈值与代码实现在 [session/README.md](../symbio/src/plugins/session/README.md "symbio/src/plugins/session/README")——**会话相关的一切文档都在 `symbio/src/plugins/session/docs/` 内**；系统级目录只保留跨模块规范（如 [arch/013-vdfs.md](005-arch/013-vdfs.md "AR-013-vdfs")）。

## 文档职责边界（一个事实只有一个 owner）

**重复描述 = 双倍维护成本。** 同一事实写两处，改代码时就要改两处，且两处必然漂移。
因此每类事实只有一个 owner，其余位置一律**引用**，不复述：

<!-- list:doc-boundaries owner -->

| 事实 | 唯一 owner | 其它文档 |
|---|---|---|
| 现在是什么（插件 / 挂载点 / 路由 / 工具 / 规模 / **每条能力的用户可见语义**） | [CURRENT.md](006-ref/006-当前事实表.md "RE-006-当前事实表")（代码生成，能力矩阵在 §2） | **不手抄**，需要就引用。机械判据见下「这张表有人守吗」 |
| 路由**契约**（调用方 / 返回类型 / 备注 / 退役索引） | [ref/003-ROUTES.md](006-ref/003-ROUTES.md "RE-003-ROUTES") | 模块 `README.md` 不抄路由表 |
| 路由**一句话职责**（写在注册处：`route()` 臂上的注释） | [CURRENT.md](006-ref/006-当前事实表.md "RE-006-当前事实表") §2.3（代码投影） | RE-003 不再手写「用途」列——同一句事实不存两份 |
| 错误码 | [ref/002-ERROR_CODES.md](006-ref/002-ERROR_CODES.md "RE-002-ERROR_CODES") | PROTOCOLS 不再列错误码表 |
| 配置项 | [ref/001-CONFIGURATION.md](006-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION") | 模块 `README.md` 只写机制 + 指向它 |
| 判据码（各守卫的编号地址） | [ref/GATE_CODES.md](./006-ref/GATE_CODES.md)（脚本声明生成） | 守卫脚本只写自己判什么，不抄全表 |
| 出厂 prompt 的**形状基线** | [ref/PROMPT_SHAPE.md](./006-ref/PROMPT_SHAPE.md)（真实链路生成，不手改） | 口径（什么进判据 / 规模只作观测）在 ADR-057，别处不复述 |
| 结构化 vs 拍平的**模型侧对照读数** | [ref/PROMPT_AB.md](./006-ref/PROMPT_AB.md)（门外实验生成，不手改） | 判据口径在 `e2e/ab/tasks.mjs`，结论与边界在 task/T-048 |
| 协议线上形状（帧 / 载荷 / 通道 / 线格式） | [arch/004-PROTOCOLS.md](005-arch/004-PROTOCOLS.md "AR-004-PROTOCOLS") | — |
| 系统拓扑（谁挂在谁下面） | [arch/005-SYSTEM_MAP.md](005-arch/005-SYSTEM_MAP.md "AR-005-SYSTEM_MAP") | 其它图只画自己那一层 |
| 排障链路（一次请求经过哪些代码） | [arch/002-DATA_FLOW.md](005-arch/002-DATA_FLOW.md "AR-002-DATA_FLOW") | — |
| 为什么这样设计（含被否决的方案与理由） | [`docs/010-adr/`](010-adr/README.md "ADR-000-架构决策汇总") 下的单文档（**每条 ADR 正文 + 状态**）；索引与域分组见 [adr 索引](010-adr/README.md "ADR-000-架构决策汇总")（生成） | 其它文档只引用 ADR 编号 |
| 模块内部机制 | 该模块 `README.md` | 系统级文档不复述 |
| 模块深度设计与不变量 | 该模块 `docs/` | — |
| **要达成什么（价值主张与可度量判据）** | [goal/000-价值主张.md](001-goal/000-价值主张.md "GO-000-价值主张") | 其余文档只引用编号，不抄判据数字 |
| **用户拿它做什么（角色 / 场景 / 用户判据）** | [scene/README.md](002-scene/README.md "SC-000-场景汇总") | 其余文档只引用编号，不复述场景定义与用户判据 |
| **目标架构（v2）与迁移计划** | [plan/README.md](004-plan/012-高阶智能体平台v2.md "PL-012-高阶智能体平台v2") | 其余文档只引用，不复述取舍 |
| 条目级登记（任务 / 规范 / 决策的编号、状态、优先级、依赖、处理过程） | `docs/<kind>/` 下的**单文档**（机制见 [plan/PL-011](004-plan/011-登记与汇总机制.md "PL-011-登记与汇总机制")；汇总页由脚本生成） | 各处不手抄条目清单，引用各 kind 汇总页 |
| 变更历史（改了什么、何时改的） | `git log` | **现行文档不写变更史**（见下） |
| 一次性评审 / 迁移记录 / 体检报告 | `docs/archive/` | 活文档不保留过程日志 |

**变更时只改 owner 那一处**：结构事实重跑 `node scripts/gen-facts.mjs`（门禁自动重生成并暂存）；跨模块架构改 [003-OVERVIEW.md](005-arch/003-OVERVIEW.md "AR-003-OVERVIEW") + [005-SYSTEM_MAP.md](005-arch/005-SYSTEM_MAP.md "AR-005-SYSTEM_MAP") 各自己那一层；常见问题进 [007-TROUBLESHOOTING.md](007-guide/007-TROUBLESHOOTING.md "GU-007-TROUBLESHOOTING")。

### 这张表有人守吗

**「一个事实只有一个 owner」此前是一张没有人执行的承诺**——T-098 把能力矩阵加进
`CURRENT.md` 时没有撤掉它覆盖掉的旧节，同一批事实在同一份文件里并存两遍，而其中一列是
人写的——它看起来和其它列一样是「从代码来的」，于是漂移不可见。现在这一层由判据守着
（`scripts/facts.test.mjs`，随门禁跑）：**文件内**一个封闭集只在一处成表、**跨文档**非 owner
文档的表里含 ≥3 个成员即判红（散文点名不算）、**owner 收留**——真源加了东西而表没跟上判红。
集合声明只有一份，住在 [`scripts/facts/_sets.mjs`](../scripts/facts/_sets.mjs)，owner 从各文档
front-matter 的 `lists: [<id>=owner]` 读（与 [RE-007-清单归属总账](006-ref/007-清单归属总账.md)
同一份输入）。已知边界：判据抓的是「抄一张清单」，抓不到「拿它当索引」。
**新增一节的纪律**：写下新节时先问**它覆盖了谁**——覆盖了就要同时删掉旧 owner 或把它改成
一行指路，否则新判据会红（那正是它的工作）。

### 两条硬规则

1. **现行文档只描述「现在是什么」**，不写「曾经是什么、后来改成了什么」。
   这类追溯一律走 `git log -S<符号>` / `git log --grep=<词>`；需要保留「为什么否决 A 选 B」的
   结论，就写进 `docs/010-adr/` 的一条 ADR——**那才是它的 owner**。
   判据：一条叙述如果随每次重构都要跟着改，它就不该活在现行文档里。
2. **代码注释与文档各有边界**：注释写「读这个文件需要知道的不变量与陷阱」，
   文档写「机制与取舍」。**注释不复述文档，文档不复述注释**（签名、方法清单、字段表以代码为准）。
   详见 [CONTRIBUTING.md §4](../CONTRIBUTING.md "CONTRIBUTING")。

## 纪律地图

**纪律的 ID 是限定名**（如 `08 §0 纪律 1`）：编号只在定义篇内有意义——08 与 13 各有一套
「纪律 3」且不同义。**跨篇引用必须带限定名**；同篇内的裸引（「纪律 3」）指**本篇 §0** 的那条。
不新开编号命名空间。

每条纪律的**登记**（限定名 + 判据对账）在 [docs/008-spec/](008-spec/README.md "SP-000-规范汇总")
（机制见 [plan/PL-011](004-plan/011-登记与汇总机制.md "PL-011-登记与汇总机制")）——「哪条纪律其实没人守」由登记条目的判据节
回答，判据码全表在 [ref/GATE_CODES.md](./006-ref/GATE_CODES.md)。条文正文各归定义篇，
此处不复述：[PL-006 §0](004-plan/006-分步实施计划.md#0-执行纪律五步每步都不得跳过 "PL-006-分步实施计划") ·
[SP-010…SP-014](008-spec/README.md "SP-000-规范汇总") ·
[PL-003 §5.1](004-plan/003-演进与验证.md#51-五条纪律 "PL-003-演进与验证") ·
[plan/README §4](004-plan/012-高阶智能体平台v2.md#4-硬证据全部实际编译运行通过 "PL-012-高阶智能体平台v2")。

## 快速导航

| 我想... | 查阅 |
|---------|------|
| 核对"现在是什么"（插件 × 挂载点 × 路由 × 工具，自动生成） | [CURRENT.md](006-ref/006-当前事实表.md "RE-006-当前事实表") |
| 了解系统全貌 | [005-SYSTEM_MAP.md](005-arch/005-SYSTEM_MAP.md "AR-005-SYSTEM_MAP") |
| 理解架构设计 | [003-OVERVIEW.md](005-arch/003-OVERVIEW.md "AR-003-OVERVIEW") |
| 查看协议规范 | [004-PROTOCOLS.md](005-arch/004-PROTOCOLS.md "AR-004-PROTOCOLS") |
| 追踪请求全链路 | [002-DATA_FLOW.md](005-arch/002-DATA_FLOW.md "AR-002-DATA_FLOW") |
| 查找路由路径 | [003-ROUTES.md](006-ref/003-ROUTES.md "RE-003-ROUTES") |
| 理解错误码 | [002-ERROR_CODES.md](006-ref/002-ERROR_CODES.md "RE-002-ERROR_CODES") |
| 配置系统 | [001-CONFIGURATION.md](006-ref/001-CONFIGURATION.md "RE-001-CONFIGURATION") |
| 给门禁加一条判据、要挑不撞的号 | [GATE_CODES.md](./006-ref/GATE_CODES.md) |
| 看 prompt 实际送出去的形状 / 结构化的代价 | [PROMPT_SHAPE.md](./006-ref/PROMPT_SHAPE.md)（重生成：`PROMPT_SHAPE_UPDATE=1 node e2e/cases/t46-prompt-shape-baseline.mjs`） |
| 看「结构化 vs 拍平」在真实模型上的对照读数 | [PROMPT_AB.md](./006-ref/PROMPT_AB.md)（门外实验：`node e2e/ab/run.mjs`，会打真 provider、不进门禁） |
| 快速上手 | [006-QUICK_START.md](007-guide/006-QUICK_START.md "GU-006-QUICK_START") |
| 开发新插件 | [005-PLUGIN_DEVELOPMENT.md](007-guide/005-PLUGIN_DEVELOPMENT.md "GU-005-PLUGIN_DEVELOPMENT") |
| 排查问题 | [007-TROUBLESHOOTING.md](007-guide/007-TROUBLESHOOTING.md "GU-007-TROUBLESHOOTING") |
| 看**全项目登记总账**（十类 kind × 条目数 × 状态分布） | 本页上方总账块（生成）；查询台 `node scripts/registry-board.mjs` |
| 了解设计决策 | [adr 索引](010-adr/README.md "ADR-000-架构决策汇总")（按域分组；正文即条目本身） |
| 了解平台**要达成什么**（价值主张与可度量判据） | [goal/GO-000 · 价值主张](001-goal/000-价值主张.md "GO-000-价值主张") |
| 看**用户拿它做什么**（角色 / 场景 / 用户判据） | [scene/SC-000 · 场景总览](002-scene/README.md "SC-000-场景汇总") |
| 了解**目标架构（v2）**与分步迁移计划 | [plan/README.md](004-plan/012-高阶智能体平台v2.md "PL-012-高阶智能体平台v2")（**先读其 §0.1：v2 是本仓的目标架构**） |
| 查「某条约定写在哪」 | `node scripts/doc-find.mjs <词>`（全仓 md + Rust 文档注释检索） |
| 查**任务 / 规范登记**与待办 | `node scripts/registry-board.mjs`（十类条目同一机制，规则见 [plan/PL-011](004-plan/011-登记与汇总机制.md "PL-011-登记与汇总机制")） |
| 看某个插件/前端的职责与机制 | 各模块 `README.md`（见下方模块文档地图） |

## 文档放在哪

**目录清单以文件系统为准**（`ls docs/`、`ls docs/archive/`）——本文不抄一份会漂移的副本。各目录的定位：

| 位置 | 放什么 |
|------|--------|
| `docs/`（本目录） | 文档中心入口：`README.md`（本文，含跨 kind 总账块，生成）、各 `<NNN>-<kind>/` 编号目录 |
| `docs/001-goal/` · `docs/004-plan/` | 目标 / 目标架构与实施方案（路线图阶梯 = 任务 T-120–133，见 [T-120](009-task/120-路线图总览.md "T-120-路线图总览")）；`plan/verify/` 是随计划的可编译验证程序（Rust） |
| `docs/005-arch/` | 系统结构与设计规范：架构总览 / 协议形状 / 排障链路 / VDFS / agent 目录 / HTTP 传输… |
| `docs/006-ref/` | 路由 / 错误码 / 配置的权威参考页（含生成物 GATE_CODES / PROMPT_SHAPE / PROMPT_AB） |
| `docs/007-guide/` | 上手 / 插件开发 / 排障 |
| `docs/009-task/` · `docs/008-spec/` · `docs/010-adr/` | 任务 / 规范 / 决策三类条目（每条一个单文档，文件名即编号） |
| `docs/archive/` | 历史归档：已废止的旧机制 / 旧规范、一次性评审与体检、已落地的实施方案与迁移；**不进登记**，**变更历史以 `git log` 为准，本仓库不维护 CHANGELOG** |
| 模块目录 | `symbio/src/plugins/<plugin>/README.md`（+ 可选 `docs/`）、`tauri/`、`cli/`、`e2e/`——就近放置，同一套登记机制管（`kind` 按内容定，不按目录定） |

**加一篇文档 = 建文件 + 写 front-matter**（五行：`kind` / `id` / `title` / `status` / `created`），
文件名按编号规范起（`<前缀>-<序号>[-名称].md`，前缀见各 kind README meta），`id` 与文件名一致；
不需要改任何清单：汇总页由脚本重生成，漏了 front-matter 的 `.md` 会被 RG-020 判红，
编号不合规范会被 RG-022 判红。

## 模块文档地图

| 模块 | 文档 | 一句话职责 |
|------|------|-----------|
| symbio_core | [symbio_core/README.md](../symbio/src/symbio_core/README.md "symbio/src/symbio_core/README") | 内核契约层：命名与结构规范 + 域清单（trait / 协议类型 / 词表 / 纯工具） |
| session | [plugins/session/README.md](../symbio/src/plugins/session/README.md "symbio/src/plugins/session/README") | 会话编排唯一入口：工具循环、提示词组装、上下文压缩 |
| model | [plugins/model/README.md](../symbio/src/plugins/model/README.md "symbio/src/plugins/model/README") | 无状态单轮 LLM 网关（execute_turn），多协议适配 |
| agent | [plugins/agent/README.md](../symbio/src/plugins/agent/README.md "symbio/src/plugins/agent/README") | 智能体域唯一所有者：agent 目录库、子树装配、子智能体委托（agent_run） |
| mcp | [plugins/mcp/README.md](../symbio/src/plugins/mcp/README.md "symbio/src/plugins/mcp/README") | MCP 外部工具接入与能力注册 |
| skill | [plugins/skill/README.md](../symbio/src/plugins/skill/README.md "symbio/src/plugins/skill/README") | 技能脚本（loader/plugin）发现与装载 |
| local | [plugins/local/README.md](../symbio/src/plugins/local/README.md "symbio/src/plugins/local/README") | 本地原生工具（shell / 内容与语义搜索 / 任务清单）；文件操作已迁 VDFS |
| vdfs | [plugins/vdfs/README.md](../symbio/src/plugins/vdfs/README.md "symbio/src/plugins/vdfs/README") | 文件系统本身：`vdfs/*` 协议入口 + `vdfs_*` LLM 工具（规范见 [arch/013-vdfs.md](005-arch/013-vdfs.md "AR-013-vdfs")） |
| web | [plugins/web/README.md](../symbio/src/plugins/web/README.md "symbio/src/plugins/web/README") | 网页抓取/搜索工具 |
| home | [plugins/home/README.md](../symbio/src/plugins/home/README.md "symbio/src/plugins/home/README") | 根插件：持应用级状态（`<homedir>/PLUGIN.yml`），构造 worker(Composite) 并传入必需插件清单 |
| composite | [plugins/composite/README.md](../symbio/src/plugins/composite/README.md "symbio/src/plugins/composite/README") | 子插件容器：扫描自己的目录（其下一层目录即一个插件）+ 路径合并分发 |
| memory | [plugins/memory/README.md](../symbio/src/plugins/memory/README.md "symbio/src/plugins/memory/README") | 记忆（两个作用域）：智能体自身（分形 `{宿主目录}/AGENTS.md`）+ 工作区（`{workdir}/AGENTS.md`），经 `<根>/memory` 编辑 |
| setting | [plugins/setting/README.md](../symbio/src/plugins/setting/README.md "symbio/src/plugins/setting/README") | 当前智能体自身的信息设置：档案（名字 / 描述）+ 偏好（语言 / 详略） |
| gateway | [plugins/gateway/README.md](../symbio/src/plugins/gateway/README.md "symbio/src/plugins/gateway/README") | HTTP/WS 入站网关 |
| plugin_manager | [plugins/plugin_manager/README.md](../symbio/src/plugins/plugin_manager/README.md "symbio/src/plugins/plugin_manager/README") | 插件管理入口：列全部插件 + 启用/停用/添加/卸载 + 各插件配置条目 + 自有分区（appearance/about） |
| hook | [plugins/hook/README.md](../symbio/src/plugins/hook/README.md "symbio/src/plugins/hook/README") | 生命周期钩子 |
| event_bus | [plugins/event_bus/README.md](../symbio/src/plugins/event_bus/README.md "symbio/src/plugins/event_bus/README") | 进程内事件总线 |
| telegram | [plugins/telegram/README.md](../symbio/src/plugins/telegram/README.md "symbio/src/plugins/telegram/README") | Telegram 通道接入 |
| tauri 前端 | [tauri/README.md](../tauri/README.md "tauri/README") → [docs/023-FRONTEND.md](../tauri/docs/023-FRONTEND.md "AR-023-FRONTEND") | Vue3 + Tauri2 桌面前端 |
| 门禁与脚本 | [scripts/facts/README.md](../scripts/facts/README.md "scripts/facts/README") | **事实生成机制**：目录下每份 `.mjs` 投影出一份代码事实（结构事实表 / 判据码登记表 / 前端路由常量…），runner 遍历生成；目录即接线 |

> **维护原则**：本文是文档体系的入口；结构性变更（新增/删除目录、职责改属）先改此文件。单条事实的增改只动它的 owner，不必回头改本文。
