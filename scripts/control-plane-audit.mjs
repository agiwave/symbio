#!/usr/bin/env node
/**
 * control-plane-audit — 控制面终态的**执行方**（`SP-033` 的 `TC-1`…`TC-6`）
 *
 * ## 它治什么
 *
 * [`SP-033`](../docs/005-spec/033.md "SP-033") §3 写了六条不变量，每条都配了
 * 「断言什么 / 为什么它是终态 / 反向用例」。但六条到本脚本落地之前**没有执行方**：
 * §3 的「今天」那一行逐条写着「形状已成立、执行方未落地」，`## 判据` 表的后两列
 * 是「落地后补……」——一份只写在规范里的判据，在形状退化时不会出声。
 *
 * 十次形状退化里，九次不是「有人决定改坏」，而是**顺手**：多写一段推进循环、
 * 在派发器外直问一次求解器、给某个执行体的动作多塞一份 `&mut`、在动作里就地落
 * 收束格。它们每一件都只多几十行，而判据面各自看不见——这正是 `TC-n` 要判的东西。
 *
 * ## 八条判据码（每条都指得到规范的一句）
 *
 * | 码 | 规范 | 判什么 |
 * |---|---|---|
 * | `CP-001` | `TC-1a` 循环体唯一 | 驱动循环只有一个；选债调用只有一个且在其中 |
 * | `CP-002` | `TC-1b` 选债口唯一 | 求解器只在 `loop_rows.rs` 被问；派发器不点问行 |
 * | `CP-003` | `TC-1c` 守卫同处 | 五道守卫各一份实现、各一个调用点；不重入由类型守 |
 * | `CP-004` | `TC-2` 机制层无域名 | 行名 / 执行体名 / 端口名 / 事件名不进 kernel 与 core |
 * | `CP-005` | `TC-4` 事实面唯一写方 | 收束格（final / fallback）各只有一个构造点 |
 * | `CP-006` | `TC-5` 横向扩展零改机制 | 机制面（驱动循环 / 选债 / 在途判据）字节冻结，控制面文件数只许降，端口不越出裁决的闭集 |
 * | `CP-007` | `TC-6` 判据集闭集 | 六条不变量与八条码一一对上；判据数不随能力数增长 |
 * | `CP-008` | `TC-3` 声明面封闭 | `seal` 是按值消费的唯一出口；冻结后没有可变方法 |
 *
 * ## 口径（四条，都影响判定，别凭直觉）
 *
 * 1. **输入是源码与规范文本，不是另一条判据的输出**（`SP-033` §3 纪律 3）：
 *    `CP-004` 的词表取自**行 / 执行体的声明处**（`loop_rows.rs`），不是手写清单——
 *    手写清单会在加一行时忘记同步，于是判据对新增的名字天生瞎。
 * 2. **断言的对象必须在终态存在**（`J0`）：不判「host 份数」「绑定表条数」「旁路点
 *    计数」这类过渡构件的数量。`CP-001` 判的是「推进形状只有一份」，不是「今天有几段」。
 * 3. **测试不是生产**：`.test.rs` / `tests.rs` / `#[cfg(test)] mod` 里造一条收束格
 *    是排演，不是第二个写方。扫描统一走 `blankTestModules`（保留行号）。
 * 4. **判据不许读结论**：`CP-006` 的基线是**输入**（字节指纹），与代码解耦；
 *    它可以被 `--print-baseline` 重算，但改它要挂一条决策（`CP-006` 自己判那条决策
 *    存不存在）。
 *
 * ## 已知边界（写清楚，免得把绿灯当成证明）
 *
 * - 全部判定基于**剥注释后的文本**，不是 AST。`CP-006` 因此**只改注释不算改机制**
 *   （散文不是算法），而改函数体里任何一行都红。
 * - `CP-001` 认的是调用形状（`loop_rows::next(` / `ring::drive(`）。把选债藏进一个
 *   同名包装函数再调用，本条看不见——但那样 `CP-002` 的「`ring::drive` 只在一处」
 *   仍是绿的，而**选债口并没多**：包装函数如果不绕开循环体，它就不是第二个口。
 *   真正的退化形态（另起一段推进）需要第二处 `loop_rows::next(`，本条判得住。
 * - `CP-003` 判「五道守卫各一份」。守卫的**语义**对不对由各自的用例判（`D-118` 决策 7
 *   的四种处境、`D-123` 的账本口径各有用例），本条只判「有没有人绕过它另写一份」。
 * - `CP-004` 只查**字面量**。经常量绕过的端口名看不见，那一半归 `PTS-001` / `PTS-002`
 *   （端口名与取值的闭集与引擎无感）。
 * - `CP-005` 只查 `Event::pending(` 的名字实参。手搓 `Event` 结构体字面量在类型上合法
 *   而本条看不见——这与 `PTS-005` 是同一条边界（要关掉它得先收窄 `Event` 字段）。
 * - `CP-007` 是元判据：它判的是**判据集自己**（六条 ↔ 八码 ↔ 规范表），判不出某条码
 *   内部是否恒真。恒真由 `control-plane-audit.test.mjs` 逐条注入反向用例来钉。
 * - `CP-006` 的端口闭集读的是 `D-222` §一里那句「今天只有…端口：…」，**不是**本脚本的
 *   一行常量——抄一份名单就是第二个 owner。改那句话 = 改裁决，判据跟着走；散文里的
 *   候选表（时钟 / 存储为什么被排除）不参与判定。它判「有没有越出闭集」，判不到
 *   「新端口是不是真往返」——那要读 `awaited` / `max_attempts` / 应答载荷，归 `D-222` §一
 *   的判定口径，不是本脚本能替的。
 *
 * ## 用法
 *
 *   node scripts/control-plane-audit.mjs                  # 判定 + 读数
 *   node scripts/control-plane-audit.mjs --print-baseline  # 打印机制面指纹（改机制后据此更新基线）
 *   node scripts/control-plane-audit.mjs --root=<dir>      # 指向别处的仓库（回归测试用）
 *
 * 退出码：0 = 通过；1 = 有违规；2 = 范围读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns CP 控制面终态（唯一循环体 / 唯一选债口 / 守卫同处 / 机制层无域名 / 收束格单写方 / 机制面冻结与端口闭集 / 判据集闭集 / 声明面封闭）
// @codes CP-001 CP-002 CP-003 CP-004 CP-005 CP-006 CP-007 CP-008

import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { red, green, dim, yellow } from './color.mjs'
import { blankComments, blankTestModules, matchBrace, skipString } from './rust-scan.mjs'
import { stringLiteralsOf } from './gate.d/_shared.mjs'

const argv = process.argv.slice(2)
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const PRINT_BASELINE = argv.includes('--print-baseline')

/** 控制面（编排层）与机制层（kernel / core）的根 */
const LOOP = 'crates/symbio-plugin-loop/src'
const KERNEL = 'crates/symbio-kernel/src'
const CORE = 'crates/symbio-core/src'
const DRIVE_REL = `${LOOP}/turn_exec/drive.rs`
const ROWS_REL = `${LOOP}/loop_rows.rs`
const TURN_EXEC_REL = `${LOOP}/turn_exec.rs`
const SETTLE_REL = `${LOOP}/turn_runner/settle.rs`
const DECL_REL = `${KERNEL}/decl/mod.rs`
const ADAPTERS_REL = `${KERNEL}/adapters.rs`
const SPEC_REL = 'docs/005-spec/033.md'
/** 端口闭集的 owner：`D-222` §一。闭集名单**不在本脚本里**——抄一份就是第二个 owner */
const RULING_REL = 'docs/003-design/015-义务环.md'
const BASELINE_REL = 'scripts/baselines/control-plane.json'

/**
 * 六条不变量的条数。**这是判据面的一部分**（`TC-6`：「判据条数不随能力数增长」）：
 * 加一条要改这里，而改这里是可见的一行 diff，不是「顺手给新能力补一条专用判据」。
 */
const TERMINAL_CRITERIA = 6

const CODES = ['CP-001', 'CP-002', 'CP-003', 'CP-004', 'CP-005', 'CP-006', 'CP-007', 'CP-008']

let errors = 0
const hits = new Map()
function report(code, where, message) {
  console.log(`${red('[ERROR]')} ${code} ${where}  ${message}`)
  errors++
  hits.set(code, (hits.get(code) ?? 0) + 1)
}

// ───────────────────────────── 读入原语 ─────────────────────────────

function abs(rel) {
  return path.join(ROOT, rel)
}

function readRel(rel) {
  const file = abs(rel)
  if (!fs.existsSync(file)) return null
  return fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n')
}

/** 剥注释 + 抹掉 `#[cfg(test)] mod` 体，**保留行号**（报告里的位置对得上原文件） */
function prodSource(text) {
  return blankTestModules(blankComments(text).join('\n'))
}

/** 一份生产文件的代码文本；读不出 ⇒ null */
function prodCodeOf(rel) {
  const text = readRel(rel)
  return text === null ? null : prodSource(text)
}

/**
 * 文件是不是**测试**：`.test.rs` / `tests.rs` / `#[cfg(test)] mod <stem>;`。
 *
 * 与 `core-surface.mjs::isTestFile` 同口径，多认一处**仓根 `lib.rs`**：`src/x.rs` 的
 * 父模块声明在 `lib.rs` 里，而 `src/mod.rs` / `src.rs` 都不存在——只查后两者的实现
 * 会把 `test_support.rs` 读成生产文件（那是 `#[cfg(test)] mod test_support;`）。
 */
function isTestFile(file) {
  if (file.endsWith('.test.rs') || path.basename(file) === 'tests.rs') return true
  const stem = path.basename(file, '.rs')
  const dir = path.dirname(file)
  const candidates = [path.join(dir, 'mod.rs'), `${dir}.rs`]
  if (path.basename(dir) === 'src') candidates.push(path.join(dir, 'lib.rs'))
  for (const parent of candidates) {
    if (!fs.existsSync(parent)) continue
    const lines = fs.readFileSync(parent, 'utf8').replace(/\r\n/g, '\n').split('\n')
    const decl = new RegExp(`^\\s*(?:pub(?:\\([^)]*\\))?\\s+)?mod\\s+${stem}\\s*;`)
    for (let i = 0; i < lines.length; i++) {
      if (!decl.test(lines[i])) continue
      for (let j = i - 1; j >= 0 && j >= i - 4; j--) {
        const line = lines[j].trim()
        if (!line || line.startsWith('//')) continue
        return line.includes('#[cfg(test)]')
      }
      return false
    }
  }
  return false
}

/** 递归列生产 `.rs`（排除测试） */
function* prodFiles(dirRel) {
  const dir = abs(dirRel)
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const rel = `${dirRel}/${e.name}`
    if (e.isDirectory()) yield* prodFiles(rel)
    else if (e.isFile() && e.name.endsWith('.rs') && !isTestFile(abs(rel))) yield rel
  }
}

/** `crates/` 下全部生产 `.rs`（`CP-002` / `CP-004` 要跨 crate 看） */
function* allCratesProd() {
  const base = abs('crates')
  if (!fs.existsSync(base)) return
  for (const e of fs.readdirSync(base, { withFileTypes: true })) {
    if (e.isDirectory()) yield* prodFiles(`crates/${e.name}`)
  }
}

/** 计数：文本里某个模式的命中数（含位置） */
function sitesOf(code, re) {
  const out = []
  for (const m of code.matchAll(re)) out.push({ index: m.index, text: m[0] })
  return out
}

/**
 * `fn <name>` 的**定义**（含泛型参数：`async fn foo<S>(`）。
 *
 * 定义行不算「调用」——把定义算进调用次数，会让「唯一调用点」的判据天生多一格。
 */
function defSites(code, name) {
  return sitesOf(code, new RegExp(`\\bfn\\s+${name}\\s*(?:<[^>]*>\\s*)?\\(`, 'g'))
}

/** 调用点 = 命中 − 定义行 */
function callSites(code, name) {
  const defs = defSites(code, name).map((d) => d.index)
  return sitesOf(code, new RegExp(`\\b${name}\\s*(?:<[^>]*>\\s*)?\\(`, 'g')).filter((s) => {
    // 定义行的命中位置比 `fn ` 晚几个字符：按「同一处」判，用定义命中的 index 近似即可
    return !defs.some((d) => s.index >= d - 8 && s.index <= d + 8)
  })
}

/** 取 `openIdx`（指向 `(`）到配对 `)` 之间的文本（字符串感知） */
function parenBody(code, openIdx) {
  let depth = 0
  for (let i = openIdx; i < code.length; i++) {
    const ch = code[i]
    if (ch === '"') {
      i = skipString(code, i)
      continue
    }
    if (ch === '(') depth++
    else if (ch === ')') {
      depth--
      if (depth === 0) return { start: openIdx + 1, end: i }
    }
  }
  return { start: openIdx + 1, end: code.length }
}

/** 顶层实参（`Event::pending(` 的第二个实参是名字） */
function topLevelArgs(code, openIdx) {
  const { end } = parenBody(code, openIdx)
  const args = []
  let depth = 0
  let cur = ''
  for (let i = openIdx + 1; i < end; i++) {
    const ch = code[i]
    if (ch === '"') {
      const close = skipString(code, i)
      cur += code.slice(i, close + 1)
      i = close
      continue
    }
    if (ch === '(' || ch === '[' || ch === '{') depth++
    if (ch === ')' || ch === ']' || ch === '}') depth--
    if (ch === ',' && depth === 0) {
      args.push(cur)
      cur = ''
      continue
    }
    cur += ch
  }
  if (cur.trim()) args.push(cur)
  return args.map((a) => a.trim())
}

/** `fn <name>` 的函数体（剥注释后的文本里），找不到 ⇒ null */
function fnBody(code, name) {
  const m = new RegExp(`\\bfn\\s+${name}\\s*\\(`).exec(code)
  if (!m) return null
  const open = code.indexOf('{', m.index)
  if (open < 0) return null
  const close = matchBrace(code, open)
  return { source: code.slice(m.index, close + 1), start: m.index, end: close + 1 }
}

/** 某个块（`{` 在 `openIdx`）的文本 */
function blockBody(code, openIdx) {
  return { start: openIdx, end: matchBrace(code, openIdx) + 1 }
}

// ───────────────────────────── 读入声明处 ─────────────────────────────

const rowsCode = prodCodeOf(ROWS_REL)
const driveCode = prodCodeOf(DRIVE_REL)
if (rowsCode === null || driveCode === null) {
  console.error(red(`✗ 范围读不出：${ROWS_REL} 或 ${DRIVE_REL} 不存在（root=${ROOT}）`))
  process.exit(2)
}

/** 行名（`Decl { name: "…" }`）——词表的 owner 在行表，**不在本脚本** */
function rowNames() {
  const out = []
  for (const m of rowsCode.matchAll(/\bDecl\s*\{/g)) {
    const body = blockBody(rowsCode, rowsCode.indexOf('{', m.index))
    const name = /\bname:\s*"([^"]+)"/.exec(rowsCode.slice(body.start, body.end))
    if (name) out.push(name[1])
  }
  return out
}

/**
 * 执行体名：`ActorDecl { name: <CONST> }` 里被点名的常量 → 它的字面值。
 *
 * 不取「所有 `const X: &str`」：同一份文件里还有插件 id 之类的常量（`"plugins/loop"`），
 * 把它们算进词表会让判据去机制层找一串本来就该出现的路径。
 */
function actorNames() {
  const consts = new Set()
  for (const m of rowsCode.matchAll(/\bActorDecl\s*\{/g)) {
    const body = blockBody(rowsCode, rowsCode.indexOf('{', m.index))
    const name = /\bname:\s*([A-Z][A-Z0-9_]*)/.exec(rowsCode.slice(body.start, body.end))
    if (name) consts.add(name[1])
  }
  const out = []
  for (const c of consts) {
    const m = new RegExp(`const\\s+${c}\\s*:\\s*&str\\s*=\\s*"([^"]+)"`).exec(rowsCode)
    if (m) out.push(m[1])
  }
  return out
}

/** 端口名（`PortDecl { port: "…" }`） */
function portNames() {
  const out = []
  for (const m of rowsCode.matchAll(/\bPortDecl\s*\{/g)) {
    const body = blockBody(rowsCode, rowsCode.indexOf('{', m.index))
    const name = /\bport:\s*"([^"]+)"/.exec(rowsCode.slice(body.start, body.end))
    if (name) out.push(name[1])
  }
  return out
}

const ROW_NAMES = rowNames()
const ACTOR_NAMES = actorNames()
const PORT_NAMES = portNames()

/**
 * 全仓生产代码里**声明**的端口：`PortDecl { port: "…" }`，跨 crate 看。
 *
 * 不从 `rowsCode` 里取（`portNames()` 只读行表）：闭集判的是「有没有人在行表之外
 * 新开一个端口」，只读行表等于只判自己。
 */
function declaredPorts() {
  const out = []
  for (const rel of allCratesProd()) {
    const code = prodCodeOf(rel)
    if (code === null) continue
    for (const m of code.matchAll(/\bPortDecl\s*\{/g)) {
      // `pub struct PortDecl {` 是**类型定义**，不是一条声明
      if (/\bstruct\s+$/.test(code.slice(Math.max(0, m.index - 12), m.index))) continue
      const body = blockBody(code, code.indexOf('{', m.index))
      const name = /\bport:\s*"([^"]+)"/.exec(code.slice(body.start, body.end))
      if (name) out.push({ port: name[1], rel })
    }
  }
  return out
}

/**
 * 裁决里那张「端口闭集」点名的端口（owner：`DE-015` `D-222` §一）。
 *
 * 读不出不是「没有约束」，是**判据没有输入**——调用方按红处置（宁红不静默）。
 * 只认那一句「今天只有…端口：`a`、`b`」：散文里的候选表（时钟 / 存储）不参与判定。
 */
function rulingPorts() {
  const text = readRel(RULING_REL)
  if (text === null) return { error: `读不出 \`${RULING_REL}\`` }
  const at = text.indexOf('#### 一、端口闭集')
  if (at < 0) return { error: '找不到「#### 一、端口闭集」' }
  // 从这一**行之后**找下一个标题：从 `at + 1` 起找会把本行自己认成下一个标题
  const rest = text.slice(text.indexOf('\n', at) + 1)
  const next = rest.search(/^#{1,6}\s/m)
  const section = next < 0 ? rest : rest.slice(0, next)
  const line = section.split('\n').find((l) => l.includes('今天') && /端口[：:]/.test(l))
  if (!line) return { error: '「端口闭集」里找不到那句「今天只有…端口：…」' }
  const ports = [...line.matchAll(/`([^`]+)`/g)].map((m) => m[1])
  if (ports.length === 0) return { error: '「端口闭集」那句话里没有点名任何端口' }
  return { ports }
}

/**
 * `CP-004` 的词表 = **行名 + 执行体名**。
 *
 * 端口名不在这里：`model` / `tool` 同时也是能力族与工具调用的通用词，把它们算成
 * 「业务名字泄漏」会让判据一上线就红在十条无关的字面量上——端口名由 `PTS-002`
 * （引擎不读端口字段 / 取值）守，本节只补它没覆盖的那一半（`SP-033` §「判据」表）。
 */
const VOCABULARY = [...new Set([...ROW_NAMES, ...ACTOR_NAMES])].sort()

// ───────────────────── CP-001：唯一循环体（TC-1a） ─────────────────────

const loops = sitesOf(driveCode, /\bloop\s*\{/g)
let driveLoop = null
if (loops.length !== 1) {
  report(
    'CP-001',
    DRIVE_REL,
    `驱动文件里有 ${loops.length} 个 \`loop {\`（应恰好 1 个）—— 控制面只有**一个**「选债 → 还债 → 读尾段对账」的循环体；` +
      `另起一段推进哪怕只是一次性代码，也不再有唯一的守卫同处（SP-033 TC-1a）`,
  )
} else {
  driveLoop = blockBody(driveCode, driveCode.indexOf('{', loops[0].index))
}

const nextCalls = []
for (const rel of prodFiles(LOOP)) {
  const code = prodCodeOf(rel)
  for (const s of sitesOf(code, /\bloop_rows::next\s*\(/g)) nextCalls.push({ rel, index: s.index })
}
if (nextCalls.length !== 1) {
  report(
    'CP-001',
    'symbio-plugin-loop/',
    `选债调用 \`loop_rows::next(\` 有 ${nextCalls.length} 处（${nextCalls.map((c) => c.rel).join(' / ') || '无'}）—— ` +
      `应恰好 1 处：任何「现在该还哪笔债」的判定都只能由那个循环体做出（SP-033 TC-1b）`,
  )
} else {
  const only = nextCalls[0]
  if (only.rel !== DRIVE_REL) {
    report('CP-001', only.rel, `唯一的选债调用不在驱动循环里（${DRIVE_REL}）—— 循环体与选债口必须是同一个地方`)
  } else if (driveLoop && (only.index < driveLoop.start || only.index > driveLoop.end)) {
    report('CP-001', DRIVE_REL, `选债调用落在 \`loop {\` 之外 —— 它不是那个循环体的一部分`)
  }
}

// ───────────────────── CP-002：唯一选债口（TC-1b） ─────────────────────

const driveSolverCalls = []
for (const rel of allCratesProd()) {
  const code = prodCodeOf(rel)
  for (const s of sitesOf(code, /\bring::drive\s*\(/g)) driveSolverCalls.push({ rel, index: s.index })
}
/**
 * 直问求解器（`owed(…)`）只在**控制面**里数：`ring.rs` 自己的 `drive` 调 `owed` 是
 * 求解器的内部实现，不是第二处选债口——把它算进来，判据会为了躲开内核而收窄射程。
 */
const owedCallSites = []
for (const rel of prodFiles(LOOP)) {
  const code = prodCodeOf(rel)
  for (const s of sitesOf(code, /\bowed\s*\(/g)) {
    const before = code.slice(Math.max(0, s.index - 8), s.index)
    if (/\bfn\s+$/.test(before)) continue // 定义行
    owedCallSites.push({ rel, index: s.index })
  }
}

if (driveSolverCalls.length !== 1) {
  report(
    'CP-002',
    'crates/',
    `\`ring::drive(\` 有 ${driveSolverCalls.length} 处（${driveSolverCalls.map((c) => c.rel).join(' / ') || '无'}）—— ` +
      `求解器只许由 \`${ROWS_REL}\` 的选债包装问一次；第二处就是第二个选债口（SP-033 TC-1b）`,
  )
} else if (driveSolverCalls[0].rel !== ROWS_REL) {
  report('CP-002', driveSolverCalls[0].rel, `\`ring::drive(\` 不在 ${ROWS_REL} —— 选债口搬走了，派发器的唯一输入断了一处`)
}

const outsideRows = owedCallSites.filter((c) => c.rel !== ROWS_REL)
for (const c of outsideRows) {
  report(
    'CP-002',
    c.rel,
    '在 `loop_rows.rs` 之外直接问求解器（`owed(…)`）—— **派发器之外直问求解器再自行派发就是第二个选债口**，' +
      '即使它不是一段 `loop`（SP-033 TC-1b 的原文）',
  )
}
if (owedCallSites.filter((c) => c.rel === ROWS_REL).length !== 1) {
  report(
    'CP-002',
    ROWS_REL,
    `求解器在 \`${ROWS_REL}\` 里被问了 ${owedCallSites.filter((c) => c.rel === ROWS_REL).length} 次（应恰好 1 次：\`fn owes\` 的点问）—— ` +
      '多出来的那一处要么是第二个选债口，要么是没人用的死代码',
  )
}

/** 幂等点问（`owes`）是**读数**不是判据：`OB-002` / `OB-004` 已按 `J0` 退场为读数 */
const pointQueries = []
for (const rel of prodFiles(LOOP)) {
  if (rel === ROWS_REL) continue
  const code = prodCodeOf(rel)
  for (const s of sitesOf(code, /\bowes\s*\(/g)) pointQueries.push({ rel, index: s.index })
}
for (const q of pointQueries) {
  if (q.rel.startsWith(`${LOOP}/turn_exec/`)) {
    report(
      'CP-003',
      q.rel,
      '派发器里出现了点问（`owes(`）—— 派发器只许调选债口给出**那一个**动作，不许按行名再问一次' +
        '（D-118 决策 1–2；SP-033 TC-1c 守卫五）',
    )
  }
}

// ───────────────────── CP-003：五道守卫同处（TC-1c） ─────────────────────

const GUARD_FNS = [
  'driver_aborted',
  'abort_before_first_request',
  'over_step_limit',
  'waiting',
  'writes_closed',
]
const guardDefs = new Map()
for (const rel of prodFiles(LOOP)) {
  const code = prodCodeOf(rel)
  for (const name of GUARD_FNS) {
    const n = defSites(code, name).length
    if (n > 0) guardDefs.set(name, [...(guardDefs.get(name) ?? []), { rel, n }])
  }
}
for (const name of GUARD_FNS) {
  const defs = guardDefs.get(name) ?? []
  const total = defs.reduce((n, d) => n + d.n, 0)
  if (total !== 1) {
    report(
      'CP-003',
      'symbio-plugin-loop/',
      `守卫实现 \`fn ${name}\` 有 ${total} 份（${defs.map((d) => d.rel).join(' / ') || '无'}）—— ` +
        '五道守卫（中止 / 步数上界 / 空账本 / 写封闭 / 不重入）各只许一份实现，各只许在唯一的循环体里被调用（SP-033 TC-1c）',
    )
  }
}

/** 五道守卫的**调用点**标记：各一次，且都在驱动文件里 */
const GUARD_MARKERS = [
  ['driver_aborted', 'driver_aborted'],
  ['abort_before_first_request', 'abort_before_first_request'],
  ['over_step_limit', 'over_step_limit'],
  ['writes_closed', 'writes_closed'],
  ['ledger.is_empty', null],
]
for (const [label, name] of GUARD_MARKERS) {
  const n = name
    ? callSites(driveCode, name).length
    : sitesOf(driveCode, new RegExp(`\\b${label.replace('.', '\\.')}\\s*\\(`, 'g')).length
  if (n !== 1) {
    report(
      'CP-003',
      DRIVE_REL,
      `守卫「${label}」在驱动文件里有 ${n} 个调用点（应恰好 1 个）—— ` +
        '少一个 = 有一条推进路径不再检查它；多一个 = 同一道守卫两处写法必然分叉（SP-033 TC-1c）',
    )
  }
}

/**
 * 第五道「不重入」：**由类型守**，不由 `if` 守。
 *
 * 判据判的是「签名给不给得出第二份 `&mut`」：动作只拿得到一个 `&mut Ctx`，
 * `Ctx` 里只有一个 `&mut` 字段（本轮草稿）。任何一处多出可变借用，签名就多给一分权力。
 */
const turnExecCode = prodCodeOf(TURN_EXEC_REL)
if (turnExecCode === null) {
  report('CP-003', TURN_EXEC_REL, '读不出 `turn_exec.rs` —— 动作签名是「不重入」判据的唯一输入')
} else {
  const alias = /type\s+Action\s*=([^;]+);/.exec(turnExecCode)
  if (!alias) {
    report('CP-003', TURN_EXEC_REL, '找不到 `type Action` —— 动作签名（不重入的类型守）没有输入了')
  } else {
    const sig = alias[1]
    const mutRefs = sitesOf(sig, /&(?:'\w+\s+)?mut\b/g).length
    if (mutRefs !== 1 || !/\bCtx\b/.test(sig)) {
      report(
        'CP-003',
        TURN_EXEC_REL,
        `动作签名给了 ${mutRefs} 份可变借用（\`type Action = ${sig.trim()}\`）—— ` +
          '一个动作只拿得到一份 `&mut Ctx`：给得出第二份 `&mut` 就是环里能再开环（SP-033 TC-1c 第五道）',
      )
    }
    if (/\b(?:TurnSession|Session|Store)\b/.test(sig)) {
      report(
        'CP-003',
        TURN_EXEC_REL,
        '动作签名里出现了 `Session` / `Store` —— 动作拿第二份 `&mut Session` 就等于绕开唯一的循环体去还债（SP-033 TC-1c 第五道）',
      )
    }
  }
  const ctx = /struct\s+Ctx\b[^{]*\{/.exec(turnExecCode)
  if (!ctx) {
    report('CP-003', TURN_EXEC_REL, '找不到 `struct Ctx` —— 可变借用的份数没有输入了')
  } else {
    const body = turnExecCode.slice(ctx.index, matchBrace(turnExecCode, turnExecCode.indexOf('{', ctx.index)))
    const mutFields = sitesOf(body, /&(?:'\w+\s+)?mut\b/g).length
    if (mutFields !== 1) {
      report(
        'CP-003',
        TURN_EXEC_REL,
        `\`Ctx\` 里有 ${mutFields} 个可变借字段（应恰好 1 个：本轮草稿）—— ` +
          '第二个 `&mut` 字段就是「动作能拿到第二份可变状态」的另一种写法（SP-033 TC-1c 第五道）',
      )
    }
  }
}

// ───────────────────── CP-004：机制层无域名（TC-2） ─────────────────────

/**
 * 同名不同义：内核自己的 `LatencyTier` 词表里也有 `"reflex"`（`adapters.rs`），
 * 与执行体名 `reflex` 撞字面量。它属**机制层自己的词汇**，不是业务名字泄漏进来——
 * 判据按「字面量的 owner 在不在机制层」判：同一个字面量只许出现在它的 owner 文件里。
 */
const adaptersCode = prodCodeOf(ADAPTERS_REL) ?? ''
const tierNames = new Set(
  [...adaptersCode.matchAll(/LatencyTier::\w+\s*=>\s*"([^"]+)"/g)].map((m) => m[1]),
)

const leaks = []
for (const rel of [...prodFiles(KERNEL), ...prodFiles(CORE)]) {
  const code = prodCodeOf(rel)
  const literals = new Set(stringLiteralsOf(code))
  for (const word of VOCABULARY) {
    if (!literals.has(word)) continue
    if (rel === ADAPTERS_REL && tierNames.has(word)) continue
    leaks.push({ rel, word })
  }
}
for (const leak of leaks) {
  report(
    'CP-004',
    leak.rel,
    `机制层出现了控制面的名字 \`"${leak.word}"\` —— 端口名 / 行名 / 执行体名只属插件，` +
      '机制层认识它就等于「加一个端口要改机制」（SP-033 TC-2）',
  )
}

// ───────────────── CP-005：收束格唯一构造点（TC-4） ─────────────────

/** 收束格的两条事实名（常量名——字面量的 owner 在 plugin-api 的声明表里） */
const CLOSURE_CELLS = [
  ['CP-005', 'EVENT_CHAT_ASSISTANT_FINAL', 'chat.assistant.final'],
  ['CP-005', 'EVENT_CHAT_ASSISTANT_FALLBACK', 'chat.assistant.fallback'],
]

/** 全仓生产代码里「落了某个名字的 `Event::pending`」的站点 */
function appendSites(constName) {
  const out = []
  for (const rel of allCratesProd()) {
    const code = prodCodeOf(rel)
    for (const m of code.matchAll(/\bEvent\s*::\s*pending\s*\(/g)) {
      const args = topLevelArgs(code, code.indexOf('(', m.index))
      if (!args[1] || !new RegExp(`\\b${constName}\\b`).test(args[1])) continue
      out.push({ rel, index: m.index })
    }
  }
  return out
}

const closureSites = {}
for (const [, constName, literal] of CLOSURE_CELLS) {
  const sites = appendSites(constName)
  closureSites[constName] = sites
  if (sites.length !== 1) {
    report(
      'CP-005',
      'crates/',
      `收束格 \`${literal}\` 有 ${sites.length} 个构造点（${sites.map((s) => s.rel).join(' / ') || '无'}）—— ` +
        '同一格多写一处，字段迟早分叉，而「收束格有几份写方」在事实面上看不出来（SP-033 TC-4）',
    )
  }
}

/** 声明面：收束格必须登记在同一个执行体的 `writes` 里，否则事实面与声明面对不上 */
const closeActor = /const\s+ACTOR_CLOSE\s*:\s*ActorDecl\s*=\s*ActorDecl\s*\{/.exec(rowsCode)
if (!closeActor) {
  report('CP-005', ROWS_REL, '找不到 `ACTOR_CLOSE` 的声明 —— 收束格的写方在声明面上没有主人')
} else {
  const open = rowsCode.indexOf('{', closeActor.index)
  const body = rowsCode.slice(open, matchBrace(rowsCode, open))
  for (const [, constName, literal] of CLOSURE_CELLS) {
    if (!new RegExp(`\\b${constName}\\b`).test(body)) {
      report(
        'CP-005',
        ROWS_REL,
        `\`ACTOR_CLOSE.writes\` 里没有 \`${literal}\` —— 构造点落在别处而声明面不认，事实面就是两写方、声明面一个`,
      )
    }
  }
}

// ───────────────── CP-006：横向扩展零改机制（TC-5） ─────────────────

/** 机制面 = 驱动循环体 + 选债包装 + 在途判据。三者一起冻结，理由见头注释 CP-006 */
function mechanismSource() {
  const parts = []
  if (driveLoop) parts.push(driveCode.slice(driveLoop.start, driveLoop.end))
  for (const name of ['next', 'waiting']) {
    const fn = fnBody(rowsCode, name)
    parts.push(fn ? fn.source : `(missing fn ${name})`)
  }
  return parts.join('\n/* ── */\n')
}

const mechanismHash =
  'sha256:' + crypto.createHash('sha256').update(mechanismSource(), 'utf8').digest('hex')
const loopProdFiles = [...prodFiles(LOOP)]

/** 设计文档里的决策编号集合（`### D-NNN` 条目） */
let decisionCache = null
function decisionIds() {
  if (decisionCache) return decisionCache
  decisionCache = new Set()
  const walk = (dir) => {
    const tree = abs(dir)
    if (!fs.existsSync(tree)) return
    for (const e of fs.readdirSync(tree, { withFileTypes: true })) {
      const rel = `${dir}/${e.name}`
      if (e.isDirectory()) {
        if (e.name === 'archive' || e.name === 'node_modules' || e.name === 'target') continue
        walk(rel)
        continue
      }
      if (!e.name.endsWith('.md')) continue
      for (const line of (readRel(rel) ?? '').split('\n')) {
        const m = /^#{2,6}\s+(D-\d{3}a?)\b/.exec(line)
        if (m) decisionCache.add(m[1])
      }
    }
  }
  for (const r of ['docs/003-design', 'crates', 'docs/007-task']) walk(r)
  return decisionCache
}

function readBaseline() {
  const text = readRel(BASELINE_REL)
  if (text === null) return null
  try {
    return JSON.parse(text)
  } catch (e) {
    report('CP-006', BASELINE_REL, `基线解析失败（${e.message}）—— 基线读不出即判据失效，宁红不静默`)
    return null
  }
}

const baseline = readBaseline()

if (PRINT_BASELINE) {
  console.log(
    JSON.stringify(
      {
        mechanism: {
          file: DRIVE_REL,
          parts: ['drive loop body', 'fn next', 'fn waiting'],
          hash: mechanismHash,
          adr: baseline?.mechanism?.adr ?? '',
          reason: baseline?.mechanism?.reason ?? '',
        },
        extension: { productionFiles: loopProdFiles.length },
      },
      null,
      2,
    ),
  )
  process.exit(0)
}

if (!baseline) {
  report(
    'CP-006',
    BASELINE_REL,
    `基线 ${BASELINE_REL} 读不出 —— 「加端口不改机制」没有比对基准，判据没有输入，宁红不静默`,
  )
} else {
  if (baseline.mechanism?.hash !== mechanismHash) {
    report(
      'CP-006',
      DRIVE_REL,
      `机制面字节与基线不符：实测 ${mechanismHash} ≠ 基线 ${baseline.mechanism?.hash ?? '(无)'}。` +
        '若这次改动**是横向扩展的代价**（为新端口改了驱动循环 / 选债 / 在途判据），那正是本条要拦的形状——改插件而不是改机制；' +
        '若是与端口无关的机制演进，同批更新基线并挂一条决策（`--print-baseline`）',
    )
  }
  const cap = baseline.extension?.productionFiles
  if (typeof cap !== 'number') {
    report('CP-006', BASELINE_REL, '基线缺 `extension.productionFiles` —— 控制面文件数没有上限，加端口可以顺手加一台 host')
  } else if (loopProdFiles.length > cap) {
    report(
      'CP-006',
      'symbio-plugin-loop/',
      `控制面生产文件 ${loopProdFiles.length} 份，超过基线 ${cap} 份 —— ` +
        '横向扩展的改动集只有「声明 + 适配器 + 事件名」，多出来的文件不是适配器就是新机制（SP-033 TC-5）',
    )
  }
  const refs = [...String(baseline.mechanism?.adr ?? '').matchAll(/D-(\d+[a-z]?)/g)].map((m) => m[1])
  if (String(baseline.mechanism?.reason ?? '').trim() === '') {
    report('CP-006', BASELINE_REL, '基线的 `reason` 为空 —— 基线是判据的一面，改它必须写清「为什么这次机制可以动」')
  }
  if (refs.length === 0) {
    report('CP-006', BASELINE_REL, '基线的 `adr` 里没有决策编号 —— 冻结机制面是一次架构决策，必须挂一条能打开的决策')
  } else {
    const decided = decisionIds()
    for (const id of refs) {
      if (!decided.has(`D-${id}`)) {
        report('CP-006', BASELINE_REL, `基线引用了决策 \`D-${id}\`，但设计文档里没有 \`### D-${id}\` 条目 —— 指向不存在的决策的基线不算基线`)
      }
    }
  }
}

/**
 * 端口闭集：代码里声明的端口必须落在 `D-222` §一的名单内。
 *
 * 这一条是 `TC-5` 的**另一半**：字节冻结只判「加端口有没有改机制」，判不到
 * 「这个端口该不该存在」。两者合起来才是「横向扩展」的代价界——端口是声明出来的，
 * 而**声明哪些端口**是裁决，不是实现。
 *
 * 方向是单向的：代码里多一个裁决没点的端口 ⇒ 红（静默扩端口）；裁决点了而代码还没
 * 声明 ⇒ 只是读数（设计先于实施，允许代码晚一步）。要加端口，先改 `D-222` §一——
 * 那是一次可见的裁决改动，而不是在 `PORTS` 里多加一个结构体。
 */
const ruling = rulingPorts()
const declaredPortsAll = declaredPorts()
let closureReading = '(未判定)'
if (ruling.error) {
  report(
    'CP-006',
    RULING_REL,
    `端口闭集读不出：${ruling.error} —— 闭集的 owner 是 \`D-222\` §一，读不出即判据没有输入（宁红不静默）`,
  )
} else {
  const closed = new Set(ruling.ports)
  for (const d of declaredPortsAll) {
    if (closed.has(d.port)) continue
    report(
      'CP-006',
      d.rel,
      `声明了端口 \`${d.port}\`，但裁决里今天的端口闭集只有 ${ruling.ports.map((p) => `\`${p}\``).join('、')}。` +
        '顺带一问：它需要一台 host 或一段循环才能跑吗？需要的话它就不是端口，是环的一部分（`D-222` §一）。' +
        '真要加，先改那条裁决——闭集不是实现顺手扩的',
    )
  }
  const declaredNames = new Set(declaredPortsAll.map((d) => d.port))
  const pending = ruling.ports.filter((p) => !declaredNames.has(p))
  closureReading =
    `${ruling.ports.map((p) => `\`${p}\``).join(' · ')}（裁决 \`D-222\` §一）· 代码声明 ${declaredPortsAll.length} 条` +
    (pending.length ? ` · 裁决有而代码未声明：${pending.join(' / ')}（读数，代码晚于裁决）` : '')
}

/** 控制面不该按能力开文件：文件名词干等于端口名 / 行名 ⇒ 有人给一个能力单开了一份机制 */
for (const rel of loopProdFiles) {
  const stem = path.basename(rel, '.rs')
  const clash = [...PORT_NAMES, ...ROW_NAMES].find((w) => w === stem)
  if (clash) {
    report(
      'CP-006',
      rel,
      `控制面文件的名就是能力名 \`${clash}\` —— 端口 / 行是**数据**，不该长出一份同名机制文件（SP-033 TC-5）`,
    )
  }
}

// ───────────────── CP-007：判据集闭集（TC-6） ─────────────────

/**
 * 六条不变量 ↔ 八条码。**这张表是判据集本身**：新增一条码必须挂到某条 `TC-n` 下，
 * 而 `TC-n` 的条数被 `TERMINAL_CRITERIA` 钉住（§3 的「六条」）。于是「为某个新能力
 * 单独加一条专用判据」在这里表现为一次显式的表改动 + 一次条数改动，不是顺手。
 */
const TC_MAP = {
  'TC-1': ['CP-001', 'CP-002', 'CP-003'],
  'TC-2': ['CP-004'],
  'TC-3': ['CP-008'],
  'TC-4': ['CP-005'],
  'TC-5': ['CP-006'],
  'TC-6': ['CP-007'],
}

const specText = readRel(SPEC_REL)
if (specText === null) {
  report('CP-007', SPEC_REL, '读不出规范 —— 判据集闭集的唯一输入是 `SP-033` 本身')
} else {
  const specIds = [...specText.matchAll(/^####\s+TC-(\d+)\b/gm)].map((m) => `TC-${m[1]}`)
  const unique = [...new Set(specIds)].sort((a, b) => Number(a.slice(3)) - Number(b.slice(3)))
  if (unique.length !== TERMINAL_CRITERIA) {
    report(
      'CP-007',
      SPEC_REL,
      `规范里有 ${unique.length} 条 \`#### TC-n\`（应为 ${TERMINAL_CRITERIA} 条）—— ` +
        '判据条数不随能力数增长：新能力落进既有 `TC-n`，不另开一条（SP-033 TC-6）',
    )
  }
  const mapKeys = Object.keys(TC_MAP).sort((a, b) => Number(a.slice(3)) - Number(b.slice(3)))
  if (mapKeys.join(',') !== unique.join(',')) {
    report(
      'CP-007',
      'scripts/control-plane-audit.mjs',
      `TC_MAP 的键（${mapKeys.join(' / ')}）与规范里的 TC 条目（${unique.join(' / ')}）对不上 —— ` +
        '每条不变量必须有唯一执行方，执行方与条文一一对应（SP-033 TC-6）',
    )
  }
  const mapped = mapKeys.flatMap((k) => TC_MAP[k])
  for (const code of CODES) {
    const n = mapped.filter((c) => c === code).length
    if (n !== 1) {
      report(
        'CP-007',
        'scripts/control-plane-audit.mjs',
        `判据码 ${code} 在 TC_MAP 里出现 ${n} 次（应恰好 1 次）—— 一条码只回答一条不变量，重复归属等于两条条文各判一半`,
      )
    }
  }
  // 规范 §「判据」表：每条 TC 一行，且行里点名一个真实存在的执行方脚本
  const tableStart = specText.indexOf('## 判据')
  const table = tableStart < 0 ? '' : specText.slice(tableStart)
  for (const tc of unique) {
    const row = table.split('\n').find((l) => new RegExp(`^\\|\\s*\`${tc}\``).test(l))
    if (!row) {
      report('CP-007', SPEC_REL, `§「判据」表里没有 \`${tc}\` 那一行 —— 条文与执行方的对应关系没有登记处`)
      continue
    }
    const scripts = [...row.matchAll(/scripts\/[\w.-]+\.mjs/g)].map((m) => m[0])
    if (scripts.length === 0) {
      report('CP-007', SPEC_REL, `\`${tc}\` 那一行没有点名任何执行方脚本 —— 判据要指得到唯一执行方`)
    }
    for (const s of new Set(scripts)) {
      if (!fs.existsSync(abs(s))) report('CP-007', SPEC_REL, `\`${tc}\` 那一行点名的 \`${s}\` 不存在 —— 「有执行方」是假的`)
    }
  }
  // 判据集不许跟着能力长：新能力只能落进既有 `TC-n`，所以「加一条专用判据」在这里
  // 必然表现为**第七条 `TC-n`**（上面那条判红）或**一条重复挂载的码**（上面那条判红）。
  // 不再单列一条「码数」判定——那会是 `CODES.length === 8` 这种恒真的自检。
}

// ───────────────── CP-008：声明面封闭（TC-3） ─────────────────

const declCode = prodCodeOf(DECL_REL)
if (declCode === null) {
  report('CP-008', DECL_REL, '读不出声明表 —— 声明面封闭的判据没有输入了')
} else {
  const builder = /impl\s+Builder\s*\{/.exec(declCode)
  if (!builder) {
    report('CP-008', DECL_REL, '找不到 `impl Builder` —— 注册口的形状没有输入了')
  } else {
    const open = declCode.indexOf('{', builder.index)
    const body = declCode.slice(open, matchBrace(declCode, open))
    const pubFns = [...body.matchAll(/pub\s+fn\s+([A-Za-z0-9_]+)/g)].map((m) => m[1]).sort()
    const expected = ['declare', 'new', 'seal']
    if (pubFns.join(',') !== expected.join(',')) {
      report(
        'CP-008',
        DECL_REL,
        `\`impl Builder\` 的公开方法是 [${pubFns.join(' / ')}]（应为 [${expected.join(' / ')}]）—— ` +
          '装载期只有「交声明 → seal」两个动作；多一个公开方法就是多一条注册路径（SP-033 TC-3）',
      )
    }
    // seal 必须是**按值消费**：`&mut self` / `&self` 都能让同一个 Builder 事后继续注册
    const seal = /pub\s+fn\s+seal\s*\(([^)]*)\)/.exec(body)
    if (!seal) {
      report('CP-008', DECL_REL, '找不到 `pub fn seal` —— 声明面没有唯一出口')
    } else if (!/^\s*self\s*$/.test(seal[1])) {
      report(
        'CP-008',
        DECL_REL,
        `\`seal\` 的接收者是 \`${seal[1].trim()}\`（应为按值 \`self\`）—— 借用式 seal 之后同一份声明还能被改，` +
          '「运行期不能注册」在类型上就不成立（SP-033 TC-3）',
      )
    }
  }

  const frozen = /impl\s+Frozen\s*\{/.exec(declCode)
  if (!frozen) {
    report('CP-008', DECL_REL, '找不到 `impl Frozen` —— 冻结表的方法面没有输入了')
  } else {
    const open = declCode.indexOf('{', frozen.index)
    const body = declCode.slice(open, matchBrace(declCode, open))
    const mutSelf = sitesOf(body, /&\s*mut\s+self\b/g).length
    if (mutSelf !== 0) {
      report(
        'CP-008',
        DECL_REL,
        `\`impl Frozen\` 里有 ${mutSelf} 个 \`&mut self\` 方法 —— 冻结后只剩读方法；可变方法就是运行期注册口（SP-033 TC-3）`,
      )
    }
    const mutReturns = sitesOf(body, /pub\s+fn\s+\w+[^;{]*->\s*&\s*(?:'\w+\s+)?mut\b/g).length
    if (mutReturns !== 0) {
      report('CP-008', DECL_REL, `\`impl Frozen\` 里有 ${mutReturns} 个方法把内部借成 \`&mut\` —— 同样是运行期注册口（SP-033 TC-3）`)
    }
  }
}

// ───────────────────────────── 读数与汇总 ─────────────────────────────

if (errors === 0) {
  console.log('')
  console.log(dim('── 控制面终态读数 ──'))
  console.log(dim(`  词表（取自行 / 执行体的声明处；端口归 PTS-002）：${VOCABULARY.join(' · ') || '（空）'}`))
  console.log(dim(`  循环体 ${loops.length} 个 · 选债调用 ${nextCalls.length} 处 · \`ring::drive\` ${driveSolverCalls.length} 处`))
  console.log(
    dim(
      `  幂等点问（读数，J0 已退场为读数）${pointQueries.length} 处：` +
        (pointQueries.map((q) => q.rel.replace(`${LOOP}/`, '')).join(' · ') || '无'),
    ),
  )
  console.log(dim(`  控制面生产文件 ${loopProdFiles.length} 份（基线 ${baseline?.extension?.productionFiles ?? '?'}）`))
  console.log(dim(`  端口闭集 ${closureReading}`))
  console.log(dim(`  机制面指纹 ${mechanismHash}`))
  for (const [, constName, literal] of CLOSURE_CELLS) {
    const sites = closureSites[constName] ?? []
    console.log(dim(`  收束格 ${literal}：${sites.map((s) => s.rel).join(' / ')}`))
  }
  console.log('')
  const names = {
    'CP-001': '唯一循环体（TC-1a）',
    'CP-002': '唯一选债口（TC-1b）',
    'CP-003': '五道守卫同处 + 不重入由类型守（TC-1c）',
    'CP-004': '机制层无域名（TC-2）',
    'CP-005': '收束格唯一构造点（TC-4）',
    'CP-006': '横向扩展零改机制（TC-5）',
    'CP-007': '判据集闭集（TC-6）',
    'CP-008': '声明面封闭（TC-3）',
  }
  for (const [code, name] of Object.entries(names)) {
    const n = hits.get(code) ?? 0
    console.log(`  ${n === 0 ? green('✓') : red('✗')} ${code}  ${name}${n ? `  (${n})` : ''}`)
  }
  console.log('')
  process.exit(0)
}

console.log('')
for (const [code, name] of Object.entries({
  'CP-001': '唯一循环体（TC-1a）',
  'CP-002': '唯一选债口（TC-1b）',
  'CP-003': '五道守卫同处 + 不重入由类型守（TC-1c）',
  'CP-004': '机制层无域名（TC-2）',
  'CP-005': '收束格唯一构造点（TC-4）',
  'CP-006': '横向扩展零改机制（TC-5）',
  'CP-007': '判据集闭集（TC-6）',
  'CP-008': '声明面封闭（TC-3）',
})) {
  const n = hits.get(code) ?? 0
  console.log(`  ${n === 0 ? green('✓') : red('✗')} ${code}  ${name}${n ? `  (${n})` : ''}`)
}
console.log('')
console.log(red(`  ${errors} 个违规`))
console.log(
  `${yellow('提示')}：判据口径见本脚本文件头与 docs/005-spec/033.md §3；` +
    '改机制面后更新基线：node scripts/control-plane-audit.mjs --print-baseline',
)
process.exit(1)
