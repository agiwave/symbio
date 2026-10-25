#!/usr/bin/env node
/**
 * kind-branch-audit — 前端的 **kind 分支**计数（判定型棘轮）
 *
 * ## 口径（先写清，再计数）
 *
 * **一个分支 = 一处对「形态」或「资源类型」做运行时选择的位置**。两条轴同构，
 * 各认三种形态：
 *
 * | 轴 | 比较 | 派生判据 | 映射表 |
 * |---|---|---|---|
 * | 形态 | `NODE_KIND_*` | `nodeKindOf(...)` / `messageTypeOf(...)` | `Record<NodeKind, …>` |
 * | 资源类型 | `VDFS_EXT_*` / `VDFS_KIND_*` | `vdfsExtOf(...)` / `resolveVdfsRenderer(...)` / `renderer*` 变量 | `Record<…, VdfsRenderer>` |
 *
 * 另：把 `form` / `session` / `message` / `markdown` / `json` / `text` / `dir` 这类
 * **渲染器字面量**直接拿来比较，也计一格——它就是资源分派写死在框架里的样子。
 * `f.widget === 'form'`、`specOf(f).tag === 'form'` 属于**表单控件轴**，不是资源
 * 分派，排除（判据：原子内带 `.widget` / `.tag`）。
 *
 * 比较按 `===` / `!==` 运算符计数（一个原子一个选择点）；一张映射表 = 一格，
 * 表内每个键不另计。`switch` 的 `case` 本仓不用。
 *
 * 排除：注释、测试文件（`__tests__/`、`*.spec.ts`、`*.test.ts`）、`tauri/src/schemas/`
 * （`NODE_KINDS` 词表、`VDFS_EXT_*` / `VDFS_KIND_*` 常量与归一表是**数据契约**，
 * 不是分派）。
 *
 * ## 两格分开计
 *
 * | 格 | 范围 | 目标 |
 * |---|---|---|
 * | `registry` | `tauri/src/registry/**` | 机制层：两条轴的唯一判据与映射表住在这里，**允许存在** |
 * | `framework` | 其余前端代码（组件 / store / service / composable / view） | **零**：框架只问具名谓词（`isFormRenderer` / `isSessionNode` / `isMessagesSeg` …），不自己认形态或资源类型 |
 *
 * 两格都是上限（只许降）。`framework` 的目标是 0——它正是 GO-002 §3 那条
 * 「三栏框架有几个 kind 分支」的机器口径：数得出来，才谈得上「框架零类型分支」。
 *
 * ## 为什么按「比较次数」而不是「行」
 *
 * 一行里写 `kind === A || kind === B` 是两个选择点；按行计会让「折行」改变数字，
 * 而折行是格式问题，不是分派问题。按运算符计数对这种重排稳定。
 *
 * ## 用法
 *   node scripts/kind-branch-audit.mjs              # 判定
 *   node scripts/kind-branch-audit.mjs --verbose    # 逐点清单
 *   node scripts/kind-branch-audit.mjs --root=<dir> # 指向别处的仓库（测试用）
 * 退出码：0 = 通过，1 = 超基线 / 范围读不出。
 *
 * 环境变量 `KIND_BRANCH_BASELINE=<framework>:<registry>` **只给回归测试用**；
 * 覆盖值不得比默认值更松（`gate.d/35-baseline.mjs` 另判一次）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns KB 前端分支（形态 / 资源类型）
// @codes KB-001 KB-002

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

const SRC_REL = 'tauri/src'
const REGISTRY_PREFIX = 'tauri/src/registry/'
const SCHEMA_PREFIX = 'tauri/src/schemas/'

/**
 * 两格上限（当前实测值）。只许降；改数字本身由 `gate.d/35-baseline.mjs` 判「不得比基准版本更松」。
 *
 * ⚠️ 键序就是 `KIND_BRANCH_BASELINE` 的位置口径，也是 `35-baseline` 提取函数认的格。
 */
const BRANCH_BASELINE = process.env.KIND_BRANCH_BASELINE
  ? (() => {
      const [framework, registry] = process.env.KIND_BRANCH_BASELINE.split(':').map((n) => Number(n))
      return { framework, registry }
    })()
  : { framework: 0, registry: 19 }

const LABEL = {
  framework: '框架层分支（目标 0）',
  registry: '机制层分支（形态 + 资源类型判据与映射表）',
}

const errors = []
const report = (rule, msg) => errors.push({ rule, msg })

/** 去注释但保留换行（行号与内容位置不变）。 */
function blankComments(text) {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, ' '))
    .replace(/\/\/[^\n]*/g, (m) => ' '.repeat(m.length))
}

const isTestPath = (rel) =>
  rel.includes('/__tests__/') ||
  rel.endsWith('.spec.ts') ||
  rel.endsWith('.test.ts') ||
  rel.endsWith('.spec.vue') ||
  rel.endsWith('.test.vue')

function* walk(dir) {
  let list
  try {
    list = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const ent of list) {
    const abs = path.join(dir, ent.name)
    if (ent.isDirectory()) {
      if (ent.name === 'node_modules' || ent.name === 'dist') continue
      yield* walk(abs)
    } else if (/\.(ts|vue)$/.test(ent.name)) {
      yield abs
    }
  }
}

/**
 * 数一个文件里的分派点。
 *
 * 比较点：每个 `===` / `!==` 运算符，若它的**同一个原子**里出现形态记号
 * （`NODE_KIND_*` / `nodeKindOf(` / `messageTypeOf(`）或资源类型记号
 * （`VDFS_EXT_*` / `VDFS_KIND_*` / `vdfsExtOf(` / `resolveVdfsRenderer(` /
 * `renderer*`），或直接把渲染器字面量拿来比较，计 1。
 * 映射表：`Record<NodeKind` / `Record<…VdfsRenderer` 每出现一次计 1。
 */
const KIND_TOKEN = /\bNODE_KIND_[A-Z]+\b|\b(?:nodeKindOf|messageTypeOf)\s*\(/
const RESOURCE_TOKEN = /\bVDFS_EXT_[A-Z_]+\b|\bVDFS_KIND_[A-Z_]+\b|\bvdfsExtOf\s*\(|\bresolveVdfsRenderer\s*\(|\brenderer\w*\b/
const RENDERER_LITERAL = /'(?:form|session|message|markdown|json|text|dir)'/
const NON_RESOURCE_AXIS = /\.(?:widget|tag)\b/
const OPERAND_BOUNDARY = /[&|?:;,(){}[\]]/

export function countSites(text) {
  const blanked = blankComments(text).replace(/\r\n/g, '\n')
  const lines = blanked.split('\n')
  const sites = []
  lines.forEach((line, i) => {
    if (!/(===|!==)/.test(line)) return
    // 把一行切成布尔「原子」：只有在同一个原子内同时出现运算符与分派记号，才算一个选择点。
    // 这样 `kind === 'x' && type === NODE_KIND_BODY` 只计后一个，而
    // `kind === NODE_KIND_BODY || kind === NODE_KIND_NOTICE` 计两个。
    for (const atom of line.split(OPERAND_BOUNDARY)) {
      const ops = [...atom.matchAll(/===|!==/g)].length
      if (!ops) continue
      const dispatch =
        KIND_TOKEN.test(atom) ||
        RESOURCE_TOKEN.test(atom) ||
        (RENDERER_LITERAL.test(atom) && !NON_RESOURCE_AXIS.test(atom))
      if (!dispatch) continue
      for (let n = 0; n < ops; n++) sites.push({ line: i + 1, text: line.trim() })
    }
  })
  for (const m of blanked.matchAll(/\bRecord<\s*NodeKind\b|\bRecord<\s*[^<>]*\bVdfsRenderer\b/g)) {
    const line = blanked.slice(0, m.index).split('\n').length
    sites.push({ line, text: `${m[0].replace(/\s+/g, ' ')} 映射表` })
  }
  return sites
}

function measure() {
  const src = path.join(ROOT, SRC_REL)
  if (!fs.existsSync(src)) {
    console.error(red(`✗ 读不出前端范围：${SRC_REL} 不存在（root=${ROOT}）—— 判据没有输入，宁红不静默`))
    process.exit(1)
  }
  const rows = []
  for (const abs of walk(src)) {
    const rel = path.relative(ROOT, abs).split(path.sep).join('/')
    if (isTestPath(rel) || rel.startsWith(SCHEMA_PREFIX)) continue
    const sites = countSites(fs.readFileSync(abs, 'utf8'))
    if (!sites.length) continue
    const cell = rel.startsWith(REGISTRY_PREFIX) ? 'registry' : 'framework'
    rows.push({ rel, cell, sites })
  }
  const values = { framework: 0, registry: 0 }
  for (const r of rows) values[r.cell] += r.sites.length
  return { rows, values }
}

function main() {
const measured = measure()

for (const [key, cap] of Object.entries(BRANCH_BASELINE)) {
  if (!Object.hasOwn(measured.values, key)) {
    report('KB-001', `基线里有一格 \`${key}\` 不是分支量（只有 ${Object.keys(measured.values).join(' / ')}）`)
    continue
  }
  if (!Number.isFinite(cap) || cap < 0) {
    report('KB-001', `基线格 \`${key}\` 不是非负数字（${cap}）—— 判据读不出即失效，宁红不静默`)
    continue
  }
  const now = measured.values[key]
  if (now > cap) {
    report(
      'KB-001',
      `${LABEL[key]}：实测 ${now} > 基线 ${cap}。` +
        (key === 'framework'
          ? '框架层不得自己认形态或资源类型——在 registry 里加一个具名谓词（isSkeletonKind / isFormRenderer / isSessionNode 同款）再从调用方引用。'
          : '机制层新增分支请同步本脚本的 BRANCH_BASELINE，并在提交信息里说明为什么它是机制而不是业务。'),
    )
  }
}

// KB-002：两格口径自洽——每一条命中都必须落进两格之一，且没有测试 / schema 漏进来。
for (const r of measured.rows) {
  if (r.cell !== 'framework' && r.cell !== 'registry') {
    report('KB-002', `${r.rel} 落不进 framework / registry 任何一格——口径有洞，宁红不静默`)
  }
}
const frameworkFiles = measured.rows.filter((r) => r.cell === 'framework')
if (frameworkFiles.length === 0 && measured.values.registry === 0) {
  // 两格都为空既可能是「真的没有」也可能是「扫描写坏了」。本仓有 registry 表，空 = 读不出。
  report('KB-002', `两格都是 0：本仓 registry 里必然有形态判据 ⇒ 扫描范围/模式失效（不是「零分支」）`)
}

if (errors.length === 0) {
  console.log('\n== kind-branch-audit ==')
  const line = Object.keys(BRANCH_BASELINE)
    .map((k) => `${k} ${measured.values[k]}/${BRANCH_BASELINE[k]}`)
    .join(' · ')
  console.log(`${green('✓')} KB-001  ${line}  ${dim(LABEL.framework + ' · ' + LABEL.registry)}`)
  console.log(`${green('✓')} KB-002  ${measured.rows.length} 个文件命中；每一条都归属两格之一`)
  const under = Object.keys(BRANCH_BASELINE).filter((k) => measured.values[k] < BRANCH_BASELINE[k])
  if (under.length) {
    console.log(
      yellow('提示') +
        `：已收到基线下方（${under.map((k) => `${k} ${measured.values[k]}<${BRANCH_BASELINE[k]}`).join(' · ')}），` +
        `请同步本脚本的 BRANCH_BASELINE——留着旧数字等于把余量白留给下一次`,
    )
  }
  if (VERBOSE) {
    for (const r of measured.rows) {
      console.log(`  [${r.cell === 'registry' ? 'R' : 'F'}] ${r.rel}`)
      for (const s of r.sites) console.log(`      ${String(s.line).padStart(4)}  ${s.text}`)
    }
  }
  return 0
}

console.error(`\n== kind-branch-audit：${errors.length} 处违规 ==`)
for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
if (VERBOSE) {
  for (const r of measured.rows) {
    console.error(`  [${r.cell === 'registry' ? 'R' : 'F'}] ${r.rel}`)
    for (const s of r.sites) console.error(`      ${String(s.line).padStart(4)}  ${s.text}`)
  }
}
return 1
}

// 只在被直接执行时跑；被测试 import 时什么也不做（口径原语 `countSites` 可单测）。
const MAIN =
  process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))
if (MAIN) process.exit(main())
