/**
 * gate-wiring-audit 回归测试
 *
 * 这条守卫判的是「名单与目录事实对不对得上」，所以两个方向的失效都要钉住：
 * 多一份没接线的测试（GW-001）、名单指名不存在的文件（GW-002）、同名两处都列（GW-003），
 * 以及**审计范围读不出必须是失败而不是绿灯**——名单读不出来时脚本本可以「无违规」退出，
 * 而那正是这类守卫最安静的死法。
 *
 * 跑法：node --test scripts/gate-wiring-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./gate-wiring-audit.mjs', import.meta.url))

/** 造一棵最小仓库：`docs30` 是 30-docs 导出的两张名单，`tests` 是磁盘上的测试名 */
function audit ({ guards = [], testOnly = [], tests = [], extra = {} } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'gate-wiring-'))
  try {
    const mod = [
      `export const GUARDS = ${JSON.stringify(guards)}`,
      `export const TEST_ONLY = ${JSON.stringify(testOnly)}`,
      'export default { id: "docs" }',
    ].join('\n')
    const files = {
      'scripts/gate.d/30-docs.mjs': mod + '\n',
      ...Object.fromEntries(tests.map((n) => [`scripts/${n}.test.mjs`, '// test\n'])),
      ...extra,
    }
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.ifError(r.error)
    return r
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

test('接线齐备：名单与目录逐一对应 ⇒ 通过（exit 0）', () => {
  const r = audit({ guards: ['grep-audit'], testOnly: ['md-table'], tests: ['grep-audit', 'md-table'] })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /gate-wiring-audit 通过：磁盘 2 份测试全部接线/)
})

test('GW-001 磁盘多一份没接线的测试 ⇒ 红（exit 1）', () => {
  const r = audit({ guards: ['grep-audit'], testOnly: [], tests: ['grep-audit', 'new-guard'] })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-001 scripts\/new-guard\.test\.mjs/)
})

test('GW-001 反向：名单全空而目录有测试 ⇒ 每一份都红，不是「无违规」', () => {
  const r = audit({ guards: [], testOnly: [], tests: ['a', 'b'] })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-001 scripts\/a\.test\.mjs/)
  assert.match(r.stdout, /GW-001 scripts\/b\.test\.mjs/)
})

test('GW-002 名单指名磁盘上不存在的测试 ⇒ 红（exit 1）', () => {
  const r = audit({ guards: ['gone-audit'], testOnly: [], tests: [] })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-002 .*gone-audit\.test\.mjs/)
})

test('GW-003 同一个名同时在 GUARDS 与 TEST_ONLY ⇒ 红（exit 1）', () => {
  const r = audit({ guards: ['dup-audit'], testOnly: ['dup-audit'], tests: ['dup-audit'] })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-003 `dup-audit`/)
})

test('子目录的测试按相对名对账：gate.d/_shared 接了线就不红', () => {
  const r = audit({
    guards: [],
    testOnly: ['gate.d/_shared'],
    tests: [],
    extra: { 'scripts/gate.d/_shared.test.mjs': '// test\n' },
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('垃圾目录不算测试：node_modules / tmp 下的 .test.mjs 不进对账', () => {
  const r = audit({
    guards: ['a'],
    testOnly: [],
    tests: ['a'],
    extra: { 'scripts/node_modules/pkg/b.test.mjs': '//\n', 'scripts/tmp/c.test.mjs': '//\n' },
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('审计范围读不出：没有 30-docs 名单 ⇒ 失败而不是绿灯（exit 2）', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'gate-wiring-'))
  try {
    fs.mkdirSync(path.join(root, 'scripts'), { recursive: true })
    fs.writeFileSync(path.join(root, 'scripts', 'a.test.mjs'), '//\n')
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.equal(r.status, 2, r.stdout + r.stderr)
    assert.match(r.stdout, /GW-002 审计范围读不出/)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('名单没导出（GUARDS 缺失）同样算读不出，不被当成「零项 ⇒ 零违规」', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'gate-wiring-'))
  try {
    const abs = path.join(root, 'scripts', 'gate.d')
    fs.mkdirSync(abs, { recursive: true })
    fs.writeFileSync(path.join(abs, '30-docs.mjs'), 'export const NOT_THE_LIST = []\n')
    fs.writeFileSync(path.join(root, 'scripts', 'a.test.mjs'), '//\n')
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.equal(r.status, 2, r.stdout + r.stderr)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})
