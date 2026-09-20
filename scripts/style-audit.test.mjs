/**
 * style-audit 回归测试 —— 证明它**会红**
 *
 * 为什么现在才补：2026-09-20 复核发现 `gate.mjs` 宣称「每个判定型守卫都先跑自己的
 * 回归测试」，而 8 个判定型守卫里有 3 个（本脚本 / `doc-link-audit` /
 * `test-layout-audit`）**从来没有过**——缺的恰恰是判定力度最弱的那三个。
 *
 * 用法：node --test scripts/style-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./style-audit.mjs', import.meta.url))

/**
 * 在临时仓库根里铺 `tauri/src/**` 夹具再跑脚本。
 *
 * 脚本原本把根写死成"脚本上一级的仓库根"，无法指向夹具 ⇒ 回归测试无从写起。
 * 为此给它加了 `--root=`（与 `mechanism-audit` 同一约定）。
 */
function audit(files, { strict = false } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'style-audit-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    // ⚠️ 必须显式传 `--root=`：脚本默认按"脚本上一级的仓库根"定位 `tauri/src`，
    // 只设 cwd 不会改变它 ⇒ 漏传就会去审**真实仓库**，于是每条用例都拿到真仓的
    // 结果（当前恰好全绿）⇒ 夹具根本没被测到，测试却全绿。这正是要防的那类假绿。
    const result = spawnSync(
      process.execPath,
      [script, `--root=${root}`, ...(strict ? ['--strict'] : [])],
      { cwd: root, env: { ...process.env, NO_COLOR: '1' }, encoding: 'utf8', timeout: 20_000 }
    )
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

const vue = (tpl, style = '') => `<template>\n  ${tpl}\n</template>\n${style}`

// ── 规则 A：被使用的样式定义必须存在（ERROR，唯一的硬失败）──────────────
test('A 命中：模板用了没在任何地方定义的类 → ERROR', () => {
  const r = audit({ 'tauri/src/components/Ghost.vue': vue('<div class="ghost-box" />') })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /ghost-box/)
})

test('A 不误报：类定义在同组件的 scoped 样式里 → 不红', () => {
  const r = audit({
    'tauri/src/components/Ok.vue': vue(
      '<div class="ok-box" />',
      '<style scoped>\n.ok-box { color: red; }\n</style>'
    ),
  })
  assert.equal(r.status, 0)
})

test('A 不误报：类定义在全局 CSS 里 → 不红', () => {
  const r = audit({
    'tauri/src/components/Ok.vue': vue('<div class="global-box" />'),
    'tauri/src/styles/base.css': '.global-box { color: red; }\n',
  })
  assert.equal(r.status, 0)
})

test('A 命中：动态类前缀没有任何已定义类以它开头 → ERROR', () => {
  // `:class="\`ghost-${x}\`"` 这类拼接，前缀至少要能命中一个已定义类
  const r = audit({
    'tauri/src/components/Dyn.vue': vue('<div :class="`ghost-${kind}`" />'),
  })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /ghost/)
})

// ── 规则 B：定义未使用只是 WARNING（--strict 才算失败）─────────────────
// 这条**刻意**不是 ERROR：误报 unused 会逼人写豁免注释，而一个靠豁免活着的守卫
// 等于没有守卫（见脚本文件头）。测试把两档都钉住，免得有人"顺手"把它升级成 ERROR。
test('B 的严重度：定义未使用默认只警告、--strict 才失败', () => {
  const files = {
    'tauri/src/styles/base.css': '.orphan-box { color: red; }\n',
    'tauri/src/components/Used.vue': vue('<div />'),
  }
  assert.equal(audit(files).status, 0)
  assert.equal(audit(files, { strict: true }).status, 1)
})

// ── 空树：守卫不该空转而红 ──────────────────────────────────────────────
test('空树通过（守卫不是空转即红）', () => {
  assert.equal(audit({}).status, 0)
})

// ── 真实仓库当前状态：防它在真仓上误报 ──────────────────────────────────
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
