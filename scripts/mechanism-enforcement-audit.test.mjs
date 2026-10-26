import assert from 'node:assert/strict'
import test from 'node:test'

import { audit, MAX_EXEMPTIONS } from './mechanism-enforcement-audit.mjs'

const HEADER = '| 机制 | 等级 | 执行方 | 说明 |\n|:-:|:-:|---|---|\n'
const mechanismsOf = (text) => [...text.matchAll(/^\| `([^`]+)` \|/gm)].map((m) => m[1])

function ledger({ rows = [], allows = [], extra = '' } = {}) {
  const body = rows.map((r) => `| ${r} |`).join('\n')
  const allowBlock = allows.length
    ? '\n```mechanism-allow\n' + allows.join('\n') + '\n```\n'
    : '\n```mechanism-allow\n```\n'
  return `# test\n\n## 1. 机制键（1，恒定）\n\n${HEADER}${body}\n${extra}\n## 6. 豁免\n${allowBlock}`
}

const CLEAN_ROWS = [
  '`store` | ② | `crates/store.rs` | ok',
  '`I2` | ③ | — | 读侧才有检查',
]
const CLEAN_ALLOWS = ['mechanism-allow I2: T-188 写侧阻断归 T-188']

function run(text) {
  return audit({
    text,
    mechanisms: mechanismsOf(text),
    codes: new Set(['ME-001', 'TD-001']),
    resolveAnchor: (token) => token === 'crates/store.rs',
    entries: new Map([
      ['T-188', 'todo'],
      ['T-186', 'done'],
    ]),
  })
}

test('干净清单：机制键齐全、执行方可解析、③ 有期限 ⇒ 零失效', () => {
  const r = run(ledger({ rows: CLEAN_ROWS, allows: CLEAN_ALLOWS }))
  assert.equal(r.errors.length, 0)
  assert.equal(r.rows.length, 2)
  assert.equal(r.allows.length, 1)
})

test('ME-001：等级写出词表 ⇒ 红', () => {
  const r = run(ledger({ rows: ['`store` | 高 | `crates/store.rs` | ok'], allows: [] }))
  assert.ok(r.errors.some((e) => e.code === 'ME-001' && /等级/.test(e.message)))
})

test('ME-001：列数不对 ⇒ 红', () => {
  const r = run(ledger({ rows: ['`store` | ② | `crates/store.rs`'], allows: [] }))
  assert.ok(r.errors.some((e) => e.code === 'ME-001' && /列数/.test(e.message)))
})

test('ME-001：机制键重名 ⇒ 红', () => {
  const r = run(
    ledger({
      rows: ['`store` | ② | `crates/store.rs` | a', '`store` | ② | `crates/store.rs` | b'],
      allows: [],
    }),
  )
  assert.ok(r.errors.some((e) => e.code === 'ME-001' && /重名/.test(e.message)))
})

test('ME-001：清单读空 ⇒ 红（不是「全部通过」）', () => {
  const r = audit({
    text: '# nothing here\n',
    mechanisms: ['store'],
    codes: new Set(),
    resolveAnchor: () => true,
    entries: new Map(),
  })
  assert.ok(r.errors.some((e) => e.code === 'ME-001' && /一行机制都没读到/.test(e.message)))
})

test('ME-002：机器来源有键而清单缺行 ⇒ 红', () => {
  const text = ledger({ rows: CLEAN_ROWS, allows: CLEAN_ALLOWS })
  const r = audit({
    text,
    mechanisms: [...mechanismsOf(text), 'projection'],
    codes: new Set(),
    resolveAnchor: () => true,
    entries: new Map([['T-188', 'todo']]),
  })
  assert.ok(r.errors.some((e) => e.code === 'ME-002' && e.where === 'projection'))
})

test('ME-002：§1 有行而机器来源没有 ⇒ 红', () => {
  const text = ledger({ rows: CLEAN_ROWS, allows: CLEAN_ALLOWS })
  const r = audit({
    text,
    mechanisms: ['store'],
    codes: new Set(),
    resolveAnchor: () => true,
    entries: new Map([['T-188', 'todo']]),
  })
  assert.ok(r.errors.some((e) => e.code === 'ME-002' && e.where === 'I2'))
})

test('ME-003：判据码不在登记表 ⇒ 红', () => {
  const r = run(ledger({ rows: ['`store` | ② | `ZZ-999` | ok'], allows: [] }))
  assert.ok(r.errors.some((e) => e.code === 'ME-003' && /ZZ-999/.test(e.message)))
})

test('ME-003：代码锚点解析不出 ⇒ 红', () => {
  const r = run(ledger({ rows: ['`store` | ② | `crates/missing.rs` | ok'], allows: [] }))
  assert.ok(r.errors.some((e) => e.code === 'ME-003' && /missing/.test(e.message)))
})

test('ME-003：①② 没写执行方 ⇒ 红', () => {
  const r = run(ledger({ rows: ['`store` | ② | — | ok'], allows: [] }))
  assert.ok(r.errors.some((e) => e.code === 'ME-003' && /必须给出执行方/.test(e.message)))
})

test('ME-004：③ 没有豁免 ⇒ 红', () => {
  const r = run(ledger({ rows: CLEAN_ROWS, allows: [] }))
  assert.ok(r.errors.some((e) => e.code === 'ME-004' && e.where === 'I2'))
})

test('ME-004：①② 挂豁免 ⇒ 红（等级与豁免必须一致）', () => {
  const r = run(
    ledger({ rows: CLEAN_ROWS, allows: [...CLEAN_ALLOWS, 'mechanism-allow store: T-188 不该挂'] }),
  )
  assert.ok(r.errors.some((e) => e.code === 'ME-004' && /不该挂豁免/.test(e.message)))
})

test('ME-004：豁免期限已闭环 ⇒ 红（缺口要么补上、要么重新立项）', () => {
  const r = run(
    ledger({ rows: CLEAN_ROWS, allows: ['mechanism-allow I2: T-186 已经做完的期限'] }),
  )
  assert.ok(r.errors.some((e) => e.code === 'ME-004' && /已闭环/.test(e.message)))
})

test('ME-004：豁免指向不存在的条目 ⇒ 红', () => {
  const r = run(ledger({ rows: CLEAN_ROWS, allows: ['mechanism-allow I2: T-999 无此条'] }))
  assert.ok(r.errors.some((e) => e.code === 'ME-004' && /不是已登记条目/.test(e.message)))
})

test('ME-005：豁免超过上限 ⇒ 红', () => {
  const rows = []
  const allows = []
  for (let i = 0; i <= MAX_EXEMPTIONS; i++) {
    rows.push(`\`m${i}\` | ③ | — | x`)
    allows.push(`mechanism-allow m${i}: T-188 理由`)
  }
  const r = audit({
    text: ledger({ rows, allows }),
    mechanisms: mechanismsOf(ledger({ rows, allows })),
    codes: new Set(),
    resolveAnchor: () => true,
    entries: new Map([['T-188', 'todo']]),
  })
  assert.ok(r.errors.some((e) => e.code === 'ME-005'))
})
