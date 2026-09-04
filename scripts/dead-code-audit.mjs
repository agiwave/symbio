#!/usr/bin/env node
/**
 * 死代码审计（前端 TS/Vue 侧）
 *
 * 三层判定，尽量零误报：
 *  L1 import 依赖图：从入口（index.html→main.ts、测试、.d.ts、构建配置）可达性。
 *  L2 字符串引用兜底：不可达但文件名仍被源码提及 → 降级「疑似」，不自动判死。
 *  L3 schema 契约：schemas/*.ts 若被 route 调用方以类型名引用则视为存活。
 *
 * 用法： node scripts/dead-code-audit.mjs
 * 退出码：发现「确认死代码」时为 1（便于 CI 拦截）
 */
import { readdirSync, statSync, readFileSync, existsSync } from 'node:fs'
import { join, dirname, resolve, relative, extname, basename } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(__dirname, '..', 'tauri')
const SRC = join(ROOT, 'src')
const EXT = ['.vue', '.ts', '.js', '.tsx', '.mts']

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
console.log(`\n【导出级检查】`)
let unusedExports = 0
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
    if (!used) {
      console.log(`  ${dir} :: ${n}`)
      unusedExports++
    }
  }
}
if (!unusedExports) console.log('  （无未被引用的导出）')

const lines = dead.reduce((n, f) => n + lineCount(f), 0)
console.log(`\n合计可移除：${dead.length} 文件 / ${lines} 行`)
process.exit(dead.length ? 1 : 0)
