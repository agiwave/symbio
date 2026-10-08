#!/usr/bin/env node
/**
 * gate-wiring-audit — 门禁接线对账（GW-001 / GW-002 / GW-003）
 *
 * 治的病：**测试文件存在 ≠ 会被门禁跑**。`gate.d/30-docs.mjs` 的 `GUARDS` / `TEST_ONLY`
 * 是两份手写名单（每条都带着「为什么值得跑」的说明，所以它不能被目录扫描替掉），
 * 而新增一份 `.test.mjs` 的人不会知道要去那里登记——于是它红了也没人知道，
 * 比没有测试更糟：仓里躺着一份「看起来有人在跑」的测试。
 *
 * 判据（名单与目录事实互为对账，四个方向都判）：
 *   - **GW-001** `scripts` 目录（递归）里的每一份 `X.test.mjs`，其去后缀名 `X` 必须出现在
 *     `GUARDS ∪ TEST_ONLY`，否则红——「存在但从不运行」的测试。
 *   - **GW-002** 三张名单里指名的每一个 `<名>` 必须在磁盘上存在，
 *     否则红——文件被改名/搬走/删掉后，门禁那一步会静默跳过（`node --test` 找不到
 *     文件时报的是「0 个测试」，不是失败）。
 *   - **GW-003** 同一个名不得同时在 `GUARDS` 与 `TEST_ONLY`：前者跑两遍（回归测试 +
 *     守卫本体），后者只跑一遍，两处语义不同却都算「接了线」。
 *   - **GW-004**（命名空间闭合）`scripts/` 下每一个**非测试** `.mjs` 必须落在
 *     `GUARDS ∪ TEST_ONLY ∪ LIBS ∪ OUT_OF_GATE` 之一，否则红。
 *     这是 GW-001 的镜像缺口：GW-001 扫的是 `.test.mjs`，于是一份**既没有回归测试、
 *     又不在任何名单里**的 `.mjs`（守卫 / 生成器 / 共享库）是完全隐形的——写一份
 *     `foo-audit.mjs` 忘了登记，它就静默地不存在，而门禁照旧全绿。实测的现成例子是
 *     `route-facts.mjs`：M10 把它立为「什么算一条路由臂」的唯一语法实现、被三个判定
 *     共读，它自己却没有一道门禁步骤。**放行对照**：`scripts/gate.d/*.mjs` 的阶段文件
 *     由 `gate.mjs` **按目录扫描**接入（不需要登记），它们的顺序由 GW-005 判。
 *   - **GW-005**（阶段顺序）`gate.mjs` 按**文件名排序**加载 `gate.d/*.mjs`，而
 *     `autoWork`（重生成并暂存）的阶段必须**排在最后**——它要把暂存状态定格成
 *     「本次改动的终态」，排在它后面跑就会让暂存发生在被验证的改动之前，
 *     `git status` 再也看不出前一阶段有没有碰过东西。判据取「源码里 import 了
 *     `autoWork`」这个形状（与 `55-verify` 读 `should_not_compile` 同源：能力由源码
 *     自声明，不维护第二份名单），于是新增一个 `65-*.mjs` 就会把它顶掉 ⇒ 红。
 *
 * 名单仍然是手写的、仍然带注释——本脚本不生成它，只判它有没有覆盖目录事实。
 * 这正是它有意义的原因：**两份真相**（名单 + 目录）各说各话时才红，
 * 若把名单改成目录扫描产物，就再也没有「忘了接线」这回事可检测了。
 *
 * 用法：node scripts/gate-wiring-audit.mjs [--root=<仓库根>] [--strict]
 *      （`--strict` 与其他判定型守卫同参数，本脚本没有 WARNING 级判定，带不带同义）
 * 退出码：0 = 通过；1 = 有失效；2 = 审计范围读不出（名单或目录都取不到）。
 */
// 判据码命名空间（登记表 docs/reference/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns GW 门禁接线（测试文件与门禁名单的对账）
// @codes GW-001 GW-002 GW-003 GW-004 GW-005

import fs from 'node:fs'
import path from 'node:path'
import { pathToFileURL, fileURLToPath } from 'node:url'
import { green, red } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? path.resolve(rootArg.slice(7)) : path.resolve(scriptDir, '..')

const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai', '.symbio'])

/** 递归收集 `scripts` 下的 `*.test.mjs`，返回相对仓库根、去掉后缀的名字（`gate.d/_shared`） */
function testNamesOf(root) {
  return collect(root, (f) => f.endsWith('.test.mjs')).map((rel) => rel.replace(/\.test\.mjs$/, ''))
}

/**
 * 递归收集 `scripts` 下的 `*.mjs`，返回 `{ rel, name }`（name 去掉 `.mjs`）。
 * GW-004 用它扫「非测试的 `.mjs`」，所以它与 GW-001 共用同一个目录事实来源
 * （`collect` 返回的是**带后缀**的相对路径，剔除测试要在这一层做）。
 */
function scriptFilesOf(root) {
  return collect(root, (f) => f.endsWith('.mjs')).map((rel) => ({
    rel,
    name: rel.replace(/\.mjs$/, ''),
  }))
}

/** 递归收集的统一实现：谓词决定收谁，返回**带后缀**的相对仓库根路径 */
function collect(root, keep) {
  const base = path.join(root, 'scripts')
  if (!fs.existsSync(base)) return null
  const out = []
  const walk = (dir, prefix) => {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      if (ent.isDirectory()) {
        if (!SKIP_DIRS.has(ent.name)) walk(path.join(dir, ent.name), `${prefix}${ent.name}/`)
        continue
      }
      if (!keep(ent.name)) continue
      out.push(`${prefix}${ent.name}`)
    }
  }
  walk(base, '')
  return out.sort()
}

/**
 * 门禁阶段：`scripts/gate.d/` 下非 `_` 前缀的 `.mjs`。它们由 `gate.mjs` **按目录扫描**
 * 接入，因此不进 GW-004 的登记对账（登记它们等于把「按扫描接入」变成「按名单接入」，
 * 那正是 GW-004 想避免的形状）；顺序由 GW-005 判。
 */
function stageFilesOf(root) {
  const dir = path.join(root, 'scripts', 'gate.d')
  if (!fs.existsSync(dir)) return null
  return fs
    .readdirSync(dir)
    .filter((f) => f.endsWith('.mjs') && !f.startsWith('_'))
    .sort()
}

/** 读 30-docs 导出的名单；读不出（没导出 / 语法错）即「审计范围读不出」 */
async function listsOf(root) {
  const modPath = path.join(root, 'scripts', 'gate.d', '30-docs.mjs')
  if (!fs.existsSync(modPath)) return null
  const mod = await import(pathToFileURL(modPath).href)
  if (!Array.isArray(mod.GUARDS) || !Array.isArray(mod.TEST_ONLY)) return null
  // `LIBS` / `OUT_OF_GATE` 可选：没导出按空表处理，于是 GW-004 会把它们报成未登记
  // ——**这是刻意的**：想要它们生效就得真的导出来，不能靠「忘了导出」蒙混过关。
  return {
    guards: mod.GUARDS,
    testOnly: mod.TEST_ONLY,
    libs: Array.isArray(mod.LIBS) ? mod.LIBS : [],
    outOfGate: Array.isArray(mod.OUT_OF_GATE) ? mod.OUT_OF_GATE : [],
  }
}

console.log('--- gate-wiring-audit: 门禁接线对账（GW-001…005） ---')

const lists = await listsOf(REPO_ROOT)
const files = testNamesOf(REPO_ROOT)
const scripts = scriptFilesOf(REPO_ROOT)
const stages = stageFilesOf(REPO_ROOT)
if (!lists || !files || !scripts || !stages) {
  console.log(red('[ERROR] GW-002 审计范围读不出：名单或 `scripts/` 目录取不到（' + REPO_ROOT + '）'))
  process.exit(2)
}

const wired = new Set([...lists.guards, ...lists.testOnly])
const unwired = files.filter((n) => !wired.has(n))
const missing = [...lists.guards, ...lists.testOnly].filter((n) => !files.includes(n))
const both = lists.guards.filter((n) => lists.testOnly.includes(n))

// GW-002 扩面：`LIBS` / `OUT_OF_GATE` 指名的**本体**必须在磁盘上。
// 前两张名单指名的是 `.test.mjs`（本体由测试文件的存在性间接保证），这两张指的
// 就是 `.mjs` 本身——指向一个不存在的文件，等于把一份缺失登记成「已覆盖」。
const libBodiesMissing = [...lists.libs, ...lists.outOfGate].filter(
  (n) => !scripts.some((f) => f.name === n),
)
// GW-004：命名空间闭合。阶段文件（`gate.d/` 下非 `_` 前缀）按扫描接入，不参与对账。
const registered = new Set([
  ...lists.guards,
  ...lists.testOnly,
  ...lists.libs,
  ...lists.outOfGate,
  ...stages.map((f) => `gate.d/${f.replace(/\.mjs$/, '')}`),
])
const unregistered = scripts
  .filter((f) => f.name !== 'gate.mjs' || !lists.outOfGate.includes('gate'))
  .filter((f) => !f.rel.endsWith('.test.mjs'))
  .filter((f) => !(f.rel.startsWith('gate.d/') && !f.rel.slice('gate.d/'.length).startsWith('_')))
  .filter((f) => !registered.has(f.name))

// GW-005：重生成阶段（源码里 import 了 `autoWork`）必须排在阶段序列末尾。
// 判据取**源码自声明**的形状，理由同 `55-verify` 读 `should_not_compile`：
// 不维护第二份「哪些阶段会重生成」的名单——那种名单必然漂移。
const regen = stages.filter((f) => {
  try {
    return fs.readFileSync(path.join(REPO_ROOT, 'scripts', 'gate.d', f), 'utf8').includes('autoWork')
  } catch {
    return false
  }
})
const lastRegen = regen.length ? regen[regen.length - 1] : null
const afterLastRegen = lastRegen ? stages.filter((f) => f > lastRegen) : []

let errors = 0
for (const n of unwired) {
  errors++
  console.log(red(`[ERROR] GW-001 scripts/${n}.test.mjs 存在，但不在 30-docs 的 GUARDS / TEST_ONLY 里 ⇒ 门禁从不跑它`))
}
for (const n of new Set(missing)) {
  errors++
  console.log(red(`[ERROR] GW-002 名单指名 scripts/${n}.test.mjs，而磁盘上没有这个文件 ⇒ 那一步跑的是空气`))
}
for (const n of new Set(libBodiesMissing)) {
  errors++
  console.log(red(`[ERROR] GW-002 名单指名 scripts/${n}.mjs，而磁盘上没有这个文件 ⇒ 它「被覆盖」是假的`))
}
for (const n of both) {
  errors++
  console.log(red(`[ERROR] GW-003 \`${n}\` 同时出现在 GUARDS 与 TEST_ONLY ⇒ 回归测试跑两遍，且「是不是守卫」两处说法不一`))
}
for (const f of unregistered) {
  errors++
  console.log(
    red(
      `[ERROR] GW-004 scripts/${f.rel} 既不在 GUARDS / TEST_ONLY，也不在 LIBS / OUT_OF_GATE ⇒ ` +
        `它既没有门禁步骤、也不在任何名单里，是一份隐形脚本`,
    ),
  )
}
for (const f of afterLastRegen) {
  errors++
  console.log(
    red(
      `[ERROR] GW-005 scripts/gate.d/${f} 排在重生成阶段 ${lastRegen} 之后 ⇒ ` +
        `暂存会发生在被验证的改动之前，git status 再也看不出前一阶段碰没碰过东西`,
    ),
  )
}

if (errors) {
  console.log(
    `gate-wiring-audit 未通过：${errors} 处接线失效（磁盘 ${files.length} 份测试 / ` +
      `${scripts.length - files.length} 份脚本 / 名单 ${registered.size} 项 / 阶段 ${stages.length} 个）`,
  )
  process.exit(1)
}
console.log(
  green(
    `gate-wiring-audit 通过：磁盘 ${files.length} 份测试全部接线，` +
      `名单 ${lists.guards.length} + ${lists.testOnly.length} + ${lists.libs.length} + ${lists.outOfGate.length} 项逐一对应；` +
      `${scripts.length - files.length} 份脚本全部登记；重生成阶段 ${lastRegen ?? '（无）'} 排在阶段末尾`,
  ),
)
