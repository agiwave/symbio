#!/usr/bin/env node
/**
 * obligation-bypass-audit — 环的**派发面**只许收窄（OB-001…003）
 *
 * ## 它判什么
 *
 * | 码 | 判什么 | 它拦的是哪一种退化 |
 * |---|---|---|
 * | OB-001 | **调度旁路**（未承认的直调点）≤ `BYPASS_BASELINE` | 派发面变宽：又长出一条绕开 `drive` 的路 |
 * | OB-002 | 两类计数**低于**各自基线时提示同步下调 | 余量白留：棘轮松着，下一次「多一处」不会红 |
 * | OB-003 | 每处直调点**与 `Dispatch { row: "…" }` 点名的行**必须是 `ROWS` 里真实存在的行 | 点名不存在的行 ⇒ `owes` 恒 `false` ⇒ 这段旁路代码**永不执行**、那一格**永不派发** |
 * | OB-004 | **幂等点问**必须就地承认（标记 + 非空理由 + 点名的行已接执行体 `by: Some`），且总数 ≤ `PRECHECK_BASELINE` | 把「第二套调度」洗成「护栏」：`by: None` 的行唯一的派发点就是这一处直问，它不是动手前复核 |
 *
 * ## 为什么数两个数，不数一个（裁决 `D-158`）
 *
 * 八个点不是同一类东西，混成一个进度指针会同时造成两种事故——误删护栏、或机械搬迁造出
 * 虚假归零：
 *
 * | 类 | 判据 | 动作 |
 * |---|---|---|
 * | **调度旁路** | 点名的行 `by = None`（表里没人跑它）且该处**没有**承认标记 | 归零对象；今天 5 处（五条 `policy.*`） |
 * | **幂等点问** | 执行体（`by = Some`）动手前复核「本行还欠不欠」——并发 / 重放下第二次进入同一动作时的护栏 | **保留**，显式登记；今天 3 处 |
 *
 * ⚠ **调度旁路的归零前件是「外环收编」**（[DE-090-过渡方案](../docs/003-design/090-过渡方案.md)
 * §1.1.2 自己声明它尚未开始），不是把 `owes()` 换个名字。理由是可执行的：给某条
 * `policy.*` 行接上执行体（`by: Some`）之后，内环 `drive_steps` 也会按 `prio` 选中它，
 * 而它的动作需要轮首上下文（编排器 / 会话上下文 / 请求级开销），内环手里没有这些 ⇒
 * `entry_of` 查不到动作 ⇒ 本轮落 `exhausted{no_progress}`。所以「接线」必须先有
 * **按 host 分片的绑定表**，那一条形状属于外环收编，不归本判据催逼。
 *
 * 计数口径写在两个基线旁边，**不写在设计文档正文里**——正文会漂，基线不会。
 * DE-090 §3 只写口径与「调度旁路归零」这个定义。
 *
 * ## 为什么需要它（2026-10-23 推敲的结论）
 *
 * DE-015 §0 声称「控制面收敛为债环，`drive` 是唯一派发器」。实测：`drive()` 是**通用**
 * 派发器（`next` → `ring::drive`，任何 `by` 能解析的行都派得动），但另有一批**直调
 * `owes()` 再用旁路代码执行**的点（现行计数看 `BYPASS_BASELINE`，清单看 `--verbose`）
 * ——那些账在到不了派发器之前就被关掉。这不是
 * bug——DE-090 §3 自己叫它
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
 * 旁路各自依赖不同的上下文（有的在流式回调里、有的在工具批次里、有的在唤醒路径上），
 * 一次性搬进 `drive` 是一次跨文件重构，风险远大于收益。而计数棘轮**立刻**给了 E5 一个
 * 可执行的形式，并且把「收口完成」定义成**调度旁路计数归零**——一个可以被逐步逼近的目标，
 * 而不是一句口号。每收编一处，`BYPASS_BASELINE` 减一；减到 0 那天，这条判据就从
 * 「棘轮」变成「零容忍」，而它的代码一行都不用改。
 *
 * **归零的前件写在 `D-158` 里**：那 5 处不是「懒得搬」，是搬不动——行一接执行体
 * （`by: Some`）内环就会按 `prio` 选中它，而它的动作要的是轮首上下文。所以真正的
 * 收编形状是「一张行表 + 按 host 分片的绑定表」，即外环收编；本判据只负责让那笔账
 * **看得见、不许漂**，不负责替它造一个归零的读数。
 *
 * ## 13 与 12：本判据诞生第一天就抓到的第一个缺陷
 *
 * 立这条判据的原始记录写的是「**13 处**」，并列了 12 个位置。少的那一个是
 * `loop_rows.rs` 里 `pub(crate) fn owes(...)` 的**定义行**——`grep -c 'owes('` 把它
 * 一起数了。这正是「靠人数」的典型坏法：清单是对的，总数是错的，而错的那一位
 * 没有任何东西会去核。所以判据的计数规则必须**排除定义**（`fn owes(`），
 * 并且把它写成代码，而不是写在一段说明里。
 *
 * ## 失效形态（五种，前三种全是静默变绿）
 *
 * 1. **把定义行算成调用点** ⇒ 基线天生多一格，永远松着。回归测试注入一个
 *    `fn owes(` 定义并断言计数不变。
 * 2. **把测试文件算进来** ⇒ `loop_rows.test.rs` 有 90 处 `owes(...)` 断言，
 *    数字从 12 跳到 100+，基线立刻失去意义（然后被人「下调」成 100）。
 *    回归测试放一个 `*.test.rs` 并断言不计。
 * 3. **范围读不出却报通过** ⇒ 最省事的实现是「没扫到 `.rs` ⇒ 没什么可判 ⇒ 通过」，
 *    于是判据永久恒真。故范围为空直接 `exit 2`。
 * 4. **承认标记洗白调度旁路** ⇒ 给策略行的直问补一句「这是护栏」，`BYPASS_BASELINE`
 *    当场归零而代码一行没动。所以 OB-004 要核对**行本身**：标记处的行必须
 *    `by: Some`（执行体真的存在，复核才成立）；`by: None` 的行了结它的唯一方式就是
 *    这一处直问 ⇒ 判红并点名。回归测试拿一条 `by = None` 的行钉它。
 * 5. **标记悬空 / 理由空转** ⇒ 承认账本变成注释装饰。理由判空与 R-002 同一口径
 *    （至少一个字母 / 数字 / 汉字），且标记必须落在一个真实直调点的同行或其上三行内。
 *
 * 用法：node scripts/obligation-bypass-audit.mjs [--root=<仓库根>] [--strict] [--verbose]
 * 退出码：0 = 通过；1 = 有违规；2 = 审计范围读不出（不退化成「什么都没检查」）
 */
// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns OB 义务环派发面（调度旁路与幂等点问的两类计数棘轮）
// @codes OB-001 OB-002 OB-003 OB-004

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
 * **调度旁路**（未承认的直调点）上限（**只许降**）。
 *
 * 口径：`owes(` 的**调用点**，排除四处——`fn owes(` 的定义行、`*.test.rs`、
 * `drive_steps` 函数体（那里问债不是旁路，它就是派发器本体），以及引擎的现读点
 * `Dispatch::row_owed`（D-136：派发前问表收敛到这一处，行名由 `Dispatch` 字面量
 * 声明、由下面那半边 OB-003 核对）。带着 OB-004 承认标记的点**不进这个数**，
 * 它们进下面那个幂等点问的数。
 *
 * 每收编一处，这个数字减一；归零那天 DE-090 §3 的「收口完成」就成立了——
 * 而它的前件是外环收编（`D-158`），不是把直问改个名字。
 * 放宽它需要 `35-baseline` 的 `obligation-bypass` 那一格留痕——本脚本不给自己开后门。
 */
const BYPASS_BASELINE = process.env.OBLIGATION_BYPASS_BASELINE
  ? Number(process.env.OBLIGATION_BYPASS_BASELINE)
  : 3

/**
 * **幂等点问**上限（**只许降**）：执行体动手前复核「本行还欠不欠」的那几处。
 *
 * 它们是并发 / 重放下的护栏，不是第二套调度，**归零不是它们的目标**。承认的形态
 * 与 R-002 同构——就地一行 `// obligation-precheck OB-004: <非空理由>`（同行或其上
 * 三行），加上 OB-004 的两半核对：点名的行必须已接执行体（`by: Some`），标记不许悬空。
 *
 * 为什么还要给它一个上限：护栏一旦可以随手加，「直问」就多了一条不用过外环收编的
 * 逃生通道——加一处、补一句理由，调度旁路的数就永远追不到它。所以它同样只许降，
 * 放宽要走 `35-baseline` 的 `obligation-precheck` 那一格。
 */
const PRECHECK_BASELINE = process.env.OBLIGATION_PRECHECK_BASELINE
  ? Number(process.env.OBLIGATION_PRECHECK_BASELINE)
  : 3

/** 就地承认标记（OB-004）：与 R-002 的 `dead-code-allow` 同一判空口径 */
const PRECHECK_MARK_RE = /\/\/\s*obligation-precheck\s+OB-004:\s*(\S[^\n]*)/

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
 * `ROWS` 里声明的行名，连带**这一行接没接执行体**（`by: Some(…)`）。
 *
 * 只取 `static ROWS: &[Decl] = &[ … ]` 那一块：同一文件里还有 `ACTORS`（执行体名）
 * 与端口声明（端口名），它们的 `name:` 长得一模一样，整文件抓会把「端口名」
 * 当成合法行名——于是 OB-003 会被一个**不是行**的名字满足。
 *
 * `wired` 的取法：`name:` 之后、下一个 `name:` 之前出现 `by: Some(` 才算接了执行体。
 * OB-004 用它区分「执行体动手前的复核」与「这一行唯一的派发点」。
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
  const rows = new Map()
  const marks = [...body.matchAll(/\bname:\s*"([^"]+)"/g)]
  for (let i = 0; i < marks.length; i++) {
    const from = marks[i].index
    const to = i + 1 < marks.length ? marks[i + 1].index : body.length
    rows.set(marks[i][1], { wired: /\bby:\s*Some\(/.test(body.slice(from, to)) })
  }
  return rows
}

/** `drive_steps` 的函数体区间（`[start, end)`）——派发器本体问债不算旁路 */
function driveStepsSpan(text) {
  const m = /fn\s+drive_steps\b/.exec(text)
  if (!m) return null
  const open = text.indexOf('{', m.index)
  if (open < 0) return null
  return [m.index, matchBrace(text, open) + 1]
}

/**
 * 引擎的现读点（`Dispatch::row_owed`）函数体区间——同 `drive_steps` 一条理由。
 *
 * D-136 之后「派发前问表」只剩这一处，它问的行名来自 `Dispatch` 字面量（声明），
 * 不是调用点自己算出来的布尔值。把它算成旁路会让基线虚高，而它又不是任何一处
 * 「绕开 `drive` 的旁路代码」。它的行名改由下面那半边 OB-003 核对。
 */
function engineReadSpan(text) {
  const m = /fn\s+row_owed\b/.exec(text)
  if (!m) return null
  const open = text.indexOf('{', m.index)
  if (open < 0) return null
  return [m.index, matchBrace(text, open) + 1]
}

/** `Dispatch { row: "…" }` 的字面点名（引擎现读的行名从这里来，也要真实存在） */
function declaredRowNames(lines) {
  const out = []
  for (let i = 0; i < lines.length; i++) {
    const m = /\brow:\s*"([^"]*)"/.exec(lines[i])
    if (m) out.push({ line: i + 1, row: m[1], src: lines[i].trim().slice(0, 90) })
  }
  return out
}

/** 承认标记的回看窗口：同行 + 其上 3 行（与 R-002 / R-001 同一口径） */
const MARK_LOOKBACK = 3

/**
 * 把全仓扫到的承认标记**贴到直调点上**（读的是原文行号，标记本身就是注释）。
 *
 * ⚠️ 一个标记只认**离它最近的那一个**直调点。按「窗口内所有点都算被承认」实现会
 * 造出一个比漏报更坏的形状：一句「这是护栏」同时洗白它下面三行里的每一处直问，
 * 而其中多半根本是另一件事。多出来的、没被采用的标记随后按悬空判红。
 */
function attachMarks(sites, marks) {
  for (const s of sites) s.mark = null
  const used = new Set()
  /** 已被采用的点 → 采用它的那条承认（后来者只有更近才顶得掉） */
  const taken = new Map()
  for (const mk of marks) {
    let best = null
    for (const s of sites) {
      if (s.file !== mk.file || mk.line > s.line || s.line - mk.line > MARK_LOOKBACK) continue
      const dist = s.line - mk.line
      if (best === null || dist < best.dist) best = { site: s, dist }
    }
    if (!best) continue
    const prev = taken.get(best.site)
    if (prev && prev.dist <= best.dist) continue
    if (prev) used.delete(prev.mk)
    taken.set(best.site, best)
    best.site.mark = mk
    used.add(mk)
  }
  return used
}

console.log('--- obligation-bypass-audit: 派发面（OB-001…004：调度旁路 / 幂等点问） ---')

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

/** 直调点：`owes(` 的调用（定义行不算），带行名与承认标记 */
const sites = []
const named = []
/** 全仓扫到的承认标记（OB-004 的悬空核对用） */
const marks = []
let definitionLines = 0
for (const file of files) {
  const raw = fs.readFileSync(file, 'utf8')
  const rawLines = raw.replace(/\r\n/g, '\n').split('\n')
  const lines = blankComments(raw)
  const text = lines.join('\n')
  const spans = [driveStepsSpan(text), engineReadSpan(text)].filter(Boolean)
  for (let j = 0; j < rawLines.length; j++) {
    const m = PRECHECK_MARK_RE.exec(rawLines[j])
    if (m) marks.push({ file, line: j + 1, reason: m[1].trim() })
  }
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
        if (spans.some(([start, end]) => at >= start && at < end)) continue
        // 行名 = 实参里第一个字符串字面量（多行调用也要抓到）
        const tail = text.slice(at, at + 300)
        const lit = /"([^"]*)"/.exec(tail)
        sites.push({
          file,
          line: i + 1,
          row: lit ? lit[1] : null,
          src: line.trim().slice(0, 90),
          mark: null,
        })
      }
    }
    offset += line.length + 1
  }
  for (const n of declaredRowNames(lines)) named.push({ ...n, file })
}

/** 标记 → 直调点的归属（一个标记只认最近的那一点）；未被采用的标记下面按悬空判红 */
const usedMarks = attachMarks(sites, marks)

// ── OB-003：行名必须存在 ────────────────────────────────────────────
let errors = 0
const check = (rel, line, row, src, why) => {
  if (rows.has(row)) return
  errors++
  console.log(
    red(
      `[ERROR] OB-003 ${rel}:${line} ${why}而 \`ROWS\` 里没有这一行 ⇒ ` +
        `恒 false（编译、测试、门禁全绿，账却永远不开或永远不派）`,
    ),
  )
  console.log(`       ${src}`)
}
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
  check(rel, s.line, s.row, s.src, `直调 \`owes("${s.row}")\``)
}
// 引擎现读的行名来自 `Dispatch { row: "…" }` 字面量（D-136）：点错一行，
// 那一格就永远派不出去——与旁路点错行是同一个失效形状，用同一半边判据核。
for (const n of named) {
  check(path.relative(ROOT, n.file).replace(/\\/g, '/'), n.line, n.row, n.src, `派发点名 \`row: "${n.row}"\``)
}

// ── OB-001 / OB-002 / OB-004：两类计数各自的棘轮 ──────────────────────
/** 未承认的直调点 = **调度旁路**（归零对象）；带 OB-004 标记的 = **幂等点问**（保留、有上限） */
const bypass = sites.filter((s) => !s.mark)
const precheck = sites.filter((s) => s.mark)

/** 理由判空与 R-002 同一口径：至少要有一个字母 / 数字 / 汉字，标点不算内容 */
const REASON_HAS_CONTENT = /[A-Za-z0-9一-鿿]/

for (const [name, value] of [
  ['BYPASS_BASELINE', BYPASS_BASELINE],
  ['PRECHECK_BASELINE', PRECHECK_BASELINE],
]) {
  if (!Number.isFinite(value)) {
    errors++
    console.log(red(`[ERROR] OB-001 基线 ${name} 不是数字（${value}）—— 判据读不出即失效，宁红不静默`))
  }
}

if (bypass.length > BYPASS_BASELINE) {
  errors++
  console.log(
    red(
      `[ERROR] OB-001 派发面变宽：调度旁路（未承认的直调 \`owes()\`）点数 ${bypass.length} > 基线 ${BYPASS_BASELINE} —— ` +
        `环的派发面只许收窄（GO-003 E5）。多出来的点要么接回 \`drive\`（加 \`ActorEntry\`），` +
        `要么说明为什么它不能接回并同步抬高基线（要 \`35-baseline\` 那一格留痕）。` +
        `**注意「接回」的前件是外环收编（\`D-158\`）**：\`by: None\` 的策略行一接执行体，内环就会按 \`prio\` 选中它而拿不到轮首上下文`,
    ),
  )
  for (const s of bypass) {
    const rel = path.relative(ROOT, s.file).replace(/\\/g, '/')
    console.log(dim(`       ${rel}:${s.line}  ${s.row ?? '?'}`))
  }
}

// ── OB-004：幂等点问的承认必须站得住 ─────────────────────────────────
if (precheck.length > PRECHECK_BASELINE) {
  errors++
  console.log(
    red(
      `[ERROR] OB-004 幂等点问变宽：承认在册的直问 ${precheck.length} > 基线 ${PRECHECK_BASELINE} —— ` +
        `护栏可以保留，但不能随手加：那等于多开一条不用过外环收编的逃生通道。` +
        `抬高它要走 \`35-baseline\` 的 \`obligation-precheck\` 那一格留痕`,
    ),
  )
}
for (const s of precheck) {
  const rel = path.relative(ROOT, s.file).replace(/\\/g, '/')
  // 标记的**另一半**：被复核的行本身得有执行体。`by = None` 的行唯一的派发点就是这一处
  // 直问，把「护栏」写在它上面只是给调度旁路洗白——OB-004 之所以有牙，牙在这里。
  const wired = s.row ? rows.get(s.row)?.wired : undefined
  if (wired === false) {
    errors++
    console.log(
      red(
        `[ERROR] OB-004 ${rel}:${s.line} 标记贴在了未接执行体的行 \`"${s.row}"\`（\`by = None\`）上 ⇒ ` +
          `这不是「动手前复核」，是这一行**唯一的派发点**：调度旁路洗白（归零对象不许用注释了结）`,
      ),
    )
    console.log(`       ${s.src}`)
  } else if (wired === undefined && s.row) {
    // 行名不存在时 OB-003 已经报错，这里只补一句「所以 OB-004 也判不了」，不重复列点
    console.log(dim(`       OB-004 ${rel}:${s.line} 的行名核对依赖 OB-003（上面已报）`))
  }
}
// 理由有内容、标记不悬空：这两半判的是**承认账本本身**，与它贴在哪个点上无关
for (const mk of marks) {
  const rel = path.relative(ROOT, mk.file).replace(/\\/g, '/')
  if (!REASON_HAS_CONTENT.test(mk.reason)) {
    errors++
    console.log(
      red(
        `[ERROR] OB-004 ${rel}:${mk.line} 承认理由为空（\`${mk.reason}\`）⇒ ` +
          `标记变成装饰：至少要写清「防的是哪一次重复进入」（判空口径同 R-002）`,
      ),
    )
  }
  if (!usedMarks.has(mk)) {
    errors++
    console.log(
      red(
        `[ERROR] OB-004 ${rel}:${mk.line} 承认标记悬空：其下 ${MARK_LOOKBACK} 行内没有**由它采用**的直调点` +
          `（没有直调点，或那一点已有更近的承认）⇒ 删掉它，或把它挪到真护栏旁边`,
      ),
    )
  }
}

const brief = (list) => {
  const byRow = new Map()
  for (const s of list) byRow.set(s.row ?? '?', (byRow.get(s.row ?? '?') ?? 0) + 1)
  return [...byRow.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([r, n]) => `${r}×${n}`)
    .join(' · ')
}

if (errors) {
  console.log(
    `obligation-bypass-audit 未通过：${errors} 处（OB-001 调度旁路 / OB-003 行名 / OB-004 承认）；` +
      `扫 ${files.length} 份 .rs，调度旁路 ${bypass.length} 处 / 基线 ${BYPASS_BASELINE}，` +
      `幂等点问 ${precheck.length} 处 / 基线 ${PRECHECK_BASELINE}`,
  )
  process.exit(1)
}

console.log(
  green(
    `obligation-bypass-audit 通过：扫 ${files.length} 份 .rs，` +
      `调度旁路 ${bypass.length}/${BYPASS_BASELINE}（${brief(bypass) || '无'}）· ` +
      `幂等点问 ${precheck.length}/${PRECHECK_BASELINE}（${brief(precheck) || '无'}）；` +
      `${rows.size} 行声明表内点名全部存在`,
  ),
)
if (bypass.length < BYPASS_BASELINE) {
  console.log(
    yellow('提示') +
      `：调度旁路已收到基线下方（${bypass.length}<${BYPASS_BASELINE}），请同步本脚本的 BYPASS_BASELINE——` +
      `留着旧数字等于把余量白留给下一次`,
  )
}
if (precheck.length < PRECHECK_BASELINE) {
  console.log(
    yellow('提示') +
      `：幂等点问已收到基线下方（${precheck.length}<${PRECHECK_BASELINE}），请同步 OBLIGATION_PRECHECK_BASELINE`,
  )
}
if (definitionLines > 0) {
  console.log(dim(`（排除的定义行 ${definitionLines} 处不计）`))
}
if (VERBOSE) {
  for (const [label, list] of [
    ['调度旁路', bypass],
    ['幂等点问', precheck],
  ]) {
    console.log(`  ── ${label} ──`)
    for (const s of list) {
      const rel = path.relative(ROOT, s.file).replace(/\\/g, '/')
      console.log(`  ${rel}:${s.line}  ${s.row ?? '?'}${s.mark ? `  承认：${s.mark.reason}` : ''}`)
    }
  }
}
