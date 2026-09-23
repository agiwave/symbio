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

// ── D-002：过程文档必须归档 ────────────────────────────────────────────
// 归档是动作，能保持住的才是机制。这组用例钉住「它真的会红」——包括**豁免必须有理由**，
// 否则加一行注释就能把规则绕成橡皮图章。
const REVIEW_DOC = '# 某系统评审\n\n> **文档类型：评审（一次性结论，不是规范）**\n\n正文。\n'
const IMPLEMENTED_DOC = '# 某改动实施方案\n\n状态：**已实施**（S1–S6 全部落地）\n\n正文。\n'

test('D-002：评审类文档留在 docs/design/ → 失败', () => {
  const r = audit({ 'docs/design/review.md': REVIEW_DOC })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /docs\/design\/review\.md/)
})

test('D-002：过程文档留在**模块目录**同样失败（只扫 docs/ 会漏掉这一整类）', () => {
  const r = audit({ 'symbio/src/plugins/foo/docs/migration.md': REVIEW_DOC })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /symbio\/src\/plugins\/foo\/docs\/migration\.md/)
})

test('D-002：examples/ 不扫（是示例包内容，不是项目文档）', () => {
  const r = audit({ 'examples/pkg/docs/review.md': REVIEW_DOC })
  assert.equal(r.status, 0)
})

test('D-002：已落地的实施方案留在活跃目录 → 失败', () => {
  const r = audit({ 'docs/design/plan.md': IMPLEMENTED_DOC })
  assert.equal(r.status, 1)
})

test('D-002：同样的内容放进 docs/archive/ → 通过', () => {
  const r = audit({
    'docs/archive/review.md': REVIEW_DOC,
    'docs/archive/plan.md': IMPLEMENTED_DOC,
  })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /应归档 0 篇/)
})

test('D-002：现行规范不误报（「状态：现行规范」不是过程文档）', () => {
  const r = audit({
    'docs/design/vdfs.md': '# VDFS 规范\n\n状态：现行规范（纯接口 + 统一文件系统 + 容器拓扑）\n',
  })
  assert.equal(r.status, 0)
})

test('D-002：豁免必须带理由，空理由视为未豁免', () => {
  const withReason = `<!-- doc-link-allow D-002: 本文是现行规范，「评审」指代码评审流程 -->\n${REVIEW_DOC}`
  assert.equal(audit({ 'docs/design/kept.md': withReason }).status, 0)

  const emptyReason = `<!-- doc-link-allow D-002:    -->\n${REVIEW_DOC}`
  assert.equal(audit({ 'docs/design/kept.md': emptyReason }).status, 1, '空理由不算豁免')
})

test('D-002：豁免写在第 15 行之后无效（只看头部自述）', () => {
  const late = `${REVIEW_DOC}\n${'填充行\n'.repeat(20)}<!-- doc-link-allow D-002: 理由 -->\n`
  assert.equal(audit({ 'docs/design/late.md': late }).status, 1)
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
