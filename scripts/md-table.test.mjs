import { test } from 'node:test'
import assert from 'node:assert/strict'

import { cellsOf, isSeparator, plain, backticked, tableWithHeader, fencedBlock } from './md-table.mjs'

test('cellsOf 只认表行，并把单元格切出来', () => {
  assert.deepEqual(cellsOf('| a | **b** |'), ['a', '**b**'])
  assert.deepEqual(cellsOf('   | a |'), ['a'])
  assert.equal(cellsOf('正文一行'), null)
  assert.equal(cellsOf(''), null)
})

test('isSeparator 认分隔行，含左/右/两端对齐三种', () => {
  assert.ok(isSeparator(['---', ':--', '--:']))
  assert.ok(!isSeparator(['参数键', '取值域']))
})

test('plain 去粗与反引号，只留纯文本', () => {
  assert.equal(plain('**`store`**'), 'store')
  assert.equal(plain('`recall` / `snapshot`'), 'recall / snapshot')
})

test('backticked 取第一个反引号片段，没有则 null', () => {
  assert.equal(backticked('新增 `task/opened` 各格'), 'task/opened')
  assert.equal(backticked('无标记'), null)
})

test('tableWithHeader 按表头首列定位，取到紧邻的连续表', () => {
  const md = [
    '| 甲 | 乙 |',
    '|---|---|',
    '| 1 | 2 |',
    '',
    '| 参数键 | 取值域 |',
    '|---|---|',
    '| `store` | `memory` |',
    '| `projection` | `snapshot` |',
    '',
    '正文',
  ].join('\n')
  const t = tableWithHeader(md, '参数键')
  assert.deepEqual(t.header, ['参数键', '取值域'])
  assert.deepEqual(t.rows, [['`store`', '`memory`'], ['`projection`', '`snapshot`']])
})

test('tableWithHeader 找不到的表返回 null——由调用方变红，不静默给空表', () => {
  assert.equal(tableWithHeader('| 别的 | 表 |', '参数键'), null)
  assert.equal(tableWithHeader('一张表都没有', '参数键'), null)
})

test('fromLine 把定位限定到某节之后（同名表头在长文档里会重复出现）', () => {
  const md = ['| 参数 | 今天 |', '|---|---|', '| a | b |', '', '## 4.', '', '| 参数 | 平凡值 |', '|---|---|', '| p | q |'].join('\n')
  assert.deepEqual(tableWithHeader(md, '参数').rows, [['a', 'b']])
  const from4 = tableWithHeader(md, '参数', { fromLine: md.split('\n').findIndex((l) => l === '## 4.') })
  assert.deepEqual(from4.header, ['参数', '平凡值'])
  assert.deepEqual(from4.rows, [['p', 'q']])
})

test('fencedBlock 按 info string 取块内容，注释行一并带回由调用方处置', () => {
  const md = ['前文', '```capability-assign', '# 注释', 'store = wal', '', 'projection = recall', '```', '后文'].join('\n')
  assert.deepEqual(fencedBlock(md, 'capability-assign'), ['# 注释', 'store = wal', '', 'projection = recall'])
})

test('fencedBlock 块不存在或块为空都返回 null（缺失不等于「零条数据」）', () => {
  assert.equal(fencedBlock('```other\nx\n```', 'capability-assign'), null)
  assert.equal(fencedBlock('```capability-assign\n```', 'capability-assign'), null)
})
