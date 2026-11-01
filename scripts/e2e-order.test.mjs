// 起跑顺序的回归测试：钉住三件事——降序、未知按中位数、`index` 仍是原序号。
import test from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { orderForStart, loadTimes, recordTimes, suggestConcurrency, TIMES_FILE } from './e2e-order.mjs'

const mk = (...names) => names.map((name) => ({ name, file: `/x/${name}.mjs` }))

test('★ 慢例先跑：起跑顺序按上一轮耗时降序', () => {
  const cases = mk('a', 'b', 'c')
  const order = orderForStart(cases, { a: 100, b: 30_000, c: 900 })
  assert.deepEqual(
    order.map((o) => o.case.name),
    ['b', 'c', 'a'],
  )
})

test('★ index 是**原序号**，不是排期位置：端口分段跟着用例走，不跟着顺序走', () => {
  // 端口基址 = 18080 + index*100，换顺序绝不能换号段——否则「同一个用例换个时间跑
  // 就换了端口」，并发时分段的意义（确定性）就没了。
  const cases = mk('a', 'b', 'c')
  const order = orderForStart(cases, { a: 100, b: 30_000, c: 900 })
  assert.deepEqual(
    order.map((o) => [o.case.name, o.index]),
    [
      ['b', 1],
      ['c', 2],
      ['a', 0],
    ],
  )
})

test('没测过的用例按**已知耗时的中位数**排，不排到最后', () => {
  // 排到最后 = 新用例若是慢例，正好落进最糟的位置（拖尾）。中位数是更安全的猜测。
  const cases = mk('fast', 'slow', 'unknown')
  const order = orderForStart(cases, { fast: 500, slow: 20_000 })
  // 中位数 = (500+20000)/2 取整 = 10250 ⇒ 排在 500 之前、20000 之后
  assert.deepEqual(
    order.map((o) => o.case.name),
    ['slow', 'unknown', 'fast'],
  )
})

test('一条耗时都没有 ⇒ 退化成文件名序（与改动前行为一致）', () => {
  const cases = mk('t1-a', 't2-b', 't3-c')
  assert.deepEqual(
    orderForStart(cases, {}).map((o) => o.case.name),
    ['t1-a', 't2-b', 't3-c'],
  )
  assert.deepEqual(
    orderForStart(cases).map((o) => o.case.name),
    ['t1-a', 't2-b', 't3-c'],
  )
})

test('耗时相同的用例保持文件名序（排序必须稳定，否则每轮顺序都在抖）', () => {
  const cases = mk('t1', 't2', 't3', 't4')
  assert.deepEqual(
    orderForStart(cases, { t1: 700, t2: 700, t3: 700, t4: 700 }).map((o) => o.case.name),
    ['t1', 't2', 't3', 't4'],
  )
})

test('脏数据当「没测过」：负数 / 零 / 字符串 / NaN 都不参与排期', () => {
  const cases = mk('a', 'b', 'c')
  const order = orderForStart(cases, { a: -5, b: '3000', c: Number.NaN })
  // 三条都不可用 ⇒ 等权 ⇒ 原名序
  assert.deepEqual(
    order.map((o) => o.case.name),
    ['a', 'b', 'c'],
  )
})

test('recordTimes / loadTimes 往返：只留正有限数，写读一致', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'e2e-order-'))
  try {
    recordTimes(root, { a: 1234.6, b: 0, c: -1, d: Number.NaN, e: 7 })
    assert.deepEqual(loadTimes(root), { a: 1235, e: 7 })
    assert.ok(fs.existsSync(path.join(root, TIMES_FILE)), '文件应落在约定的位置')
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('缓存缺失或损坏 ⇒ loadTimes 返回空表，不抛（排期坏了不该弄红门禁）', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'e2e-order-'))
  try {
    assert.deepEqual(loadTimes(root), {}, '文件不存在')
    fs.mkdirSync(path.join(root, '.workbuddy-ai'), { recursive: true })
    fs.writeFileSync(path.join(root, TIMES_FILE), '{ 这不是 JSON')
    assert.deepEqual(loadTimes(root), {}, '文件名对但内容坏')
    fs.writeFileSync(path.join(root, TIMES_FILE), '[1,2,3]')
    assert.deepEqual(loadTimes(root), {}, 'JSON 是数组不是表')
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('★ 并发度 = min(机器槽位, ⌈总工作量/最慢单例⌉)：慢例被削掉时它自己上调', () => {
  // 15 例各 20s：总 300s，最慢 20s ⇒ 理论最优 15，被机器槽位 12 压住
  const flat = mk(...Array.from({ length: 15 }, (_, i) => `t${i}`))
  const flatTimes = Object.fromEntries(flat.map((c) => [c.name, 20_000]))
  assert.equal(suggestConcurrency({ cases: flat, times: flatTimes, machineSlots: 12 }), 12)
  // 尾部长例回归（一例 150s）⇒ 自动收缩到 2，不必有人记得回来改常量
  const skew = mk('a', 'b', 'c')
  assert.equal(suggestConcurrency({ cases: skew, times: { a: 150_000, b: 100_000, c: 50_000 }, machineSlots: 12 }), 2)
  // 槽位小（4 核机）⇒ 听机器的
  assert.equal(suggestConcurrency({ cases: flat, times: flatTimes, machineSlots: 3 }), 3)
})

test('没有耗时数据时并发度退回 fallback，且不越过机器槽位', () => {
  const cases = mk('a', 'b')
  assert.equal(suggestConcurrency({ cases, times: {}, machineSlots: 12 }), 6, '默认 6')
  assert.equal(suggestConcurrency({ cases, times: {}, machineSlots: 2 }), 2, '小机器听机器')
  assert.equal(suggestConcurrency({ cases, times: { a: 0, b: -1 }, machineSlots: 12 }), 6, '脏数据等于没数据')
})
