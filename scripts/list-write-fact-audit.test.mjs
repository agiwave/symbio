/**
 * list-write-fact-audit 回归测试 —— 证明它**会红**
 *
 * 一个只会亮绿灯的守卫等于没有守卫。本用例注入真实违规并断言脚本变红
 * （守卫 v2：结构性判定，无登记行）：
 *   ① 干净夹具（write.rs 里的原语 + apply 汇聚）→ exit 0；
 *   ② `append_messages` 出了 write.rs → LWF-001（绕过唯一写入口）；
 *   ③ `replace_all` 出了 write.rs 且没有寄存锚 → LWF-002；
 *   ④ `replace_all` 出了 write.rs 但 ±NEAR 行内有真实的 `consolidated = Some(...)` → 放行；
 *   ⑤ 寄存锚只在**注释**里 → LWF-002（锚必须是真代码）；
 *   ⑥ 寄存锚在窗口边界之外 → LWF-002；
 *   ⑦ `v2_messages::edited` 出了 write.rs → LWF-001（第二个写入口）；
 *   ⑧ `.test.rs` 与注释里的提及不触发（不误报）。
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

/** 唯一写入口本体的干净写法：原语实现 + apply 汇聚，全在 write.rs 里。 */
const WRITER = [
  'pub(crate) async fn apply(&self, change: MessageChange) -> Result<Vec<ChatNode>, PluginError> {',
  '    match change {',
  '        MessageChange::Born { nodes, .. } => self.append_messages(nodes).await,',
  '        MessageChange::Edited(msgs) => { self.replace_all(list).await?; Ok(msgs) }',
  '        MessageChange::Removed(ids) => { self.replace_all(list).await?; Ok(Vec::new()) }',
  '    }',
  '}',
  '',
].join('\n')

test('干净夹具通过（write.rs 内的原语 + apply 汇聚）', () => {
  const r = audit({ 'plugins/session/chat_session/write.rs': WRITER })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('append_messages 出了 write.rs → LWF-001 变红（绕过唯一写入口）', () => {
  const r = audit({
    'plugins/session/commands.rs': 'async fn patch(session: &S) {\n    session.append_messages(msgs).await?;\n}\n',
  })
  assert.equal(r.status, 1, '绕过 apply 的节点级写原语必须红')
  assert.match(r.stderr, /LWF-001/)
  assert.doesNotMatch(r.stderr, /LWF-002/, '此例只应报 LWF-001')
})

test('replace_all 出了 write.rs 且没有寄存锚 → LWF-002 变红', () => {
  const r = audit({
    'plugins/session/other.rs': 'async fn rewrite(session: &S) {\n    session.replace_all(list).await?;\n}\n',
  })
  assert.equal(r.status, 1, '无寄存锚的取代原语必须红')
  assert.match(r.stderr, /LWF-002/)
})

test('replace_all 带真实寄存锚（±20 行内的 consolidated = Some(...)）→ 放行', () => {
  const lines = []
  for (let i = 0; i < 15; i++) lines.push(`    let filler_${i} = ${i};`)
  lines.push('    context.consolidated = Some(new_messages.clone());')
  lines.push('    let _ = context.session.replace_all(context.messages.clone()).await;')
  lines.push('    Ok(())')
  lines.push('}')
  const r = audit({ 'plugins/session/context/pipeline.rs': lines.join('\n') + '\n' })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('寄存锚只在注释里 → LWF-002 变红（锚必须是真代码）', () => {
  const r = audit({
    'plugins/session/other.rs': [
      'async fn rewrite(session: &S) {',
      '    // 寄存：context.consolidated = Some(new_messages.clone());——只是说说',
      '    session.replace_all(list).await?;',
      '}',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 1, '注释里的「锚」不算锚')
  assert.match(r.stderr, /LWF-002/)
})

test('寄存锚在窗口边界之外（>20 行）→ LWF-002 变红', () => {
  const lines = ['    context.consolidated = Some(new_messages.clone());']
  for (let i = 0; i < 25; i++) lines.push(`    let filler_${i} = ${i};`)
  lines.push('    session.replace_all(list).await?;')
  lines.push('}')
  const r = audit({ 'plugins/session/other.rs': lines.join('\n') + '\n' })
  assert.equal(r.status, 1, '锚离调用点太远就不算这次的锚')
  assert.match(r.stderr, /LWF-002/)
})

test('v2_messages::edited 出了 write.rs → LWF-001 变红（第二个写入口）', () => {
  const r = audit({
    'plugins/session/commands.rs': 'fn land(dir: &Path, m: &ChatNode) {\n    v2_messages::edited(dir, m);\n}\n',
  })
  assert.equal(r.status, 1, '事实落格函数不得有第二个调用方')
  assert.match(r.stderr, /LWF-001/)
})

test('.test.rs 与注释里的提及不触发（不误报）', () => {
  const r = audit({
    'plugins/session/commands.test.rs': [
      'async fn patch(session: &S) {',
      '    session.append_messages(msgs).await?; // 测试是排演驱动方',
      '}',
      '',
    ].join('\n'),
    'plugins/session/docs.rs': [
      '//! 文档里提到 `append_messages` 与 `v2_messages::edited` 不算调用。',
      '/// 另一段：`replace_all` 的契约是只补缺号。',
      'pub fn unrelated() {}',
      '',
    ].join('\n'),
  })
  assert.equal(r.status, 0, r.stderr + r.stdout)
})
