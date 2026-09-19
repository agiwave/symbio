#!/usr/bin/env node
/**
 * 死代码审计（前端 TS/Vue 判定 + Rust 声明级判定 R-001）
 *
 * 四层判定，尽量零误报：
 *  L1 import 依赖图：从入口（index.html→main.ts、测试、.d.ts、构建配置）可达性。
 *  L2 字符串引用兜底：不可达但文件名仍被源码提及 → 降级「疑似」，不自动判死。
 *  L3 schema 契约：schemas/*.ts 若被 route 调用方以类型名引用则视为存活。
 *  L4 Rust 侧 **R-001（判定型）**：`pub` 声明但全仓（含 cli / tauri / 前端）
 *     一次都没被提及。判据是「这个名字在整仓只出现一次（就是声明那行）」——
 *     `dead_code` lint 对 `pub` 项结构性失明（`symbio_core/mod.rs` 一句
 *     `pub use error::*` 就能让整批函数被当成对外 API），故需要这条补网。
 *
 * 承认通道：确需保留但无 Rust 消费方的（消费方在前端 / 闭集成员），在声明行或
 * 紧邻其上一行写 `// dead-code-allow R-001: <理由>`；**理由不可为空**（空理由
 * 视为未承认，与 `grep-audit` / `plugin-entry-audit` 的豁免同一约定）。
 *
 * 用法： node scripts/dead-code-audit.mjs             # 死代码清单（明细只印死代码）
 *        node scripts/dead-code-audit.mjs --verbose   # 附带「未被引用的导出」明细
 * 退出码：1 = 有「确认死代码」（前端文件级）或「未承认的 R-001」；
 *         0 = 通过。两者都在 gate.mjs 的 docs 阶段（判定型）。
 *
 * 已知边界：R-001 只抓「整仓一次都没被提过」这一最低风险形态。`pub` 项被**弱引用**
 * （只在文档 / 注释里被提到，或在运行期被拼名字调用）一律视为存活——宁可漏报。
 */
import { readdirSync, statSync, readFileSync, existsSync } from 'node:fs'
import { join, dirname, resolve, relative, extname, basename } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
// `--root=` 只为回归测试开的口子：测试在临时目录造一棵最小仓库，好让本脚本的判据
// 能被「注入真实违规并断言变红」地验证——判定型守卫的教条是「一个只会亮绿灯的
// 守卫等于没有守卫」，而它腐烂的方式恰恰是「规则写错了所以永远不命中」。
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? resolve(rootArg.slice(7)) : resolve(__dirname, '..')
const ROOT = join(REPO_ROOT, 'tauri')
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

// ── Rust 侧 R-001：`pub` 声明但全仓一次都没被提及（**判定型**）──
//
// 为什么需要：前端侧有 import 依赖图这张硬网；**Rust 侧一张都没有**。`dead_code`
// lint 对 `pub` 项结构性失明——`symbio_core/mod.rs` 里一句 `pub use error::*`
// 就足以让整批函数被当成"对外 API"，于是攒出过一批零调用的 helper（8 个锁/错误
// 辅助函数，全仓含 cli / tauri 零引用）。
//
// ## 为什么这条能判失败，而 `plugin-entry-audit` 的 `refs=0` 不能
//
// 「定义了但没人用」在**路由**上不可判：路径可以在运行期拼（工具名、子插件名），
// 网关还会把外部 `path` 原样转发给容器 route（见 plugin-entry-audit 头部的两条
// 理由）。但本规则判的不是运行期字符串，而是**一个静态符号名**——`pub fn foo`
// 的 `foo` 有没有在仓库任何地方被写过一次，是纯文本事实，没有动态解析的余地。
//
// ## 承认通道（唯一出口）
//
// 确有必须保留而无 Rust 消费方的（消费方在前端、闭集成员等），在声明行或紧邻其上
// 一行写 `// dead-code-allow R-001: <理由>`。**理由不可为空**——否则「随手加个
// 注释就过」会让这条守卫退化成橡皮图章（与 grep-audit / plugin-entry-audit 同一约定）。
const REPO = REPO_ROOT
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

/** 承认通道：`// dead-code-allow R-001: <理由>`（理由为空 = 未承认） */
const WAIVER_RE = /\/\/\s*dead-code-allow\s+(R-\d+)\s*:\s*(\S.*)$/

/**
 * 取声明的承认理由：看**声明行本身**与紧邻其上 3 行（覆盖 `#[allow(dead_code)]`
 * 同行注理由、或独立一行注释两种写法）。返回理由字符串；未承认返回 `null`。
 */
function waiverReason(lines, declLine) {
  for (let i = declLine; i >= Math.max(0, declLine - 3); i--) {
    const hit = lines[i].match(WAIVER_RE)
    if (hit) return hit[2].trim()
  }
  return null
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
const rustWaived = []
let rustDecls = 0
for (const [file, code] of rustCorpus) {
  if (file.endsWith('.test.rs') || basename(file) === 'tests.rs') continue
  const lines = code.split(/\r?\n/)
  for (const m of code.matchAll(DECL_RE)) {
    const [, kind, name] = m
    rustDecls++
    if ((tokenCount.get(name) ?? 0) > 1) continue
    // 声明所在行（0 基）：`m.index` 落在 `^` 之后，即行首
    const declLine = code.slice(0, m.index).split('\n').length - 1
    const reason = waiverReason(lines, declLine)
    const where = `${repoRel(file)} :: ${kind} ${name}`
    if (reason) rustWaived.push(`${where}  ← ${reason}`)
    else rustUnused.push(where)
  }
}

console.log(`\n【R-001】Rust 声明级检查（判定型）`)
console.log(`  扫描 ${rustCorpus.length} 个 .rs · ${rustDecls} 处 pub 声明`)
if (rustUnused.length) {
  console.log(`  ✗ ${rustUnused.length} 处全仓零引用且未承认：`)
  for (const line of rustUnused) console.log(`    ${line}`)
  console.log(`    ↳ 删掉它；确需保留则写 \`// dead-code-allow R-001: 理由\`（理由必填）`)
} else {
  console.log('  ✓ 无全仓零引用的 pub 声明')
}
if (rustWaived.length) {
  console.log(`  · ${rustWaived.length} 处已承认保留（消费方不在 Rust 侧，见各行理由）：`)
  for (const line of rustWaived) console.log(`    ${line}`)
}

const lines = dead.reduce((n, f) => n + lineCount(f), 0)
console.log(`\n合计可移除：${dead.length} 文件 / ${lines} 行`)
process.exit(dead.length || rustUnused.length ? 1 : 0)
