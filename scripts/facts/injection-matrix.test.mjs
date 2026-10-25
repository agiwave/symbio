/**
 * facts/injection-matrix 回归测试 —— 生成物必须与数据源逐格一致
 *
 * 这份生成器的失效形态是**排版吃掉一格**：表头列数错位让两格并成一列、某类归属
 * （`na` / `gap`）渲染成空白、或行数少于 KINDS。矩阵一旦少一格，读者看到的「全覆盖」
 * 就是排版造出来的。测试直接吃同一份 `render()`：
 *
 *   1. 行数 = `KINDS.length`（每个 kind 一行），列数 = `FACES.length + 1`；
 *   2. 每格非空，且 `na` / `gap` 两种归属都真的渲染出了文字；
 *   3. front-matter 的 `title` 与首个 H1 一致（登记审计 RG-003 的前提）；
 *   4. 同一输入两次渲染逐字相等（render 是纯函数）。
 *
 * 用法：node --test scripts/facts/injection-matrix.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { FACES, KINDS, CELLS } from '../../e2e/injection-matrix.mjs'
import fact, { cellText, render } from './injection-matrix.mjs'

test('render 是纯函数：同一输入两次逐字相等', () => {
  assert.equal(render(), render())
})

test('每个 kind 一行、每行与表头等宽', () => {
  const out = render()
  const section = out.slice(out.indexOf('## 2. 矩阵'))
  const lines = section.split('\n')
  const header = lines.find((l) => l.startsWith('| 注入字段 |'))
  assert.ok(header, '缺矩阵表头')
  const width = header.split('|').length
  const rows = lines.filter((l) => l.startsWith('| **'))
  assert.equal(rows.length, KINDS.length, `矩阵行数应等于 KINDS（${KINDS.length}）`)
  for (const row of rows) assert.equal(row.split('|').length, width, `列数错位：${row.slice(0, 80)}`)
})

test('每一格都渲染出归属，na / gap 不会被排版吃掉', () => {
  const byPair = new Map(CELLS.map((c) => [`${c.kind}|${c.face}`, c]))
  for (const k of KINDS) {
    for (const f of FACES) {
      const text = cellText(byPair.get(`${k.id}|${f.id}`))
      assert.ok(text && text !== '**未归属**', `${k.id} × ${f.id} 没有归属`)
    }
  }
  const out = render()
  assert.ok(out.includes('— 工具轮的 provider 错误'), 'na 格必须渲染理由')
  assert.ok(out.includes('**缺口**：DE-039'), 'gap 格必须渲染 owner')
})

test('front-matter 的 title 与首个 H1 一致', () => {
  const lines = render().split('\n')
  const title = lines.find((l) => l.startsWith('title: '))?.slice('title: '.length)
  const h1 = lines.find((l) => l.startsWith('# '))?.slice(2)
  assert.ok(title)
  assert.equal(title, h1)
})

test('描述符契约：id / out / 不带溯源行', () => {
  assert.equal(fact.id, 'injection-matrix')
  assert.equal(fact.out, 'docs/004-ref/012-注入矩阵.md')
  assert.equal(fact.stamp, false)
  assert.equal(fact.render, render)
})
