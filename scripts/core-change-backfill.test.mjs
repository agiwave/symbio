import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { validateBackfill } from './core-change-backfill.mjs'

/** 最小现行文档：只提供被判据打开的决策号。 */
function rootWith(decisions) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-backfill-'))
  const dir = path.join(root, 'docs', '003-design')
  fs.mkdirSync(dir, { recursive: true })
  fs.writeFileSync(
    path.join(dir, '999-fixture.md'),
    decisions.map((d) => `### ${d} 夹具决策\n\n正文。\n`).join('\n'),
  )
  return root
}

function table(entry = {}) {
  return {
    rule: 'D-333',
    authorized_by: 'D-333',
    entries: [
      {
        sha: '60cedd50',
        rules: ['CC-001'],
        change: '环侧旧编排整族退役',
        decision: 'D-318',
        authorized_by: 'D-333',
        ...entry,
      },
    ],
  }
}

const ROOT = rootWith(['D-318', 'D-333'])

test('形状齐全的补记行成立', () => {
  assert.deepEqual(validateBackfill(table(), ROOT), [])
})

test('缺 change 的补记等于永久后门', () => {
  assert.match(validateBackfill(table({ change: '' }), ROOT).join('\n'), /缺 `change`/)
})

test('打不开的决策号等于没写', () => {
  assert.match(validateBackfill(table({ decision: 'D-999' }), ROOT).join('\n'), /D-999.*打不开/)
})

test('表外的判据名不许出现', () => {
  assert.match(validateBackfill(table({ rules: ['R-001'] }), ROOT).join('\n'), /只允许 CC-001 \/ EX-007/)
})

test('行的 authorized_by 必须与表级同值（追加只能同一条决策）', () => {
  const errors = validateBackfill(table({ authorized_by: 'D-318' }), ROOT).join('\n')
  assert.match(errors, /与表级 D-333 不一致/)
})

test('同一提交不能有两条补记', () => {
  const t = table()
  t.entries.push({ ...t.entries[0], change: '重复' })
  assert.match(validateBackfill(t, ROOT).join('\n'), /提交号重复/)
})
