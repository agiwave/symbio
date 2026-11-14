/**
 * gate-wiring-audit 回归测试
 *
 * 这条守卫判的是「**声明 ≠ 会被跑**」，所以四个方向都要钉住：
 * 声明缺了（GW-001）、守卫没有回归测试（GW-002）、守卫声明了却不在任务表里
 * （GW-003）、测试文件在而任务表里没有它（GW-004）、名字是 `-audit` 却不在 `scripts/audits/`
 * （GW-007），以及**审计范围读不出必须是失败而不是绿灯**——读不出来时脚本本可以「无违规」
 * 退出，而那正是这类守卫最安静的死法。
 *
 * 夹具里的 `30-docs` 是**真的按声明枚举**的（扫描 `scripts/` 读 `@gate`），所以绿的那几条
 * 走的是和真仓同一条路；专门测「枚举写窄」的两条才换成手写的残废版。
 *
 * 跑法：node --test scripts/audits/gate-wiring-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { withTempTree } from '../fs-walk.mjs'
import { expectAudit } from '../audit-assert.mjs'

const script = fileURLToPath(new URL('./gate-wiring-audit.mjs', import.meta.url))

/**
 * 夹具阶段的最小任务体。
 *
 * GW-006 判「阶段本轮产出 0 项」，所以凡是被当成**正常阶段**用的夹具都必须至少给一项——
 * 否则每条用例都额外红在 GW-006 上（这不是噪音，正是这条判据的形状：旧的夹具一片
 * `tasks: () => []`，写的人没意识到那就是一个空转阶段）。只有专门测 GW-006 的用例才用空表。
 */
const ONE = '[{ label: "x", run: () => true }]'

/**
 * 夹具版 `30-docs`：按 `@gate` 声明枚举任务（与真仓同一形状，只是不认桶以外的细节）。
 * 放在夹具里而不是 import 真文件——夹具要能在临时目录里独立成立。
 */
const DOCS = [
  "import fs from 'node:fs'",
  "import path from 'node:path'",
  "import { fileURLToPath } from 'node:url'",
  "const scripts = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')",
  'const files = []',
  "const walk = (d, p) => { for (const e of fs.readdirSync(d, { withFileTypes: true })) { if (e.isDirectory()) walk(path.join(d, e.name), p + e.name + '/'); else if (e.name.endsWith('.mjs')) files.push(p + e.name) } }",
  "walk(scripts, '')",
  "const decl = (f) => { for (const l of fs.readFileSync(path.join(scripts, f), 'utf8').split(/\\r?\\n/).slice(0, 80)) { const m = /^\\/\\/\\s*@gate\\s+([A-Za-z]+)\\s*$/.exec(l.trim()); if (m) return m[1] } return null }",
  "export default { id: 'docs', *tasks () {",
  '  for (const f of files.sort()) {',
  "    if (f.endsWith('.test.mjs')) { yield { args: ['--test', path.join(scripts, f)] }; continue }",
  "    if (f.startsWith('gate.d/') && !f.slice(7).startsWith('_')) continue",
  "    if (decl(f) === 'guard') yield { args: [path.join(scripts, f), '--strict'] }",
  '  }',
  '} }',
].join('\n') + '\n'

/**
 * 夹具版 `62-weekly`：与 DOCS 同构，只枚举 `weekly` 桶（与真仓同一形状）。
 * 自包含——夹具要在临时目录里独立成立，不能 import 真仓的 `_shared.mjs`。
 */
const WEEKLY = [
  "import fs from 'node:fs'",
  "import path from 'node:path'",
  "import { fileURLToPath } from 'node:url'",
  "const scripts = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')",
  'const files = []',
  "const walk = (d, p) => { for (const e of fs.readdirSync(d, { withFileTypes: true })) { if (e.isDirectory()) { if (e.name !== 'gate.d') walk(path.join(d, e.name), p + e.name + '/') } else if (e.name.endsWith('.mjs') && !e.name.endsWith('.test.mjs')) files.push(p + e.name) } }",
  "walk(scripts, '')",
  "const decl = (f) => { for (const l of fs.readFileSync(path.join(scripts, f), 'utf8').split(/\\r?\\n/).slice(0, 80)) { const m = /^\\/\\/\\s*@gate\\s+([A-Za-z]+)\\s*$/.exec(l.trim()); if (m) return m[1] } return null }",
  "export default { id: 'weekly', *tasks () {",
  '  for (const f of files.sort()) {',
  "    if (decl(f) === 'weekly') yield { args: [path.join(scripts, f), '--strict'] }",
  '  }',
  '} }',
].join('\n') + '\n'

/**
 * 造一棵最小仓库。
 * @param {object} o
 * @param {Record<string,string|null>} o.scripts  `{ '名字': 'bucket' }`；值 `null` = 干脆不写声明
 * @param {string[]} o.tests   磁盘上的测试名（相对 `scripts/`，不带 `.test.mjs`）
 * @param {{name:string,body:string}[]} [o.stages] 阶段文件；不传则只放一份夹具 `30-docs`
 * @param {Record<string,string>} [o.extra]  额外文件（键是相对仓库根的路径）
 */
function audit({
  scripts = { 'grep-check': 'guard' },
  tests = ['grep-check'],
  stages = undefined,
  extra = {},
} = {}) {
  return withTempTree({}, (root) => {
    const files = {
      ...Object.fromEntries(
        Object.entries(scripts).map(([n, bucket]) => [
          `scripts/${n}.mjs`,
          bucket === null ? '// 一份没有 @gate 声明的脚本\n' : `// @gate ${bucket}\n`,
        ]),
      ),
      ...Object.fromEntries(tests.map((n) => [`scripts/${n}.test.mjs`, '// test\n'])),
      ...(stages === undefined
        ? { 'scripts/gate.d/30-docs.mjs': DOCS }
        : Object.fromEntries(stages.map((s) => [`scripts/gate.d/${s.name}`, s.body]))),
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
  }, { prefix: 'gate-wiring-' })
}

test('接线齐备：声明、测试、任务表三方一致 ⇒ 通过（exit 0）', () => {
  const r = audit({ scripts: { 'grep-check': 'guard', 'md-table': 'lib' }, tests: ['grep-check'] })
  expectAudit(r, 0, [/gate-wiring-audit 通过：2 份脚本全部声明/])
})

// ── GW-001 · 声明齐全（缺 / 拼错 / 两条）────────────────────────────────
// 三种坏法都通向同一件事：脚本在仓里，却不在任何阶段的任务表里，而门禁全绿。

test('GW-001 脚本没有 @gate 声明 ⇒ 红（它是隐形脚本）', () => {
  const r = audit({ scripts: { 'grep-check': 'guard', 'orphan-check': null } })
  expectAudit(r, 1, [/GW-001 scripts\/orphan-check\.mjs 没有/])
})

test('GW-001 桶名拼错 ⇒ 红（它会落进「不跑」那一桶）', () => {
  const r = audit({ scripts: { 'typo-check': 'gaurd' }, tests: ['typo-check'] })
  expectAudit(r, 1, [/GW-001 scripts\/typo-check\.mjs 的 .*gaurd.* 不在闭集内/])
})

test('GW-001 两条 @gate 声明 ⇒ 红（「到底算哪一类」没有答案）', () => {
  const r = audit({
    scripts: { 'grep-check': 'guard' },
    extra: { 'scripts/two-face.mjs': '// @gate guard\n// @gate lib\n' },
  })
  expectAudit(r, 1, [/GW-001 scripts\/two-face\.mjs 有两条以上/])
})

test('GW-001 声明掉在正文深处不算（前 80 行之外读不到，人也就看不到）', () => {
  const body = `${'// 填充\n'.repeat(90)}// @gate guard\n`
  const r = audit({ extra: { 'scripts/deep-tag.mjs': body } })
  expectAudit(r, 1, [/GW-001 scripts\/deep-tag\.mjs 没有/])
})

// ── GW-002 · 守卫必须有回归测试 ────────────────────────────────────────
// 没有回归测试的守卫只是一个「会亮绿灯的脚本」：它腐烂的方式恰恰是规则写错所以永不命中。

test('GW-002 声明 guard 而没有同名测试 ⇒ 红', () => {
  const r = audit({ scripts: { 'gone-check': 'guard' }, tests: [] })
  expectAudit(r, 1, [/GW-002 scripts\/gone-check\.mjs 声明 .*guard.*没有/])
})

test('GW-002 声明 lib 不要求测试 ⇒ 绿（共享库放行对照）', () => {
  const r = audit({ scripts: { 'grep-check': 'guard', 'fs-walk': 'lib' }, tests: ['grep-check'] })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

// ── GW-003 / GW-004 · 声明的东西真的被跑到 ──────────────────────────────
// 这一对守的是本机制唯一的坏法：枚举写窄。声明与目录都在，任务表里却没有它们。

/** 枚举写窄的夹具：只跑测试，一条守卫本体都不跑（正则只认 `-audit` 之类的最小复刻） */
const DOCS_NARROW = [
  "import fs from 'node:fs'",
  "import path from 'node:path'",
  "import { fileURLToPath } from 'node:url'",
  "const scripts = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')",
  "const files = fs.readdirSync(scripts).filter((f) => f.endsWith('.test.mjs'))",
  "export default { id: 'docs', *tasks () { for (const f of files.sort()) yield { args: ['--test', path.join(scripts, f)] } } }",
].join('\n') + '\n'

test('GW-003 守卫声明着却不在任务表里 ⇒ 红（枚举写窄）', () => {
  const r = audit({
    scripts: { 'grep-check': 'guard' },
    tests: ['grep-check'],
    stages: [{ name: '30-docs.mjs', body: DOCS_NARROW }],
  })
  expectAudit(r, 1, [/GW-003 scripts\/grep-check\.mjs 声明 .*guard.*没有产出/])
})

test('GW-004 测试文件在而任务表里没有它 ⇒ 红（这个方向最贵：早就红了没人知道）', () => {
  // 反方向的枚举写窄：本体跑了、回归测试漏了。
  const docsNoTests = [
    "import fs from 'node:fs'",
    "import path from 'node:path'",
    "import { fileURLToPath } from 'node:url'",
    "const scripts = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')",
    "const files = fs.readdirSync(scripts).filter((f) => f.endsWith('.mjs') && !f.endsWith('.test.mjs'))",
    "export default { id: 'docs', *tasks () { for (const f of files.sort()) yield { args: [path.join(scripts, f), '--strict'] } } }",
  ].join('\n') + '\n'
  const r = audit({
    scripts: { 'grep-check': 'guard' },
    tests: ['grep-check', 'stale'],
    stages: [{ name: '30-docs.mjs', body: docsNoTests }],
  })
  expectAudit(r, 1, [/GW-004 scripts\/stale\.test\.mjs 存在/])
})

// ── weekly 桶：执行方是 62-weekly，对账口径与 guard 相同 ────────────────
// 换了执行方，「声明 ↔ 物化任务」的对账要跟着换一次：30-docs 不跑 weekly，
// 所以 weekly 声明齐不齐要看 62-weekly 的任务表，缺执行方 = 判据静默不跑的同一种病。

test('weekly 接线齐备：62-weekly 产出它的 --strict 任务 ⇒ 通过（exit 0）', () => {
  const r = audit({
    scripts: { 'grep-check': 'guard', 'style-check': 'weekly' },
    tests: ['grep-check', 'style-check'],
    stages: [
      { name: '30-docs.mjs', body: DOCS },
      { name: '62-weekly.mjs', body: WEEKLY },
    ],
  })
  expectAudit(r, 0, [/weekly 1/])
})

test('GW-002 声明 weekly 却没有同名测试 ⇒ 红（周档守卫一样只是会亮绿灯的脚本）', () => {
  const r = audit({ scripts: { 'style-check': 'weekly' }, tests: [] })
  expectAudit(r, 1, [/GW-002 scripts\/style-check\.mjs 声明 .*weekly.*没有/])
})

test('GW-003 声明 weekly 而 62-weekly 不存在 ⇒ 红（执行方缺席，声明全部落空）', () => {
  const r = audit({ scripts: { 'style-check': 'weekly' }, tests: ['style-check'] })
  expectAudit(r, 1, [/GW-003 \/ GW-004 62-weekly 的任务表读不出/])
})

test('GW-003 62-weekly 跑了没声明 weekly 的脚本 ⇒ 红（这一步不在机制的账上）', () => {
  const runaway = [
    "import fs from 'node:fs'",
    "import path from 'node:path'",
    "import { fileURLToPath } from 'node:url'",
    "const scripts = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')",
    'const files = []',
    "const walk = (d, p) => { for (const e of fs.readdirSync(d, { withFileTypes: true })) { if (e.isDirectory()) { if (e.name !== 'gate.d') walk(path.join(d, e.name), p + e.name + '/') } else if (e.name.endsWith('.mjs') && !e.name.endsWith('.test.mjs')) files.push(p + e.name) } }",
    "walk(scripts, '')",
    "export default { id: 'weekly', *tasks () { for (const f of files.sort()) yield { args: [path.join(scripts, f), '--strict'] } } }",
  ].join('\n') + '\n'
  const r = audit({
    scripts: { 'grep-check': 'guard', 'style-check': 'weekly' },
    tests: ['grep-check', 'style-check'],
    stages: [
      { name: '30-docs.mjs', body: DOCS },
      { name: '62-weekly.mjs', body: runaway },
    ],
  })
  expectAudit(r, 1, [/GW-003 62-weekly 跑了 scripts\/grep-check\.mjs/])
})

test('子目录按相对名对账：`gate.d/_shared` 声明 test 并真有测试 ⇒ 绿', () => {
  const r = audit({
    scripts: { 'grep-check': 'guard', 'gate.d/_shared': 'test' },
    tests: ['grep-check', 'gate.d/_shared'],
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('阶段文件（`gate.d/` 非 `_` 前缀）按文件名接入，不要求声明', () => {
  const r = audit({
    stages: [
      { name: '30-docs.mjs', body: DOCS },
      { name: '10-backend.mjs', body: `export default { id: "backend", tasks: () => ${ONE} }\n` },
    ],
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('垃圾目录不算：`node_modules` / `tmp` 下的 .mjs 不进对账', () => {
  const r = audit({
    extra: {
      'scripts/node_modules/pkg/b.test.mjs': '//\n',
      'scripts/tmp/c.mjs': '//\n',
      'scripts/target/d.test.mjs': '//\n',
    },
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('审计范围读不出：没有 `30-docs` ⇒ 失败而不是绿灯（exit 2）', () => {
  return withTempTree({}, (root) => {
    fs.mkdirSync(path.join(root, 'scripts'), { recursive: true })
    fs.writeFileSync(path.join(root, 'scripts', 'a.test.mjs'), '//\n')
    fs.writeFileSync(path.join(root, 'scripts', 'a.mjs'), '// @gate guard\n')
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    expectAudit(r, 2, [/30-docs 的任务表读不出|审计范围读不出/])
  }, { prefix: 'gate-wiring-' })
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

const run = (stages) => audit({ stages: [{ name: '30-docs.mjs', body: DOCS }, ...stages] })

test('GW-005 真实形状：生成在前、只读复核在后 ⇒ 绿（放行对照）', () => {
  const r = run([
    { name: '05-fmt.mjs', body: GEN('fmt') },
    { name: '06-facts-write.mjs', body: GEN('facts-write') },
    { name: '55-verify.mjs', body: PLAIN('verify') },
    { name: '60-facts.mjs', body: RECHK('facts') },
  ])
  expectAudit(r, 0, [/生成阶段 06-facts-write\.mjs 先于只读复核 60-facts\.mjs/])
})

test('GW-005 只读复核排在生成阶段之前 ⇒ 红（它复核的是上一版投影）', () => {
  const r = run([
    { name: '50-facts.mjs', body: RECHK('facts') },
    { name: '60-facts-write.mjs', body: GEN('facts-write') },
  ])
  expectAudit(r, 1, [/GW-005 scripts\/gate\.d\/50-facts\.mjs/])
})

test('GW-005 声明 regenerates 却没有调用形状 ⇒ 红（声明是假的）', () => {
  const r = run([
    { name: '06-facts-write.mjs', body: `export default { id: "facts-write", regenerates: true, tasks: () => ${ONE} }\n` },
    { name: '60-facts.mjs', body: RECHK('facts') },
  ])
  expectAudit(r, 1, [/GW-005 scripts\/gate\.d\/06-facts-write\.mjs 声明 .*regenerates.*却没有/])
})

test('GW-005 调用了原语却不声明 ⇒ 红（判据看不见这个生成阶段）', () => {
  const r = run([
    { name: '06-facts-write.mjs', body: `export default { id: "facts-write", tasks: () => [{ run: (c) => autoChain(c, {}) }] }\n` },
    { name: '60-facts.mjs', body: RECHK('facts') },
  ])
  expectAudit(r, 1, [/GW-005 scripts\/gate\.d\/06-facts-write\.mjs 调用/])
})

test('GW-005 有生成却没有只读复核 ⇒ 红（没人对整轮末态）', () => {
  const r = run([{ name: '06-facts-write.mjs', body: GEN('facts-write') }, { name: '55-verify.mjs', body: PLAIN('verify') }])
  expectAudit(r, 1, [/GW-005 1 个阶段声明 .*regenerates.*，却没有一个阶段声明 .*recheck/])
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
  expectAudit(r, 1, [/GW-005 scripts\/gate\.d\/60-facts\.mjs 同时声明/])
})

test('GW-005 阶段文件读不进（语法错）⇒ 红，不再静默按「没声明」处理', () => {
  const r = run([
    { name: '06-facts-write.mjs', body: GEN('facts-write') },
    { name: '60-facts.mjs', body: 'export default { id: "facts", recheck: true,\n' },
  ])
  expectAudit(r, 1, [/GW-005 scripts\/gate\.d\/60-facts\.mjs 导入失败/])
})

test('GW-005 没有生成阶段也没有复核 ⇒ 不判（空表不制造义务）', () => {
  const r = run([{ name: '10-a.mjs', body: PLAIN('a') }, { name: '20-b.mjs', body: PLAIN('b') }])
  expectAudit(r, 0, [/生成阶段 （无）/])
})

// ── GW-006 · 阶段不空转（本轮产出 0 项就判红）──────────────────────────────
// 治的是「阶段还在、判据没了」：`50-msrv` 写死了 D-191 之前那两个顶层目录，拆成 crate 之后
// 路径不存在 ⇒ 每轮产出 0 项、汇总表上一行都没有，与「全绿」同形，
// 而 CI 的 msrv job 一直绿。所以下面第 1 条就是这个形状的最小复刻。

const EMPTY = (id, extra = '') =>
  `export default { id: ${JSON.stringify(id)}, ${extra}${extra ? ', ' : ''}tasks: () => [] }\n`

test('GW-006 阶段产出 0 项且没声明 ⇒ 红（msrv 空转的最小复刻）', () => {
  const r = run([{ name: '50-msrv.mjs', body: EMPTY('msrv') }])
  expectAudit(r, 1, [/GW-006 scripts\/gate\.d\/50-msrv\.mjs 产出 0 个检查项/])
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
  expectAudit(r, 1, [/GW-006 scripts\/gate\.d\/58-e2e\.mjs 声明了 .*mayBeEmpty.*不是一句理由/])
})

test('GW-006 阶段文件没有 default 导出 ⇒ 红（它在阶段目录里，却不是阶段）', () => {
  const r = run([{ name: '40-half-written.mjs', body: '// 忘了写 export default\n' }])
  expectAudit(r, 1, [/GW-006 scripts\/gate\.d\/40-half-written\.mjs 没有 `export default/])
})

test('GW-006 tasks() 物化时抛 ⇒ 报「物化失败」，不伪装成「0 项」', () => {
  // 「读不出」与「确实没有」是两件事：把前者报成后者，等于让一个坏阶段躲进豁免的形状里。
  const r = run([
    {
      name: '45-weird.mjs',
      body: 'export default { id: "weird", tasks: (ctx) => { throw new Error("派生路径不存在") } }\n',
    },
  ])
  expectAudit(r, 1, [/GW-006 scripts\/gate\.d\/45-weird\.mjs 的 tasks\(\) 物化失败/, /派生路径不存在/])
  assert.doesNotMatch(r.stdout, /45-weird\.mjs 产出 0 个检查项/)
})

test('GW-006 有项就完全不介入：正常阶段不因为「本轮 1 项」被念一句', () => {
  const r = run([{ name: '10-a.mjs', body: PLAIN('a') }])
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.doesNotMatch(r.stdout, /GW-006/)
})

// ── GW-007 · 目录即分组（`-audit` 名字必须住进 `scripts/audits/`）───────────
// 留在根目录不会被漏跑，所以没有任何机制会提醒搬家——实测就这么剩了四份，
// 而 CONTRIBUTING 里那句「静态审计脚本都在 scripts/audits/」当时已经是假话。

test('GW-007 `-audit` 脚本留在 scripts 根下 ⇒ 红', () => {
  const r = audit({ scripts: { 'legacy-audit': 'guard' }, tests: ['legacy-audit'] })
  expectAudit(r, 1, [/GW-007 scripts\/legacy-audit\.mjs 名字以 `-audit` 结尾却不在 `scripts\/audits\/` 下/])
})

test('GW-007 测试文件也算：脚本搬了、测试留在根下 ⇒ 红', () => {
  const r = audit({ scripts: { 'audits/legacy-audit': 'guard' }, tests: ['audits/legacy-audit', 'legacy-audit'] })
  expectAudit(r, 1, [/GW-007 scripts\/legacy-audit\.test\.mjs/])
  assert.doesNotMatch(r.stdout, /GW-007 scripts\/audits\/legacy-audit\.mjs/)
})

test('GW-007 住进 scripts/audits/ ⇒ 绿（放行对照）', () => {
  const r = audit({ scripts: { 'audits/legacy-audit': 'guard' }, tests: ['audits/legacy-audit'] })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.doesNotMatch(r.stdout, /GW-007/)
})

test('GW-007 是单向判据：audits/ 下不带 `-audit` 的名字照样放行', () => {
  // `projection-purity` 就是这种形状。把「名字」变成判据的输入，等于让改名成为判据的一部分。
  const r = audit({ scripts: { 'audits/projection-purity': 'guard' }, tests: ['audits/projection-purity'] })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.doesNotMatch(r.stdout, /GW-007/)
})

// ── GW-008 · 生成链的声明卫生（序数唯一 / 模式在闭集）──────────────────
// 声明与**物化任务**的双向对账由真仓 dogfood（`06-facts-write` 的 `STEPS` 与
// `60-facts` 的任务表）；这里钉的是解析面的三种坏法——它们不必有阶段文件就能判。

test('GW-008 两条声明抢同一个序数 ⇒ 红（链上「谁先跑」没有答案）', () => {
  const r = audit({
    scripts: { g1: 'lib', g2: 'lib' },
    tests: [],
    extra: {
      'scripts/g1.mjs': '// @gate lib\n// @gens 10 write\n',
      'scripts/g2.mjs': '// @gate lib\n// @gens 10 check\n',
    },
  })
  expectAudit(r, 1, [/GW-008 scripts\/g2\.mjs 的 `@gens` 声明不合格（duplicate-order/])
})

test('GW-008 模式名拼错 ⇒ 红（拼错的那一档从此不跑，而阶段遍历的就是同一份声明）', () => {
  const r = audit({
    scripts: { g1: 'lib' },
    tests: [],
    extra: { 'scripts/g1.mjs': '// @gate lib\n// @gens 10 writes\n' },
  })
  expectAudit(r, 1, [/GW-008 scripts\/g1\.mjs 的 `@gens` 声明不合格（bad-mode：writes）/])
})

test('GW-008 声明形状不对（缺序数）⇒ 红', () => {
  const r = audit({
    scripts: { g1: 'lib' },
    tests: [],
    extra: { 'scripts/g1.mjs': '// @gate lib\n// @gens write\n' },
  })
  expectAudit(r, 1, [/GW-008 scripts\/g1\.mjs 的 `@gens` 声明不合格（bad-shape/])
})

test('GW-008 一份 write 一份 check、序数不同 ⇒ 绿（没有阶段文件时不制造义务）', () => {
  const r = audit({
    scripts: { 'grep-check': 'guard', g1: 'lib', g2: 'lib' },
    tests: ['grep-check'],
    extra: {
      'scripts/g1.mjs': '// @gate lib\n// @gens 10 write\n',
      'scripts/g2.mjs': '// @gate lib\n// @gens 20 check\n',
    },
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.doesNotMatch(r.stdout, /GW-008/)
})

// ── GW-009 · 判据生命周期（@state 状态字 ↔ off / 任务表 / 裁决账）────────
// 状态字读法在 `gate.d/_shared.mjs::stateDeclarations`（缺省即 live，entries 只有离场判据）。
// 夹具里 `--root=` 传入 ⇒ 裁决号锚点面收窄到夹具自己的 `docs/`，与真仓同语义。

/** 一份在役守卫的默认夹具，供各用例叠加离场脚本。 */
const GREEN_BASE = { 'grep-check': 'guard' }

test('GW-009 retired 状态字 + 能打开的裁决 + off + 不在任何任务表 ⇒ 绿', () => {
  const r = audit({
    scripts: GREEN_BASE,
    tests: ['grep-check'],
    extra: {
      'scripts/old-check.mjs': '// @gate off\n// @state retired: D-123 判的是迁走前的旧形状\n',
      'docs/design.md': '### D-123 退役裁决\n',
    },
  })
  expectAudit(r, 0, [/离场判据 1 条/])
})

test('GW-009 redundant 状态字 + 非空承继者 + off ⇒ 绿（壳留到下一批删）', () => {
  const r = audit({
    scripts: GREEN_BASE,
    tests: ['grep-check'],
    extra: {
      'scripts/old-check.mjs': '// @gate off\n// @state redundant: GW-001\n',
    },
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('GW-009 retired 的裁决打不开 ⇒ 红（退役必须挂在一条设计裁决上）', () => {
  const r = audit({
    scripts: GREEN_BASE,
    tests: ['grep-check'],
    extra: {
      'scripts/old-check.mjs': '// @gate off\n// @state retired: D-123 理由\n',
      'docs/design.md': '### D-120 另一条裁决\n',
    },
  })
  expectAudit(r, 1, [/GW-009 scripts\/old-check\.mjs 自述 `@state retired: D-123`.*锚点在登记面打不开/])
})

test('GW-009 状态字拼错 ⇒ 红（闭集之外没有第四种状态）', () => {
  const r = audit({
    scripts: GREEN_BASE,
    tests: ['grep-check'],
    extra: { 'scripts/old-check.mjs': '// @gate off\n// @state gone: 想不出词\n' },
  })
  expectAudit(r, 1, [/GW-009 scripts\/old-check\.mjs 的 `@state` 声明不合格（unknown：gone）/])
})

test('GW-009 两条 @state 声明 ⇒ 红（在不在役没有答案）', () => {
  const r = audit({
    scripts: GREEN_BASE,
    tests: ['grep-check'],
    extra: { 'scripts/old-check.mjs': '// @gate off\n// @state live\n// @state redundant: GW-001\n' },
  })
  expectAudit(r, 1, [/GW-009 scripts\/old-check\.mjs 的 `@state` 声明不合格（duplicate/])
})

test('GW-009 redundant / retired 缺参数 ⇒ 红（空指向与没有裁决号的退役）', () => {
  const r = audit({
    scripts: GREEN_BASE,
    tests: ['grep-check'],
    extra: {
      'scripts/a.mjs': '// @gate off\n// @state redundant:\n',
      'scripts/b.mjs': '// @gate off\n// @state retired: D-123\n',
    },
  })
  expectAudit(r, 1, [/GW-009 scripts\/a\.mjs 的 `@state` 声明不合格（empty-arg/, /GW-009 scripts\/b\.mjs 的 `@state` 声明不合格（retired-needs-decision/])
})

test('GW-009 自述退场、@gate 却还在跑 ⇒ 红（两行自述互相矛盾）', () => {
  const r = audit({
    scripts: GREEN_BASE,
    tests: ['grep-check'],
    extra: { 'scripts/old-check.mjs': '// @gate lib\n// @state redundant: GW-001\n' },
  })
  expectAudit(r, 1, [/GW-009 scripts\/old-check\.mjs 自述 `@state redundant`，`@gate` 却不是 off/])
})

test('GW-009 退场判据（或它的测试）仍被任务表跑到 ⇒ 红（退场不等于留壳）', () => {
  const r = audit({
    scripts: GREEN_BASE,
    tests: ['grep-check', 'old-check'],
    extra: {
      'scripts/old-check.mjs': '// @gate off\n// @state retired: D-123 判的是迁走前的旧形状\n',
      'docs/design.md': '### D-123 退役裁决\n',
    },
  })
  expectAudit(r, 1, [/GW-009 scripts\/old-check\.mjs 自述 `@state retired`，却仍被物化出的任务表跑到/])
})
