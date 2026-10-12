#!/usr/bin/env node
/**
 * registry-board — 登记看板：把条目按状态分组打印到 stdout
 *
 * 与 `registry-audit` 的分工：audit 是**守卫**（判红、--write 写汇总页），
 * 本脚本是**查询台**（读 + 校验 + 打印，不写任何文件）。它先跑同一套校验再打印——
 * 报告型脚本说假话比没有更坏（`schema-audit` 的实测教训：坏掉的报告天天被读），
 * 所以有违规时**只报错、不打印结论**。
 *
 * 用法：
 *   node scripts/registry-board.mjs                    全部分组（任务 / 规范 / 决策 / 各文档类 kind）
 *   node scripts/registry-board.mjs --kind=task        只看某个 kind（含 goal / stage / plan / arch / ref / guide）
 *   node scripts/registry-board.mjs --status=todo      按状态过滤
 *   node scripts/registry-board.mjs --id=T-001         打印单条（字段 + 正文全文；文档类的 id 是相对路径）
 *   node scripts/registry-board.mjs --children=T-056   打印该条目的分解子树（parent 字段，递归孙辈）
 *   node scripts/registry-board.mjs --parent=T-062     打印该条目的父链（从顶层祖先到当前条目）
 *   node scripts/registry-board.mjs --root=<仓库根>    （回归测试夹具用）
 *
 * 退出码：0 = 通过；1 = 条目有违规（不打印看板）；2 = 范围读不出 / 参数不识别。
 * 汇总页的文件形态在 `docs/INDEX.md` 与 `docs/<kind>/INDEX.md`（门禁自动重生成）；
 * 本脚本只服务「现在要干什么」。
 */

import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, dim } from './color.mjs'
import { KINDS, loadEntries, auditEntries, topoOrder, sectionOf, isEmpty, parentDepth } from './registry-lib.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')

const USAGE =
  '用法：node scripts/registry-board.mjs [--kind=<kind>] [--status=<状态>] [--id=<编号>] [--children=<编号>] [--parent=<编号>] [--root=<仓库根>]\n' +
  '退出码：0 = 通过；1 = 条目有违规；2 = 范围读不出 / 参数不识别'

const TASK_GLYPH = { todo: '☐', doing: '🟡', done: '☑', wont: '❌' }

/** task：状态 → 分组标题（按依赖序）；spec：状态 → 分组标题。 */
const TASK_GROUPS = [
  ['todo', '任务 · 待处理（按依赖顺序）'],
  ['doing', '任务 · 进行中'],
  ['done', '任务 · 已完成'],
  ['wont', '任务 · 不做'],
]
const SPEC_GROUPS = [
  ['active', '规范 · 现行'],
  ['draft', '规范 · 草案'],
  ['retired', '规范 · 废止'],
]

/** 其余 kind（adr + 文档类）的状态图标与理由列——按各自的状态词表分组。 */
const GENERIC_GLYPH = { active: '✅', draft: '📝', retired: '❌', superseded: '↩️', withdrawn: '🚫' }

function printGenericGroup(lines, entries, status, title, depth) {
  const mine = entries.filter((e) => e.meta.status === status)
  lines.push(`—— ${title} ${mine.length ? '' : '——（无）'}`)
  if (!mine.length) return
  const pad = Math.max(...mine.map((e) => e.meta.id.length))
  for (const e of mine) {
    const impl = e.kind === 'adr' && e.meta.impl ? ` 落地 ${e.meta.impl}` : ''
    const why = e.meta.reason ? `  理由 ${e.meta.reason}` : ''
    lines.push(`  ${GENERIC_GLYPH[status] ?? '·'} ${'  '.repeat(depth?.get(e.meta.id) ?? 0)}${e.meta.id.padEnd(pad)}  ${e.meta.title}${impl}${why}`)
  }
}

function printTaskGroup(lines, entries, status, title, depth) {
  const mine = entries.filter((e) => e.meta.status === status)
  lines.push(`—— ${title} ${mine.length ? '' : '——（无）'}`)
  if (!mine.length) return
  const pad = Math.max(...mine.map((e) => e.meta.id.length))
  for (const e of mine) {
    const deps = Array.isArray(e.meta.deps) && e.meta.deps.length ? `  依赖 ${e.meta.deps.join('、')}` : ''
    const prio = e.meta.prio ? `·${e.meta.prio}` : ''
    const extra = status === 'done' && e.meta.evidence ? `  证据 ${e.meta.evidence}` : ''
    const why = status === 'wont' && e.meta.reason ? `  理由 ${e.meta.reason}` : ''
    lines.push(
      `  ${TASK_GLYPH[status]} ${'  '.repeat(depth?.get(e.meta.id) ?? 0)}${e.meta.id.padEnd(pad)}  ${e.meta.title}  [${e.meta.track}${prio}]${deps}${extra}${why}`,
    )
  }
}

function printSpecGroup(lines, entries, status, title, depth) {
  const mine = entries.filter((e) => e.meta.status === status)
  lines.push(`—— ${title} ${mine.length ? '' : '——（无）'}`)
  if (!mine.length) return
  const pad = Math.max(...mine.map((e) => e.meta.id.length))
  for (const e of mine) {
    lines.push(
      `  ${status === 'retired' ? '❌' : status === 'draft' ? '📝' : '✅'} ${'  '.repeat(depth?.get(e.meta.id) ?? 0)}${e.meta.id.padEnd(pad)}  ${e.meta.title}  ${e.meta.scope}`,
    )
  }
}

const glyphOf = (e) => TASK_GLYPH[e.meta.status] ?? GENERIC_GLYPH[e.meta.status] ?? '·'
const idCmp = (a, b) => (a.meta.id < b.meta.id ? -1 : 1)

/**
 * 编号找不到时的友好提示：文档类编号带名称段（`GO-001-前端去业务化`），
 * 传裸前缀（`GO-001`）时指出唯一候选——查询仍保持精确匹配（与 `--id` 同语义）。
 */
function notFound(lines, entries, id, what) {
  const hits = entries.filter((e) => e.meta && e.meta.id.startsWith(`${id}-`))
  const hint = hits.length === 1 ? `（是否想找 \`${hits[0].meta.id}\`？文档类编号带名称段，查询用完整 id）` : ''
  return { code: 1, lines: [red(`[ERROR] 没有编号为 ${id} 的${what}${hint}`)] }
}

/**
 * `--children=<编号>`：以该条目为根的**分解子树**（parent 字段的投影）。
 * 跨状态全景（子条目 wont / done 也显示）——与 INDEX 的分解树小节同一数据两个出口。
 */
function printChildren(lines, entries, rootId) {
  const root = entries.find((e) => e.meta && e.meta.id === rootId)
  if (!root) return notFound(lines, entries, rootId, '条目')
  const kids = (id) => entries.filter((e) => e.meta && e.meta.parent === id).sort(idCmp)
  const direct = kids(root.meta.id)
  // 递归总数（含各级孙辈；seen 防环——环已被 RG-024 判红，这里只是不赌）
  const seenAll = new Set([root.meta.id])
  let total = 0
  const countAll = (id) => {
    for (const c of kids(id)) {
      if (seenAll.has(c.meta.id)) continue
      seenAll.add(c.meta.id)
      total++
      countAll(c.meta.id)
    }
  }
  countAll(root.meta.id)
  // 直接子项的状态分布（「大任务完成多少了」的最短答案，按该 kind 状态机顺序）
  const statusOrder = Object.keys(KINDS[root.kind].statusRules ?? {})
  const sc = new Map()
  for (const k of direct) sc.set(k.meta.status, (sc.get(k.meta.status) ?? 0) + 1)
  const parts = statusOrder.filter((s) => sc.has(s)).map((s) => `${s} ${sc.get(s)}`)
  const head = direct.length
    ? `：直接子 ${direct.length}（${parts.join(' / ')}）${total > direct.length ? ` · 递归共 ${total} 个` : ''}`
    : ''
  lines.push(`=== ${rootId} 的分解子树（parent 字段）${head}===`)
  const seen = new Set()
  const walk = (e, d) => {
    if (seen.has(e.meta.id)) return
    seen.add(e.meta.id)
    lines.push(`${'  '.repeat(d)}${glyphOf(e)} ${e.meta.id}  ${e.meta.title}（${e.meta.status}）`)
    for (const c of kids(e.meta.id)) walk(c, d + 1)
  }
  walk(root, 0)
  if (!direct.length) lines.push(dim('（该条目没有子条目——顶层条目）'))
  return { code: 0, lines }
}

/** `--parent=<编号>`：该条目的**父链**（从顶层祖先到当前条目，一级一缩进）。 */
function printParentChain(lines, entries, id) {
  const byId = new Map(entries.filter((e) => e.meta).map((e) => [e.meta.id, e]))
  const cur = byId.get(id)
  if (!cur) return notFound(lines, entries, id, '条目')
  const chain = []
  const guard = new Set()
  let e = cur
  while (e && !guard.has(e.meta.id)) {
    guard.add(e.meta.id)
    chain.unshift(e)
    e = typeof e.meta.parent === 'string' ? byId.get(e.meta.parent) : null
  }
  lines.push(`=== ${id} 的父链（parent 字段，深度 ${chain.length - 1}）===`)
  chain.forEach((x, i) => {
    const mark = i === chain.length - 1 ? '  ← 当前' : ''
    lines.push(`${'  '.repeat(i)}${glyphOf(x)} ${x.meta.id}  ${x.meta.title}（${x.meta.status}）${mark}`)
  })
  if (chain.length === 1) lines.push(dim('（无 parent——顶层条目）'))
  return { code: 0, lines }
}

function printRecord(lines, e) {
  lines.push(`=== ${e.meta.id}（${e.kind} · ${e.meta.status}）===`)
  for (const [k, v] of Object.entries(e.meta)) {
    lines.push(`  ${k}: ${Array.isArray(v) ? (v.length ? v.join(', ') : '[]') : isEmpty(v) ? '（空）' : v}`)
  }
  lines.push('')
  for (const l of e.body.split('\n')) lines.push(`  ${l}`)
}

/**
 * CLI 入口（导出供回归测试直接调用）。
 * @param {string[]} argv
 * @returns {{code: 0|1|2, lines: string[]}}
 */
export function runRegistryBoard(argv) {
  let root = defaultRoot
  let kindFilter = null
  let statusFilter = null
  let idFilter = null
  let childrenFilter = null
  let parentFilter = null
  for (const a of argv) {
    if (a === '--strict') continue
    else if (a.startsWith('--root=')) root = path.resolve(a.slice(7))
    else if (a.startsWith('--kind=')) kindFilter = a.slice(7)
    else if (a.startsWith('--status=')) statusFilter = a.slice(9)
    else if (a.startsWith('--id=')) idFilter = a.slice(5)
    else if (a.startsWith('--children=')) childrenFilter = a.slice(11)
    else if (a.startsWith('--parent=')) parentFilter = a.slice(9)
    else return { code: 2, lines: [red(`[ERROR] 参数不识别：${a}`), USAGE] }
  }
  if (kindFilter && !(kindFilter in KINDS)) {
    return { code: 2, lines: [red(`[ERROR] 未知 kind：${kindFilter}（可选：${Object.keys(KINDS).join(' / ')}）`), USAGE] }
  }
  if ((idFilter && (childrenFilter || parentFilter)) || (childrenFilter && parentFilter)) {
    return { code: 2, lines: [red('[ERROR] --id / --children / --parent 三选一（单条视图与分解视图不混用）'), USAGE] }
  }

  const { entries, errors, missing } = loadEntries(root)
  if (missing) return { code: 2, lines: [red(`[ERROR] RG-010 登记范围读不出（${root}）——仓内扫不到任何已登记条目`)] }
  if (entries.length === 0) {
    if (errors.length) {
      const lines = [red('registry-board：条目有违规，拒绝打印（报告说假话比没有更坏）')]
      for (const e of errors) lines.push(red(`[ERROR] ${e}`))
      return { code: 1, lines }
    }
    return { code: 2, lines: [red('[ERROR] RG-010 仓内扫不到任何已登记条目——登记范围读不出')] }
  }
  const errs = [...errors, ...auditEntries(entries, root)]
  if (errs.length) {
    const lines = [red('registry-board：条目有违规，拒绝打印（报告说假话比没有更坏）')]
    for (const e of errs) lines.push(red(`[ERROR] ${e}`))
    return { code: 1, lines }
  }

  const lines = []
  if (idFilter) {
    const e = entries.find((x) => x.meta && x.meta.id === idFilter)
    if (!e) return { code: 1, lines: [red(`[ERROR] 没有编号为 ${idFilter} 的条目`)] }
    printRecord(lines, e)
    return { code: 0, lines }
  }
  if (childrenFilter) return printChildren(lines, entries, childrenFilter)
  if (parentFilter) return printParentChain(lines, entries, parentFilter)

  const shown = entries.filter((e) => !kindFilter || e.kind === kindFilter)
  const depth = parentDepth(shown)
  const dist = Object.entries(KINDS)
    .filter(([kind]) => !kindFilter || kind === kindFilter)
    .map(([kind, cfg]) => {
      const mine = shown.filter((e) => e.kind === kind)
      const parts = Object.keys(cfg.statusRules)
        .map((s) => `${s} ${mine.filter((e) => e.meta.status === s).length}`)
        .join(' · ')
      return `${kind} ${mine.length}（${parts}）`
    })
    .join(' · ')
  lines.push(`=== 登记看板（全仓登记条目）===`)
  lines.push(`共 ${shown.length} 个条目：${dist}`)

  const byKind = (kind, groups, printer) => {
    if (kindFilter && kindFilter !== kind) return
    const mine = topoOrder(shown.filter((e) => e.kind === kind))
    if (kind === 'task') {
      const ordered = topoOrder(shown.filter((e) => e.kind === 'task'))
      const order = new Map(ordered.map((e, i) => [e, i]))
      mine.sort((a, b) => order.get(a) - order.get(b))
    }
    for (const [status, title] of groups) {
      if (statusFilter && statusFilter !== status) continue
      // 组内按 parent 深度缩进（顶层 = 0，无 parent 数据时全部为 0——输出与加机制前一致）
      printer(lines, mine, status, title, depth)
    }
  }
  byKind('task', TASK_GROUPS, printTaskGroup)
  byKind('spec', SPEC_GROUPS, printSpecGroup)
  for (const [kind, cfg] of Object.entries(KINDS)) {
    if (kind === 'task' || kind === 'spec') continue
    const groups = Object.keys(cfg.statusRules).map((s) => [s, `${cfg.label} · ${s}`])
    byKind(kind, groups, printGenericGroup)
  }
  if (lines.length === 2) lines.push(dim('（没有匹配的分组）'))
  return { code: 0, lines }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { code, lines } = runRegistryBoard(process.argv.slice(2))
  for (const l of lines) console.log(l)
  process.exit(code)
}
