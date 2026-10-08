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

/** 造一棵最小仓库：`docs30` 是 30-docs 导出的名单，`tests` 是磁盘上的测试名 */
function audit ({
  guards = [],
  testOnly = [],
  tests = [],
  libs = undefined,
  outOfGate = undefined,
  stages = undefined,
  extra = {},
} = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'gate-wiring-'))
  try {
    // `libs` / `outOfGate` 默认**不导出**：GW-004 会把它们报成未登记，
    // 而这正是「忘了导出」不能蒙混过关的形状（判据注释里写明了这一点）。
    const mod = [
      `export const GUARDS = ${JSON.stringify(guards)}`,
      `export const TEST_ONLY = ${JSON.stringify(testOnly)}`,
      ...(libs ? [`export const LIBS = ${JSON.stringify(libs)}`] : []),
      ...(outOfGate ? [`export const OUT_OF_GATE = ${JSON.stringify(outOfGate)}`] : []),
      'export default { id: "docs" }',
    ].join('\n')
    const stageFiles = stages ?? [{ name: '30-docs.mjs', body: '// stage\n' }]
    const files = {
      ...Object.fromEntries(stageFiles.map((s) => [`scripts/gate.d/${s.name}`, s.body])),
      ...(stages === undefined ? { 'scripts/gate.d/30-docs.mjs': mod + '\n' } : {}),
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

// ── GW-004 · 命名空间闭合 ─────────────────────────────────────────────
// GW-001 扫的是 `.test.mjs`，于是「既没有回归测试、又不在任何名单里」的 `.mjs`
// 是隐形的。这组用例钉的是它的**两个方向**：漏登记要红、登记了要绿。

test('GW-004 目录里有既无测试又未登记的 .mjs ⇒ 红（exit 1）', () => {
  const r = audit({
    guards: ['a'],
    testOnly: [],
    tests: ['a'],
    extra: { 'scripts/orphan-lib.mjs': '// 一个没人登记也没人测的库\n' },
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-004 scripts\/orphan-lib\.mjs/)
})

test('GW-004 登记进 LIBS 之后 ⇒ 不红（共享库放行对照）', () => {
  const r = audit({
    guards: ['a'],
    testOnly: [],
    libs: ['orphan-lib'],
    tests: ['a'],
    extra: { 'scripts/orphan-lib.mjs': '// 一个被登记的库\n' },
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /通过/)
})

test('GW-004 登记进 OUT_OF_GATE ⇒ 不红（不进门的辅助脚本放行对照）', () => {
  const r = audit({
    guards: ['a'],
    testOnly: [],
    outOfGate: ['side-tool'],
    tests: ['a'],
    extra: { 'scripts/side-tool.mjs': '// git hook 用\n' },
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('GW-004 阶段文件（gate.d/ 非 _ 前缀）按扫描接入，不需要登记', () => {
  const r = audit({
    guards: ['a'],
    testOnly: [],
    tests: ['a'],
    stages: [
      { name: '10-backend.mjs', body: '// stage\n' },
      { name: '30-docs.mjs', body: 'export const GUARDS=["a"];export const TEST_ONLY=[];export default{id:"docs"}\n' },
    ],
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('GW-004 忘了导出 LIBS ⇒ 那份库被报成未登记（不能靠「忘了导出」蒙混）', () => {
  const r = audit({
    guards: ['a'],
    testOnly: [],
    tests: ['a'],
    extra: { 'scripts/lib.mjs': '// 存在但 LIBS 没导出\n' },
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-004 scripts\/lib\.mjs/)
})

test('GW-002 扩面：LIBS 指名磁盘上没有的 .mjs ⇒ 红（「已覆盖」是假的）', () => {
  const r = audit({ guards: ['a'], testOnly: [], libs: ['ghost-lib'], tests: ['a'] })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-002 .*ghost-lib\.mjs/)
})

// ── GW-005 · 阶段顺序 ────────────────────────────────────────────────
// `autoWork`（重生成并暂存）必须排在阶段末尾，否则暂存发生在被验证的改动之前，
// `git status` 再也看不出前一阶段碰没碰过东西。

test('GW-005 放一个不重生成的阶段在 autoWork 阶段之后 ⇒ 红', () => {
  const r = audit({
    guards: [],
    testOnly: [],
    tests: [],
    stages: [
      { name: '30-docs.mjs', body: 'export const GUARDS=[];export const TEST_ONLY=[];export default{id:"docs"}\n' },
      { name: '60-facts.mjs', body: 'import { autoWork } from "./_shared.mjs"\nexport default { id: "facts" }\n' },
      { name: '65-late.mjs', body: 'export default { id: "late" }\n' },
    ],
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-005 scripts\/gate\.d\/65-late\.mjs/)
})

test('GW-005 autoWork 阶段排在末尾 ⇒ 绿（放行对照）', () => {
  const r = audit({
    guards: [],
    testOnly: [],
    tests: [],
    stages: [
      { name: '30-docs.mjs', body: 'export const GUARDS=[];export const TEST_ONLY=[];export default{id:"docs"}\n' },
      { name: '55-verify.mjs', body: 'import { autoWork } from "./_shared.mjs"\nexport default { id: "verify" }\n' },
      { name: '60-facts.mjs', body: 'import { autoWork } from "./_shared.mjs"\nexport default { id: "facts" }\n' },
    ],
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /重生成阶段 60-facts\.mjs 排在阶段末尾/)
})

test('GW-005 一个 autoWork 阶段都没有时不判（无重生成 ⇒ 无「排在最后」可言）', () => {
  const r = audit({
    guards: [],
    testOnly: [],
    tests: [],
    stages: [
      { name: '10-a.mjs', body: 'export default { id: "a" }\n' },
      { name: '20-b.mjs', body: 'export default { id: "b" }\n' },
      { name: '30-docs.mjs', body: 'export const GUARDS=[];export const TEST_ONLY=[];export default{id:"docs"}\n' },
    ],
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /重生成阶段 （无）/)
})
