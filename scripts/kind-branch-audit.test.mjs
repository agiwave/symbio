/**
 * kind-branch-audit 回归测试 —— 证明它**会红**，且**不误报**
 *
 * 这个守卫的失效形态有两头：
 * ① **计数写宽**：一行多处比较只算一格、注释里的比较被计入、映射表没算 ⇒ 框架层
 *    目标 0 永远达标，GO-002 T4 的最后一块读数变成装饰；
 * ② **范围写窄**：漏掉 `.vue` / composable / schema 误入 ⇒ 分支藏在扫描面外。
 *
 * 覆盖：
 *   1. `countSites` 的口径：一行两处比较计 2、布尔原子只计形态那一侧、注释不计、
 *      `Record<NodeKind, …>` 计 1；
 *   2. 框架层超过基线 ⇒ KB-001 红；等于基线 ⇒ 绿；
 *   3. `schemas/` 是数据契约，不算分支；
 *   4. 范围读不出（没有 `tauri/src`）⇒ 红，不是静默通过。
 *
 * 用法：node --test scripts/kind-branch-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { countSites } from './kind-branch-audit.mjs'

const script = fileURLToPath(new URL('./kind-branch-audit.mjs', import.meta.url))

function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

function audit(files, { baseline = '0:1' } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'kind-branch-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1', KIND_BRANCH_BASELINE: baseline },
      encoding: 'utf8',
      timeout: 30_000,
    })
    assert.ifError(r.error)
    return r
  } finally {
    rmTree(root)
  }
}

const REG = 'tauri/src/registry/messageFacts.ts'
const REG_SRC = "export const isBodyKind = (kind: NodeKind) => kind === NODE_KIND_BODY\n"

test('countSites：一行两处比较计 2；布尔原子只计形态那一侧', () => {
  assert.equal(countSites('const x = kind === NODE_KIND_BODY || kind === NODE_KIND_NOTICE').length, 2)
  assert.equal(countSites("const x = kind === 'x' && type === NODE_KIND_BODY").length, 1)
  assert.equal(countSites('const x = nodeKindOf(t) !== NODE_KIND_SKELETON').length, 1)
})

test('countSites：注释不计，Record<NodeKind, …> 计 1', () => {
  assert.equal(countSites('// kind === NODE_KIND_BODY\n').length, 0)
  assert.equal(countSites('/* kind !== NODE_KIND_NOTICE */').length, 0)
  assert.equal(countSites('const table: Record<NodeKind, TypePresentation> = {}\n').length, 1)
})

test('countSites：资源类型分派与形态分派同计一格', () => {
  assert.equal(countSites('const x = ext === VDFS_EXT_SESSION').length, 1)
  assert.equal(countSites('const x = n.kind !== VDFS_KIND_INBOX').length, 1)
  assert.equal(countSites("const x = vdfsExtOf(n) === 'session'").length, 1)
  assert.equal(countSites("const x = renderer.value === 'form'").length, 1)
  assert.equal(countSites('const t: Record<string, VdfsRenderer> = {}').length, 1)
})

test('countSites：表单控件轴（.widget / .tag）不是资源分派', () => {
  assert.equal(countSites("const x = f.widget === 'form'").length, 0)
  assert.equal(countSites("const x = specOf(f).tag === 'form'").length, 0)
})

test('框架层与机制层都等于基线 ⇒ 通过', () => {
  const r = audit({ [REG]: REG_SRC }, { baseline: '0:1' })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('框架层自己认形态 ⇒ KB-001 变红', () => {
  const r = audit({
    [REG]: REG_SRC,
    'tauri/src/components/Foo.vue': '<script setup lang="ts">\nconst x = kind === NODE_KIND_BODY\n</script>\n',
  }, { baseline: '0:1' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stderr, /KB-001/)
  assert.match(r.stderr, /框架层分支.*实测 1 > 基线 0/)
})

test('框架层自己认资源类型 ⇒ KB-001 变红', () => {
  const r = audit({
    [REG]: REG_SRC,
    'tauri/src/services/session.ts': 'export const keep = (n) => n.ext === VDFS_EXT_SESSION\n',
  }, { baseline: '0:1' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stderr, /KB-001/)
  assert.match(r.stderr, /框架层分支.*实测 1 > 基线 0/)
})

test('反向：schemas/ 是数据契约，不算分支（多数会误报）', () => {
  const r = audit({
    [REG]: REG_SRC,
    'tauri/src/schemas/chat_node.ts': 'export const x = kind === NODE_KIND_BODY\n',
  }, { baseline: '0:1' })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('范围读不出（没有 tauri/src）⇒ 红，不是静默通过', () => {
  const r = audit({ 'docs/x.md': 'nothing\n' }, { baseline: '0:0' })
  assert.equal(r.status, 1)
  assert.match(r.stderr, /读不出前端范围/)
})
