#!/usr/bin/env node
/**
 * panic-audit — 健壮性面变成显式登记（PN-001…003 + 只减棘轮）
 *
 * ## 它治的病
 *
 * 非测试 Rust 里的 `unwrap()` / `expect()` / `panic!()` / `unreachable!()` / `todo!()` /
 * `unimplemented!()` 此前**没有任何地方说清哪一处是哪一种**。于是「这一处是不变量」
 * 与「这一处是事故」在文本上完全一样，而 J3（静默失效 ⇒ 编译期强制或 CI 断言）要求
 * 两者必须可区分。
 *
 * 本守卫不判「该不该 panic」——那是人的判断。它只判**每一处都必须是一个显式事实**：
 * 有一行登记说明它为什么可以 panic。没登记就是红。
 *
 * ## 判据
 *
 * - **PN-001** 每一处命中（按 `文件::所属函数` 归并）必须在**同一个文件**里有一条
 *   `// panic-allow <文件>::<函数>: <理由>`。登记写在代码旁边而不是集中一张表——
 *   理由与它解释的那段代码会一起被 review、一起被删。
 * - **PN-002** 理由**非空**。空理由的登记等于没有登记（与 `dead-code-allow` /
 *   `baseline-allow` 同一条约定）。
 * - **PN-003** 登记项必须有对应的命中代码：登记了而代码里已经没有 ⇒ 红。
 *   否则豁免只增不减，最后变成一张没人维护的清单。
 * - **只减棘轮**（`BASELINE.panicSites`）：命中总数只许降。清掉一处登记必须同时
 *   删掉那一处代码——这是让「显式事实」不退化成「历史包袱」的唯一办法。
 *
 * ## 取范围的三个约定（每一处都对应一种真实的漏法）
 *
 * 1. **不判测试**：`*.test.rs`、`tests.rs`、内联 `#[cfg(test)] mod`、以及
 *    **单独带 `#[cfg(test)]` 的项**。最后这条最容易漏——`stripTestModules` 只认
 *    `mod tests { }` 块，而本仓的 `partial_json.rs` 把两个测试辅助写成
 *    `#[cfg(test)] pub(crate) fn assert_*`，它们在**生产文件**里，照 `mod` 判会
 *    被当成生产 panic 面。
 * 2. **不判注释**：命中在 `//` / `///` / `/*` 之后的不算（注释里提到 `unwrap()` 是
 *    常态，写「这里不能用 unwrap」就会自我命中）。
 * 3. **所属函数取「向上最近的 `fn` 定义行」**。rustfmt 下「一行一个签名」很稳；
 *    实测 267 份非测试 .rs、命中 0 处取不到所属函数（回归测试钉住这一条）。
 *
 * 用法：node scripts/panic-audit.mjs [--root=<仓库根>] [--strict]
 * 退出码：0 = 通过；1 = 有失效；2 = 审计范围读不出。
 */
// 判据码命名空间（登记表 docs/reference/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns PN 健壮性登记（每一处 panic 都是显式事实）
// @codes PN-001 PN-002 PN-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red } from './color.mjs'
import { stripComments, stripTestModules } from './rust-scan.mjs'
import { BASELINE } from './gate.d/_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? path.resolve(rootArg.slice(7)) : path.resolve(scriptDir, '..')

const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai', '.symbio', 'archive', 'docs'])
const SRC = path.join('symbio', 'src')
/** 命中形态：与 M6 的 S-012 同一族的「形状」判据，不取词表。
 *  `HIT`（非全局）给 `.test` 用；`HIT_G`（全局）给 `matchAll` 数出现次数用——
 *  带 `g` 的正则做 `.test` 是**有状态**的（`lastIndex` 会留在上一次匹配之后），
 *  那样每两行就会漏判一次。 */
const HIT = /\.unwrap\(\)|\.expect\(\s*|panic!\(|unreachable!\(|todo!\(|unimplemented!\(/
const HIT_G = new RegExp(HIT.source, 'g')
const FN_DEF = /\b(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)/

/** 递归收集参与生产事实的 .rs（排除测试文件） */
function collectRs(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) {
      if (!SKIP_DIRS.has(e.name)) collectRs(p, out)
      continue
    }
    if (e.name.endsWith('.rs') && !e.name.endsWith('.test.rs') && e.name !== 'tests.rs') out.push(p)
  }
  return out
}

/**
 * 再抹一层**带 `#[cfg(test)]` 的单个项**。
 *
 * `stripTestModules` 只认 `mod xxx { }` 块；本仓有把测试辅助写成
 * `#[cfg(test)] pub(crate) fn assert_*` 的（`plugins/model/protocols/partial_json.rs`），
 * 它们住在**生产文件**里，不抹掉就会被当成生产 panic 面登记进去——于是登记表里
 * 混进两处根本不会在生产编译的代码，而「登记 = 显式事实」这句话就掺了假。
 *
 * 判据取属性紧贴项首（中间只允许空行与其它属性），从项首按大括号配对抹到平衡。
 */
function stripCfgTestItems(text) {
  const lines = text.split('\n')
  const out = lines.slice()
  for (let i = 0; i < lines.length; i++) {
    if (!/^\s*#\[cfg\(test\)\]\s*$/.test(lines[i])) continue
    // 往下找项首（跳过空行与其它属性）
    let j = i + 1
    while (j < lines.length && (lines[j].trim() === '' || /^\s*#\[/.test(lines[j]))) j++
    if (j >= lines.length) continue
    const open = lines[j].indexOf('{')
    if (open < 0) {
      // 无花括号的项（如 `#[cfg(test)] use x::*;`）——只抹属性那一行即可
      out[i] = ''
      continue
    }
    let depth = 0
    for (let k = j; k < lines.length; k++) {
      for (const ch of lines[k]) {
        if (ch === '{') depth++
        else if (ch === '}') depth--
      }
      out[k] = ''
      if (depth === 0) {
        i = k
        break
      }
    }
  }
  return out.join('\n')
}

/** 某个文件里的全部命中：Map(key → 命中处），key = `相对路径::函数名`。
 *  命中按**出现次数**计而不是按行——`a.unwrap(), b.expect("x")` 是一行两处，
 *  按行数会把健壮性面少报一半（棘轮随之偏松）。 */
function sitesOf(rel, abs) {
  const raw = fs.readFileSync(abs, 'utf8').replace(/\r\n/g, '\n')
  // 登记要从**原始文本**里读（注释就是登记的地方），命中要从抹过的文本里数
  const clean = stripCfgTestItems(stripTestModules(stripComments(raw)))
  const lines = clean.split('\n')
  const byKey = new Map()
  let curFn = null
  for (let i = 0; i < lines.length; i++) {
    const m = FN_DEF.exec(lines[i])
    if (m) curFn = m[1]
    if (!HIT.test(lines[i])) continue
    if (!curFn) continue
    const key = `${rel}::${curFn}`
    if (!byKey.has(key)) byKey.set(key, [])
    byKey.get(key).push({ line: i + 1, n: [...lines[i].matchAll(HIT_G)].length })
  }
  return byKey
}

/** 文件里的登记：Map(key → 理由) */
function registrationsOf(rel, abs) {
  const raw = fs.readFileSync(abs, 'utf8').replace(/\r\n/g, '\n')
  const out = new Map()
  const re = new RegExp(`^\\s*//\\s*panic-allow\\s+${regExpEscape(rel)}::([A-Za-z_][A-Za-z0-9_]*):\\s*(.*)$`, 'gm')
  for (const m of raw.matchAll(re)) out.set(`${rel}::${m[1]}`, m[2].trim())
  return out
}

const regExpEscape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

console.log('--- panic-audit: 健壮性面登记对账（PN-001…003） ---')

const srcDir = path.join(REPO_ROOT, SRC)
if (!fs.existsSync(srcDir)) {
  console.log(red(`[ERROR] PN-001 审计范围读不出：${SRC} 不存在（${REPO_ROOT}）`))
  process.exit(2)
}

const files = collectRs(srcDir)
if (files.length === 0) {
  console.log(red('[ERROR] PN-001 审计范围读不出：一份非测试 .rs 都没收集到'))
  process.exit(2)
}

let errors = 0
let totalSites = 0
let totalKeys = 0
const allKeys = new Map()
const allRegs = new Map()

for (const abs of files) {
  const rel = path.relative(REPO_ROOT, abs).replace(/\\/g, '/')
  const sites = sitesOf(rel, abs)
  const regs = registrationsOf(rel, abs)
  for (const [k, v] of sites) allKeys.set(k, v)
  for (const [k, v] of regs) allRegs.set(k, { reason: v, rel })
  totalKeys += sites.size
  totalSites += [...sites.values()].reduce((n, l) => n + l.reduce((m, h) => m + h.n, 0), 0)
}

for (const [key, hits] of allKeys) {
  const reg = allRegs.get(key)
  const n = hits.reduce((m, h) => m + h.n, 0)
  if (!reg) {
    errors++
    console.log(
      red(
        `[ERROR] PN-001 ${key} 有 ${n} 处 panic 面未登记（${rel1(key)}:${hits.map((h) => h.line).join(', ')}）`,
      ),
    )
    continue
  }
  if (!reg.reason) {
    errors++
    console.log(red(`[ERROR] PN-002 ${key} 的登记理由为空 ⇒ 等于没有登记`))
  }
}

for (const [key, reg] of allRegs) {
  if (allKeys.has(key)) continue
  errors++
  console.log(red(`[ERROR] PN-003 ${key} 有登记但代码里已无对应 panic 面 ⇒ 登记只增不减就成了一堆过期豁免`))
}

// 只减棘轮。与其它棘轮走**同一个来源**（`BASELINE` 在 `_shared.mjs`，不在这里另立
// 一份），但方向相反：本数**涨了就是债**。
//
// 为什么涨也要判红：新增一处 `.unwrap()` 已被 PN-001 拦住（必须带非空理由的登记），
// 若计数上涨就此放过，这道棘轮就会在无人察觉的情况下被削——正是本仓反复栽的那一类
// （黄字天天打、门禁天天绿）。涨要**显式**改 `BASELINE.panicSites` 并写明理由，
// 那一步本身就是一次判断。
if (totalSites > BASELINE.panicSites) {
  errors++
  console.log(
    red(
      `      ↳ panic 面 ${totalSites} > 基线 ${BASELINE.panicSites}：健壮性面变多了。` +
        `要么去掉那一处，要么改 scripts/gate.d/_shared.mjs 的 BASELINE.panicSites 并写明为什么这一处值得留着`,
    ),
  )
}

function rel1(key) {
  return key.split('::')[0]
}

if (errors) {
  console.log(
    `panic-audit 未通过：${errors} 处（${totalKeys} 个登记项 / ${totalSites} 处 panic 面 / ${files.length} 份非测试 .rs）`,
  )
  process.exit(1)
}
console.log(
  green(
    `panic-audit 通过：${totalKeys} 个登记项全部有非空理由，${totalSites} 处 panic 面全部登记` +
      `（${files.length} 份非测试 .rs）`,
  ),
)