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
 * | SV-001 | ① **有结构**的名字（含 `.` 或 `/`）出现在两个以上非 `plugin-api` 的插件生产代码里；② 同名 `const` 在两个以上插件 crate 各声明一次 | 两份字面量 = 两个 owner：改名只改一处，另一处静默漂移；或绕过词汇层把名字抄回实现 |
 * | SV-002 | `symbio-plugin-api` 的**非生成**生产代码里出现任一 manifest 视图名（`view_names.rs` 里的 `shared_views` 投影） | 业务视图名渗进机制层：机制层于是「认识」某个 owner 的视图，横扩又要改门面 |
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
 * 退出码：1 = 有违规；2 = 范围读不出（缺 `crates/` 或没有任何插件 crate）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns SV 共享词汇
// @codes SV-001 SV-002

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { red, green } from './color.mjs'

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

/** 生产代码里的 `.rs` 文件（测试文件不算：它们不是跨插件的词汇消费方）。 */
function productionFiles(dir) {
  const out = []
  const walk = (d) => {
    if (!fs.existsSync(d)) return
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      if (e.name === 'target' || e.name.startsWith('.')) continue
      const p = path.join(d, e.name)
      if (e.isDirectory()) walk(p)
      else if (p.endsWith('.rs') && !/test/i.test(e.name)) out.push(p)
    }
  }
  walk(dir)
  return out
}

const API_SRC = path.join(ROOT, 'crates', 'symbio-plugin-api', 'src')
const VIEW_NAMES_RS = path.join(API_SRC, 'view_names.rs')

/** `view_names.rs` 里公开的视图名（只认生成物里的字符串常量）。 */
function manifestViewNames(file) {
  if (!fs.existsSync(file)) return []
  const text = fs.readFileSync(file, 'utf8')
  const names = new Set()
  for (const m of text.matchAll(
    /pub const [A-Z][A-Z0-9_]*\s*:\s*&(?:'static\s+)?str\s*=\s*"([^"]+)"/g,
  )) {
    if (VOCAB_RE.test(m[1]) && STRUCTURED_RE.test(m[1])) names.add(m[1])
  }
  return [...names]
}

/** 生成物特征：文件头声明「由 `scripts/facts/…` 生成」。 */
function isGenerated(text) {
  return /由 `scripts\/facts\//.test(text.slice(0, 600))
}

/**
 * SV-002：机制层（`symbio-plugin-api`）的非生成生产代码里出现任一 manifest 视图名。
 *
 * 视图名属于 owner 的业务身份，只该由 `submit_reports!` 交给读方；写进门面就意味着
 * 「加一条视图要改机制层」。`view_names.rs` 是名字的生成地、生成物整体不判。
 */
function mechanismViewNameErrors() {
  const names = manifestViewNames(VIEW_NAMES_RS)
  if (names.length === 0) return []
  const out = []
  for (const abs of productionFiles(API_SRC)) {
    if (path.resolve(abs) === path.resolve(VIEW_NAMES_RS)) continue
    const raw = fs.readFileSync(abs, 'utf8')
    if (isGenerated(raw)) continue
    const text = stripComments(raw)
    const rel = path.relative(ROOT, abs).replace(/\\/g, '/')
    for (const name of names) {
      const needle = `"${name}"`
      let at = text.indexOf(needle)
      while (at !== -1) {
        const line = text.slice(0, at).split('\n').length
        out.push(
          `${rel}:${line} 写了 "${name}"——业务视图名不该出现在机制层（它只该由 owner 的 ` +
            'submit_reports! 交出，读方按登记遍历）',
        )
        at = text.indexOf(needle, at + needle.length)
      }
    }
  }
  return out
}

function pluginCrates(root) {
  const crates = path.join(root, 'crates')
  if (!fs.existsSync(crates)) {
    throw new Error('范围读不出：缺少 crates/')
  }
  const ids = fs
    .readdirSync(crates, { withFileTypes: true })
    .filter((e) => e.isDirectory() && e.name.startsWith('symbio-plugin-') && e.name !== 'symbio-plugin-api')
    .map((e) => e.name)
  if (ids.length === 0) throw new Error('范围读不出：crates/ 下没有任何 symbio-plugin-* 插件 crate')
  return ids
}

let crates
try {
  crates = pluginCrates(ROOT)
} catch (e) {
  console.log(`${red('[ERROR]')} shared-name-audit：${e.message}`)
  process.exit(2)
}

/** value → Map(crate → Set(文件相对路径))；只收词汇形字面量。 */
const literalHomes = new Map()
/** value → Map(crate → Set(const 名 + 文件))；只收字符串常量。 */
const constHomes = new Map()

for (const crate of crates) {
  for (const abs of productionFiles(path.join(ROOT, 'crates', crate, 'src'))) {
    const rel = path.relative(ROOT, abs).replace(/\\/g, '/')
    const text = stripComments(fs.readFileSync(abs, 'utf8'))
    for (const m of text.matchAll(CONST_RE)) {
      const value = m[2]
      if (!VOCAB_RE.test(value)) continue
      if (!constHomes.has(value)) constHomes.set(value, new Map())
      const per = constHomes.get(value)
      if (!per.has(crate)) per.set(crate, new Set())
      per.get(crate).add(`${m[1]} @ ${rel}`)
    }
    for (const m of text.matchAll(/"([^"\\\n]*)"/g)) {
      const value = m[1]
      if (!VOCAB_RE.test(value)) continue
      if (!literalHomes.has(value)) literalHomes.set(value, new Map())
      const per = literalHomes.get(value)
      if (!per.has(crate)) per.set(crate, new Set())
      per.get(crate).add(rel)
    }
  }
}

const errors = []

// ① 有结构的名字在 ≥2 插件 crate 各写一次字面量。
for (const [value, per] of [...literalHomes.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
  if (per.size < 2) continue
  if (!STRUCTURED_RE.test(value)) continue
  if (isMime(value) || isFileName(value)) continue
  const where = [...per.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([crate, files]) => `${crate}（${[...files].join('、')}）`)
    .join('；')
  errors.push(
    `"${value}" 在 ${per.size} 个插件 crate 各写了一次字面量：${where}——` +
      '共享名应住 `symbio-plugin-api` 的词汇面（或清单投影），两侧 import 同一常量（D-364）',
  )
}

// ② 同名 const 在两个插件各声明一次。
for (const [value, per] of [...constHomes.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
  if (per.size < 2) continue
  if (isMime(value) || isFileName(value)) continue
  const where = [...per.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([crate, decls]) => `${crate}（${[...decls].join('、')}）`)
    .join('；')
  errors.push(
    `"${value}" 的 const 在 ${per.size} 个插件 crate 各声明一次：${where}——` +
      '同一拼写两个家，改名只改一处就漂移；共享常量应住词汇层，其余改成 import（D-364）',
  )
}

const viewNameErrors = mechanismViewNameErrors()

if (errors.length || viewNameErrors.length) {
  for (const e of errors) console.log(red(`✗ SV-001 ${e}`))
  for (const e of viewNameErrors) console.log(red(`✗ SV-002 ${e}`))
  console.log(`\n共享词汇：${errors.length + viewNameErrors.length} 条违规`)
  process.exit(1)
}

const skipped = [...literalHomes.entries()].filter(
  ([v, per]) => STRUCTURED_RE.test(v) && per.size >= 2 && (isMime(v) || isFileName(v)),
).length
console.log(
  green(
    `✓ SV-001 共享词汇：${crates.length} 个插件 crate 的生产代码里，` +
      '没有「同一有结构名字出现在两个 crate」的字面量，也没有同名 const 的两个家' +
      (skipped ? `（另有 ${skipped} 条 MIME / 文件名形态，按语法形状不判）` : ''),
  ),
)
console.log(
  green(
    `✓ SV-002 机制层没有业务视图名：symbio-plugin-api 的非生成生产代码里没有 manifest 视图名（${manifestViewNames(VIEW_NAMES_RS).length} 个名字待守）`,
  ),
)
