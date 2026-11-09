#!/usr/bin/env node
/**
 * capability-migration-audit — 迁移批次只搬运（EX-003 / EX-007）
 *
 * ## 它治的病
 *
 * 拆分期最贵的走样不是「搬得慢」，而是**红绿不再说明任何事**：一次提交里既搬走
 * 一条能力、又顺手在 `loop` 加一行 `Decl`，端到端用例照旧全绿，于是没人知道那条
 * 用例证明的是「搬运可行」还是「新机制在兜底」。D-247 因此把提交分成两类，本判据
 * 判的就是**这两类各自有没有越界**。
 *
 * | 类别 | 怎么认 | 判什么 |
 * |---|---|---|
 * | **迁移批次** | 提交正文里一行 `Migration: <批次>` | 机制面（`sdk` / `kernel` / `plugin-api`）的**净变化**：非生成 `.rs` 的机制行「新增 − 删除」**不得为正**（净零 / 净减皆合法，`D-404`）；`loop` / `session` 没有新增**带能力名**的源码行（绑定切换可以是通用胶水，为某条能力新加的行不行）；且至少删掉一行旧实现 |
 * | **搬迁批次** | 同一 `Migration:` 段里 `from: <path> (<n>)` / `to: <path> (<n>)`，且净变化 ≤ 0 | `to` 新增行 ≤ `from` 删除行（净增即红，不许借搬迁名义加面）；两侧行数与 diff 实际相符；`from` 在 `plugin-api` 时，`plugin-api` 必须仍有 `pub use` 把该模块转出（否则扩展插件多一条依赖边，E-009）。**净零既不记欠账也不抵账，净减才按迁移抵 1 笔** |
 * | **架构补件** | 在架构面新增 `pub` 项，且没有 `Migration:` | 引用一条**能打开**的 `### D-NNN`；新增的非注释代码不含插件 id 字符串、不含工具名 |
 *
 * 三类之外（只改注释、只删架构面、只动文档）不判——判据不假装能判它没定义的东西。
 *
 * ## 为什么必须显式声明，而不是自动猜
 *
 * 「这次提交是迁移还是补件」是一次提交的**意图**，不是某个文件的属性——同一个
 * 删除行，在迁移批次里是「搬走」，在补件里是「换掉一个不通用的小口子」。这与
 * `arch-change-audit` 的 `Core-Change:` 同一种形状：提交者声明类别，判据判声明
 * 是否成立。没有声明时宁可漏判，也不猜——猜错的判据会逼人写假声明。
 *
 * ## 两半的分工
 *
 * - **EX-003（T2）**：`旧实现已删（同提交）` 这一半在本脚本——迁移批次在 `loop` /
 *   `session` 的删除行必须非零，且新增行不得带能力名（为某条能力新加一行，意味着
 *   两份实现可以同时生效；绑定切换用的通用胶水不算）。`两份实现同时生效` 的**装配期**
 *   另一半归 `seal` 的 `Gap::Duplicate{Name,Event,Fact,View,Actor,Port}`
 *   （`crates/symbio-kernel/src/decl/ring.rs`）：同名声明在装载期红，
 *   不在这里按名字再判一遍（SC-006 的教训：同一形状两个 owner）。
 * - **EX-007（D-247）**：夹带、定制、反向拆分三条失败形态——机制面**零新增**是「反向拆分」
 *   与「夹带」的机械面（`D-283`：删除行不算夹带，旧实现住在机制面的那一批要靠它搬）；
 *   补件的决策号与通用性判「定制」。语义是否真的通用（不是把
 *   一条能力改名成 trait）由 review 判，脚本只保证「写着能力名 / 工具名」无法通过。
 *
 * ## 搬迁的量法是「净变化」，不是「逐树新增」（D-404）
 *
 * 逐树新增把每一次搬迁都判红：`--no-renames` 的 diff 把「搬走一份实现」写成
 * 「旧文件全删 + 新文件全增」，于是 `sdk` 侧看起来凭空多了几百行——字没变、面没宽，
 * 判据却在阻止架构变干净。改为判**机制面总量**：新增行数减删除行数，为正才红。
 * 注释、`use` 行（含 `pub use` 转发）与 `mod` 声明两边都不计——它们是搬家的附带与接线。
 *
 * 净零的搬迁（`from` 删多少、`to` 就增多少）单独记一类 `relocation`：它**不记欠账，
 * 也不抵账**；只有净减才算一次迁移、抵 1 笔。这样「欠账笔数因为换了口径而自己消失」
 * 就不可能发生（`D-404` §3）。
 *
 * 接线行（`use` / `mod` 声明）两边都不计：搬家的合法形态是「实现在新家 + 旧家留一行
 * 转发 + 宿主模块树加一行 `pub mod`」，把这三行算成加面，净零搬迁永远差几行。
 *
 * 口径不追溯：`NET_GUARD` 之前的提交按旧口径判（与 `BEFORE_GUARD` 同一条纪律）。
 *
 * ## 搬家不算新增面（D-279）
 *
 * 机制面**收缩**时（把一个 crate 并进另一棵树），`--no-renames` 的 diff 会说「旧文件全删
 * + 新文件全增」，于是「补件不许带能力名」把本来就在机制面上的行判成新通道：一行
 * `pub const SKILL_TAG: &str = "skill";` 从 core 搬进 sdk，字没变、面没宽，却要按
 * 「为 `skill` 定制」处理。判据要判的是**面变宽了没有**，不是这一行出现在哪一侧——
 * 所以同一行若来自本提交**删掉的文件**，它是搬家（`movedLines`）。
 *
 * 限定「文件被删掉」而不是「提交里出现过同样的删除行」：后者会把「从 `loop` 删掉一条
 * 定制分支、同一行加进 `sdk`」也算搬家，而那是 D-247 反向拆分要拦的形态。
 *
 * 同一把尺在迁移侧的形状是 **D-283**：判的是「面变宽了没有」，所以迁移批在机制面
 * 不许有**新增行**，而**删除行**照放——`S8` 那六行折叠今天就住在 `sdk/projection/` 里，
 * 「旧实现删除与绑定切换同提交」（`D-245`）落到它们身上必然是删机制面的文件。
 * 一条要求「diff = 0」的判据会把整段 `S8` 变成措辞游戏：要么拆成三个提交（中间那个
 * 让 `PP-003` 判红），要么不写 `Migration:`（让 `EX-003` 判不到）。
 *
 * ## 失效形态（回归测试钉的是这几条）
 *
 * - 把「有 `Migration:`」当成「不用判」⇒ 只要写上 trailer 就能夹带，判据变装饰；
 * - 机制面判成「diff = 0」⇒ 折叠长在 `sdk` 的那批（`S8` 六行）搬不动，只能拆提交或
 *   省 trailer（`D-283`）；
 * - 只判新增、不判删除 ⇒ 插件落地而旧实现留着（两份同时生效）照样绿；
 * - 注释里的工具名 / 插件 id 被算成代码 ⇒ 补件几乎每条都红，豁免写满之后没人再看；
 * - 决策号只做正则不查存在 ⇒ 拼一个不存在的 `D-999` 就能进架构；
 * - 搬迁按「逐树新增」判 ⇒ 每一次搬迁都红，判据把架构变干净这件事本身拦下（`D-404`）；
 * - 搬迁按「净变化」判却不要 `from` / `to` 与行数 ⇒ 净增的那一行可以混在「删除更多」
 *   的搬迁里溜过去，借搬迁名义加面；
 * - 搬走 `plugin-api` 的实现却不留 `pub use` ⇒ 扩展插件被迫依赖 `sdk`，多一条依赖边（E-009）。
 *
 * 用法：
 * ```bash
 * node scripts/capability-migration-audit.mjs --range origin/main..HEAD
 * node scripts/capability-migration-audit.mjs --last
 * node scripts/capability-migration-audit.mjs --require-range
 * node scripts/capability-migration-audit.mjs            # 自动取 merge-base(main, HEAD)..HEAD
 * ```
 * 退出码：0 = 通过（或本地无可判范围）；1 = 有违规，或自动化里无可判范围，或夹具读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns EX 能力全面拆分（名册处置与迁移边界）
// @ns D 文档正文与链接
// @codes EX-003 EX-007 D-403 D-404

import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { green, red, yellow, dim } from './color.mjs'
import { BASELINE } from './gate.d/_shared.mjs'
import { blankComments } from './rust-scan.mjs'
import { toolFacts } from './facts/_capability.mjs'
import { pluginIds } from './facts/_plugin-yml.mjs'
import { backfillShas, loadBackfill } from './core-change-backfill.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const argv = process.argv.slice(2)

/** 机制面：迁移批次只许在这里收窄、不许变宽的三棵树（`D-283`；补件才改它）。 */
export const ARCH_TREES = [
  'crates/symbio-sdk',
  'crates/symbio-kernel',
  'crates/symbio-plugin-api',
]

/**
 * 补件的判定面：`sdk` / `kernel`。
 *
 * `plugin-api` 是**插件面**的通用契约——为端口执行体加一个上下文读面、把驱动契约定在这里，
 * 不改内核的公开面与语义，不该要求架构理由；把只有 `plugin-api` 改动的提交判成补件，
 * 会让「通用契约随实现演进」变成每次都要挂决策号的填表。机制面变宽的判据仍在
 * `arch-shape-audit` / `plugin-entry-audit` 从结果面判。
 */
export const CORE_TREES = ['crates/symbio-sdk', 'crates/symbio-kernel']

/** 控制面：迁移批次只许在这里删旧实现。 */
export const CONTROL_TREES = ['crates/symbio-plugin-loop', 'crates/symbio-plugin-session']

/** 门面：扩展插件只许依赖它（E-009）。搬迁的红 3 判的就是这里的转出行。 */
const PLUGIN_API_TREES = ['crates/symbio-plugin-api']

/**
 * **受保护目录的唯一一份**：`EX-002`（新增能力只改自己目录）判的就是这五棵树的
 * 非生成物指纹。`extension-locality-audit` 直接引用本常量，不再抄第二份——
 * 此前两条任务的 exit 与审计脚本各写一套（有的漏 `sdk`、有的漏
 * `plugin-api`），同一提交在三处得到不同结论（D-372）。
 *
 * 取**并集**而不是交集：任何一套旧口径里出现的树都算受保护。少一棵就留一条
 * 「加能力时顺手改这里」的路，而这条路的代价正是本判据要防的。
 */
export const PROTECTED_TREES = [...ARCH_TREES, ...CONTROL_TREES]

/** 迁移批次的声明：提交正文里一行 `Migration: <批次>`。 */
export const MIGRATION_RE = /^\s*Migration[:：]\s*(\S.*)$/m

/** `Core-Change:` 段是补件引用的决策号所在处（与 `arch-change-audit` 同一格式）。 */
const CORE_CHANGE_RE = /^\s*Core-Change[:：]\s*(.+)$/m

/** 生成物：清单 / 装配的镜像，随能力包增删；它不是机制（D-247）。 */
const GENERATED_RE = /\.gen\.rs$/

/**
 * 生成物清单：`scripts/facts/` 各描述符声明的输出。
 *
 * 生成物不都叫 `*.gen.rs`——`plugin-api` 的 `events.rs` / `routes.rs` 是插件清单的
 * 投影，随能力包增删而变。按名字猜会把「加一条能力顺带重生成登记表」判成机制增量；
 * 只认后缀则这两份漏网，补件几乎条条假红。清单直接读描述符的 `OUT_REL`：那是生成器
 * 的唯一注册处（`scripts/gen-facts.mjs` 目录即接线），不在这里抄第二份路径。
 */
export function generatedPaths(root) {
  const out = new Set()
  const dir = path.join(root, 'scripts', 'facts')
  let entries = []
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    if (!e.isFile() || !e.name.endsWith('.mjs') || e.name.startsWith('_') || e.name.endsWith('.test.mjs')) continue
    const m = fs.readFileSync(path.join(dir, e.name), 'utf8').match(/const\s+OUT_REL\s*=\s*['"]([^'"]+)['"]/)
    if (m) out.add(m[1])
  }
  return out
}

/** 本仓的生成物清单（模块加载时读一次；描述符是唯一注册处）。 */
const GENERATED = generatedPaths(repoRoot)

/**
 * 已退役的生成物：曾由描述符产出，退役后描述符不在了，但**历史提交**里的增删仍是
 * 清单镜像的变化，不是机制增量——否则退役本身会把当年那些提交追溯判红。
 *
 * 只在文件于当前树**确已消失**时生效（`isRetiredGenerated`）：若同名文件被重新手写，
 * 它不再算生成物，判据照常扫它。退役决策见 `D-387`。
 */
export const RETIRED_GENERATED = new Set(['crates/symbio-plugin-api/src/view_names.rs'])

/** 退役生成物且当前树里确已不存在。 */
export function isRetiredGenerated(p) {
  return RETIRED_GENERATED.has(p) && !fs.existsSync(path.join(repoRoot, p))
}

/** 测试文件：迁移批次可能改测试而机制不变（断言跟着搬）。 */
const TEST_RE = /\.test\.rs$/

/**
 * 判据诞生之前的提交不追溯。此前 868 个提交按旧纪律写，追溯的唯一效果是逼人
 * 重写历史——那不是收紧，是制造一次不可复核的改写。名单只有一条，且必须是
 * 「判据落地前的最后一个提交」：判据生效的提交是它的子提交。
 */
export const BEFORE_GUARD = {
  sha: '811b2e7d',
  why: 'EX-002 落地时的 HEAD：迁移纪律（D-247）从此后的提交开始判，之前的不追溯（不重写历史）',
}

/**
 * 口径起点：机制面从「逐树新增」改为「总量净变化」（`D-404`）。
 *
 * 判据变更同样不追溯：此 sha **之前**的提交按旧口径判，从它的子提交起才按净变化判。
 * 少了这一条，改口径那天会把历史上所有跨树搬迁按新规则重判一遍——而它们当年是按
 * 旧规则通过的，重判唯一的效果是逼人重写历史。
 *
 * 取「改口径前最后一个提交」：判据生效的提交是它的子提交（与 `BEFORE_GUARD` 同形）。
 */
export const NET_GUARD = {
  sha: 'b08197b3',
  why: 'EX-007 口径改为「ARCH_TREES 总量净变化」前的最后一个提交（D-404）：此前按旧口径，此后按净变化判搬迁',
}

/**
 * 补件的正向证据：给内核 `EventDecl` 加宿主写口的那次提交（D-296）。
 *
 * 为什么不取 `709ee583`（给 `plugin-api` 加前置判断端口）：判定面是 `sdk` / `kernel`，
 * `plugin-api` 是插件面契约、不算补件——夹具必须落在判定面之内，否则「正向证据」
 * 验的是一条判据已经不管的路。
 */
export const FIXTURE = {
  sha: '89490914',
  why: '事件声明的宿主写口（D-296）：在 kernel 新增公开项、挂一条能打开的决策，新增面不含插件 id 与工具名',
}

/**
 * 补件次数的告警阈值（D-269 §5.3）。
 *
 * 它不是棘轮：计数随「判据起点..HEAD」的范围变化，拿它当基线会在换基线时假红。
 * 但它必须刺眼——**补件次数是设计缺口的读数**。阈值取 2：一次补件可以是真的位置缺失，
 * 连着两次就是「每一批都在补架构」，那时该做的是 §5.1 的站位表复核，不是继续加面。
 */
export const SUPPLEMENT_ALARM = 2

/**
 * 累计「补件 : 迁移」的付账读数（D-372）。
 *
 * 判据落地那天的存量 `stock` 是**欠账本金**：从那天起，每多一个补件必须由一次
 * 迁移抵账。`debt > 0` ⇒ 红。不直接拿「比值 ≤ 某数」当阈值——范围基线一变计数
 * 就变，比值会假红（`SUPPLEMENT_ALARM` 上面那条理由仍然成立）；而「欠账不许
 * 变大」与范围无关：两边的计数同源同范围，差值只在真实新增时变。
 *
 * `ratio` 只是读数（打印给人看），不是判据。
 */
export function supplementRatioReading({ supplements, migrations, stock }) {
  const dSupp = supplements - stock.supplements
  const dMig = migrations - stock.migrations
  return {
    dSupp,
    dMig,
    debt: dSupp - dMig,
    ratio: migrations > 0 ? supplements / migrations : Infinity,
  }
}

/**
 * 累计欠账的递减台阶（D-375）。
 *
 * 起算日建账 6 笔（21 − 15）。此后每个检查点把上限压 1，日期一过而上限没达到 ⇒ 红；
 * 最后一个检查点就是硬期限，上限 0——`2026-11-25` 之后任何欠账即红。
 * 到期后只有两种合法结局：追平（欠账 0，台阶表冻结），或另裁一条新决策并指名它换来了
 * 什么（新的到期日与代价）。**不许把检查点往后加来「续期」**——那会让判据永远绿（D-372）。
 *
 * 为什么是台阶而不是每日线性：线性在宽限期内多数日子算出同一个 `allowed`，报错只能说
 * 「还差一点」；台阶把「每批该还多少」写成日期上的义务，每个检查点至少抵 1 笔，
 * 报错能同时给出「当前欠账 / 今日允许 / 下一台阶」。
 */
export const DEBT_SCHEDULE = {
  start: '2026-11-07',
  checkpoints: [
    { by: '2026-11-10', max: 5 },
    { by: '2026-11-13', max: 4 },
    { by: '2026-11-16', max: 3 },
    { by: '2026-11-19', max: 2 },
    { by: '2026-11-22', max: 1 },
    { by: '2026-11-25', max: 0 },
  ],
}

/**
 * 某一天的欠账上限：取**已经到期的最后一个**检查点；一个都没到期时用起算本金。
 *
 * 「已经到期」而不是「下一个检查点」：否则起算日当天就会拿 `2026-11-10 ≤5` 去判
 * 当天的 6 笔，判据一落地就假红。宽限到第一个检查点，之后每过一点降一档。
 */
export function debtAllowance({ today, initialDebt, schedule = DEBT_SCHEDULE }) {
  let allowed = initialDebt
  let next = null
  for (const p of schedule.checkpoints) {
    if (p.by <= today) {
      allowed = p.max
      continue
    }
    next = p
    break
  }
  return { today, allowed, next, deadline: schedule.checkpoints.at(-1)?.by ?? null }
}

/**
 * 到期豁免（`D-375` §5 的「另裁一条新决策」路径）。
 *
 * 豁免**不改判据、不改计数口径**，只把 `EX-007` 的绝对欠账线推迟到 `through`：
 * 到期后照常按台阶判红，续期必须另裁一条决策并写明代价。
 * 条目必须挂一个真实存在的 `### D-NNN`、理由非空、日期可读——形状不对一律不生效，
 * 宁可按台阶判红，也不让一条拼号或匿名的豁免把红藏起来。
 */
export const DEBT_EXEMPTIONS = [
  {
    decision: 'D-403',
    through: '2026-11-12',
    reason:
      '欠账 7 > 6（2026-11-09 台阶）：存量 6 笔来自 T-053 待搬，第 7 笔来自 91b097fc（D-402 的压缩失败投影新增 pub 项）。' +
      '不做凑数迁移：迁移批次必须是真的该搬的东西。另更正：91b097fc 正文所载 188/188 在该提交点不成立（实为 187/188，红项即本欠账）；' +
      '960a692f 的 188/188 成立（该点欠账 6 ≤ 6）',
  },
]

/** 当前仍生效的到期豁免（`null` = 没有）。判定见 `DEBT_EXEMPTIONS` 的注释。 */
export function activeDebtExemption({ today, decisions, exemptions = DEBT_EXEMPTIONS }) {
  return (
    exemptions.find(
      (e) =>
        /^\d{4}-\d{2}-\d{2}$/.test(String(e.through ?? '')) &&
        String(e.through) >= today &&
        decisions.has(String(e.decision ?? '')) &&
        String(e.reason ?? '').replace(/\s+/g, '').length >= 8,
    ) ?? null
  )
}

/**
 * 台阶表自身的判据：日期严格递增、每点至少降 1、末点压到 0。
 *
 * 它挡的是「把台阶改成平的」这类沉默放宽——表一旦不再递减，绝对线就退回本金线，
 * 与 D-372 的增量线完全重合，等于没有期限。读不出形状就判红，不静默跳过。
 */
export function validateDebtSchedule(schedule = DEBT_SCHEDULE, initialDebt = 6) {
  const errors = []
  const points = schedule.checkpoints ?? []
  if (!points.length) errors.push('没有任何检查点')
  let prevMax = initialDebt
  let prevBy = schedule.start ?? ''
  for (const p of points) {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(String(p.by ?? ''))) errors.push(`检查点日期不是 YYYY-MM-DD：${p.by}`)
    if (prevBy && p.by <= prevBy) errors.push(`检查点日期必须严格递增：${prevBy} → ${p.by}`)
    if (!(p.max < prevMax)) errors.push(`检查点上界必须至少降 1：${prevMax} → ${p.max}（${p.by}）`)
    prevMax = p.max
    prevBy = p.by
  }
  if (points.at(-1)?.max !== 0) errors.push('最后一个检查点必须把上限压到 0（到期清零）')
  return errors
}

/** 架构面「新增一个公开项」的形状——机制增量的机械面。 */
const PUB_ITEM_RE =
  /^\s*pub(?:\([^)]*\))?\s+(?:trait|struct|enum|fn|type|const|static|mod|use|async\s+fn|unsafe\s+fn|extern\s+fn)\b/

/** 非测试、非生成的 Rust 源码：迁移期唯一该变少的文件类。 */
export function isCapabilitySource(p) {
  return p.endsWith('.rs') && !TEST_RE.test(p) && !isGenerated(p)
}

/** 生成物：后缀一类的 `*.gen.rs`，加上描述符声明的输出（`events.rs` / `routes.rs` 等）。 */
export function isGenerated(p, generated = GENERATED) {
  return GENERATED_RE.test(p) || generated.has(p) || isRetiredGenerated(p)
}

function inTrees(p, trees) {
  return trees.some((t) => p.startsWith(`${t}/`))
}

/**
 * 解析 `git show --unified=0` 的补丁：按文件给出新增 / 删除行。
 *
 * 文件边界只认 `diff --git` 之后的第一对 `---` / `+++`：一行内容恰好以 `--- ` 开头
 * （删掉一行 `-- …` 的 diff 行就是 `--- …`）不会被当成文件头；新增文件的
 * `--- /dev/null` 与删除文件的 `+++ /dev/null` 各取另一侧作路径，否则整份新文件的
 * 增行会挂到上一个文件上——报错点名错文件，比不报更贵。
 */
export function parsePatch(text) {
  const files = []
  let cur = null
  let oldPath = null
  let header = false
  let inHunk = false
  for (const line of String(text).split('\n')) {
    if (line.startsWith('diff --git ')) {
      cur = null
      oldPath = null
      header = true
      inHunk = false
      continue
    }
    if (header && line.startsWith('--- ')) {
      const p = line.slice(4).trim()
      oldPath = p === '/dev/null' ? null : p.replace(/^a\//, '')
      continue
    }
    if (header && line.startsWith('+++ ')) {
      const p = line.slice(4).trim()
      const deleted = p === '/dev/null'
      const path = deleted ? oldPath : p.replace(/^b\//, '')
      cur = path ? { path, deleted, added: [], removed: [] } : null
      if (cur) files.push(cur)
      header = false
      oldPath = null
      continue
    }
    if (line.startsWith('@@')) {
      inHunk = true
      continue
    }
    if (!inHunk || !cur) continue
    if (line.startsWith('+')) cur.added.push(line.slice(1))
    else if (line.startsWith('-')) cur.removed.push(line.slice(1))
  }
  return files
}

/**
 * 搬家行（D-279 的教训）：本提交里**整份文件被删掉**的那些行。
 *
 * 机制面收缩——把一个 crate 并进另一棵树——在 `--no-renames` 的 diff 里是「旧文件全删
 * + 新文件全增」。这时 EX-007 的「补件不许带能力名」会把**本来就在机制面上的那几行**
 * 判成新通道：`pub const SKILL_TAG: &str = "skill";` 从 core 搬进 sdk，行一个字没变，
 * 面也一分没宽，却要按「为 `skill` 定制」处理。判据要判的是「面变宽了没有」，不是
 * 「这一行有没有出现在新增侧」——所以同一行若来自本次提交删掉的文件，它是搬家。
 *
 * 为什么限定「文件被删掉」，而不是「提交里出现过同样的删除行」：后者会把「从 `loop`
 * 删掉一条定制分支、同一行加进 `sdk`」也算搬家，而那是 D-247 反向拆分要拦的形态
 * （把定制通道塞进机制面）。只有文件整体消失，那一行才轮不到继续当通道。
 */
export function movedLines(patch) {
  const out = new Map()
  for (const file of patch) {
    if (!file.deleted) continue
    for (const raw of file.removed) {
      const key = raw.trim()
      if (!key) continue
      out.set(key, (out.get(key) ?? 0) + 1)
    }
  }
  return out
}

/**
 * 补件在**机制面**（`sdk` / `kernel`）新增的公开项（前几项，供报错点名）。
 *
 * `plugin-api` 的改动不算补件：那是插件面的通用契约，不改内核的公开面与语义。
 * 只看**新增的非注释行**：注释里提一句 `pub fn` 不是机制。
 */
export function mechanismAdditions(patch) {
  const out = []
  for (const file of patch) {
    if (!inTrees(file.path, CORE_TREES) || !isCapabilitySource(file.path)) continue
    for (const line of file.added) {
      // `use` / `mod` 是接线不是机制面（同 `isMechanismLine`）：只重写 `pub use` 的名单
      // （删符号时必然发生）不构成「新增公开项」。
      if (!isMechanismLine(line)) continue
      if (PUB_ITEM_RE.test(line)) out.push({ path: file.path, line: line.trim() })
    }
  }
  return out
}

/**
 * 新增行里的能力名：插件 id 的字符串字面量、或任一工具名。
 *
 * 判的是**代码里的名字**，注释不算（`// notes 曾经在这里分支` 是叙述，不是通道）。
 * 迁移与补件两侧共用这一把尺：迁移侧判「有没有为某条能力新加一行」，补件侧判
 * 「这条通用口子有没有为某条能力定制」。
 */
export function capabilityNameHits(file, ids = [], toolNames = []) {
  const out = []
  for (const raw of file.added) {
    if (!raw.trim()) continue
    const code = blankComments(raw)[0] ?? ''
    // 只作为日志归属 / 展示词出现的纯字面量（如 `plugin_error!("session", …)` 的实参）
    // 不构成「新通道」：它不选择行为、不新增分支，删掉只影响日志署名。判的是
    // 「为某条能力新加行 / 分支 / 常量」，不是字符串出现过。
    if (BARE_LITERAL_LINE.test(code)) continue
    for (const id of ids) {
      if (code.includes(`"${id}"`)) out.push({ path: file.path, line: raw.trim(), what: `插件 id \`"${id}"\`` })
    }
    for (const name of toolNames) {
      const re = new RegExp(`\\b${name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`)
      if (re.test(code)) out.push({ path: file.path, line: raw.trim(), what: `工具名 \`${name}\`` })
    }
  }
  return out
}

/** 一整行只有一个字符串字面量（可带尾逗号 / 分号）：属于署名或数据，不是判据。 */
const BARE_LITERAL_LINE = /^\s*"[^"]*"\s*[,;]?\s*$/

/**
 * 一整行只有一条 `use`（含 `pub use` 转发）：删除符号时 import 行被重写、搬走实现时
 * 留一行转发，都是搬家的附带，不构成机制新增。
 *
 * `pub use` 必须一起不算：搬迁的合法形态就是「实现进 `sdk` + `plugin-api` 留一行转发」，
 * 若把转发行算成新增，净零搬迁会被记成净增 1 行而永远判红（`D-404` 红 3 的另一半）。
 */
const USE_ONLY_LINE = /^\s*(?:pub(?:\s*\(\s*[^)]*\s*\))?\s+)?use\s[^;]+;\s*$/
/**
 * 一整行只是一条模块声明（`mod x;` / `pub mod x;`）：它声明一个文件的存在，
 * 和 `use` 一样是**接线**而不是机制面。搬迁必然要在宿主的模块树上加这么一行。
 */
const MOD_DECL_ONLY_LINE = /^\s*(?:pub(?:\s*\(\s*[^)]*\s*\))?\s+)?mod\s+[A-Za-z_][A-Za-z0-9_]*;\s*$/

function describeHit(h) {
  return `${h.path}: ${h.what} —— ${h.line.slice(0, 80)}`
}

/** 一行是否算「机制面源码」：去掉注释与空行后还剩内容，且不是纯接线（`use` / `mod`）。 */
function isMechanismLine(raw) {
  const code = (blankComments(raw)[0] ?? '').trim()
  return code.length > 0 && !USE_ONLY_LINE.test(code) && !MOD_DECL_ONLY_LINE.test(code)
}

/** 一组 diff 行里算作机制面的条数。 */
function mechanismLineCount(lines) {
  return lines.filter(isMechanismLine).length
}

/**
 * 机制面的**净变化**（`D-404` 的口径）：`ARCH_TREES` 内非生成 `.rs` 的机制行，
 * 新增条数 − 删除条数。为正才是变宽；净零是搬迁；净减是收窄。
 *
 * 只数**删得掉的行**：注释与 `use` 行两边都不计——它们随符号改名 / 搬家而必然重写，
 * 数进来只会让「搬迁」看起来像「加面」。
 */
export function archNetChange(patch) {
  const perFile = []
  let added = 0
  let removed = 0
  for (const f of patch) {
    if (!inTrees(f.path, ARCH_TREES) || !f.path.endsWith('.rs') || isGenerated(f.path)) continue
    const a = mechanismLineCount(f.added)
    const r = mechanismLineCount(f.removed)
    if (!a && !r) continue
    added += a
    removed += r
    perFile.push({ path: f.path, added: a, removed: r })
  }
  return { added, removed, net: added - removed, perFile }
}

/** 提交正文里的搬迁声明：`from: <path> (<n>)` / `to: <path> (<n>)`。 */
const FROM_RE = /^[ \t]*from[:：][ \t]*(\S+)[ \t]*\(([0-9]+)\)[ \t]*$/gm
const TO_RE = /^[ \t]*to[:：][ \t]*(\S+)[ \t]*\(([0-9]+)\)[ \t]*$/gm

/**
 * 解析搬迁声明。
 *
 * 一次搬迁可能搬多个文件（实现 + 它的测试），所以 `from` / `to` 成对出现、可有若干对：
 * 按出现顺序配对，`pairs[i] = { from, to }`。`complete` 要求两侧条数相等且至少一对
 * ——差一条就等于有一侧没写行数。
 */
export function parseRelocationHeader(message) {
  const text = String(message ?? '')
  const froms = [...text.matchAll(FROM_RE)].map((m) => ({ path: m[1], lines: Number(m[2]) }))
  const tos = [...text.matchAll(TO_RE)].map((m) => ({ path: m[1], lines: Number(m[2]) }))
  const pairs = []
  for (let i = 0; i < Math.max(froms.length, tos.length); i += 1) {
    pairs.push({ from: froms[i] ?? null, to: tos[i] ?? null })
  }
  return {
    from: froms[0] ?? null,
    to: tos[0] ?? null,
    pairs,
    declared: froms.length > 0 || tos.length > 0,
    complete: froms.length > 0 && froms.length === tos.length,
  }
}

/** 路径所属的树（crate 根）；不在 `crates/` 下则返回路径本身。 */
function treeOf(filePath) {
  const m = String(filePath).match(/^crates\/[^/]+/)
  return m ? m[0] : String(filePath)
}

/**
 * 跨树搬行：本提交把**同一份文本**从一棵树搬到另一棵树。
 *
 * 这是「没写 `from` / `to` 却搬了东西」唯一可机械判定的证据——因为 `--no-renames`
 * 会把整份搬迁写成「旧文件全删 + 新文件全增」，两边的行文本一一对上。
 * 同树内的删了又加不算（那是同一处重写，不是搬家）。
 */
export function crossTreeMoves(patch) {
  const removedLines = new Map() // 行文本 -> 它被删所在的树集合
  for (const f of patch) {
    if (!inTrees(f.path, ARCH_TREES) || !f.path.endsWith('.rs') || isGenerated(f.path)) continue
    for (const raw of f.removed) {
      if (!isMechanismLine(raw)) continue
      const code = (blankComments(raw)[0] ?? '').trim()
      const trees = removedLines.get(code) ?? new Set()
      trees.add(treeOf(f.path))
      removedLines.set(code, trees)
    }
  }
  const samples = []
  let count = 0
  for (const f of patch) {
    if (!inTrees(f.path, ARCH_TREES) || !f.path.endsWith('.rs') || isGenerated(f.path)) continue
    for (const raw of f.added) {
      if (!isMechanismLine(raw)) continue
      const code = (blankComments(raw)[0] ?? '').trim()
      const from = removedLines.get(code)
      if (!from) continue
      const here = treeOf(f.path)
      if (![...from].some((tree) => tree !== here)) continue
      count += 1
      if (samples.length < 3) samples.push(`${f.path}: ${code.slice(0, 60)}`)
    }
  }
  return { count, samples }
}

/** 模块名：`.../text.rs` ⇒ `text`；`.../port/mod.rs` ⇒ `port`。 */
function moduleStem(filePath) {
  const parts = String(filePath).split('/')
  const base = (parts.pop() ?? '').replace(/\.rs$/, '')
  if (base === 'mod' || base === 'lib') return parts.pop() ?? base
  return base
}

/**
 * 提交类别。
 *
 * - `Migration:` + 完整的 `from` / `to` + 机制面**净零** ⇒ `relocation`（不记欠账、不抵账）；
 * - `Migration:` + 完整声明 + 机制面**净减** ⇒ `migration`（真收窄，抵 1 笔）；
 * - `Migration:` 其余情况 ⇒ `migration`（净增由 `relocationErrors` 判红）；
 * - 没有 `Migration:` 而确实跨树搬了同一份文本 ⇒ `relocation`（缺声明，判红）；
 * - 没有 `Migration:` 而在 sdk / kernel 新增公开项 ⇒ `supplement`。
 *
 * `net = false` 是 `NET_GUARD` 之前的旧口径：搬迁概念不存在，有 trailer 即迁移。
 */
export function classifyCommit({ message, patch, net = true }) {
  const text = String(message ?? '')
  if (MIGRATION_RE.test(text)) {
    if (!net) return 'migration'
    const header = parseRelocationHeader(text)
    if (header.complete) {
      return archNetChange(patch).net === 0 ? 'relocation' : 'migration'
    }
    return 'migration'
  }
  // 跨树搬家先判：它本来就会在 `to` 侧显示成「新增公开项」，按补件判只会逼人补一个
  // 与本意无关的决策号；缺声明要红在「没写搬迁声明」，不是「没写补件理由」（D-404 红 2）。
  if (net && crossTreeMoves(patch).count > 0) return 'relocation'
  if (mechanismAdditions(patch).length > 0) return 'supplement'
  return null
}

/**
 * 搬迁批次的判定（`D-404` 的三条红线）。
 *
 * 只在 `NET_GUARD` 之后的提交上跑：之前的历史是按旧口径通过的，重判等于要求重写历史。
 */
export function relocationErrors({ message, patch }) {
  const errors = []
  const text = String(message ?? '')
  const header = parseRelocationHeader(text)
  const moves = crossTreeMoves(patch)

  if (!MIGRATION_RE.test(text)) {
    if (moves.count > 0) {
      errors.push(
        `EX-007 本提交跨树搬了 ${moves.count} 行机制面源码却没有 \`Migration:\` 段（如 ${moves.samples[0]}）` +
          ' —— 搬迁必须显式声明，否则与「在机制面新增一行」在 diff 里无法区分（D-404）',
      )
    }
    return { errors }
  }

  if (moves.count > 0 && !header.complete) {
    errors.push(
      `EX-007 本提交跨树搬了 ${moves.count} 行机制面源码，但 \`Migration:\` 段没写 \`from: <path> (<行数>)\` / \`to: <path> (<行数>)\`` +
        ' —— 没有两侧行数就无法判「净增」（D-404 红 2）',
    )
    return { errors }
  }

  if (!header.complete) return { errors }

  for (const pair of header.pairs) {
    if (!pair.from || !pair.to) continue
    const fromFile = patch.find((f) => f.path === pair.from.path)
    const toFile = patch.find((f) => f.path === pair.to.path)
    if (!fromFile) {
      errors.push(
        `EX-007 \`from:\` 指的 \`${pair.from.path}\` 不在本提交的 diff 里 —— 行数无法核验（D-404 红 2）`,
      )
    } else if (fromFile.removed.length !== pair.from.lines) {
      errors.push(
        `EX-007 \`from:\` 写了 ${pair.from.lines} 行，实际从 \`${pair.from.path}\` 删了 ${fromFile.removed.length} 行` +
          ' —— 行数写错就等于没写（D-404 红 2）',
      )
    }
    if (!toFile) {
      errors.push(
        `EX-007 \`to:\` 指的 \`${pair.to.path}\` 不在本提交的 diff 里 —— 行数无法核验（D-404 红 2）`,
      )
    } else if (toFile.added.length !== pair.to.lines) {
      errors.push(
        `EX-007 \`to:\` 写了 ${pair.to.lines} 行，实际往 \`${pair.to.path}\` 加了 ${toFile.added.length} 行` +
          ' —— 行数写错就等于没写（D-404 红 2）',
      )
    }
  }

  // 红 1（逐对）：`to` 侧新增的**机制行**多于 `from` 侧删掉的机制行 ⇒ 借搬迁加面。
  // 比的是机制行，不是物理行：文件名相同的空白 / 注释段会被 diff 当成未改动，
  // 物理行数天然对不上（那两行不是「搬过去的新面」，是「没动的旧行」）。
  for (const pair of header.pairs) {
    if (!pair.from || !pair.to) continue
    const fromFile = patch.find((f) => f.path === pair.from.path)
    const toFile = patch.find((f) => f.path === pair.to.path)
    if (!fromFile || !toFile) continue
    const grew = mechanismLineCount(toFile.added) - mechanismLineCount(fromFile.removed)
    if (grew > 0) {
      errors.push(
        `EX-007 \`${pair.to.path}\` 比 \`${pair.from.path}\` 多搬了 ${grew} 行机制面源码` +
          ' —— 不许借搬迁名义在机制面加面（D-404 红 1）',
      )
    }
  }

  // 红 1（总量）：搬迁可能夹带第三处的新增（搬 A 的同时在 sdk 里加了 B）。
  const total = archNetChange(patch)
  if (total.net > 0) {
    errors.push(
      `EX-007 机制面净增 ${total.net} 行（新增 ${total.added} / 删除 ${total.removed}）` +
        ' —— 搬迁必须是净零或净减，多出来的机制增量拆成带 `### D-NNN` 的独立补件提交（D-404 红 1）',
    )
  }

  // 红 3：搬走的是 `plugin-api` 的实现时，门面必须仍把该模块转出去。
  // 否则扩展插件只能直接依赖 `sdk`——那是 E-009 要消灭的那条依赖边。
  for (const pair of header.pairs) {
    if (!pair.from || !pair.to) continue
    // 测试文件不是公开面：搬走它没有「插件少一条路」的问题，不必留转发。
    if (!isCapabilitySource(pair.from.path)) continue
    if (!inTrees(pair.from.path, PLUGIN_API_TREES) || inTrees(pair.to.path, PLUGIN_API_TREES)) {
      continue
    }
    const stem = moduleStem(pair.from.path)
    const reexported = patch.some(
      (f) =>
        inTrees(f.path, PLUGIN_API_TREES) &&
        f.added.some((raw) => {
          const code = (blankComments(raw)[0] ?? '').trim()
          return /\bpub\s+use\b/.test(code) && new RegExp(`\\b${stem}\\b`).test(code)
        }),
    )
    if (!reexported) {
      errors.push(
        `E-009 从 \`plugin-api\` 搬走了 \`${pair.from.path}\`，却没有在同一文件树里留 \`pub use\` 把 \`${stem}\` 转出` +
          ' —— 扩展插件只许依赖 `plugin-api`，缺这行转发它们就得多依赖 `sdk`（D-404 红 3）',
      )
    }
  }

  return { errors }
}

/**
 * 迁移批次的判定（EX-003 的「同提交删除」+ EX-007 的两条 containment）。
 *
 * 机制面判的是**净变化**（`D-404`）：迁移与搬迁都可能把实现从这棵树搬到那棵树，
 * 逐树看新增必然把搬迁判成变宽。判「新增行数 − 删除行数」：为正才是变宽；净零 / 净减
 * 都合法。`S8` 那六行折叠住在 `sdk/projection/`，「搬走 = 同一提交删掉旧实现」
 * （`D-245`）落到它们身上必然是删机制面的文件——判「diff = 0」会把这族的删除半边
 * 也拦下，于是整段只能拆提交（中间那个让 `PP-003` 判红）或不写 `Migration:`
 * （让 `EX-003` 判不到），两条都是把判据变成措辞游戏。
 *
 * `net = false` 是 `NET_GUARD` 之前的旧口径（逐树新增），只为不追溯历史而保留。
 */
export function migrationErrors({
  sha,
  message = '',
  patch,
  pluginIds: ids = [],
  toolNames = [],
  net = true,
}) {
  const errors = []

  if (net) {
    const { net: delta, added, removed, perFile } = archNetChange(patch)
    if (delta > 0) {
      const wider = perFile.filter((f) => f.added > f.removed)
      errors.push(
        `EX-007 迁移提交在机制面净增了 ${delta} 行源码（新增 ${added} / 删除 ${removed}；` +
          `${wider.map((f) => `\`${f.path}\` +${f.added - f.removed}`).join(' / ')}）` +
          ' —— 机制面只许收窄：净增的机制增量要拆成带 `### D-NNN` 的独立补件提交；搬迁必须是净零或净减（D-247 / D-404）',
      )
    }
  } else {
    const archAdded = patch.filter(
      (f) =>
        inTrees(f.path, ARCH_TREES) &&
        f.path.endsWith('.rs') &&
        !isGenerated(f.path) &&
        // 判的是源码变宽：纯注释行（含 `arch-shape-allow` 登记）与纯 `use` 行
        // （删符号时同一行被重写的必然结果）不构成机制新增。
        f.added.some(isMechanismLine),
    )
    if (archAdded.length) {
      errors.push(
        `EX-007 迁移提交在机制面新增了源码（${archAdded.map((f) => `\`${f.path}\``).join(' / ')}）` +
          ' —— 机制面只许收窄：迁移批次在 sdk / kernel / plugin-api 不许有新增行，确有机制增量就拆成带 `### D-NNN` 的独立补件提交（D-247 / D-283）',
      )
    }
  }

  const control = patch.filter((f) => inTrees(f.path, CONTROL_TREES) && isCapabilitySource(f.path))
  const named = control.flatMap((f) => capabilityNameHits(f, ids, toolNames))
  if (named.length) {
    errors.push(
      `EX-007 迁移提交在控制面新增了 ${named.length} 行带能力名的源码（如 ${describeHit(named[0])}）` +
        ' —— 绑定切换可以是通用胶水，为某条能力新加行 / 分支 / 常量不行：那会让两份实现同时生效',
    )
  }

  const deletions = control.reduce((n, f) => n + f.removed.length, 0)
  if (deletions === 0) {
    errors.push(
      'EX-003 迁移提交没有删除 `loop` / `session` 的非测试源码——插件落地与旧实现删除必须同提交（D-245），否则两份实现同时生效',
    )
  }

  return { sha, errors }
}

/**
 * 补件的判定（EX-007 的决策号与通用性）。
 *
 * 决策号取 `Core-Change:` 段；没有这一段（只动 `plugin-api` 的补件不要求它）则取
 * 整条信息里任意一个 `D-NNN`。**必须能打开**：拼一个不存在的号等于没写。
 */
export function supplementErrors({
  sha,
  message,
  patch,
  decisions,
  pluginIds: ids = [],
  toolNames = [],
  moved = new Map(),
}) {
  const errors = []
  const text = String(message ?? '')
  const trailer = text.match(CORE_CHANGE_RE)
  const scope = trailer ? trailer[1] : text
  const cited = [...scope.matchAll(/\bD-\d{3,}\b/g)].map((m) => m[0])
  if (cited.length === 0) {
    errors.push(
      'EX-007 补件没有决策号 —— 架构增量必须挂一条 `Core-Change: D-NNN`，说清哪条能力因缺它而迁不动、它服务哪些能力',
    )
  } else if (!cited.some((d) => decisions.has(d))) {
    errors.push(
      `EX-007 补件引用的决策号在 docs/003-design/ 里打不开（${[...new Set(cited)].join(' / ')}）—— 写错的号等于没写`,
    )
  }

  const offenders = []
  const movedLeft = new Map(moved)
  for (const file of patch) {
    if (!inTrees(file.path, ARCH_TREES) || !isCapabilitySource(file.path)) continue
    for (const hit of capabilityNameHits(file, ids, toolNames)) {
      // 搬家行不是新通道（`movedLines`）：同一条已删除文件里的行，按搬家消耗掉。
      const n = movedLeft.get(hit.line) ?? 0
      if (n > 0) {
        movedLeft.set(hit.line, n - 1)
        continue
      }
      offenders.push(hit)
    }
  }
  if (offenders.length) {
    errors.push(
      `EX-007 补件不是通用的（${offenders.length} 处带能力名 / 工具名，如 ${describeHit(offenders[0])}）` +
        ' —— 补件至少服务一条真实能力，但不是为某一条能力定制的通道（D-247）',
    )
  }

  return { sha, errors }
}

/**
 * 豁免名单自身的形状：必须是 7–40 位提交号 + 非空理由。一条不解释自己的豁免
 * 就是永久后门。
 */
export function validateBeforeGuard(entry) {
  const errors = []
  if (!entry?.sha || !/^[0-9a-f]{7,40}$/.test(entry.sha)) {
    errors.push(`${entry?.sha ?? '(无 sha)'}：判据起点必须有 7–40 位提交号`)
  }
  if (typeof entry?.why !== 'string' || !entry.why.trim()) {
    errors.push(`${entry?.sha ?? '(无 sha)'}：判据起点没有理由——不解释起点的豁免就是永久后门`)
  }
  return errors
}

/** `docs/003-design/` 里存在的决策号（与 arch-change-audit 同一口径）。 */
export function knownDecisions(root) {
  const out = new Set()
  const walk = (dir) => {
    let entries
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true })
    } catch {
      return
    }
    for (const e of entries) {
      const p = path.join(dir, e.name)
      if (e.isDirectory()) walk(p)
      else if (e.isFile() && e.name.endsWith('.md')) {
        for (const m of fs.readFileSync(p, 'utf8').matchAll(/^### (D-\d+)/gm)) out.add(m[1])
      }
    }
  }
  walk(path.join(root, 'docs', '003-design'))
  return out
}

function git(args) {
  const r = spawnSync('git', args, {
    cwd: repoRoot,
    encoding: 'utf8',
    shell: false,
    maxBuffer: 64 * 1024 * 1024,
  })
  if (r.status !== 0) return null
  return r.stdout
}

const REC = '\x01'
const PRETTY = `%x01%H%x00%B`
const RAW_FILE_LINE_RE = /^:[0-7]{6} [0-7]{6} /

/** 与 extension-locality-audit 同一形状：`git log --raw` 的记录解析。 */
export function parseRawCommits(out) {
  const commits = []
  for (const rec of String(out).split(REC)) {
    if (!rec.trim()) continue
    const nul = rec.indexOf('\x00')
    if (nul < 0) continue
    const sha = rec.slice(0, nul).trim()
    const lines = rec.slice(nul + 1).split('\n')
    const firstFile = lines.findIndex((l) => RAW_FILE_LINE_RE.test(l))
    const message = (firstFile < 0 ? lines : lines.slice(0, firstFile)).join('\n')
    const files =
      firstFile < 0
        ? []
        : lines
            .slice(firstFile)
            .map((l) => l.slice(l.lastIndexOf('\t') + 1).trim())
            .filter(Boolean)
    commits.push({ sha, message, files })
  }
  return commits
}

function patchOf(sha) {
  const out = git([
    'show',
    '--format=',
    '--unified=0',
    '--no-renames',
    sha,
    '--',
    ...ARCH_TREES,
    ...CONTROL_TREES,
  ])
  return out === null ? null : parsePatch(out)
}

function touchesAuditedTree(files) {
  return files.some((p) => (inTrees(p, ARCH_TREES) || inTrees(p, CONTROL_TREES)) && p.endsWith('.rs'))
}

function subjectOf(message) {
  return String(message ?? '').split('\n')[0] ?? ''
}

function isCiEnv(env = process.env) {
  const v = env.CI ?? env.GITHUB_ACTIONS ?? ''
  return v !== '' && v !== 'false' && v !== '0'
}

export function requiresRange(args = argv, env = process.env) {
  return args.includes('--require-range') || isCiEnv(env)
}

function resolveRange() {
  const idx = argv.indexOf('--range')
  if (idx >= 0) return argv[idx + 1] || null
  if (argv.includes('--last')) return 'HEAD~1..HEAD'
  for (const base of ['origin/main', 'origin/master', 'main']) {
    const mb = git(['merge-base', base, 'HEAD'])
    if (mb && mb.trim()) return `${mb.trim()}..HEAD`
  }
  return null
}

function reportFailures(bad) {
  console.log(red(`\ncapability-migration-audit 未通过：${bad.length} 个提交`))
  for (const r of bad) {
    console.log(`\n  ${r.sha.slice(0, 8)} ${r.subject || ''}`)
    for (const e of r.errors) console.log(`    ${e}`)
  }
  console.log(
    dim(
      '\n  两类提交各有一条退路：迁移批次只删不增；机制增量拆成独立补件，并挂一条能打开的 `### D-NNN`。',
    ),
  )
}

function judgeCommit({ sha, message, patch, decisions, ids, tools, net = true }) {
  const kind = classifyCommit({ message, patch, net })
  if (kind === 'migration') {
    const errors = migrationErrors({ sha, message, patch, pluginIds: ids, toolNames: tools, net }).errors
    // 迁移也可能是「净减式搬迁」：三条红线对迁移同样成立（声明 / 行数 / api 转发）。
    if (net) errors.push(...relocationErrors({ message, patch }).errors)
    return { sha, subject: subjectOf(message), kind, errors }
  }
  if (kind === 'relocation') {
    return {
      sha,
      subject: subjectOf(message),
      kind,
      errors: relocationErrors({ message, patch }).errors,
    }
  }
  if (kind === 'supplement') {
    // 搬家行只看**被删掉的文件**，故单独取一次 diff（`patch` 收窄在机制面 + 控制面两棵树，
    // 达不到像旧 core 那种已经消失的路径上）。
    const moved = movedLines(parsePatch(git(['show', '--format=', '--unified=0', '--no-renames', '--diff-filter=D', sha]) ?? ''))
    return {
      sha,
      subject: subjectOf(message),
      kind,
      errors: supplementErrors({ sha, message, patch, decisions, pluginIds: ids, toolNames: tools, moved })
        .errors,
    }
  }
  return { sha, subject: subjectOf(message), kind, errors: [] }
}

/** 今天的本地日期（`YYYY-MM-DD`）：台阶按自然日判，本地跨月当天就该降档。 */
function todayIso(now = new Date()) {
  const p = (n) => String(n).padStart(2, '0')
  return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`
}

function main() {
  const guardErrors = [...validateBeforeGuard(BEFORE_GUARD), ...validateBeforeGuard(NET_GUARD)]
  if (guardErrors.length) {
    for (const e of guardErrors) console.error(red(`✗ 判据起点：${e}`))
    return 1
  }
  if (git(['rev-parse', '--verify', `${NET_GUARD.sha}^{commit}`]) === null) {
    console.error(red(`✗ capability-migration-audit：口径起点 ${NET_GUARD.sha} 打不开 —— 新旧口径无法分界`))
    return 1
  }

  const decisions = knownDecisions(repoRoot)
  if (decisions.size === 0) {
    console.error(red('✗ capability-migration-audit：docs/003-design/ 里一个 `### D-NNN` 都读不到——补件的资格线没有输入'))
    return 1
  }

  // 逐提交补记表（D-333）：与 `arch-change-audit` 共用同一张表与同一套形状判据。
  // 表只补历史提交的署名，不退役 EX-007：表外的提交缺决策号照样红。
  const backfill = loadBackfill(repoRoot)
  if (backfill.errors.length) {
    for (const e of backfill.errors) console.error(red(`✗ 补记表：${e}`))
    return 1
  }
  const BACKFILLED_EX = backfillShas(backfill.entries, 'EX-007')
  const ids = pluginIds(repoRoot)
  const tools = [...toolFacts(repoRoot).keys()]

  // ── 正向证据：补件夹具（与范围无关）──
  if (git(['rev-parse', '--verify', `${FIXTURE.sha}^{commit}`]) === null) {
    console.error(red(`✗ capability-migration-audit：夹具提交 ${FIXTURE.sha} 打不开（浅克隆？先 \`git fetch --unshallow\`）`))
    return 1
  }
  const fixtureMsg = git(['log', '-1', '--format=%B', FIXTURE.sha])
  const fixturePatch = patchOf(FIXTURE.sha)
  if (fixtureMsg === null || fixturePatch === null) {
    console.error(red(`✗ capability-migration-audit：夹具提交 ${FIXTURE.sha} 的 diff 读不出`))
    return 1
  }
  const fixture = judgeCommit({
    sha: FIXTURE.sha,
    message: fixtureMsg,
    patch: fixturePatch,
    decisions,
    ids,
    tools,
  })
  if (fixture.kind !== 'supplement') {
    console.error(
      red(`✗ capability-migration-audit：夹具提交 ${FIXTURE.sha} 没有被认成补件——夹具指错了对象（${FIXTURE.why}）`),
    )
    return 1
  }
  if (fixture.errors.length) {
    reportFailures([fixture])
    return 1
  }

  // ── 向前判据：判据起点之后、提交范围之内的提交 ──
  const range = resolveRange()
  if (!range) {
    const msg = '⊘ capability-migration-audit：找不到可判的提交范围（无 origin/main 也无 --range）——跳过，不当作通过'
    if (requiresRange()) {
      console.error(
        red(
          `✗ capability-migration-audit：${msg.slice(1)}；${argv.includes('--require-range') ? '本次要求有可判范围' : '自动化环境（CI 变量）要求有可判范围'} ⇒ 判红`,
        ),
      )
      return 1
    }
    console.log(yellow(msg))
    return 0
  }

  const log = git(['log', '--no-merges', '--reverse', '--raw', '--no-renames', `--pretty=format:${PRETTY}`, range])
  if (log === null) {
    console.error(red(`✗ capability-migration-audit：解析提交范围失败：${range}`))
    return 1
  }
  const forwardOut = git(['rev-list', '--no-merges', `${BEFORE_GUARD.sha}..HEAD`])
  if (forwardOut === null) {
    console.error(red(`✗ capability-migration-audit：读不出判据起点之后的提交（${BEFORE_GUARD.sha}..HEAD）`))
    return 1
  }
  const forward = new Set(forwardOut.split('\n').filter(Boolean))
  const commits = parseRawCommits(log).filter((c) => forward.has(c.sha))

  // 口径分界（D-404）：`NET_GUARD` 之后的提交才按「净变化」判；之前的按旧口径（逐树新增）。
  const netOut = git(['rev-list', `${NET_GUARD.sha}..HEAD`])
  if (netOut === null) {
    console.error(red(`✗ capability-migration-audit：读不出口径起点之后的提交（${NET_GUARD.sha}..HEAD）`))
    return 1
  }
  const netCommits = new Set(netOut.split('\n').filter(Boolean))

  const bad = []
  /**
   * 不追溯名单：`ARCH_TREES` 从旧 core 扩到 `symbio-sdk` 之后（D-279），
   * 那几批把域搬进 SDK 的提交会被**按新范围重新判一遍**——它们当年在旧范围下通过，
   * 而现在「新增的 sdk 文件」看起来像架构补件。
   *
   * 判据不追溯历史（`BEFORE_GUARD` 同一条纪律），所以逐条列出来，各自写明理由。
   * 这张表**只增不减地老去**：新提交不许进来，进来就等于「我不打算守这条」。
   *
   * 为什么 `movedLines` 接不住这两条：那是**复制式**搬迁——原文件没有消失，而是被改成
   * 一行转发（`pub use symbio_sdk::<域>::*;`）。diff 里它既不是删除、也不是同一行搬家，
   * 所以 sdk 侧看起来是全新的面。D-279 那种「整份文件搬走」由 `movedLines` 接住，
   * 这一种只能靠名单——两种形状的区别就是「旧文件还在不在」。
   *
   * `72fb57af`（D-259）在本轮复核里**已不再被命中**（那三域搬的行不带插件 id / 工具名），
   * 故删除：一条打不中的豁免不是保险，是多一条没人复核的后门（D-280）。
   */
  const GRANDFATHERED = new Map([
    ['7d2811b9', 'D-258 第一批：7 个域搬进 SDK，原文件留转发桩（复制式搬迁，不是搬家）'],
    ['09906cbb', 'D-258 第三批：agent / embedding 两域，同上'],
    ['1069f346', 'D-313 P2-3b2-1：配置 / facts 读点切到 session 端口同源投影（补件随 P2 批次绿点，缺 Core-Change 行；不追改历史）'],
  ])
  let migrations = 0
  let relocations = 0
  let supplements = 0
  /** 逐笔账：欠账明细要能点到具体提交（`D-403`），不能只印总数。 */
  const ledger = []
  for (const c of commits) {
    if (!touchesAuditedTree(c.files)) continue
    const patch = patchOf(c.sha)
    if (patch === null) {
      bad.push({ sha: c.sha, subject: subjectOf(c.message), errors: ['读不出本提交的补丁——判据没有输入，宁红不静默'] })
      continue
    }
    const r = judgeCommit({
      sha: c.sha,
      message: c.message,
      patch,
      decisions,
      ids,
      tools,
      net: netCommits.has(c.sha),
    })
    if (r.kind === 'migration') migrations += 1
    if (r.kind === 'relocation') relocations += 1
    if (r.kind === 'supplement') supplements += 1
    // 净零搬迁也进账本：它不记欠账、不抵账，但读数要解释得清「这笔为什么不算」（D-404 §3）。
    if (r.kind) ledger.push({ sha: r.sha.slice(0, 8), subject: r.subject, kind: r.kind })
    if (r.errors.length && GRANDFATHERED.has(r.sha.slice(0, 8))) continue
    if (r.errors.length && BACKFILLED_EX.has(r.sha.slice(0, 8))) continue
    if (r.errors.length) bad.push(r)
  }
  if (bad.length) {
    reportFailures(bad)
    return 1
  }

  console.log(
    green(
      `✓ capability-migration-audit：夹具 ${FIXTURE.sha}（补件）` +
        `+ 判据起点 ${BEFORE_GUARD.sha} 之后 ${migrations} 个迁移批次 / ${relocations} 个净零搬迁 / ${supplements} 个补件，全部只搬运`,
    ),
  )

  // ── 累计付账线（D-372）：补件必须由迁移抵账，欠账不许变大 ──
  const stock =
    BASELINE.capabilitySupplements !== undefined && BASELINE.capabilityMigrations !== undefined
      ? { supplements: BASELINE.capabilitySupplements, migrations: BASELINE.capabilityMigrations }
      : null
  if (!stock) {
    console.error(
      red(
        '✗ capability-migration-audit：`BASELINE.capabilitySupplements` / `capabilityMigrations` 读不到' +
          '——付账判据没有输入，宁红不静默',
      ),
    )
    return 1
  }
  const initialDebt = stock.supplements - stock.migrations
  const scheduleErrors = validateDebtSchedule(DEBT_SCHEDULE, initialDebt)
  if (scheduleErrors.length) {
    console.error(red(`✗ capability-migration-audit：欠账台阶表失效——${scheduleErrors.join('；')}`))
    return 1
  }
  const reading = supplementRatioReading({ supplements, migrations, stock })
  const ratioText = Number.isFinite(reading.ratio) ? reading.ratio.toFixed(2) : '∞'
  const signed = (n) => `${n >= 0 ? '+' : ''}${n}`
  console.log(
    `  · 累计补件 ${supplements} / 迁移 ${migrations} = ${ratioText}:1（目标 ≤1:1）` +
      `；自本判据起 补件 ${signed(reading.dSupp)} / 迁移 ${signed(reading.dMig)}（欠账 ${signed(reading.debt)}）`,
  )

  // ── 绝对欠账的期限线（D-375）：增量线只拦新债，旧债按台阶还清 ──
  const absoluteDebt = supplements - migrations
  const allowance = debtAllowance({ today: todayIso(), initialDebt })
  const nextText = allowance.next
    ? `下一台阶 ${allowance.next.by} ≤${allowance.next.max}` +
      `（至少再抵 ${Math.max(0, absoluteDebt - allowance.next.max)} 笔）`
    : `已过最后检查点，${allowance.deadline} 之后必须为 0`
  console.log(
    `  · 累计欠账 ${absoluteDebt} 笔（补件 ${supplements} − 迁移 ${migrations}）；` +
      `今日（${allowance.today}）允许 ≤${allowance.allowed}，${nextText}`,
  )
  // 欠账明细（D-403）：按提交顺序把最早的补件配给迁移，剩下未被抵掉的补件就是欠账——
  // 豁免必须能逐笔复核，不能只印一个总数。
  const supplementsSeen = ledger.filter((e) => e.kind === 'supplement')
  const migrationsSeen = ledger.filter((e) => e.kind === 'migration').length
  const unpaid = supplementsSeen.slice(Math.min(migrationsSeen, supplementsSeen.length))
  if (absoluteDebt > 0) {
    console.log(`  · 欠账明细（${unpaid.length} 笔补件尚未被迁移抵掉，按提交顺序）：`)
    for (const e of unpaid) console.log(dim(`    - ${e.sha} ${e.subject}`))
  }
  const exemption = activeDebtExemption({ today: allowance.today, decisions })
  if (absoluteDebt > allowance.allowed) {
    if (exemption) {
      console.log(
        yellow(
          `  ⊘ EX-007 到期豁免：欠账 ${absoluteDebt} > 今日允许 ${allowance.allowed}，` +
            `按 ${exemption.decision} 暂不判红，至 ${exemption.through}（到期仍欠即红，续期须另裁决策）` +
            `；理由：${exemption.reason}`,
        ),
      )
    } else {
      console.error(
        red(
          `✗ EX-007 累计欠账 ${absoluteDebt} 笔 > 今日（${allowance.today}）允许 ≤${allowance.allowed}：` +
            '欠账按台阶递减，每个检查点至少抵 1 笔迁移；' +
            `${allowance.deadline} 必须归零，到期仍欠即红（D-375）`,
        ),
      )
      return 1
    }
  }
  if (absoluteDebt <= 0) console.log(green('  ✓ 累计欠账已归零（补件 ≤ 迁移）'))

  if (reading.debt > 0) {
    if (exemption) {
      console.log(
        yellow(
          `  ⊘ EX-007 新增欠账 +${reading.debt} 同由 ${exemption.decision} 豁免至 ${exemption.through}` +
            '（到期仍欠即红，续期须另裁决策）',
        ),
      )
    } else {
      console.error(
        red(
          `✗ EX-007 累计补件/迁移比恶化：自本判据起新增补件 ${reading.dSupp} 个、迁移 ${reading.dMig} 个` +
            `（欠账 +${reading.debt}）—— 补件必须与迁移同批或由既有迁移抵账；` +
            '只补不搬等于每个能力都在拓宽机制面（D-372）',
        ),
      )
      return 1
    }
  }
  if (reading.debt < 0) console.log(green(`  ✓ 已提前 ${-reading.debt} 笔迁移抵账`))
  if (reading.ratio <= 1) console.log(green('  ✓ 累计补件/迁移比已达 ≤1:1'))
  if (migrations === 0) {
    console.log(
      dim('  ⊘ 还没有迁移批次（S6 起生效）：本轮的读数只是「补件夹具成立」，不证明任何一批拆得动'),
    )
  }
  // 补件次数是**设计缺口的读数**（D-269 §5.3）：不设棘轮（范围一变计数就变，基线会假红），
  // 但它必须刺眼——补件与迁移批次同数，说明每一批都在补架构，而不是在搬能力。
  if (supplements >= SUPPLEMENT_ALARM) {
    console.log(
      yellow(
        `  △ 本段补件 ${supplements} 次 / 迁移批次 ${migrations} 个：补件次数是**设计缺口**的读数` +
          '（D-269 §5.3）。每一条先过 §5.1 的两条准入（不提能力名 ∧ ≥2 条能力要用），' +
          '过不了的记进 §5.2 缺口账、结论写「留控制面」，不改架构。',
      ),
    )
  }
  return 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(main())
}
