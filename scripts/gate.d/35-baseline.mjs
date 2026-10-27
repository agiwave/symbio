// 棘轮元守卫：**每一处「只许往紧改」的基线都不得被放松**（`RATCHETS` 清单驱动）。
//
// ## 为什么单独立一个阶段
//
// 棘轮基线是源码常量，而它就是「有测试被删就红」这条判据本身。把它改小不需要动测试、
// 不需要动守卫代码，门禁照样全绿且不留痕迹——与「测试消失」同形。所以守它的判据必须
// **读基准版本**，而不是读同一份源码。
//
// 基准取「最近触碰过该文件的提交及其父」：只比最近一版会漏掉「已经提交的那次下调」，
// 比到分支根则会被历史里那些**本来就低**的值天天判红——那只会逼人写出空理由的豁免。
// `--base=<ref>` 是唯一覆盖入口（CI 上要指 PR base 就用它）；一个基准都取不到 ⇒ 诚实跳过。
//
// ## 为什么要一张清单而不是一个写死的路径（批 M12）
//
// 此前 `SHARED` 是常量，只守 `_shared.mjs` 的 `BASELINE` 一个对象。同类棘轮另有三条
// 全在守卫之外，其中两条还是 `process.env` 读的——**可逐次调用覆盖**，于是
// 「默认值钉得死、CI 传个更松的值就绕过」是件不需要改任何文件的事。
//
// 清单每项 = 落点文件 + 提取函数 + 方向 + 豁免标记名。加一条落点不必重新决定一次
// 「什么算削判据」——那一条由 `_shared.ratchetErosion` 定义一次（与 M2 把
// `ratchetVerdict` 收进单点同理由）。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow } from '../color.mjs'
import {
  baselineWaivers,
  parseBaselineCells,
  ratchetErosion,
} from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')

const git = (ctx, args, label) => ctx.run({ label, cmd: 'git', args, cwd: repoRoot, echo: 'none' })

// ── 提取函数：各落点的形状不同，判据的**读法**也必须不同 ─────────

/** `scripts/core-export-audit.mjs` 的 `BASELINE`：默认值写在三元里，形如 `{ zero: 0, one: 61 }`。
 *  **只取默认分支**（`: { … }` 那一侧）——env 覆盖值不进基线判定，它由 `envLoosened` 单判。 */
function coreExportCells(text) {
  const cells = new Map()
  const m = /:\s*\{\s*zero:\s*(\d+),\s*one:\s*(\d+)\s*\}/.exec(text.replace(/\r\n/g, '\n'))
  if (m) {
    cells.set('zero', Number(m[1]))
    cells.set('one', Number(m[2]))
  }
  return cells
}

/** `scripts/core-shape-audit.mjs` 的 `SHAPE_BASELINE`：同样是三元尾部的对象字面量，三格。
 *  键名与键序就是 `CORE_SHAPE_BASELINE=<traits>:<methods>:<defaults>` 的位置口径，
 *  也必须是审计侧读的那三格——任何一边单改，这里就会读不出格（⇒ 判红，不是判绿）。 */
function coreShapeCells(text) {
  const cells = new Map()
  const m = /:\s*\{\s*traits:\s*(\d+),\s*methods:\s*(\d+),\s*defaults:\s*(\d+)\s*\}/.exec(
    text.replace(/\r\n/g, '\n'),
  )
  if (m) {
    cells.set('traits', Number(m[1]))
    cells.set('methods', Number(m[2]))
    cells.set('defaults', Number(m[3]))
  }
  return cells
}

/** `scripts/core-shape-audit.mjs` 的 `VOCAB_BASELINE`：同样是三元尾部的对象字面量，两格。
 *  键序就是 `VOCAB_BASELINE=<kernelEvents>:<cellsRows>` 的位置口径。 */
function vocabCells(text) {
  const cells = new Map()
  const m = /:\s*\{\s*kernelEvents:\s*(\d+),\s*cellsRows:\s*(\d+)\s*\}/.exec(
    text.replace(/\r\n/g, '\n'),
  )
  if (m) {
    cells.set('kernelEvents', Number(m[1]))
    cells.set('cellsRows', Number(m[2]))
  }
  return cells
}

/** `scripts/kind-branch-audit.mjs` 的 `BRANCH_BASELINE`：三元尾部对象，两格。
 *  `framework` 是「框架层自己认形态」的存量（目标 0），`registry` 是机制层判据与映射表；
 *  两格都只许降。键序就是 `KIND_BRANCH_BASELINE=<framework>:<registry>` 的位置口径。 */
function kindBranchCells(text) {
  const cells = new Map()
  const m = /:\s*\{\s*framework:\s*(\d+),\s*registry:\s*(\d+)\s*\}/.exec(text.replace(/\r\n/g, '\n'))
  if (m) {
    cells.set('framework', Number(m[1]))
    cells.set('registry', Number(m[2]))
  }
  return cells
}

/** `scripts/spec-coverage-audit.mjs` 的 `BASELINE`：两格债务（人工判据存量 / 无执行方）。 */
function specCoverageCells(text) {
  const cells = new Map()
  const m = /:\s*\{\s*manual:\s*(\d+),\s*missing:\s*(\d+)\s*\}/.exec(text.replace(/\r\n/g, '\n'))
  if (m) {
    cells.set('manual', Number(m[1]))
    cells.set('missing', Number(m[2]))
  }
  return cells
}

/** `scripts/injection-matrix-audit.mjs` 的 `BASELINE`：未归属格 / 已登记缺口格。 */
function injectionMatrixCells(text) {
  const cells = new Map()
  const m = /:\s*\{\s*unassigned:\s*(\d+),\s*deferred:\s*(\d+)\s*\}/.exec(text.replace(/\r\n/g, '\n'))
  if (m) {
    cells.set('unassigned', Number(m[1]))
    cells.set('deferred', Number(m[2]))
  }
  return cells
}

/** `scripts/dead-code-audit.mjs` 的 `R002_BASELINE`：默认值是三元尾部的那个字面量。
 *  注意三元问号那一侧是 `? Number(…)` 而**不是** `: Number(…)`——写成后者会永远取不到
 *  格，于是这一项在日志里显示成「跳过」，看上去无害，实则这条棘轮**已经没人守了**。 */
function deadCodeCells(text) {
  const cells = new Map()
  const norm = text.replace(/\r\n/g, '\n')
  const m = /\?\s*Number\(process\.env\.DEAD_CODE_R002_BASELINE\)\s*\n\s*:\s*(\d+)/.exec(norm)
  if (m) cells.set('R-002', Number(m[1]))
  return cells
}

/** `scripts/core-export-audit.mjs` 的 `DOMAIN_LEDGER_BASELINE`（C-004）：与 R-002 同构的
 *  三元形状，一格——**非内核根账本**条目数上限（只许降）。C-003 收窄到内核公共面之后，
 *  其余根出口只剩这一格冻结它们的增长；放宽它就是让「加共享类型就得改架构」回来。 */
function domainLedgerCells(text) {
  const cells = new Map()
  const norm = text.replace(/\r\n/g, '\n')
  const m = /\?\s*Number\(process\.env\.CORE_LEDGER_BASELINE\)\s*\n\s*:\s*(\d+)/.exec(norm)
  if (m) cells.set('C-004', Number(m[1]))
  return cells
}

/** `scripts/core-export-audit.mjs` 的 `DOMAIN_ONE_BASELINE`（C-004 的下沉候选格）：
 *  与 R-002 同构的三元形状，一格——域账本里单消费方条目的上限（只许降）。 */
function domainLedgerOneCells(text) {
  const cells = new Map()
  const norm = text.replace(/\r\n/g, '\n')
  const m = /\?\s*Number\(process\.env\.CORE_LEDGER_ONE_BASELINE\)\s*\n\s*:\s*(\d+)/.exec(norm)
  if (m) cells.set('C-004-one', Number(m[1]))
  return cells
}

/** `scripts/grep-audit.mjs` 的 `LEGACY_ROLE_WRITE_BASELINE`（S-014）：与 R-002 同构的
 *  三元形状，一格——ChatNode 存量 `role` 格直写的总数上限（只许降）。 */
function legacyRoleCells(text) {
  const cells = new Map()
  const norm = text.replace(/\r\n/g, '\n')
  const m = /\?\s*Number\(process\.env\.LEGACY_ROLE_WRITE_BASELINE\)\s*\n\s*:\s*(\d+)/.exec(norm)
  if (m) cells.set('S-014', Number(m[1]))
  return cells
}

/** `scripts/obligation-bypass-audit.mjs` 的 `BYPASS_BASELINE`（OB-001）：同构的三元形状，
 *  一格——**调度旁路**（未承认的直调 `owes()`）点数上限（只许降）。它是「收口完成 = 调度
 *  旁路归零」这条定义的**唯一持有者**：设计文档正文只写口径，数字住这里，所以放宽它就
 *  等于放宽那个定义。 */
function obligationBypassCells(text) {
  const cells = new Map()
  const norm = text.replace(/\r\n/g, '\n')
  const m = /\?\s*Number\(process\.env\.OBLIGATION_BYPASS_BASELINE\)\s*\n\s*:\s*(\d+)/.exec(norm)
  if (m) cells.set('OB-001', Number(m[1]))
  return cells
}

/** 同一脚本的 `PRECHECK_BASELINE`（OB-004）：**幂等点问**（执行体动手前的复核）上限。
 *  它不是归零对象，但也不许随手加——每加一处就等于开一条不用过外环收编的逃生通道，
 *  所以它单独一格、单独一条 env、单独一个豁免标记。 */
function obligationPrecheckCells(text) {
  const cells = new Map()
  const norm = text.replace(/\r\n/g, '\n')
  const m = /\?\s*Number\(process\.env\.OBLIGATION_PRECHECK_BASELINE\)\s*\n\s*:\s*(\d+)/.exec(norm)
  if (m) cells.set('OB-004', Number(m[1]))
  return cells
}

/** `tauri/vitest.config.ts` 的 `coverage.thresholds`：一个嵌套块，四条全局 + 一条分目录。
 *  **只扫 `thresholds: {` 块**——同一文件里别处也有 `lines:` 这类键。 */
function coverageCells(text) {
  const cells = new Map()
  const norm = text.replace(/\r\n/g, '\n')
  const start = norm.indexOf('thresholds: {')
  if (start < 0) return cells
  const body = norm.slice(start)
  const end = body.indexOf('\n      }')
  const lines = (end < 0 ? body : body.slice(0, end)).split('\n')
  for (const line of lines) {
    // 分目录那条：`'src/registry/**': { lines: 87, statements: 88 }`
    const scoped = /^\s*'([^']+)':\s*\{([^}]*)\}/.exec(line)
    if (scoped) {
      for (const m of scoped[2].matchAll(/(lines|statements|functions|branches):\s*(\d+)/g)) {
        cells.set(`${scoped[1]}.${m[1]}`, Number(m[2]))
      }
      continue
    }
    const m = /^\s*(lines|statements|functions|branches):\s*(\d+),/.exec(line)
    if (m) cells.set(m[1], Number(m[2]))
  }
  return cells
}

/** 豁免注释：沿用各守卫自己的标记名（`baseline-allow` / `core-export-allow` /
 *  `dead-code-allow`），不新造第四种拼写——那会让豁免散在四种语法里。 */
function waiverReader(tag) {
  const re = new RegExp(`^ {0,2}//\\s*${tag}\\s+([A-Za-z0-9_:/-]+):\\s*(\\S[^\\n]*)$`, 'gm')
  return (text) => {
    const out = new Map()
    for (const m of text.replace(/\r\n/g, '\n').matchAll(re)) out.set(m[1], m[2].trim())
    return out
  }
}

// ── 棘轮清单 ──────────────────────────────────────────────────────────
// `dir`：`floor` = 只增（下限），`ceiling` = 只减（上限）。
// `envVar`：该落点的值可经环境变量覆盖 ⇒ 额外判「覆盖值不得比默认值更松」。
//   **这一条不是多余的**：env 覆盖不动任何文件，不留痕迹，而它是 CI 那一跑的
//   实际判据——默认值钉得再死，一行 `--env` 就能绕过去。
const RATCHETS = [
  {
    id: 'baseline',
    file: 'scripts/gate.d/_shared.mjs',
    dir: 'floor',
    cellsOf: parseBaselineCells,
    waivers: waiverReader('baseline-allow'),
    label: 'BASELINE（Rust / CI / 前端用例数、verify 程序数与断言数、e2e 用例数）',
  },
  {
    id: 'core-export',
    file: 'scripts/core-export-audit.mjs',
    dir: 'ceiling',
    cellsOf: coreExportCells,
    waivers: waiverReader('core-export-allow'),
    envVar: 'CORE_EXPORT_BASELINE',
    label: 'core 根出口的 0 / 单消费方符号存量（只许降）',
  },
  {
    // C-004：C-003 收窄到内核公共面后，其余根出口不再被判「配不配占出口」；
    // 它们的存量由这一格冻结（只许降）。放宽 = 让「新增共享类型仍改 core 根出口」
    // 悄悄回来，所以与 core-export 同一条纪律：必须改一个提交进仓库的数字。
    id: 'core-ledger',
    file: 'scripts/core-export-audit.mjs',
    dir: 'ceiling',
    cellsOf: domainLedgerCells,
    waivers: waiverReader('core-export-allow'),
    envVar: 'CORE_LEDGER_BASELINE',
    label: '非内核根账本条目数（C-004，只许降）',
  },
  {
    // C-004 的下沉候选格：域账本里只有一个消费方的条目。条目总数只说明账本多长，
    // 这一格说明其中有多少「本该下沉」；涨 = 又多了一个只为单点需要而占根出口的共享符号。
    id: 'core-ledger-one',
    file: 'scripts/core-export-audit.mjs',
    dir: 'ceiling',
    cellsOf: domainLedgerOneCells,
    waivers: waiverReader('core-export-allow'),
    envVar: 'CORE_LEDGER_ONE_BASELINE',
    label: '域账本单消费方条目数（C-004 下沉候选，只许降）',
  },
  {
    // 形状三格（`pub trait` 数 / `fn` 成员总数 / 带默认体的成员数）。放宽的**资格**由
    // `core-shape-audit` 的 CS-002 判（注释里必须有能打开的 ADR），本条只判「数字有没有比
    // 上一版更松」——两半合起来才是「放宽 = 写一份决策」。
    id: 'core-shape',
    file: 'scripts/core-shape-audit.mjs',
    dir: 'ceiling',
    cellsOf: coreShapeCells,
    waivers: waiverReader('core-shape-allow'),
    envVar: 'CORE_SHAPE_BASELINE',
    label: 'core 公共形状三格（trait 数 / 成员数 / 默认体数，只许降）',
  },
  {
    // CS-005：内核业务事件常量 / CELLS 业务行。声明化之后这两格只许降，而「凭什么还
    // 留着」由 CS-006 逐名判据（DE-024 D-202）；放宽 = 让「新事件仍要改内核手写表」
    // 悄悄回来，所以必须改一个提交进仓库的数字。
    id: 'vocab',
    file: 'scripts/core-shape-audit.mjs',
    dir: 'ceiling',
    cellsOf: vocabCells,
    // 不设豁免标记：收窄自动生效，变宽只能改基线，而改基线会被本条按基准版本抓住。
    waivers: () => new Map(),
    envVar: 'VOCAB_BASELINE',
    label: '内核事件词汇（业务事件常量 / CELLS 业务行，只许降）',
  },
  {
    // 前端 kind 分支两格：框架层目标 0（只许降），机制层是判据与映射表的存量。
    // 放宽的**资格**由 `kind-branch-audit` 的 KB-001 判（超过基线即红），本条只判
    // 「数字有没有比上一版更松」——两半合起来才是「框架真的零分支」。
    id: 'kind-branch',
    file: 'scripts/kind-branch-audit.mjs',
    dir: 'ceiling',
    cellsOf: kindBranchCells,
    waivers: waiverReader('kind-branch-allow'),
    envVar: 'KIND_BRANCH_BASELINE',
    label: '前端 kind 分支（框架层 / 机制层，只许降）',
  },
  {
    // 规范执行方覆盖：`manual` 是「无机器判据」的合法存量，`missing` 是违规存量（目标 0）。
    id: 'spec-coverage',
    file: 'scripts/spec-coverage-audit.mjs',
    dir: 'ceiling',
    cellsOf: specCoverageCells,
    waivers: waiverReader('spec-coverage-allow'),
    envVar: 'SPEC_COVERAGE_BASELINE',
    label: '规范执行方覆盖（人工判据存量 / 无执行方，只许降）',
  },
  {
    // 异常注入矩阵：`unassigned` 是没归属的格（目标 0），`deferred` 是已登记缺口的格。
    // 缺口只能被补成用例而减少；把 `deferred` 改大等于把「暂不测」变成新常态。
    id: 'injection-matrix',
    file: 'scripts/injection-matrix-audit.mjs',
    dir: 'ceiling',
    cellsOf: injectionMatrixCells,
    waivers: waiverReader('injection-allow'),
    envVar: 'INJECTION_MATRIX_BASELINE',
    label: '异常注入矩阵（未归属格 / 已登记缺口，只许降）',
  },
  {
    id: 'dead-code',
    file: 'scripts/dead-code-audit.mjs',
    dir: 'ceiling',
    cellsOf: deadCodeCells,
    waivers: waiverReader('dead-code-allow'),
    envVar: 'DEAD_CODE_R002_BASELINE',
    label: 'R-002 `#[allow(dead_code)]` 存量（只许降）',
  },
  {
    // d-iii：`role` 退出 wire 后每一处直写都是待清零的债务。字段私有化
    // 会炸掉全部 `..Default::default()` 构造点（函数式更新要求字段可见），
    // 所以钉数字而不是钉编译。
    id: 'legacy-role',
    file: 'scripts/grep-audit.mjs',
    dir: 'ceiling',
    cellsOf: legacyRoleCells,
    waivers: waiverReader('legacy-role-allow'),
    envVar: 'LEGACY_ROLE_WRITE_BASELINE',
    label: 'S-014 ChatNode 存量 role 格直写（只许降）',
  },
  {
    // 旁路直调 `owes()` 的点数。DE-015 的 D-158 把「收口完成」定义成**这个数归零**，
    // 而定义要成立，那个数就不能被悄悄改大——「改一个数字」看起来像维护，实际是在
    // 取消一条已经写进设计的定义。`D-158` 之后这里只数**调度旁路**：动手前复核另立一格。
    id: 'obligation-bypass',
    file: 'scripts/obligation-bypass-audit.mjs',
    dir: 'ceiling',
    cellsOf: obligationBypassCells,
    waivers: waiverReader('obligation-bypass-allow'),
    envVar: 'OBLIGATION_BYPASS_BASELINE',
    label: '调度旁路（未承认的直调 `owes()`）点数（只许降，归零 = 派发面收口完成）',
  },
  {
    // 幂等点问（OB-004 承认在册的直问）的存量。它不归零，但也不许随手加：每一处都要
    // 就地理由 + 点名的行确实接了执行体，而那件事由 `obligation-bypass-audit` 自己判；
    // 这一格只管「在册的护栏有没有变多」。
    id: 'obligation-precheck',
    file: 'scripts/obligation-bypass-audit.mjs',
    dir: 'ceiling',
    cellsOf: obligationPrecheckCells,
    waivers: waiverReader('obligation-precheck-allow'),
    envVar: 'OBLIGATION_PRECHECK_BASELINE',
    label: '幂等点问（执行体动手前复核）存量（只许降，但不以归零为目标）',
  },
  {
    id: 'coverage',
    file: 'tauri/vitest.config.ts',
    dir: 'floor',
    cellsOf: coverageCells,
    waivers: waiverReader('coverage-allow'),
    label: '前端覆盖率阈值（只许升）',
  },
]

/** 基准版本：`--base=<ref>` 优先；否则取最近触碰过**该落点文件**的提交及其父 */
async function baseVersions(ctx, ratchet) {
  const explicit = process.argv.map((a) => /^--base=(.+)$/.exec(a)).find(Boolean)
  const refs = []
  if (explicit) {
    refs.push(explicit[1])
  } else {
    const last = await git(
      ctx,
      ['log', '-1', '--format=%H', 'HEAD', '--', ratchet.file],
      `git log：最近触碰 ${ratchet.file} 的提交`,
    )
    const sha = last.ok ? last.output.trim().split('\n')[0] : ''
    if (sha) refs.push(sha, `${sha}~1`)
  }
  const out = []
  for (const ref of refs) {
    // 显示口径与判据口径一致：`<sha>~1` 若按 `slice(0,8)` 打印会和子提交同名，
    // 日志里就成了「和 f9bdcbf9 / f9bdcbf9 比对」——看不出确实读了两版。
    const alias = ref.endsWith('~1') ? `${ref.slice(0, 8)}^` : ref.slice(0, 8)
    const r = await git(ctx, ['show', `${ref}:${ratchet.file}`], `git show ${alias}：${ratchet.file}`)
    if (r.ok && r.output && ratchet.cellsOf(r.output).size > 0) {
      out.push({ ref: alias, text: r.output })
    } else if (r.ok && r.output) {
      // 基准版本里连这处基线的 envVar 名字都没有 ⇒ 这一处棘轮是那一提交**新立的**，
      // `sha~1` 正落在引入点上：从来没有「更松的基准」可 relax。属判不了，不是判据失效
      // （判不了按「跳过」处理，见方法注释）。与「文件在、名字在、却一格都解析不出」
      // ——真形状脱节——区分开，后者照红。
      if (ratchet.envVar && !r.output.includes(ratchet.envVar)) continue
      // ⚠️ 基准**取到了**却一格都解析不出 ⇒ 提取函数与这处落点的形状脱节了。
      // 这不是「判不了」，是「判据已经失效」——而两种情况在日志里长得**一模一样**
      // （都显示「跳过」）。混为一谈的后果是：一条棘轮悄悄没人守，而门禁全绿。
      // 与下面 `sha` 为空（浅克隆 / 从未提交 ⇒ 真判不了）区分开，宁红不静默。
      out.push({ ref: alias, text: r.output, unreadable: true })
    }
  }
  return out
}

/**
 * env 覆盖值是否比默认值更松。
 *
 * 两条 env 棘轮的默认值是**提交进仓库的字面量**，而门禁那一跑读的是 `process.env`。
 * 只守默认值的话，`CORE_EXPORT_BASELINE=0:999` 这种覆盖不需要改任何文件就能把上限
 * 放到天边，且不留痕迹——比改默认值更隐蔽。所以覆盖值本身也要判。
 */
function envLoosened(ratchet, defaults) {
  if (!ratchet.envVar) return null
  const raw = process.env[ratchet.envVar]
  if (raw === undefined || raw === '') return null
  const keys = [...defaults.keys()]
  const nums = raw.split(':').map((n) => Number(n))
  if (nums.some((n) => Number.isNaN(n))) {
    return { error: `${ratchet.envVar}=${raw} 不是数字（或冒号分隔的数字）` }
  }
  // 覆盖值与默认值**按位置**对齐：`0:58` → 默认值的第一、二格；`20:72:24` 同理三格。
  // 早先这里写死 `[zero, one]` 两个键名，落点一多就露底：第三格的形状是「数得清格、
  // 但对不上键名」，报出来的红指向一个不存在的格——判据本身没错，错在读法只认一处形状。
  if (nums.length !== keys.length) {
    return {
      error:
        `${ratchet.envVar}=${raw} 给了 ${nums.length} 格，而默认值是 ${keys.length} 格` +
        `（${keys.join(':')}）—— 按位置给全，少一格等于把那格的判据删掉`,
    }
  }
  const override = new Map(keys.map((k, i) => [k, nums[i]]))
  const bad = []
  for (const [key, want] of override) {
    const have = defaults.get(key)
    if (have === undefined) {
      bad.push(`${key} 在默认值里没有这一格（env 覆盖=${want}）`)
      continue
    }
    const looser = ratchet.dir === 'ceiling' ? want > have : want < have
    if (looser) bad.push(`${key} env 覆盖 ${want} 比默认值 ${have} 更松`)
  }
  return bad.length ? { bad } : null
}

export default {
  id: 'baseline',
  title: '棘轮元守卫（每处基线只许往紧改）',
  // 阶段级并发：本阶段**只读**（git log/show + 读落点文件），批次里没有别的
  // `.git` 写方——批内的写只有 `target/` 系（backend / msrv 各写自己的产物目录）与
  // verify 的 `verify-target/`；重生成后的暂存只发生在 fmt（05，批前）、
  // facts-write（06，批前）与 facts（60，最后）。⚠️ `git log`/`git show` 读对象库，
  // 不读索引——即便有别的泳道在写工作区文件，本阶段的基准比对也不受影响。
  parallel: true,
  tasks(ctx) {
    return RATCHETS.map((ratchet) => ({
      label: `${ratchet.file}：${ratchet.label} 不得比基准版本放松`,
      skipNote: `没有可比基准（浅克隆、无历史，或基准版本里还没有这处基线）⇒ 本项判不了`,
      run: async () => {
        const bases = await baseVersions(ctx, ratchet)
        if (bases.length === 0) return 'skipped'

        // 「基准里读不出」有两种成因，把它们并成一判红会把第一种当成第二种：
        //  - **提取函数与落点形状脱节** ⇒ 判据真的失效，必须红（这正是原来那条注释要的）；
        //  - **这一格比基准新**：棘轮是在那条基准提交里引入的，它的父提交里根本没有这处
        //    基线（`git show` 取得到文件，但文件里还没有这格）。
        // 两者分得开：形状脱节时**每一条**基准都读不出，而「比基准新」时引入它的那条读得出、
        // 只有更早的那条读不出。于是前者红，后者跳过那一版继续比其余基准——不豁免、不静默。
        const readable = bases.filter((b) => !b.unreadable)
        if (readable.length === 0) {
          console.log(
            red(
              `      ↳ 基准 ${bases.map((b) => b.ref).join(' / ')} 里都读不出 ${ratchet.file} 的基线格` +
                ` ⇒ 提取函数与这处落点的形状脱节，本项判据已失效（不是「没基准可判」）`,
            ),
          )
          return { ok: false, note: `提取函数读不出 ${ratchet.file} 的基线格，判据失效` }
        }
        if (readable.length < bases.length) {
          console.log(
            yellow(
              `      ↳ 基准 ${bases.filter((b) => b.unreadable).map((b) => b.ref).join(' / ')} ` +
                `早于这处基线（棘轮在更晚的提交里才引入）⇒ 不与它们比`,
            ),
          )
        }

        const current = fs.readFileSync(path.join(repoRoot, ratchet.file), 'utf8')
        const cells = ratchet.cellsOf(current)
        // 一格都没取到 ⇒ 提取口径已随这处基线的形状一起失效。宁红不静默。
        if (cells.size === 0) {
          console.log(red(`      ↳ 当前 ${ratchet.file} 里取不到任何基线数值格 ⇒ 本项失去判据`))
          return { ok: false, note: `解析不到 ${ratchet.file} 的数值格（形状变了？），判据失效` }
        }

        const problems = []
        const erosion = ratchetErosion({ dir: ratchet.dir }, ratchet.cellsOf, current, readable, ratchet.waivers(current))
        for (const e of erosion) {
          const line = `      ↳ ${e.key}: ${e.from} → ${e.to ?? '整格被删'}（基准 ${e.source}）${e.waived ? ' · 已登记豁免' : ''}`
          console.log(e.waived ? yellow(line) : red(line))
        }
        const unwaived = erosion.filter((e) => !e.waived)
        if (unwaived.length) {
          problems.push(
            `${unwaived.length} 格被放松：${unwaived
              .map((e) => `${e.key} ${e.from}→${e.to ?? '删格'}`)
              .join('、')}`,
          )
        }

        const env = envLoosened(ratchet, cells)
        if (env?.error) {
          console.log(red(`      ↳ ${env.error}`))
          problems.push(env.error)
        } else if (env?.bad) {
          for (const b of env.bad) console.log(red(`      ↳ ${b}`))
          problems.push(`${ratchet.envVar} 覆盖比默认值更松：${env.bad.join('；')}`)
        }

        if (problems.length) return { ok: false, note: problems.join('；') }
        console.log(
          green(
            `      ↳ ${cells.size} 格与 ${readable.map((b) => b.ref).join(' / ')} 逐格比对：无放松` +
              (ratchet.envVar ? `（${ratchet.envVar} 未被更松地覆盖）` : ''),
          ),
        )
        return { ok: true }
      },
    }))
  },
}