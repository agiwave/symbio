/**
 * doc-link-audit 回归测试 —— 证明它**会红**
 *
 * 补它的两个理由：① 它是 `gate.mjs` 声称"每个判定型守卫都先跑回归测试"里**缺的那个**
 * （2026-09-20 复核发现 8 个里缺 3 个）；② 它原先只有 `--strict` 才失败，而门禁从不带
 * 该参数 ⇒ **它从未真的红过**。后者已一并修掉（失效链接是精确判定，没有"疑似"中间态）。
 *
 * 用法：node --test scripts/doc-link-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./doc-link-audit.mjs', import.meta.url))

/** 脚本要求 5 个扫描根都存在（缺了会以「找不到扫描根」退出 1），故夹具先铺空目录 */
const SCAN_ROOTS = ['docs', 'symbio/src', 'tauri', 'cli', 'examples']

function audit(files, argv = []) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-link-audit-'))
  try {
    for (const d of SCAN_ROOTS) fs.mkdirSync(path.join(root, d), { recursive: true })
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(process.execPath, [script, `--root=${root}`, ...argv], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 20_000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

// ── 命中：指向不存在的文件 ──────────────────────────────────────────────
test('失效链接 → 失败（默认即失败，不再需要 --strict）', () => {
  const r = audit({ 'docs/a.md': '[去这儿](./nope.md)\n' })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /nope\.md/)
})

test('失效链接：带锚点也要判（先把 # 后段剥掉再查文件）', () => {
  const r = audit({ 'docs/a.md': '[去这儿](./nope.md#section)\n' })
  assert.equal(r.status, 1)
})

test('失效链接：上级目录的相对路径同样判', () => {
  const r = audit({ 'docs/sub/a.md': '[上级](../../nope.md)\n' })
  assert.equal(r.status, 1)
})

// ── 不误报 ──────────────────────────────────────────────────────────────
test('有效链接 → 通过', () => {
  const r = audit({
    'docs/a.md': '[去这儿](./b.md)\n',
    'docs/b.md': '# B\n',
  })
  assert.equal(r.status, 0)
})

test('外链 / 纯锚点 / 空目标一律跳过（不是失效链接）', () => {
  const r = audit({
    'docs/a.md':
      '[外链](https://example.com/x)\n[锚点](#section)\n[邮件](mailto:a@b.c)\n[空]()\n',
  })
  assert.equal(r.status, 0)
})

test('docs/archive/ 整体豁免 → 其中的失效链接不判（改写归档等于篡改历史）', () => {
  const r = audit({ 'docs/archive/old.md': '[当年的兄弟文档](./gone.md)\n' })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /豁免/)
})

// ── 空树 ────────────────────────────────────────────────────────────────
test('空树通过（守卫不是空转即红）', () => {
  assert.equal(audit({}).status, 0)
})

// ── 真实仓库当前状态 ────────────────────────────────────────────────────
test('真实仓库当前状态通过（防本守卫在真仓库上误报）', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: path.resolve(path.dirname(script), '..'),
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 60_000,
  })
  assert.ifError(r.error)
  assert.equal(r.status, 0, r.stdout)
})
