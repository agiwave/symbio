#!/usr/bin/env node
/**
 * core-surface-audit — `symbio_core` 公开面的**消费方数量**报告（报告型，不判失败）
 *
 * ## 为什么需要它
 *
 * ADR-023 的判据是**依赖方数量**：「只被一个模块依赖的内容一律下沉回该模块」。
 * 这条规则写在 `symbio_core/README.md` 与 ADR 里，但**此前没有任何脚本在判它** ——
 * 2026-09-26 那次审计是靠人工逐条 grep 的，且当场就漏报了：`TurnToolCallAccumulator`
 * （当时住 `symbio_core::llm::turn`）的真实消费方 `plugins/model/stream.rs` 走的是
 * **字段访问** `.tool_accumulator`，类型名根本不出现 ⇒ 按类型名 grep 数不到。
 * （该符号其后已随 core 收口迁出 `symbio_core`，现住 `plugins/model/tool_accumulator.rs`。
 * 仍记在这里，是因为它正是下面口径 2「字段访问型消费点」这个警告的来历。）
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
 * ## 计数口径
 *
 * **口径与解析收在 [`core-surface.mjs`](./core-surface.mjs)**，本脚本与
 * `core-naming-audit.mjs` 共用同一份「公开面是什么」的答案——各写一份必然演化成
 * 两套口径（报告说 A、判定说 B）。四条口径的全文见该模块文件头，摘要：
 *
 * 1. 按「模块」不按「文件」数消费方；`cli/` 与 `tauri/src-tauri/` 各算**一个**
 *    跨 crate 消费方。
 * 2. 剥注释后再匹配 —— 这会**漏报字段访问型消费点**（见上面 `TurnToolCallAccumulator`
 *    的例子），所以「0 个消费方」只说明**名字没出现**。
 * 3. 一个文件都不能跳过：`symbio/src/` 直属文件也算消费方。**「数不到」与「真的没人
 *    用」是两件事。**
 * 4. 公开面 = 根 `mod.rs` 的重导出，不是「域目录下所有 `pub`」。
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
import { walk, stripComments, collectCoreSurface } from './core-surface.mjs'

const argv = process.argv.slice(2)
const VERBOSE = argv.includes('--verbose')
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const CORE_REL = 'symbio/src/symbio_core'

/** 跨 crate 消费方：整个 crate 算一个单位（它们是独立 crate，与插件模块不可比） */
const CROSS_CRATE = [
  { rel: 'cli/src', label: 'cli' },
  { rel: 'tauri/src-tauri/src', label: 'tauri-shell' },
]

let surface
try {
  surface = collectCoreSurface(ROOT)
} catch (e) {
  console.error(red(`✗ ${e.message}`))
  process.exit(1)
}
const symbols = surface.symbols

// ==================== 数消费方 ====================

/**
 * 文件 → 消费方单位。**永远返回一个单位，绝不返回 null**（口径 3）。
 *
 * 返回 `null` 会让调用方 `continue` 跳过该文件，于是定义在那里的符号被算成
 * 「0 个消费方」——`PluginErrorCode` / `PluginIdentity` 一族就这样被假报过。
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
 * 自引用：插件 id 常量（`PLUGIN_ID_<X>`）与嵌入服务 id（`EMBEDDING_<X>`）被
 * **同名插件 / provider** 使用。这是**正常**的——常量就是它自己的名字，不该算
 * 「下放候选」。
 */
function isSelfReference(r) {
  const m = r.name.match(/^(?:PLUGIN_ID|EMBEDDING)_([A-Z]+)$/)
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
  console.log(`\n--- 自引用（插件 id / 嵌入服务 id 被它自己用；正常，不必处置）---`)
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
