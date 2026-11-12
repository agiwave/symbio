/**
 * obligation-ring-accept 回归测试
 *
 * 这条判据是「CLI 实测的账实相符」，它最危险的失效形态有两半：**假红**（一次完全
 * 合规的实测被判成一堆失效，于是「再跑一遍看看」变成习惯）与**假绿**（读不到事实，
 * 于是每一项都「没有对象 ⇒ 通过」）。两半都在 2026-11 的重锚前同时发生过：判据按
 * 上一代本体（`payload.id` / `payload.port` / `turn.stage`）读现行 WAL，报 29 项
 * 假红，而 J7 的观测恒 0 ⇒ 恒绿。
 *
 * 所以本文件逐条钉的是**读法**，不是判据本身：
 *
 *   ① **归档段要读**：live 只有后半、前半在归档段时，跨段的 `produced_by` 必须解析；
 *   ② **不误报**：没有归档目录（归档前的会话）照样只读 live，行为不变；
 *   ③ **撕裂尾行停止重放**：崩溃安全留下的半行不是「JSON 不可解析 ⇒ 退出 2」，
 *      而是「到此为止」——与 `store/wal.rs::replay` 同口径；
 *   ④ **不许冒充**：归档目录里不匹配 `<wal 文件名>.<first>-<last>` 的文件不是段；
 *   ⑤ **输入缺失即退出 2**：读不出会话语就不判，不退化成「什么都没检查 ⇒ 绿灯」；
 *   ⑥ **反向五条**（假绿的牙齿）：多一次 HTTP 而少一笔 `sent` ⇒ J8 红；一笔缺终态
 *      ⇒ J1 红；协议事实不带 `ring:` 身份 ⇒ J6 红（形状判据，端口白名单退场后仍不放宽）；
 *      `sent` 晚于终态 ⇒ J3 红；收束格 `reason` 为空 ⇒ J4 红。连同下面的 J2 / J5 /
 *      J7 / J9 反向夹具，**J1…J9 每一条都有可打开的失效用例**，不是只有正向绿灯；
 *   ⑦ **J7 双向**：三格多于观测、观测多于三格，各判红；
 *   ⑧ **J9 配对**：压缩产物找不到配对的摘要往返 ⇒ 红；
 *   ⑨ **动态端口准入**（D-447 ①）：含未在静态表中的自定义端口 `custom_probe` 的完整
 *      事实序列，J6 仍通过、且输出里有「新端口」与「声明未出现」的漂移提示——
 *      「新增端口不改验收表也能通过 J6」的可跑回归夹具。旧版那条「扫源码声明 ↔
 *      `DECLARED_PORTS` 逐项比对，多一个端口不改表就红」的守卫随白名单一起退场：
 *      它正是被收窄的那道门（见 obligation-ring-accept.mjs 文件头的退场 ④）。
 *
 * 夹具用**现行 WAL 形状**（`ring:{port}:{id}:{attempt}:{kind}[:{shard}]`），不留
 * 上一代形状的「历史读法」夹具：那会让下一个人以为旧本体还在。
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

/** 现行协议事实身份：`ring:{port}:{id}:{attempt}:{kind}[:{shard}]`。 */
const ring = (port, id, attempt, kind, shard = null) =>
  `ring:${port}:${id}:${attempt}:${kind}${shard === null ? '' : `:${shard}`}`

/**
 * 一条现行事实。`produced_by` 缺省指向 seq-1（线性链），seq 0 为 null——
 * 这与宿主的写入顺序同形，夹具不必逐条手写溯源位。
 */
const ev = (seq, kind, eventId, payload = {}, producedBy = undefined) => ({
  event_id: eventId,
  kind,
  entity: 'Scope',
  verb: 'Asserted',
  actor: 'user',
  produced_by: producedBy === undefined ? (seq === 0 ? null : seq - 1) : producedBy,
  cost_ms: 0,
  ts: 1794200000000 + seq,
  payload,
  seq,
})

const SID = 'ring-test'

/** 一轮最小合规往返：输入格 → 配置 → 三格 → 收束格（J1…J7 全绿的形状）。 */
const base = () => [
  ev(0, 'user.message', 'u1', { id: 'u1', text: '第一轮' }),
  ev(1, 'session.config', 'session-config:u1', { classify_enabled: true, compose_enabled: true }),
  ev(2, 'sent', ring('model', 'model:u1', 1, 'sent'), { deadline_ms: 9_000_000_000_000 }),
  // 溯源位指配置格（seq 1）：归档段只读 live 时这一格会悬空——③ 的判据靠它。
  ev(3, 'request', ring('model', 'model:u1', 1, 'request'), { root_id: 'u1' }, 1),
  ev(4, 'response', ring('model', 'model:u1', 1, 'response'), { port: 'model' }),
  ev(5, 'chat.assistant.final', 'chat.assistant.final:u1', { root_id: 'u1', reason: 'completed' }),
]

/** 一笔模型往返三格（sent → request → terminal），返回占用的下一条 seq。 */
function round(facts, seq, port, id, { attempt = 1, request = {}, terminal = 'response' } = {}) {
  facts.push(ev(seq, 'sent', ring(port, id, attempt, 'sent'), { deadline_ms: 9_000_000_000_000 }))
  facts.push(ev(seq + 1, 'request', ring(port, id, attempt, 'request'), request))
  facts.push(ev(seq + 2, terminal, ring(port, id, attempt, terminal), { port }))
  return seq + 3
}

/** 一条判决事实（模型产出的理由码：from_context / clarify / refuse / needs_work / unclassified）。 */
function verdict(facts, seq, root, kind, reason) {
  facts.push(ev(seq, kind, `${kind}:${root}`, { id: root, reason }))
  return seq + 1
}

/**
 * 造一棵最小 homedir 并跑验收。
 *
 * `archive`：归档段文件表（段名 → 内容）；`live`：live 段内容（字符串，可直接塞
 * 撕裂行）；`extra`：归档目录里的其他文件（段名 → 内容），用来判「不匹配就不读」；
 * `cliLog`：写成 `cli.log` 并以 `--cli-log` 传入；`strict`：加 `--strict`。
 */
function accept({ archive = {}, live, extra = {}, cliLog = null, strict = false }) {
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
    const argv = [script, `--homedir=${homedir}`, `--session=${SID}`]
    if (cliLog !== null) {
      const logFile = path.join(homedir, 'cli.log')
      fs.writeFileSync(logFile, cliLog)
      argv.push(`--cli-log=${logFile}`)
    }
    if (strict) argv.push('--strict')
    const r = spawnSync(process.execPath, argv, {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.ifError(r.error)
    return r
  } finally {
    fs.rmSync(homedir, { recursive: true, force: true })
  }
}

const lines = (...events) => `${events.map((e) => JSON.stringify(e)).join('\n')}\n`
const out = (r) => `${r.stdout}${r.stderr}`

// ── 读法：归档段 / 撕裂行 / 输入缺失 ───────────────────────────────────────

test('归档段要读：前半在归档段、后半在 live ⇒ 跨段 produced_by 解析，全项通过', () => {
  const all = base()
  const r = accept({
    archive: { 'v2-events.wal.0-1': lines(...all.slice(0, 2)) },
    live: lines(...all.slice(2)),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
  assert.match(out(r), /6 格 \/ 1 笔端口往返/)
})

test('同一份事实只读 live（无归档目录）⇒ 照样通过（归档前的会话不受影响）', () => {
  const r = accept({ live: lines(...base()) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

test('漏读归档段的失效形态：把前半丢掉 ⇒ J2/J5 判红（这条用例钉住「为什么要读」）', () => {
  const r = accept({ live: lines(...base().slice(2)) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J5 request#3 的 produced_by=1 不存在/)
  assert.match(out(r), /J2 request#3 之前没有 user.message 输入格/)
})

test('撕裂尾行停止重放：live 末行是半行 JSON ⇒ 不退出 2，前 6 格照判', () => {
  const r = accept({
    live: `${lines(...base()).trimEnd()}\n{"seq":99,"kind":"user.mes`,
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

test('撕裂尾行在归档段里同理：半行之后的段截断，不退出 2', () => {
  const all = base()
  const r = accept({
    archive: { 'v2-events.wal.0-3': `${lines(...all.slice(0, 4)).trimEnd()}\n{"seq":9,"kind":"tur` },
    live: lines(...all.slice(4)),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

test('段名不匹配就不是段：归档目录里的 `.bak` 与别的 WAL 名都不读', () => {
  const bogus = lines(ev(90, 'request', ring('bogus', 'x', 1, 'request'), {}))
  const all = base()
  const r = accept({
    archive: { 'v2-events.wal.0-1': lines(...all.slice(0, 2)) },
    extra: {
      'v2-events.wal.bak': bogus,
      'other-events.wal.1-2': bogus,
      'v2-events.wal.not-a-number': bogus,
    },
    live: lines(...all.slice(2)),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

test('两个归档段都要读：后一段的引用指向前一段 ⇒ 少读一段就红', () => {
  const all = base()
  const tail = lines(
    // 开轮事实没有上游（`produced_by: null`），与真实 WAL 同形。
    ev(7, 'user.message', 'u2', { id: 'u2', text: '第二轮' }, null),
    // 指向段 A 的 seq 1：只读 live / 只读后一段都会悬空。
    ev(8, 'memory.recalled', 'recalled-1', { root_id: 'u2' }, 1),
  )
  const r = accept({
    archive: { 'v2-events.wal.0-1': lines(...all.slice(0, 2)), 'v2-events.wal.7-8': tail },
    live: lines(...all.slice(2)),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /8 格 \/ 1 笔端口往返/)
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

test('--strict 没给 --cli-log ⇒ J8 判红（不判不等于放行）', () => {
  const r = accept({ live: lines(...base()), strict: true })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /--strict：没给 --cli-log——J8/)
})

// ── 反向三条：假绿的牙齿 ───────────────────────────────────────────────────

test('反向①：一次模型 HTTP 调用没有对应 sent ⇒ J8 红', () => {
  const r = accept({
    live: lines(...base()),
    // 两条请求发起，但 WAL 里只有一笔 model 端口的 sent。
    cliLog: [
      '[LLM] 请求发起 chat (model=m, msgs=2, tools=0, root=u1, body=1 bytes)',
      '[LLM] 请求发起 chat (model=m, msgs=4, tools=0, root=u1, body=2 bytes)',
    ].join('\n'),
  })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J8 真实模型调用 2 次 ≠ 入账 1 次/)
})

test('反向②：一组往返缺终态（也没有挂起 sent）⇒ J1 红', () => {
  const facts = base().filter((e) => e.kind !== 'response')
  // 去掉终态后，收束格的溯源位要跟着改到前一条仍存在的事实上，避免连带 J5 噪音。
  const final = facts.find((e) => e.kind === 'chat.assistant.final')
  final.produced_by = 3
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J1 model\/model:u1#1: 缺终态/)
})

test('反向③：协议事实不带 ring: 身份 ⇒ J6 红（形状判据，端口白名单已退场）', () => {
  const facts = base()
  // 一条没有 ring: 身份的协议事实：groupRounds / J1 不会为它建笔，J2 只管 request，
  // 只有 J6 会因为「端口无从判定」判红——钉住形状判据没随白名单一起被放宽。
  // （端口名不在静态表已不再是失效，见本文件末尾的动态端口准入夹具。）
  facts.push(ev(6, 'sent', 'orphan-event-6', { deadline_ms: 9_000_000_000_000 }))
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J6 .*不是 ring: 身份/)
})

test('反向④：sent 的 seq 晚于终态 ⇒ J3 红（引擎写 before 的顺序绕不过）', () => {
  const facts = base()
  const sent = facts.find((e) => e.kind === 'sent')
  const response = facts.find((e) => e.kind === 'response')
  // 只换 seq，不换数组次序：判定器按 event_id 分组、按 seq 比先后，正是这条判据要钉的
  // 「派发锚点必须在应答之前」。终态的溯源位一并指回配置格，避免连带 J5 噪音。
  sent.seq = 4
  response.seq = 2
  response.produced_by = 1
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J3 model\/model:u1#1: sent#4 不在终态之前/)
})

test('反向⑤：收束格 reason 为空 ⇒ J4 红（结局必须有名）', () => {
  const facts = base()
  facts.find((e) => e.kind === 'chat.assistant.final').payload.reason = ''
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J4 收束格#5 的 reason 为空/)
})

// ── J1 的挂起补条：多一条 sent 不是「两笔派发」 ───────────────────────────

test('挂起补条（deadline_ms: null + shard）不算第二笔派发，也不要求终态', () => {
  const facts = base()
  facts.splice(5, 0, ev(5, 'sent', ring('model', 'model:u1', 1, 'sent', 1), { deadline_ms: null, shard: 1 }))
  // 收束格顺延一位；它的溯源位仍指前一格（挂起补条），仍满足 J5。
  for (const e of facts.slice(6)) e.seq += 1
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

test('可重试失败：retry 落在**下一笔的键**上，被重试掉的一笔不算缺终态', () => {
  const facts = [
    ev(0, 'user.message', 'u1', { id: 'u1', text: '第一轮' }),
    ev(1, 'session.config', 'session-config:u1', { classify_enabled: true, compose_enabled: true }),
    // 第一次尝试：只有 sent + request（HTTP 500 ⇒ 可重试）
    ev(2, 'sent', ring('model', 'model:u1', 1, 'sent'), { deadline_ms: 9_000_000_000_000 }),
    ev(3, 'request', ring('model', 'model:u1', 1, 'request'), {}),
    // retry 写在 attempt=2 的键上，`from: 1` 指回本次尝试（engine.rs 的口径）
    ev(4, 'retry', ring('model', 'model:u1', 2, 'retry'), { from: 1, class: 'retryable', reason: 'boom' }),
    ev(5, 'sent', ring('model', 'model:u1', 2, 'sent'), { deadline_ms: 9_000_000_000_000 }),
    ev(6, 'request', ring('model', 'model:u1', 2, 'request'), {}),
    ev(7, 'response', ring('model', 'model:u1', 2, 'response'), { port: 'model' }),
    ev(8, 'chat.assistant.final', 'chat.assistant.final:u1', { root_id: 'u1', reason: 'completed' }),
  ]
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

// ── J7 双向 ────────────────────────────────────────────────────────────────
test('J7 分类路径：两组三格只有一次判决观察 ⇒ 红（记了账没人发起）', () => {
  const facts = base()
  let seq = 6
  seq = round(facts, seq, 'model.assist', 'classify.decide:u1')
  seq = round(facts, seq, 'model.assist', 'classify.decide:u2') // 第二笔无判决的往返
  verdict(facts, seq, 'u1', 'classify.escalate', 'needs_work')
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J7 classify\/decide: 入账三格 2 组 > 观测 1 次/)
})

test('J7 措辞路径：from_context 判决没有对应的 compose 往返 ⇒ 红', () => {
  const facts = base()
  let seq = 6
  seq = round(facts, seq, 'model.assist', 'classify.decide:u1')
  verdict(facts, seq, 'u1', 'classify.answered', 'from_context')
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J7 compose\/wording: 观测 1 次、入账三格 0 组/)
})

test('J7 分类路径的正向：一笔判决 + 一笔往返 ⇒ 通过', () => {
  const facts = base()
  let seq = 6
  seq = round(facts, seq, 'model.assist', 'classify.decide:u1')
  verdict(facts, seq, 'u1', 'classify.escalate', 'needs_work')
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

test('J7 的 compose 守卫：措辞产线关掉时 from_context 不要求三格（不是缺口）', () => {
  const facts = base()
  const cfg = facts.find((e) => e.kind === 'session.config')
  cfg.payload.compose_enabled = false
  let seq = 6
  seq = round(facts, seq, 'model.assist', 'classify.decide:u1')
  verdict(facts, seq, 'u1', 'classify.answered', 'from_context')
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
})

// ── J9 压缩产物 ⇔ 摘要往返 ─────────────────────────────────────────────────

test('J9：压缩产物能指名到一笔已收束的摘要往返 ⇒ 通过', () => {
  const facts = base()
  let seq = 6
  seq = round(facts, seq, 'model.assist', 'compact-summarize:fp1')
  facts.push(ev(seq, 'session.consolidated', 'session.consolidated:fp1', { root_id: 'u1' }))
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /J9 通过/)
})

test('J9：压缩产物没有配对的摘要往返 ⇒ 红（产物不是模型产出的）', () => {
  const facts = base()
  facts.push(ev(6, 'session.consolidated', 'session.consolidated:fp2', { root_id: 'u1' }))
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J9 ring-test 压缩产物 session\.consolidated:fp2/)
  assert.match(out(r), /找不到配对的摘要往返/)
})

// ── J2 的 scope 边界：终止后的 scope 不许再接派发 ─────────────────────────

test('J2：scope 终止后没有新输入格就派发 ⇒ 红', () => {
  const facts = base()
  facts.push(
    ev(6, 'session.terminated', 'session.terminated:u1', { root_id: 'u1', reason: 'completed' }),
  )
  round(facts, 7, 'model', 'model:u1:late')
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /J2 request#8 落在已终止的 scope/)
})

// ── 动态端口准入（D-447 ①）：静态白名单退场后的回归夹具 ───────────────────

test('动态端口准入：含未在静态表中的自定义端口 custom_probe ⇒ J6 通过，且有「新端口」提示', () => {
  // 旧版在这里放的是「扫源码声明 ↔ DECLARED_PORTS 逐项比对」的漂移守卫：多一个端口
  // 不改表就红。它把「加端口先改验收表」钉成门禁，与 D-447 ① 正面冲突，随静态白名单
  // 一起退场（见 obligation-ring-accept.mjs 文件头退场 ④）。承接夹具就是这一条：
  // `custom_probe` 既不在静态表里、也不在源码里，一笔完整合规往返必须照样过 J6——
  // 端口准入读的是**实测端口集**，不是那张表。
  const facts = base()
  round(facts, 6, 'custom_probe', 'probe:u1')
  const r = accept({ live: lines(...facts) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /全项通过/)
  assert.match(out(r), /新端口 "custom_probe"/)
  // 另一侧漂移也只提示、不判红：静态表里多数声明端口本次没出现。
  assert.match(out(r), /声明端口.*未出现/)
})
