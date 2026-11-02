/**
 * loop-purity-audit 回归测试
 *
 * 判的是「环有没有悄悄把世界语义拿回来」（LP-001…005，口径见 `DE-047` §4）。
 * 这一族的失效形态是**读不出 ⇒ 没什么可判 ⇒ 通过**，所以反向用例必须钉住
 * 「红的是哪一条码」，而不是只断言 exit 1。
 *
 * 跑法：node --test scripts/loop-purity-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./loop-purity-audit.mjs', import.meta.url))
const BASELINE_REL = 'scripts/loop-purity-baseline.json'
const CARGO = 'crates/symbio-plugin-loop/Cargo.toml'
const LIB = 'crates/symbio-plugin-loop/src/lib.rs'
const API_PORT = 'crates/symbio-plugin-api/src/port.rs'

function cleanFiles() {
  return {
    [CARGO]: '[package]\nname = "symbio-plugin-loop"\n\n[dependencies]\nsymbio-kernel = { path = "../symbio-kernel" }\nsymbio-plugin-api = { path = "../symbio-plugin-api" }\n',
    [LIB]: 'pub fn ok() -> u8 {\n    1\n}\n',
    [API_PORT]:
      'pub trait PortProvider: Send + Sync {\n    fn ports(&self) -> &\'static [&\'static str];\n}\n\npub trait PortExecutor: Send + Sync {\n    async fn execute(&self, input: u64) -> u64;\n}\n',
  }
}

function cleanBaseline() {
  return {
    lp001PayloadImports: {},
    lp002ForbiddenDeps: [],
    lp003WorldRoots: {},
    lp004WorldNamedContracts: 0,
    lp005WorldPayloadSig: 0,
  }
}

function sandbox(files, baseline = cleanBaseline()) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'loop-purity-'))
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

test('干净夹具：五面为 0 且与基线一致 ⇒ 通过', () => {
  const box = sandbox(cleanFiles())
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /LP-001/)
  } finally {
    box.cleanup()
  }
})

test('LP-001 新增：loop 生产源码 import 端口载荷类型 ⇒ 红', () => {
  const files = cleanFiles()
  files[LIB] = 'use symbio_plugin_api::port::{PortPayload, ToolCallRequest};\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-001/)
    assert.match(r.out, /lib\.rs 0 → 2/)
  } finally {
    box.cleanup()
  }
})

test('LP-002 新增：Cargo 清单多一条 provider crate ⇒ 红', () => {
  const files = cleanFiles()
  files[CARGO] = files[CARGO].replace(
    'symbio-plugin-api = { path = "../symbio-plugin-api" }',
    'symbio-plugin-api = { path = "../symbio-plugin-api" }\nsymbio-provider-memory = { path = "../symbio-provider-memory" }',
  )
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-002/)
    assert.match(r.out, /symbio-provider-memory/)
  } finally {
    box.cleanup()
  }
})

test('LP-002 放行机制门面 symbio-plugin-api', () => {
  const box = sandbox(cleanFiles())
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('LP-003 新增：loop 生产源码出现 batch 词根 ⇒ 红', () => {
  const files = cleanFiles()
  files[LIB] = 'pub fn run_batch() -> u8 {\n    0\n}\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-003/)
    assert.match(r.out, /lib\.rs 0 → 1/)
  } finally {
    box.cleanup()
  }
})

test('LP-003：注释里的词不算（注释由 D-016 管）', () => {
  const files = cleanFiles()
  files[LIB] = '// 这里曾经是一个 batch / gate 的世界\npub fn ok() -> u8 {\n    1\n}\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('LP-004 新增：端口受理面出现世界具名的类型 ⇒ 红', () => {
  const files = cleanFiles()
  files[API_PORT] = `${files[API_PORT]}\npub struct ModelAsk;\n`
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-004/)
    assert.match(r.out, /0 → 1/)
  } finally {
    box.cleanup()
  }
})

test('LP-004：注释里的世界词不算（名字才是接口事实）', () => {
  const files = cleanFiles()
  files[API_PORT] =
    '// 这个文件曾经有 tool / model 具名方法\n' + files[API_PORT]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('LP-005 新增：端口 trait 签名里出现具名世界载荷 ⇒ 红', () => {
  const files = cleanFiles()
  files[API_PORT] =
    'pub trait PortProvider: Send + Sync {\n    fn ports(&self) -> &\'static [&\'static str];\n    fn model(&self) -> Option<ModelProvider>;\n}\n\npub trait PortExecutor: Send + Sync {\n    async fn execute(&self, input: u64) -> u64;\n}\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /LP-005/)
  } finally {
    box.cleanup()
  }
})

test('LP-005：两个端口 trait 找不到 ⇒ exit 2（读不出就不亮绿灯）', () => {
  const files = cleanFiles()
  files[API_PORT] = 'pub fn ports() -> u8 {\n    0\n}\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 2, r.out)
    assert.match(r.out, /PortProvider/)
  } finally {
    box.cleanup()
  }
})

test('端口受理面文件缺失 ⇒ exit 2（不许当「没有端口接口」放行）', () => {
  const files = cleanFiles()
  delete files[API_PORT]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 2, r.out)
    assert.match(r.out, /端口受理面/)
  } finally {
    box.cleanup()
  }
})

test('收窄未同批下调基线 ⇒ 红（棘轮必须跟着收紧走）', () => {
  const baseline = { lp001PayloadImports: {}, lp002ForbiddenDeps: [], lp003WorldRoots: { 'lib.rs': 1 } }
  const box = sandbox(cleanFiles(), baseline)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /收窄未同批下调基线/)
  } finally {
    box.cleanup()
  }
})

test('基线缺失 ⇒ exit 2（不许当「无存量」放行）', () => {
  const box = sandbox(cleanFiles(), null)
  try {
    const r = run(box.root)
    assert.equal(r.status, 2, r.out)
    assert.match(r.out, /基线/)
  } finally {
    box.cleanup()
  }
})

test('源目录缺失 ⇒ exit 2（读不出就不亮绿灯）', () => {
  const files = cleanFiles()
  delete files[LIB]
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 2, r.out)
    assert.match(r.out, /源目录/)
  } finally {
    box.cleanup()
  }
})
