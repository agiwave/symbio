/**
 * list-write-fact-audit 回归测试 —— 证明它**会红**
 *
 * 一个只会亮绿灯的守卫等于没有守卫。本用例注入真实违规并断言脚本变红：
 *   ① 干净夹具（调用 + 就地登记 + 事实在现场）→ exit 0；
 *   ② 调用没有就地登记 → LWF-001；
 *   ③ 登记的理由为空 → LWF-002（空理由 = 没有登记）；
 *   ④ 登记声明的事实不在现场 → LWF-002；
 *   ⑤ 登记在、调用不在（函数改名 / 调用点搬走）→ LWF-004；
 *   ⑥ 待补（`pending`）超过棘轮基线 → LWF-003；
 *   ⑦ `by-design` 与 `pending` 是**放行**而不是漏判（不误报）。
 *
 * 用法：node --test scripts/list-write-fact-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./list-write-fact-audit.mjs', import.meta.url))

/** 删临时目录 —— 逐个删，不用 rmSync(recursive)（沙箱会拦截批量删除，见 test-layout-audit.test.mjs）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

function audit(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'list-write-fact-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(process.execPath, [script, `ROOT=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30_000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    rmTree(root)
  }
}

/** 一处「调用 + 就地登记 + 事实在现场」的干净写法。 */
const CLEAN = [
  'async fn patch_message(session: &S) {',
  '    // list-write-fact patch_message: v2_messages::edited — 编辑是「这条现在读作 Y」',
  '    session.replace_messages(messages).await?;',
  '    super::v2_messages::edited(&dir, &updated);',
  '}',
  '',
].join('\n')

test('干净夹具通过（调用 + 登记 + 事实在现场）', () => {
  const r = audit({ 'plugins/session/commands.rs': CLEAN })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('调用没有就地登记 → LWF-001 变红', () => {
  const r = audit({
    'plugins/session/commands.rs': 'async fn patch_message(session: &S) {\n    session.replace_messages(m).await?;\n}\n',
  })
  assert.equal(r.status, 1, '未登记的列表写入口必须红')
  assert.match(r.stderr, /LWF-001/)
})

test('登记的理由为空 → LWF-002 变红（空着就等于没有登记）', () => {
  const r = audit({
    'plugins/session/commands.rs': [
      'async fn patch_message(session: &S) {',
      '    // list-write-fact patch_message: v2_messages::edited —',
      '    session.replace_messages(m).await?;',
      '    super::v2_messages::edited(&dir, &updated);',
      '}',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 1, '空理由必须红')
  assert.match(r.stderr, /LWF-002/)
})

test('登记声明的事实不在现场 → LWF-002 变红', () => {
  const r = audit({
    'plugins/session/commands.rs': [
      'async fn patch_message(session: &S) {',
      '    // list-write-fact patch_message: v2_messages::removed — 说要删，但代码里只有编辑',
      '    session.replace_messages(m).await?;',
      '    super::v2_messages::edited(&dir, &updated);',
      '}',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 1, '声明的事实不在现场必须红')
  assert.match(r.stderr, /LWF-002/)
})

test('登记在、调用不在（函数改名 / 调用点搬走）→ LWF-004 变红', () => {
  const r = audit({
    'plugins/session/commands.rs': [
      'async fn renamed_away(session: &S) {',
      '    // list-write-fact patch_message: v2_messages::edited — 这个函数已经不存在了',
      '    session.replace_messages(m).await?;',
      '    super::v2_messages::edited(&dir, &updated);',
      '}',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 1, '登记指着一个不存在的函数必须红')
  assert.match(r.stderr, /LWF-004/)
})

test('待补（pending）超过棘轮基线 → LWF-003 变红', () => {
  const files = {}
  for (let i = 0; i < 5; i++) {
    files[`plugins/session/f${i}.rs`] = [
      `async fn f${i}(session: &S) {`,
      `    // list-write-fact f${i}: pending — 已知缺口 ${i}`,
      '    session.replace_messages(m).await?;',
      '}',
      '',
    ].join('\n')
  }
  const r = audit(files)
  assert.equal(r.status, 1, '待补只降不升：多出来的必须红')
  assert.match(r.stderr, /LWF-003/)
})

test('by-design 与 pending 是放行（不误报）', () => {
  const r = audit({
    'plugins/session/io.rs': [
      'async fn persist_messages(session: &S) {',
      '    // list-write-fact persist_messages: by-design — 事实由运行器在别处落',
      '    append_and_publish(&s, m, to, p).await?;',
      '}',
      '',
    ].join('\n'),
    'plugins/session/failure.rs': [
      'async fn persist_failure(session: &S) {',
      '    // list-write-fact persist_failure: pending — 状态改写 + partial 新节点待补',
      '    session.replace_messages(all).await?;',
      '}',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})
