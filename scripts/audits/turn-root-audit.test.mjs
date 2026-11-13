/**
 * turn-root-audit 回归测试
 *
 * 判的是「turn 词根存量有没有变宽」（TR-001…004，口径见 T0 冻结门禁的判据面）。这一族的失效
 * 形态是**读不出 ⇒ 没什么可判 ⇒ 通过**，所以四条红都必须有反向用例，且要钉住
 * 「红的是那一条码、那一面」——只断言 exit 1 会让四个判据互相掩盖。
 *
 * 另有一条**不该误红**的路径：基线缺失时不许当「无存量」放行。
 *
 * 跑法：node --test scripts/audits/turn-root-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./turn-root-audit.mjs', import.meta.url))

const CHAT_NODE = 'tauri/src/schemas/chat_node.ts'
const BASELINE_REL = 'scripts/turn-root-baseline.json'

/** 干净夹具：各面恰好等于基线（见 `cleanBaseline`）。 */
function cleanFiles() {
  return {
    // F1：`TurnState` + 裸 `turn` 字段 = 2（生产，本文件）+ main.rs 的 `turn:` = 1；测试面 2
    'crates/demo/src/lib.rs': 'pub struct TurnState {\n    turn: u64,\n}\n',
    'crates/demo/src/lib.test.rs': 'pub fn f() { let _ = TurnState { turn: 0 }; }\n',
    // F2：一条事实字面量 = 1
    'crates/demo/src/fact.rs': 'const EVENT: &str = "turn.stage";\n',
    // F3：Rust 命令层 DTO 字段 = 1
    'crates/symbio/src/main.rs': 'fn row() {\n    let _ = turn: 0;\n}\n',
    // F4：两处（迁移通道那一行也算前端词根）；F5：迁移通道 = 1
    [CHAT_NODE]: 'const LEGACY = { turn: NODE_KIND_SKELETON }\n',
    'tauri/src/services/row.ts': 'export interface Row {\n  turn: number\n}\n',
  }
}

function cleanBaseline() {
  return {
    target: 0,
    faces: {
      F1: { prod: 3, test: 2 },
      F2: { prod: 1, test: 0 },
      F3: { prod: 1, test: 0 },
      F4: { prod: 2, test: 0 },
      F5: { prod: 1, test: 0 },
    },
    allow: [
      {
        face: 'F5',
        file: CHAT_NODE,
        pattern: 'turn:\\s*NODE_KIND_SKELETON',
        reason: '存量数据迁移通道',
        until: '存量清迁',
      },
    ],
    pending: [],
  }
}

function sandbox(files, baseline = cleanBaseline()) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'turn-root-'))
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
  if (baseline !== null) {
    const abs = path.join(root, BASELINE_REL)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, JSON.stringify(baseline, null, 2))
  }
  return { root, cleanup: () => fs.rmSync(root, { recursive: true, force: true }) }
}

function run(root) {
  const r = spawnSync(process.execPath, [script, `--root=${root}`], {
    encoding: 'utf8',
    env: { ...process.env, NO_COLOR: '1' },
    timeout: 30_000,
  })
  return { status: r.status, out: `${r.stdout}${r.stderr}` }
}

test('干净夹具：五面与基线一致 ⇒ 通过', () => {
  const box = sandbox(cleanFiles())
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /F5/)
    assert.match(r.out, /5 面与基线一致/)
  } finally {
    box.cleanup()
  }
})

test('TR-001 新增：生产面多一处 turn 词根 ⇒ 红（不许无声变宽）', () => {
  const files = cleanFiles()
  files['crates/demo/src/extra.rs'] = 'pub struct TurnSpec;\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /TR-001/)
    assert.match(r.out, /F1 生产面新增：3 → 4/)
  } finally {
    box.cleanup()
  }
})

test('TR-002 只升不降：基线比实测松 ⇒ 红（收窄必须同批下调基线）', () => {
  const baseline = cleanBaseline()
  baseline.faces.F1.prod = 4
  const box = sandbox(cleanFiles(), baseline)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /TR-002/)
    assert.match(r.out, /F1 生产面收窄 4 → 3/)
  } finally {
    box.cleanup()
  }
})

test('TR-002 只升不降：删掉一处 turn 词根但基线不动 ⇒ 红', () => {
  const noFact = cleanFiles()
  delete noFact['crates/demo/src/fact.rs']
  const box = sandbox(noFact)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /TR-002/)
    assert.match(r.out, /F2 生产面收窄 1 → 0/)
  } finally {
    box.cleanup()
  }
})

test('TR-002 目标写死 0：基线把 target 改大 ⇒ 红（不许给退场设终点）', () => {
  const baseline = cleanBaseline()
  baseline.target = 5
  const box = sandbox(cleanFiles(), baseline)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /TR-002/)
    assert.match(r.out, /target 必须是 0/)
  } finally {
    box.cleanup()
  }
})

test('TR-003 空扫：整个范围读不出 ⇒ 红（不许「没什么可判 ⇒ 通过」）', () => {
  const files = cleanFiles()
  delete files['crates/demo/src/lib.rs']
  delete files['crates/demo/src/lib.test.rs']
  delete files['crates/demo/src/fact.rs']
  // F1 / F2 / F3 的扫描根整个读不出（不再是「实测 0」而是「一个文件都没扫到」）。
  delete files['crates/symbio/src/main.rs']
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /TR-003/)
    assert.match(r.out, /F1 空扫/)
  } finally {
    box.cleanup()
  }
})

test('TR-004 陈旧 allow：条目指向的文件不存在 ⇒ 红', () => {
  const baseline = cleanBaseline()
  baseline.allow[0].file = 'tauri/src/schemas/gone.ts'
  const box = sandbox(cleanFiles(), baseline)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /TR-004/)
    assert.match(r.out, /allow 指向的文件不存在/)
  } finally {
    box.cleanup()
  }
})

test('TR-004 陈旧 allow：文件在但写法已消失 ⇒ 红（迁移通道该转正了）', () => {
  const baseline = cleanBaseline()
  baseline.allow[0].pattern = 'turn:\\s*NODE_KIND_BODY'
  const box = sandbox(cleanFiles(), baseline)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /TR-004/)
    assert.match(r.out, /allow 的写法已不存在/)
  } finally {
    box.cleanup()
  }
})

test('TR-004 隐式豁免：迁移通道在、allow 没登记 ⇒ 红', () => {
  const baseline = cleanBaseline()
  baseline.allow = []
  const box = sandbox(cleanFiles(), baseline)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /TR-004/)
    assert.match(r.out, /没有登记/)
  } finally {
    box.cleanup()
  }
})

test('基线缺失 ⇒ 红（不许把「读不出」当成「无存量」）', () => {
  const box = sandbox(cleanFiles(), null)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /读不出基线/)
  } finally {
    box.cleanup()
  }
})
