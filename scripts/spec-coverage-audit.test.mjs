/**
 * spec-coverage-audit 回归测试 —— 证明它**会红**，且**不误报**
 *
 * 这个守卫的失效形态全是恒真：
 * ① 脚本指认不检查文件存在 ⇒ 死链也算执行方；
 * ② 空理由被接受 ⇒「无机器判据」退化成「不想写」；
 * ③ 范围读空 ⇒ 「33 条全有判据」在 0 条输入上成立。
 *
 * 覆盖：
 *   1. 脚本存在 ⇒ machine；已登记判据码 ⇒ machine；两者都没有 ⇒ missing；
 *   2. `无机器判据：<理由>` ⇒ manual；空 / 纯标点 ⇒ SPC-002 红；
 *   3. 没有 `## 判据` 小节 ⇒ SPC-001 红；
 *   4. 活跃规范为 0 / 编号重复 ⇒ SPC-003 红；
 *   5. `manual` 存量超过基线 ⇒ SPC-001 红（棘轮）。
 *
 * 用法：node --test scripts/spec-coverage-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { classify } from './spec-coverage-audit.mjs'

const script = fileURLToPath(new URL('./spec-coverage-audit.mjs', import.meta.url))

function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

function spec(id, judge, { status = 'active' } = {}) {
  return [
    '---',
    'kind: spec',
    `id: ${id}`,
    `title: ${id}`,
    `status: ${status}`,
    'created: 2026-10-25',
    '---',
    '',
    '# 夹具规范',
    '',
    '## 判据',
    '',
    judge,
    '',
  ].join('\n')
}

function audit(files, { baseline = '0:0' } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'spec-coverage-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1', SPEC_COVERAGE_BASELINE: baseline },
      encoding: 'utf8',
      timeout: 30_000,
    })
    assert.ifError(r.error)
    return r
  } finally {
    rmTree(root)
  }
}

const SCRIPT = 'scripts/foo.mjs'
const DECL = '// @ns XX 夹具\n// @codes XX-001\nexport const x = 1\n'

test('classify：存在的脚本 / 已登记判据码 ⇒ machine；都没有 ⇒ missing', () => {
  const exists = (rel) => rel === SCRIPT
  const codes = new Set(['XX-001'])
  assert.equal(classify('执行方是 `scripts/foo.mjs`。', { exists, codes }).kind, 'machine')
  assert.equal(classify('执行方是 `XX-001`。', { exists, codes }).kind, 'machine')
  assert.equal(classify('执行方是 `scripts/nope.mjs`。', { exists, codes }).kind, 'missing')
  assert.equal(classify('执行方是 `YY-999`。', { exists, codes }).kind, 'missing')
})

test('classify：无机器判据带理由 ⇒ manual；空理由 ⇒ invalidReason', () => {
  const ok = classify('无机器判据：这条判据依赖变更意图，不是代码形状。', { exists: () => false, codes: new Set() })
  assert.equal(ok.kind, 'manual')
  assert.equal(ok.validReason, true)
  const bad = classify('无机器判据：——', { exists: () => false, codes: new Set() })
  assert.equal(bad.kind, 'manual')
  assert.equal(bad.validReason, false)
})

test('脚本存在 ⇒ 通过', () => {
  const r = audit({
    'docs/005-spec/001.md': spec('SP-001', '执行方是 `scripts/foo.mjs`。'),
    [SCRIPT]: DECL,
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('人工判据在基线内 ⇒ 通过；超过基线 ⇒ SPC-001 红', () => {
  const files = {
    'docs/005-spec/001.md': spec('SP-001', '无机器判据：这条判据依赖变更意图，不是代码形状。'),
  }
  const ok = audit(files, { baseline: '1:0' })
  assert.equal(ok.status, 0, ok.stdout + ok.stderr)
  const bad = audit(files, { baseline: '0:0' })
  assert.equal(bad.status, 1, bad.stdout)
  assert.match(bad.stderr, /人工判据存量 1 > 基线 0/)
})

test('空理由 ⇒ SPC-002 红（空豁免等于没有判据）', () => {
  const r = audit({
    'docs/005-spec/001.md': spec('SP-001', '无机器判据：——'),
  }, { baseline: '1:0' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stderr, /SPC-002/)
})

test('没有 `## 判据` 小节 ⇒ SPC-001 红', () => {
  const r = audit({
    'docs/005-spec/001.md': '---\nkind: spec\nid: SP-001\ntitle: SP-001\nstatus: active\ncreated: 2026-10-25\n---\n\n# x\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stderr, /没有 .## 判据. 小节/)
})

test('一条活跃规范都没有 ⇒ SPC-003 红（范围读空不是全覆盖）', () => {
  const r = audit({ 'docs/005-spec/001.md': spec('SP-001', 'x', { status: 'done' }) })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stderr, /一条活跃规范都没读到/)
})

test('编号重复 ⇒ SPC-003 红', () => {
  const r = audit({
    'docs/005-spec/001.md': spec('SP-001', '执行方是 `scripts/foo.mjs`。'),
    'docs/005-spec/002.md': spec('SP-001', '执行方是 `scripts/foo.mjs`。'),
    [SCRIPT]: DECL,
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stderr, /编号 SP-001 重复/)
})
