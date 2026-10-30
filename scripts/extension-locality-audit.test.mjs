/**
 * extension-locality-audit 回归测试 —— 证明 EX-002 **改输入后结论必须变**
 *
 * `EX-002`（T1）的判定对象是**提交 diff 的四棵树指纹**，不是「有没有新 crate」。
 * 这条守卫的失效形态都是「看起来在算」：
 *
 * - 只判新 crate 是否存在 ⇒ 新增能力包夹带 `loop` 分支照样绿；
 * - 把 `*.gen.rs` 一律豁免 ⇒ 手写一个新文件冒充生成物；
 * - 指纹读不出时报「没有违规」⇒ 判据没跑却全绿。
 *
 * 本文件逐条注入并断言红；另有一条真仓冒烟，证明读 git 那一半也真的在跑。
 *
 * 用法：node --test scripts/extension-locality-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import {
  TREES,
  addedPackages,
  fingerprints,
  isMigrationBatch,
  judgeNewCapability,
  parseRawCommits,
  validateBeforeGuard,
} from './extension-locality-audit.mjs'

const script = fileURLToPath(new URL('./extension-locality-audit.mjs', import.meta.url))
const repoRoot = path.resolve(path.dirname(script), '..')

const f = (p, oid) => ({ path: p, oid })
const paths = (entries) => new Set(entries.map((e) => e.path))

test('parseRawCommits：正文里以冒号开头的行不截断消息，状态与路径读得出', () => {
  const out = [
    '\x01abc123\x00feat: 新增能力包',
    '',
    'Core-Change: D-247 补件已先行',
    '',
    ':000000 100644 0000000 aaaa111 A\tcrates/symbio-plugin-notes/PLUGIN.yml',
    ':100644 100644 bbbb222 cccc333 M\tcrates/symbio-plugin-loop/src/loop_rows.rs',
  ].join('\n')
  const [c] = parseRawCommits(out)
  assert.equal(c.sha, 'abc123')
  assert.match(c.message, /Core-Change: D-247/)
  assert.deepEqual(
    c.files.map((x) => [x.status, x.path]),
    [
      ['A', 'crates/symbio-plugin-notes/PLUGIN.yml'],
      ['M', 'crates/symbio-plugin-loop/src/loop_rows.rs'],
    ],
  )
})

test('addedPackages：只认新增的插件清单', () => {
  const commit = {
    files: [
      { status: 'A', path: 'crates/symbio-plugin-notes/PLUGIN.yml' },
      { status: 'M', path: 'crates/symbio-plugin-heartbeat/PLUGIN.yml' },
      { status: 'A', path: 'crates/symbio-plugin-notes/src/decl.rs' },
      { status: 'A', path: 'examples/fullstack-dev/mcp/PLUGIN.yml' },
      { status: 'A', path: 'agent/x/PLUGIN.yml' },
    ],
  }
  assert.deepEqual(addedPackages(commit), ['crates/symbio-plugin-notes/PLUGIN.yml'])
})

test('isMigrationBatch：正文声明 Migration: 的提交是迁移批次（EX-003 接管），没声明的不豁免', () => {
  assert.equal(isMigrationBatch('feat(x): 搬家\n\nMigration: L3a'), true)
  assert.equal(isMigrationBatch('Migration: L1\n'), true)
  assert.equal(isMigrationBatch('feat(task): 新增能力包\n\n正文里提到 Migration 这个词但没声明行'), false)
  assert.equal(isMigrationBatch(''), false)
  assert.equal(isMigrationBatch(null), false)
})

test('fingerprints：与输入顺序无关', () => {
  const entries = [
    f('crates/symbio-core/src/a.rs', '111'),
    f('crates/symbio-kernel/src/b.rs', '222'),
    f('crates/symbio-plugin-loop/src/c.rs', '333'),
  ]
  assert.deepEqual(fingerprints(entries), fingerprints([...entries].reverse()))
})

test('fingerprints：已存在的 *.gen.rs 随清单改，不动指纹', () => {
  const before = [
    f('crates/symbio-core/src/authz/rows.gen.rs', 'old'),
    f('crates/symbio-core/src/authz/mod.rs', 'same'),
  ]
  const after = [
    f('crates/symbio-core/src/authz/rows.gen.rs', 'new'),
    f('crates/symbio-core/src/authz/mod.rs', 'same'),
  ]
  const parent = paths(before)
  assert.equal(fingerprints(before, parent)['crates/symbio-core'], fingerprints(after, parent)['crates/symbio-core'])
})

test('fingerprints：新出现的 *.gen.rs 不是生成物镜像，会动指纹', () => {
  const before = [f('crates/symbio-core/src/domain.rs', 'same')]
  const after = [
    f('crates/symbio-core/src/domain.rs', 'same'),
    f('crates/symbio-core/src/agent_helpers.gen.rs', 'fresh'),
  ]
  const parent = paths(before)
  assert.notEqual(fingerprints(before, parent)['crates/symbio-core'], fingerprints(after, parent)['crates/symbio-core'])
})

test('fingerprints：一棵树的改动不串到另一棵', () => {
  const before = [
    f('crates/symbio-core/src/a.rs', '1'),
    f('crates/symbio-plugin-loop/src/b.rs', '2'),
  ]
  const after = [
    f('crates/symbio-core/src/a.rs', '9'),
    f('crates/symbio-plugin-loop/src/b.rs', '2'),
  ]
  const b = fingerprints(before, paths(before))
  const a = fingerprints(after, paths(before))
  assert.notEqual(a['crates/symbio-core'], b['crates/symbio-core'])
  assert.equal(a['crates/symbio-plugin-loop'], b['crates/symbio-plugin-loop'])
})

test('judgeNewCapability：没有新增能力包的提交不判', () => {
  const r = judgeNewCapability({ sha: 'x', added: [], before: {}, after: {} })
  assert.deepEqual(r.errors, [])
})

test('judgeNewCapability：四棵树指纹不变 ⇒ 绿', () => {
  const same = Object.fromEntries(TREES.map((t) => [t, 'h']))
  const r = judgeNewCapability({ sha: 'x', added: ['crates/symbio-plugin-new/PLUGIN.yml'], before: same, after: { ...same } })
  assert.deepEqual(r.errors, [])
})

test('judgeNewCapability：新增能力包夹带 loop 分支 ⇒ 红且点名 loop', () => {
  const before = Object.fromEntries(TREES.map((t) => [t, 'h']))
  const after = { ...before, 'crates/symbio-plugin-loop': 'changed' }
  const r = judgeNewCapability({ sha: 'x', added: ['crates/symbio-plugin-new/PLUGIN.yml'], before, after })
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /EX-002/)
  assert.match(r.errors[0], /crates\/symbio-plugin-loop/)
})

test('judgeNewCapability：新增能力包改 core ⇒ 红', () => {
  const before = Object.fromEntries(TREES.map((t) => [t, 'h']))
  const after = { ...before, 'crates/symbio-core': 'changed' }
  const r = judgeNewCapability({ sha: 'x', added: ['crates/symbio-plugin-new/PLUGIN.yml'], before, after })
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /crates\/symbio-core/)
})

test('judgeNewCapability：指纹读不出 ⇒ 红（判据没有输入）', () => {
  const r = judgeNewCapability({ sha: 'x', added: ['crates/symbio-plugin-new/PLUGIN.yml'], before: {}, after: {} })
  assert.equal(r.errors.length, TREES.length)
  assert.ok(r.errors.every((e) => /读不出/.test(e)))
})

test('validateBeforeGuard：豁免必须有提交号与理由', () => {
  assert.deepEqual(validateBeforeGuard([{ sha: '92cd0528', why: '搬家本身' }]), [])
  assert.match(validateBeforeGuard([{ sha: 'xyz', why: 'x' }])[0], /提交号/)
  assert.match(validateBeforeGuard([{ sha: '92cd0528', why: '  ' }])[0], /理由/)
})

test('真仓冒烟：夹具提交读得出，--last 跑绿', () => {
  const r = spawnSync(process.execPath, [script, '--last'], { cwd: repoRoot, encoding: 'utf8', shell: false })
  assert.equal(r.status, 0, r.stderr || r.stdout)
  assert.match(r.stdout, /77ff41fa/)
  assert.match(r.stdout, /四棵树指纹不变/)
})
