#!/usr/bin/env node
/**
 * core-surface-audit — `symbio_core` 公开面的**消费方数量**报告（报告型，不判失败）
 *
 * ## 为什么需要它
 *
 * ADR-023 的判据是**依赖方数量**：「只被一个模块依赖的内容一律下沉回该模块」。
 * 这条规则写在 `symbio_core/README.md` 与 ADR 里，但**此前没有任何脚本在判它** ——
 * 2026-09-26 那次审计是靠人工逐条 grep 的，且当场就漏报了（`TurnToolCallAccumulator`
 * 的真实消费方 `plugins/model/stream.rs` 走的是**字段访问** `.tool_accumulator`，
 * 类型名根本不出现 ⇒ 按类型名 grep 数不到）。
 *
 * 本脚本把那次人工统计固化成可重复执行的报告。
 *
 * ## 它是**报告型**，不是判定型
 *
 * 「只有 1 个消费方」不等于「该下沉」——`ExecTranscriptWriter` 只有一个消费方，
 * 但它存在的理由是**避免 core 反向依赖 session 的 `Transcript`**（宿主接缝）；
 * `PluginStopReason` 只有一个调用方，但它是 `Plugin::stop` 的**形参类型**。
 * 所以本脚本只**列出**候选，判定由人做（判据见 `symbio_core/README.md` §4 四问）。
 * 退出码恒 0 —— 但正因如此它**更需要回归测试**（`core-surface-audit.test.mjs`）。
 *
 * ## 计数口径（四条，都影响结果，别凭直觉）
 *
 * 1. **按「模块」不按「文件」**：`symbio/src/plugins/<名>/` 下任意深度的文件都算
 *    同一个消费方。`cli/` 与 `tauri/src-tauri/` 各算**一个**跨 crate 消费方
 *    （它们是独立 crate，与 ADR-023 里 `EventBusSubscribeRequest` 的保留理由同源）。
 * 2. **剥注释后再匹配**：文档注释里提到一个符号不代表依赖它。
 *    ⚠️ 这条会**漏报字段访问型消费点**（见上面 `TurnToolCallAccumulator` 的例子）——
 *    所以「0 个消费方」只说明**名字没出现**，动手前先确认它是不是只经字段被用到。
 * 3. **一个文件都不能跳过**：`symbio/src/` 直属文件（`lib.rs`、`plugins/mod.rs` 这类
 *    **不在插件子目录里**的）也算消费方。第一版把它们跳过了，于是「只在注册表里被
 *    用到」的符号被算成 0 —— `PluginErrorCode` / `PluginIdentity` 一族全部假报。
 *    **「数不到」与「真的没人用」是两件事。**
 * 4. **公开面 = 根 `mod.rs` 的重导出**，不是「域目录下所有 `pub`」：私有子模块里的
 *    `pub` 从 crate 外够不着（`TOOL_NAME_WIRE_SEPARATOR` 就是这种），把域内所有
 *    `pub` 都算进来会凭空多出一批「零消费方」。
 *
 * ## 用法
 *   node scripts/core-surface-audit.mjs            # 报告
 *   node scripts/core-surface-audit.mjs --verbose  # 附每个符号的消费方清单
 *   node scripts/core-surface-audit.mjs --root=<dir>   # 指向别处的仓库（测试用）
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'

const argv = process.argv.slice(2)
const VERBOSE = argv.includes('--verbose')
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const CORE_REL = 'symbio/src/symbio_core'
const CORE = path.join(ROOT, CORE_REL)

/** 跨 crate 消费方：整个 crate 算一个单位（它们是独立 crate，与插件模块不可比） */
const CROSS_CRATE = [
  { rel: 'cli/src', label: 'cli' },
  { rel: 'tauri/src-tauri/src', label: 'tauri-shell' },
]

function* walk(dir) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of entries) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) yield* walk(p)
    else yield p
  }
}

/** 去掉块注释与行注释（文档注释里引用符号不算依赖） */
function stripComments(src) {
  return src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')
}

/**
 * 从 `pub use …;` 语句里抽名字。
 *
 * ⚠️ **通配必须收集成一个数组，不能用 Map 的固定键**（这里错过一次）：
 * 根 `mod.rs` 有 `pub use plugin::*` / `pub use keys::*` / `pub use logger::*`
 * **三条**通配，用 `map.set('*', …)` 会让后一条覆盖前一条 —— 于是 `PLUGIN_*`、
 * `PathKey`、`PluginStopReason`、`KEY_*` 全部凭空消失，公开面少算 85 个符号。
 * 返回 `{ names, globs }`，`globs` 是「域 → 该域被通配导入」的数组。
 */
function parsePubUses(source, domains) {
  const names = new Map()
  const globs = []
  for (const m of stripComments(source).matchAll(/^[ \t]*pub use\s+([^;]+);/gm)) {
    const body = m[1].trim()
    const head = body.split('::')[0].trim()
    const domain = domains.has(head) ? head : null
    const brace = body.match(/\{([\s\S]*)\}/)
    if (brace) {
      for (const part of brace[1].split(',')) {
        const name = part.trim().split(/\s+as\s+/).pop().trim()
        if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) names.set(name, domain)
      }
      continue
    }
    if (/::\s*\*\s*$/.test(body)) {
      globs.push(domain ?? head)
      continue
    }
    const name = body.split('::').pop().trim()
    if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) names.set(name, domain)
  }
  return { names, globs }
}

/** 直接声明在某个 .rs 文件里的 `pub` 符号 */
function declaredIn(file) {
  const out = new Set()
  if (!fs.existsSync(file)) return out
  const src = stripComments(fs.readFileSync(file, 'utf8'))
  for (const m of src.matchAll(
    /^pub\s+(?:struct|enum|trait|union|type|const|static|fn)\s+([A-Za-z_][A-Za-z0-9_]*)/gm,
  )) {
    out.add(m[1])
  }
  // 宏生成的公开符号：`keys` 的 26 个字符串键（`define_string_key!(PathKey, PATH, "path")`）
  // 与它们的键类型都不带 `pub` 关键字，正则扫不到 —— 而它们恰是**消费方最多**的一批。
  // 这里按宏的形参位置取（`($类型, $常量, $键名)`），换宏就得跟着改。
  for (const m of src.matchAll(
    /^[ \t]*define_string_key!\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*,\s*([A-Za-z_][A-Za-z0-9_]*)/gm,
  )) {
    out.add(m[1])
    out.add(m[2])
  }
  return out
}

const rootMod = path.join(CORE, 'mod.rs')
if (!fs.existsSync(rootMod)) {
  console.error(red(`✗ 找不到 ${path.relative(ROOT, rootMod)}`))
  process.exit(1)
}

const domains = new Set(
  fs
    .readdirSync(CORE, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name),
)

const rootSource = fs.readFileSync(rootMod, 'utf8')
const rootUses = parsePubUses(rootSource, domains)

/** 名字 → 定义域（`(root)` = 直接声明在根 mod.rs） */
const symbols = new Map()
for (const [name, domain] of rootUses.names) symbols.set(name, domain ?? '(explicit)')
for (const n of declaredIn(rootMod)) symbols.set(n, '(root)')

// 展开通配：`pub use <域>::*` ⇒ 该域 mod.rs 的重导出 + 直接声明（不含私有子模块里的 pub）
for (const domain of rootUses.globs) {
  const domMod = path.join(CORE, domain, 'mod.rs')
  if (!fs.existsSync(domMod)) continue
  const dom = parsePubUses(fs.readFileSync(domMod, 'utf8'), domains)
  for (const [n] of dom.names) if (!symbols.has(n)) symbols.set(n, domain)
  for (const n of declaredIn(domMod)) symbols.set(n, domain)
}

// ==================== 数消费方 ====================

/**
 * 文件 → 消费方单位。**永远返回一个单位，绝不返回 null**（口径 3）。
 */
function unitOf(rel) {
  const norm = rel.split(path.sep).join('/')
  const m = norm.match(/^symbio\/src\/plugins\/([^/]+)\//)
  if (m) return `plugins/${m[1]}`
  const m2 = norm.match(/^symbio\/src\/providers\/([^/]+)\//)
  if (m2) return `providers/${m2[1]}`
  if (/^symbio\/src\/plugins\/[^/]+$/.test(norm)) return 'plugins/(registry)'
  if (norm.startsWith('symbio/src/')) return 'symbio/(crate root)'
  for (const c of CROSS_CRATE) {
    if (norm.startsWith(c.rel)) return c.label
  }
  return null
}

const allNames = [...symbols.keys()]
const consumers = new Map()
for (const scanRoot of ['symbio/src', 'cli/src', 'tauri/src-tauri/src'].map((r) => path.join(ROOT, r))) {
  for (const file of walk(scanRoot)) {
    if (!file.endsWith('.rs')) continue
    const rel = path.relative(ROOT, file)
    if (rel.split(path.sep).join('/').startsWith(CORE_REL + '/')) continue // core 自身不算消费方
    const unit = unitOf(rel)
    if (!unit) continue
    const src = stripComments(fs.readFileSync(file, 'utf8'))
    for (const name of allNames) {
      if (!new RegExp(`\\b${name}\\b`).test(src)) continue
      if (!consumers.has(name)) consumers.set(name, new Set())
      consumers.get(name).add(unit)
    }
  }
}

// ==================== 报告 ====================

const rows = allNames.map((name) => ({
  name,
  domain: symbols.get(name),
  units: [...(consumers.get(name) ?? [])].sort(),
}))

/**
 * 自引用：插件 id 常量（`PLUGIN_<X>` / `EMBEDDING_<X>`）被**同名插件**使用。
 * 这是**正常**的——常量就是那个插件自己的名字，不该算「下放候选」。
 */
function isSelfReference(r) {
  const m = r.name.match(/^(?:PLUGIN|EMBEDDING)_([A-Z]+)$/)
  if (!m || r.units.length !== 1) return false
  const slug = m[1].toLowerCase()
  return r.units[0] === `plugins/${slug}` || r.units[0] === `providers/${slug}`
}

const zero = rows.filter((r) => r.units.length === 0)
const one = rows.filter((r) => r.units.length === 1 && !isSelfReference(r))
const selfRef = rows.filter(isSelfReference)
const many = rows.filter((r) => r.units.length >= 2)

console.log(`\n== symbio_core 公开面消费方统计 ==`)
console.log(
  `   公开符号 ${rows.length} · ≥2 消费方 ${many.length} · 仅 1 个 ${one.length}` +
    ` · 自引用 ${selfRef.length} · 0 个 ${zero.length}`,
)

const byName = (a, b) => a.domain.localeCompare(b.domain) || a.name.localeCompare(b.name)

console.log(`\n--- 仅 1 个消费方（ADR-023 的**下放候选**，逐条判定，见 README §4 四问）---`)
console.log(`    ⚠️ 动手前先排除两类误判：① 它是某个**多消费方函数的形参/返回类型**吗？`)
console.log(`       ② 消费点是不是走**字段访问**（类型名不出现）？`)
if (one.length === 0) console.log('  （无）')
for (const r of one.sort(byName)) {
  console.log(`  ${(r.domain ?? '?').padEnd(12)} ${r.name.padEnd(32)} -> ${r.units.join(', ')}`)
}

console.log(`\n--- 0 个消费方（**名字**未在本 crate 外出现；先确认是否只经字段访问被用到）---`)
if (zero.length === 0) console.log('  （无）')
for (const r of zero.sort(byName)) {
  console.log(`  ${(r.domain ?? '?').padEnd(12)} ${r.name}`)
}

if (VERBOSE) {
  console.log(`\n--- 自引用（插件 id 常量被它自己用；正常，不必处置）---`)
  for (const r of selfRef.sort(byName)) {
    console.log(`  ${(r.domain ?? '?').padEnd(12)} ${r.name.padEnd(32)} -> ${r.units.join(', ')}`)
  }
  console.log(`\n--- 全部符号 ---`)
  for (const r of rows.sort(byName)) {
    console.log(
      `  [${String(r.units.length).padStart(2)}] ${(r.domain ?? '?').padEnd(12)} ${r.name.padEnd(32)} ${dim(r.units.join(', '))}`,
    )
  }
}

console.log(
  `\n${yellow('提示')}：本报告**不判失败**。「只有 1 个消费方」不等于「该下沉」——` +
    `宿主接缝（\`ExecTranscriptWriter\`）、跨 crate 契约（\`EventBusSubscribeRequest\`）、` +
    `trait 形参类型（\`PluginStopReason\`）都只有一个消费方而必须留在 core。` +
    `判据见 ${green('symbio_core/README.md §4')}。`,
)
