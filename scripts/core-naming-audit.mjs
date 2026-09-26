#!/usr/bin/env node
/**
 * core-naming-audit — `symbio_core` 公开面**命名规范**的判定型守卫
 *
 * ## 它守的是什么
 *
 * `symbio_core/README.md` §1.2 有一张「域前缀对照表」：每个域的类型 / 常量该用什么
 * 前缀，写得一清二楚。但**此前没有任何脚本在判它**——表是靠人记的，而「靠人记」的
 * 规则会以两种方式烂掉：
 *
 * 1. **表与代码漂移**：新增符号时没查表。已发生过的三处——`PLUGIN_PROVIDER_FIELD`
 *    住在 `vdfs` 却姓 `PLUGIN`；`KEY_PROVIDER` 住在 `plugin` 却用着 `keys` 域的前缀；
 *    插件工厂 id（16 个）住在 `keys` 却姓 `PLUGIN`。
 * 2. **表与脚本漂移**：表改了脚本没改（或反过来），于是守卫亮着绿灯、而规则早已不是
 *    文档写的那条——这与 `schema-audit` 曾出现的「报告说 A、判定说 B」是同一类失效。
 *
 * 本脚本把那张表**当作规则源解析**：读 README §1.2 的表格，对每个公开符号检查前缀
 * 是否落在**所属域**登记的前缀里。于是「改表」与「改代码」被绑成一件事。
 *
 * ## 规则
 *
 * | 编号  | 规则 | 为什么 |
 * |---|---|---|
 * | N-001 | 类型 / 常量的名字必须匹配**所属域**登记的前缀 | 「符号带域前缀」是 §1.2 的核心；匹配不上 ⇒ 调用点读不出归属 |
 * | N-002 | 一个前缀只能被**一个**域登记 | 「一域一前缀」——两个域共用前缀 = 一个前缀两种东西，调用点反而更难读 |
 * | N-003 | 符号用了**别的域**的前缀时，按「错放」报（比 N-001 更具体） | `KEY_PROVIDER` 住 `plugin` 却用 `keys` 的前缀：它要么改名，要么搬家——报出「该前缀属于谁」才能让人一次改对 |
 * | N-004 | 每个域目录都必须在表里登记，反之亦然 | 新增域却不登记前缀 ⇒ 该域符号无人核对；表里留一个不存在的域 ⇒ 表在说假话 |
 * | N-005 | 公开面里不得有**无归属域**的类型 / 常量 | 根 `mod.rs` 直接声明的符号没有域 ⇒ 无法核对前缀（函数不在此列，见下） |
 *
 * **函数不判**：§1.2 明文「函数 / 自由函数不强求前缀」——导入行已经带着模块路径，
 * 前缀只是噪音。
 *
 * ## 它承认的三件事（不是「豁免」，是 §1.2 / §3 登记的规则）
 *
 * - **限定词**：名字可以带一个限定词前缀，限定词不算域前缀。
 *   `DynVdfsProvider` = `Dyn` + `VdfsProvider`；`DefaultToolVisitor` = `Default` + `ToolVisitor`。
 *   限定词表由 README 里的 `<!-- core-naming:modifiers … -->` 标记持有（与 E-008 的
 *   `<!-- vocab:… -->` 同一约定：**标了才认，没标不猜**）。
 * - **后缀**：`keys` 域的类型用 `…Key` 后缀（`PathKey`）——`KeyPath` 会读成「键的路径」，
 *   语义反了。
 * - **裸名**：`keys` 域的实例用裸名（`PATH`）——实例是**取值的凭据**（`ctx.get(&PATH)`），
 *   不是域符号。把实例改成 `KEY_PATH` 会让 `KEY_` 同时指代「类型」与「键对象」，
 *   用一个统一前缀换掉一个真实的区分，是净亏损（README §3）。
 *
 * ## 匹配怎么做的
 *
 * 前缀不是**字面** startsWith，而是**词**级：把名字切成词（类型按 CamelCase、常量按
 * `_`），跳过开头的限定词，再要求与登记前缀的**词序列**逐词相等。这样：
 * `EVENT_BUS_KIND_SYSTEM` 匹配 `EVENT_BUS_`（词 `[EVENT,BUS]`）；
 * `VDFS_PLUGIN_PROVIDER_FIELD` 只匹配 `VDFS_`（首词是 `VDFS`），**不会**因为含
 * `PLUGIN_` 就被判成越界。
 *
 * ## 已知边界
 *
 * - 判据是**名字形态**（`kindOf` 用 Rust 命名约定推断 const / type / fn），不是 AST。
 * - 只判 `symbio_core` 的**根平铺公开面**，与 `core-surface-audit` 共用
 *   [`core-surface.mjs`](./core-surface.mjs) 的同一份口径——两者必须对「公开面是什么」
 *   给出同一个答案。
 * - `schemas` 域整体豁免：协议词就是它的命名空间，逐字段与前端镜像，改名要跨栈同步。
 *
 * ## 用法
 *
 *   node scripts/core-naming-audit.mjs               # 审计
 *   node scripts/core-naming-audit.mjs --root=<dir>  # 换根目录（回归测试用）
 *
 * 退出码：0 = 通过；1 = 有违规或规则源不可解析。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'
import { collectCoreSurface, kindOf } from './core-surface.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : defaultRoot

const README_REL = 'symbio/src/symbio_core/README.md'
const MODIFIER_MARK = 'core-naming:modifiers'

// ── 规则源：README §1.2 的域前缀对照表 ────────────────────────────────────

/**
 * 解析一个「类型 / 常量」单元格。
 *
 * 表里的写法只有五种（README §1.2 是它们唯一的 owner）：
 *   - `—`            该域没有这一类符号（**不是**「不用前缀」）
 *   - `协议词`        整域豁免（`schemas`）
 *   - `裸名`          不要求前缀（`keys` 的实例）
 *   - `` `…Key` ``    后缀规则（`…` 表示「前面任意」）
 *   - `` `X` `` / `` `X*` ``  前缀规则（`*` 是「子命名空间」的记号，与前缀同义）
 */
function parseSpec(cell) {
  const c = cell.replace(/\*\*/g, '').trim()
  if (/^[—–-]+$/.test(c)) return { kind: 'none' }
  if (c.includes('协议词')) return { kind: 'exempt' }
  if (c.includes('裸名')) return { kind: 'bare' }
  const tokens = [...c.matchAll(/`([^`]+)`/g)].map((m) => m[1])
  let suffix = null
  const prefixes = []
  for (const t of tokens) {
    if (t.includes('…')) {
      suffix = t.replace(/[^A-Za-z0-9]/g, '')
      continue
    }
    prefixes.push(t.replace(/\*+$/, ''))
  }
  if (suffix) return { kind: 'suffix', suffix }
  if (prefixes.length) return { kind: 'prefix', prefixes }
  return { kind: 'none' }
}

/**
 * 读 README §1.2：返回 `{ rules: Map<域, {typeSpec, constSpec}>, modifiers: Set }`。
 *
 * 任何解析失败都**抛错**（由调用方转成退出码 1）：规则源读不出来时，一个「没找到
 * 违规」的绿灯是**假的**——它只说明没检查，不说明合规。这与 `mechanism-audit` 的
 * 「守卫读不到规则必须红，不能静默放过」同一条原则。
 */
function readRules() {
  const md = fs.readFileSync(path.join(ROOT, README_REL), 'utf8')
  const lines = md.split(/\r?\n/)

  const modLine = lines.find((l) => l.includes(MODIFIER_MARK))
  if (!modLine) {
    throw new Error(`README §1.2 缺少限定词标记 \`<!-- ${MODIFIER_MARK} … -->\``)
  }
  const modifiers = new Set(
    modLine
      .replace(new RegExp(`.*${MODIFIER_MARK}`), '')
      .replace(/-->.*/, '')
      .split(/[,\s]+/)
      .map((s) => s.trim())
      .filter(Boolean),
  )

  const head = lines.findIndex((l) => l.includes('域前缀对照表'))
  if (head < 0) throw new Error('README 里找不到「域前缀对照表」小节')

  const rules = new Map()
  let inTable = false
  for (let i = head; i < lines.length; i++) {
    const line = lines[i]
    if (!line.trim().startsWith('|')) {
      if (inTable) break
      continue
    }
    const cells = line.split('|').slice(1, -1).map((x) => x.trim())
    if (cells.length < 3) continue
    if (cells[0] === '域' || /^[-: ]+$/.test(cells[0])) continue
    const domain = cells[0].replace(/`/g, '').trim()
    if (!domain) continue
    inTable = true
    rules.set(domain, { typeSpec: parseSpec(cells[1]), constSpec: parseSpec(cells[2]) })
  }
  if (!rules.size) throw new Error('README §1.2 的表没有解析出任何域')
  return { rules, modifiers }
}

// ── 词级前缀匹配 ─────────────────────────────────────────────────────────

const CONST_RE = /^[A-Z][A-Z0-9_]*$/

/** 把名字切成词：常量按 `_`，类型按 CamelCase（`DynVdfsProvider` → `[Dyn,Vdfs,Provider]`） */
function words(name) {
  if (CONST_RE.test(name)) return name.split('_').filter(Boolean)
  return name.match(/[A-Z][a-z0-9]*/g) ?? [name]
}

/** 名字（跳过开头的限定词后）的词序列是否以该前缀的词序列开头 */
function matchesPrefix(name, prefix, modifiers) {
  const pw = words(prefix)
  let sw = words(name)
  while (sw.length && modifiers.has(sw[0])) sw = sw.slice(1)
  if (sw.length < pw.length) return false
  return pw.every((w, i) => sw[i] === w)
}

// ── 汇总输出 ─────────────────────────────────────────────────────────────

let errors = 0
const hitsByRule = new Map()
function report(rule, where, message) {
  console.log(`${red('[ERROR]')} ${rule} ${where}  ${message}`)
  errors++
  hitsByRule.set(rule, (hitsByRule.get(rule) ?? 0) + 1)
}

const RULE_NAMES = {
  'N-001': '前缀落在所属域登记的前缀里',
  'N-002': '一个前缀只属于一个域',
  'N-003': '不借用别的域的前缀',
  'N-004': '域目录与表一一对应',
  'N-005': '公开面符号都有归属域',
}

// ── 主流程 ───────────────────────────────────────────────────────────────

let rules
let modifiers
let surface
try {
  ;({ rules, modifiers } = readRules())
  surface = collectCoreSurface(ROOT)
} catch (e) {
  console.error(red(`✗ 规则源不可解析：${e.message}`))
  console.error(dim(`  （守卫读不到规则时必须红——绿灯只说明没检查，不说明合规）`))
  process.exit(1)
}

// N-004：域目录 ↔ 表，双向一一对应
const dirs = [...surface.domains].sort()
for (const d of dirs) {
  if (!rules.has(d)) {
    report('N-004', `symbio_core/${d}/`, `域目录存在但 README §1.2 的表里没有它的前缀规则`)
  }
}
for (const d of [...rules.keys()].sort()) {
  if (!surface.domains.has(d)) {
    report('N-004', `README §1.2`, `表里登记了域 \`${d}\`，但 \`symbio_core/${d}/\` 目录不存在`)
  }
}

// N-002：前缀跨域复用
const prefixOwner = new Map() // 前缀 → 域[]
for (const [d, r] of rules) {
  for (const spec of [r.typeSpec, r.constSpec]) {
    if (spec.kind !== 'prefix') continue
    for (const p of spec.prefixes) {
      if (!prefixOwner.has(p)) prefixOwner.set(p, [])
      if (!prefixOwner.get(p).includes(d)) prefixOwner.get(p).push(d)
    }
  }
}
for (const [p, ds] of [...prefixOwner].sort()) {
  if (ds.length > 1) {
    report(
      'N-002',
      'README §1.2',
      `前缀 \`${p}\` 被 ${ds.length} 个域登记：${ds.join(' / ')}` +
        ` —— 「一域一前缀」，共用前缀就是「一个前缀两种东西」，调用点反而更难读`,
    )
  }
}

// 全部登记前缀的扁平表（用于 N-003 报「该前缀属于谁」）
const allPrefixes = []
for (const [d, r] of rules) {
  for (const spec of [r.typeSpec, r.constSpec]) {
    if (spec.kind !== 'prefix') continue
    for (const p of spec.prefixes) allPrefixes.push({ prefix: p, domain: d })
  }
}

// N-001 / N-003 / N-005：逐个公开符号
const rows = [...surface.symbols].sort((a, b) => a[0].localeCompare(b[0]))
let checked = 0
let skippedFn = 0
for (const [name, domain] of rows) {
  const kind = kindOf(name)
  if (kind === 'fn') {
    skippedFn++
    continue
  }
  // N-005：无归属域（根 mod.rs 直接声明 / 根显式重导出且未标域）
  if (domain === '(root)' || domain === '(explicit)') {
    report(
      'N-005',
      `symbio_core/mod.rs`,
      `\`${name}\`（${kind}）没有归属域 —— 请移入对应域，` +
        `否则「符号带域前缀」无从核对`,
    )
    continue
  }
  const rule = rules.get(domain)
  if (!rule) continue // N-004 已报过，不重复刷屏
  checked++
  const spec = kind === 'type' ? rule.typeSpec : rule.constSpec
  const what = kind === 'type' ? '类型' : '常量'

  if (spec.kind === 'exempt' || spec.kind === 'bare') continue

  if (spec.kind === 'suffix') {
    if (!name.endsWith(spec.suffix)) {
      report(
        'N-001',
        `symbio_core/${domain}/`,
        `\`${name}\`（${what}）不以 \`${spec.suffix}\` 结尾 —— \`${domain}\` 域的${what}用该后缀`,
      )
    }
    continue
  }

  if (spec.kind === 'prefix') {
    if (spec.prefixes.some((p) => matchesPrefix(name, p, modifiers))) continue
    const foreign = allPrefixes.find(
      (x) => x.domain !== domain && matchesPrefix(name, x.prefix, modifiers),
    )
    if (foreign) {
      report(
        'N-003',
        `symbio_core/${domain}/`,
        `\`${name}\`（${what}）用的是 \`${foreign.domain}\` 域的前缀 \`${foreign.prefix}\` —— ` +
          `它要么改名，要么搬去 \`${foreign.domain}\``,
      )
    } else {
      report(
        'N-001',
        `symbio_core/${domain}/`,
        `\`${name}\`（${what}）不匹配 \`${domain}\` 域登记的任何前缀 ` +
          `（${spec.prefixes.map((p) => `\`${p}\``).join(' / ')}）`,
      )
    }
    continue
  }

  // spec.kind === 'none'
  report(
    'N-001',
    `symbio_core/${domain}/`,
    `\`${name}\`（${what}）—— \`${domain}\` 域没有这一类符号（README §1.2 的该格是「—」）`,
  )
}

// ── 汇总 ─────────────────────────────────────────────────────────────────
console.log('')
console.log(
  dim(
    `── 命名核对：${checked} 个类型/常量 · ${skippedFn} 个函数（不判）· ` +
      `${rules.size} 个域 · ${modifiers.size} 个限定词 ──`,
  ),
)
for (const [rule, name] of Object.entries(RULE_NAMES)) {
  const n = hitsByRule.get(rule) ?? 0
  console.log(`  ${n === 0 ? green('✓') : red('✗')} ${rule}  ${name}${n ? `  (${n})` : ''}`)
}
console.log('')

if (errors > 0) {
  console.log(red(`  ${errors} 个违规`))
  process.exit(1)
}
console.log(green(`  ${Object.keys(RULE_NAMES).length} 条规则全部通过`))
process.exit(0)
