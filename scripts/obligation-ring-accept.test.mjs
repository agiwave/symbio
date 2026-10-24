/**
 * obligation-ring-accept 回归测试
 *
 * 这条判据是「CLI 实测的账实相符」，它最危险的失效形态是**假红**：一次完全合规的
 * 实测被判成一堆失效，于是「再跑一遍看看」变成习惯，判据失去牙齿。归档段漏读正是
 * 这个形态——长会话实测里 20 轮之后 live 段只剩 12 格，`produced_by` 全指向
 * 已搬进 `transcripts/wal-archive/` 的前缀，连报 16 项失效，而真正的结论（压缩真
 * 发生、摘要往返入了账）一段都没看。
 *
 * 所以逐条钉的是**读法**，不是判据本身（J1…J9 各有实测对账）：
 *
 *   ① **归档段要读**：live 只有后半、前半在归档段时，跨段的 `produced_by` 必须解析；
 *   ② **不误报**：没有归档目录（归档前的会话）照样只读 live，行为不变；
 *   ③ **撕裂尾行停止重放**：崩溃安全留下的半行不是「JSON 不可解析 ⇒ 退出 2」，
 *      而是「到此为止」——与 `store/wal.rs::replay` 同口径，否则崩溃恢复过的
 *      会话永远无法被验收；
 *   ④ **不许冒充**：归档目录里不匹配 `<wal 文件名>.<first>-<last>` 的文件不是段，
 *      读它会把别的内容混进事实层；
 *   ⑤ **输入缺失即退出 2**：读不出会话语就不判，不退化成「什么都没检查 ⇒ 绿灯」。
 *
 * 跑法：node --test scripts/obligation-ring-accept.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./obligation-ring-accept.mjs', import.meta.url))

const ev = (seq, kind, payload, produced_by = null) => ({
  event_id: `e${seq}`,
  kind,
  entity: 'Turn',
  verb: 'Asserted',
  turn: 1,
  actor: 'user',
  produced_by,
  cost_ms: 0,
  ts: 1792803040849 + seq,
  payload,
  seq,
})

/** 一轮最小合规往返：输入格 → 判决格 → 三格 → 收束格（J1…J7 全绿的形状）。 */
const USER = ev(1, 'user.message', { id: 'u1', text: '第一轮' })
const DECIDE = ev(2, 'turn.stage', { stage: 'decide', model_calls: 1, reason: 'answered' })
const REQUEST = ev(3, 'request', { id: 'r1', port: 'model', via: 'classify/decide', attempt: 1 }, 2)
const SENT = ev(4, 'sent', { id: 'r1', port: 'model', attempt: 1 }, 3)
const RESPONSE = ev(5, 'response', { id: 'r1', port: 'model', attempt: 1 }, 4)
const FINAL = ev(6, 'chat.assistant.final', { reason: 'completed', text: '好' }, 5)

const SID = 'ring-test'

/**
 * 造一棵最小 homedir 并跑验收。
 *
 * `archive`：归档段文件表（段名 → 内容）；`live`：live 段内容（字符串，可直接塞
 * 撕裂行）；`extra`：归档目录里的其他文件（段名 → 内容），用来判「不匹配就不读」。
 */
function accept({ archive = {}, live, extra = {} }) {
  const homedir = fs.mkdtempSync(path.join(os.tmpdir(), 'ring-accept-'))
  try {
    const sess = path.join(homedir, 'session', SID)
    fs.mkdirSync(sess, { recursive: true })
    if (live !== undefined) fs.writeFileSync(path.join(sess, 'v2-events.wal'), live)
    const adir = path.join(sess, 'transcripts', 'wal-archive')
    if (Object.keys(archive).length > 0 || Object.keys(extra).length > 0) {
      fs.mkdirSync(adir, { recursive: true })
      for (const [name, content] of Object.entries(archive)) {
        fs.writeFileSync(path.join(adir, name), content)
      }
      for (const [name, content] of Object.entries(extra)) {
        fs.writeFileSync(path.join(adir, name), content)
      }
    }
    const r = spawnSync(
      process.execPath,
      [script, `--homedir=${homedir}`, `--session=${SID}`],
      { env: { ...process.env, NO_COLOR: '1' }, encoding: 'utf8', timeout: 30000 },
    )
    assert.ifError(r.error)
    return r
  } finally {
    fs.rmSync(homedir, { recursive: true, force: true })
  }
}

const lines = (...events) => `${events.map((e) => JSON.stringify(e)).join('\n')}\n`
const out = (r) => `${r.stdout}${r.stderr}`

test('归档段要读：前半在归档段、后半在 live ⇒ 跨段 produced_by 解析，全项通过', () => {
  const r = accept({
    archive: { 'v2-events.wal.1-2': lines(USER, DECIDE) },
    live: lines(REQUEST, SENT, RESPONSE, FINAL),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
  assert.match(out(r), /6 格 \/ 1 组端口往返/)
})

test('同一份事实只读 live（无归档目录）⇒ 照样通过（归档前的会话不受影响）', () => {
  const r = accept({ live: lines(USER, DECIDE, REQUEST, SENT, RESPONSE, FINAL) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

test('漏读归档段的失效形态：把前半丢掉 ⇒ J2/J5 判红（这条用例钉住「为什么要读」）', () => {
  const r = accept({ live: lines(REQUEST, SENT, RESPONSE, FINAL) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J5 request#3 的 produced_by=2 不存在/)
  assert.match(out(r), /J2 request#3（turn=1）之前没有同轮 user.message 输入格/)
})

test('撕裂尾行停止重放：live 末行是半行 JSON ⇒ 不退出 2，前 6 格照判', () => {
  const r = accept({
    live: `${lines(USER, DECIDE, REQUEST, SENT, RESPONSE, FINAL).trimEnd()}\n{"seq":99,"kind":"user.mes`,
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

test('撕裂尾行在归档段里同理：半行之后的段截断，不退出 2', () => {
  const r = accept({
    archive: { 'v2-events.wal.1-3': `${lines(USER, DECIDE, REQUEST).trimEnd()}\n{"seq":9,"kind":"tur` },
    live: lines(SENT, RESPONSE, FINAL),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

test('段名不匹配就不是段：归档目录里的 `.bak` 与别的 WAL 名都不读', () => {
  const bogus = lines(ev(90, 'request', { id: 'x', port: 'bogus', via: 'nope', attempt: 1 }))
  const r = accept({
    archive: { 'v2-events.wal.1-2': lines(USER, DECIDE) },
    extra: {
      'v2-events.wal.bak': bogus,
      'other-events.wal.1-2': bogus,
      'v2-events.wal.not-a-number': bogus,
    },
    live: lines(REQUEST, SENT, RESPONSE, FINAL),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

test('两个归档段都要读：后一段的引用指向前一段 ⇒ 少读一段就红', () => {
  // 段 A：seq 1-2（输入 + 判决）；段 B：seq 7-8 是**追加的一段历史**，它自己的
  // `produced_by` 指向段 A 的 seq 2（`archive_before` 之后同一会话会留下多段，
  // 段与段之间的引用就这个形状）。只读段 A ⇒ 少两格；只读 live ⇒ 段 B 的引用悬空。
  // 段 B 的第二格取 `recall` 而不是 `decide`：它不参与 J7 的观测对账，
  // 这条用例要钉的是「段读全了」，不是把 J7 的账弄乱。
  const tail = lines(
    ev(7, 'user.message', { id: 'u2', text: '第二轮' }, null),
    ev(8, 'turn.stage', { stage: 'recall', result: 'miss' }, 2),
  )
  const r = accept({
    archive: { 'v2-events.wal.1-2': lines(USER, DECIDE), 'v2-events.wal.7-8': tail },
    live: lines(REQUEST, SENT, RESPONSE, FINAL),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /8 格 \/ 1 组端口往返/)
})

test('读不出会话语 ⇒ 退出 2，不退化成绿灯', () => {
  const r = accept({ live: undefined })
  assert.equal(r.status, 2, out(r))
  assert.match(out(r), /读不到 WAL/)
})

test('live 是空文件 ⇒ 退出 2', () => {
  const r = accept({ live: '' })
  assert.equal(r.status, 2, out(r))
  assert.match(out(r), /是空的/)
})
