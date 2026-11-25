/**
 * domain-vocab-audit 回归测试 —— 证明它**会红**
 *
 * 一个只会亮绿灯的守卫等于没有守卫。三条用例，各自钉一种失效：
 *   ① 干净夹具（无 `schemas/`、域间单向）→ exit 0；
 *   ② V-001：`schemas/` 目录重新出现 → exit 1（它是被拆掉的杂物抽屉）；
 *   ③ V-002：两个域互引 → exit 1；把这一族写进基线 → exit 0（基线只放行**登记过的**环）；
 *   ④ V-003：kernel 臂反向引用 sdk 臂 → exit 1（合并前这条由 cargo 依赖图挡住）。
 *
 * 夹具只需要 `symbio-sdk/src/lib.rs` + 各域 `mod.rs`：审计的口径是「域目录名 +
 * 生产文件里的 `crate::sdk::<域>::…` 路径」，两样都在最小夹具里可造。
 *
 * 用法：node --test scripts/audits/domain-vocab-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { withTempTree } from '../fs-walk.mjs'
import { auditCase, expectAudit } from '../audit-assert.mjs'

const script = fileURLToPath(new URL('./domain-vocab-audit.mjs', import.meta.url))


/** 建夹具 → 跑审计 → 返回 spawnSync 结果。`baseline` 默认空（不给任何环放行）。 */
function audit(files, { baseline = [] } = {}) {
  return withTempTree(files, (root) => {
    return spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1', CORE_VOCAB_BASELINE: JSON.stringify(baseline) },
      encoding: 'utf8',
    })
  }, { prefix: 'core-vocab-' })
}

const CORE = 'crates/symbio-platform/src/sdk'

/** 两个域、单向依赖 `beta → alpha` 的干净夹具。 */
function cleanTree() {
  return {
    [`${CORE}/lib.rs`]: `mod alpha;\nmod beta;\n\npub use alpha::Alpha;\npub use beta::Beta;\n`,
    [`${CORE}/alpha/mod.rs`]: `pub struct Alpha;\n`,
    [`${CORE}/beta/mod.rs`]: `use crate::sdk::alpha::Alpha;\n\npub struct Beta {\n    _a: Alpha,\n}\n`,
  }
}

auditCase('干净夹具：无 schemas、依赖单向 ⇒ exit 0', () => audit(cleanTree()), 0, [/✓ V-001/, /✓ V-002/])

test('V-001：schemas 目录重新出现 ⇒ exit 1', () => {
  const files = cleanTree()
  files[`${CORE}/schemas/mod.rs`] = `pub struct OldThing;\n`
  const r = audit(files)
  expectAudit(r, 1, [/V-001/, /schemas/])
})

test('V-002：两域互引 ⇒ exit 1（新环不在基线里）', () => {
  const files = cleanTree()
  // beta 已经用 Alpha；再让 alpha 用 Beta ⇒ alpha ⇄ beta
  files[`${CORE}/alpha/mod.rs`] = `use crate::sdk::beta::Beta;\n\npub struct Alpha {\n    _b: Option<Box<Beta>>,\n}\n`
  const r = audit(files)
  expectAudit(r, 1, [/V-002/, /alpha ⇄ beta/])
})

test('V-002：登记进基线的环放行（基线只放行登记过的）', () => {
  const files = cleanTree()
  files[`${CORE}/alpha/mod.rs`] = `use crate::sdk::beta::Beta;\n\npub struct Alpha {\n    _b: Option<Box<Beta>>,\n}\n`
  const r = audit(files, { baseline: ['alpha|beta'] })
  expectAudit(r, 0, [/✓ V-002/])
})

test('V-003：kernel 臂引用 sdk 臂 ⇒ exit 1（cargo 图不再免费挡住，靠判据）', () => {
  const files = cleanTree()
  files['crates/symbio-platform/src/kernel/mod.rs'] = `use crate::sdk::alpha::Alpha;\n\npub struct Kernel;\n`
  const r = audit(files)
  expectAudit(r, 1, [/V-003/, /kernel 臂不得依赖 sdk 臂/])
})

test('V-003：允许 sdk → kernel/contract、contract → kernel；注释里的反向引用不算', () => {
  const files = cleanTree()
  files['crates/symbio-platform/src/kernel/mod.rs'] = `//! 这里只是比较：[\`crate::sdk::alpha\`] 在宿主侧。\n\npub struct Kernel;\n`
  files['crates/symbio-platform/src/contract/mod.rs'] = `use crate::kernel::Kernel;\n`
  files[`${CORE}/keys.rs`] = `use crate::kernel::Kernel;\nuse crate::contract::Nothing;\n`
  const r = audit(files)
  expectAudit(r, 0, [/✓ V-003/])
})

test('注释里的路径不算边（否则文档链接会造假环）', () => {
  const files = cleanTree()
  // beta 已经用 alpha（真边）。这里只在**文档注释**里提到 beta：如果没剥注释，
  // 就会凭空造出「alpha → beta」的回边、当场成环。剥掉 ⇒ 只有单向边 ⇒ 绿。
  files[`${CORE}/alpha/mod.rs`] = `//! 与 [\`crate::sdk::beta::Beta\`] 的关系写在注释里。\n\npub struct Alpha;\n`
  const r = audit(files)
  expectAudit(r, 0, [/✓ V-002/])
})
