/**
 * engine-ledger-audit 回归测试
 *
 * 这份守卫判的是**读数有没有变宽**（EL-001…003，判据见 DE-044 §3）。它的失效形态
 * 是这一族的老病：**读不出 ⇒ 没什么可判 ⇒ 通过**。所以三条码各配一条「多一处就红」的
 * 反向用例，另加两条输入失效用例——文件不在、结构体找不到，都必须 exit 1，不许亮绿灯。
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

const LOOP_ROWS = 'crates/symbio-plugin-loop/src/loop_rows.rs'
const DISPATCH = 'crates/symbio-plugin-loop/src/tool_dispatch.rs'
const GATES = 'crates/symbio-plugin-loop/src/tools/dispatch/gates.rs'
const PORT = 'crates/symbio-plugin-loop/src/port.rs'

/** 与基线等量的干净夹具：16 行 / 1 通道 / 11 处全量读。 */
function cleanFiles() {
  const decl = `    Decl {\n        name: "row",\n        prio: 1,\n    },\n`
  const rows = `pub(crate) const TABLE: &[Decl] = &[\n${decl.repeat(16)}];\n`
  const dispatch = `pub(crate) struct SessionDispatchPort {\n    a: u8,\n    out: Mutex<PortOutput>,\n}\n`
  const scan = `fn f() { let _ = store.range(Seq::new(0)); }\n`
  return {
    [LOOP_ROWS]: rows,
    [DISPATCH]: dispatch + scan.repeat(7),
    [GATES]: scan,
    [PORT]: `fn g() { let _ = store.range(symbio_core::Seq::new(0)); }\n`.repeat(3),
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

test('EL-001：多一行 Decl ⇒ 红，并指出变宽几格', () => {
  const files = cleanFiles()
  files[LOOP_ROWS] += `    Decl {\n        name: "extra",\n        prio: 2,\n    },\n`
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /EL-001/)
    assert.match(r.out, /16 → 17/)
  } finally {
    box.cleanup()
  }
})

test('EL-002：多一个 Mutex 字段 ⇒ 红', () => {
  const files = cleanFiles()
  files[DISPATCH] = files[DISPATCH].replace(
    '    out: Mutex<PortOutput>,',
    '    out: Mutex<PortOutput>,\n    extra: Mutex<Vec<u8>>,',
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
  files[GATES] += 'fn h() { let _ = store.range(symbio_core::Seq::new(0)); }\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /EL-003/)
    assert.match(r.out, /11 → 12/)
  } finally {
    box.cleanup()
  }
})

test('收窄不判红：少一行只提示可降基线', () => {
  const files = cleanFiles()
  const one = `    Decl {\n        name: "row",\n        prio: 1,\n    },\n`
  files[LOOP_ROWS] = files[LOOP_ROWS].replace(one, '')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /16 → 15/)
    assert.match(r.out, /可以顺手把基线降下来/)
  } finally {
    box.cleanup()
  }
})

test('输入失效：行表文件不在 ⇒ 红（不许「没什么可判 ⇒ 通过」）', () => {
  const files = cleanFiles()
  delete files[LOOP_ROWS]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /读数读不出/)
  } finally {
    box.cleanup()
  }
})

test('输入失效：结构体找不到 ⇒ 红（通道数不能默认成 0）', () => {
  const files = cleanFiles()
  files[DISPATCH] = files[DISPATCH].replace('SessionDispatchPort', 'SomethingElse')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /SessionDispatchPort/)
  } finally {
    box.cleanup()
  }
})
