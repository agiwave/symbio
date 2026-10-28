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

/**
 * 夹具阶段的最小任务体。
 *
 * GW-006 判「阶段本轮产出 0 项」，所以凡是被当成**正常阶段**用的夹具都必须至少给一项——
 * 否则每条用例都额外红在 GW-006 上（这不是噪音，正是这条判据的形状：旧的夹具一片
 * `tasks: () => []`，写的人没意识到那就是一个空转阶段）。只有专门测 GW-006 的用例才用空表。
 */
const ONE = '[{ label: "x", run: () => true }]'

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
      `export default { id: "docs", tasks: () => ${ONE} }`,
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
      { name: '10-backend.mjs', body: 'export default { id: "backend", tasks: () => [{ label: "x", run: () => true }] }\n' },
      { name: '30-docs.mjs', body: 'export const GUARDS=["a"];export const TEST_ONLY=[];export default{id:"docs",tasks:()=>[{label:"x",run:()=>true}]}\n' },
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
// 那个词），见 `gate-wiring-audit.mjs` 文件头与 `docs/007-task/251-*.md`。

/** 会重生成的阶段：声明 + 调用形状都在 */
const GEN = (id) => `export default { id: ${JSON.stringify(id)}, regenerates: true, tasks: () => [{ run: (c) => autoWork(c, {}) }] }\n`
/** 只读复核阶段 */
const RECHK = (id) => `export default { id: ${JSON.stringify(id)}, recheck: true, tasks: () => ${ONE} }\n`
/** 什么都不做的阶段 */
const PLAIN = (id) => `export default { id: ${JSON.stringify(id)}, tasks: () => ${ONE} }\n`
const DOCS = `export const GUARDS=[];export const TEST_ONLY=[];export default{id:"docs",tasks:()=>${ONE}}\n`
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
    { name: '06-facts-write.mjs', body: `export default { id: "facts-write", regenerates: true, tasks: () => ${ONE} }\n` },
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

// ── GW-006 · 阶段不空转（本轮产出 0 项就判红）──────────────────────────────
// 治的是「阶段还在、判据没了」：`50-msrv` 写死了 D-191 之前那两个顶层目录，拆成 crate 之后
// 路径不存在 ⇒ 每轮产出 0 项、汇总表上一行都没有，与「全绿」同形，
// 而 CI 的 msrv job 一直绿。所以下面第 1 条就是这个形状的最小复刻。

const EMPTY = (id, extra = '') =>
  `export default { id: ${JSON.stringify(id)}, ${extra}${extra ? ', ' : ''}tasks: () => [] }\n`

test('GW-006 阶段产出 0 项且没声明 ⇒ 红（msrv 空转的最小复刻）', () => {
  const r = run([{ name: '50-msrv.mjs', body: EMPTY('msrv') }])
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-006 scripts\/gate\.d\/50-msrv\.mjs 产出 0 个检查项/)
})

test('GW-006 0 项但声明了一句理由 ⇒ 绿（有名有姓的豁免，放行对照）', () => {
  const r = run([
    {
      name: '58-e2e.mjs',
      body: EMPTY('e2e', 'mayBeEmpty: "本机没有 e2e 用例目录（仓里只有 mock LLM 的那批才算）"'),
    },
  ])
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('GW-006 `mayBeEmpty: true` 不算豁免 ⇒ 红（开关三个月后没人记得为什么允许）', () => {
  const r = run([{ name: '58-e2e.mjs', body: EMPTY('e2e', 'mayBeEmpty: true') }])
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-006 scripts\/gate\.d\/58-e2e\.mjs 声明了 .*mayBeEmpty.*不是一句理由/)
})

test('GW-006 阶段文件没有 default 导出 ⇒ 红（它在阶段目录里，却不是阶段）', () => {
  const r = run([{ name: '40-half-written.mjs', body: '// 忘了写 export default\n' }])
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-006 scripts\/gate\.d\/40-half-written\.mjs 没有 `export default/)
})

test('GW-006 tasks() 物化时抛 ⇒ 报「物化失败」，不伪装成「0 项」', () => {
  // 「读不出」与「确实没有」是两件事：把前者报成后者，等于让一个坏阶段躲进豁免的形状里。
  const r = run([
    {
      name: '45-weird.mjs',
      body: 'export default { id: "weird", tasks: (ctx) => { throw new Error("派生路径不存在") } }\n',
    },
  ])
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /GW-006 scripts\/gate\.d\/45-weird\.mjs 的 tasks\(\) 物化失败/)
  assert.match(r.stdout, /派生路径不存在/)
  assert.doesNotMatch(r.stdout, /45-weird\.mjs 产出 0 个检查项/)
})

test('GW-006 有项就完全不介入：正常阶段不因为「本轮 1 项」被念一句', () => {
  const r = run([{ name: '10-a.mjs', body: PLAIN('a') }])
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.doesNotMatch(r.stdout, /GW-006/)
})
