/**
 * slo-exit-audit 回归测试
 *
 * 这条守卫读两个来源：`04 §1.3` 出口表的「用户可见出口」格，与 `stats.rs` 的读数列
 * 集合。它最危险的失效形态与 `doc-count-audit` 同族而更隐蔽——**列名是我在代码里
 * 用结构体字段名提的，而不是手上的一份清单**：提取器一旦漏提一列，表里那一条指认
 * 就会被判成「不存在」而红（安全的一侧）；反过来若退化成硬编码列名，加列时表与
 * 守卫会**一起说谎**（都说不存在）而全绿。
 *
 * 所以这组用例钉四件事：
 *
 *   ① **抓得住**：表里指认一个不存在的列 ⇒ 红；
 *   ② **读不出即失败**：出口表没有那一列 / `stats.rs` 提不出字段 ⇒ exit 2；
 *   ③ **`—` 不是违规**：整表都是 `—` 时不得假红（否则守卫会被驯化成「别填了」）；
 *   ④ **点号子列**：`tiers.p99` 这类指认可解析，且随 `TierRow` 的字段集合长。
 *
 * 跑法：node --test scripts/slo-exit-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./slo-exit-audit.mjs', import.meta.url))
const PLAN04 = path.join('docs', '004-plan', '004-工程落地.md')
const STATS = path.join('symbio', 'src', 'plugins', 'session', 'stats.rs')

/** `stats.rs` 的最小夹具：`SessionStats` 顶层列 + `TierRow` 子列 */
const STATS_FIXTURE = [
  '#[derive(Serialize)]',
  'pub(crate) struct SessionStats {',
  '    pub session_id: String,',
  '    pub has_wal: bool,',
  '    pub tiers: Vec<TierRow>,',
  '    pub invariants: serde_json::Value,',
  '}',
  '',
  '#[derive(Serialize)]',
  'pub(crate) struct TierRow {',
  '    pub tier: String,',
  '    pub p99: u64,',
  '}',
].join('\n')

/** 一张形状正确的出口表（5 列，最后一列是新列） */
function table(exitCells) {
  const rows = ['S01', 'S02', 'S03'].map((s, i) => `| ${s} | 某指标 | 达标 | 埋点 | ${exitCells[i]} |`)
  return [
    '## 1.3 逐阶出口 SLO',
    '',
    '| 阶 | 出口指标 | 初值目标 | 测量方式 | 用户可见出口 |',
    '|---|---|---|---|---|',
    ...rows,
  ].join('\n')
}

function audit({ docBody = table(['`invariants`', '—', '—']), stats = STATS_FIXTURE } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slo-exit-'))
  try {
    const write = (rel, content) => {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    write(PLAN04, docBody)
    write(STATS, stats)
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

// ── ① 抓得住 ──────────────────────────────────────────────────────────

test('指认一列真实存在的列 ⇒ 通过', () => {
  const r = audit()
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('指认不存在的列 ⇒ SX-001 红', () => {
  const r = audit({ docBody: table(['`invariants`', '`no_such_column`', '—']) })
  assert.equal(r.status, 1, '指认漂移必须红：' + r.stdout)
  assert.match(r.stdout, /SX-001/)
  assert.match(r.stdout, /no_such_column/)
})

test('列被改名后旧指认必须红（这是守卫存在的理由）', () => {
  // 代码里 `calibration` 改名成 `calib`：表还写着旧名 ⇒ 必须红。
  // 这条是这个守卫唯一的存在理由——没有它，加列/改列时表与读数会一起说谎。
  const withOld = STATS_FIXTURE.replace(
    '    pub invariants: serde_json::Value,',
    '    pub invariants: serde_json::Value,\n    pub calibration: serde_json::Value,',
  )
  const renamed = withOld.replace('pub calibration:', 'pub calib:')
  const r = audit({ docBody: table(['`calibration`', '—', '—']), stats: renamed })
  assert.equal(r.status, 1, '旧名指认在改名后必须红：' + r.stdout)
  assert.match(r.stdout, /SX-001/)
  // 同一张表指着新名时是绿的（证明红的是「名字对不上」，不是这一行有问题）
  const ok = audit({ docBody: table(['`calib`', '—', '—']), stats: renamed })
  assert.equal(ok.status, 0, ok.stdout)
})

// ── ② 读不出即失败 ────────────────────────────────────────────────────

test('出口表没有那一列 ⇒ exit 2（读不出规则源，不退化成绿灯）', () => {
  const r = audit({
    docBody: ['## 1.3', '', '| 阶 | 出口指标 |', '|---|---|', '| S01 | 某指标 |'].join('\n'),
  })
  assert.equal(r.status, 2, r.stdout)
  assert.match(r.stdout, /SX-002|SX-003/)
})

test('出口表整张不在 ⇒ exit 2', () => {
  const r = audit({ docBody: '# 没有表\n' })
  assert.equal(r.status, 2, r.stdout)
})

test('stats.rs 提不出字段 ⇒ exit 2', () => {
  const r = audit({
    stats: ['// 结构体被改名了', 'pub(crate) struct Readings { pub a: u8 }'].join('\n'),
  })
  assert.equal(r.status, 2, r.stdout)
  assert.match(r.stdout, /SX-002/)
})

test('stats.rs 不在 ⇒ exit 2', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'slo-exit-'))
  try {
    fs.mkdirSync(path.dirname(path.join(root, PLAN04)), { recursive: true })
    fs.writeFileSync(path.join(root, PLAN04), table(['`invariants`', '—', '—']))
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.equal(r.status, 2, r.stdout)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

// ── ③ `—` 不是违规 ────────────────────────────────────────────────────

test('整表都是 `—` ⇒ 通过（不得假红，否则没人再填这一列）', () => {
  const r = audit({ docBody: table(['—', '—', '—']) })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /3 行\*\*没有\*\*用户可见读数/)
})

test('`-` / 全角破折号也算「没有出口」', () => {
  const r = audit({ docBody: table(['-', '－', '']) })
  assert.equal(r.status, 0, r.stdout)
})

// ── ④ 点号子列 ────────────────────────────────────────────────────────

test('点号指认随 TierRow 的字段集合长', () => {
  const r = audit({
    docBody: table(['`tiers.p99`', '`tiers.tier`', '`tiers.nope`']),
  })
  assert.equal(r.status, 1, '前两条该过、第三条该红：' + r.stdout)
  assert.match(r.stdout, /tiers\.nope/)
  assert.doesNotMatch(r.stdout, /tiers\.p99`，/)
})

test('一行里多个指认可解析（顿号 / 分号 / 逗号都算分隔）', () => {
  const r = audit({
    docBody: table(['`invariants`、`tiers.p99`', '`session_id`, `has_wal`；`tiers.tier`', '—']),
  })
  assert.equal(r.status, 0, r.stdout)
})
