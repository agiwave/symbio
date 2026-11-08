#!/usr/bin/env node
/**
 * sink-row-audit — 归宿行（`by = None`）的条数棘轮（SR-001…004）
 *
 * ## 它治什么
 *
 * `D-397` 写下了「删读者不构成新增归宿行的理由」：某格的读者被删之后，必须先判它
 * 是否产生债务；只有确认「无债务、无时机」才允许补一条归宿行。但这句话此前只有散文
 * ——每删一处读者补一条 `by = None` 的行，`seal` 的 `OrphanFact` 就绿，
 * 「没人执行」于是被写成「有人看护」。文字约束拦不住逐条添加，数字才拦得住。
 *
 * 本脚本把「生产声明表里有几条归宿行」冻成**只许降**的棘轮，基线是
 * `scripts/baselines/sink-rows.json`（初始 14 条，逐条带 `why` 与 `adr`）。
 *
 * | 码 | 判什么 | 处理 |
 * |---|---|---|
 * | `SR-001` | 生产比基线**多**一条归宿行（按名字计数，不是按名字集合） | 红；确需新增必须同批在基线里登记 `why` / `adr`，并在提交说明里回答「这一格为什么不需要派发」 |
 * | `SR-002` | 基线有而生产**没有**（清债了却没同批删基线） | 红；否则「删一条再加一条」能把总数抹平 |
 * | `SR-003` | 基线条目形状不对（缺 `name` / `why` / `adr`、理由为空、`D-NNN` 不存在、重名） | 红；新增的署名不许匿名或空头 |
 * | `SR-004` | 生产里有 `by: None` 的行，但它的 `name` 是解析不出的常量 | 红；否则这条行对棘轮隐形 |
 *
 * ## 统计口径（这条比脚本本身重要）
 *
 * 数是**生产 `Decl` 字面量里 `by: None` 的行**，不是文本里出现 `by: None` 的次数：
 *
 * - 先 `blankComments`，注释里提一句 `by: None` 不算；
 * - 再 `blankTestModules`，并跳过 `*.test.rs` / `tests.rs`，测试夹具不算；
 * - 只认 `Decl { … }` 字面量体内的 `by` 字段（`\bby\s*:\s*None\b`），
 *   所以 `produced_by: None`（`_` 前没有词边界）不会被误算。
 *
 * `name` 可能是常量（`EVENT_MEMORY_ENCODED`）或别名（`use … as CLASSIFY_ANSWERED`），
 * 脚本从全仓生产 `.rs` 收集 `const …: &str = "…"` 与 `as` 别名后解析出字符串：
 * 改常量名但值不变 ⇒ 仍是同一行；解析不出 ⇒ `SR-004`，不静默跳过。
 *
 * ## 用法
 *
 *   node scripts/sink-row-audit.mjs                  # 对真仓判定
 *   node scripts/sink-row-audit.mjs --strict         # 门禁统一参数（本脚本无 WARNING 级）
 *   node scripts/sink-row-audit.mjs --root=<dir>     # 回归测试夹具
 *   node scripts/sink-row-audit.mjs --print-baseline # 打印当前生产清单（保留已有 why/adr）
 *
 * 退出码：0 = 通过；1 = 有违规；2 = 范围读不出（缺 `crates/` / 基线缺失或 JSON 坏）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns SR 归宿行（by = None）条数棘轮
// @codes SR-001 SR-002 SR-003 SR-004

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, dim } from './color.mjs'
import { walk, isTestFile } from './fs-walk.mjs'
import { blankComments, blankTestModules, matchBrace } from './rust-scan.mjs'
import { adrExists } from './adr-index.mjs'

const argv = process.argv.slice(2)
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const PRINT = argv.includes('--print-baseline')
const BASELINE_REL = 'scripts/baselines/sink-rows.json'

const CONST_RE =
  /(?:^|\n)\s*(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)\s*:\s*&(?:'static\s+)?str\s*=\s*("(?:[^"\\]|\\.)*")/g
const ALIAS_RE = /\b([A-Za-z_][A-Za-z0-9_]*)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\b/g
const DECL_RE = /\bDecl\s*\{/g
const BY_NONE_RE = /\bby\s*:\s*None\b/
const NAME_RE = /\bname\s*:\s*([A-Za-z_][A-Za-z0-9_:]*|"(?:[^"\\]|\\.)*")/

/** Rust 字符串字面量里的常见转义（这些常量值都只有普通字符，仍然解一遍）。 */
function unescapeString(body) {
  return body.replace(/\\(["\\nrt'])/g, (_, c) => {
    switch (c) {
      case 'n':
        return '\n'
      case 'r':
        return '\r'
      case 't':
        return '\t'
      default:
        return c
    }
  })
}

/** 收集全仓生产 `.rs` 里的字符串常量与 `use … as …` 别名。 */
function scanSymbols(root) {
  const dir = path.join(root, 'crates')
  if (!fs.existsSync(dir)) return null
  const consts = new Map()
  const aliases = new Map()
  const files = []
  for (const abs of walk(dir)) {
    if (!abs.endsWith('.rs') || isTestFile(abs)) continue
    const rel = path.relative(root, abs).split(path.sep).join('/')
    const text = blankTestModules(blankComments(fs.readFileSync(abs, 'utf8')).join('\n'))
    files.push({ rel, text })
    for (const m of text.matchAll(CONST_RE)) {
      const body = unescapeString(m[2].slice(1, -1))
      const set = consts.get(m[1]) ?? new Set()
      set.add(body)
      consts.set(m[1], set)
    }
    for (const stmt of text.matchAll(/\buse\s+([^;]+);/g)) {
      for (const m of stmt[1].matchAll(ALIAS_RE)) aliases.set(m[2], m[1])
    }
  }
  return { files, consts, aliases }
}

/** 把一个 `name:` token（字面量 / 常量 / 路径末段）解析成事实名字符串。 */
export function resolveName(token, consts, aliases) {
  if (token.startsWith('"')) return unescapeString(token.slice(1, -1))
  let cur = token.includes('::') ? token.split('::').pop() : token
  const seen = new Set()
  while (cur && !seen.has(cur)) {
    seen.add(cur)
    const vals = consts.get(cur)
    if (vals) return vals.size === 1 ? [...vals][0] : null
    if (!aliases.has(cur)) return null
    cur = aliases.get(cur)
  }
  return null
}

/**
 * 提取生产声明表里的归宿行。
 *
 * @returns {{ rows: {name,file,line}[], unresolved: {file,line,token}[], error?: string }}
 */
export function scanRows(root = ROOT) {
  const scanned = scanSymbols(root)
  if (!scanned) return { rows: [], unresolved: [], error: 'crates/ 不存在' }
  const rows = []
  const unresolved = []
  for (const { rel, text } of scanned.files) {
    for (const m of text.matchAll(DECL_RE)) {
      const open = m.index + m[0].length - 1
      const close = matchBrace(text, open)
      const body = text.slice(open, close)
      if (!BY_NONE_RE.test(body)) continue
      const line = text.slice(0, m.index).split('\n').length
      const nameM = NAME_RE.exec(body)
      if (!nameM) {
        unresolved.push({ file: rel, line, token: '(没有 name 字段)' })
        continue
      }
      const name = resolveName(nameM[1], scanned.consts, scanned.aliases)
      if (!name) {
        unresolved.push({ file: rel, line, token: nameM[1] })
        continue
      }
      rows.push({ name, file: rel, line })
    }
  }
  return { rows, unresolved }
}

/** 读基线；缺失或不是合法 JSON 都返回 `error`（调用方退 2，不许「读空 ⇒ 通过」）。 */
export function readBaseline(root = ROOT) {
  const file = path.join(root, BASELINE_REL)
  if (!fs.existsSync(file)) return { error: `基线缺失：${BASELINE_REL}` }
  let obj
  try {
    obj = JSON.parse(fs.readFileSync(file, 'utf8'))
  } catch (e) {
    return { error: `基线不是合法 JSON：${e.message}` }
  }
  if (!Array.isArray(obj?.rows)) return { error: `基线缺少 rows 数组：${BASELINE_REL}` }
  return { note: obj.note ?? '', rows: obj.rows }
}

/** 基线条目的形状与署名（SR-003）。`adrExists` 是决策号存在性的唯一扫描实现。 */
export function checkBaseline(rows, root = ROOT) {
  const errors = []
  const seen = new Set()
  const cache = new Map()
  const exists = (adr) => {
    if (!cache.has(adr)) cache.set(adr, adrExists(root, adr))
    return cache.get(adr)
  }
  for (const [i, r] of rows.entries()) {
    const where = `${BASELINE_REL}#rows[${i}]`
    if (!r || typeof r !== 'object' || Array.isArray(r)) {
      errors.push({ code: 'SR-003', message: `${where} 不是对象` })
      continue
    }
    if (typeof r.name !== 'string' || r.name.trim() === '') {
      errors.push({ code: 'SR-003', message: `${where} 缺少 name` })
      continue
    }
    if (seen.has(r.name)) {
      errors.push({ code: 'SR-003', message: `${where} 重复登记 ${r.name}` })
    }
    seen.add(r.name)
    if (typeof r.why !== 'string' || r.why.trim() === '') {
      errors.push({ code: 'SR-003', message: `${where}（${r.name}）缺少非空 why ⇒ 必须回答「这一格为什么不需要派发」` })
    }
    if (typeof r.adr !== 'string' || !/^D-\d{3}$/.test(r.adr.trim())) {
      errors.push({ code: 'SR-003', message: `${where}（${r.name}）缺少 adr（D-NNN）` })
    } else if (!exists(r.adr.trim())) {
      errors.push({ code: 'SR-003', message: `${where}（${r.name}）挂的决策 ${r.adr} 不存在 ⇒ 新增归宿行必须带能打开的 D 号` })
    }
  }
  return errors
}

/**
 * 按**名字的出现次数**对账（不是集合）：同名行多出一条也要红，
 * 否则「把一条已有行复制到另一个插件」会从棘轮里隐形。
 */
export function judge(rows, baselineRows, unresolved) {
  const errors = []
  for (const u of unresolved) {
    errors.push({
      code: 'SR-004',
      message: `${u.file}:${u.line} 的归宿行 name 解析不出（${u.token}）⇒ 这条行对棘轮不可见，先把它写成可解析的字符串常量`,
    })
  }
  const counts = (list, key) => {
    const m = new Map()
    for (const r of list) {
      const k = typeof r === 'string' ? r : r[key]
      if (typeof k !== 'string') continue
      m.set(k, (m.get(k) ?? 0) + 1)
    }
    return m
  }
  const now = counts(rows, 'name')
  const was = counts(baselineRows, 'name')
  for (const [name, n] of now) {
    const base = was.get(name) ?? 0
    if (n <= base) continue
    const file = rows.find((r) => r.name === name)
    const where = file ? `${file.file}:${file.line}` : '（未知位置）'
    errors.push({
      code: 'SR-001',
      message:
        `新增归宿行 ${name}（${where}；生产 ${n} 条 / 基线 ${base} 条）—— 归宿行只许降。` +
        `确需新增必须同批把它写进 ${BASELINE_REL}，带非空 why 与能打开的 adr（D-NNN），` +
        `并在提交说明里回答「这一格为什么不需要派发」`,
    })
  }
  for (const [name, n] of was) {
    const live = now.get(name) ?? 0
    if (live >= n) continue
    errors.push({
      code: 'SR-002',
      message:
        `基线登记了 ${name}（${n} 条），生产只剩 ${live} 条 —— 清债必须同批删基线；` +
        `否则「删一条再加一条」能把总数抹平`,
    })
  }
  return errors
}

/** 打印当前生产清单；已有条目的 why/adr 原样保留，新行留空等待填写。 */
function printBaseline(rows, baselineRows) {
  const prev = new Map(baselineRows.map((r) => [r.name, r]))
  const out = {
    note: `归宿行（by = None）条数棘轮：只许降。统计口径是生产 Decl 字面量里 by: None 的行（剥注释、剔除 #[cfg(test)] 与 *.test.rs），不是文本出现次数。新增一行必须同批登记非空 why 与能打开的 adr（D-NNN），并在提交说明里回答「这一格为什么不需要派发」。`,
    rows: rows.map((r) => {
      const p = prev.get(r.name)
      return { name: r.name, why: p?.why ?? '', adr: p?.adr ?? '' }
    }),
  }
  console.log(JSON.stringify(out, null, 2))
}

const baseline = readBaseline(ROOT)
const scanned = scanRows(ROOT)

if (PRINT) {
  printBaseline(scanned.rows, baseline.rows ?? [])
  process.exit(scanned.error ? 2 : 0)
}

if (scanned.error) {
  console.error(red(`范围读不出：${scanned.error} —— 棘轮审计不许在看不见生产声明表时通过`))
  process.exit(2)
}
if (baseline.error) {
  console.error(red(`范围读不出：${baseline.error} —— 棘轮审计不许在看不见基线时通过`))
  process.exit(2)
}

const errors = [...checkBaseline(baseline.rows, ROOT), ...judge(scanned.rows, baseline.rows, scanned.unresolved)]

if (errors.length) {
  for (const e of errors) console.log(red(`[ERROR] ${e.code} ${e.message}`))
  console.log(
    `sink-row-audit 未通过：${errors.length} 处违规（生产 ${scanned.rows.length} 条 / 基线 ${baseline.rows.length} 条，` +
      `棘轮读数 ${scanned.rows.length}）`,
  )
  process.exit(1)
}

console.log(
  green(
    `sink-row-audit 通过：生产 ${scanned.rows.length} 条归宿行，基线 ${baseline.rows.length} 条，` +
      `棘轮读数 ${scanned.rows.length}/${baseline.rows.length}（只许降）`,
  ),
)
for (const r of scanned.rows) console.log(dim(`  · ${r.name}  ${r.file}:${r.line}`))
