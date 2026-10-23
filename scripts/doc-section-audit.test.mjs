/**
 * doc-section-audit 回归测试
 *
 * 这条守卫判两件事：一级小节编号**唯一且递增**（DS-001）、`§N` 引用**解析得到**（DS-002）。
 * 它最危险的失效形态全是**静默变绿**，所以逐条钉：
 *
 *   ① **抓得住**：编号重复 / 回退、同篇悬空引用、跨篇悬空引用，四种都必须红；
 *   ② **不误报**：缺号（删除留痕）不判、行内代码里的 `§` 不算引用、
 *      目标文档不用 `§N` 寻址时不判；
 *   ③ **不许冒充**：`### 1. 自相似性` 挂在**无号**父节下时不是地址 `§1`——
 *      若把它当地址，正文里指「§1」的悬空引用会**假装解析成功**，守卫变绿而缺陷还在。
 *      这是本判据最危险的失效形态，单独一条用例钉它；
 *   ④ **读不出即失败**：没有 `docs/` 时必须 exit 2，不退化成「什么都没检查 ⇒ 绿灯」。
 *
 * 跑法：node --test scripts/doc-section-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./doc-section-audit.mjs', import.meta.url))

/** 造一棵最小仓库：`files` 是 相对路径 → 内容 */
function audit(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-section-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.ifError(r.error)
    return r
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

const out = (r) => `${r.stdout}${r.stderr}`

test('干净仓：编号递增 + 引用全部解析 ⇒ 通过', () => {
  const r = audit({
    'docs/a.md': '# A\n\n## 1. 一\n\n## 2. 二\n\n见 §1 与 §2.1。\n\n### 2.1 子\n',
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /通过/)
})

test('DS-001 编号重复 ⇒ 红', () => {
  const r = audit({ 'docs/a.md': '# A\n\n## 1. 一\n\n## 1. 又一\n' })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /DS-001 .*编号重复/)
})

test('DS-001 编号回退（两篇并进一个文件）⇒ 红', () => {
  const r = audit({ 'docs/a.md': '# A\n\n## 1. 一\n\n## 2. 二\n\n## 1. 另一篇的 1\n' })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /编号回退/)
})

test('缺号不判：§12 之后直接 §14 是通过（缺号是删除留痕，重编号会让地址漂移）', () => {
  const r = audit({ 'docs/a.md': '# A\n\n## 12. 十二\n\n## 14. 十四\n\n见 §14。\n' })
  assert.equal(r.status, 0, out(r))
})

test('DS-002 同篇悬空 ⇒ 红', () => {
  const r = audit({ 'docs/a.md': '# A\n\n## 1. 一\n\n详见 §7.2。\n' })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /DS-002 .*§7\.2/)
})

test('DS-002 跨篇悬空 ⇒ 红（锚点取链接 href 指向的文档）', () => {
  const r = audit({
    'docs/a.md': '# A\n\n## 1. 一\n\n见 [B](b.md) §9。\n',
    'docs/b.md': '# B\n\n## 1. 一\n',
  })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /DS-002 .*§9/)
})

test('DS-002 跨篇解析得到 ⇒ 通过', () => {
  const r = audit({
    'docs/a.md': '# A\n\n## 1. 一\n\n见 [B](b.md) §2。\n',
    'docs/b.md': '# B\n\n## 1. 一\n\n## 2. 二\n',
  })
  assert.equal(r.status, 0, out(r))
})

test('行内代码里的 § 不算引用（那是引用一个标识符，不是指认一处地址）', () => {
  const r = audit({ 'docs/a.md': '# A\n\n## 1. 一\n\n缺陷是 `§7.2` 悬空。\n' })
  assert.equal(r.status, 0, out(r))
})

test('目标文档不用 §N 寻址 ⇒ 跳过不判（场景 / 规格 / 任务类文档）', () => {
  const r = audit({
    'docs/a.md': '# A\n\n## 1. 一\n\n见 [B](b.md) §9。\n',
    'docs/b.md': '# B\n\n## 场景\n\n## 边界\n',
  })
  assert.equal(r.status, 0, out(r))
})

test('不许冒充：`### 1.` 挂在无号父节下不是地址 §1 ⇒ 指 §1 必须红', () => {
  // 同一篇里另有 `## 2.` 使本文「用 §N 寻址」，于是 §1 会被判；而 `### 1. 自相似性`
  // 的父节是无号的 `## 核心设计哲学`，它在节内从 1 数起，**不是**地址 §1。
  const r = audit({
    'docs/a.md':
      '# A\n\n## 核心设计哲学\n\n### 1. 自相似性\n\n## 2. 横向\n\n正文见 §1 与 §2。\n',
  })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /DS-002 .*§1 /)
})

test('平铺写法：`### 3.2` 与 `### 3.2.1` 同级时，§3.2.1 仍是地址', () => {
  const r = audit({
    'docs/a.md': '# A\n\n## 3. 三\n\n### 3.2 子\n\n### 3.2.1 孙\n\n见 §3.2.1。\n',
  })
  assert.equal(r.status, 0, out(r))
})

test('读不出范围 ⇒ exit 2（不退化成「什么都没检查 ⇒ 绿灯」）', () => {
  const r = audit({ 'README.md': '# 空仓\n' })
  assert.equal(r.status, 2, out(r))
})
