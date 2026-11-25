import assert from 'node:assert/strict'
import test from 'node:test'
import { collectDeclarations } from './core-change-declarations.mjs'

const DECISIONS = new Set(['D-551'])
const commits = [
  { sha: 'aaaaaaaa1111', message: 'feat: 缺签名的前一笔\n' },
  {
    sha: 'bbbbbbbb2222',
    message: 'docs: 后继补签\n\nCore-Change-For: aaaaaaaa D-551 共享类型必须落在 SDK\n',
  },
]

test('合法的后继补签被收集', () => {
  const { declarations, errors } = collectDeclarations(commits, DECISIONS)
  assert.deepEqual(errors, [])
  assert.equal(declarations.get('aaaaaaaa').decision, 'D-551')
  assert.equal(declarations.get('aaaaaaaa').by, 'bbbbbbbb')
})

test('目标不在本批 ⇒ 红', () => {
  const { errors } = collectDeclarations(
    [
      { sha: 'bbbbbbbb2222', message: 'Core-Change-For: cccccccc D-551 理由\n' },
    ],
    DECISIONS,
  )
  assert.match(errors.join('\n'), /不在本批提交里/)
})

test('不能给自己补签 ⇒ 红', () => {
  const { errors } = collectDeclarations(
    [{ sha: 'aaaaaaaa1111', message: 'Core-Change-For: aaaaaaaa D-551 理由\n' }],
    DECISIONS,
  )
  assert.match(errors.join('\n'), /不能给自己补签/)
})

test('签名必须晚于目标 ⇒ 红', () => {
  const { errors } = collectDeclarations(
    [
      { sha: 'aaaaaaaa1111', message: 'Core-Change-For: bbbbbbbb D-551 理由\n' },
      { sha: 'bbbbbbbb2222', message: '后继\n' },
    ],
    DECISIONS,
  )
  assert.match(errors.join('\n'), /严格早于签名提交/)
})

test('打不开的决策号 ⇒ 红', () => {
  const { errors } = collectDeclarations(
    [
      { sha: 'aaaaaaaa1111', message: '前一笔\n' },
      { sha: 'bbbbbbbb2222', message: 'Core-Change-For: aaaaaaaa D-999 理由\n' },
    ],
    new Set(['D-551']),
    process.cwd(),
  )
  assert.match(errors.join('\n'), /打不开/)
})

test('同一目标重复补签 ⇒ 红', () => {
  const { errors } = collectDeclarations(
    [
      { sha: 'aaaaaaaa1111', message: '前一笔\n' },
      { sha: 'bbbbbbbb2222', message: 'Core-Change-For: aaaaaaaa D-551 理由\n' },
      { sha: 'cccccccc3333', message: 'Core-Change-For: aaaaaaaa D-551 理由\n' },
    ],
    DECISIONS,
  )
  assert.match(errors.join('\n'), /一笔只能签一次/)
})

test('中文冒号与多行正文里的签名同样识别', () => {
  const { declarations, errors } = collectDeclarations(
    [
      { sha: 'aaaaaaaa1111', message: '前一笔\n' },
      { sha: 'bbbbbbbb2222', message: '标题\n\n正文\n\nCore-Change-For：aaaaaaaa D-551 共享类型\n' },
    ],
    DECISIONS,
  )
  assert.deepEqual(errors, [])
  assert.equal(declarations.get('aaaaaaaa').reason, '共享类型')
})
