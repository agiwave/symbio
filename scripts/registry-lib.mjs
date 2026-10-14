#!/usr/bin/env node
/**
 * registry-lib — 登记机制共享库：单文档解析、schema 校验、汇总渲染
 *
 * ## 这套顶层机制是什么
 *
 * 项目里的汇总文档（待办清单 / 规范清单 / 批次台账…）此前都是**手写表格**：
 * 状态流转靠人改格子、依赖倒置没人判、「表里的编号指到不存在的东西」无人核对——
 * 正是 `plan/13` 纪律 2（机制优先）与纪律 4（出口可核对）要治的形态。
 * 顶层解法只有三段：
 *
 *   1. **条目 = 数据**：每条任务 / 规范 / 决策 / **文档**都是一个带 front-matter 的单文档，
 *      编号、状态、依赖、证据全在机读字段里。**全仓一套编号规范**（RG-022 判形状）：
 *        · **编号条目**（九类 kind 都是）：`id` = 文件名（去 `.md`）=
 *          `<前缀>-<三位序号>[-名称]`，前缀闭集 `GO/ST/PL/T/ADR/SP/AR/RE/GU` 一 kind 一个；
 *        · **导航文档**（`README` / `INDEX`）：不发编号——通用名没有编号段、不构成命名空间，
 *          `id` = 仓根相对路径（去 `.md`），**禁止进 `deps`**（RG-023：依赖图只收编号条目）。
 *      文档类（goal/stage/plan/arch/ref/guide）仍**在位存放**（就近放置优先于按类归堆），
 *      `title` = 正文首个 H1（两面对账，RG-021）。
 *      **编号平铺不体现层级**：分解关系（总目标→子目标、大任务→小任务）走 `parent` 字段
 *      （RG-024 结构 / RG-025 父子状态一致性），层级只是投影（INDEX 分解树小节、board
 *      `--children`）——与「目录不参与判定」同款哲学：位置与层级都会变，身份不朽。
 *   2. **汇总 = 投影**：任何「列表 / 看板 / 总账」都由本库渲染，
 *      `registry/` 下各级 `INDEX.md` 由门禁自动重生成（`gate.d/60-facts.mjs` 的 `autoWork`），
 *      不存在第二处手抄；
 *   3. **校验 = 守卫**：`scripts/registry-audit.mjs` 把字段 / 状态机 / 依赖 /
 *      优先级 / 取代链 / 判据指认 / **未登记文档** / **分解**全部机械判定（RG-001…RG-025），
 *      违规即红，范围读不出 exit 2。
 *
 * ## 受限 YAML 子集（front-matter）
 *
 * 零外部依赖（Node 无内置 YAML），所以**只认以下形状**，其余一律报 RG-001（宁红不漏）：
 *
 *   - 首行 `---` 起始、`---` 闭合；键形如 `key: value`（取**第一个**冒号之后整段为值）；
 *   - **值与键同一行**：块标量 `|` / `>` 与缩进续行都不解析——续行没有冒号，
 *     按下面那条「不合形状的行」判红，不会静默吃掉半截值；
 *   - 整行注释以 `#` 开头；**行内 `#` 属于值的一部分**（不剥离，避免吞掉正当内容）；
 *   - 值四种形态：裸标量、`"..."` / `'...'`（去外层引号，不支持转义）、
 *     `[]` / `[a, b]` 行内列表、键后换行的 `  - item` 块列表；
 *   - 未知字段 / 重复字段 / 不合形状的行 ⇒ RG-001（未知字段按 kind 判，
 *     错域字段如 task 里写 `scope` 会被指名）。
 *
 * ## 判据码的归属
 *
 * 全部 RG 码声明在**本文件**（消息字面量住这里，`gate-codes-audit` GC-003 要求
 * 声明的码在本文件里出现）。CLI 出口只有两个：`registry-audit`（守卫）与
 * `registry-board`（查询），它们只 import，不重复判据。
 */
// 判据码命名空间（登记表 docs/ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns RG 登记条目校验（registry 单文档的字段 / 状态 / 依赖 / 判据指认 / 分解）
// @codes RG-001 RG-002 RG-003 RG-004 RG-005 RG-006 RG-007 RG-008 RG-009 RG-010 RG-011 RG-012 RG-013 RG-014 RG-015 RG-016 RG-017 RG-018 RG-019 RG-020 RG-021 RG-022 RG-023 RG-024 RG-025 RG-026 RG-027 RG-029

import fs from 'node:fs'
import path from 'node:path'

// ==================== schema 配置 ====================

/** 公共必填字段：所有 kind 一致。 */
export const COMMON_REQUIRED = ['kind', 'id', 'title', 'status', 'created']
/**
 * 公共可选字段：
 * - `deps` 是**前置义务**（依赖图的边，允许跨 kind——全 registry 编号唯一才敢跨）；
 * - `parent` 是**分解归属**（父子分解树的边，只许同 kind，RG-024）——「总目标分解出子目标、
 *   大任务分解出小任务、产品功能分解出子功能」这类「整体与部分」的关系。两者语义正交：
 *   deps 回答「必须先有什么」，parent 回答「是谁的一部分」。同一对条目之间可以两条都有
 *   （子任务的前置是别的任务，同时它是大任务的一部分），互不替代。
 */
export const COMMON_OPTIONAL = ['deps', 'parent', 'lists', 'projection']

/**
 * `lists` 的 role 词表（RG-026 封闭）：**只有 `owner`——我维护这份清单**。
 *
 * 曾有过第二个角色 `view`（我只呈现 / 引用它），已**整体取消**：`lists` 这套机制
 * （RG-027…RG-029）的初衷就是**消灭重复**——一个事实只在一个文档里出现，那篇文档
 * 就是它的 owner。而 `view` 给的是一条绕开重复的旁路：A 维护、B 用投影重渲一遍，
 * 「没有第二个手写 owner」成立，**读者面前的事实仍有两份**（生成的重复也是重复），
 * 且投影会随 B 的组织需要越铺越大。要读另一份清单就**指路**（正文写 `[[编号]]`），
 * 不重渲。同理 `mirror`（镜像副本）也永不给合法角色——镜像正是重复的别名。
 *
 * 想恢复 view = 改这里 + 改 SP-001 + 复活 RG-028，那道成本就是「迫不得已」的门槛。
 */
export const LIST_ROLES = ['owner']

/**
 * 每个 kind 一份配置——**新增一个 kind = 在这里加一项 + 一个渲染器 + 回归测试**，
 * 不新开脚本、不新开命名空间。字段事实的文档侧真相是 `docs/spec/SP-001.md`
 * （其字段表由 RG-011 与本配置对账，两边不等即红）。
 */
/**
 * 文档类 kind 的**共享状态机**：`draft`（草案）/ `active`（现行）/ `retired`（废止）。
 * 与 `spec` 同轴——文档只有一条状态轴：**这篇文档还算不算数**；「做到哪一步」由
 * `task` 承载，不压进文档的状态列（同 adr 的 status/impl 拆轴之理）。
 */
const DOC_STATUS_RULES = {
  draft: { require: [], forbid: ['reason'] },
  active: { require: [], forbid: ['reason'] },
  retired: { require: ['reason'], forbid: [] },
}

/**
 * 文档类 kind 的通用形状：**在位存放、正文即条目**——所以没有必有小节
 * （正文结构由文档自己定，按 kind 强切只会逼出空壳小节），编号 = 文件名。
 * `blurb` 是这个 kind 回答的问题（写进汇总页，兼作新文档选 kind 的指路）。
 * `titleEqualsH1`：文档类的 `title` 必须逐字等于正文首个 H1（RG-021，两面对账）；
 * 条目类（task / adr / spec）的 title 带编号前缀惯例，不判。
 *
 * `requiresDepsDone: ['active']` 与 `spec` 同款：**依赖是义务，不是注释**——
 * `deps` 对全部 kind 同一套强制性机制（RG-006 存在性 / 不自指 / 不重复 / 不成环
 * + RG-023 值域：只收编号条目 + RG-008 状态门槛），文档类不搞特殊化：
 * `active` 断言「这篇文档现行」，其前置就必须已闭环（闭环态按目标 kind 取：
 * task = `done`，其余 = `active`）。`draft` / `retired` 不在门槛里——草案可以引草案，
 * 废止不因前置废止而红。
 *
 * `retiredStates` 是**退场档**（RG-025 读它）：父条目进入退场档 ⇒ 不得有子条目停在生效档
 * （「大任务已经 done/wont，还有小任务 doing」是状态谎报）。与 `requiresDepsDone`
 * （生效档）成对：子生效 ⇒ 父生效；父退场 ⇒ 子退场——父子状态档位单调。
 */
const docKind = (label, blurb, retiredStates = ['retired']) => ({
  label,
  blurb,
  titleEqualsH1: true,
  required: [],
  optional: [],
  sections: [],
  refFields: [],
  refSections: [],
  statusRules: DOC_STATUS_RULES,
  closedStatus: 'active',
  requiresDepsDone: ['active'],
  retiredStates,
})

export const KINDS = {
  goal: docKind('目标', '要达成什么——价值主张与可度量的验收判据'),
  stage: docKind('阶段', '分几步走、现在在哪一步——路线图与能力扩充阶段'),
  // `index`：跨 kind 的登记总账（`docs/KIND-000-登记总账.md`）。它是**投影页**，
  // 与各 kind 的 `<前缀>-000-<X>汇总.md` 同形，只是位置在 `docs/` 根而不是某个
  // kind 目录——目录不参与判定（kind 只来自 front-matter），所以它**不需要**自己的目录。
  index: { ...docKind('总账', '跨 kind 的登记总账——各 kind 条目数 / 状态分布 / 分解边数'), blurb: '跨 kind 的登记总账' },
  plan: docKind('计划', '要变成什么样——目标架构与实施方案'),
  arch: docKind('架构', '现在是什么样——系统结构 / 设计规范 / 模块机制'),
  ref: docKind('参考', '查数据——路由 / 错误码 / 配置等权威表'),
  guide: docKind('指南', '怎么用——上手 / 贡献 / 排障'),
  task: {
    label: '任务',
    required: ['track'],
    optional: ['where', 'exit', 'signal', 'evidence', 'reason', 'prio'],
    sections: ['内容', '处理过程'],
    refFields: ['exit'],
    refSections: [],
    statusRules: {
      todo: { require: [], forbid: ['evidence', 'reason'] },
      doing: { require: [], forbid: ['evidence', 'reason'] },
      done: { require: ['evidence'], forbid: ['signal', 'reason'] },
      wont: { require: ['reason'], forbid: ['evidence'] },
    },
    closedStatus: 'done',
    requiresDepsDone: ['doing', 'done'],
    // 退场档（RG-025 R2）：大任务 done / wont ⇒ 小任务必须同处退场档——「父已完成、子还开着」
    // 是谎报；反过来（子 wont 父 doing）合法：分解允许中途砍掉某个子项。
    retiredStates: ['done', 'wont'],
  },
  spec: {
    label: '规范',
    required: ['scope'],
    optional: ['reason', 'supersedes', 'clears'],
    sections: ['规范', '判据'],
    refFields: [],
    refSections: ['判据'],
    statusRules: {
      draft: { require: [], forbid: ['reason'] },
      active: { require: [], forbid: ['reason'] },
      retired: { require: ['reason'], forbid: [] },
    },
    closedStatus: 'active',
    requiresDepsDone: ['active'],
    retiredStates: ['retired'],
  },
  adr: {
    label: '架构决策',
    required: ['adr', 'impl', 'domain'],
    optional: ['reason', 'supersedes'],
    sections: ['决策'],
    refFields: [],
    refSections: [],
    statusRules: {
      active: { require: [], forbid: ['reason'] },
      superseded: { require: ['reason'], forbid: [] },
      withdrawn: { require: ['reason'], forbid: [] },
    },
    closedStatus: 'active',
    // 与 spec / 文档类同构：`active` 断言「这条决策现行」，其 `deps` 前置就必须已闭环。
    // 依赖机制不按 kind 打折——RG-008 的门槛档位对全部 kind 同一套。
    requiresDepsDone: ['active'],
    retiredStates: ['superseded', 'withdrawn'],
  },
}

/**
 * 编号前缀表：**一个 kind 一个前缀**，全局闭集（RG-022 判形状的依据）。
 * 编号 = `<前缀>-<三位序号>[-名称]`——`id` 不含 `/`；含 `/` 的 `id` 是导航文档
 * （README / INDEX），等于仓根相对路径，不进 `deps`（RG-023）。
 */
export const ID_PREFIX = {
  goal: 'GO',
  stage: 'ST',
  plan: 'PL',
  task: 'T',
  adr: 'ADR',
  spec: 'SP',
  arch: 'AR',
  ref: 'RE',
  guide: 'GU',
  // `index` 是**跨 kind 的登记总账**。它需要自己的前缀，而不是硬塞进九个 kind 里的
  // 任何一个——给它 `PL-` 会在 plan 汇总页里出现一行「计划目录的索引页」，那是撒谎；
  // 而「不发编号」又会让指向它的地址只能手写（正是各 kind 汇总页改名要消灭的东西）。
  // `KIND` 这个前缀字面就说清了它横跨所有 kind，且不与任何内容条目相争。
  index: 'KIND',
}
/** 编号条目的 id 形状（名称段可选，允许中文 / 点 / 连字符）。 */
export const ENTRY_ID_RE = /^(GO|ST|PL|T|ADR|SP|AR|RE|GU|KIND)-[0-9]{3}[a-z]?(-[^\s/]+)?$/
/**
 * `adr` 的**域**闭集（归域规则见 [`docs/spec/SP-002.md`](../docs/spec/SP-002.md "SP-002")）。
 *
 * 域回答「这条决策**约束谁**」——它决定读者该去哪找、README 该往哪指。
 * 它是**条目上的一个字段**（不是目录、不是分册）：过滤 / 分组都是「按字段筛」，
 * 而字段是机读的、可判的。
 * 闭集由 RG-019 执行——新增域必须同批进这张表与 SP-001，不能随手写。
 */
export const ADR_DOMAIN = {
  平台基座: '插件架构 / 路由 / 注册 / 执行接口 / symbio_core 准入 / 身份 / 生命周期 / 对象创建',
  会话与执行: '编排 / 上下文 / 转写节点与实时面 / 输入 / 响应性',
  资源与地址: 'VdfsProvider / 读侧成本 / 变更信封 / 新建与导入 / 地址归属',
  模型与协议: '多协议适配 / TLS / 本地嵌入 / SSE 解析',
  智能体与记忆: 'agent 目录宿主 / 记忆的两个作用域',
}

/**
 * `adr` 的**落地进展**词表（与 `status` 正交的第二个字段）。
 *
 * `status` 回答「这条决策**还算不算数**」（规范合法性），`impl` 回答「**做到哪一步**」
 * （落地进展）——两者此前被压在同一列状态里（「现行（含未完成项）」「已接受 · 部分实施」），
 * 于是「一条现行决策只落地了一半」这种最常见的状态在机读面根本表达不出来。
 * 拆开之后：`status` 走 `KINDS.adr.statusRules` 的闭集，`impl` 走这张表（RG-017）。
 *
 * 词的语义**按序单调**：`planned` < `partial` < `done`——渲染按此序分组，
 * 断言「`done` ⇒ 必有落地证据」时也读这张表。
 */
export const ADR_IMPL = { planned: '未实施', partial: '部分实施', done: '已落地' }
/** `impl` 之外的终结词：决策被否决 / 从未实施便退出——不进单调序，单列。 */
export const ADR_IMPL_OTHER = { none: '未落地' }

/** SP-001 字段表必须**恰好**覆盖的键集（= 公共 + 各 kind 的并集）。 */
export const ALL_KEYS = [
  ...COMMON_REQUIRED,
  ...COMMON_OPTIONAL,
  ...new Set(
    Object.values(KINDS).flatMap((cfg) => [...cfg.required, ...cfg.optional]),
  ),
].sort()

// 编号必须能当标识：字母 / 数字 / 点开头，其后为字母数字 / . _ - / 与 Unicode 字母数字。
// 文档类的编号是**仓根相对路径**（如 `PL-001-核心架构`、`.github/workflows/README`），
// 所以放行 `/`、数字开头与隐藏目录前导点（`01-核心架构` / `T-0491-最小闭环` 这类「编号 + 名字」
// 天然合法）；条目类的历史批次号里有 `T-046` 这类圈号——编号是**身份**不是代号，
// 迁移登记时不得重排。
export const ID_RE = /^[A-Za-z0-9.][A-Za-z0-9._\/\-\p{L}\p{N}\p{M}]+$/u
/** 路径形编号的额外约束：不空段、不以 `/` 起止、不含 `..` 段（ID_RE 管字符集，这里管形状）。 */
export function validId(s) {
  return (
    typeof s === 'string' &&
    ID_RE.test(s) &&
    !s.startsWith('/') &&
    !s.endsWith('/') &&
    !s.includes('//') &&
    !s.split('/').includes('..')
  )
}
export const DATE_RE = /^\d{4}-\d{2}-\d{2}$/

/**
 * 登记范围的**扫描跳过**（目录）与**豁免**（单文件）。
 *
 * 判据是「仓内每一篇活跃 `.md` 都必须登记」（RG-020），所以下面这张表是它的**反面**：
 * 进表的要么不是阅读文档，要么已有别的 owner（职责表）。每项都写明理由——
 * **豁免不是方便门，是归属声明**，新增一项必须能回答「为什么它不受登记机制管」。
 */
export const SCAN_SKIP_DIRS = new Set([
  'node_modules',
  'target',
  '.git',
  'dist',
  'build',
  '.venv',
  // 工作区数据与对外模板：不是本仓的阅读文档（内容由会话 / 模板自己管）
  '.workbuddy-ai',
  // 会话工作区（记忆 / 项目数据）：与 `.workbuddy-ai` 同性质，`.gitignore` 的
  // `.workbuddy*/` 一并盖住；内容由会话自己管，不进本仓登记
  '.workbuddy',
  // 临时区（`.gitignore` 的 `tmp/`）：迁移备份 / 报告草稿的落点，不进登记
  'tmp',
  '.symbio',
  '.agents',
  'examples',
])
/** 目录前缀豁免（相对仓根，以 `/` 结尾）：归档是历史快照，现行登记不接管。 */
export const SCAN_SKIP_PREFIXES = ['docs/archive/']
/** 单文件豁免：路径 → 理由（与职责表 / plan/14「明确不接管」同口径）。 */
export const SCAN_SKIP_FILES = new Map([
  // ⚠️ `docs/ref/RE-006-当前事实表.md` 已不在此列：它搬进 `docs/ref/RE-006-当前事实表.md` 并**正式登记**。
  // 原先免登记的理由是「它是代码投影，不是条目」——但那条理由连累了它的**引用**：
  // 没有编号，别人只能手写 `../CURRENT.md#…`（地址机制搬不动），于是本仓为它专门
  // 加了一条判据盯着节次漂移。搬进来并给编号后，那些引用改走 `[[RE-006#…]]`，
  // 地址由 `gen-doc-refs` 重算——那条判据随之退休。
  ['docs/ref/GATE_CODES.md', '代码投影（facts/gate-codes 生成）'],
  ['docs/ref/PROMPT_SHAPE.md', '真实链路生成（e2e t46 生成）'],
  ['docs/ref/PROMPT_AB.md', '门外实验生成（e2e/ab 生成）'],
  ['AGENTS.md', '指令注入面（每会话直接进上下文，不是阅读文档）'],
])
/**
 * 各 kind 的汇总页文件名：`<前缀>-000-<X>汇总.md`。
 *
 * ## 为什么从 `INDEX.md` 改名
 *
 * 汇总页原先叫 `docs/<kind>/INDEX.md`，靠**固定名**被引用，于是每个指向它的地方都
 * 得手写 `./<kind>/INDEX.md` —— 一串不随目标位置变化的**手写地址**。而这些页面由
 * `isGeneratedIndex` 从 `loadEntries` 里整个排除，所以它们对 `gen-doc-refs` 与 RF-003
 * **完全隐形**：地址不会被重算、手写也不会被判红。与 RG-027 当初漏过的那个洞同类
 * （**生成投影对机制隐形**）。
 *
 * 改名 + 纳入登记（自己发 front-matter、自己有 id）之后，指向它们可以写编号引用，
 * 地址交给机制——与全仓其它文档同一条规矩，没有例外。
 *
 * 编号取 `000`：本仓各 kind 的编号从 `001` 起（`ENTRY_ID_RE` 允许三位零），
 * `000` 在该命名空间里**不与任何内容条目相争**，读起来也一眼知道它是「这一类从
 * 这里开始看」的入口页而不是内容。
 */
export function kindIndexFile(kind) {
  // `index` 的文件名不套 `<label>汇总` 那个模板：`总账汇总` 读着别扭（总账本身就是
  // 汇总，再加「汇总」），而文件名是**显示选择**，不是身份——身份是 id。
  if (kind === 'index') return `${ID_PREFIX[kind]}-000-登记总账.md`
  return `${ID_PREFIX[kind]}-000-${KINDS[kind].label}汇总.md`
}

/** 该 kind 汇总页的条目 id（= 文件名去 `.md`）。 */
export function kindIndexId(kind) {
  return kindIndexFile(kind).replace(/\.md$/, '')
}

/** 生成的汇总页（投影不是条目）：仓根总账 + 各 kind 的汇总页。 */
export function isGeneratedIndex(rel) {
  // 各 kind 汇总页已改名并**纳入登记**（自己发 front-matter、自己有 id），
  // 不再走「投影豁免」这条路——豁免正是让它们对机制隐形的那一步。
  for (const kind of Object.keys(KINDS)) {
    if (rel === `docs/${kind}/${kindIndexFile(kind)}`) return false
    if (kind === 'index' && rel === `docs/${kindIndexFile(kind)}`) return false
  }
  // 旧路径残留（各 kind 的 `INDEX.md` 与仓根 `KIND-000-登记总账.md`）：**必须判红**。
  // 它们是改名后没人删掉的过期投影——生成器不再写它们，也没有任何判据覆盖它们
  // （RG-020 豁免、D-001 只看活跃文档），于是会静默漂移、读者却点得到。
  if (rel === 'docs/KIND-000-登记总账.md') return false
  const m = rel.match(/^docs\/([^/]+)\/INDEX\.md$/)
  return false
}

/**
 * task 的排序提示词表。**prio 是提示（判断），deps 是义务（事实）**：
 * 只影响同一拓扑层内的排序（先拓扑、后 prio、再编号），不参与任何冲突裁决。
 * 缺席 = 最低档——不想定级不必硬填，判据不因此判红。
 */
export const PRIO = { now: 0, next: 1, later: 2 }

/** RG-010 的消息以常量形式住本文件（GC-003：声明的码必须在声明文件里出现）。 */
export const SCOPE_ERROR =
  'RG-010 登记范围读不出：仓内扫不到任何已登记条目——退化成「什么都没校验」是最坏的失败'

// ==================== front-matter 解析 ====================

function unquote(s) {
  if (s.length >= 2 && (s[0] === '"' || s[0] === "'") && s[s.length - 1] === s[0]) return s.slice(1, -1)
  return s
}

function parseScalar(raw) {
  const v = raw.trim()
  if (v === '[]') return []
  if (v.startsWith('[') && v.endsWith(']')) {
    const inner = v.slice(1, -1).trim()
    return inner === '' ? [] : inner.split(',').map((s) => unquote(s.trim()))
  }
  return unquote(v)
}

/**
 * 解析一个条目文件。
 *
 * @param text 文件全文
 * @param name 报错用的仓根相对路径
 * @returns {{meta: object|null, body: string, errors: string[]}}
 *          解析失败时 `meta` 为 null（调用方跳过语义校验，避免级联噪音）。
 */
export function parseEntryText(text, name) {
  const errors = []
  if (text.charCodeAt(0) === 0xfeff) text = text.slice(1)
  const lines = text.split(/\r?\n/)
  if (lines[0]?.trim() !== '---') {
    return { meta: null, body: '', errors: [`${name}：RG-001 首行必须是 \`---\`（front-matter 起始分隔符）`] }
  }
  let close = -1
  for (let i = 1; i < lines.length; i++) {
    if (lines[i].trim() === '---') {
      close = i
      break
    }
  }
  if (close < 0) {
    return { meta: null, body: '', errors: [`${name}：RG-001 front-matter 没有用 \`---\` 闭合`] }
  }
  const meta = {}
  let i = 1
  while (i < close) {
    const raw = lines[i]
    if (!raw.trim() || raw.trim().startsWith('#')) {
      i++
      continue
    }
    const m = raw.match(/^([A-Za-z][A-Za-z0-9_-]*):(.*)$/)
    if (!m) {
      errors.push(`${name}：RG-001 第 ${i + 1} 行不是 \`键: 值\` 形状：${raw.trim()}`)
      i++
      continue
    }
    const key = m[1]
    const rest = m[2]
    if (key in meta) {
      errors.push(`${name}：RG-001 字段 \`${key}\` 重复`)
      i++
      continue
    }
    if (rest.trim() === '') {
      // 块列表（`key:` 换行 + `  - item`）或空值
      const items = []
      let j = i + 1
      while (j < close && /^\s+-\s+\S/.test(lines[j])) {
        items.push(lines[j].replace(/^\s+-\s+/, '').trim())
        j++
      }
      if (items.length) {
        meta[key] = items
        i = j
      } else {
        meta[key] = null
        i++
      }
    } else {
      meta[key] = parseScalar(rest)
      i++
    }
  }
  return { meta, body: lines.slice(close + 1).join('\n'), errors }
}

// ==================== 装载 ====================

/** 正文首个 H1（跳过代码围栏）；不存在返回 null。RG-021 与迁移脚本共用同一口径。 */
export function firstH1(body) {
  let fence = null
  for (const line of body.split(/\r?\n/)) {
    const f = line.match(/^\s*(```+|~~~+)/)
    if (f) {
      if (fence === null) fence = f[1][0]
      else if (f[1][0] === fence) fence = null
      continue
    }
    if (fence !== null) continue
    const h = line.match(/^#\s+(.+?)\s*$/)
    if (h) return h[1]
  }
  return null
}

/**
 * 扫描**整个仓**（按 `SCAN_SKIP_*` 豁免）：每个活跃 `.md` 都必须登记——
 * 带合法 front-matter 且 `kind` 在 `KINDS` 闭集内。kind 只来自 front-matter，
 * **目录不参与判定**（文档在哪个目录下是组织问题，不是机制问题）。
 *
 * @param root 仓根
 * @returns {{entries: Array, errors: string[], notes: string[], missing: boolean}}
 *          `missing` = 扫不到任何 md（范围读不出，RG-010 走 exit 2）
 */
export function loadEntries(root) {
  const errors = []
  const notes = []
  const entries = []
  let scanned = 0
  const walk = (dir) => {
    let list
    try {
      list = fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))
    } catch {
      return
    }
    for (const ent of list) {
      const abs = path.join(dir, ent.name)
      const rel = path.relative(root, abs).split(path.sep).join('/')
      if (ent.isDirectory()) {
        if (SCAN_SKIP_DIRS.has(ent.name)) continue
        if (SCAN_SKIP_PREFIXES.some((p) => rel + '/' === p)) continue
        walk(abs)
        continue
      }
      if (!ent.name.endsWith('.md')) continue
      if (SCAN_SKIP_FILES.has(rel) || isGeneratedIndex(rel)) {
        // 免**登记**（kind / id / title / status 那套）≠ 免**跨文档事实**。
        // `lists` 声明是全局的：owner 很可能是一份**生成物**（CURRENT.md 的三张表就是），
        // 它若对分工重叠判据隐形，那「两个 owner」永远只看得见一个——RG-027 直接失效。
        // （反向验证当场抓到这一条：给 RE-003 注入 `routes=owner` 时判据**漏过**，
        //  因为 CURRENT.md 那份 owner 声明根本没进 registry 的视野。）
        const text = fs.readFileSync(abs, 'utf8')
        const { meta, body, errors: perr } = parseEntryText(text, rel)
        if (meta && !isEmpty(meta.lists)) {
          entries.push({ file: abs, rel, kind: meta.kind ?? null, name: ent.name, meta, body, errors: perr, projection: true })
        }
        continue
      }
      scanned += 1
      const text = fs.readFileSync(abs, 'utf8')
      const first = text.replace(/^\uFEFF/, '').split(/\r?\n/, 1)[0]?.trim()
      if (first !== '---') {
        errors.push(
          `${rel}：RG-020 未登记文档——活跃 \`.md\` 必须自带 front-matter 声明 kind（豁免名单住 registry-lib 的 SCAN_SKIP_*）`,
        )
        continue
      }
      const { meta, body, errors: perr } = parseEntryText(text, rel)
      if (!meta) {
        entries.push({ file: abs, rel, kind: null, name: ent.name, meta: null, body: '', errors: perr })
        continue
      }
      if (isEmpty(meta.kind)) {
        errors.push(`${rel}：RG-020 front-matter 缺 \`kind\`——文档未登记到任何 kind`)
        continue
      }
      if (!(meta.kind in KINDS)) {
        errors.push(
          `RG-013 ${rel}：kind=${JSON.stringify(meta.kind)} 不在 KINDS 闭集（${Object.keys(KINDS).join(' / ')}）` +
            '——命名空间必须闭合，先在 registry-lib 的 KINDS 里声明',
        )
        continue
      }
      entries.push({ file: abs, rel, kind: meta.kind, name: ent.name, meta, body, errors: perr })
    }
  }
  walk(root)

  for (const [kind, cfg] of Object.entries(KINDS)) {
    if (!entries.some((e) => e.kind === kind)) notes.push(`kind ${kind}（${cfg.label}）：暂无条目`)
  }
  // 投影条目（免登记的生成物）只参与**清单归属**那一族判据（RG-026…RG-029）——
  // 它们没有 kind，不进状态机 / 依赖 / 分解 / 字段完整性任何一条。分开是因为
  // 「免登记」的理由是「它不是条目」，而清单归属问的是「这份清单有没有主」，
  // 后者与它是不是条目无关。
  return {
    entries: entries.filter((e) => !e.projection),
    projections: entries.filter((e) => e.projection),
    errors,
    notes,
    missing: scanned === 0,
  }
}

// ==================== 校验 ====================

export function isEmpty(v) {
  return v == null || (typeof v === 'string' && !v.trim()) || (Array.isArray(v) && v.length === 0)
}

/** 某 kind 的「闭环 / 成立」状态词（取代链与 deps 都用它判「还算不算数」）。 */
export function cfgClosedStatus(kind) {
  return KINDS[kind]?.closedStatus
}

/** 取 `## <name>` 小节正文（到下一个二级标题为止）；不存在返回 ''。 */
export function sectionOf(body, name) {
  const m = body.match(new RegExp(`^##\\s+${name}\\s*$`, 'm'))
  if (!m) return ''
  const after = body.slice(m.index + m[0].length)
  const next = after.search(/^##\s/m)
  return next < 0 ? after : after.slice(0, next)
}

// doc-link-allow D-010: 下一行是这条判据的形态示例（x / y / z 是占位名），不是入链
/** 正文 / 字段里形如 `docs/x.md`、`scripts/y.mjs`、`e2e/z.mjs` 的反引号指认。 */
export function refsIn(text) {
  return [...text.matchAll(/`((?:docs|scripts|e2e)\/[^`\s]+\.(?:mjs|md|rs))`/g)].map((m) => m[1])
}

function validDate(s) {
  if (!DATE_RE.test(s)) return false
  const d = new Date(`${s}T00:00:00Z`)
  return !Number.isNaN(d.getTime()) && d.toISOString().slice(0, 10) === s
}

/**
 * 同层比较器：先 `prio`（now < next < later < 缺席），再编号字典序。
 * 拓扑约束永远优先于它——`deps` 是义务（事实），`prio` 是提示（判断）。
 */
function cmpPrio(byIdEntry) {
  const rank = (id) => {
    const p = byIdEntry.get(id)?.meta?.prio
    return typeof p === 'string' && p in PRIO ? PRIO[p] : 9
  }
  return (a, b) => rank(a) - rank(b) || (a < b ? -1 : a > b ? 1 : 0)
}

/** 在已有依赖图上找一条环（确定性：按 id 字典序起步，deps 排序）。 */function findCycle(ids, depsOf) {
  const state = new Map()
  const stack = []
  let cycle = null
  const visit = (id) => {
    if (cycle) return
    const s = state.get(id) ?? 0
    if (s === 2) return
    if (s === 1) {
      cycle = [...stack.slice(stack.indexOf(id)), id]
      return
    }
    state.set(id, 1)
    stack.push(id)
    for (const d of depsOf(id)) visit(d)
    stack.pop()
    state.set(id, 2)
  }
  for (const id of [...ids].sort()) visit(id)
  return cycle
}

/**
 * 全量校验。返回**纯文本**一行一条的违规（颜色由 CLI 上），
 * 空数组 = 零违规。`root` 用于 RG-012 的存在性判定。
 */
export function auditEntries(entries, root, projections = []) {
  const out = []
  const byId = new Map()

  for (const e of entries) {
    out.push(...e.errors)
    if (!e.meta) continue
    const cfg = KINDS[e.kind]
    const meta = e.meta
    const allowed = new Set([...COMMON_REQUIRED, ...COMMON_OPTIONAL, ...cfg.required, ...cfg.optional])

    // RG-001 未知字段（按 kind 判，错域字段被指名）
    for (const k of Object.keys(meta)) {
      if (!allowed.has(k)) out.push(`${e.rel}：RG-001 未知字段 \`${k}\`（${e.kind} 允许：${[...allowed].sort().join('、')}）`)
    }
    // RG-002 必填非空（kind 即装载时的目录判据已取消：kind 只来自 front-matter，目录不参与判定）
    // `projection: true` 豁免**kind 专属**必填字段（`track` / `scope` / `impl` …）：
    // 那些字段描述的是「这份内容处在什么位置」，而投影页没有内容。
    // **公共**五行一条不豁免——那是「它是一份文档」的定义。
    const isProj = meta.projection === true || meta.projection === 'true'
    for (const k of [...COMMON_REQUIRED, ...(isProj ? [] : cfg.required)]) {
      if (isEmpty(meta[k])) out.push(`${e.rel}：RG-002 必填字段 \`${k}\` 缺失或为空`)
    }
    // RG-003 编号形状 / 编号 ↔ 文件对账 / 全 registry 唯一
    // 按 id 形态自判（不按 kind——同一 kind 下既有编号条目也有导航 README）：
    // id = 仓根相对路径 ⇒ 导航文档（仓根文档的路径即 basename，如 `README`）；
    // 否则 id 必须是编号条目（不含 `/`，文件名等于编号）。
    if (!isEmpty(meta.id)) {
      if (typeof meta.id !== 'string' || !validId(meta.id)) {
        out.push(
          `${e.rel}：RG-003 id 不合形状（字母 / 数字 / 点开头，其后为字母数字 / . _ - / 与 Unicode 字母数字）：${JSON.stringify(meta.id)}`,
        )
      } else if (meta.id === e.rel.replace(/\.md$/, '')) {
        // 导航文档：id 即仓根相对路径——README / INDEX 这类通用名没有编号段，
        // 不构成命名空间，禁止进 deps（RG-023）
      } else if (meta.id.includes('/')) {
        out.push(
          `${e.rel}：RG-003 导航文档 id 必须等于仓根相对路径（去 \`.md\`）（应为 \`${e.rel.replace(/\.md$/, '')}\`）——含 \`/\` 的 id 是路径不是编号`,
        )
      } else if (!ENTRY_ID_RE.test(meta.id)) {
        out.push(
          `${e.rel}：RG-022 编号条目 id 必须匹配 <前缀>-<三位序号>[-名称]（前缀闭集 ${Object.values(ID_PREFIX).join(' / ')}）：${JSON.stringify(meta.id)}`,
        )
      } else if (!meta.id.startsWith(`${ID_PREFIX[e.kind]}-`)) {
        out.push(
          `${e.rel}：RG-022 ${e.kind} 的编号前缀必须是 \`${ID_PREFIX[e.kind]}-\`（读到 ${JSON.stringify(meta.id)}）——前缀与 kind 对账，编号空间才可盲信`,
        )
      } else if (e.name !== `${meta.id}.md`) {
        out.push(`${e.rel}：RG-003 文件名必须等于编号（应为 \`${meta.id}.md\`）——文件名即编号的对账`)
      }
      if (byId.has(meta.id)) {
        out.push(`${e.rel}：RG-003 编号 \`${meta.id}\` 重复（已登记于 ${byId.get(meta.id).rel}）`)
      } else {
        byId.set(meta.id, e)
      }
    }
    // `projection: true` = 这一页**有身份、没内容**（内容由渲染器决定）。它豁免
    // **内容面**判据，逐条列明而不是笼统放行：kind 专属必填字段（RG-002，如 task 的
    // `track`）、状态词（RG-004）、状态×字段矩阵（RG-007）、必备小节（RG-009）、
    // 取代链 / 清除指认（RG-015/016）。
    // **身份面**一条不豁免：front-matter 可解析、编号形状、id ↔ 文件名、created、
    // title == H1、清单归属——那几条判的正是「它是不是一份可被引用的文档」。
    const proj = meta.projection === true || meta.projection === 'true'
    // RG-004 状态词
    if (!proj && !isEmpty(meta.status) && !(meta.status in cfg.statusRules)) {
      out.push(`${e.rel}：RG-004 status=${JSON.stringify(meta.status)} 不在 ${e.kind} 的状态词（${Object.keys(cfg.statusRules).join(' / ')}）`)
    }
    // RG-005 日期
    if (!isEmpty(meta.created) && (typeof meta.created !== 'string' || !validDate(meta.created))) {
      out.push(`${e.rel}：RG-005 created 不是有效日期（YYYY-MM-DD）：${JSON.stringify(meta.created)}`)
    }
    // RG-014 排序提示词表（prio 是提示不是义务：义务在 deps，词表封闭只是为了让排序确定）
    if (!isEmpty(meta.prio) && !(typeof meta.prio === 'string' && meta.prio in PRIO)) {
      out.push(`${e.rel}：RG-014 prio 不在词表（${Object.keys(PRIO).join(' / ')}）：${JSON.stringify(meta.prio)}`)
    }
    // RG-017 adr 的落地进展词表（与 status 正交：status 管「算不算数」，impl 管「做到哪一步」）
    if (e.kind === 'adr' && !isEmpty(meta.impl)) {
      const implWords = { ...ADR_IMPL, ...ADR_IMPL_OTHER }
      if (!(typeof meta.impl === 'string' && meta.impl in implWords)) {
        out.push(
          `${e.rel}：RG-017 impl 不在词表（${Object.keys(implWords).join(' / ')}）：${JSON.stringify(meta.impl)}` +
            '——status 管规范合法性、impl 管落地进展，两者不得混成同一字段',
        )
      }
    }
    // RG-019 adr 的域闭集（原来住 DECISIONS.md §0.1 的分册表；合流后按字段过滤）
    if (e.kind === 'adr' && !isEmpty(meta.domain)) {
      if (!(typeof meta.domain === 'string' && meta.domain in ADR_DOMAIN)) {
        out.push(
          `${e.rel}：RG-019 domain 不在闭集（${Object.keys(ADR_DOMAIN).join(' / ')}）：${JSON.stringify(meta.domain)}` +
            '——域是「这条决策约束谁」，新增域须同批进 ADR_DOMAIN 与 SP-001',
        )
      }
    }
    // RG-006 deps 形状 / 自指 / 重复（存在性与成环在跨文件段判）
    if ('deps' in meta && !isEmpty(meta.deps) && !Array.isArray(meta.deps)) {
      out.push(`${e.rel}：RG-006 deps 必须是 \`[]\` / \`[A, B]\` / 块列表，读到：${JSON.stringify(meta.deps)}`)
    }
    if (Array.isArray(meta.deps)) {
      const seen = new Set()
      for (const d of meta.deps) {
        if (d === meta.id) out.push(`${e.rel}：RG-006 deps 含自身（自己不能是自己的前置）`)
        if (seen.has(d)) out.push(`${e.rel}：RG-006 deps 重复登记 \`${d}\``)
        // RG-023：deps 是依赖图的边，只收编号条目——导航文档（id 含 `/`，README / INDEX
        // 这类通用名）没有编号段、不构成命名空间，不许进图；要指它用 `[[...]]` 引用。
        if (typeof d === 'string' && d.includes('/')) {
          out.push(
            `${e.rel}：RG-023 deps 目标必须是编号条目（id 不得含 \`/\`），读到 \`${d}\`——导航文档用 \`[[${d}]]\` 引用，不进依赖图`,
          )
        }
        seen.add(d)
      }
    }
    // RG-024 parent 形状（单文件段）：单值标量——一个子条目只有一个父亲（树不是 DAG），
    // 「共同归属」没有语义（那正是 deps 的场景）。存在性 / 同 kind / 成环在跨文件段判。
    if ('parent' in meta && !isEmpty(meta.parent)) {
      const p = meta.parent
      if (typeof p !== 'string' || !validId(p)) {
        out.push(
          `${e.rel}：RG-024 parent 必须是编号条目 id（字母 / 数字 / 点开头，其后为字母数字 / . _ - 与 Unicode 字母数字），读到：${JSON.stringify(p)}`,
        )
      } else if (p.includes('/')) {
        out.push(
          `${e.rel}：RG-024 parent 不得含 \`/\`（读到 \`${p}\`）——parent 是编号（身份）不是路径（位置）；跨位置或跨 kind 的关联走 deps / \`[[引用]]\``,
        )
      } else if (p === meta.id) {
        out.push(`${e.rel}：RG-024 parent 含自身（自己不能分解出自己——分解环没有父）`)
      }
    }
    // RG-007 状态 × 字段矩阵
    const rule = typeof meta.status === 'string' ? cfg.statusRules[meta.status] : null
    if (rule && !proj) {
      for (const k of rule.require) {
        if (isEmpty(meta[k])) out.push(`${e.rel}：RG-007 status=${meta.status} 必须填 \`${k}\``)
      }
      for (const k of rule.forbid) {
        if (!isEmpty(meta[k])) out.push(`${e.rel}：RG-007 status=${meta.status} 不得带 \`${k}\`（已填非空值）`)
      }
    }
    // RG-009 正文小节（文档类的 sections 为空：正文结构由文档自己定）
    // 投影页豁免：它的正文由渲染器决定，`## 内容` / `## 处理过程` 那套是人写的结构。
    if (!proj) {
      for (const sec of cfg.sections) {
        if (!new RegExp(`^##\\s+${sec}\\s*$`, 'm').test(e.body)) {
          out.push(`${e.rel}：RG-009 正文缺 \`## ${sec}\` 小节`)
        }
      }
    }
    // RG-021：文档类的 title 必须逐字等于正文首个 H1（title 的唯一来源是 H1，两面对账）
    if (cfg.titleEqualsH1 && !isEmpty(meta.title)) {
      const h1 = firstH1(e.body)
      if (h1 == null) {
        out.push(`${e.rel}：RG-021 文档类缺正文首个 \`# 标题\`（title 的唯一来源读不出）`)
      } else if (h1 !== meta.title) {
        out.push(
          `${e.rel}：RG-021 title=${JSON.stringify(meta.title)} 与正文首个 H1 ${JSON.stringify(h1)} 不一致——title 逐字等于 H1`,
        )
      }
    }
    // RG-012 判据指认的文件必须存在（落点 `where` 不判——落点可以尚不存在）
    for (const f of cfg.refFields) {
      if (typeof meta[f] !== 'string') continue
      for (const ref of refsIn(meta[f])) {
        if (!fs.existsSync(path.join(root, ref))) {
          out.push(`${e.rel}：RG-012 \`${f}\` 指认的 \`${ref}\` 不存在（出口判据不得依赖尚不存在的东西）`)
        }
      }
    }
    for (const sec of cfg.refSections) {
      for (const ref of refsIn(sectionOf(e.body, sec))) {
        if (!fs.existsSync(path.join(root, ref))) {
          out.push(`${e.rel}：RG-012 判据小节指认的 \`${ref}\` 不存在`)
        }
      }
    }
  }

  // ---- 跨条目：依赖存在性（RG-006）、依赖状态（RG-008）、成环（RG-006） ----
  for (const e of entries) {
    if (!e.meta || !Array.isArray(e.meta.deps)) continue
    const cfg = KINDS[e.kind]
    for (const d of e.meta.deps) {
      const dep = byId.get(d)
      if (!dep) {
        out.push(`${e.rel}：RG-006 deps 指到不存在的编号 \`${d}\``)
        continue
      }
      if (cfg.requiresDepsDone.includes(e.meta.status) && dep.meta) {
        const depClosed = KINDS[dep.kind].closedStatus === dep.meta.status
        if (!depClosed) {
          out.push(
            `${e.rel}：RG-008 status=${e.meta.status} 要求前置 \`${d}\` 已闭环，而它是 ${dep.meta.status}` +
              `（${KINDS[dep.kind].label}的闭环状态是 ${KINDS[dep.kind].closedStatus}）`,
          )
        }
      }
    }
  }
  {
    const ids = [...byId.keys()]
    const depsOf = (id) => (Array.isArray(byId.get(id).meta.deps) ? byId.get(id).meta.deps.filter((d) => byId.has(d)) : [])
    const cycle = findCycle(ids, depsOf)
    if (cycle) out.push(`RG-006 依赖成环：${cycle.join(' -> ')}（环上的编号永远等不到前置闭环）`)
  }

  /** `lists` 声明收集处（RG-026/027/029 读它）：{rel, id, role, kind} */
  const listClaims = []
  // 清单归属这一族同时看**条目**与**投影**（免登记的生成物）。owner 往往就是投影
  // （CURRENT.md 的三张表由 facts/current 生成），它若隐形，RG-027 的「两个 owner」
  // 永远只看得见一个——反向验证当场抓到这条漏过。
  const listScope = [...entries, ...projections]

  // ---- RG-024：parent 的存在性 / 同 kind / 不成环（分解树的边） ----
  // 分解是**同类事物的大小之分**：总目标→子目标、大任务→小任务、产品功能→子功能、
  // 大阶段→子阶段——所以 parent 只许同 kind。跨 kind 的关联（任务服务于某阶段）走 deps
  // （前置）或正文引用（导航），不塞进 parent：一个字段一种语义，塞两种就两种都读不出。
  {
    const parentOf = new Map()
    /** `lists` 声明收集处（RG-026/027/029 读它）：{rel, id, role, kind} */
    const listClaims = []
    for (const e of entries) {
      if (!e.meta || isEmpty(e.meta.parent) || typeof e.meta.parent !== 'string') continue
      if (e.meta.parent === e.meta.id) continue // 自指已在单文件段判（不重复进环检测）
      if (e.meta.parent.includes('/')) continue // 形状错已在单文件段判（一个违规只红一条）
      const parent = byId.get(e.meta.parent)
      if (!parent) {
        out.push(`${e.rel}：RG-024 parent 指到不存在的编号 \`${e.meta.parent}\``)
        continue
      }
      if (parent.kind !== e.kind) {
        out.push(
          `${e.rel}：RG-024 parent \`${e.meta.parent}\` 是${KINDS[parent.kind].label}（${parent.kind}），` +
            `而本篇是${KINDS[e.kind].label}（${e.kind}）——分解只发生在同 kind 之间；` +
            '跨 kind 的关联用 deps（前置）或正文引用（导航）',
        )
        continue
      }
      parentOf.set(e.meta.id, e.meta.parent)
    }
    const cycle = findCycle([...parentOf.keys()], (id) => (parentOf.has(id) ? [parentOf.get(id)] : []))
    if (cycle) out.push(`RG-024 parent 成环：${cycle.join(' -> ')}（分解环上的条目永远说不清谁是父）`)
  }

  // ---- RG-026 / RG-027 / RG-029：清单归属（一份清单只有一个 owner） ----
  //
  // **为什么要有这个字段**：`docs/README.md` 的「一个事实只有一个 owner」此前是一张
  // **没人执行的承诺**——它写着「不手抄，需要就引用」，而 D-001…D-010 查链、查措辞、
  // 查形态，没有一条查「同一份清单被两处各自维护」。于是 CURRENT.md 内部并存过两张
  // 工具表（行集交集 25/25），RE-003 与 CURRENT §2.3 各持一份手写/生成的路由地址，
  // 两处**保证等级还不一样**（一处「必然与代码一致」、一处「人工维护，唯一会漂移」）。
  //
  // `lists` 把那份承诺变成**可机读的声明**：每篇文档在 front-matter 里写它**包含哪些
  // 清单**、每份是不是 `owner`（我维护它）。不写 = 这篇不含任何清单，单内容文档——
  // 不需要为「没有」额外声明。
  //
  // **role 词表只有 owner**（`view` 已取消，理由见 LIST_ROLES 的注释）：这套机制的
  // 初衷是消灭重复，而 view 用「A 维护、B 投影重渲」把重复换个形态留下来——生成的
  // 重复也是重复，读者面前的事实仍是两份。想读另一份清单就指路（`[[编号]]`），不重渲。
  //
  // 判据分**两层**，各自在最小作用域内解决——这是本设计的全部要点：
  //
  //   | 层 | 判什么 | 要读多少 | 坏了归谁 |
  //   |---|---|---|---|
  //   | 单文档 | 我声明的清单，正文里有没有对应的东西 | **这一篇** | 这一篇自己 |
  //   | 跨文档 | 两篇是否都声称拥有同一份清单 | **每篇 front-matter**（不碰正文表） | 两篇一起谈 |
  //
  // 跨文档层**只比声明**，所以回答「有没有重复」不需要扫描任何正文——那正是它便宜的原因。
  // 我最初写成「扫全仓每张表、猜它像哪份清单」，那是**错的形状**：成本随文档数×表数增长，
  // 且判据是**形状猜测**（插件目录名长得像工具短名、`vdfs/read` 长得像路由，当场三处误报）。
  //
  // 三条判据各管一个方向，缺一不可：
  //   RG-026 形状：`id=role` 行内列表，role ∈ {owner}；
  //   RG-027 **分工重叠**：同一个 id 出现两个 owner（或零个）——这就是「重复」，判红；
  //   RG-029 声明两面对账：正文里每处 `<!-- list:<id> owner -->` 标记都要与 front-matter
  //           一一对应（多一处少一处都红）——**声明不许撒谎**。
  //
  // （RG-028 原判「view 必须链到 owner」，随 view 角色取消而退役：词表里没有 view，
  //   声明即被 RG-026 挡下，那条检查已无输入。想复活 view 就连它一起复活。）
  //
  // RG-029 挡的是「有人抄了一张表却没写 `lists`」那个洞：单靠 front-matter，两层都看不见。
  // 标记写在**清单旁边**而不只写在 front-matter，于是**只读一篇文档**就能回答
  // 「它声明了什么、正文里对应的东西在不在」——两面对不上是这篇自己的问题。
  for (const e of listScope) {
    if (!e.meta) continue
    const raw = e.meta.lists
    if (raw === undefined || raw === null) continue
    if (!Array.isArray(raw)) {
      out.push(`${e.rel}：RG-026 \`lists\` 必须是行内列表 \`[id=role, …]\`，实际读到 ${typeof raw}——值形状见 SP-001`)
      continue
    }
    for (const item of raw) {
      if (typeof item !== 'string' || !item.includes('=')) {
        out.push(`${e.rel}：RG-026 \`lists\` 的每一项必须形如 \`id=role\`，实际读到 \`${item}\``)
        continue
      }
      const [id, role] = item.split('=').map((s) => s.trim())
      if (!id) out.push(`${e.rel}：RG-026 \`lists\` 有一项的 id 为空：\`${item}\``)
      if (!LIST_ROLES.includes(role)) {
        out.push(
          `${e.rel}：RG-026 \`lists\` 的 role 必须是 \`owner\`，实际读到 \`${role}\`——` +
            'owner = 我维护它；词表只有这一个角色（`view` 已取消：投影重渲是把重复换个形态留下，' +
            '见 LIST_ROLES 注释；想读另一份清单就指路，不重渲）',
        )
      }
      if (id) listClaims.push({ rel: e.rel, id, role, kind: e.kind })
    }
  }
  // RG-027：同一 id 恰好一个 owner（零个 owner = 无主清单，同样是分工缺失）
  const owners = new Map()
  for (const c of listClaims.filter((c) => c.role === 'owner')) {
    if (!owners.has(c.id)) owners.set(c.id, [])
    owners.get(c.id).push(c.rel)
  }
  for (const [id, rels] of owners) {
    if (rels.length > 1) {
      out.push(
        `RG-027 清单 \`${id}\` 有 ${rels.length} 个 owner：${rels.join('、')}——` +
          '**分工重叠**：同一份清单两处各自维护，改一次要动两处且必然漂移。' +
          '处置：留一个 owner，另一处去掉 `lists` 声明、改为在正文指路它；若两份内容本就不一样，' +
          '那它们其实是两份不同的清单，各起各的 id',
      )
    }
  }
  // RG-028 已退役：view 角色取消后无输入（见上方注释块）。

  // ---- RG-029：声明两面对账（front-matter ↔ 正文标记） ----
  // **它挡的洞**：front-matter 是**自报**的。没有人写 `lists`，这一篇就当自己不含清单，
  // 于是 RG-027 的分工重叠判据看不见「这里有一份副本」——声明层唯一的前提是**声明完整**，
  // 而完整性得有个执行方。
  //
  // 执行方就是这条：**正文里每处清单旁边必须有 `<!-- list:<id> <role> -->` 标记**，
  // 且与 front-matter 声明**一一对应**（多一处 = 声明漏了；少一处 = 声明了却没落实）。
  // 于是「只读一篇文档」就能回答「它声明了什么、正文里对应的东西在不在」，
  // 两面对不上是**这篇自己的**问题，不必牵出任何别的文档。
  const LIST_MARK = /<!--\s*list:\s*([a-z0-9][a-z0-9-]*)\s+(owner)\s*-->/g
  for (const e of listScope) {
    if (!e.body) continue
    const marks = [...e.body.matchAll(LIST_MARK)].map((m) => ({ id: m[1], role: m[2] }))
    const declared = listClaims.filter((c) => c.rel === e.rel)
    if (marks.length === 0 && declared.length === 0) continue // 单内容文档，两边都空即一致
    for (const d of declared) {
      if (!marks.some((m) => m.id === d.id && m.role === d.role)) {
        out.push(
          `${e.rel}：RG-029 front-matter 声明了清单 \`${d.id}=${d.role}\`，正文里却没有对应的 ` +
            `\`<!-- list:${d.id} ${d.role} -->\` 标记——清单必须**在它旁边**标出自己，` +
            '否则「声明了却没落实」与「落实了却没声明」都判不出来',
        )
      }
    }
    for (const m of marks) {
      if (!declared.some((d) => d.id === m.id && d.role === m.role)) {
        out.push(
          `${e.rel}：RG-029 正文里有 \`<!-- list:${m.id} ${m.role} -->\` 标记，front-matter 却没声明它——` +
            '一份清单不登记归属，就成了没有 owner 的副本',
        )
      }
    }
  }

  // ---- RG-025：父子状态一致性（分解树上的档位单调——状态只许说真话） ----
  // 两条规则，对全部 kind 同一套（档位表按 kind 取，与 RG-008「依赖机制不按 kind 打折」同风）：
  //   R1 子生效 ⇒ 父生效：子条目进入生效档 ⇒ 父不得停在未生效档
  //     （「三个小任务已经 doing，大任务还写着 todo」是谎报——父确实已开始推进）；
  //   R2 父退场 ⇒ 子退场：父条目进入退场档 ⇒ 不得有子条目停在生效档
  //     （「大任务已经 done / wont，还有小任务 doing」同样是谎报）。
  // 中间组合合法：父 doing 子 todo（分解未动）、子 wont 父 doing（中途砍子项）——
  // 分解允许部分未动，不允许档位倒挂。draft / 草案引草案的同款宽容这里也有：
  // 草案子条目不触发 R1（未生效档本来就可以引未生效档）。
  for (const e of entries) {
    if (!e.meta || isEmpty(e.meta.parent) || typeof e.meta.parent !== 'string') continue
    if (e.meta.parent === e.meta.id) continue
    if (e.meta.parent.includes('/')) continue // 形状错已在单文件段判（一个违规只红一条）
    const parent = byId.get(e.meta.parent)
    if (!parent || !parent.meta || parent.kind !== e.kind) continue // 存在性 / 同 kind 归 RG-024
    const cfg = KINDS[e.kind]
    const childStatus = e.meta.status
    const parentStatus = parent.meta.status
    if (cfg.requiresDepsDone.includes(childStatus) && !cfg.requiresDepsDone.includes(parentStatus)) {
      out.push(
        `${e.rel}：RG-025 子条目 status=${childStatus}（生效档）要求父条目 \`${e.meta.parent}\` 同在生效档` +
          `（${cfg.requiresDepsDone.join(' / ')}），而它是 ${parentStatus}——部分子项已推进、父整体却停在未生效档，是状态谎报`,
      )
    }
    if (cfg.retiredStates.includes(parentStatus) && !cfg.retiredStates.includes(childStatus)) {
      out.push(
        `${e.rel}：RG-025 父条目 \`${e.meta.parent}\` status=${parentStatus}（退场档）要求子条目同处退场档` +
          `（${cfg.retiredStates.join(' / ')}），而它是 ${childStatus}——父整体已退场、子项却停在生效档，是状态谎报`,
      )
    }
  }

  // ---- RG-015：spec 的 supersedes（显式取代链——冲突靠数据消解，不靠优先级裁决） ----
  // 取代关系只在规范之间：`spec` 取代 `spec`、`adr` 取代 `adr`（决策也有被推翻的形态）。
  // 跨 kind 取代（spec 取代 adr）没有语义——规范是「必须成立的条文」，决策是「为什么这样设计」。
  for (const e of entries) {
    if (!e.meta || !['spec', 'adr'].includes(e.kind) || !('supersedes' in e.meta)) continue
    const v = e.meta.supersedes
    if (isEmpty(v)) continue
    if (!Array.isArray(v)) {
      out.push(`${e.rel}：RG-015 supersedes 必须是 \`[编号]\` 列表，读到：${JSON.stringify(v)}`)
      continue
    }
    const seen = new Set()
    for (const t of v) {
      if (t === e.meta.id) {
        out.push(`${e.rel}：RG-015 supersedes 含自身（自己不取代自己）`)
        continue
      }
      if (seen.has(t)) out.push(`${e.rel}：RG-015 supersedes 重复指认 \`${t}\``)
      seen.add(t)
      const target = byId.get(t)
      if (!target) {
        out.push(`${e.rel}：RG-015 supersedes 指到不存在的编号 \`${t}\``)
        continue
      }
      if (target.kind !== e.kind) {
        out.push(`${e.rel}：RG-015 supersedes 指向 ${target.kind}（\`${t}\`）——取代关系只在同 kind 之间（spec↔spec / adr↔adr）`)
        continue
      }
      if (e.meta.status === cfgClosedStatus(e.kind) && target.meta && target.meta.status === cfgClosedStatus(target.kind)) {
        out.push(
          `RG-015 status=${e.meta.status} 的${KINDS[e.kind].label}不得取代另一条 ${target.meta.status} ` +
            `${KINDS[target.kind].label}（\`${t}\`）——两条同框即矛盾未消解，先把旧条改状态`,
        )
      }
    }
  }
  {
    const supIds = [...byId.values()]
      .filter((x) => x.kind === 'spec' || x.kind === 'adr')
      .map((x) => x.meta.id)
    const supOf = (id) => {
      const m = byId.get(id).meta
      return Array.isArray(m.supersedes)
        ? m.supersedes.filter((t) => t !== id && byId.has(t) && byId.get(t).kind === byId.get(id).kind)
        : []
    }
    const cycle = findCycle(supIds, supOf)
    if (cycle) out.push(`RG-015 supersedes 成环：${cycle.join(' -> ')}（取代链必须有尽头）`)
  }

  // ---- RG-016：spec 的 clears（豁免的清除任务指认） ----
  // clears 是**指认**不是前置（与 deps 的语义分界）：豁免可以合法地挂在**开放**的
  // 清除任务上——那正是「现在不修 + 倒计时」的语义，所以这里**不**做 RG-008 式的闭环要求。
  for (const e of entries) {
    if (!e.meta || e.kind !== 'spec' || !('clears' in e.meta)) continue
    const v = e.meta.clears
    if (isEmpty(v)) continue
    if (!Array.isArray(v)) {
      out.push(`${e.rel}：RG-016 clears 必须是 \`[编号]\` 列表，读到：${JSON.stringify(v)}`)
      continue
    }
    const seen = new Set()
    for (const t of v) {
      if (seen.has(t)) out.push(`${e.rel}：RG-016 clears 重复指认 \`${t}\``)
      seen.add(t)
      const target = byId.get(t)
      if (!target) {
        out.push(`${e.rel}：RG-016 clears 指到不存在的编号 \`${t}\`（清除任务必须先登记）`)
        continue
      }
      if (target.kind !== 'task') {
        out.push(`${e.rel}：RG-016 clears 指向 ${target.kind}（\`${t}\`）——清除任务是任务，不是规范`)
      }
    }
  }

  // ---- RG-011：SP-001 的字段表必须恰好等于代码侧 schema ----
  const sp = entries.find((e) => e.kind === 'spec' && e.meta && e.meta.id === 'SP-001')
  if (!sp) {
    out.push('RG-011 docs/spec/SP-001.md 不存在——schema 的文档侧真相没有登记')
  } else {
    const m = sp.body.match(/^##\s+字段\s*$/m)
    if (!m) {
      out.push('RG-011 docs/spec/SP-001.md 缺 \`## 字段\` 小节——字段表读不出')
    } else {
      const section = sectionOf(sp.body, '字段')
      const keys = [...section.matchAll(/^\|\s*`([A-Za-z][A-Za-z0-9_-]*)`\s*\|/gm)].map((x) => x[1])
      if (keys.length === 0) {
        out.push('RG-011 docs/spec/SP-001.md 字段表读不出（\`## 字段\` 小节里没有 \`| \`键\` |\` 形状的行）')
      } else {
        const got = new Set(keys)
        const want = new Set(ALL_KEYS)
        const missing = ALL_KEYS.filter((k) => !got.has(k))
        const extra = keys.filter((k) => !want.has(k))
        if (missing.length || extra.length) {
          out.push(
            'RG-011 docs/spec/SP-001.md 字段表与代码 schema 不一致' +
              (missing.length ? `；表里缺：${missing.join('、')}` : '') +
              (extra.length ? `；表里多出：${extra.join('、')}` : '') +
              '（改 schema 必须同批更新 SP-001，反之亦然）',
          )
        }
      }
    }
  }

  // ---- RG-018：adr 的 `adr` 字段与 id 两面对账 ----
  // `id` 是登记身份（`ADR-048a`），`adr` 是 ADR 序号本身（`048a`）——两者必须只差
  // 一个 `ADR-` 前缀（存在的理由：文件名可改名 / 可手抄错，`adr` 字段会与
  // 取代链、渲染列一起被机器读，它是序号的**数据面**，必须与身份严格对账）。
  for (const e of entries) {
    if (!e.meta || e.kind !== 'adr') continue
    if (isEmpty(e.meta.adr)) continue // 必填性由 RG-002 判
    const num = e.meta.adr
    const idNum = String(e.meta.id ?? '').replace(/^ADR-/, '')
    if (num !== idNum) {
      out.push(`${e.rel}：RG-018 adr=\`${num}\` 与 id \`${e.meta.id}\` 不一致（adr 应等于 id 去掉 \`ADR-\` 前缀：\`${idNum}\`）`)
    }
  }

  return out
}

// ==================== 排序与渲染 ====================

/**
 * 拓扑序（前置在前），同层按 id 字典序——**确定性**，是汇总页可再生的前提。
 * 边有两类：`deps`（前置义务，RG-006 成环判红）与 `parent`（分解归属，RG-024 成环判红）——
 * 后者只贡献「父先于子」的渲染序（子开工不要求父闭环，那是 RG-025 的档位单调），
 * 让父子条目在分组表里相邻。有环时退回字典序（环已被判红，渲染不再抢报）。
 */
export function topoOrder(entries) {
  const withMeta = entries.filter((e) => e.meta && !isEmpty(e.meta.id))
  const ids = withMeta.map((e) => e.meta.id)
  const idSet = new Set(ids)
  const indeg = new Map(ids.map((id) => [id, 0]))
  const dependents = new Map(ids.map((id) => [id, []]))
  for (const e of withMeta) {
    for (const d of Array.isArray(e.meta.deps) ? e.meta.deps : []) {
      if (!idSet.has(d)) continue
      indeg.set(e.meta.id, indeg.get(e.meta.id) + 1)
      dependents.get(d).push(e.meta.id)
    }
    const p = e.meta.parent
    if (typeof p === 'string' && idSet.has(p) && p !== e.meta.id) {
      indeg.set(e.meta.id, indeg.get(e.meta.id) + 1)
      dependents.get(p).push(e.meta.id)
    }
  }
  const byIdEntry = new Map(withMeta.map((e) => [e.meta.id, e]))
  const ready = ids.filter((id) => indeg.get(id) === 0).sort(cmpPrio(byIdEntry))
  const order = []
  while (ready.length) {
    const id = ready.shift()
    order.push(id)
    for (const dep of dependents.get(id).slice().sort()) {
      indeg.set(dep, indeg.get(dep) - 1)
      if (indeg.get(dep) === 0) {
        ready.push(dep)
        ready.sort(cmpPrio(byIdEntry))
      }
    }
  }
  if (order.length < ids.length) {
    const seen = new Set(order)
    order.push(...ids.filter((id) => !seen.has(id)).sort())
  }
  return order.map((id) => byIdEntry.get(id)).filter(Boolean)
}

/**
 * parent 链深度（顶层 = 0）——board 组内缩进读它。跨 kind / 指不到的 parent 不计
 * （树按 kind 渲染；那种边已被 RG-024 判红）。visiting 兜底防环上无限递归
 * （board 打印前先跑审计，有环根本到不了这里）。
 */
export function parentDepth(entries) {
  const byId = new Map(entries.filter((e) => e.meta && !isEmpty(e.meta.id)).map((e) => [e.meta.id, e]))
  const depth = new Map()
  const visiting = new Set()
  const of = (e) => {
    if (depth.has(e.meta.id)) return depth.get(e.meta.id)
    if (visiting.has(e.meta.id)) return 0
    visiting.add(e.meta.id)
    const p = e.meta.parent
    const parent = typeof p === 'string' ? byId.get(p) : null
    const d = parent && parent.kind === e.kind ? of(parent) + 1 : 0
    visiting.delete(e.meta.id)
    depth.set(e.meta.id, d)
    return d
  }
  for (const e of entries) if (e.meta && !isEmpty(e.meta.id)) of(e)
  return depth
}

const cell = (s) => String(s ?? '')
  .replace(/\|/g, '\\|')
  .replace(/\s*\n\s*/g, ' ')
  .trim() || '—'

const depsCell = (e) => (Array.isArray(e.meta.deps) && e.meta.deps.length ? e.meta.deps.join('、') : '—')

const byIdCmp = (a, b) => (a.meta.id < b.meta.id ? -1 : 1)

/**
 * 分解树小节：`parent` 字段的投影——按父子关系组织成森林（缩进列表）。
 *
 * **只在该 kind 真的用到了 parent 时出现**：没有分解数据的 kind，汇总页与加机制之前
 * 逐字节一致（新机制零回归面）。编号平铺不体现层级——层级是投影不是身份，
 * 与「目录不参与判定」同款哲学；同状态的父子在分组表里已相邻（topoOrder 纳入
 * parent 边），本小节给跨状态的分解全景（父 doing 子 todo 时表里看不到完整树）。
 * 环防御：seen 之外的平铺补行——环已被 RG-024 判红，渲染不抢报也不死循环。
 */
function treeSection(kind, allEntries) {
  const mine = allEntries.filter((e) => e.kind === kind && e.meta && !isEmpty(e.meta.id))
  if (!mine.some((e) => !isEmpty(e.meta.parent))) return ''
  const byId = new Map(mine.map((e) => [e.meta.id, e]))
  const children = new Map()
  for (const e of mine) {
    const p = e.meta.parent
    if (typeof p !== 'string' || p === e.meta.id || !byId.has(p)) continue
    if (!children.has(p)) children.set(p, [])
    children.get(p).push(e)
  }
  for (const list of children.values()) list.sort(byIdCmp)
  const href = (e) => relFrom(`docs/${kind}`, e.rel)
  // 子项状态计数：父行追加「· 子项 done 2 / todo 1」——「大任务完成多少了」是分解机制
  // 最常用的查询，纯投影（数 children map 的 status，按该 kind 状态机顺序列），不新增字段。
  const statusOrder = Object.keys(KINDS[kind].statusRules ?? {})
  const childSummary = (id) => {
    const kids = children.get(id)
    if (!kids || !kids.length) return ''
    const c = new Map()
    for (const k of kids) c.set(k.meta.status, (c.get(k.meta.status) ?? 0) + 1)
    return `· 子项 ${statusOrder.filter((s) => c.has(s)).map((s) => `${s} ${c.get(s)}`).join(' / ')}`
  }
  const line = (e, depth) =>
    `${'  '.repeat(depth)}- [\`${e.meta.id}\`](${href(e)}) ${cell(e.meta.title)}（${e.meta.status}）` +
    (childSummary(e.meta.id) ? ` ${childSummary(e.meta.id)}` : '')
  const lines = [
    '## 分解树',
    '',
    '分解关系只写在 `parent` 字段（子条目指向父条目、同 kind；编号平铺不体现层级——层级是投影不是身份）。',
    '同状态的父子在上面的分组表里相邻；本小节是跨状态的分解全景。',
    '',
  ]
  const seen = new Set()
  const walk = (e, depth) => {
    if (seen.has(e.meta.id)) return
    seen.add(e.meta.id)
    lines.push(line(e, depth))
    for (const c of children.get(e.meta.id) ?? []) walk(c, depth + 1)
  }
  for (const e of mine.filter((e) => !(typeof e.meta.parent === 'string' && byId.has(e.meta.parent))).sort(byIdCmp)) {
    walk(e, 0)
  }
  for (const e of mine) {
    if (!seen.has(e.meta.id)) lines.push(line(e, 0))
  }
  return lines.join('\n')
}

/** 渲染收尾：有分解数据的 kind 追加「分解树」小节（无 parent 数据时输出不变）。 */
function withTree(kind, allEntries, lines) {
  const tree = treeSection(kind, allEntries)
  if (tree) lines.push(tree, '')
  return lines
}

const HEADER_TASK =
  '> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 从 `docs/task/*.md` 生成，请勿手改。**\n' +
  '> 状态的唯一登记处是各任务文件本身，本页是投影。字段规范见 [[SP-001]]，' +
  '原则与流程见 [[PL-011-登记与汇总机制|PL-011 · 登记与汇总机制]]。\n'

const HEADER_SPEC =
  '> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 从 `docs/spec/*.md` 生成，请勿手改。**\n' +
  '> 状态的唯一登记处是各规范文件本身，本页是投影。字段规范见 [[SP-001]]，' +
  '原则与流程见 [[PL-011-登记与汇总机制|PL-011 · 登记与汇总机制]]。\n'

const HEADER_ADR =
  '> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 从 `docs/adr/*.md` 生成，请勿手改。**\n' +
  '> **ADR 的唯一真相源是 `docs/adr/`**：每条 ADR 的正文（决策 / 理由 / 被否决的方案 /\n' +
  '> 后果与不变量）住它自己的 `## 决策` 小节，规范合法性（`status`）与落地进展（`impl`）住 front-matter。\n' +
  '> 字段规范见 [[SP-001]]，机制见 [[PL-011-登记与汇总机制|PL-011 · 登记与汇总机制]]。\n'

/** posix 相对路径：从目录 `fromDir` 到文件 `toRel`（确定性，不依赖 path.relative 的平台差异）。 */
export function relFrom(fromDir, toRel) {
  const from = fromDir === '' || fromDir === '.' ? [] : fromDir.split('/')
  const to = toRel.split('/')
  let i = 0
  while (i < from.length && i < to.length - 1 && from[i] === to[i]) i++
  return '../'.repeat(from.length - i) + to.slice(i).join('/')
}

function table(header, rows) {
  if (rows.length === 0) return '（无）'
  return [`| ${header.join(' | ')} |`, `|${header.map(() => '---').join('|')}|`, ...rows.map((r) => `| ${r.join(' | ')} |`)].join('\n')
}

function renderTaskIndex(allEntries) {
  const mine = topoOrder(allEntries.filter((e) => e.kind === 'task'))
  const count = (s) => mine.filter((e) => e.meta.status === s).length
  const lines = [
    '# 任务登记索引（自动生成）',
    '',
    HEADER_TASK,
    `**计数**：☐ todo ${count('todo')} · 🟡 doing ${count('doing')} · ☑ done ${count('done')} · ❌ wont ${count('wont')}（共 ${mine.length}）`,
    '',
    '## 待处理（按依赖顺序）',
    table(
      ['编号', '任务', '优先', '轨', '依赖'],
      mine.filter((e) => e.meta.status === 'todo').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.prio), cell(e.meta.track), depsCell(e)]),
    ),
    '',
    '## 进行中',
    table(
      ['编号', '任务', '优先', '轨', '依赖'],
      mine.filter((e) => e.meta.status === 'doing').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.prio), cell(e.meta.track), depsCell(e)]),
    ),
    '',
    '## 已完成',
    // 证据列**不上面**：汇总页是导航——「这条任务怎么落地的」要去读任务文档本身
    // （提交 / 守卫锚在它的 `evidence` 字段与 `## 处理过程` 里）。在列表里展开证据，
    // 只会把每行撑成长段、让人对着第二手摘要判断，而 SP-001 的 `done ⇒ evidence`
    // 判据管的是**条目**有没有写，不是投影展不展示。
    table(
      ['编号', '任务', '轨'],
      mine.filter((e) => e.meta.status === 'done').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.track)]),
    ),
    '',
    '## 不做',
    table(
      ['编号', '任务', '轨', '理由'],
      mine.filter((e) => e.meta.status === 'wont').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.track), cell(e.meta.reason)]),
    ),
    '',
  ]
  return withTree('task', allEntries, lines).join('\n')
}

/** spec 的判据列：`## 判据` 里第一个 `scripts/*.mjs` 指认；没有则是人工判据。 */
function guardCell(e) {
  const ref = refsIn(sectionOf(e.body, '判据')).find((r) => r.startsWith('scripts/'))
  return ref ? `\`${ref}\`` : '人工'
}

/** supersedes 列：显式取代链（冲突消解留下的数据痕迹）。 */
const supCell = (e) => (Array.isArray(e.meta.supersedes) && e.meta.supersedes.length ? e.meta.supersedes.join('、') : '—')

/** clears 列：豁免类规范的清除任务指认（存在性由 RG-016 保证）。 */
const clearsCell = (e) => (Array.isArray(e.meta.clears) && e.meta.clears.length ? e.meta.clears.join('、') : '—')

function renderSpecIndex(allEntries) {
  const mine = allEntries.filter((e) => e.kind === 'spec').slice().sort((a, b) => (a.meta.id < b.meta.id ? -1 : 1))
  const count = (s) => mine.filter((e) => e.meta.status === s).length
  const lines = [
    '# 规范登记索引（自动生成）',
    '',
    HEADER_SPEC,
    `**计数**：active ${count('active')} · draft ${count('draft')} · retired ${count('retired')}（共 ${mine.length}）`,
    '',
    '## 现行规范（active）',
    table(
      ['编号', '规范', '适用范围', '判据', '清除', '取代'],
      mine.filter((e) => e.meta.status === 'active').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.scope), guardCell(e), clearsCell(e), supCell(e)]),
    ),
    '',
    '## 草案（draft）',
    table(
      ['编号', '规范', '适用范围', '取代'],
      mine.filter((e) => e.meta.status === 'draft').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.scope), supCell(e)]),
    ),
    '',
    '## 废止（retired）',
    table(
      ['编号', '规范', '理由', '取代'],
      mine.filter((e) => e.meta.status === 'retired').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.reason), supCell(e)]),
    ),
    '',
  ]
  return withTree('spec', allEntries, lines).join('\n')
}

/** adr 的落地进展列：`impl` 的中文标签（词表见 `ADR_IMPL` / `ADR_IMPL_OTHER`）。 */
function implCell(e) {
  const v = e.meta.impl
  if (typeof v !== 'string' || !v) return '—'
  return ADR_IMPL[v] ?? ADR_IMPL_OTHER[v] ?? v
}

/**
 * `adr` 汇总页。
 *
 * 与前两个 kind 不同的一点：**状态分两组**——`status` 是规范合法性（决定分组），
 * `impl` 是落地进展（决定组内列）。这正是「一条现行决策只落地了一半」能在机读面
 * 表达出来的原因；把它压回单列就会退化成自由文本。
 *
 * 每条另带 `domain`（域）——五册合流后，域从「翻到哪一册」退化为可筛字段。
 */
function renderAdrIndex(allEntries) {
  const mine = allEntries
    .filter((e) => e.kind === 'adr')
    .slice()
    .sort((a, b) => (a.meta.id < b.meta.id ? -1 : 1))
  const count = (s) => mine.filter((e) => e.meta.status === s).length
  const domainOf = (e) => cell(e.meta.domain)
  const lines = [
    '# 架构决策登记索引（自动生成）',
    '',
    HEADER_ADR,
    `**计数**：active ${count('active')} · superseded ${count('superseded')} · withdrawn ${count('withdrawn')}（共 ${mine.length}）`,
    '',
    '## 现行决策（active）',
    table(
      ['编号', '决策', '域', '落地', '取代'],
      mine
        .filter((e) => e.meta.status === 'active')
        .map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), domainOf(e), implCell(e), supCell(e)]),
    ),
    '',
    '## 已被取代（superseded）',
    table(
      ['编号', '决策', '域', '理由', '取代'],
      mine
        .filter((e) => e.meta.status === 'superseded')
        .map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), domainOf(e), cell(e.meta.reason), supCell(e)]),
    ),
    '',
    '## 已回退 / 未落地（withdrawn）',
    table(
      ['编号', '决策', '理由'],
      mine.filter((e) => e.meta.status === 'withdrawn').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.reason)]),
    ),
    '',
    '## 按域分组',
    '',
    ...Object.keys(ADR_DOMAIN).flatMap((dom) => {
      const rows = mine.filter((e) => e.meta.domain === dom)
      if (rows.length === 0) return []
      return [
        `### ${dom}（${rows.length}）`,
        '',
        `> ${ADR_DOMAIN[dom]}`,
        '',
        table(
          ['编号', '决策', '状态', '落地'],
          rows.map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.status), implCell(e)]),
        ),
        '',
      ]
    }),
  ]
  return withTree('adr', allEntries, lines).join('\n')
}

/**
 * 文档类汇总页。
 *
 * 与条目类不同：条目住在 `docs/` 下，而**文档在位存放**——本页的每行都是
 * 一个真实文件，「编号」列同时是链接（从本页到文档的相对路径），读者从索引一步到正文。
 * 分组按 `status`（文档只有一条状态轴：这篇文档还算不算数）。
 */
function renderDocIndex(kind, allEntries) {
  const cfg = KINDS[kind]
  const mine = allEntries.filter((e) => e.kind === kind).slice().sort((a, b) => (a.meta.id < b.meta.id ? -1 : 1))
  const count = (s) => mine.filter((e) => e.meta.status === s).length
  // 从 `docs/<kind>/INDEX.md` 到文档的相对路径（posix，确定性；能一步点到正文）
  const href = (e) => relFrom(`docs/${kind}`, e.rel)
  const idCell = (e) => `[\`${e.meta.id}\`](${href(e)})`
  const lines = [
    `# ${cfg.label}登记索引（自动生成）`,
    '',
    '> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 生成，请勿手改。**',
    `> \`${kind}\`（${cfg.label}）的条目**在位存放**：文件本身就是登记，编号 = 仓根相对路径（去 \`.md\`），`,
    '> `title` 逐字等于正文首个 H1（RG-021 两面对账）；本页是投影。字段规范见 [[SP-001]]，',
    '> 机制见 [[PL-011-登记与汇总机制|PL-011 · 登记与汇总机制]]。',
    '',
    `> **这个 kind 回答什么**：${cfg.blurb}`,
    '',
    `**计数**：active ${count('active')} · draft ${count('draft')} · retired ${count('retired')}（共 ${mine.length}）`,
    '',
    '## 现行文档（active）',
    table(['编号', '标题'], mine.filter((e) => e.meta.status === 'active').map((e) => [idCell(e), cell(e.meta.title)])),
    '',
    '## 草案（draft）',
    table(['编号', '标题'], mine.filter((e) => e.meta.status === 'draft').map((e) => [idCell(e), cell(e.meta.title)])),
    '',
    '## 废止（retired）',
    table(['编号', '标题', '理由'], mine.filter((e) => e.meta.status === 'retired').map((e) => [idCell(e), cell(e.meta.title), cell(e.meta.reason)])),
    '',
  ]
  return withTree(kind, allEntries, lines).join('\n')
}

const RENDERERS = {
  task: renderTaskIndex,
  spec: renderSpecIndex,
  adr: renderAdrIndex,
  // `index` kind 的汇总页 = 仓根**跨 kind 总账**（各 kind 条目数 / 状态分布 / 分解边数）。
  // 它与其余 kind 同形（`<前缀>-000-<X>汇总.md` + `projection: true` + `lists` 声明），
  // 只是内容是横切的那一张表。
  index: renderRootIndex,
}
// 文档类 kind 共用同一个渲染器（kind 的差异只在 `KINDS` 的配置里，不新开脚本）。
for (const kind of Object.keys(KINDS)) {
  if (!(kind in RENDERERS)) RENDERERS[kind] = (allEntries) => renderDocIndex(kind, allEntries)
}

/** 某 kind 的汇总页全文（确定性：同输入同输出，无时间戳）。 */
export function renderKindIndex(kind, allEntries) {
  const selfId = kindIndexId(kind)
  // 汇总页现在是**登记条目**（自己发 front-matter、自己有 id），所以它会出现在
  // `allEntries` 里——**列自己**就是「索引里有一行指向自己」。那不是错，但它是噪音：
  // 读者点进去到的还是这一页。故渲染时把自己剔掉。
  const withoutSelf = allEntries.filter((e) => !(e.kind === kind && e.meta && e.meta.id === selfId))
  const body = RENDERERS[kind](withoutSelf)
  // 清单标记写在**清单旁边**而不是只写在 front-matter（RG-029 两面对账）：
  // 只读这一页就能回答「它声明了什么、正文里对应的东西在不在」。
  const marked = `<!-- list:${kind}-entries owner -->\n\n${body}`
  // `title` 必须**逐字等于正文首个 H1**（RG-021 两面对账）：从渲染结果里取，
  // 不另写一份——两份真相必然漂移，而它漂移时红的是登记守卫、查起来最难。
  const h1 = (body.match(/^# (.+)$/m) || [, cfgFallbackTitle(kind)])[1]
  return [
    '---',
    `kind: ${kind}`,
    `id: ${selfId}`,
    `title: ${h1}`,
    'status: active',
    // `projection: true` —— 这一页**有身份、没内容**。
    //
    // 它需要身份：编号 + id 才能被 `[[PL-000-计划汇总]]` 引用，地址才能交给机制
    // （原先各 kind 汇总页用固定名 `INDEX.md`，指向它的地址只能手写，而生成页对
    // `gen-doc-refs` 与 RF-003 完全隐形——那是本仓唯一一处「手写地址不会被重算」的地方）。
    //
    // 它不需要内容要求：`kind: task` 会拖来 `todo/doing/done/wont` 状态机、
    // `done` 必填 `evidence`、`## 内容` + `## 处理过程` 必备小节（RG-009）——
    // 一份**投影页**满足这些没有意义，它的内容由渲染器决定，不是人写的。
    // 所以：`projection: true` 豁免**内容面**判据（状态词 / 状态×字段矩阵 / 必备小节 /
    // 取代链），保留**身份面**判据（front-matter / 编号形状 / id↔文件名 / created /
    // title==H1 / 清单归属）。豁免是**逐条列明**的，不是笼统放行。
    'projection: true',
    // 汇总页自己**也是一份清单的 owner**：`docs/<kind>/` 下该 kind 的全部条目。
    // 不声明，它就进不了 [RE-007-清单归属总账](../docs/ref/RE-007-清单归属总账.md)——那份总账是「谁拥有哪份清单」的
    // 唯一可查出口，一份没登记的 owner 清单等于让那个出口漏一块。
    // id 用 `<kind>-entries` 而非复用 kind 名：id 是**这份清单**的身份，与「哪个 kind」
    // 有关但不是同一件事（将来某 kind 出第二份清单时不必改这里）。
    `lists: [${kind}-entries=owner]`,
    // `created` 是**本仓首次生成它的日期**，不是「本次运行的时间」：逐次变化的字段
    // 会让 `--check` 永远判漂移（同 `_lib.mjs` 的 stripStamp）。
    'created: 2026-10-14',
    '---',
    '',
    marked,
  ].join('\n')
}

function cfgFallbackTitle(kind) {
  return `${KINDS[kind].label}登记索引（自动生成）`
}

/** 仓根总账页：各 kind 的条目数、状态分布、分解边数、汇总页链接。 */
export function renderRootIndex(allEntries) {
  // 投影页（各 kind 汇总页）**不计入条目数**：它们是「关于这些条目的页」，不是条目。
  // 计进去有两个坏处——读者会把「task 100」读成「100 个任务」，而 `--write`
  // 的收敛要两步（第一次建页、第二次它们进计数、第三次才稳），那条「第二次逐字节
  // 无变化」的不变式因此破掉。
  const isProj = (e) => e?.meta?.projection === true || e?.meta?.projection === 'true'
  const rows = Object.entries(KINDS).map(([kind, cfg]) => {
    const mine = allEntries.filter((e) => e.kind === kind && !isProj(e))
    const dist = Object.keys(cfg.statusRules)
      .map((s) => `${s} ${mine.filter((e) => e.meta && e.meta.status === s).length}`)
      .join(' · ')
    const identity = `编号前缀 \`${ID_PREFIX[kind]}-\``
    // 分解维度：该 kind 的有效 parent 边数（RG-024 保证边都存在且同 kind，这里只数数）。
    const ids = new Set(mine.filter((e) => e.meta && !isEmpty(e.meta.id)).map((e) => e.meta.id))
    const edges = mine.filter((e) => e.meta && typeof e.meta.parent === 'string' && ids.has(e.meta.parent)).length
    return [
      // **编号引用**而不是手写地址：这些汇总页现已登记，指向它们的地址该由
      // `gen-doc-refs` 重算（RF-003 会把手写地址判红——写这行时它当场红过一次）。
      `[[${kindIndexId(kind)}|${kind} · ${cfg.label}]]`,
      identity,
      String(mine.length),
      dist,
      edges ? `${edges} 条分解边` : '—',
      `[[${kindIndexId(kind)}]]`,
    ]
  })
  return [
    '# 登记总账（自动生成）',
    '',
    '> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 生成，请勿手改。**',
    '> 条目各是一个单文档：条目类（task / spec / adr）住 `docs/<kind>/`，文档类（goal / stage / plan /',
    '> arch / ref / guide / index）**在位存放**（文件即条目，kind 只来自 front-matter，目录不参与判定）；',
    '> 机制见 [[PL-011-登记与汇总机制|PL-011 · 登记与汇总机制]]，字段规范见 [[SP-001]]。',
    '',
    table(['类别', '身份', '条目数', '状态分布', '分解', '汇总页'], rows),
    '',
  ].join('\n')
}
