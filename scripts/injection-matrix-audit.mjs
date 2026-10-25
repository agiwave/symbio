#!/usr/bin/env node
/**
 * injection-matrix-audit — **异常注入矩阵的归属完整性**（判定型读数）
 *
 * ## 它回答什么
 *
 * GO-003 E2：「注入按场景字段组织，每种注入有独立用例钉『注入会红 / 被测系统按纪律
 * 收敛』。」在矩阵之前，这句话只能靠人翻 `e2e/cases/`：没有一处能回答「12 种注入 ×
 * 4 个能力面，哪些有锚、哪些没有」。`docs/004-ref/005-e2e覆盖矩阵.md` 是按**能力域**
 * 列的，它看不见「同一种注入在另一个面上有没有测」。
 *
 * ## 口径
 *
 * 数据源是 `e2e/injection-matrix.mjs`（唯一 owner），本脚本只判三件事：
 *
 * 1. **每格归属**：`12 种注入 × 4 个面 = 48 格`，每格必须恰好一条记录，且是
 *    `cases` / `na` / `gap` 之一。少一格 = `unassigned`，进基线（目标 0）。
 * 2. **归属有效**：
 *    - `cases` 的每个编号必须解析到 `e2e/cases/tNN-*.mjs`，且该用例文本真的用了这个字段
 *      （`kind.usage`）——编号指向不存在的用例、或指向一个没用该字段的用例，都算假锚；
 *    - `na` 必须写理由（≥ 8 个非空白字符）；
 *    - `gap` 必须有 `owner` + 理由，`owner` 必须解析到存在的任务（`T-NNN`）或设计条目
 *      （`DE-NNN`）。指向不存在的任务 = 缺口没人接。
 * 3. **反向不漏**：扫 `e2e/cases/*.mjs`，某种注入字段被用到、但矩阵里没有任何一格认领
 *    该用例 ⇒ 红。这条判的是「夹具加了新用例、矩阵没人记得改」的那一侧。
 *
 * ## 判据码
 * - INJ-001：48 格全部归属；`unassigned` / `deferred` 只许降（棘轮进 `35-baseline`）。
 * - INJ-002：每格的归属有效（用例真用了字段 / `na` 有理由 / `gap` 有能打开的 owner）。
 * - INJ-003：反向不漏 + 数据源自洽（每种注入在夹具里真的存在、表非空）。
 *
 * ## 用法
 *   node scripts/injection-matrix-audit.mjs              # 判定
 *   node scripts/injection-matrix-audit.mjs --verbose    # 逐格清单
 *   node scripts/injection-matrix-audit.mjs --root=<dir> # 指向别处的仓库（测试用）
 * 退出码：0 = 通过，1 = 违规 / 范围读不出。
 *
 * 环境变量 `INJECTION_MATRIX_BASELINE=<unassigned>:<deferred>` **只给回归测试用**；
 * 覆盖值不得比默认值更松（`gate.d/35-baseline.mjs` 另判一次）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns INJ 异常注入矩阵
// @codes INJ-001 INJ-002 INJ-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'
import { FACES, KINDS, CELLS } from '../e2e/injection-matrix.mjs'

const argv = process.argv.slice(2)
const VERBOSE = argv.includes('--verbose')
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const CASES_REL = 'e2e/cases'
const BASELINE = process.env.INJECTION_MATRIX_BASELINE
  ? (() => {
      const [unassigned, deferred] = process.env.INJECTION_MATRIX_BASELINE.split(':').map((n) => Number(n))
      return { unassigned, deferred }
    })()
  : { unassigned: 0, deferred: 3 }

const nonSpace = (s) => (s ?? '').replace(/\s+/g, '').length

/**
 * 去注释、保留字符串：矩阵判的是「用例**代码**用没用到这个字段」，注释里提到字段名
 * （t18 就有一句 `failTimes: 3` 在解释计数口径）不是使用。字符串要留下——
 * `mock-actions.json` 这类证据恰恰住在参数里。
 */
export function stripComments(text) {
  let out = ''
  let quote = null
  for (let i = 0; i < text.length; i++) {
    const c = text[i]
    const n = text[i + 1]
    if (quote) {
      out += c
      if (c === '\\') {
        out += n ?? ''
        i++
      } else if (c === quote) quote = null
      continue
    }
    if (c === '"' || c === "'" || c === '`') {
      quote = c
      out += c
      continue
    }
    if (c === '/' && n === '/') {
      while (i < text.length && text[i] !== '\n') i++
      out += '\n'
      continue
    }
    if (c === '/' && n === '*') {
      i += 2
      while (i < text.length && !(text[i] === '*' && text[i + 1] === '/')) i++
      i++
      continue
    }
    out += c
  }
  return out
}

/** 编号解析到实际用例文件；多个同号文件也算读不出（编号必须唯一）。 */
export function caseFiles(root) {
  let names
  try {
    names = fs.readdirSync(path.join(root, CASES_REL)).filter((n) => /^t\d+-.*\.mjs$/.test(n))
  } catch {
    return null
  }
  const map = new Map()
  for (const name of names) {
    const id = name.slice(0, name.indexOf('-'))
    if (!map.has(id)) map.set(id, [])
    map.get(id).push(`${CASES_REL}/${name}`)
  }
  return map
}

/** `T-NNN` 任务文件 / `DE-NNN` 设计条目是否存在。 */
export function ownerResolves(root, owner) {
  const task = /^T-(\d+)(?:-|$)/.exec(owner)
  if (task) {
    const dir = path.join(root, 'docs', '007-task')
    try {
      return fs.readdirSync(dir).some((n) => n.startsWith(`${task[1]}-`))
    } catch {
      return false
    }
  }
  const de = /^DE-(\d+)/.exec(owner)
  if (de) {
    const dir = path.join(root, 'docs', '003-design')
    try {
      return fs.readdirSync(dir).some((n) => n.startsWith(`${de[1]}-`))
    } catch {
      return false
    }
  }
  return false
}

/**
 * 纯判定：输入数据 + 仓库根，返回违规列表与计数。
 * 抽成纯函数是为了回归测试能注入坏矩阵，而不必在临时目录里复制 70 个用例。
 */
export function validate({ kinds, faces, cells }, root) {
  const errors = []
  const report = (rule, msg) => errors.push({ rule, msg })
  const kindById = new Map(kinds.map((k) => [k.id, k]))
  const faceIds = new Set(faces.map((f) => f.id))
  const files = caseFiles(root)

  if (files === null) {
    report('INJ-003', `读不出 ${CASES_REL}——矩阵没有输入，宁红不静默`)
    return { errors, unassigned: 0, deferred: 0, rows: [] }
  }
  if (!kinds.length || !faces.length) {
    report('INJ-003', '注入种类或能力面为空——空表不能算「全覆盖」')
  }
  if (kindById.size !== kinds.length) report('INJ-003', '注入种类 id 重复——一格里出现两个字段')

  // 夹具字段自洽：每个 kind 声称的字段必须在对应夹具源码里真的存在。
  for (const k of kinds) {
    let text
    try {
      text = fs.readFileSync(path.join(root, k.mock), 'utf8')
    } catch {
      report('INJ-003', `${k.id}：读不出夹具 ${k.mock}`)
      continue
    }
    if (!k.inMock.test(text)) {
      report('INJ-003', `${k.id}：夹具 ${k.mock} 里找不到字段 ${k.field}（inMock 不命中）——矩阵在描述一个不存在的注入`)
    }
  }

  // 每格恰好一条。
  const byPair = new Map()
  for (const c of cells) {
    if (!kindById.has(c.kind)) {
      report('INJ-001', `${c.kind} × ${c.face}：kind 不在 KINDS 里`)
      continue
    }
    if (!faceIds.has(c.face)) {
      report('INJ-001', `${c.kind} × ${c.face}：face 不在 FACES 里`)
      continue
    }
    const key = `${c.kind}|${c.face}`
    if (byPair.has(key)) report('INJ-001', `${c.kind} × ${c.face}：重复归属（一格只能有一条记录）`)
    byPair.set(key, c)
  }

  let unassigned = 0
  let deferred = 0
  const rows = []
  for (const k of kinds) {
    for (const f of faces) {
      const c = byPair.get(`${k.id}|${f.id}`)
      if (!c) {
        unassigned++
        report('INJ-001', `${k.id} × ${f.id}：没有归属（用例 / 不适用理由 / 缺口 owner 三选一）`)
        continue
      }
      const hasCases = Array.isArray(c.cases) && c.cases.length > 0
      const hasNa = typeof c.na === 'string'
      const hasGap = c.gap && typeof c.gap === 'object'
      const modes = [hasCases, hasNa, hasGap].filter(Boolean).length
      if (modes !== 1) {
        report('INJ-002', `${k.id} × ${f.id}：必须且只能属于 cases / na / gap 之一（当前 ${modes} 种）`)
        continue
      }
      if (hasCases) {
        for (const id of c.cases) {
          const resolved = files.get(id)
          if (!resolved || resolved.length !== 1) {
            report('INJ-002', `${k.id} × ${f.id}：用例 ${id} ${resolved ? '重号' : '不存在'}`)
            continue
          }
          const text = stripComments(fs.readFileSync(path.join(root, resolved[0]), 'utf8'))
          if (!k.usage.test(text)) {
            report('INJ-002', `${k.id} × ${f.id}：${resolved[0]} 没有用到该注入字段（usage 不命中）——假锚`)
          }
        }
      } else if (hasNa) {
        if (nonSpace(c.na) < 8) report('INJ-002', `${k.id} × ${f.id}：\`na\` 理由过短——「不适用」必须说清为什么不适用`)
      } else {
        deferred++
        if (nonSpace(c.gap.reason) < 8) report('INJ-002', `${k.id} × ${f.id}：缺口没有可读理由`)
        if (!ownerResolves(root, c.gap.owner ?? '')) {
          report('INJ-002', `${k.id} × ${f.id}：缺口 owner \`${c.gap.owner ?? ''}\` 指向不存在的任务 / 设计条目`)
        }
      }
      rows.push({ ...c, kindLabel: k.label, faceLabel: f.label })
    }
  }
  if (unassigned > BASELINE.unassigned) {
    report('INJ-001', `未归属格 ${unassigned} > 基线 ${BASELINE.unassigned}——新注入字段必须先把 12 × 4 填满`)
  }
  if (deferred > BASELINE.deferred) {
    report('INJ-001', `已登记缺口的格 ${deferred} > 基线 ${BASELINE.deferred}——新缺口要么补用例，要么把 owner 写进矩阵并论证`)
  }

  // 反向：用例真的用了字段，而矩阵没有任何一格认领它。
  const claimed = new Map(kinds.map((k) => [k.id, new Set()]))
  for (const c of cells) {
    if (Array.isArray(c.cases)) for (const id of c.cases) claimed.get(c.kind)?.add(id)
  }
  for (const [id, paths] of files) {
    if (paths.length !== 1) continue
    const text = stripComments(fs.readFileSync(path.join(root, paths[0]), 'utf8'))
    for (const k of kinds) {
      if (k.usage.test(text) && !claimed.get(k.id)?.has(id)) {
        report('INJ-003', `${paths[0]} 使用了 ${k.id}（${k.field}），但矩阵没有认领它——补一格或把用例挪进已有格`)
      }
    }
  }

  return { errors, unassigned, deferred, rows }
}

function main() {
  const result = validate({ kinds: KINDS, faces: FACES, cells: CELLS }, ROOT)
  const { errors } = result
  if (errors.length === 0) {
    console.log('\n== injection-matrix-audit ==')
    console.log(
      `${green('✓')} INJ-001  ${KINDS.length} 种注入 × ${FACES.length} 个面 = ${KINDS.length * FACES.length} 格；` +
        `未归属 ${result.unassigned}/${BASELINE.unassigned} · 已登记缺口 ${result.deferred}/${BASELINE.deferred}`,
    )
    console.log(`${green('✓')} INJ-002  每格归属有效：用例真用了字段 / na 有理由 / gap 有能打开的 owner`)
    console.log(`${green('✓')} INJ-003  夹具字段与用例侧反向扫描一致，没有漏认领`)
    const under = Object.keys(BASELINE).filter((k) => result[k] < BASELINE[k])
    if (under.length) {
      console.log(
        yellow('提示') +
          `：已收到基线下方（${under.map((k) => `${k} ${result[k]}<${BASELINE[k]}`).join(' · ')}），` +
          `请同步本脚本的 BASELINE——留着旧数字等于把余量白留给下一次`,
      )
    }
    if (VERBOSE) {
      for (const r of result.rows) {
        const cell = r.cases
          ? r.cases.join(' · ')
          : r.na
            ? `不适用：${r.na.slice(0, 60)}`
            : `缺口：${r.gap.owner}`
        console.log(`  [${r.face}] ${r.kind.padEnd(22)} ${cell}`)
      }
    }
    return 0
  }
  console.error(`\n== injection-matrix-audit：${errors.length} 处违规 ==`)
  for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
  console.error(
    `\n${dim('数据源：e2e/injection-matrix.mjs；生成物：docs/004-ref/012-注入矩阵.md（node scripts/gen-facts.mjs）')}`,
  )
  return 1
}

const MAIN =
  process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))
if (MAIN) process.exit(main())
