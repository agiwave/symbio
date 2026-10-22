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

// ── GW-005 · 阶段顺序（读自声明，不读注释措辞）─────────────────────────
// 判据是「改写工作树的工作做完之后，有人只读地对一次末态」：`regenerates` 阶段必须早于
// 每一个 `recheck` 阶段，且声明与源码里的 `autoWork(` / `autoChain(` 调用形状双向对上。
// 旧的「`includes('autoWork')` 的阶段必须排末尾」已被实测否证（判定的真假取决于注释里有没有
// 那个词），见 `gate-wiring-audit.mjs` 文件头与 `docs/009-task/251-*.md`。

/** 会重生成的阶段：声明 + 调用形状都在 */
const GEN = (id) => `export default { id: ${JSON.stringify(id)}, regenerates: true, tasks: () => [{ run: (c) => autoWork(c, {}) }] }\n`
/** 只读复核阶段 */
const RECHK = (id) => `export default { id: ${JSON.stringify(id)}, recheck: true, tasks: () => [] }\n`
/** 什么都不做的阶段 */
const PLAIN = (id) => `export default { id: ${JSON.stringify(id)}, tasks: () => [] }\n`
const DOCS = 'export const GUARDS=[];export const TEST_ONLY=[];export default{id:"docs",tasks:()=>[]}\n'
const run = (stages) => audit({ guards: [], testOnly: [], tests: [], stages: [{ name: '30-docs.mjs', body: DOCS }, ...stages] })

test('GW-005 真实形状：生成在前、只读复核在后 ⇒ 绿（放行对照）', () => {
  const r = run([
    { name: '05-fmt.mjs', body: GEN('fmt') },
    { name: '06-facts-write.mjs', body: GEN('facts-write') },
    { name: '55-verify.mjs', body: PLAIN('verify') },
    { name: '60-facts.mjs', body: RECHK('facts') },
  ])
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /生成阶段 06-facts-write\.mjs 先于只读复核 60-facts\.mjs/)
})

test('GW-005 只读复核排在生成阶段之前 ⇒ 红（它复核的是上一版投影）', () => {
  const r = run([
    { name: '50-facts.mjs', body: RECHK('facts') },
    { name: '60-facts-write.mjs', body: GEN('facts-write') },
  ])
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-005 scripts\/gate\.d\/50-facts\.mjs/)
})

test('GW-005 声明 regenerates 却没有调用形状 ⇒ 红（声明是假的）', () => {
  const r = run([
    { name: '06-facts-write.mjs', body: `export default { id: "facts-write", regenerates: true, tasks: () => [] }\n` },
    { name: '60-facts.mjs', body: RECHK('facts') },
  ])
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-005 scripts\/gate\.d\/06-facts-write\.mjs 声明 .*regenerates.*却没有/)
})

test('GW-005 调用了原语却不声明 ⇒ 红（判据看不见这个生成阶段）', () => {
  const r = run([
    { name: '06-facts-write.mjs', body: `export default { id: "facts-write", tasks: () => [{ run: (c) => autoChain(c, {}) }] }\n` },
    { name: '60-facts.mjs', body: RECHK('facts') },
  ])
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-005 scripts\/gate\.d\/06-facts-write\.mjs 调用/)
})

test('GW-005 有生成却没有只读复核 ⇒ 红（没人对整轮末态）', () => {
  const r = run([{ name: '06-facts-write.mjs', body: GEN('facts-write') }, { name: '55-verify.mjs', body: PLAIN('verify') }])
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-005 1 个阶段声明 .*regenerates.*，却没有一个阶段声明 .*recheck/)
})

test('GW-005 注释里出现 `autoWork` 一词不影响判定 ⇒ 绿（旧写法的假阳性来源）', () => {
  // 这一条就是反向判据的正面形式：旧版把这段注释删掉会报 5 条红，新版对措辞无感。
  const r = run([
    { name: '06-facts-write.mjs', body: GEN('facts-write') },
    { name: '30-docs.mjs', body: DOCS },
    { name: '60-facts.mjs', body: `// 生成链按 \`autoWork\` 逐格判差异这一段历史叙述里提到 autoWork\n${RECHK('facts')}` },
  ])
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('GW-005 同时声明 regenerates 与 recheck ⇒ 红（两条判据合起来什么都判不了）', () => {
  const r = run([
    { name: '06-facts-write.mjs', body: GEN('facts-write') },
    { name: '60-facts.mjs', body: `export default { id: "facts", regenerates: true, recheck: true, tasks: () => [{ run: (c) => autoWork(c, {}) }] }\n` },
  ])
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-005 scripts\/gate\.d\/60-facts\.mjs 同时声明/)
})

test('GW-005 阶段文件读不进（语法错）⇒ 红，不再静默按「没声明」处理', () => {
  const r = run([
    { name: '06-facts-write.mjs', body: GEN('facts-write') },
    { name: '60-facts.mjs', body: 'export default { id: "facts", recheck: true,\n' },
  ])
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-005 scripts\/gate\.d\/60-facts\.mjs 导入失败/)
})

test('GW-005 没有生成阶段也没有复核 ⇒ 不判（空表不制造义务）', () => {
  const r = run([{ name: '10-a.mjs', body: PLAIN('a') }, { name: '20-b.mjs', body: PLAIN('b') }])
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /生成阶段 （无）/)
})
