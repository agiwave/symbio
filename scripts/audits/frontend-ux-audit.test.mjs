/**
 * frontend-ux-audit 回归测试
 *
 * 这份守卫的失效方式是「规则写错所以永不命中」——一条永远不红的可用性审计，
 * 看起来和「前端很干净」完全一样。所以每条判据都配一条**注入违规**的反向用例，
 * 另加两条边界：干净夹具必须绿（防过拟合）与读不到 `.vue` 必须红（防静默全绿）。
 *
 * 跑法：node --test scripts/audits/frontend-ux-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { auditCase, expectAudit } from '../audit-assert.mjs'

const script = fileURLToPath(new URL('./frontend-ux-audit.mjs', import.meta.url))

function run(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'frontend-ux-audit-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    return spawnSync(process.execPath, [script, `--root=${root}`], {
      encoding: 'utf8',
      env: { ...process.env, NO_COLOR: '1' },
      timeout: 30_000,
    })
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

const clean = `
<template>
  <div>
    <button title="刷新" aria-label="刷新"><svg /></button>
    <button>保存</button>
    <input aria-label="模型名" />
    <input id="api-base" />
    <label for="api-base">API Base</label>
    <label>温度 <input type="number" /></label>
    <textarea aria-label="系统提示词" />
    <img src="a.png" alt="" />
  </div>
</template>
<script setup lang="ts">
const ok = true
</script>
`

auditCase('干净夹具 → 通过（不误报正常写法）', () => run({ 'tauri/src/Clean.vue': clean }), 0, [
  /frontend-ux-audit|结果/,
])

test('UX-001：原生 confirm 会冻结 WebView → 红', () => {
  const r = run({
    'tauri/src/Bad.vue': '<script>confirm("真的删除？")</script>',
  })
  expectAudit(r, 1, [/UX-001/, /Bad\.vue/])
})

test('UX-002：纯 SVG 按钮没有可访问名称 → 红', () => {
  const r = run({
    'tauri/src/Bad.vue': '<template><button @click="go"><svg /></button></template>',
  })
  expectAudit(r, 1, [/UX-002/, /Bad\.vue/])
})

test('UX-003：只有 placeholder 的输入框 → 红', () => {
  const r = run({
    'tauri/src/Bad.vue': '<template><input placeholder="输入模型名" /></template>',
  })
  expectAudit(r, 1, [/UX-003/, /Bad\.vue/])
})

test('UX-004：图片没有 alt → 红', () => {
  const r = run({
    'tauri/src/Bad.vue': '<template><img src="logo.png" /></template>',
  })
  expectAudit(r, 1, [/UX-004/, /Bad\.vue/])
})

test('边界：读不到 .vue 时不能静默全绿 → 退出码 2', () => {
  const r = run({ 'tauri/src/readme.txt': 'not a vue file' })
  assert.equal(r.status, 2, r.stdout + r.stderr)
  assert.match(r.stderr, /读不到任何 \.vue/)
})
