#!/usr/bin/env node
/**
 * gate-wiring-audit — 门禁接线对账（GW-001 / GW-002 / GW-003）
 *
 * 治的病：**测试文件存在 ≠ 会被门禁跑**。`gate.d/30-docs.mjs` 的 `GUARDS` / `TEST_ONLY`
 * 是两份手写名单（每条都带着「为什么值得跑」的说明，所以它不能被目录扫描替掉），
 * 而新增一份 `.test.mjs` 的人不会知道要去那里登记——于是它红了也没人知道，
 * 比没有测试更糟：仓里躺着一份「看起来有人在跑」的测试。
 *
 * 判据（名单与目录事实互为对账，两个方向都判）：
 *   - **GW-001** `scripts` 目录（递归）里的每一份 `X.test.mjs`，其去后缀名 `X` 必须出现在
 *     `GUARDS ∪ TEST_ONLY`，否则红——「存在但从不运行」的测试。
 *   - **GW-002** 两份名单里指名的每一个 `<名>.test.mjs` 必须在磁盘上存在，
 *     否则红——文件被改名/搬走/删掉后，门禁那一步会静默跳过（`node --test` 找不到
 *     文件时报的是「0 个测试」，不是失败）。
 *   - **GW-003** 同一个名不得同时在 `GUARDS` 与 `TEST_ONLY`：前者跑两遍（回归测试 +
 *     守卫本体），后者只跑一遍，两处语义不同却都算「接了线」。
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
// @codes GW-001 GW-002 GW-003

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
  const base = path.join(root, 'scripts')
  if (!fs.existsSync(base)) return null
  const out = []
  const walk = (dir, prefix) => {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      if (ent.isDirectory()) {
        if (!SKIP_DIRS.has(ent.name)) walk(path.join(dir, ent.name), `${prefix}${ent.name}/`)
        continue
      }
      if (!ent.name.endsWith('.test.mjs')) continue
      out.push(`${prefix}${ent.name.replace(/\.test\.mjs$/, '')}`)
    }
  }
  walk(base, '')
  return out.sort()
}

/** 读 30-docs 导出的两张名单；读不出（没导出 / 语法错）即「审计范围读不出」 */
async function listsOf(root) {
  const modPath = path.join(root, 'scripts', 'gate.d', '30-docs.mjs')
  if (!fs.existsSync(modPath)) return null
  const mod = await import(pathToFileURL(modPath).href)
  if (!Array.isArray(mod.GUARDS) || !Array.isArray(mod.TEST_ONLY)) return null
  return { guards: mod.GUARDS, testOnly: mod.TEST_ONLY }
}

console.log('--- gate-wiring-audit: 门禁接线对账（GW-001…003） ---')

const lists = await listsOf(REPO_ROOT)
const files = testNamesOf(REPO_ROOT)
if (!lists || !files) {
  console.log(red('[ERROR] GW-002 审计范围读不出：名单或 `scripts/` 目录取不到（' + REPO_ROOT + '）'))
  process.exit(2)
}

const wired = new Set([...lists.guards, ...lists.testOnly])
const unwired = files.filter((n) => !wired.has(n))
const missing = [...lists.guards, ...lists.testOnly].filter((n) => !files.includes(n))
const both = lists.guards.filter((n) => lists.testOnly.includes(n))

let errors = 0
for (const n of unwired) {
  errors++
  console.log(red(`[ERROR] GW-001 scripts/${n}.test.mjs 存在，但不在 30-docs 的 GUARDS / TEST_ONLY 里 ⇒ 门禁从不跑它`))
}
for (const n of new Set(missing)) {
  errors++
  console.log(red(`[ERROR] GW-002 名单指名 scripts/${n}.test.mjs，而磁盘上没有这个文件 ⇒ 那一步跑的是空气`))
}
for (const n of both) {
  errors++
  console.log(red(`[ERROR] GW-003 \`${n}\` 同时出现在 GUARDS 与 TEST_ONLY ⇒ 回归测试跑两遍，且「是不是守卫」两处说法不一`))
}

if (errors) {
  console.log(`gate-wiring-audit 未通过：${errors} 处接线失效（磁盘 ${files.length} 份测试 / 名单 ${wired.size} 项）`)
  process.exit(1)
}
console.log(
  green(
    `gate-wiring-audit 通过：磁盘 ${files.length} 份测试全部接线，` +
      `名单 ${lists.guards.length} + ${lists.testOnly.length} 项逐一对应`,
  ),
)
