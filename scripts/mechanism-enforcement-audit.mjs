#!/usr/bin/env node
/**
 * mechanism-enforcement-audit — 机制强制等级清单的对账（ME-001…ME-005）
 *
 * ## 它治什么
 *
 * 仓里的机制按「违规时谁会红」分三级：① 编译期强制 / ② 运行时阻断 / ③ 纯声明。
 * 前两级有执行方，第三级**没有**——它只写在文档、配置或默认值里。单看每个机制都
 * 「设计得不错」，但设计得好的机制里规则没有被执行方接住时，代码上长得完全一样：
 * 没有任何东西会红（[T-184] 的来源审计把这称为 9 个 B 的共同根因）。
 *
 * 本脚本不重复判「哪条机制该是几级」（那是 [RE-010] 的正文与各守卫的事），只判
 * **清单本身是否闭合**：
 *
 * | 编号 | 判什么 | 为什么 |
 * |---|---|---|
 * | ME-001 | 表形状 / 等级词表 / 机制键重名 | 形状读歪时后面的判据全是空转 |
 * | ME-002 | §1 机制键 ↔ `verify/facts/mod.rs::MECHANISMS` 双向相等 | 新机制键没进清单 = 没被过账 |
 * | ME-003 | 执行方解析不出（判据码未登记 / 代码锚点不存在） | 指向一个不存在的执行方是**假绿灯** |
 * | ME-004 | ③ 与豁免必须一一对应，且期限可解析、未闭环 | 没有期限的豁免 = 永久豁免 |
 * | ME-005 | 豁免总数超上限 | 「全部挂豁免」等于没有清单 |
 *
 * ## 两条口径
 *
 * - **判据码只有一份登记**：执行方写判据码时必须是 `docs/006-ref/GATE_CODES.md`
 *   里真实存在的号；写代码锚点（`path` / `path::symbol`）时文件必须存在、符号名
 *   必须在该文件正文里出现。两种形态都不认 ⇒ ME-003，不做「看起来像就算」。
 * - **豁免即期限**：`mechanism-allow <键>: <ADR 或任务编号> <理由>` 的第二列必须是
 *   真实登记条目；指向的条目已经闭环（done / wont / superseded / withdrawn）即红
 *   ——那时缺口要么已经被补上（删掉这一行、把等级改成 ①②），要么应当重新立项。
 *
 * ## 用法
 *
 *   node scripts/mechanism-enforcement-audit.mjs
 *   node scripts/mechanism-enforcement-audit.mjs --root=<仓库根>   # 回归测试用
 *
 * 退出码：0 = 清单闭合；1 = 有失效；2 = 判据输入读不出（不许「读空即全绿」）。
 */
// 判据码命名空间（登记表 docs/006-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns ME 机制强制等级清单
// @codes ME-001 ME-002 ME-003 ME-004 ME-005

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'
import { cellsOf, isSeparator, plain } from './md-table.mjs'
import { loadEntries } from './registry-lib.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')

/** 清单的地址（唯一 owner；地址变了就改这一处）。 */
export const LEDGER_REL = 'docs/006-ref/010-机制强制等级清单.md'
/** 机制键的机器可读来源（生成物：`scripts/gen-verify-facts.mjs` 从 PL-001 §8 抽取）。 */
export const MECHANISMS_REL = 'docs/004-plan/verify/facts/mod.rs'
/** 判据码登记表（生成物：`scripts/facts/gate-codes.mjs`）。 */
export const GATE_CODES_REL = 'docs/006-ref/GATE_CODES.md'

/** ③ 类豁免的总数上限。调大这个数等于弱化清单本身——要论证就走 ADR。 */
export const MAX_EXEMPTIONS = 8

const LEVELS = ['①', '②', '③']
const CODE_RE = /^[A-Z]{1,4}-\d{3}$/
const ALLOW_RE = /^mechanism-allow\s+(.+?):\s+(\S+)\s+(.+)$/
/** 指向「已经闭环」的条目 = 豁免过期（缺口要么补上了，要么应当重新立项）。 */
const CLOSED_STATUS = new Set(['done', 'wont', 'retired', 'superseded', 'withdrawn'])

/**
 * 解析清单正文。返回 `{ rows, errors }`，rows 带 `section`（用于 §1 覆盖判据）。
 *
 * 只认首列是 `机制` 的表——§0 的三列说明表不是清单数据。
 */
export function parseLedger(text) {
  const errors = []
  const rows = []
  const lines = text.split(/\r?\n/)
  let section = ''
  let inTable = false
  let header = null
  for (const line of lines) {
    const h = line.match(/^##\s+(.+?)\s*$/)
    if (h) {
      section = h[1]
      inTable = false
      header = null
      continue
    }
    const cells = cellsOf(line)
    if (!cells) {
      inTable = false
      header = null
      continue
    }
    if (isSeparator(cells)) continue
    if (!inTable) {
      if (plain(cells[0]) !== '机制') continue
      inTable = true
      header = cells.map(plain)
      if (header.length !== 4 || header[1] !== '等级' || header[2] !== '执行方' || header[3] !== '说明') {
        errors.push({ code: 'ME-001', where: section, message: `表头应为「机制 | 等级 | 执行方 | 说明」，实得 ${header.join(' | ')}` })
      }
      continue
    }
    if (cells.length !== 4) {
      errors.push({ code: 'ME-001', where: `${section}`, message: `列数为 ${cells.length}，应为 4` })
      continue
    }
    rows.push({
      section,
      key: plain(cells[0]),
      level: plain(cells[1]),
      executor: cells[2],
      note: plain(cells[3]),
    })
  }
  return { rows, errors }
}

/** 取 `mechanism-allow` 围栏内的行；围栏不存在返回 null（空围栏返回 `[]`，不是 null）。 */
export function allowBlock(text) {
  const lines = text.split(/\r?\n/)
  const out = []
  let open = false
  for (const raw of lines) {
    const line = raw.trim()
    if (!open) {
      if (/^```+\s*mechanism-allow\s*$/.test(line)) open = true
      continue
    }
    if (/^```+\s*$/.test(line)) return out
    out.push(raw)
  }
  return null
}

/** 一份机制清单的完整判定（纯函数：所有输入由调用方给，便于双向回归）。 */
export function audit({ text, mechanisms, codes, resolveAnchor, entries }) {
  const errors = []
  const { rows, errors: shapeErrors } = parseLedger(text)
  errors.push(...shapeErrors)
  if (rows.length === 0) {
    errors.push({ code: 'ME-001', where: LEDGER_REL, message: '一行机制都没读到（清单读空不是「全部通过」）' })
    return { errors, rows, allows: [] }
  }

  // ME-001：等级词表 + 重名
  const byKey = new Map()
  for (const row of rows) {
    if (!LEVELS.includes(row.level)) {
      errors.push({ code: 'ME-001', where: row.key, message: `等级 ${JSON.stringify(row.level)} 不在 ${LEVELS.join(' / ')}` })
    }
    if (byKey.has(row.key)) {
      errors.push({ code: 'ME-001', where: row.key, message: '机制键重名（同一行事实只能有一个 owner）' })
    } else {
      byKey.set(row.key, row)
    }
  }

  // ME-002：§1 与机器来源双向相等
  const declared = new Set(rows.filter((r) => /^1\./.test(r.section)).map((r) => r.key))
  const expected = new Set(mechanisms)
  for (const key of expected) {
    if (!declared.has(key)) {
      errors.push({ code: 'ME-002', where: key, message: '机制键不在清单 §1（新键必须过账；缺行即红）' })
    }
  }
  for (const key of declared) {
    if (!expected.has(key)) {
      errors.push({ code: 'ME-002', where: key, message: '清单 §1 有这个键，但机器来源（MECHANISMS）里没有' })
    }
  }

  // ME-003：执行方必须可解析
  for (const row of rows) {
    if (row.level === '③') continue
    const tokens = [...row.executor.matchAll(/`([^`]+)`/g)].map((m) => m[1].trim())
    if (tokens.length === 0) {
      errors.push({ code: 'ME-003', where: row.key, message: `${row.level} 类必须给出执行方（判据码或代码锚点）` })
      continue
    }
    for (const token of tokens) {
      if (CODE_RE.test(token)) {
        if (!codes.has(token)) {
          errors.push({ code: 'ME-003', where: row.key, message: `判据码 ${token} 不在 GATE_CODES 登记表里` })
        }
        continue
      }
      if (!/[/.]/.test(token)) {
        errors.push({ code: 'ME-003', where: row.key, message: `执行方 ${JSON.stringify(token)} 既不是判据码也不是代码锚点` })
        continue
      }
      if (!resolveAnchor(token)) {
        errors.push({ code: 'ME-003', where: row.key, message: `代码锚点 ${token} 解析不出（文件或符号不存在）` })
      }
    }
  }

  // ME-004 / ME-005：豁免
  const block = allowBlock(text)
  const allows = []
  if (block === null) {
    errors.push({ code: 'ME-001', where: '§6', message: '缺少 ```mechanism-allow 围栏（③ 的豁免住在那里）' })
  } else {
    for (const line of block) {
      if (!line.trim()) continue
      const m = line.match(ALLOW_RE)
      if (!m) {
        errors.push({ code: 'ME-004', where: line.trim(), message: '格式应为 `mechanism-allow <机制键>: <ADR 或任务编号> <理由>`' })
        continue
      }
      const [, key, term, reason] = m
      allows.push({ key: key.trim(), term, reason: reason.trim() })
      const row = byKey.get(key.trim())
      if (!row) {
        errors.push({ code: 'ME-004', where: key.trim(), message: '豁免指向一个清单里不存在的机制键' })
      } else if (row.level !== '③') {
        errors.push({ code: 'ME-004', where: key.trim(), message: `${row.level} 类有执行方，不该挂豁免（等级与豁免必须一致）` })
      }
      if (!reason.trim()) {
        errors.push({ code: 'ME-004', where: key.trim(), message: '豁免理由不可为空' })
      }
      const status = entries.get(term)
      if (status === undefined) {
        errors.push({ code: 'ME-004', where: key.trim(), message: `豁免期限 ${term} 不是已登记条目` })
      } else if (CLOSED_STATUS.has(status)) {
        errors.push({
          code: 'ME-004',
          where: key.trim(),
          message: `豁免期限 ${term} 已闭环（${status}）——缺口要么已补上（改等级并删掉这一行），要么重新立项`,
        })
      }
    }
    for (const row of rows) {
      if (row.level === '③' && !allows.some((a) => a.key === row.key)) {
        errors.push({ code: 'ME-004', where: row.key, message: '③ 纯声明没有显式豁免（要么接执行方，要么挂 `mechanism-allow`）' })
      }
    }
  }
  if (allows.length > MAX_EXEMPTIONS) {
    errors.push({ code: 'ME-005', where: '§6', message: `豁免 ${allows.length} 条，超过上限 ${MAX_EXEMPTIONS}——「全部挂豁免」等于没有清单` })
  }

  return { errors, rows, allows }
}

// ── CLI ──────────────────────────────────────────────────────────────────

function read(file) {
  return fs.readFileSync(path.join(ROOT, file), 'utf8')
}

/** 代码锚点解析：`path` 必须存在；`path::symbol` 还要求符号名在该文件正文里。 */
function resolveAnchor(token) {
  const [file, symbol] = token.split('::')
  const abs = path.join(ROOT, file)
  if (!fs.existsSync(abs)) return false
  if (!symbol) return true
  const text = fs.readFileSync(abs, 'utf8')
  const leaf = symbol.split('::').pop().split('.').pop()
  return text.includes(leaf)
}

function mechanismsFromFacts(text) {
  const m = text.match(/pub const MECHANISMS:\s*&\[&str\]\s*=\s*&\[([\s\S]*?)\];/)
  if (!m) return null
  return [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1])
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log('--- mechanism-enforcement-audit: 机制强制等级清单（ME-001…ME-005） ---')

  let text
  let mechanisms
  let codesText
  let registry
  try {
    text = read(LEDGER_REL)
    mechanisms = mechanismsFromFacts(read(MECHANISMS_REL))
    codesText = read(GATE_CODES_REL)
    registry = loadEntries(ROOT)
  } catch (e) {
    console.error(red(`✗ 判据输入读不出：${e.message}`))
    process.exit(2)
  }
  if (!mechanisms || mechanisms.length === 0) {
    console.error(red(`✗ 失去判据：${MECHANISMS_REL} 里一个机制键都没读出来`))
    process.exit(2)
  }
  const codes = new Set([...codesText.matchAll(/\b([A-Z]{1,4}-\d{3})\b/g)].map((m) => m[1]))
  if (codes.size === 0) {
    console.error(red(`✗ 失去判据：${GATE_CODES_REL} 里一个判据码都没读出来`))
    process.exit(2)
  }
  // 豁免期限写短号（`T-188` / `ADR-099`），登记条目的 id 带标题后缀——两种形态都要认。
  const entries = new Map()
  for (const e of registry.entries) {
    const id = String(e.meta.id)
    const m = id.match(/^([A-Z]{1,4}-\d{3})/)
    entries.set(m ? m[1] : id, String(e.meta.status))
  }

  const { errors, rows, allows } = audit({ text, mechanisms, codes, resolveAnchor, entries })

  const names = {
    'ME-001': '表形状 / 等级词表 / 机制键重名',
    'ME-002': '§1 机制键与机器来源不相等',
    'ME-003': '执行方解析不出',
    'ME-004': '③ 与豁免不一一对应 / 期限失效',
    'ME-005': '豁免总数超上限',
  }
  for (const e of errors) console.log(`${red('[ERROR]')} ${e.code} ${e.where}  ${e.message}`)

  console.log('')
  console.log(`机制行 ${rows.length} · 机制键 ${mechanisms.length} · 豁免 ${allows.length}/${MAX_EXEMPTIONS}`)
  if (errors.length) {
    const hits = new Map()
    for (const e of errors) hits.set(e.code, (hits.get(e.code) ?? 0) + 1)
    for (const [code, n] of hits) console.log(`  ${red('✗')} ${code} ${names[code]} × ${n}`)
    console.log(red(`\n机制强制等级清单不闭合：${errors.length} 处失效`))
    process.exit(1)
  }
  console.log(green('✓ 清单闭合：机制键齐全、执行方全部可解析、③ 全部有期限'))
  console.log(dim(`  资产：${LEDGER_REL}`))
}
