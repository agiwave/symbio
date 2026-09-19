#!/usr/bin/env node
/**
 * 死代码审计（前端 TS/Vue 判定 + Rust 导出级降级提示）
 *
 * 三层判定，尽量零误报：
 *  L1 import 依赖图：从入口（index.html→main.ts、测试、.d.ts、构建配置）可达性。
 *  L2 字符串引用兜底：不可达但文件名仍被源码提及 → 降级「疑似」，不自动判死。
 *  L3 schema 契约：schemas/*.ts 若被 route 调用方以类型名引用则视为存活。
 *  L4 Rust 侧：`pub` 声明但全仓（含 cli / tauri / 前端）无任何外部引用
 *     → **降级提示，永不判失败**（Rust 没有 import 依赖图，只能靠名称是否被
 *     提过这种弱判据；`dead_code` lint 对 `pub` 项结构性失明，见该段注释）。
 *
 * 用法： node scripts/dead-code-audit.mjs             # 死代码清单（明细只印死代码）
 *        node scripts/dead-code-audit.mjs --verbose   # 附带「未被引用的导出」明细
 * 退出码：发现「确认死代码」时为 1（已接入 gate.mjs 的 docs 阶段，是判定型检查）。
 *         仅**前端文件级**死代码会触发；Rust 与「导出级」发现一律不影响退出码。
 */
import { readdirSync, statSync, readFileSync, existsSync } from 'node:fs'
import { join, dirname, resolve, relative, extname, basename } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..', 'tauri')
const SRC = join(ROOT, 'src')
const EXT = ['.vue', '.ts', '.js', '.tsx', '.mts']
/** 打印「未被引用的导出」明细（默认只给个数，见文件末段说明） */
const VERBOSE = process.argv.includes('--verbose')

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) walk(p, out)
    else if (EXT.includes(extname(p))) out.push(p)
  }
  return out
}

const allFiles = walk(SRC)
const fileSet = new Set(allFiles)
const rel = (p) => relative(ROOT, p).replace(/\\/g, '/')
const isTest = (p) => p.includes('__tests__') || /\.(spec|test)\.[cm]?[jt]sx?$/.test(p)
const lineCount = (p) => readFileSync(p, 'utf8').split(/\r?\n/).length

function resolveSpec(spec, fromFile) {
  let base
  if (spec.startsWith('@/')) base = join(SRC, spec.slice(2))
  else if (spec.startsWith('.')) base = resolve(dirname(fromFile), spec)
  else return null
  return [base, ...EXT.map((e) => base + e), ...EXT.map((e) => join(base, 'index' + e))].find((c) =>
    fileSet.has(c),
  )
}

/** index.html 里的 /src/main.ts 在 Windows 下不能直接 resolve（会跑到盘符根） */
const projPath = (spec) => resolve(ROOT, spec.replace(/^[\\/]+/, ''))

const IMPORT_RE =
  /(?:import|export)\s+[^'"]*?from\s*['"]([^'"]+)['"]|import\s*\(\s*['"]([^'"]+)['"]\s*\)|import\s+['"]([^'"]+)['"]/g

function reachableFrom(entries) {
  const seen = new Set()
  const queue = [...entries]
  while (queue.length) {
    const f = queue.pop()
    if (!f || seen.has(f) || !fileSet.has(f)) continue
    seen.add(f)
    for (const m of readFileSync(f, 'utf8').matchAll(IMPORT_RE)) {
      const r = resolveSpec(m[1] ?? m[2] ?? m[3], f)
      if (r) queue.push(r)
    }
  }
  return seen
}

const entries = []
const seenEntry = new Set()
const addEntry = (p, why) => {
  if (fileSet.has(p) && !seenEntry.has(p)) {
    seenEntry.add(p)
    entries.push(p)
  }
}

if (existsSync(join(ROOT, 'index.html')))
  for (const m of readFileSync(join(ROOT, 'index.html'), 'utf8').matchAll(/\ssrc=["']([^"']+)["']/g))
    addEntry(projPath(m[1]))

for (const f of allFiles)
  if (isTest(f) || f.endsWith('.d.ts') || /(^|[\\/])(vite|vitest)\.config\.[cm]?[jt]s$/.test(f))
    addEntry(f)

const reachable = reachableFrom(entries)
const unreachable = allFiles.filter((f) => !reachable.has(f))

const corpus = allFiles.map((f) => [f, readFileSync(f, 'utf8')])
function stringReferenced(file) {
  const stem = basename(file).replace(/\.[^.]+$/, '')
  if (!stem || stem === 'index') return null
  const re = new RegExp(`${stem.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\.(vue|ts|js)`)
  for (const [f, code] of corpus) {
    if (f === file) continue
    const m = code.match(re)
    if (m) return { by: rel(f), token: m[0] }
  }
  return null
}

const dead = []
const suspicious = []
for (const f of unreachable) {
  const hit = stringReferenced(f)
  ;(hit ? suspicious : dead).push(hit ? [f, hit] : f)
}

console.log(`扫描 ${allFiles.length} 文件 · 入口 ${entries.length} · 可达 ${reachable.size}`)

console.log(`\n【死代码】${dead.length} —— 无 import、无字符串引用：`)
for (const f of dead) console.log(`  ${rel(f)}  (${lineCount(f)} 行)`)

if (suspicious.length) {
  console.log(`\n【疑似】${suspicious.length} —— 不可达但有字符串引用：`)
  for (const [f, h] of suspicious) console.log(`  ${rel(f)}  ← ${h.by} 提到 "${h.token}"`)
}

// ── 未使用的导出（死代码的细粒度形式）──
// 属**降级提示**：本检查对「仅在本文件内按类型用」「经 barrel 再导出」都会命中，
// 故默认只给个数，明细加 `--verbose` 才打印——否则每次门禁刷 50+ 行噪音。
console.log(`\n【导出级检查】`)
const unusedList = []
for (const f of reachable) {
  if (isTest(f) || f.endsWith('.d.ts')) continue
  const code = readFileSync(f, 'utf8')
  const names = [
    ...[...code.matchAll(/export\s+(?:const|function|async\s+function|class|interface|type|enum)\s+([A-Za-z0-9_$]+)/g)].map((m) => m[1]),
  ]
  if (!names.length) continue
  const dir = rel(f)
  for (const n of names) {
    // 同名标识符在其它任何文件中出现即视为可能被使用（保守）
    const re = new RegExp(`\\b${n.replace(/\$/g, '\\$')}\\b`)
    const used = corpus.some(([g, c]) => g !== f && re.test(c))
    if (!used) unusedList.push(`${dir} :: ${n}`)
  }
}
const unusedExports = unusedList.length
if (VERBOSE) {
  for (const line of unusedList) console.log(`  ${line}`)
} else if (unusedExports) {
  console.log(`  ${unusedExports} 个导出未被其他文件引用（降级提示，不判失败；--verbose 看明细）`)
}
if (!unusedExports) console.log('  （无未被引用的导出）')

// ── Rust 侧：`pub` 声明但全仓无任何外部引用 ──
//
// 为什么需要：前端侧有 import 依赖图这张硬网；**Rust 侧一张都没有**。`dead_code`
// lint 对 `pub` 项结构性失明——`symbio_core/mod.rs` 里一句 `pub use error::*`
// 就足以让整批函数被当成"对外 API"，于是攒出过一批零调用的 helper（8 个锁/错误
// 辅助函数，全仓含 cli / tauri 零引用）。
//
// 口径与上面的「导出级检查」一致：**名称在别处出现过即视为可能被使用**（保守），
// 故只作**降级提示、不判失败**——它抓的是"从没被任何人提过"这一最低风险的形态。
// 作者已显式写下 `#[allow(dead_code)]` 的，视为刻意的保留，不与应用者对抗。
const REPO = resolve(__dirname, '..')
const repoRel = (p) => relative(REPO, p).replace(/\\/g, '/')

const rustSources = []
const tsSources = []
const widen = (dir, exts, sink) => {
  let ents
  try {
    ents = readdirSync(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of ents) {
    const p = join(dir, e.name)
    if (e.isDirectory()) {
      if (['target', 'vendor', 'node_modules', 'dist', '.git'].includes(e.name)) continue
      widen(p, exts, sink)
      continue
    }
    if (exts.some((x) => e.name.endsWith(x))) sink.push(p)
  }
}
widen(join(REPO, 'symbio'), ['.rs'], rustSources)
widen(join(REPO, 'cli'), ['.rs'], rustSources)
widen(join(REPO, 'tauri', 'src-tauri'), ['.rs'], rustSources)
// 名字可能被非 Rust 侧提及（路由串常量等），一并纳入「是否被提过」的语料
widen(join(REPO, 'tauri', 'src'), ['.ts', '.vue'], tsSources)

const read = (files) => files.map((f) => [f, readFileSync(f, 'utf8')])
const rustCorpus = read(rustSources)
const nameCorpus = rustCorpus.concat(read(tsSources))

const DECL_RE =
  /^pub(?:\([^)]*\))?\s+(?:async\s+)?(?:unsafe\s+)?(fn|struct|enum|trait|const|static|type)\s+([A-Za-z_][A-Za-z0-9_]*)/gm

/** 紧邻其上的属性里是否写了 `#[allow(dead_code)]`（跳过空行与文档注释） */
function allowedDeadCode(code, idx) {
  const lines = code.slice(0, idx).split('\n')
  for (let i = lines.length - 2; i >= 0; i--) {
    const t = lines[i].trim()
    if (t === '' || t.startsWith('//')) continue
    return /#\[allow\([^\]]*dead_code/.test(t)
  }
  return false
}

// 判据：**该名字在全仓只出现过一次**（就是声明那一行）⇒ 没人用过。
//
// 为什么不按"别的文件里有没有"来判：同一文件内的引用同样是真引用——
// `#[serde(default = "default_max_messages")]`、`submit_object_creator!(…, SettingPlugin::build, …)`
// 都只在声明文件里出现，按"别文件"判会把它们全数误报（实测踩坑）。
// 反过来，本规则只抓"从没被任何人提过"这一最低风险形态，与「导出级检查」同一取向。
const tokenCount = new Map()
for (const [, code] of nameCorpus) {
  for (const m of code.matchAll(/[A-Za-z_][A-Za-z0-9_]*/g)) {
    tokenCount.set(m[0], (tokenCount.get(m[0]) ?? 0) + 1)
  }
}

const rustUnused = []
let rustDecls = 0
for (const [file, code] of rustCorpus) {
  if (file.endsWith('.test.rs') || basename(file) === 'tests.rs') continue
  for (const m of code.matchAll(DECL_RE)) {
    const [, kind, name] = m
    rustDecls++
    if (allowedDeadCode(code, m.index)) continue
    if ((tokenCount.get(name) ?? 0) <= 1) {
      rustUnused.push(`${repoRel(file)} :: ${kind} ${name}`)
    }
  }
}

console.log(`\n【Rust 导出级检查】（降级提示，不判失败）`)
console.log(`  扫描 ${rustCorpus.length} 个 .rs · ${rustDecls} 处 pub 声明`)
if (rustUnused.length) {
  for (const line of rustUnused) console.log(`  ${line}`)
} else {
  console.log('  （无全仓零引用的 pub 声明）')
}

const lines = dead.reduce((n, f) => n + lineCount(f), 0)
console.log(`\n合计可移除：${dead.length} 文件 / ${lines} 行`)
process.exit(dead.length ? 1 : 0)
