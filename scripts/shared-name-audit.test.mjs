/**
 * shared-name-audit 回归测试 —— 证明 SV-001 **会红**（D-364）
 *
 * 一个只会亮绿灯的守卫等于没有守卫。逐条注入真实违规并断言脚本变红：
 *   ① 干净夹具（两侧只 import）→ exit 0；
 *   ② 同一个有结构名字（`vdfs/write`）在两个插件各写一次字面量 → exit 1；
 *   ③ 同名 `const`（`task_table`）在两个插件各声明一次 → exit 1；
 *   ④ 标准 MIME 类型在两个插件都出现 → exit 0（语法形状不是词汇名）；
 *   ⑤ 测试文件里的重复 → exit 0（测试不是生产消费方）；
 *   ⑥ 范围读不出 → exit 2。
 *
 * 用法：node --test scripts/shared-name-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./shared-name-audit.mjs', import.meta.url))

/** 删临时目录 —— 逐个删，不用 rmSync(recursive)（沙箱会拦截批量删除）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

/** 造一个最小仓库：`crates/<插件>/src/<文件>`，内容由调用方给。 */
function fixture(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'shared-name-'))
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
  return root
}

function runOn(root) {
  return spawnSync(process.execPath, [script, `--root=${root}`], { encoding: 'utf8' })
}

test('① 干净夹具：两侧都 import，不写字面量 ⇒ 绿', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs':
      'use symbio_plugin_api::vocab::PORT_MODEL_ASSIST;\npub fn a() { let _ = PORT_MODEL_ASSIST; }\n',
    'crates/symbio-plugin-b/src/lib.rs':
      'use symbio_plugin_api::vocab::PORT_MODEL_ASSIST as P;\npub fn b() { let _ = P; }\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-001/)
  } finally {
    rmTree(root)
  }
})

test('② 同一个有结构名字在两个插件各写一次字面量 ⇒ 红', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'pub const A: &str = "vdfs/write";\n',
    'crates/symbio-plugin-b/src/lib.rs': 'pub fn b() -> &\'static str { "vdfs/write" }\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-001/)
    assert.match(r.stdout, /vdfs\/write/)
  } finally {
    rmTree(root)
  }
})

test('③ 同名 const 在两个插件各声明一次 ⇒ 红（无分隔符的名字也抓）', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'const TASK_TABLE: &str = "task_table";\n',
    'crates/symbio-plugin-b/src/lib.rs': 'pub(crate) const TASK_TABLE: &str = "task_table";\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-001/)
    assert.match(r.stdout, /task_table/)
  } finally {
    rmTree(root)
  }
})

test('④ 标准 MIME 类型在两个插件都出现 ⇒ 绿（语法形状不算词汇名）', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'pub const A: &str = "application/json";\n',
    'crates/symbio-plugin-b/src/lib.rs': 'pub fn b() -> &\'static str { "application/json" }\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑤ 测试文件里的重复 ⇒ 绿（测试不是生产消费方）', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'pub fn a() {}\n',
    'crates/symbio-plugin-a/src/foo.test.rs': 'pub const A: &str = "vdfs/write";\n',
    'crates/symbio-plugin-b/src/lib.rs': 'pub fn b() {}\n',
    'crates/symbio-plugin-b/src/bar.test.rs': 'pub const B: &str = "vdfs/write";\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑥ 范围读不出 ⇒ exit 2', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'shared-name-empty-'))
  try {
    const r = runOn(root)
    assert.equal(r.status, 2, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})
