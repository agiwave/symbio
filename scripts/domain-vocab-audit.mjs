#!/usr/bin/env node
/**
 * domain-vocab-audit — 分域形状与依赖方向（V-001 判定 / V-002 判定 / 其余报告）
 *
 * ## 它治什么
 *
 * [D-087](../docs/003-design/001-核心架构.md "D-087") 把 `schemas/` 拆成
 * `chat` / `dialog` / `present` / `turn` 四个域，并给出两条**可执行**的判据：
 *
 * 1. `schemas` 目录不再存在——它是「暂时没处放」的同义词，重新长出来意味着分类失败；
 * 2. 域间依赖**无环**——`chat ◄ llm ◄ turn ◄ projection` 这条链是拆分形状的**唯一理由**
 *    （把 `node_build` 放回 `chat` 就会造成 `chat ⇄ llm`）。环不会让编译器报错，
 *    只会让「改一个域要改另一个域、再改回来」慢慢变成常态。
 *
 * 第二条是本脚本真正的内容：没有任何别的门禁判**依赖方向**。D-087 §2 那张图当时是
 * 人工核对的，人工核对的东西会在下一次搬家时失效。
 *
 * ## 判据的对象是 `symbio-sdk`（D-279）
 *
 * 它原判 `symbio_core`。20 个域搬进 `symbio-sdk` 之后，core 只剩两个模块，图上的节点
 * 与边都跟着走了——**判据必须跟着对象走**，否则它会像 2026-11 那次一样：路径不在了，
 * 扫到 0 条边，报「无新增环」，而它守的东西一次都没被检验。
 *
 * ## 依赖边怎么取（口径）
 *
 * 只取**代码层**的边，且只取 `crate::<域>` 开头的路径：
 *
 * | 源码写法 | 归属 |
 * |---|---|
 * | `crate::chat::ChatNode` | `chat` |
 * | `crate::projection::transcript::…` | `projection` |
 * | `use super::…` / `use self::…` | 域内，不计 |
 * | `crate::keys::PATH` | `keys`（域间边，即使只取一个键） |
 *
 * 文档注释里的路径先被剥掉（`rust-scan.mjs` 的 `stripComments`）——**文档链接造成的
 * 假边**是本仓反复踩过的坑（`arch-shape-audit` 的注里记着「大量环是文档链接造成的
 * 假阳性」）。
 *
 * ## 判据
 *
 * | 编号 | 判什么 | 为什么 |
 * |---|---|---|
 * | V-001 | `symbio-sdk/src/schemas/` 不存在 | 它是被拆掉的杂物抽屉；回来 = 分类失败 |
 * | V-002 | 域间依赖图无环 | 拆分形状的唯一承重理由；环不会让编译器报错 |
 *
 * 其余输出是**报告**（每域体量、边数明细），不判失败：它们是下一次决策的输入，不是
 * 本次的红线——一份只会误报的守卫最后一定被喂豁免喂到失效。
 *
 * ## 用法
 *
 *   node scripts/domain-vocab-audit.mjs               # 判定 + 报告
 *   node scripts/domain-vocab-audit.mjs --root=<dir>  # 换根目录（回归测试用）
 *
 * 退出码：0 = 通过；1 = 有违规；2 = 范围读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成）
// @ns V 分域形状
// @codes V-001 V-002

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'
import { stripComments } from './rust-scan.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : defaultRoot

/** 判据面：`symbio-sdk` 的 `src`。读不出 ⇒ 非 0 退出，不许「0 处违规」（D-280）。 */
const SRC_REL = 'crates/symbio-sdk/src'
const SRC_DIR = path.join(ROOT, SRC_REL)

let errors = 0
const hits = new Map()
function report(code, where, message) {
  console.log(`${red('[ERROR]')} ${code} ${where}  ${message}`)
  errors++
  hits.set(code, (hits.get(code) ?? 0) + 1)
}

let domains
try {
  domains = fs
    .readdirSync(SRC_DIR, { withFileTypes: true })
    .filter((e) => e.isDirectory())
    .map((e) => e.name)
    .sort()
} catch {
  console.error(red(`✗ 范围读不出：${SRC_REL} 不存在—— 判据没有输入，宁红不静默（D-280）`))
  process.exit(2)
}
if (domains.length === 0) {
  console.error(red(`✗ 范围读不出：${SRC_REL} 下一个域目录都没有—— 判据没有输入（D-280）`))
  process.exit(2)
}
const domainSet = new Set(domains)

// ── V-001：schemas 不许回来 ───────────────────────────────────────────────
//
// 否定型判据：它守的是「不许回来」。正例在回归测试里（造一个 schemas/ ⇒ 必须红）。

const schemasDir = path.join(SRC_DIR, 'schemas')
if (fs.existsSync(schemasDir)) {
  report(
    'V-001',
    'symbio-sdk/src/schemas/',
    '这个目录已被 D-087 拆成 chat / dialog / present / turn —— 它重新出现意味着' +
      '又出现了「暂时没处放」的东西，请按「它描述什么」找域，而不是新建一个抽屉',
  )
}

// ── 依赖边 ───────────────────────────────────────────────────────────────

const edges = new Map() // "from→to" → Set<file>

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

const PATH_RE = /crate::([A-Za-z_][A-Za-z0-9_]*)/g

for (const domain of domains) {
  const dir = path.join(SRC_DIR, domain)
  for (const file of prodFiles(dir)) {
    const src = stripComments(fs.readFileSync(file, 'utf8'))
    const rel = path.relative(ROOT, file).replace(/\\/g, '/')
    for (const m of src.matchAll(PATH_RE)) {
      // 只认域名的第一段；`crate::actors::control` 与 `crate::actors` 同一条边。
      if (domainSet.has(m[1])) addEdge(domain, m[1], rel)
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
for (const d of domains) adj.set(d, [])
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
for (const d of [...domains].sort()) if (!idx.has(d)) strongconnect(d)

const cyclic = sccs.filter((c) => c.length > 1).map((c) => c.join('|')).sort()

/**
 * 已登记的回边族（只许降）。每一条都要写清「为什么现在拆不动」。
 *
 * 基线在 D-279 搬迁后**重新实测**：此前那张表是按 core 的 20 个域 + `crate::symbio_sdk::`
 * 路径口径量出来的，而那个路径前缀随 crate 一起消失了——也就是说 V-002 在 D-258…D-279
 * 之间一直扫到 0 条边、报「无新增环」（D-280 记的空转绿第二例）。改指 sdk 后第一次跑出
 * 的环族才是真值。
 */
const DEFAULT_BASELINE = [
  // 空表。最后一条登记项 `adapters|projection` 在 D-279 的重新实测里已不复存在
  // （`adapters` 不再引用 `projection`，那条 `fallback` 读档位统计的边随搬迁断掉了）——
  // 环族 1 → **0**，按「只许降」把它从基线里删掉。此后出现任何环族都是新环。
  //
  // 历史（保留为反例）：2026-10-20 拆掉的八域族（D-095）曾是
  // plugin ⇄ keys ⇄ vdfs ⇄ chat/present ⇄ capability ⇄ llm ⇄ exec ⇄ event_bus。
  // 根因是 **`keys` 的键值类型指向各域的类型**（键必须能装 `Arc<dyn Capability>`），
  // 而各域又用键取上下文——「万能胶」型的环。三刀拆解：① 键与值同住；② Aborted 载荷
  // 去 ChatNode 化；③ vdfs_change_of 搬 event_bus。边数 44 → 40，环族 2 → 1。
]

/** 基线可由环境变量替换（回归测试用）：它与仓库数据解耦，守卫逻辑不含具体域名。 */
const envBaseline = process.env.CORE_VOCAB_BASELINE
const BASELINE = envBaseline === undefined ? DEFAULT_BASELINE : JSON.parse(envBaseline)

for (const c of cyclic) {
  if (BASELINE.includes(c)) continue
  report('V-002', 'symbio-sdk/', `出现新的域间依赖环：${c.split('|').join(' ⇄ ')}（不在 BASELINE 里）`)
}

// ── 报告 ─────────────────────────────────────────────────────────────────

const locByDomain = []
for (const domain of domains) {
  const dir = path.join(SRC_DIR, domain)
  let loc = 0
  for (const f of prodFiles(dir)) loc += fs.readFileSync(f, 'utf8').split(/\r?\n/).length
  locByDomain.push([domain, loc])
}
locByDomain.sort((a, b) => b[1] - a[1])

console.log('')
console.log(dim('── 分域形状（symbio-sdk） ──'))
console.log(dim(`  ${domains.length} 域 · 域间依赖 ${edges.size} 条边`))
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

console.log('')
const codeNames = {
  'V-001': 'schemas 目录不存在（否定型：正例见回归测试）',
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
