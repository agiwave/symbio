#!/usr/bin/env node
/**
 * 共享词汇判据 —— 同一个名字在两个插件 crate 各写一遍字面量即红
 *
 * ## 它判什么
 *
 * 名字属于词汇层，实现属于各自插件（D-364）。共享名（端口 / 事件 / 路由 / 行为 /
 * 视图 / 路径段 / 详情条件键）住宿主 `symbio-plugin-api` 的词汇面或清单投影，
 * 两个 crate 都 import 同一常量；插件之间仍不互相依赖（E-009）。
 *
 * | 编号 | 判什么 | 它拦的是哪一种腐烂 |
 * |---|---|---|
 * | SV-001 | ① **有结构**的名字（含 `.` 或 `/`）出现在两个以上非 `plugin-api` 的插件生产代码里；② 同名 `const` 在两个以上插件 crate 各声明一次；③ 机制层（`symbio-sdk` / `symbio-kernel` / `symbio`）的生产代码里出现**词汇层已有的事件名**字面量（词汇家 = `symbio-plugin-api/src/events.rs`，owner 声明补私有名） | 两份字面量 = 两个 owner：改名只改一处，另一处静默漂移；或绕过词汇层把名字抄回实现（D-364 / D-392） |
 * | SV-003 | 全仓 `submit_reports!` 每条登记的 `(slot, fleet)` 与 `024` 的出品契约表**双向**逐行相等 | 加了列 / 改了 slot 却没更新契约，或表里留着一条已经删掉的列——出口形状与声称的形状静默分叉 |
 *
 * ## 为什么扫 `test_fixtures.rs`，又跳过生成物
 *
 * `crates/symbio-sdk/src/test_fixtures.rs` 是 core 用例唯一的事件拼写面。它若手抄
 * 词汇层的名字，正是本判据要拦的第二份定义；所以这个文件名**不按「测试文件」排除**。
 * 但它（以及任何事实生成物）由生成器投影、首行带「生成，请勿手改」标记——带标记的
 * 文件整体跳过：**同一份 owner 声明生成两处产物**是允许的，手抄副本不是（D-392）。
 *
 * ## 为什么不扫普通单词
 *
 * `"tool"` 同时是端口名与消息角色，`"inbox"` 同时是路径段与消息种类。字面量扫描
 * 分不清同形异义，把它们判重复只会逼人给不同概念起不同的名字。这类名字由 ②
 * （同名 `const` 两个家）守：一个拼写若真需要一个常量家，它就不该有两个。
 *
 * ## 不设手写豁免表
 *
 * 唯一的「不判」是两类**有语法形状**的非词汇：标准 MIME 类型（`<type>/<subtype>`，
 * type 取自 IANA 顶层集合）与文件名（`*.rs` / `*.json` / …）。前者 owner 是 IANA
 * 注册表，后者是路径不是名字——都没有「两个插件要约定同一个拼写」的含义。
 * 没有按名字逐条开的口子。
 *
 * 用法：node scripts/shared-name-audit.mjs [--root=<目录>]
 * 退出码：1 = 有违规；2 = 范围 / 规则源读不出（缺 `crates/`、没有任何插件 crate、
 * 缺机制层 crate、缺词汇层 `events.rs`，或契约表 / `submit_reports!` 的 `slot` / `fleet`
 * 解析不了）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns SV 共享词汇
// @codes SV-001 SV-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { red, green } from './color.mjs'
import { pluginDeclarationFaces } from './facts/events-rs.mjs'

const argv = process.argv.slice(2)
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

/** 词汇形：小写字母 / 数字 / `_`，可带 `.` 或 `/` 分段。 */
const VOCAB_RE = /^[a-z][a-z0-9_]*(?:[./][a-z][a-z0-9_]*)*$/
/** 有结构 = 至少一个 `.` 或 `/`。 */
const STRUCTURED_RE = /[./]/
/** 标准 MIME 类型的顶层 type（IANA）。 */
const MIME_TOP = new Set(['application', 'audio', 'font', 'image', 'message', 'model', 'multipart', 'text', 'video'])
/** 文件名后缀：路径不是名字。 */
const FILE_EXT_RE = /\.(rs|md|json|ya?ml|toml|txt|lock|log|html|css|js|mjs|ts|tsx|vue)$/
/** 字符串常量声明（只认字符串字面量初始化；别名 `use … as X` 不算）。 */
const CONST_RE = /(?:pub(?:\([^)]*\))?\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)\s*:[^=]*=\s*"([^"]*)"/g

function isMime(v) {
  const i = v.indexOf('/')
  if (i <= 0 || v.indexOf('/', i + 1) !== -1) return false
  return MIME_TOP.has(v.slice(0, i))
}

function isFileName(v) {
  return FILE_EXT_RE.test(v)
}

/** 去掉注释但保留字符串字面量（注释里的名字不是代码事实）。 */
function stripComments(s) {
  let out = ''
  let i = 0
  while (i < s.length) {
    if (s[i] === '/' && s[i + 1] === '/') {
      while (i < s.length && s[i] !== '\n') i++
    } else if (s[i] === '/' && s[i + 1] === '*') {
      i += 2
      let depth = 1
      while (i < s.length && depth) {
        if (s[i] === '/' && s[i + 1] === '*') {
          depth++
          i += 2
        } else if (s[i] === '*' && s[i + 1] === '/') {
          depth--
          i += 2
        } else i++
      }
    } else if (s[i] === '"') {
      out += s[i++]
      while (i < s.length) {
        out += s[i]
        if (s[i] === '\\') {
          out += s[i + 1] ?? ''
          i += 2
          continue
        }
        if (s[i] === '"') {
          i++
          break
        }
        i++
      }
    } else out += s[i++]
  }
  return out
}

/**
 * 生产代码里的 `.rs` 文件（测试文件不算：它们不是跨插件的词汇消费方）。
 *
 * 例外是 `test_fixtures.rs`：它不是普通测试文件，而是 core 用例**唯一的事件拼写面**。
 * 它若手抄词汇层的名字，正是本判据要拦的第二份定义；生成物由 {@link isGenerated} 另判
 * （见文件头「为什么扫 `test_fixtures.rs`」）。
 */
function productionFiles(dir) {
  const out = []
  const walk = (d) => {
    if (!fs.existsSync(d)) return
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      if (e.name === 'target' || e.name.startsWith('.')) continue
      const p = path.join(d, e.name)
      if (e.isDirectory()) walk(p)
      else if (p.endsWith('.rs') && (!/test/i.test(e.name) || e.name === 'test_fixtures.rs')) out.push(p)
    }
  }
  walk(dir)
  return out
}

/**
 * 生成物标记：首五行里带「生成，请勿手改」。
 *
 * 事实生成物（`events.rs`、`test_fixtures.rs` 等）与手写实现同形，但它的重复是
 * **单源投影**的结果，不是第二份定义。允许它；手写副本没有这行标记，照样红（D-392）。
 */
function isGenerated(text) {
  return text.split('\n', 5).some((line) => /生成[，,]?\s*请勿手改/.test(line))
}

const PLAN_REL = path.join('docs', '003-design', '024-能力横向扩充的通路.md')

/**
 * 出品契约表：`[{ slot, topPath, fleet }]`，`fleet` 为 `null` 表示「无实例级落点」。
 *
 * 表住 `024` 的「报表列注册面」节，表头首格是 `` `slot` ``。文件不在 / 找不到表 ⇒
 * `null`（规则源读不出，调用方 exit 2）；表在但没有数据行 ⇒ `[]`（合法的空契约）。
 */
function readReportContract(root) {
  const abs = path.join(root, PLAN_REL)
  if (!fs.existsSync(abs)) return null
  const lines = fs.readFileSync(abs, 'utf8').split('\n')
  const start = lines.findIndex((line) => /^\|\s*`slot`\s*\|/.test(line.trim()))
  if (start < 0) return null
  const rows = []
  for (let i = start + 2; i < lines.length; i++) {
    const line = lines[i].trim()
    if (!line.startsWith('|')) break
    const cells = line
      .replace(/^\||\|$/g, '')
      .split('|')
      .map((s) => s.trim().replace(/`/g, ''))
    if (cells.length < 3) return null
    rows.push({
      slot: cells[0],
      topPath: cells[1],
      fleet: cells[2] === '—' || cells[2] === '-' ? null : cells[2],
    })
  }
  return rows
}

/**
 * 全仓 `submit_reports!` 里每条 `ReportDecl` 的 `(slot, fleet)`。
 *
 * `fleet` 归一成出口里的落点写法（`Some("by_tier")` → `fleet.by_tier`），好与契约表
 * 的直接对照。测试文件不算（`reports.test.rs` 里有造数的 `slot`）；解析不出 ⇒ `null`
 * ——宁可 exit 2，也不静默漏掉一条登记（漏掉会让 SV-003 变成假绿）。
 */
function registeredReports(root) {
  const cratesDir = path.join(root, 'crates')
  if (!fs.existsSync(cratesDir)) return null
  const bySlot = new Map()
  const duplicates = []
  for (const abs of productionFiles(cratesDir)) {
    const text = stripComments(fs.readFileSync(abs, 'utf8'))
    let at = 0
    while ((at = text.indexOf('submit_reports!', at)) >= 0) {
      const start = text.indexOf('[', at)
      if (start < 0) return null
      let depth = 0
      let end = -1
      for (let j = start; j < text.length; j++) {
        if (text[j] === '[') depth++
        else if (text[j] === ']' && --depth === 0) {
          end = j
          break
        }
      }
      if (end < 0) return null
      const block = text.slice(start, end)
      for (const m of block.matchAll(/ReportDecl\s*\{/g)) {
        const open = m.index + m[0].length - 1
        let braces = 0
        let close = -1
        for (let j = open; j < block.length; j++) {
          if (block[j] === '{') braces++
          else if (block[j] === '}' && --braces === 0) {
            close = j
            break
          }
        }
        if (close < 0) return null
        const body = block.slice(open, close + 1)
        const slot = body.match(/slot\s*:\s*"([^"]+)"/)
        const fleet = body.match(/fleet\s*:\s*(None|Some\s*\(\s*"([^"]+)"\s*\))/)
        if (!slot || !fleet) return null
        const landing = fleet[1] === 'None' ? null : `fleet.${fleet[2]}`
        if (bySlot.has(slot[1])) duplicates.push(slot[1])
        bySlot.set(slot[1], landing)
      }
      at = end + 1
    }
  }
  return { bySlot, duplicates }
}

/**
 * SV-003：登记与契约表**双向**逐行相等。
 *
 * 只按 `(slot, fleet)` 比——`topPath` 由 slot 拼出，比它只会多一个可以漂的副本。
 * 规则源任何一半读不出 ⇒ `null`（exit 2），不把「没读到」当成「没有」。
 */
function reportContractErrors(root) {
  const table = readReportContract(root)
  const reg = registeredReports(root)
  if (!table || !reg) return null
  const errors = []
  for (const slot of reg.duplicates) {
    errors.push(`slot \`${slot}\` 被登记了两次——一条报表列只有一个家`)
  }
  const tableBySlot = new Map()
  for (const row of table) {
    if (tableBySlot.has(row.slot)) {
      errors.push(`契约表里 slot \`${row.slot}\` 出现了两次`)
      continue
    }
    tableBySlot.set(row.slot, row)
  }
  for (const [slot, row] of tableBySlot) {
    if (!reg.bySlot.has(slot)) {
      errors.push(
        `契约表有 \`${slot}\`（\`${row.topPath}\`），全仓却没有任何 \`submit_reports!\` 登记` +
          '——表在说一条不存在的列',
      )
      continue
    }
    const got = reg.bySlot.get(slot)
    if (got !== row.fleet) {
      errors.push(
        `\`${slot}\` 的实例级落点不一致：契约表 \`${row.fleet ?? '—'}\`，登记 \`${got ?? '—'}\``,
      )
    }
  }
  for (const [slot, landing] of reg.bySlot) {
    if (!tableBySlot.has(slot)) {
      errors.push(
        `登记了 \`${slot}\`（实例级落点 \`${landing ?? '—'}\`），契约表里没有这一行` +
          '——先补 024 的契约表再登记',
      )
    }
  }
  return { errors, table, registered: reg.bySlot }
}

/** 机制层 crate：它们不许自己写词汇层的事件名，只能 import / 由生成器投影（D-392）。 */
const CORE_CRATES = ['symbio-sdk', 'symbio-kernel', 'symbio']

function auditCrates(root) {
  const crates = path.join(root, 'crates')
  if (!fs.existsSync(crates)) {
    throw new Error('范围读不出：缺少 crates/')
  }
  const plugins = fs
    .readdirSync(crates, { withFileTypes: true })
    .filter((e) => e.isDirectory() && e.name.startsWith('symbio-plugin-') && e.name !== 'symbio-plugin-api')
    .map((e) => e.name)
  if (plugins.length === 0) throw new Error('范围读不出：crates/ 下没有任何 symbio-plugin-* 插件 crate')
  const core = CORE_CRATES.filter((id) => fs.existsSync(path.join(crates, id, 'src')))
  if (core.length !== CORE_CRATES.length) {
    throw new Error(`范围读不出：机制层 crate 缺 ${CORE_CRATES.filter((x) => !core.includes(x)).join('、')}`)
  }
  return { plugins, core, all: [...plugins, ...core] }
}

const EVENTS_REL = path.join('crates', 'symbio-plugin-api', 'src', 'events.rs')

/**
 * 词汇层的事件名面：`events.rs` 的常量值（必需），加上 owner 声明补出来的私有名
 * （可选增强——夹具仓库没有插件清单时只以 `events.rs` 为准）。
 *
 * 返回 `Map(名字 → 家)`；`events.rs` 不在 ⇒ `null`（规则源读不出，exit 2）。
 */
function vocabulary(root) {
  const abs = path.join(root, EVENTS_REL)
  if (!fs.existsSync(abs)) return null
  const text = stripComments(fs.readFileSync(abs, 'utf8'))
  const names = new Map()
  const constNames = new Set()
  for (const m of text.matchAll(CONST_RE)) {
    names.set(m[2], `\`${EVENTS_REL.replace(/\\/g, '/')}\` 的 ${m[1]}`)
    constNames.add(m[1])
  }
  try {
    for (const [owner, face] of pluginDeclarationFaces(root)) {
      for (const name of face.cells.keys()) {
        if (!names.has(name)) names.set(name, `owner ${owner} 的 \`EventDecl\``)
      }
    }
  } catch {
    // 夹具仓库没有 PLUGIN.yml ⇒ 只以 events.rs 为准（私有名这一步只是增强）。
  }
  return { names, constNames }
}

let crates
let vocab
try {
  const scope = auditCrates(ROOT)
  crates = scope
  vocab = vocabulary(ROOT)
  if (!vocab) throw new Error(`范围读不出：缺少词汇层 ${EVENTS_REL.replace(/\\/g, '/')}`)
} catch (e) {
  console.log(`${red('[ERROR]')} shared-name-audit：${e.message}`)
  process.exit(2)
}

/** 插件生产代码：value → Map(crate → Set(文件相对路径))，只收词汇形字面量（①）。 */
const literalHomes = new Map()
/** 插件生产代码：value → Map(crate → Set(const 名 + 文件))，只收字符串常量（②）。 */
const constHomes = new Map()
/** 机制层生产代码：value → Map(crate → Set(文件相对路径))，词汇形字面量（③）。 */
const coreLiteralHomes = new Map()
/** 机制层生产代码：const 名 → Map(crate → Set(文件相对路径))（③）。 */
const coreConstHomes = new Map()

for (const crate of crates.all) {
  const isCore = crates.core.includes(crate)
  for (const abs of productionFiles(path.join(ROOT, 'crates', crate, 'src'))) {
    const rel = path.relative(ROOT, abs).replace(/\\/g, '/')
    const raw = fs.readFileSync(abs, 'utf8')
    // 生成物是单源投影，不是第二份定义（`events.rs` / `test_fixtures.rs` 都带这行标记）。
    if (isGenerated(raw)) continue
    const text = stripComments(raw)
    for (const m of text.matchAll(CONST_RE)) {
      const value = m[2]
      if (!VOCAB_RE.test(value)) continue
      if (isCore) {
        if (!coreConstHomes.has(m[1])) coreConstHomes.set(m[1], new Map())
        const per = coreConstHomes.get(m[1])
        if (!per.has(crate)) per.set(crate, new Set())
        per.get(crate).add(rel)
      } else {
        if (!constHomes.has(value)) constHomes.set(value, new Map())
        const per = constHomes.get(value)
        if (!per.has(crate)) per.set(crate, new Set())
        per.get(crate).add(`${m[1]} @ ${rel}`)
      }
    }
    for (const m of text.matchAll(/"([^"\\\n]*)"/g)) {
      const value = m[1]
      if (!VOCAB_RE.test(value)) continue
      const homes = isCore ? coreLiteralHomes : literalHomes
      if (!homes.has(value)) homes.set(value, new Map())
      const per = homes.get(value)
      if (!per.has(crate)) per.set(crate, new Set())
      per.get(crate).add(rel)
    }
  }
}

/** `Map(crate → Set(条目))` → 「crate（条目、条目）」的排序串。 */
function whereOf(per) {
  return [...per.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([crate, entries]) => `${crate}（${[...entries].join('、')}）`)
    .join('；')
}

const errors = []

// ① 有结构的名字在 ≥2 插件 crate 各写一次字面量。
for (const [value, per] of [...literalHomes.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
  if (per.size < 2) continue
  if (!STRUCTURED_RE.test(value)) continue
  if (isMime(value) || isFileName(value)) continue
  errors.push(
    `"${value}" 在 ${per.size} 个插件 crate 各写了一次字面量：${whereOf(per)}——` +
      '共享名应住 `symbio-plugin-api` 的词汇面（或清单投影），两侧 import 同一常量（D-364）',
  )
}

// ② 同名 const 在两个插件各声明一次。
for (const [value, per] of [...constHomes.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
  if (per.size < 2) continue
  if (isMime(value) || isFileName(value)) continue
  errors.push(
    `"${value}" 的 const 在 ${per.size} 个插件 crate 各声明一次：${whereOf(per)}——` +
      '同一拼写两个家，改名只改一处就漂移；共享常量应住词汇层，其余改成 import（D-364）',
  )
}

// ③ 机制层（`symbio-sdk` / `symbio-kernel` / `symbio`）不许自己写词汇层已有的事件名。
//   合法形态只有 import 同一常量，或由生成器从同源投影一份（带「生成」标记的整份跳过）。
for (const [value, per] of [...coreLiteralHomes.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
  const home = vocab.names.get(value)
  if (!home) continue
  errors.push(
    `词汇层已有事件名 "${value}"（${home}），机制层 crate 却自己写了一份字面量：` +
      `${whereOf(per)}——机制层只能 import 或由生成器投影（D-392）`,
  )
}
for (const [name, per] of [...coreConstHomes.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
  if (!vocab.constNames.has(name)) continue
  errors.push(
    `机制层 crate 声明了常量 \`${name}\`：${whereOf(per)}，与词汇层 ` +
      `\`${EVENTS_REL.replace(/\\/g, '/')}\` 同名——词表只有一个家（D-392）`,
  )
}

const contract = reportContractErrors(ROOT)
if (!contract) {
  console.log(
    `${red('[ERROR]')} shared-name-audit：SV-003 规则源读不出` +
      '（024 契约表或 `submit_reports!` 的 `slot` / `fleet` 解析不了）',
  )
  process.exit(2)
}

if (errors.length || contract.errors.length) {
  for (const e of errors) console.log(red(`✗ SV-001 ${e}`))
  for (const e of contract.errors) console.log(red(`✗ SV-003 ${e}`))
  console.log(`\n共享词汇：${errors.length + contract.errors.length} 条违规`)
  process.exit(1)
}

const skipped = [...literalHomes.entries()].filter(
  ([v, per]) => STRUCTURED_RE.test(v) && per.size >= 2 && (isMime(v) || isFileName(v)),
).length
console.log(
  green(
    `✓ SV-001 共享词汇：${crates.plugins.length} 个插件 crate + ${crates.core.length} 个机制层 crate 的生产代码里，` +
      '没有「同一有结构名字出现在两个 crate」的字面量，没有同名 const 的两个家，' +
      '机制层也没有自己写词汇层已有的事件名' +
      (skipped ? `（另有 ${skipped} 条 MIME / 文件名形态，按语法形状不判）` : ''),
  ),
)
const withFleet = contract.table.filter((row) => row.fleet !== null).length
console.log(
  green(
    `✓ SV-003 报表契约：${contract.table.length} 行与 ${contract.registered.size} 条登记逐行相等` +
      `（实例级落点 ${withFleet} 条非空）`,
  ),
)
