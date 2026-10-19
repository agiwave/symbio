/**
 * core-vocab-audit 回归测试 —— 证明它**会红**
 *
 * 一个只会亮绿灯的守卫等于没有守卫。三条用例，各自钉一种失效：
 *   ① 干净夹具（无 `schemas/`、域间单向）→ exit 0；
 *   ② V-001：`schemas/` 目录重新出现 → exit 1（它是被拆掉的杂物抽屉）；
 *   ③ V-002：两个域互引 → exit 1；把这一族写进基线 → exit 0（基线只放行**登记过的**环）。
 *
 * 夹具只需要 `symbio_core/mod.rs` + 各域 `mod.rs`：审计的口径是「根出口符号的归属 +
 * 生产文件的 `crate::symbio_core::…` 路径」，两样都在最小夹具里可造。
 *
 * 用法：node --test scripts/core-vocab-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./core-vocab-audit.mjs', import.meta.url))

/** 逐个删 —— 不用 rmSync(recursive)（沙箱会拦截批量删除）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

/** 建夹具 → 跑审计 → 返回 spawnSync 结果。`baseline` 默认空（不给任何环放行）。 */
function audit(files, { baseline = [] } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'core-vocab-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    return spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1', CORE_VOCAB_BASELINE: JSON.stringify(baseline) },
      encoding: 'utf8',
    })
  } finally {
    rmTree(root)
  }
}

const CORE = 'symbio/src/symbio_core'

/** 两个域、单向依赖 `beta → alpha` 的干净夹具。 */
function cleanTree() {
  return {
    [`${CORE}/mod.rs`]: `mod alpha;\nmod beta;\n\npub use alpha::Alpha;\npub use beta::Beta;\n`,
    [`${CORE}/alpha/mod.rs`]: `pub struct Alpha;\n`,
    [`${CORE}/beta/mod.rs`]: `use crate::symbio_core::Alpha;\n\npub struct Beta {\n    _a: Alpha,\n}\n`,
  }
}

test('干净夹具：无 schemas、依赖单向 ⇒ exit 0', () => {
  const r = audit(cleanTree())
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /✓ V-001/)
  assert.match(r.stdout, /✓ V-002/)
})

test('V-001：schemas 目录重新出现 ⇒ exit 1', () => {
  const files = cleanTree()
  files[`${CORE}/schemas/mod.rs`] = `pub struct OldThing;\n`
  const r = audit(files)
  assert.equal(r.status, 1, r.stdout + r.stderr)
  assert.match(r.stdout, /V-001/)
  assert.match(r.stdout, /schemas/)
})

test('V-002：两域互引 ⇒ exit 1（新环不在基线里）', () => {
  const files = cleanTree()
  // beta 已经用 Alpha；再让 alpha 用 Beta ⇒ alpha ⇄ beta
  files[`${CORE}/alpha/mod.rs`] = `use crate::symbio_core::Beta;\n\npub struct Alpha {\n    _b: Option<Box<Beta>>,\n}\n`
  const r = audit(files)
  assert.equal(r.status, 1, r.stdout + r.stderr)
  assert.match(r.stdout, /V-002/)
  assert.match(r.stdout, /alpha ⇄ beta/)
})

test('V-002：登记进基线的环放行（基线只放行登记过的）', () => {
  const files = cleanTree()
  files[`${CORE}/alpha/mod.rs`] = `use crate::symbio_core::Beta;\n\npub struct Alpha {\n    _b: Option<Box<Beta>>,\n}\n`
  const r = audit(files, { baseline: ['alpha|beta'] })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /✓ V-002/)
})

test('注释里的路径不算边（否则文档链接会造假环）', () => {
  const files = cleanTree()
  // 只在文档注释里提到 Alpha：不该产生 beta → alpha 之外的任何边，
  // 更不该造出「alpha → beta」的回边。
  files[`${CORE}/alpha/mod.rs`] = `//! 与 [\`crate::symbio_core::Beta\`] 的关系写在注释里。\n\npub struct Alpha;\n`
  const r = audit(files)
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /✓ V-002/)
})
