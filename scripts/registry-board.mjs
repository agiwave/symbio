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
 *   node scripts/registry-board.mjs                    全部分组（待处理 / 进行中 / 已完成 / 不做 / 现行规范…）
 *   node scripts/registry-board.mjs --kind=task        只看某个 kind
 *   node scripts/registry-board.mjs --status=todo      按状态过滤
 *   node scripts/registry-board.mjs --id=T-001         打印单条（字段 + 正文全文）
 *   node scripts/registry-board.mjs --root=<仓库根>    （回归测试夹具用）
 *
 * 退出码：0 = 通过；1 = 条目有违规（不打印看板）；2 = 范围读不出 / 参数不识别。
 * 汇总页的文件形态在 `registry/` 各级 `INDEX.md`（门禁自动重生成）；本脚本只服务「现在要干什么」。
 */

import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, dim } from './color.mjs'
import { KINDS, loadEntries, auditEntries, topoOrder, sectionOf, isEmpty } from './registry-lib.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')

const USAGE =
  '用法：node scripts/registry-board.mjs [--kind=<kind>] [--status=<状态>] [--id=<编号>] [--root=<仓库根>]\n' +
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

function printTaskGroup(lines, entries, status, title) {
  const mine = entries.filter((e) => e.meta.status === status)
  lines.push(`—— ${title} ${mine.length ? '' : '——（无）'}`)
  if (!mine.length) return
  const pad = Math.max(...mine.map((e) => e.meta.id.length))
  for (const e of mine) {
    const deps = Array.isArray(e.meta.deps) && e.meta.deps.length ? `  依赖 ${e.meta.deps.join('、')}` : ''
    const prio = e.meta.prio ? `·${e.meta.prio}` : ''
    const extra = status === 'done' && e.meta.evidence ? `  证据 ${e.meta.evidence}` : ''
    const why = status === 'wont' && e.meta.reason ? `  理由 ${e.meta.reason}` : ''
    lines.push(`  ${TASK_GLYPH[status]} ${e.meta.id.padEnd(pad)}  ${e.meta.title}  [${e.meta.track}${prio}]${deps}${extra}${why}`)
  }
}

function printSpecGroup(lines, entries, status, title) {
  const mine = entries.filter((e) => e.meta.status === status)
  lines.push(`—— ${title} ${mine.length ? '' : '——（无）'}`)
  if (!mine.length) return
  const pad = Math.max(...mine.map((e) => e.meta.id.length))
  for (const e of mine) {
    lines.push(`  ${status === 'retired' ? '❌' : status === 'draft' ? '📝' : '✅'} ${e.meta.id.padEnd(pad)}  ${e.meta.title}  ${e.meta.scope}`)
  }
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
  for (const a of argv) {
    if (a === '--strict') continue
    else if (a.startsWith('--root=')) root = path.resolve(a.slice(7))
    else if (a.startsWith('--kind=')) kindFilter = a.slice(7)
    else if (a.startsWith('--status=')) statusFilter = a.slice(9)
    else if (a.startsWith('--id=')) idFilter = a.slice(5)
    else return { code: 2, lines: [red(`[ERROR] 参数不识别：${a}`), USAGE] }
  }
  if (kindFilter && !(kindFilter in KINDS)) {
    return { code: 2, lines: [red(`[ERROR] 未知 kind：${kindFilter}（可选：${Object.keys(KINDS).join(' / ')}）`), USAGE] }
  }

  const { entries, errors, missing } = loadEntries(path.join(root, 'registry'), root)
  if (missing) return { code: 2, lines: [red(`[ERROR] RG-010 registry/ 不存在（${root}）——登记范围读不出`)] }
  if (entries.length === 0) {
    if (errors.length) {
      const lines = [red('registry-board：条目有违规，拒绝打印（报告说假话比没有更坏）')]
      for (const e of errors) lines.push(red(`[ERROR] ${e}`))
      return { code: 1, lines }
    }
    return { code: 2, lines: [red('[ERROR] RG-010 registry/ 下没有任何条目——登记范围读不出')] }
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

  const shown = entries.filter((e) => !kindFilter || e.kind === kindFilter)
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
  lines.push(`=== 登记看板（registry/）===`)
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
      printer(lines, mine, status, title)
    }
  }
  byKind('task', TASK_GROUPS, printTaskGroup)
  byKind('spec', SPEC_GROUPS, printSpecGroup)
  if (lines.length === 2) lines.push(dim('（没有匹配的分组）'))
  return { code: 0, lines }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { code, lines } = runRegistryBoard(process.argv.slice(2))
  for (const l of lines) console.log(l)
  process.exit(code)
}
