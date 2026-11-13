/**
 * baseline-scope-audit 回归测试 —— 证明「基线变了」这件事真的会红
 *
 * 每条用例只动一处：改一份基线、加/删一个键、动 mechanism 指纹、或写一条
 * `baseline-allow`。夹具是一个真的 git 仓（`git show <base>:<file>` 是判据的输入，
 * 不能拿两份 JSON 手抄替代），基线在初始提交里，改动放在工作树。
 *
 * 用法：node --test scripts/audits/baseline-scope-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./baseline-scope-audit.mjs', import.meta.url))
const LP_REL = 'scripts/loop-purity-baseline.json'
const CP_REL = 'scripts/baselines/control-plane.json'

function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

function writeTree(root, files) {
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
}

function lpBaseline() {
  return {
    note: '夹具：只许降',
    lp001PayloadImports: { 'port.rs': 2 },
    lp002ForbiddenDeps: ['dev-dependencies:symbio-providers'],
    lp003WorldRoots: { 'ring.rs': 3, 'entry.rs': 5 },
    lp004WorldNamedContracts: 2,
    lp005WorldPayloadSig: 2,
  }
}

function cpBaseline() {
  return {
    mechanism: {
      file: 'crates/symbio-plugin-loop/src/ring.rs',
      hash: 'sha256:old',
      parts: ['drive loop body'],
      adr: 'D-234',
      reason: '夹具：机制面冻结在干净形状上',
    },
    extension: { productionFiles: 47 },
  }
}

/** 建仓 → 提交基线 → 写工作树改动 → 跑审计。 */
function audit({ lp = lpBaseline(), cp = cpBaseline(), baseLp = lpBaseline(), baseCp = cpBaseline(), base = 'HEAD' } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'baseline-scope-'))
  try {
    writeTree(root, {
      [LP_REL]: JSON.stringify(baseLp, null, 2),
      [CP_REL]: JSON.stringify(baseCp, null, 2),
      'docs/003-design/048.md': '### D-321 撤销 D-320\n',
    })
    spawnSync('git', ['init'], { cwd: root, encoding: 'utf8' })
    spawnSync('git', ['add', '-A'], { cwd: root, encoding: 'utf8' })
    const commit = spawnSync(
      'git',
      ['-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-m', 'base'],
      { cwd: root, encoding: 'utf8' },
    )
    assert.equal(commit.status, 0, commit.stdout + commit.stderr)
    writeTree(root, {
      [LP_REL]: JSON.stringify(lp, null, 2),
      [CP_REL]: JSON.stringify(cp, null, 2),
      'docs/003-design/048.md': '### D-321 撤销 D-320\n',
    })
    return spawnSync(process.execPath, [script, `--root=${root}`, `--base=${base}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
    })
  } finally {
    rmTree(root)
  }
}

function assertRed(r, code) {
  assert.equal(r.status, 1, r.stdout + r.stderr)
  assert.match(r.stdout, new RegExp(`\\[ERROR\\] ${code} `), r.stdout)
}

// ───────────────────────────── 不该红 ─────────────────────────────

test('基线面一字未动 ⇒ 绿', () => {
  const r = audit()
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /无键增删、无计数上调/)
})

test('计数下调 ⇒ 绿（棘轮往下走不需要 allow）', () => {
  const lp = lpBaseline()
  lp.lp003WorldRoots['ring.rs'] = 2
  const r = audit({ lp })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

// ───────────────────────────── BS-001 / BS-002 ─────────────────────────────

test('BS-001：新增基线键（新文件）⇒ 红，且没有 allow 通道', () => {
  const lp = lpBaseline()
  lp.lp003WorldRoots['new_file.rs'] = 1
  const r = audit({ lp })
  assertRed(r, 'BS-001')
})

test('BS-002：任一项计数上调 ⇒ 红', () => {
  const lp = lpBaseline()
  lp.lp003WorldRoots['ring.rs'] = 4
  const r = audit({ lp })
  assertRed(r, 'BS-002')
})

test('BS-002：控制面 productionFiles 上调 ⇒ 红', () => {
  const cp = cpBaseline()
  cp.extension.productionFiles = 48
  const r = audit({ cp })
  assertRed(r, 'BS-002')
})

// ───────────────────────────── BS-003 ─────────────────────────────

test('BS-003：删除基线键而没有 allow ⇒ 红', () => {
  const lp = lpBaseline()
  delete lp.lp003WorldRoots['ring.rs']
  const r = audit({ lp })
  assertRed(r, 'BS-003')
})

test('BS-003：删除基线键并带 allow + D-321 ⇒ 绿', () => {
  const lp = lpBaseline()
  delete lp.lp003WorldRoots['ring.rs']
  lp['baseline-allow'] = ['lp003WorldRoots/ring.rs→(none) 该文件的世界词根已归零 D-321']
  const r = audit({ lp })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

// ───────────────────────────── BS-004 ─────────────────────────────

test('BS-004：mechanism.hash 变了而没有 allow ⇒ 红', () => {
  const cp = cpBaseline()
  cp.mechanism.hash = 'sha256:new'
  const r = audit({ cp })
  assertRed(r, 'BS-004')
})

test('BS-004：mechanism.hash 变了并带 allow ⇒ 绿', () => {
  const cp = cpBaseline()
  cp.mechanism.hash = 'sha256:new'
  cp['baseline-allow'] = ['sha256:old→sha256:new 驱动循环体随 D-321 迁移，逻辑逐字未动 D-321']
  const r = audit({ cp })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('BS-004：mechanism.file 变了而没有 allow ⇒ 红', () => {
  const cp = cpBaseline()
  cp.mechanism.file = 'crates/symbio-plugin-loop/src/entry.rs'
  const r = audit({ cp })
  assertRed(r, 'BS-004')
})

// ───────────────────────────── BS-005 ─────────────────────────────

test('BS-005：allow 形状不对（缺理由 / 缺决策号）⇒ 红', () => {
  const lp = lpBaseline()
  lp['baseline-allow'] = ['sha256:old→sha256:new']
  const r = audit({ lp })
  assertRed(r, 'BS-005')
})

test('BS-005：allow 指向一个没发生的变更 ⇒ 红（空头豁免）', () => {
  const lp = lpBaseline()
  lp['baseline-allow'] = ['x.rs→(none) 理由 D-321']
  const r = audit({ lp })
  assertRed(r, 'BS-005')
})

test('BS-005：allow 引用一个不存在的决策 ⇒ 红（不许匿名豁免）', () => {
  const lp = lpBaseline()
  delete lp.lp003WorldRoots['ring.rs']
  lp['baseline-allow'] = ['lp003WorldRoots/ring.rs→(none) 归零 D-999']
  const r = audit({ lp })
  assertRed(r, 'BS-005')
})

test('BS-005：allow 已落地（to 是当前机制面值）⇒ 红并报「请删除」', () => {
  const landed = cpBaseline()
  landed.mechanism.hash = 'sha256:new'
  landed['baseline-allow'] = ['sha256:old→sha256:new 机制面迁移，三段语义未变 D-321']
  // base 就是落地后的状态：本轮没有 diff 可命中，to 已等于当前 hash ⇒ 它是永久豁免，必须删。
  const r = audit({ cp: landed, baseCp: landed })
  assertRed(r, 'BS-005')
  assert.match(r.stdout + r.stderr, /已落地 allow，请删除/)
})

test('BS-005：清债 allow 已落地（键在当前基线里确实不存在）⇒ 红并报「请删除」', () => {
  const landed = lpBaseline()
  delete landed.lp003WorldRoots['ring.rs']
  landed['baseline-allow'] = ['lp003WorldRoots/ring.rs→(none) 该文件的世界词根已归零 D-321']
  const r = audit({ lp: landed, baseLp: landed })
  assertRed(r, 'BS-005')
  assert.match(r.stdout + r.stderr, /已落地 allow，请删除/)
})

test('BS-005：to 不等于当前值、相对 base 有变更并带 allow ⇒ 绿（在途豁免）', () => {
  const cp = cpBaseline()
  cp.mechanism.hash = 'sha256:new'
  cp['baseline-allow'] = ['sha256:old→sha256:new 驱动循环体随 D-321 迁移 D-321']
  // base 是旧值（`sha256:old`）：改动尚未落地 ⇒ 这条 allow 正在起作用。
  const r = audit({ cp })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

// ───────────────────────────── 读不出 ─────────────────────────────

test('基准版本读不出 ⇒ exit 2（不许在看不见变更时通过）', () => {
  const r = audit({ base: 'no-such-rev' })
  assert.equal(r.status, 2, r.stdout + r.stderr)
  assert.match(r.stdout + r.stderr, /范围读不出/)
})
