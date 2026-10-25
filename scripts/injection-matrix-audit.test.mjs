/**
 * injection-matrix-audit 回归测试 —— 证明它**会红**，且**不误报**
 *
 * 这个守卫的失效形态全是恒真：
 * ① 用例号不解析到文件 ⇒ 死链也算锚；
 * ② 用例文件存在但没用该字段 ⇒ 假锚；
 * ③ 夹具加字段 / 用例但矩阵没改 ⇒ 反向漏；
 * ④ `na` 随便写一句、`gap` 的 owner 不存在 ⇒「每格有归属」变成一句空话。
 *
 * 覆盖：
 *   1. 完整矩阵 ⇒ 绿；
 *   2. 缺一格 ⇒ INJ-001；
 *   3. 假锚（用例没用字段）⇒ INJ-002；
 *   4. 反向漏（用例用了字段但没人认领）⇒ INJ-003；注释里的字段不算使用；
 *   5. `na` 太短 / `gap` owner 不存在 ⇒ INJ-002；owner 存在 ⇒ 绿；
 *   6. 夹具里找不到字段 / 用例目录读不出 ⇒ INJ-003。
 *
 * 用法：node --test scripts/injection-matrix-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { validate, stripComments } from './injection-matrix-audit.mjs'

const script = fileURLToPath(new URL('./injection-matrix-audit.mjs', import.meta.url))

function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

const KIND = {
  id: 'x.alpha',
  label: '夹具注入',
  mock: 'e2e/mock-x.mjs',
  field: 'alpha',
  inMock: /\bscenario\.alpha\b/,
  usage: /\balpha\s*:\s*\S/,
}
const FACE = { id: 'dialog', label: '对话' }
const MOCK = 'const x = scenario.alpha\n'
const CASE = 'const plan = { alpha: 1 }\n'

function repo(files = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'injection-matrix-'))
  const base = {
    'e2e/mock-x.mjs': MOCK,
    'e2e/cases/t1-alpha.mjs': CASE,
    'docs/007-task/999-fake.md': '---\nkind: task\n---\n',
    ...files,
  }
  for (const [rel, content] of Object.entries(base)) {
    if (content === null) continue
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
  return root
}

function run(files, cells) {
  const root = repo(files)
  try {
    return validate({ kinds: [KIND], faces: [FACE], cells }, root)
  } finally {
    rmTree(root)
  }
}

const OK = [{ kind: KIND.id, face: 'dialog', cases: ['t1'] }]

test('完整矩阵 ⇒ 没有违规', () => {
  const r = run({}, OK)
  assert.deepEqual(r.errors, [])
  assert.equal(r.unassigned, 0)
  assert.equal(r.deferred, 0)
})

test('缺一格 ⇒ INJ-001', () => {
  const r = run({}, [])
  assert.equal(r.unassigned, 1)
  assert.ok(r.errors.some((e) => e.rule === 'INJ-001' && /没有归属/.test(e.msg)))
})

test('假锚：用例文件存在但代码没用该字段 ⇒ INJ-002', () => {
  const r = run({ 'e2e/cases/t1-alpha.mjs': 'const x = 1\n' }, OK)
  assert.ok(r.errors.some((e) => e.rule === 'INJ-002' && /假锚/.test(e.msg)))
})

test('反向漏：用例用了字段但没被认领 ⇒ INJ-003', () => {
  const r = run({ 'e2e/cases/t2-other.mjs': CASE }, OK)
  assert.ok(r.errors.some((e) => e.rule === 'INJ-003' && /t2-other/.test(e.msg)))
})

test('注释里的字段不算使用（反向扫描与锚判定都去注释）', () => {
  assert.equal(stripComments('// alpha: 1\nconst x = 1\n').includes('alpha'), false)
  const r = run({ 'e2e/cases/t2-other.mjs': '// alpha: 1\n' }, OK)
  assert.deepEqual(r.errors, [])
})

test('na 太短 ⇒ INJ-002；写清理由 ⇒ 绿', () => {
  const bad = run({ 'e2e/cases/t1-alpha.mjs': null, 'e2e/cases/t9-empty.mjs': '' }, [{ kind: KIND.id, face: 'dialog', na: '无' }])
  assert.ok(bad.errors.some((e) => e.rule === 'INJ-002' && /理由过短/.test(e.msg)))
  const ok = run({ 'e2e/cases/t1-alpha.mjs': null, 'e2e/cases/t9-empty.mjs': '' }, [{ kind: KIND.id, face: 'dialog', na: '这条面没有独立观测点，由另一条面覆盖。' }])
  assert.deepEqual(ok.errors, [])
})

test('gap owner 不存在 ⇒ INJ-002；指向存在的任务 ⇒ 绿', () => {
  const bad = run({ 'e2e/cases/t1-alpha.mjs': null, 'e2e/cases/t9-empty.mjs': '' }, [{ kind: KIND.id, face: 'dialog', gap: { owner: 'T-404', reason: '待接线批次补用例。' } }])
  assert.ok(bad.errors.some((e) => e.rule === 'INJ-002' && /owner/.test(e.msg)))
  const ok = run({ 'e2e/cases/t1-alpha.mjs': null, 'e2e/cases/t9-empty.mjs': '' }, [{ kind: KIND.id, face: 'dialog', gap: { owner: 'T-999', reason: '待接线批次补用例。' } }])
  assert.deepEqual(ok.errors, [])
})

test('夹具里找不到字段 ⇒ INJ-003（矩阵在描述不存在的注入）', () => {
  const r = run({ 'e2e/mock-x.mjs': 'const x = 1\n' }, OK)
  assert.ok(r.errors.some((e) => e.rule === 'INJ-003' && /找不到字段/.test(e.msg)))
})

test('用例目录读不出 ⇒ INJ-003，不是静默通过', () => {
  const root = repo()
  try {
    fs.rmSync(path.join(root, 'e2e/cases'), { recursive: true, force: true })
    const r = validate({ kinds: [KIND], faces: [FACE], cells: OK }, root)
    assert.ok(r.errors.some((e) => e.rule === 'INJ-003' && /读不出 e2e\/cases/.test(e.msg)))
  } finally {
    rmTree(root)
  }
})

test('真实仓库：数据源与用例对得上 ⇒ 退出 0', () => {
  const r = spawnSync(process.execPath, [script], {
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 30_000,
  })
  assert.ifError(r.error)
  assert.equal(r.status, 0, r.stdout + r.stderr)
})
