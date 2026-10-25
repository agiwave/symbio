#!/usr/bin/env node
/**
 * 事实 · `docs/004-ref/012-注入矩阵.md` —— 异常注入的**字段 × 能力面**矩阵
 *
 * 数据源是 `e2e/injection-matrix.mjs`（与 `scripts/injection-matrix-audit.mjs` 同一份）。
 * 本生成器只做排版：**能生成的表不手抄**——手抄的矩阵有两个坏死方向（用例号变死链、
 * 夹具加字段而矩阵漏行），换行重排也会让「有没有变」变成人工 diff。
 *
 * 不带溯源行（`stamp: false`）：输入是仓内数据，同输入必然同输出；时间戳只会让门禁
 * 每次都报「修复了 1 个文件」。
 *
 * 由 `scripts/gen-facts.mjs` 遍历 `scripts/facts/` 时调用，不要直接跑。
 */

import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { FACES, KINDS, CELLS } from '../../e2e/injection-matrix.mjs'
import { createRefLink } from './_ref-link.mjs'

const OUT_REL = 'docs/004-ref/012-注入矩阵.md'
const TITLE = '异常注入矩阵 · 字段 × 能力面'
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')

/** 一格的渲染：用例号 / 不适用理由 / 缺口归属。 */
function cellText(cell) {
  if (!cell) return '**未归属**'
  if (Array.isArray(cell.cases)) return cell.cases.join(' · ')
  if (typeof cell.na === 'string') return `— ${cell.na}`
  return `**缺口**：${cell.gap.owner} —— ${cell.gap.reason}`
}

function render({ root = ROOT } = {}) {
  const refLink = createRefLink(root, 'docs/004-ref')
  const byPair = new Map(CELLS.map((c) => [`${c.kind}|${c.face}`, c]))
  const cases = CELLS.filter((c) => Array.isArray(c.cases)).length
  const na = CELLS.filter((c) => typeof c.na === 'string').length
  const gap = CELLS.filter((c) => c.gap).length
  const total = KINDS.length * FACES.length

  const L = []
  L.push('---')
  L.push('kind: ref')
  L.push(`id: RE-012-注入矩阵`)
  L.push(`title: ${TITLE}`)
  L.push('status: active')
  L.push('created: 2026-10-25')
  L.push('---')
  L.push('')
  L.push(`# ${TITLE}`)
  L.push('')
  L.push('> ⚠️ **本表由 `scripts/facts/injection-matrix.mjs` 从 `e2e/injection-matrix.mjs` 生成，请勿手改。**')
  L.push('> 数据源是同一条判据的输入，审计方是 `scripts/injection-matrix-audit.mjs`（INJ-001…003）。')
  L.push('')
  L.push('## 1. 口径')
  L.push('')
  L.push('**注入字段** = 夹具上可编排的故障 / 时序字段（`mock-llm` 的场景字段、`mock-mcp` 的脚本位）。')
  L.push('正常内容形状（`chunks` / `toolCalls`）与固定端点（`mock-http` 的 `/slow` / `/missing`）不是注入：')
  L.push('前者是正常输入，后者是靶子内容，都不能「按字段分级」。')
  L.push('')
  L.push('**能力面** = 用户能看见注入后果的那一层，不是代码目录。一格 = 一个 `(kind, face)` 对，必须属于以下三类之一：')
  L.push('')
  L.push('| 归属 | 含义 | 判据 |')
  L.push('|---|---|---|')
  L.push('| 用例号 | 该字段在这条面上有独立锚 | 编号必须解析到 `e2e/cases/tNN-*.mjs`，且该用例**代码**真的用了这个字段（注释不算） |')
  L.push('| `— 理由` | 这条面没有独立观测点 | 理由必须说清「为什么已由另一条面或另一层判据覆盖」；写不出就应落成缺口 |')
  L.push('| **缺口** | 应该有、暂时没有 | `owner` 必须解析到存在的任务（`T-NNN`）或设计条目（`DE-NNN`） |')
  L.push('')
  L.push('未归属格与已登记缺口都进基线（只降不升，`gate.d/35-baseline.mjs`）。反向扫描也在审计里：')
  L.push('用例用了某字段、矩阵没有任何一格认领 ⇒ INJ-003 红——防的是「夹具加了用例、矩阵没人记得改」。')
  L.push('')
  L.push('## 2. 矩阵')
  L.push('')
  L.push(`| 注入字段 | ${FACES.map((f) => f.label).join(' | ')} |`)
  L.push(`|---|${FACES.map(() => '---').join('|')}|`)
  for (const k of KINDS) {
    const cells = FACES.map((f) => cellText(byPair.get(`${k.id}|${f.id}`)))
    L.push(`| **${k.label}**（\`${k.field}\`） | ${cells.join(' | ')} |`)
  }
  L.push('')
  L.push('## 3. 读数')
  L.push('')
  L.push(`- ${KINDS.length} 种注入 × ${FACES.length} 个面 = **${total} 格**：用例 ${cases} 格 · 不适用 ${na} 格 · 已登记缺口 ${gap} 格。`)
  L.push(`- 缺口 owner 全部为 \`DE-039\`（外部执行与熔断）：mock 夹具已具备编排能力，接线前不写判据，理由见 ${refLink('RE-005-e2e覆盖矩阵', 'RE-005-覆盖矩阵')} 的缺口节。`)
  L.push('- 本表的正常 / 异常覆盖面按**能力域**看另一张表：`RE-005`；本表只回答「每种注入在每个面上有没有锚」。')
  L.push('')
  return L.join('\n')
}

// ================= 事实描述符（由 `scripts/gen-facts.mjs` 遍历调用） =================
//
// 不带溯源行：全部输入是仓内数据，同输入必然同输出（理由见 `facts/_lib.mjs` 的 `stamp` 契约）。
export default {
  id: 'injection-matrix',
  title: '异常注入矩阵（注入字段 × 能力面）',
  out: OUT_REL,
  stamp: false,
  render,
}

// 让回归测试能直接吃同一份渲染函数，而不必起子进程。
export { render, cellText }
