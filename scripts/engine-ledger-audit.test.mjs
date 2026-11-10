/**
 * engine-ledger-audit 回归测试
 *
 * 这份守卫判的是**读数有没有变宽**（EL-001…003，判据见 DE-044 §3）。它的失效形态
 * 是这一族的老病：**读不出 ⇒ 没什么可判 ⇒ 通过**。所以三条码各配一条「多一处就红」的
 * 反向用例，另加两条输入失效用例——扫描范围不在、结构体找不到，都必须 exit 1，不许亮绿灯。
 *
 * 另有一条**不该红**的用例：读数**收窄**时通过（只提示可降基线）。等于会把收窄也判红，
 * 于是改基线成了常规动作，棘轮就失效了——这条用例钉的是方向只留一个。
 *
 * 跑法：node --test scripts/engine-ledger-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./engine-ledger-audit.mjs', import.meta.url))

// 读数面：环源码（EL-001）、环的收集器（EL-002）、驱动 / 回合路径（EL-003）。
const LOOP_SRC = 'crates/symbio-plugin-loop/src'
const TABLE = 'crates/symbio-plugin-loop/src/ring/table.rs'
const ENGINE = 'crates/symbio-plugin-loop/src/ring/engine.rs'
const DRIVE = 'crates/symbio/src/drive.rs'
const TURN = 'crates/symbio-plugin-session/src/plugin/turn.rs'

/** 与基线等量的干净夹具：0 行 / 1 通道 / 0 处全量读。 */
function cleanFiles() {
  const decl = `    Decl {\n        name: "row",\n        prio: 1,\n    },\n`
  return {
    [TABLE]: 'pub(crate) struct DebtRow {\n    pub(crate) name: &\'static str,\n}\n',
    [ENGINE]:
      'struct CollectingWriter {\n    outlet: PortFactEmit,\n    collected: Mutex<Vec<FactDraft>>,\n}\n',
    [DRIVE]: 'fn d() { let _ = facts.range(Seq::new(head - 1)); }\n',
    // 续读不带全量：基线已收到 0，夹具必须同档，否则「干净夹具」本身就成了变宽用例。
    [TURN]: 'fn f() { let _ = facts.range(Seq::new(head)); }\n',
    // 夹具里的行常量：证明 `Decl {` 只有在 loop 源码里才进读数。
    'crates/symbio-plugin-session/src/rows.rs': `pub(crate) const TABLE: &[Decl] = &[\n${decl}];\n`,
  }
}

function sandbox(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'engine-ledger-'))
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
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

test('干净夹具：三格与基线一致 ⇒ 通过', () => {
  const box = sandbox(cleanFiles())
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /EL-001/)
    assert.match(r.out, /3 格与基线一致/)
  } finally {
    box.cleanup()
  }
})

test('EL-001：loop 里多一行 Decl ⇒ 红（引擎不再自持行表）', () => {
  const files = cleanFiles()
  files[TABLE] += `    Decl {\n        name: "extra",\n        prio: 2,\n    },\n`
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /EL-001/)
    assert.match(r.out, /0 → 1/)
  } finally {
    box.cleanup()
  }
})

test('EL-001 判定面：别的 crate 里的 `Decl {` 不进读数', () => {
  const files = cleanFiles()
  files['crates/symbio-plugin-session/src/rows.rs'] += `    Decl {\n        name: "other",\n    },\n`
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /EL-001 引擎自持的行数：0/)
  } finally {
    box.cleanup()
  }
})

test('EL-002：多一个 Mutex 字段 ⇒ 红', () => {
  const files = cleanFiles()
  files[ENGINE] = files[ENGINE].replace(
    '    collected: Mutex<Vec<FactDraft>>,',
    '    collected: Mutex<Vec<FactDraft>>,\n    extra: Mutex<Vec<u8>>,',
  )
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /EL-002/)
    assert.match(r.out, /1 → 2/)
  } finally {
    box.cleanup()
  }
})

test('EL-003：多一处全量读 ⇒ 红（两种写法都算）', () => {
  const files = cleanFiles()
  files[DRIVE] += 'fn h() { let _ = facts.range(symbio_sdk::Seq::new(0)); }\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /EL-003/)
    assert.match(r.out, /0 → 1/)
  } finally {
    box.cleanup()
  }
})

test('收窄不判红：少一条通道只提示可降基线', () => {
  const files = cleanFiles()
  files[ENGINE] = files[ENGINE].replace('    collected: Mutex<Vec<FactDraft>>,\n', '')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /1 → 0/)
    assert.match(r.out, /可以顺手把基线降下来/)
  } finally {
    box.cleanup()
  }
})

test('输入失效：环源码整体不在 ⇒ 红（不许「没什么可判 ⇒ 通过」）', () => {
  const files = cleanFiles()
  delete files[TABLE]
  delete files[ENGINE]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /读数读不出/)
  } finally {
    box.cleanup()
  }
})

test('输入失效：CollectingWriter 结构体找不到 ⇒ 红（通道数不能默认成 0）', () => {
  const files = cleanFiles()
  files[ENGINE] = files[ENGINE].replace('CollectingWriter', 'SomethingElse')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /CollectingWriter/)
  } finally {
    box.cleanup()
  }
})

test('输入失效：扫描文件缺失 ⇒ 红（全量读数不能少读一处当通过）', () => {
  const files = cleanFiles()
  delete files[TURN]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /读数读不出/)
  } finally {
    box.cleanup()
  }
})
