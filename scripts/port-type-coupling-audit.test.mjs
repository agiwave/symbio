/**
 * port-type-coupling-audit 回归测试
 *
 * 判据面（PTC-001/002，见 D-447 ② 与 2026-11-11 team-lead 边界修正）：
 *  - PTC-001：三个通用端口契约文件不得出现 `PromptMessage`（含 `Vec<PromptMessage>`）；
 *  - PTC-002：四个通用端口契约文件不得出现 `needs_wording_generation`；
 *    **不扫整个 `crates/symbio-sdk/src/`**——`sdk/dialog` 的 `Verdict::needs_wording_generation`
 *    是会话域判定（消费方 compose / session），本轮不迁出（行为闭包迁移，报告标未闭合），
 *    故在扫描面外。收窄的是判据边界，不是放宽形状判据。
 *
 * 这一族守卫最危险的失效是**假绿**：关键词被写进注释就当它不存在、扫不到输入就当
 * 通过、或把不在扫描面内的会话域判定误判成红。夹具钉：
 *
 *   ① 干净树 ⇒ 绿；② `PortReply { messages: Vec<PromptMessage> }` ⇒ 红并点名 `文件:行号`；
 *   ③ PTC-001 三个面各注入都红；④ 通用端口契约里出现 `needs_wording_generation` ⇒ 红并点名；
 *   ⑤ 四个契约文件任一命中都红；⑥ `sdk/dialog` 里出现该符号 ⇒ 不算违规、仍绿；
 *   ⑦ 注释叙述不算耦合；⑧ 「靠注释声明已删除」骗不过真实代码行；⑨ 读不出 ⇒ 红。
 *
 * 夹具树用临时目录 + 脚本的 `--root=<dir>`（与 engine-ledger-audit.test.mjs 同约定），
 * 不碰真实 crates。
 *
 * 跑法：node --test scripts/port-type-coupling-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./port-type-coupling-audit.mjs', import.meta.url))

const PORT_RS = 'crates/symbio-sdk/src/capability/port.rs'
const DRIVE_RS = 'crates/symbio-plugin-api/src/drive.rs'
const FACTS_RS = 'crates/symbio-plugin-loop/src/ring/facts.rs'
const API_PORT_RS = 'crates/symbio-plugin-api/src/port.rs'
const SDK_DIALOG = 'crates/symbio-sdk/src/dialog/mod.rs'
const SDK_LIB = 'crates/symbio-sdk/src/lib.rs'

/** PTC-001 扫描面：三个通用端口契约文件。 */
const PTC001_FILES = [PORT_RS, DRIVE_RS, FACTS_RS]
/** PTC-002 扫描面：同一份表 + plugin-api 的 port.rs（共四个）。 */
const PTC002_FILES = [...PTC001_FILES, API_PORT_RS]

/** 干净夹具：四个契约文件都干净；sdk/dialog 是会话域代码，不在扫描面内。 */
function cleanFiles() {
  return {
    [PORT_RS]: [
      'use std::sync::Arc;',
      '',
      '/// 通用端口契约：应答是不透明承载，环只按序追加、按 links 配对。',
      'pub struct PortReply {',
      '    pub links: Vec<(String, String)>,',
      '}',
      '',
    ].join('\n'),
    [DRIVE_RS]: ['pub fn drive(rows: Vec<String>) -> usize { rows.len() }', ''].join('\n'),
    [FACTS_RS]: ['pub const RING: &str = "ring:";', ''].join('\n'),
    [API_PORT_RS]: [
      '/// plugin-api 端口受理面：只认端口键数据。',
      'pub fn port_row_name(port: &str, provider: &str) -> String { format!("<port:{port}>:{provider}") }',
      '',
    ].join('\n'),
    [SDK_DIALOG]: [
      'pub enum Verdict { Report, Answer }',
      'impl Verdict {',
      '    pub fn is_report(&self) -> bool { matches!(self, Verdict::Report) }',
      '}',
      '',
    ].join('\n'),
    [SDK_LIB]: 'pub mod dialog;\n',
  }
}

function sandbox(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ptc-audit-'))
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
  assert.ifError(r.error)
  return { status: r.status, out: `${r.stdout}${r.stderr}` }
}

test('干净夹具树：四个通用端口契约都干净 ⇒ 通过', () => {
  const box = sandbox(cleanFiles())
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /通过/)
  } finally {
    box.cleanup()
  }
})

test('PTC-001：PortReply { messages: Vec<PromptMessage> } ⇒ 红并点名 文件:行号', () => {
  const files = cleanFiles()
  files[PORT_RS] = [
    'pub struct PortReply {',
    '    pub messages: Vec<PromptMessage>,',
    '    pub links: Vec<(String, String)>,',
    '}',
    '',
  ].join('\n')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /PTC-001/)
    assert.match(r.out, /port\.rs:2/)
    assert.match(r.out, /Vec<PromptMessage>/)
  } finally {
    box.cleanup()
  }
})

test('PTC-001 判定面：三个通用端口契约文件任一出现 PromptMessage 都判红', () => {
  for (const target of PTC001_FILES) {
    const files = cleanFiles()
    files[target] += 'fn stray() { let _x: Vec<PromptMessage> = Vec::new(); }\n'
    const box = sandbox(files)
    try {
      const r = run(box.root)
      assert.equal(r.status, 1, r.out)
      assert.match(r.out, /PTC-001/)
      assert.ok(r.out.includes(target), `输出没有点名 ${target}：\n${r.out}`)
    } finally {
      box.cleanup()
    }
  }
})

test('PTC-002：通用端口契约里出现 needs_wording_generation ⇒ 红并点名 文件:行号', () => {
  const files = cleanFiles()
  files[PORT_RS] += 'fn stray(&self) -> bool { self.needs_wording_generation() }\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /PTC-002/)
    assert.match(r.out, /port\.rs:\d+/)
    assert.match(r.out, /needs_wording_generation/)
  } finally {
    box.cleanup()
  }
})

test('PTC-002 判定面：四个通用端口契约文件任一出现该符号都判红', () => {
  for (const target of PTC002_FILES) {
    const files = cleanFiles()
    files[target] += 'fn stray(&self) -> bool { self.needs_wording_generation() }\n'
    const box = sandbox(files)
    try {
      const r = run(box.root)
      assert.equal(r.status, 1, r.out)
      assert.match(r.out, /PTC-002/)
      assert.ok(r.out.includes(target), `输出没有点名 ${target}：\n${r.out}`)
    } finally {
      box.cleanup()
    }
  }
})

test('PTC-002 边界：sdk/dialog 里出现 needs_wording_generation 不算违规（扫描面外、未闭合项）⇒ 通过', () => {
  const files = cleanFiles()
  // `Verdict::needs_wording_generation` 是会话域判定，消费方 compose / session，本轮不迁出。
  files[SDK_DIALOG] = [
    'pub enum Verdict { Report, Answer }',
    'impl Verdict {',
    '    pub fn needs_wording_generation(&self) -> bool { true }',
    '}',
    '',
  ].join('\n')
  files['crates/symbio-sdk/src/dialog/mod.test.rs'] =
    '#[test]\nfn t() { assert!(Verdict::Report.needs_wording_generation()); }\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /通过/)
  } finally {
    box.cleanup()
  }
})

test('注释叙述不算耦合：只在 doc comment 里提到 PromptMessage、无真实代码 ⇒ 通过', () => {
  const files = cleanFiles()
  files[PORT_RS] = [
    '/// 迁移说明：模型协议类型 PromptMessage 已移出通用端口（D-447 ②）。',
    'pub struct PortReply {',
    '    pub links: Vec<(String, String)>,',
    '}',
    '',
  ].join('\n')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('「靠注释声明已删除」骗不过真实代码行：注释 + Vec<PromptMessage> ⇒ 仍红', () => {
  const files = cleanFiles()
  files[PORT_RS] = [
    '/// PromptMessage 已删除（本行只是注释声明）。',
    'pub struct PortReply {',
    '    pub messages: Vec<PromptMessage>,',
    '}',
    '',
  ].join('\n')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /PTC-001/)
    assert.match(r.out, /port\.rs:3/)
  } finally {
    box.cleanup()
  }
})

test('输入失效：空夹具树（0 个具名契约文件）⇒ 红，不退化成绿灯', () => {
  const box = sandbox({})
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /读不出/)
  } finally {
    box.cleanup()
  }
})
