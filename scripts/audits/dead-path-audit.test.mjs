/**
 * dead-path-audit 回归测试 —— 证明它**会红**（DR-001 / DR-002 / DR-003）
 *
 * 一个只会亮绿灯的守卫等于没有守卫。逐条注入真实违规并断言脚本变红：
 *   ① 干净夹具 → exit 0（写出来的 crate 名都在）；
 *   ② 代码里引用一个不存在的 crate → exit 1（DR-001）——D-279 那 186 处的形状；
 *   ③ 文档里只有一个**文件路径**指向不存在的 crate → exit 1（同一条，另一种形态）；
 *   ④ 拼错真实 crate 名（kernel → kernal）→ exit 1 —— 判据不问「是不是删掉的」，问「打不打得开」；
 *   ⑤ 决策记录里出现旧名且没有历史标记 → exit 1（DR-002）——把过去写成现在；
 *   ⑥ 同一行带「当时」→ exit 0 —— 写明了是历史就放行；
 *   ⑦ 文件级 `dead-path-history:` 标记 → exit 0 —— 决策正文的合理粒度；
 *   ⑧ 行内 `dead-path-allow:` → exit 0 —— 承认通道，且**空理由不算**（⑨）；
 *   ⑨ `dead-path-allow:` 后没写理由 → 仍 exit 1 —— 空理由等于没承认；
 *   ⑩ 缺 `crates/` → exit 2 —— 范围读不出必须红（DR-003，D-280）。
 *
 * 用法：node --test scripts/audits/dead-path-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { rmTree, withTempTree } from '../fs-walk.mjs'
import { expectAudit } from '../audit-assert.mjs'

const script = fileURLToPath(new URL('./dead-path-audit.mjs', import.meta.url))


/** 造一个最小仓库：`crates/symbio-kernel` + 若干给定文件（相对路径 → 内容）。 */
function fixture(files = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'dead-path-'))
  const base = {
    'crates/symbio-kernel/src/lib.rs': 'pub fn k() {}\n',
    'crates/symbio-plugin-api/src/lib.rs': 'pub fn api() {}\n',
  }
  for (const [rel, body] of Object.entries({ ...base, ...files })) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, body)
  }
  return root
}

function runOn(root) {
  return spawnSync(process.execPath, [script, `--root=${root}`], { encoding: 'utf8' })
}

test('① 干净夹具通过', () => {
  const root = fixture({
    'crates/symbio-sdk/src/lib.rs': '//! 见 `symbio_kernel::event::Event`。\n',
    'docs/006-guide/x.md': '用 `symbio_sdk::event_bus` 与 `crates/symbio-sdk/src/plugin/dir.rs`。\n',
  })
  try {
    const r = runOn(root)
    expectAudit(r, 0, [/DR-001 现行面/])
  } finally {
    rmTree(root)
  }
})

test('② 代码里引用不存在的 crate ⇒ 红（DR-001）', () => {
  const root = fixture({ 'crates/symbio-sdk/src/lib.rs': '//! 见 `symbio_gone::Thing`。\n' })
  try {
    const r = runOn(root)
    expectAudit(r, 1, [/symbio_gone/, /DR-001/])
  } finally {
    rmTree(root)
  }
})

test('③ 文档里只有文件路径指向不存在的 crate ⇒ 红（同一条判据的另一种形态）', () => {
  const root = fixture({ 'docs/006-guide/x.md': '定义在 `crates/symbio-gone/src/lib.rs`。\n' })
  try {
    const r = runOn(root)
    expectAudit(r, 1, [/symbio_gone/])
  } finally {
    rmTree(root)
  }
})

test('④ 拼错真实 crate 名 ⇒ 红（判据问「打不打得开」，不问「是不是删掉的」）', () => {
  const root = fixture({ 'crates/symbio-sdk/src/lib.rs': '//! 见 `symbio_kernal::event`。\n' })
  try {
    const r = runOn(root)
    expectAudit(r, 1, [/symbio_kernal/])
  } finally {
    rmTree(root)
  }
})

test('⑤ 决策记录里出现旧名且无历史标记 ⇒ 红（DR-002）', () => {
  const root = fixture({
    'docs/003-design/001-x.md': '# x\n\n权威定义是 `symbio_gone::Thing`。\n',
  })
  try {
    const r = runOn(root)
    expectAudit(r, 1, [/DR-002/, /symbio_gone/])
  } finally {
    rmTree(root)
  }
})

test('⑥ 同一行带「当时」⇒ 绿（写明了是历史）', () => {
  const root = fixture({
    'docs/003-design/001-x.md': '# x\n\n当时路径是 `symbio_gone::Thing`，D-279 之后并成两层。\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑦ 文件级 `dead-path-history:` 标记 ⇒ 绿（决策正文的合理粒度）', () => {
  const root = fixture({
    'docs/003-design/001-x.md':
      '# x\n\n<!-- dead-path-history: 本文件是决策记录，旧名按当时写 -->\n\n定义在 `symbio_gone::A` 与 `symbio_gone::B`。\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑧ 行内 `dead-path-allow:` 带理由 ⇒ 绿（承认通道）', () => {
  const root = fixture({
    'scripts/x.mjs': '// 讲清楚为什么删 `symbio_gone::X` [dead-path-allow: 判据自述必须能写出被禁的名字]\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑨ `dead-path-allow:` 后面没有理由 ⇒ 仍红（空理由等于没承认）', () => {
  const root = fixture({
    'scripts/x.mjs': '// 见 `symbio_gone::X` [dead-path-allow:]\n',
  })
  try {
    const r = runOn(root)
    expectAudit(r, 1, [/symbio_gone/])
  } finally {
    rmTree(root)
  }
})

test('⑩ 缺 `crates/` ⇒ exit 2（DR-003：范围读不出必须红）', () => {
  return withTempTree({}, (root) => {
    fs.mkdirSync(path.join(root, 'docs'))
    fs.writeFileSync(path.join(root, 'docs', 'x.md'), '空仓库\n')
    const r = runOn(root)
    expectAudit(r, 2, [/DR-003|范围读不出/])
  }, { prefix: 'dead-path-bare-' })
})
