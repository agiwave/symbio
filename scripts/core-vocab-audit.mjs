#!/usr/bin/env node
/**
 * core-vocab-audit — `symbio_core` 的分域形状与依赖方向（V-001 判定 / V-002 判定 / 其余报告）
 *
 * ## 它治什么
 *
 * [D-087](../docs/003-design/001-核心架构.md "D-087") 把 `symbio_core/schemas/` 拆成
 * `chat` / `dialog` / `present` / `turn` 四个域，并给出两条**可执行**的判据：
 *
 * 1. `schemas` 目录不再存在——它是「暂时没处放」的同义词，重新长出来意味着分类失败；
 * 2. 域间依赖**无环**——`chat ◄ llm ◄ turn ◄ projection` 这条链是拆分形状的**唯一理由**
 *    （把 `node_build` 放回 `chat` 就会造成 `chat ⇄ llm`）。环不会让编译器报错，
 *    只会让「改一个域要改另一个域、再改回来」慢慢变成常态。
 *
 * 第二条是本脚本真正的内容：`core-naming-audit` 判**名字**、`core-export-audit` 判
 * **出口宽度**，没有任何一道门禁判**依赖方向**。D-087 §2 那张图当时是人工核对的，
 * 人工核对的东西会在下一次搬家时失效。
 *
 * ## 依赖边怎么取（口径）
 *
 * 只取**代码层**的边，且只取 `crate::symbio_core::…` 开头的路径：
 *
 * | 源码写法 | 归属 |
 * |---|---|
 * | `crate::symbio_core::ChatNode` | 按根出口符号表 → `chat` |
 * | `crate::symbio_core::chat_node::ChatNode` | `chat_node` 是叶子模块 → `chat` |
 * | `crate::symbio_core::plugin::ROUTE_…` | `plugin` 是域名 → `plugin` |
 * | `use super::…` / `use self::…` | 域内，不计 |
 *
 * 文档注释里的路径先被剥掉（`core-surface` 的 `stripComments`）——**文档链接造成的
 * 假边**是本仓反复踩过的坑（`core-shape-audit` 的注里记着「大量环是文档链接造成的
 * 假阳性」）。
 *
 * ## 判据
 *
 * | 编号 | 判什么 | 为什么 |
 * |---|---|---|
 * | V-001 | `symbio_core/schemas/` 不存在 | 它是被拆掉的杂物抽屉；回来 = 分类失败 |
 * | V-002 | 域间依赖图无环 | 拆分形状的唯一承重理由；环不会让编译器报错 |
 *
 * 其余输出是**报告**（根出口宽度、每域体量、边数明细），不判失败：它们是下一次
 * 决策的输入，不是本次的红线——一份只会误报的守卫最后一定被喂豁免喂到失效。
 *
 * ## 用法
 *
 *   node scripts/core-vocab-audit.mjs               # 判定 + 报告
 *   node scripts/core-vocab-audit.mjs --root=<dir>  # 换根目录（回归测试用）
 *
 * 退出码：0 = 通过；1 = 有违规；2 = 范围读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成）
// @ns V core 分域
// @codes V-001 V-002

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'
import { collectCoreSurface, stripComments } from './core-surface.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : defaultRoot

let errors = 0
const hits = new Map()
function report(code, where, message) {
  console.log(`${red('[ERROR]')} ${code} ${where}  ${message}`)
  errors++
  hits.set(code, (hits.get(code) ?? 0) + 1)
}

// ── V-001：schemas 不许回来 ───────────────────────────────────────────────

const schemasDir = path.join(ROOT, 'crates/symbio-core/src/schemas')
if (fs.existsSync(schemasDir)) {
  report(
    'V-001',
    'symbio_core/schemas/',
    '这个目录已被 D-087 拆成 chat / dialog / present / turn —— 它重新出现意味着' +
      '又出现了「暂时没处放」的东西，请按「它描述什么」找域，而不是新建一个抽屉',
  )
}

// ── 依赖边 ───────────────────────────────────────────────────────────────

let surface
try {
  surface = collectCoreSurface(ROOT)
} catch (e) {
  console.error(red(`✗ 范围读不出：${e.message}`))
  process.exit(2)
}

/** 名字 → 所属域：根出口符号 + 叶子模块 + 域名本身。 */
const owner = new Map()
for (const [name, domain] of surface.symbols) {
  if (domain !== '(root)' && domain !== '(explicit)') owner.set(name, domain)
}
for (const d of surface.domains) if (!owner.has(d)) owner.set(d, d)

const edges = new Map() // "from→to" → Set<file>
const edgeNote = new Map()

function addEdge(from, to, file) {
  if (from === to) return
  const key = `${from}→${to}`
  if (!edges.has(key)) edges.set(key, new Set())
  edges.get(key).add(file)
}

/** 递归列出某域下的生产 `.rs`（排除 `*.test.rs`）。 */
function* prodFiles(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) yield* prodFiles(p)
    else if (e.isFile() && e.name.endsWith('.rs') && !e.name.endsWith('.test.rs')) yield p
  }
}

const PATH_RE = /crate::symbio_core::([A-Za-z_][A-Za-z0-9_]*)/g

for (const domain of surface.domains) {
  const dir = path.join(surface.coreDir, domain)
  if (!fs.existsSync(dir)) continue
  for (const file of prodFiles(dir)) {
    const raw = fs.readFileSync(file, 'utf8')
    const src = stripComments(raw)
    const rel = path.relative(ROOT, file).replace(/\\/g, '/')
    for (const m of src.matchAll(PATH_RE)) {
      const target = owner.get(m[1])
      if (target) addEdge(domain, target, rel)
      else edgeNote.set(m[1], rel) // 认不出的名字：不猜归属
    }
  }
}

// ── V-002：无环（Tarjan 强连通分量） ────────────────────────────────────
//
// 为什么用 SCC 而不是三色 DFS 找「环」：同一个环从不同起点会被找到多次，
// 而环绕着更大的环走时还会派生出成串的**超集**（`a→b→a` 与 `a→b→c→a` 都报会
// 让基线变成一份永远对不上的名单）。SCC 是**规范形**：一个强连通分量 = 一组
// 互相可达的域，再加不进任何别的域，与遍历顺序无关。
//
// 基线只许降（见 BASELINE）：新增一条**跨分量的回边**才会造出新分量。

const adj = new Map()
for (const d of surface.domains) adj.set(d, [])
for (const key of edges.keys()) {
  const [from, to] = key.split('→')
  adj.get(from).push(to)
}

let index = 0
const idx = new Map()
const low = new Map()
const onStack = new Set()
const st = []
const sccs = []

function strongconnect(v) {
  idx.set(v, index)
  low.set(v, index)
  index++
  st.push(v)
  onStack.add(v)
  for (const w of adj.get(v) ?? []) {
    if (!idx.has(w)) {
      strongconnect(w)
      low.set(v, Math.min(low.get(v), low.get(w)))
    } else if (onStack.has(w)) {
      low.set(v, Math.min(low.get(v), idx.get(w)))
    }
  }
  if (low.get(v) === idx.get(v)) {
    const comp = []
    let w
    do {
      w = st.pop()
      onStack.delete(w)
      comp.push(w)
    } while (w !== v)
    sccs.push(comp.sort())
  }
}
for (const d of [...surface.domains].sort()) if (!idx.has(d)) strongconnect(d)

const cyclic = sccs.filter((c) => c.length > 1).map((c) => c.join('|')).sort()

/** 已登记的回边族（只许降）。每一条都要写清「为什么现在拆不动」。 */
const DEFAULT_BASELINE = [
  // adapters（LLM 适配接缝）与 projection 互指：`fallback` 读适配层的档位统计，
  // 适配层又按它的时延档表产出。拆法存在（档位词表下沉一层），但那是 adapters
  // 自己的改造，不在 D-087 的范围内。
  'adapters|projection',
  // 2026-10-20 拆掉的八域族（D-095）：曾是
  // plugin ⇄ keys ⇄ vdfs ⇄ chat/present ⇄ capability ⇄ llm ⇄ exec ⇄ event_bus。
  // 根因是 **`keys` 的键值类型指向各域的类型**（键必须能装 `Arc<dyn Capability>`），
  // 而各域又用键取上下文——「万能胶」型的环。三刀拆解：
  //   ① 键与值同住：8 个值类型属某域的键随值迁出 keys（keys→capability/exec/plugin
  //      三条出边消失，keys 成汇点）；
  //   ② Aborted 载荷去 ChatNode 化（plugin→chat 消失）；
  //   ③ vdfs_change_of 搬 event_bus（vdfs→plugin / vdfs→event_bus 消失）。
  // 边数 44 → 40，环族 2 → 1。留此注释为「基线会长回来」立反例：keys 域今后只收
  // 值类型为通用词汇的键，新增值类型属某域的键就是往这个坑里填土。
]

/** 基线可由环境变量替换（回归测试用）：它与仓库数据解耦，守卫逻辑不含具体域名。 */
const envBaseline = process.env.CORE_VOCAB_BASELINE
const BASELINE = envBaseline === undefined ? DEFAULT_BASELINE : JSON.parse(envBaseline)

for (const c of cyclic) {
  if (BASELINE.includes(c)) continue
  report('V-002', 'symbio_core/', `出现新的域间依赖环：${c.split('|').join(' ⇄ ')}（不在 BASELINE 里）`)
}

// ── 报告 ─────────────────────────────────────────────────────────────────

const byDomain = new Map()
for (const domain of surface.symbols.values()) {
  if (domain === '(root)' || domain === '(explicit)') continue
  byDomain.set(domain, (byDomain.get(domain) ?? 0) + 1)
}
const topDomains = [...byDomain].sort((a, b) => b[1] - a[1]).slice(0, 8)

const locByDomain = []
for (const domain of surface.domains) {
  const dir = path.join(surface.coreDir, domain)
  if (!fs.existsSync(dir)) continue
  let loc = 0
  for (const f of prodFiles(dir)) loc += fs.readFileSync(f, 'utf8').split(/\r?\n/).length
  locByDomain.push([domain, loc])
}
locByDomain.sort((a, b) => b[1] - a[1])

const totalSymbols = [...surface.symbols.keys()].length
const totalMods = surface.modules.size

console.log('')
console.log(dim('── core 分域形状 ──'))
console.log(
  dim(
    `  根出口 ${totalSymbols} 个符号（${surface.domains.size} 域 · ${totalMods} 个叶子模块）· ` +
      `域间依赖 ${edges.size} 条边`,
  ),
)
console.log(dim(`  符号最多的域：${topDomains.map(([d, n]) => `${d}(${n})`).join(' · ')}`))
console.log(dim(`  体量最大的域：${locByDomain.slice(0, 6).map(([d, n]) => `${d}(${n}行)`).join(' · ')}`))
console.log(
  dim(
    `  依赖环族 ${cyclic.length} 个（基线 ${BASELINE.length}）：` +
      (cyclic.length ? cyclic.map((c) => c.split('|').join('⇄')).join(' · ') : '无'),
  ),
)

if (process.argv.includes('--edges')) {
  for (const [key, files] of [...edges].sort()) {
    console.log(dim(`  ${key}  ←  ${[...files].slice(0, 3).join(', ')}${files.size > 3 ? ` …×${files.size}` : ''}`))
  }
}

if (edgeNote.size) {
  console.log(
    dim(
      `  未识别归属的根路径（不猜，仅供核对）：${[...edgeNote.keys()].slice(0, 8).join(' · ')}` +
        (edgeNote.size > 8 ? ` …共 ${edgeNote.size} 个` : ''),
    ),
  )
}

console.log('')
const codeNames = {
  'V-001': 'schemas 目录不存在',
  'V-002': `域间依赖无新增环（基线 ${BASELINE.length} 族，只许降）`,
}
for (const [code, name] of Object.entries(codeNames)) {
  const n = hits.get(code) ?? 0
  console.log(`  ${n === 0 ? green('✓') : red('✗')} ${code}  ${name}${n ? `  (${n})` : ''}`)
}
console.log('')
if (errors > 0) {
  console.log(red(`  ${errors} 个违规`))
  process.exit(1)
}
