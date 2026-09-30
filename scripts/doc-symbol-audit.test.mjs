/**
 * doc-symbol-audit 回归测试
 *
 * 判据的每条分支都用「注入真实违规并断言变红 / 注入合法文本并断言不变红」
 * 双向钉住：一个只会亮绿灯的守卫等于没有守卫，它腐烂的方式恰恰是规则写错
 * 后永远不命中（判据写宽 ⇒ 注释、普通词、单段词误报 ⇒ 被豁免喂到失效）。
 *
 * 跑法：node --test scripts/doc-symbol-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./doc-symbol-audit.mjs', import.meta.url))

/**
 * 在临时目录造一棵最小仓库并跑审计。
 * @param {Record<string, string>} files 相对仓库根的路径 → 内容
 */
function audit (files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-symbol-audit-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 20000,
    })
    assert.ifError(r.error)
    return r
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

/** 最小 Rust 语料：两个真实符号（一个带前缀、一个仅字段名） */
const RS = {
  'symbio/src/lib.rs': [
    'pub const PLUGIN_PAYLOAD_KEY: &str = "payload";',
    'fn register_option_field(order: i32) {}',
  ].join('\n'),
}

test('失效指认变红：文档指认源码里不存在的符号（exit 1）', () => {
  const r = audit({ ...RS, 'README.md': '桶名见 `symbio_core::GONE_FN`。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /D-005 README\.md:1/)
  assert.match(r.stdout, /GONE_FN/)
})

test('整词边界：`register_option` 不因 `register_option_field` 存在而放行（exit 1）', () => {
  const r = audit({ ...RS, 'README.md': '接口 `OptionVisitor::register_option`。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /register_option`/)
})

test('有效指认通过（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '桶名见 `symbio_core::PLUGIN_PAYLOAD_KEY`。\n' })
  assert.equal(r.status, 0, r.stdout)
  assert.match(r.stdout, /doc-symbol-audit 通过/)
})

test('单段反引号词不是符号指认（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '旧名 `GONE_WORD` 已不再使用。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('非标识符形态（泛型 / 路径带参数）不查（exit 0）', () => {
  const r = audit({ ...RS, 'README.md': '形如 `map::get::<K, V>` 的写法。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('历史行豁免：行内含「已删除」（exit 0）', () => {
  const r = audit({
    ...RS,
    'README.md': '| `OptionVisitor::GONE_METHOD` | **已删除** |\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('DECISIONS.md 整体排除：ADR 是历史快照（exit 0）', () => {
  const r = audit({
    ...RS,
    'docs/DECISIONS.md': '当年引入 `symbio_core::GONE_FN` 又删掉了。\n',
    'README.md': '正常文档。\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('docs/decisions/ 整目录排除：ADR 分域正文与索引同源（exit 0）', () => {
  const r = audit({
    ...RS,
    'docs/decisions/core.md': '当年引入 `symbio_core::GONE_FN` 又删掉了。\n',
    'README.md': '正常文档。\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('docs/archive/ 整目录排除：归档是历史快照（exit 0）', () => {
  const r = audit({
    ...RS,
    'docs/archive/old-plan.md': '当时叫 `symbio_core::GONE_FN`。\n',
    'README.md': '正常文档。\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('排除只认整目录前缀：docs/decisions.md 这类同名前缀文件仍受查（exit 1）', () => {
  const r = audit({
    ...RS,
    'docs/decisions.md': '指认 `symbio_core::GONE_FN`。\n',
    'README.md': '正常文档。\n',
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /docs\/decisions\.md:1/)
})

test('承认通道：理由非空则放行（exit 0）', () => {
  const r = audit({
    ...RS,
    'README.md': '桶名见 `external_crate::GONE_FN`。<!-- doc-symbol-allow: 外部 crate 符号 -->\n',
  })
  assert.equal(r.status, 0, r.stdout)
})

test('承认通道：空理由视为未承认（exit 1）', () => {
  const r = audit({
    ...RS,
    'README.md': '桶名见 `external_crate::GONE_FN`。<!-- doc-symbol-allow: -->\n',
  })
  assert.equal(r.status, 1, r.stdout)
})

test('审计范围读不出：没有任何 md ⇒ 失败而非绿灯（exit 1）', () => {
  const r = audit(RS)
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /0 篇候选 md/)
})

test('审计范围读不出：没有 .rs 语料 ⇒ 失败而非绿灯（exit 1）', () => {
  const r = audit({ 'README.md': '正常文档。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /0 个 \.rs 语料根/)
})
