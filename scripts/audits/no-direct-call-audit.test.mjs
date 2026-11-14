/**
 * no-direct-call-audit 回归测试 —— 证明它**会红**
 *
 * 一个只会亮绿灯的守卫等于没有守卫。本用例注入真实违规并断言脚本变红：
 *   ① 干净夹具 → exit 0；
 *   ② 运行时代码引用主体类型（NDC-001）→ exit 1；
 *   ③ 主体互持句柄 `Arc<Reasoner>`（NDC-002，连测试文件也拦）→ exit 1；
 *   ④ 定义域 / 根重导出 / 测试文件引用主体 → 放行（不误报）；
 *   ⑤ 注释里提到主体名 → 放行（文档 ≠ 对象图边）。
 *
 * 用法：node --test scripts/audits/no-direct-call-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { withTempTree } from '../fs-walk.mjs'

const script = fileURLToPath(new URL('./no-direct-call-audit.mjs', import.meta.url))


function audit(files) {
  return withTempTree(files, (root) => {
    const result = spawnSync(process.execPath, [script, `ROOT=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30_000,
    })
    assert.ifError(result.error)
    return result
  }, { prefix: 'no-direct-call-' })
}

const DEFINITION = 'pub struct Reasoner;\n'

test('干净夹具通过', () => {
  const r = audit({ 'symbio-sdk/src/actors/mod.rs': DEFINITION })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('运行时代码引用主体类型 → NDC-001 变红', () => {
  const r = audit({
    'symbio-sdk/src/actors/mod.rs': DEFINITION,
    'symbio-sdk/src/exec/driver.rs': 'use crate::symbio_sdk::actors::Reasoner;\nlet r: Reasoner = Reasoner;\n',
  })
  assert.equal(r.status, 1, '运行时代码拿到主体必须红')
  assert.match(r.stderr, /NDC-001/)
})

test('主体互持句柄 Arc<Reasoner> → NDC-002 变红（测试文件也不例外）', () => {
  const r = audit({
    'symbio-sdk/src/actors/mod.rs': 'pub struct Decider { peer: Arc<Reasoner> }\n' + DEFINITION,
    'symbio-sdk/src/actors/mod.test.rs': 'let shared = Arc::new(Reasoner);\n',
  })
  assert.equal(r.status, 1, '互持句柄必须红（协作只走事件）')
  assert.match(r.stderr, /NDC-002/)
  assert.equal(r.stderr.match(/NDC-002/g).length, 2, '定义域与测试文件各一处')
})

test('定义域 / 根重导出 / 测试引用 + 注释提及 → 放行', () => {
  const r = audit({
    'symbio-sdk/src/actors/mod.rs': DEFINITION,
    'symbio-sdk/src/actors/mod.test.rs': 'let r = Reasoner;\n',
    'symbio-sdk/src/lib.rs': 'pub use actors::Reasoner;\n',
    'symbio-sdk/src/adapters/mod.rs': '//! ② actors 的 Reasoner 是类型边（注释不算耦合）。\n',
  })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('定义域是整个 actors 模块树：域内新拆出的文件不被误报', () => {
  const r = audit({
    'symbio-sdk/src/actors/mod.rs': DEFINITION,
    'symbio-sdk/src/actors/reasoner.rs': 'impl Reasoner { fn reply() {} }\n',
    'symbio-sdk/src/actors/commitment.rs': 'pub struct CommitmentKeeper;\n',
  })
  assert.equal(r.status, 0, '按顶层类型拆出的域内文件是定义域的一部分：' + r.stderr + r.stdout)
})

test('定义域**之外**的 actors 相邻文件仍判 NDC-001（目录判据不得漏拦）', () => {
  const r = audit({
    'symbio-sdk/src/actors/mod.rs': DEFINITION,
    'symbio-sdk/src/actors_extra/foo.rs': 'let r = Reasoner;\n',
  })
  assert.equal(r.status, 1, 'actors_extra 不在定义域内，必须红')
  assert.match(r.stderr, /NDC-001/)
})
