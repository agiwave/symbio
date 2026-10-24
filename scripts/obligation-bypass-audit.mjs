#!/usr/bin/env node
/**
 * obligation-bypass-audit — 环的**派发面**只许收窄（OB-001…003）
 *
 * ## 它判什么
 *
 * | 码 | 判什么 | 它拦的是哪一种退化 |
 * |---|---|---|
 * | OB-001 | `owes()` 的**直调点数** ≤ `BYPASS_BASELINE` | 派发面变宽：又长出一条绕开 `drive` 的路 |
 * | OB-002 | 直调点数**低于**基线时提示同步下调 | 余量白留：棘轮松着，下一次「多一处」不会红 |
 * | OB-003 | 每处直调点的**行名**必须是 `ROWS` 里真实存在的行 | 点名不存在的行 ⇒ `owes` 恒 `false` ⇒ 这段旁路代码**永不执行** |
 *
 * 计数口径写在 `BYPASS_BASELINE` 那一行旁边，**不写在设计文档正文里**——正文会漂，
 * 基线不会。DE-015 §5.3 只写口径与「归零」这个定义。
 *
 * ## 为什么需要它（2026-10-23 推敲的结论）
 *
 * DE-015 §0 声称「控制面收敛为债环，`drive` 是唯一派发器」。实测：`drive()` 是**通用**
 * 派发器（`next` → `ring::drive`，任何 `by` 能解析的行都派得动），但有 12 处**直调
 * `owes()` 再用旁路代码执行**（10 行）——那些账在到不了派发器之前就被关掉。这不是
 * bug——DE-015 §8 自己叫它
 * 「旁路收编」，是已知过渡态。真正的病是**它没有数字**，于是三句话都无从执行：
 *
 * 1. GO-003 E5 说「过渡结构与死码**只减不增**」——没有计数，这句话没有主语；
 * 2. 「收口完成」没有定义 ⇒ 没人知道什么时候可以删掉这些旁路，于是永远不删；
 * 3. `obligation-ring-accept` 的 J8 口径写着「DE-015 §0『没有别的调度代码』」，
 *    但它只对账**模型往返**（`HTTP = sent(model) + 1`）——工具与策略行的旁路它看不见。
 *    那句声称今天只被检查了三分之一。
 *
 * ## 为什么是棘轮，不是立刻收编
 *
 * 12 处旁路各自依赖不同的上下文（有的在流式回调里、有的在工具批次里、有的在唤醒路径上），
 * 一次性搬进 `drive` 是一次跨文件重构，风险远大于收益。而计数棘轮**立刻**给了 E5 一个
 * 可执行的形式，并且把「收口完成」定义成**计数归零**——一个可以被逐步逼近的目标，
 * 而不是一句口号。每收编一处，`BYPASS_BASELINE` 减一；减到 0 那天，这条判据就从
 * 「棘轮」变成「零容忍」，而它的代码一行都不用改。
 *
 * ## 13 与 12：本判据诞生第一天就抓到的第一个缺陷
 *
 * 立这条判据的原始记录写的是「**13 处**」，并列了 12 个位置。少的那一个是
 * `loop_rows.rs` 里 `pub(crate) fn owes(...)` 的**定义行**——`grep -c 'owes('` 把它
 * 一起数了。这正是「靠人数」的典型坏法：清单是对的，总数是错的，而错的那一位
 * 没有任何东西会去核。所以判据的计数规则必须**排除定义**（`fn owes(`），
 * 并且把它写成代码，而不是写在一段说明里。
 *
 * ## 失效形态（三种，全是静默变绿）
 *
 * 1. **把定义行算成调用点** ⇒ 基线天生多一格，永远松着。回归测试注入一个
 *    `fn owes(` 定义并断言计数不变。
 * 2. **把测试文件算进来** ⇒ `loop_rows.test.rs` 有 90 处 `owes(...)` 断言，
 *    数字从 12 跳到 100+，基线立刻失去意义（然后被人「下调」成 100）。
 *    回归测试放一个 `*.test.rs` 并断言不计。
 * 3. **范围读不出却报通过** ⇒ 最省事的实现是「没扫到 `.rs` ⇒ 没什么可判 ⇒ 通过」，
 *    于是判据永久恒真。故范围为空直接 `exit 2`。
 *
 * 用法：node scripts/obligation-bypass-audit.mjs [--root=<仓库根>] [--strict] [--verbose]
 * 退出码：0 = 通过；1 = 有违规；2 = 审计范围读不出（不退化成「什么都没检查」）
 */
// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns OB 义务环派发面（旁路直调计数棘轮）
// @codes OB-001 OB-002 OB-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow, dim } from './color.mjs'
import { blankComments, matchBrace } from './rust-scan.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice(7)) : path.resolve(scriptDir, '..')
const VERBOSE = process.argv.includes('--verbose')

/**
 * 旁路直调 `owes()` 的点数上限（**只许降**）。
 *
 * 口径：`owes(` 的**调用点**，排除三处——`fn owes(` 的定义行、`*.test.rs`、
 * 以及 `drive_steps` 函数体（那里问债不是旁路，它就是派发器本体）。
 *
 * 每收编一处，这个数字减一；归零那天 DE-015 §5.3 的「收口完成」就成立了。
 * 放宽它需要 `35-baseline` 的 `obligation-bypass` 那一格留痕——本脚本不给自己开后门。
 */
const BYPASS_BASELINE = process.env.OBLIGATION_BYPASS_BASELINE
  ? Number(process.env.OBLIGATION_BYPASS_BASELINE)
  : 12

const SKIP_DIRS = new Set([
  'target',
  'verify-target',
  'node_modules',
  '.git',
  '.workbuddy-ai',
  '.symbio',
  'dist',
  'archive',
])
const SCAN_ROOTS = ['symbio/src', 'cli/src', 'tauri/src-tauri/src']

/** 扫 `SCAN_ROOTS` 下的 `.rs`（跳过测试文件与垃圾目录） */
function rustFiles() {
  const out = []
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, ent.name)
      if (ent.isDirectory()) {
        if (!SKIP_DIRS.has(ent.name)) walk(p)
        continue
      }
      // `*.test.rs` 是 Rust 侧的测试落点（本仓约定，`test-layout-audit` 判它）：
      // 那里的 `owes(...)` 是在**断言行表**，不是在旁路执行，计数会把基线喂爆。
      if (ent.name.endsWith('.rs') && !ent.name.endsWith('.test.rs')) out.push(p)
    }
  }
  for (const r of SCAN_ROOTS) walk(path.join(ROOT, r))
  return out.sort()
}

/**
 * `ROWS` 里声明的行名。
 *
 * 只取 `static ROWS: &[Decl] = &[ … ]` 那一块：同一文件里还有 `ACTORS`（执行体名）
 * 与端口声明（端口名），它们的 `name:` 长得一模一样，整文件抓会把「端口名」
 * 当成合法行名——于是 OB-003 会被一个**不是行**的名字满足。
 */
function declaredRows(rowsFile) {
  if (!fs.existsSync(rowsFile)) return null
  const text = blankComments(fs.readFileSync(rowsFile, 'utf8')).join('\n')
  const start = text.search(/static ROWS\s*:\s*&\[Decl\]\s*=\s*&\[/)
  if (start < 0) return null
  const open = text.indexOf('=', start)
  if (open < 0) return null
  // ⚠️ 不能用 `matchBrace` 找结尾：它数的是 `{}`，而这里的**最外层是 `[]`**。
  // 拿 `[` 的下标喂它，第一个 `Decl { … }` 的 `}` 就会让深度归零 ⇒ 只截到第一行，
  // 于是「行表里只有 authorize-tool」——OB-003 会把 10 处合法点名报成悬空。
  // 顶层 `static` 以行首 `];` 收尾，按行首锚定才是这一块的边界。
  const close = text.indexOf('\n];', open)
  const body = text.slice(open, close < 0 ? text.length : close)
  const names = new Set()
  for (const m of body.matchAll(/\bname:\s*"([^"]+)"/g)) names.add(m[1])
  return names
}

/** `drive_steps` 的函数体区间（`[start, end)`）——派发器本体问债不算旁路 */
function driveStepsSpan(text) {
  const m = /fn\s+drive_steps\b/.exec(text)
  if (!m) return null
  const open = text.indexOf('{', m.index)
  if (open < 0) return null
  return [m.index, matchBrace(text, open) + 1]
}

console.log('--- obligation-bypass-audit: 派发面（OB-001…003） ---')

const files = rustFiles()
if (files.length === 0) {
  console.log(red('[ERROR] OB-001 审计范围读不出：`symbio/src` 等根下一个 `.rs` 都没有 ⇒ 不退化成「什么都没检查」'))
  process.exit(2)
}

const rowsFile = path.join(ROOT, 'symbio/src/plugins/loop/loop_rows.rs')
const rows = declaredRows(rowsFile)
if (!rows || rows.size === 0) {
  console.log(red('[ERROR] OB-003 读不出 `ROWS` 行表 ⇒ 行名无从核对（判据没有输入，宁红不静默）'))
  process.exit(2)
}

/** 直调点：`owes(` 的调用（定义行不算），带行名 */
const sites = []
let definitionLines = 0
for (const file of files) {
  const raw = fs.readFileSync(file, 'utf8')
  const lines = blankComments(raw)
  const text = lines.join('\n')
  const span = driveStepsSpan(text)
  let offset = 0
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    if (line.includes('owes')) {
      for (const m of line.matchAll(/\bowes\s*\(/g)) {
        const at = offset + m.index
        if (/\bfn\s+$/.test(line.slice(0, m.index))) {
          definitionLines++
          continue
        }
        if (span && at >= span[0] && at < span[1]) continue
        // 行名 = 实参里第一个字符串字面量（多行调用也要抓到）
        const tail = text.slice(at, at + 300)
        const lit = /"([^"]*)"/.exec(tail)
        sites.push({
          file,
          line: i + 1,
          row: lit ? lit[1] : null,
          src: line.trim().slice(0, 90),
        })
      }
    }
    offset += line.length + 1
  }
}

// ── OB-003：行名必须存在 ────────────────────────────────────────────
let errors = 0
for (const s of sites) {
  const rel = path.relative(ROOT, s.file).replace(/\\/g, '/')
  if (!s.row) {
    errors++
    console.log(
      red(`[ERROR] OB-003 ${rel}:${s.line} 直调点的行名不是字面量 ⇒ 判据与人都读不出它问的是哪一行`),
    )
    console.log(`       ${s.src}`)
    continue
  }
  if (rows.has(s.row)) continue
  errors++
  console.log(
    red(
      `[ERROR] OB-003 ${rel}:${s.line} 直调 \`owes("${s.row}")\` 而 \`ROWS\` 里没有这一行 ⇒ ` +
        `\`owes\` 恒 false，这段旁路代码永不执行（编译、测试、门禁全绿）`,
    ),
  )
  console.log(`       ${s.src}`)
}

// ── OB-001 / OB-002：计数棘轮 ───────────────────────────────────────
const count = sites.length
if (!Number.isFinite(BYPASS_BASELINE)) {
  errors++
  console.log(red(`[ERROR] OB-001 基线不是数字（${BYPASS_BASELINE}）—— 判据读不出即失效，宁红不静默`))
} else if (count > BYPASS_BASELINE) {
  errors++
  console.log(
    red(
      `[ERROR] OB-001 派发面变宽：直调 \`owes()\` 的点数 ${count} > 基线 ${BYPASS_BASELINE} —— ` +
        `环的派发面只许收窄（GO-003 E5）。多出来的点要么接回 \`drive\`（加 \`ActorEntry\`），` +
        `要么说明为什么它不能接回并同步抬高基线（要 \`35-baseline\` 那一格留痕）`,
    ),
  )
  for (const s of sites.slice(0, count)) {
    const rel = path.relative(ROOT, s.file).replace(/\\/g, '/')
    console.log(dim(`       ${rel}:${s.line}  ${s.row ?? '?'}`))
  }
}

const byRow = new Map()
for (const s of sites) byRow.set(s.row ?? '?', (byRow.get(s.row ?? '?') ?? 0) + 1)
const rowLine = [...byRow.entries()]
  .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
  .map(([r, n]) => `${r}×${n}`)
  .join(' · ')

if (errors) {
  console.log(
    `obligation-bypass-audit 未通过：${errors} 处（OB-001 计数 / OB-003 行名）；` +
      `扫 ${files.length} 份 .rs，直调 ${count} 处 / 基线 ${BYPASS_BASELINE}`,
  )
  process.exit(1)
}

console.log(
  green(
    `obligation-bypass-audit 通过：扫 ${files.length} 份 .rs，旁路直调 \`owes()\` ${count}/${BYPASS_BASELINE} 处` +
      `（${rowLine}）；${rows.size} 行声明表内点名全部存在`,
  ),
)
if (count < BYPASS_BASELINE) {
  console.log(
    yellow('提示') +
      `：已收到基线下方（${count}<${BYPASS_BASELINE}），请同步本脚本的 BYPASS_BASELINE——` +
      `留着旧数字等于把余量白留给下一次（排除的定义行 ${definitionLines} 处不计）`,
  )
}
if (VERBOSE) {
  for (const s of sites) {
    const rel = path.relative(ROOT, s.file).replace(/\\/g, '/')
    console.log(`  ${rel}:${s.line}  ${s.row ?? '?'}`)
  }
}
