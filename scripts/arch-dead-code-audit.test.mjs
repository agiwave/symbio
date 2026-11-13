import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { test } from 'node:test'
import {
  extractSymbols,
  callCorpus,
  zeroCallSymbols,
  checkBaseline,
  judge,
  surfaceErrors,
} from './arch-dead-code-audit.mjs'

const AUDIT = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\//, '')), 'arch-dead-code-audit.mjs')
const REPO = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\//, '')))

function tmpRoot() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'ad-audit-'))
}

/**
 * 最小夹具：架构树各一条 `pub fn`，`crates` 下补足语料下限（50 个生产 .rs）。
 *
 * `symbio-contract`（`D-486` 之后是扫描面的第四棵树）只放一个 `pub struct`：
 * 判据要的是「这棵树下有生产 .rs」，而本测试判的是「零调用的 `pub fn`」——
 * 多放一个 `pub fn` 会把每条断言里的符号数一起推高，测的就不再是判据本身。
 */
function fixture({ sdk = 'pub fn target() {}\n', caller = '', testCaller = '', baseline = null, filler = 48 } = {}) {
  const root = tmpRoot()
  const w = (rel, text) => {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, text)
  }
  w('crates/symbio-sdk/src/lib.rs', sdk)
  w('crates/symbio-kernel/src/lib.rs', 'pub fn k() {}\n')
  w('crates/symbio-plugin-api/src/lib.rs', 'pub fn a() {}\n')
  w('crates/symbio-contract/src/lib.rs', 'pub fn contract_marker() {}\nfn uses_marker() {\n    contract_marker()\n}\n')
  w('docs/003-design/018-模块架构.md', '# 模块架构\n')
  if (caller) w('crates/consumer/src/lib.rs', caller)
  if (testCaller) w('crates/symbio-sdk/src/lib.test.rs', testCaller)
  for (let i = 0; i < filler; i++) w(`crates/filler/src/f${i}.rs`, `pub fn f${i}() {}\n`)
  if (baseline) {
    w(
      'scripts/baselines/arch-dead-code.json',
      JSON.stringify({ note: '', symbols: baseline }, null, 2),
    )
  }
  return root
}

function run(root, extra = []) {
  return spawnSync(process.execPath, [AUDIT, `--root=${root}`, ...extra], {
    encoding: 'utf8',
    cwd: REPO,
  })
}

const goodBaseline = [
  {
    symbol: 'target',
    path: 'crates/symbio-sdk/src/lib.rs',
    why: '契约实现',
    evidence: 'docs/003-design/018-模块架构.md',
  },
  { symbol: 'k', path: 'crates/symbio-kernel/src/lib.rs', why: '契约实现', evidence: 'docs/003-design/018-模块架构.md' },
  { symbol: 'a', path: 'crates/symbio-plugin-api/src/lib.rs', why: '契约实现', evidence: 'docs/003-design/018-模块架构.md' },
]
const kAndA = goodBaseline.slice(1)

// ── 口径：生产语料、use 行、同名多定义 ────────────────────────────

test('调用只出现在测试文件 ⇒ 仍算零生产调用', () => {
  const root = fixture({ testCaller: '#[test] fn t() { target(); }\n', baseline: kAndA })
  const r = run(root)
  assert.equal(r.status, 1, r.stdout + r.stderr)
  assert.match(r.stdout, /AD-001.*target/, '测试里的调用不许给符号续命')
})

test('use / pub use 行不算调用（转出不是调用）', () => {
  const root = fixture({ caller: 'pub use symbio_sdk::target;\n', baseline: kAndA })
  const r = run(root)
  assert.equal(r.status, 1, r.stdout + r.stderr)
  assert.match(r.stdout, /AD-001.*target/)
})

test('跨 crate 的普通调用算生产调用', () => {
  const root = fixture({ caller: 'pub fn boot() { symbio_sdk::target(); }\n', baseline: kAndA })
  const r = run(root)
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('同名多定义不判（名字级计数分不清是哪一处）', () => {
  const symbols = [
    { symbol: 'new', path: 'crates/symbio-sdk/src/lib.rs', line: 1 },
    { symbol: 'new', path: 'crates/symbio-kernel/src/lib.rs', line: 1 },
  ]
  const corpus = new Map([
    ['crates/symbio-sdk/src/lib.rs', callCorpus('impl A { pub fn new() {} }\npub fn target() { new(); }\n')],
  ])
  const dead = zeroCallSymbols(symbols, corpus)
  assert.deepEqual(dead, [], '两处同名 new 不得判零调用')
})

test('extractSymbols 认 async / const / unsafe / extern 形态，不认 pub(crate)', () => {
  const text = [
    'pub async fn a() {}',
    'pub const fn b() {}',
    'pub unsafe fn c() {}',
    'pub extern "C" fn d() {}',
    'pub(crate) fn hidden() {}',
  ].join('\n')
  const names = extractSymbols(text, 'x.rs').map((s) => s.symbol)
  assert.deepEqual(names, ['a', 'b', 'c', 'd'])
})

// ── AD-001 / AD-002 ────────────────────────────────────────────────

test('AD-001：新出现的零调用符号即红', () => {
  const root = fixture({ baseline: kAndA })
  const r = run(root)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /AD-001.*target/)
})

test('AD-002：基线有而源码已删 / 已接线 ⇒ 红，逼同批下调基线', () => {
  const root = fixture({ caller: 'pub fn boot() { symbio_sdk::target(); }\n', baseline: goodBaseline })
  const r = run(root)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /AD-002.*target/, '基线留余量必须同批下调')
})

test('基线逐条对账通过 ⇒ 绿', () => {
  const root = fixture({ baseline: goodBaseline })
  const r = run(root)
  assert.equal(r.status, 0, r.stdout + r.stderr)
  const baseline = JSON.parse(fs.readFileSync(path.join(root, 'scripts/baselines/arch-dead-code.json'), 'utf8'))
  const errors = judge(
    [
      { symbol: 'target', path: 'crates/symbio-sdk/src/lib.rs', line: 1 },
      { symbol: 'k', path: 'crates/symbio-kernel/src/lib.rs', line: 1 },
      { symbol: 'a', path: 'crates/symbio-plugin-api/src/lib.rs', line: 1 },
    ],
    baseline,
  )
  assert.deepEqual(errors, [])
  assert.deepEqual(checkBaseline(baseline, root), [])
})

// ── AD-003：为什么 + 证据可打开 ────────────────────────────────────

test('AD-003：why 为空 ⇒ 红', () => {
  const root = fixture({ baseline: [{ symbol: 'target', path: 'crates/symbio-sdk/src/lib.rs', why: '  ', evidence: 'D-404' }] })
  const r = run(root)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /AD-003.*why/)
})

test('AD-003：evidence 指向不存在的决策 / 路径 ⇒ 红', () => {
  const root = fixture({
    baseline: [
      { symbol: 'target', path: 'crates/symbio-sdk/src/lib.rs', why: '契约', evidence: 'D-999' },
    ],
  })
  assert.match(run(root).stdout, /AD-003.*D-999/)

  const root2 = fixture({
    baseline: [
      { symbol: 'target', path: 'crates/symbio-sdk/src/lib.rs', why: '契约', evidence: 'docs/nope.md' },
    ],
  })
  assert.match(run(root2).stdout, /AD-003.*docs\/nope\.md/)
})

test('AD-003：缺 path / 重复登记 ⇒ 红', () => {
  const row = { symbol: 'target', path: 'crates/symbio-sdk/src/lib.rs', why: '契约', evidence: 'docs/003-design/018-模块架构.md' }
  const root = fixture({
    baseline: [{ symbol: 'target', why: '契约', evidence: 'docs/003-design/018-模块架构.md' }, row, { ...row }],
  })
  const out = run(root).stdout
  assert.match(out, /AD-003.*path/)
  assert.match(out, /AD-003.*重复/)
})

// ── AD-004：扫描面读不出 ⇒ 退出码 2 ───────────────────────────────

test('AD-004：架构树缺失 / 语料过少 ⇒ 退出码 2', () => {
  const root = fixture({ filler: 0 })
  fs.rmSync(path.join(root, 'crates/symbio-kernel'), { recursive: true, force: true })
  const r = run(root)
  assert.equal(r.status, 2, r.stdout + r.stderr)
  assert.match(r.stdout, /AD-004/)
})

test('AD-004：基线缺失 ⇒ 退出码 2（不按「零违规」通过）', () => {
  const root = fixture({ caller: 'pub fn boot() { symbio_sdk::target(); }\n' })
  assert.equal(run(root).status, 2)
})

test('surfaceErrors 对空扫描面报 AD-004', () => {
  const { errors } = surfaceErrors([])
  assert.ok(errors.some((e) => e.code === 'AD-004'))
})

// ── 历史回放：判据能指出「从哪一刻开始零调用」 ──────────────────────

test('--at：同一条提交前后，符号从「有调用」变成「零调用」', () => {
  const root = fixture({ caller: 'pub fn boot() { symbio_sdk::target(); }\n' })
  const git = (...args) =>
    spawnSync('git', args, { cwd: root, encoding: 'utf8' })
  git('init')
  git('config', 'user.email', 't@t')
  git('config', 'user.name', 't')
  git('add', '-A')
  const c1 = git('commit', '-m', 'caller present')
  assert.equal(c1.status, 0, c1.stderr)
  const before = git('rev-parse', 'HEAD').stdout.trim()

  fs.writeFileSync(path.join(root, 'crates/consumer/src/lib.rs'), 'pub fn boot() {}\n')
  git('add', '-A')
  const c2 = git('commit', '-m', 'caller removed')
  assert.equal(c2.status, 0, c2.stderr)
  const after = git('rev-parse', 'HEAD').stdout.trim()

  const a = run(root, [`--at=${before}`])
  const b = run(root, [`--at=${after}`])
  assert.equal(a.status, 0, a.stdout + a.stderr)
  assert.equal(b.status, 0, b.stdout + b.stderr)
  assert.doesNotMatch(a.stdout, /target/, '删除调用点之前不应零调用')
  assert.match(b.stdout, /target/, '删除调用点之后必须被指认')
})
