import test from 'node:test'
import assert from 'node:assert/strict'
import { validateBackfill } from './core-change-backfill.mjs'

function table(entry = {}) {
  return {
    rule: 'CC-001',
    why: '历史提交无法改写，缺 Core-Change 的事实只能逐笔登记；本表只登记既成事实',
    entries: [
      {
        sha: '60cedd50',
        rules: ['CC-001'],
        change: '环侧旧编排整族退役',
        why: '判据生效前已推送，补记是为了让账本可逐笔复核',
        ...entry,
      },
    ],
  }
}

test('形状齐全的补记行成立', () => {
  assert.deepEqual(validateBackfill(table()), [])
})

test('缺 change 的补记等于永久后门', () => {
  assert.match(validateBackfill(table({ change: '' })).join('\n'), /缺 `change`/)
})

test('缺当前理由 why ⇒ 红（不能只挂一个旧编号）', () => {
  assert.match(validateBackfill(table({ why: '' })).join('\n'), /缺当前理由 why/)
})

test('表外的判据名不许出现', () => {
  assert.match(validateBackfill(table({ rules: ['R-001'] })).join('\n'), /只允许 CC-001 \/ EX-007/)
})

test('表级 rule 必须是判据码', () => {
  const t = table()
  t.rule = ''
  assert.match(validateBackfill(t).join('\n'), /rule 必须是/)
})

test('同一提交不能有两条补记', () => {
  const t = table()
  t.entries.push({ ...t.entries[0], change: '重复', why: '另一笔重复登记的当前理由' })
  assert.match(validateBackfill(t).join('\n'), /提交号重复/)
})
