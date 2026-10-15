#!/usr/bin/env node
/**
 * core-shape-audit — `symbio_core` 的**公共形状**只许收窄（判定型）
 *
 * ## 它判什么
 *
 * 三格棘轮，全部来自 core 生产侧的 `pub trait`：
 *
 * | 编号 | 格 | 判什么 | 它拦的是哪一种膨胀 |
 * |---|---|---|---|
 * | CS-001 | `traits` | `pub trait` 声明数（冻结 20） | **新增接缝**：每多一个抽象，就多一处「所有实现方都得知道」的地方 |
 * | CS-001 | `methods` | 这些 trait 的 `fn` 成员总数（冻结 73） | **契约变宽**：既有接缝上挂新方法，影响面 = 全部实现方 |
 * | CS-001 | `defaults` | 其中**带默认体**的成员数（冻结 25） | **接口上长实现**：行为代码住进架构模块 |
 * | CS-002 | —— | 放宽上面任何一个数字，必须写 `core-shape-allow <格>: ADR-NNN <非空理由>`，且那个 ADR 文件在 `docs/010-adr/` **能打开** | **改一个数字**看起来像维护，而它实际是在改架构 |
 *
 * 表里的冻结值是**现值**，owner 是本文件的 `SHAPE_BASELINE`（下面那一行）；放宽一格
 * 必须按 CS-002 在它旁边留一条 `core-shape-allow`，并有一条能打开的 ADR 论证「必需」。
 *
 * ## 为什么需要它（2026-10-12 的实测缺口）
 *
 * 2026-10-12 那一批「`VdfsProvider` 去样板」的处方（把单点分发推广成默认实现）要把
 * `symbio_core::VdfsProvider` 从**一个方法**扩成**十个方法**。按当时全部门禁的口径算一遍：
 *
 * - `core-export-audit` C-001 / C-002：动的是那个符号**内部**，根出口一个字符没改；
 * - C-003：`VdfsProvider` 早就是多消费方导出，它数的是**符号表**（Map 的键），
 *   方法数不在其视野内——符号 +0，形状 ×10；
 * - `dead-code` / `core-naming` / `no-direct-call` / `mechanism-audit`：判据里都没有
 *   「一个 trait 有几个成员」这一项。
 *
 * ⇒ 一次把核心契约面扩到十倍、并在接口上多挂 9 个默认体的改动，**门禁全绿也能提交**。
 * 而这条纪律本仓早就写了：`08 §0 纪律 3`「架构改动必须逐条论证『必需』」，登记条目
 * [SP-007](../docs/008-spec/007.md)——它的判据写的是「论证不出即删（**人工核对**）」。
 * 于是「架构改动」的全部约束，取决于执行的人**记得**去核对；而批次执行时最不缺的就是
 * 合理化：「加 9 个方法是实现层的样板收敛，不算架构改动」。
 *
 * **本脚本把这条纪律从「记得去看」变成「改了就红」。** 它不判某个 trait 该不该宽——那是
 * 论证的事；它只保证：让核心形状变宽，必须显式触碰一处基线数字，并留下一条能被人打开的
 * 决策。红不红不由执行者判断。
 *
 * ## 为什么只有这三格（口径边界，别顺手加）
 *
 * - **不判行数**：`08 §3.3` 写着「加了数据 / 加了调用点 / 改了文档，都不是架构改动」，
 *   而契约加字段是数据变化——行棘轮会对它红，那是误报；一个只会误报的守卫最后一定被人
 *   喂豁免喂到失效（本仓为此写过一整节，见 `core-export-audit.mjs` 的 W1 注）。
 * - **不判枚举变体数**：变体**是**形状量（`08 §3.5` 把「`MessageType` 新取值」列为不新增），
 *   但它与「trait 宽度」不是同一个失效面。要加就单独论证一次，不在本批顺手扩判据面。
 * - **不判 `impl` 数 / 宏**：宏展开的形状静态读不到（`08 §3` 的判据本来也不看这个）。
 *
 * ## 与既有判据的分工（一个事实只有一个 owner）
 *
 * | 谁 | 判什么 |
 * |---|---|
 * | 本脚本 | core 公共**形状**的量（三格）+ 放宽痕迹必须挂得住一条决策 |
 * | `core-export-audit`（C-001…C-003） | 根出口的**归属**：什么能出现在公共面上、谁在用 |
 * | `gate.d/35-baseline` | 基线数字本身**不得比上一版更松**（比对 git 基准版本）；本脚本的三格登记在它的 `RATCHETS` 清单里，`CORE_SHAPE_BASELINE` 的环境变量覆盖由它兜 |
 * | [SP-007](../docs/008-spec/007.md) | 纪律**条文**本身（owner 在 `08 §3`），本脚本是它的判据执行方 |
 *
 * CS-002 与 `35-baseline` 各判半条：那边只发现「数字变松了」（红不红取决于有没有豁免注释），
 * 这边判「这条豁免**够不够格**」。两边合起来才是完整的「放宽 = 写决策」。
 *
 * ## 已知边界（写清楚，免得把绿灯当成证明）
 *
 * 判定基于**去注释、剔测试模块后的文本**，不是 AST：
 * - `pub trait` 必须写成 `pub trait <名>`（`pub unsafe trait` 读不到）；
 * - 宏生成的 trait / 成员读不到（本仓的 `define_string_key!` 生成的是 impl，不是 trait）；
 * - 成员签名扫描**不跟踪尖括号**（`->` 里那个 `>` 会把括号计数拖成负数）——代价是
 *   「泛型参数里带 `{`」这种罕见形状会被看成带默认体，**只会多数不会少数**，棘轮方向上安全；
 * - 它挡的是「顺手改宽」，不挡「刻意放宽」——后者正是 CS-002 要求留决策的原因。
 *
 * ## 用法
 *   node scripts/core-shape-audit.mjs              # 判定
 *   node scripts/core-shape-audit.mjs --verbose    # 逐 trait 形状（成员数 / 其中默认体）
 *   node scripts/core-shape-audit.mjs --root=<dir> # 指向别处的仓库（测试用）
 * 退出码：0 = 通过，1 = 违规。
 *
 * 环境变量 `CORE_SHAPE_BASELINE=<traits>:<methods>:<defaults>` **只给回归测试用**（真仓门禁
 * 不设）；它不动任何文件就能把上限放到天边，所以 `gate.d/35-baseline.mjs` 另判一次
 * 「覆盖值不得比默认值更松」。
 */

// 判据码命名空间（登记表 docs/006-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns CS core 形状
// @codes CS-001 CS-002

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'
import { isTestFile, walk } from './core-surface.mjs'
import { blankComments, blankTestModules, matchBrace } from './rust-scan.mjs'

const argv = process.argv.slice(2)
const VERBOSE = argv.includes('--verbose')
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const CORE_REL = 'symbio/src/symbio_core'
/** 本脚本自己：CS-002 读它源码里的放宽痕迹 */
const SELF_REL = 'scripts/core-shape-audit.mjs'

/**
 * 形状基线（2026-10-12 冻结，决策与理由见 [ADR-055](../docs/010-adr/055.md)）。
 * 三格都是**上限**，只许降。数字本身不是判据，**改数字才是**：任何让形状变宽的改动都会
 * 撞红，而把它继续推下去只有一条路——改这一行的数字，那又会要求 CS-002 的一条
 * `core-shape-allow <格>: ADR-NNN <理由>`，且那个 ADR 文件必须真实存在。
 * 于是「顺手把架构改宽一点」在机制上等于「写一份决策」，而不是「改一个数字」。
 *
 * ⚠️ 键序就是 `CORE_SHAPE_BASELINE` 的位置口径（`35-baseline` 的 env 覆盖判定按它 zip），
 * 也必须是 `35-baseline` 那条提取函数认的键名；任何一边单改都会红。
 */
// core-shape-allow methods: ADR-062 —— WalStore 唯一写入口对事件类型是泛型的（Store::Event: EventEnvelope），泛型触达信封自身 ts 的唯一通道就是 trait 成员（与 assign_seq 同纪律），下沉回十余处构造点即 ADR-062 明确否决的方案
// core-shape-allow defaults: ADR-062 —— 无时刻概念的信封须「保持原样」，默认 no-op 体就是它的平凡值（README §3 的 J2）；去掉默认体反而让每个实现方手写同一个空函数，公共面更宽
const SHAPE_BASELINE = process.env.CORE_SHAPE_BASELINE
  ? (() => {
      const [traits, methods, defaults] = process.env.CORE_SHAPE_BASELINE.split(':').map((n) => Number(n))
      return { traits, methods, defaults }
    })()
  : { traits: 20, methods: 73, defaults: 25 }

const CELL_LABEL = {
  traits: '`pub trait` 数（接缝数）',
  methods: '`fn` 成员总数（契约面宽度）',
  defaults: '带默认体的成员数（接口上的实现）',
}

/** 放宽痕迹的读法：与 `35-baseline` 的 `waiverReader('core-shape-allow')` 同一条拼写约定 */
const WAIVER_RE = /^ {0,2}\/\/\s*core-shape-allow\s+([A-Za-z0-9_]+)\s*:\s*(\S[^\n]*)$/gm
const ADR_REF_RE = /ADR-(\d+[a-z]?)/g

const errors = []
const report = (rule, msg) => errors.push({ rule, msg })

// ==================== 读形状 ====================

/**
 * 数一个 trait 体的形状（体已剥掉外层花括号）。
 *
 * 成员 = 深度 0 上的 `fn`；「带默认体」= 该 `fn` 的签名以 `{` 收尾而不是 `;`。命中默认体后
 * 用 `matchBrace` 跳过整个体，否则体内的 `fn`（闭包 / 局部函数）会被当成成员。
 */
function traitShape(body) {
  let fns = 0
  let defaults = 0
  let depth = 0
  for (let i = 0; i < body.length; i++) {
    const c = body[i]
    if (c === '{' || c === '(' || c === '[') {
      depth++
      continue
    }
    if (c === '}' || c === ')' || c === ']') {
      depth--
      continue
    }
    if (depth !== 0 || c !== 'f' || !body.startsWith('fn', i)) continue
    if (!/[^A-Za-z0-9_]/.test(body[i - 1] ?? ' ')) continue
    const m = /^fn\s+([A-Za-z0-9_]+)/.exec(body.slice(i))
    if (!m) continue
    fns++
    let d = 0
    let open = -1
    for (let j = i + 2; j < body.length; j++) {
      const k = body[j]
      if (k === '(' || k === '[') d++
      else if (k === ')' || k === ']') d--
      else if (d === 0 && k === '{') {
        open = j
        break
      } else if (d === 0 && k === ';') break
    }
    if (open >= 0) {
      defaults++
      i = matchBrace(body, open)
    }
  }
  return { fns, defaults }
}

/**
 * core 生产侧每个 `pub trait` 的形状。
 *
 * 测试文件与 `#[cfg(test)]` 模块都不算——口径与 `core-export-audit` C-003 一致：
 * **测试不是架构消费方**，测试里的 trait 形状也不是长期接口。
 */
function measureCoreShape(coreDir) {
  const rows = []
  for (const file of walk(coreDir)) {
    if (!file.endsWith('.rs') || isTestFile(file)) continue
    const txt = blankTestModules(blankComments(fs.readFileSync(file, 'utf8')).join('\n'))
    for (const m of txt.matchAll(/\bpub\s+trait\s+([A-Za-z0-9_]+)/g)) {
      const open = txt.indexOf('{', m.index)
      if (open < 0) continue
      const body = txt.slice(open + 1, matchBrace(txt, open))
      rows.push({ name: m[1], file: path.relative(ROOT, file).split(path.sep).join('/'), ...traitShape(body) })
    }
  }
  rows.sort((a, b) => b.fns - a.fns || a.name.localeCompare(b.name))
  return {
    rows,
    values: {
      traits: rows.length,
      methods: rows.reduce((s, r) => s + r.fns, 0),
      defaults: rows.reduce((s, r) => s + r.defaults, 0),
    },
  }
}

const coreDir = path.join(ROOT, CORE_REL)
if (!fs.existsSync(path.join(coreDir, 'mod.rs'))) {
  console.error(red(`✗ 读不出 core 范围：${CORE_REL}/mod.rs 不存在（root=${ROOT}）—— 判据没有输入，宁红不静默`))
  process.exit(1)
}
const shape = measureCoreShape(coreDir)

// ==================== CS-001 形状棘轮 ====================

for (const [key, cap] of Object.entries(SHAPE_BASELINE)) {
  if (!Object.hasOwn(shape.values, key)) {
    report('CS-001', `基线里有一格 \`${key}\` 不是形状量（只有 ${Object.keys(shape.values).join(' / ')}）`)
    continue
  }
  if (!Number.isFinite(cap)) {
    report('CS-001', `基线格 \`${key}\` 不是数字（${cap}）—— 判据读不出即失效，宁红不静默`)
    continue
  }
  const now = shape.values[key]
  if (now > cap) {
    report(
      'CS-001',
      `核心形状变宽：${CELL_LABEL[key]} 实测 ${now} > 基线 ${cap} —— symbio_core 的公共形状只许收窄。` +
        `要让这一步成立，先在 \`docs/010-adr/\` 立一条 ADR 逐条论证「必需」（08 §0 纪律 3，登记条目 SP-007；` +
        `被这份棘轮钉住的先例见 ADR-055），再在基线旁写 core-shape-allow ${key}: ADR-NNN <理由>。逐 trait 清单：--verbose`,
    )
  }
}

// ==================== CS-002 放宽必须挂得住一条决策 ====================

/**
 * 读本脚本**自己**源码里那几行放宽痕迹。
 *
 * 为什么要读自己：棘轮的放宽口就是「把 SHAPE_BASELINE 的数字改大」这一行，而那一行的
 * 合法性（有没有挂得住的决策）只有它自己知道。`35-baseline` 比对的是基准版本的数字，
 * 它只知道「变松了、旁边有没有注释」；注释**够不够格**在这里判——两半合起来才是
 * 「放宽 = 写一份能打开的决策」。
 *
 * 夹具仓库里可能没有本脚本 ⇒ 读不到就跳过（与 `core-surface` 的 `walk` 同一约定：
 * 路径不存在不是违规）。
 */
const waivers = []
const selfPath = path.join(ROOT, SELF_REL)
if (fs.existsSync(selfPath)) {
  const selfText = fs.readFileSync(selfPath, 'utf8').replace(/\r\n/g, '\n')
  for (const m of selfText.matchAll(WAIVER_RE)) {
    const [, key, rawReason] = m
    const reason = rawReason.trim()
    waivers.push({ key, reason })
    if (!Object.hasOwn(SHAPE_BASELINE, key)) {
      report('CS-002', `\`core-shape-allow ${key}\` 放宽的不是形状基线的格（只有 ${Object.keys(SHAPE_BASELINE).join(' / ')}）`)
      continue
    }
    const refs = [...reason.matchAll(ADR_REF_RE)].map((r) => r[1])
    if (refs.length === 0) {
      report(
        'CS-002',
        `放宽 \`${key}\` 的理由里没有 ADR 编号（\`${key}: ${reason}\`）—— 动棘轮数字必须挂一条**决策**。` +
          `「批次需要 / 顺手 / 待补 / 见对话」都不算`,
      )
      continue
    }
    for (const id of refs) {
      if (!fs.existsSync(path.join(ROOT, 'docs', '010-adr', `${id}.md`))) {
        report(
          'CS-002',
          `放宽 \`${key}\` 引用了 \`ADR-${id}\`，但 \`docs/010-adr/${id}.md\` 打不开 —— 指向不存在的决策的豁免不算豁免`,
        )
      }
    }
  }
}

// ==================== 输出 ====================

const ruleLine = (ok, rule, desc) =>
  console.log(`${ok ? green('✓') : red('✗')} ${rule}  ${dim(desc)}`)

const cells = Object.keys(SHAPE_BASELINE)
const shapeLine = cells
  .map((k) => `${k} ${shape.values[k]}/${SHAPE_BASELINE[k]}`)
  .join(' · ')

if (errors.length === 0) {
  console.log('\n== core-shape-audit ==')
  ruleLine(true, 'CS-001', `形状棘轮：${shapeLine}`)
  ruleLine(
    true,
    'CS-002',
    waivers.length
      ? `放宽 ${waivers.length} 格，引用的 ADR 全部能打开：${waivers.map((w) => `${w.key}←${w.reason.match(ADR_REF_RE)?.[0]}`).join('、')}`
      : '没有放宽痕迹（基线就是当前形状）',
  )
  const under = cells.filter((k) => shape.values[k] < SHAPE_BASELINE[k])
  if (under.length) {
    console.log(
      yellow('提示') +
        `：已收到基线下方（${under.map((k) => `${k} ${shape.values[k]}<${SHAPE_BASELINE[k]}`).join(' · ')}），` +
        `请同步本脚本的 SHAPE_BASELINE——留着旧数字等于把余量白留给下一次`,
    )
  }
  if (VERBOSE) {
    console.log('  —— pub trait 形状（成员数 / 其中带默认体）')
    for (const r of shape.rows)
      console.log(`  [T] ${r.name.padEnd(26)} ${String(r.fns).padStart(2)}/${String(r.defaults).padEnd(2)} ${r.file}`)
  }
  process.exit(0)
}

console.error(`\n== core-shape-audit：${errors.length} 处违规 ==`)
for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
console.error(
  `\n${yellow('提示')}：形状量的口径与不判什么见本脚本文件头；放宽口径见 ` +
    `${green('docs/010-adr/055.md')}；逐 trait 清单跑 node scripts/core-shape-audit.mjs --verbose`,
)
if (VERBOSE) {
  for (const r of shape.rows)
    console.error(`  [T] ${r.name.padEnd(26)} ${String(r.fns).padStart(2)}/${String(r.defaults).padEnd(2)} ${r.file}`)
}
process.exit(1)
