#!/usr/bin/env node
/**
 * obligation-ring-accept — 义务环符合性的 **CLI 实测判定**（[] 批口径 1 的第三件套）
 *
 * ## 它治的病
 *
 * 「跑了一遍 CLI，看着日志挺全」不是证据。判据必须能在**没有人读日志**的情况下重跑，
 * 否则同一次实测在不同人手里得出不同结论——[] 登记的缺口正是这样活了很久：
 * 短路轮两次打模型、协议格为 0，而代码里到处是「每次往返都有三格」的注释。
 *
 * ## 输入 = 一份 CLI 实测产物，不是构造的夹具
 *
 * 读 `<homedir>/session/<sid>/v2-events.wal`（事实层唯一真相）**及其归档段**
 * （`transcripts/wal-archive/`，`archive_before` 搬走的前缀；见 `readSessionEvents`）。
 * 只读 live 会把每一段被归档的历史判成悬空引用——会话一长就满屏假红。`--cli-log` 可选：
 * 给了它才能判 J8 / J9（账实相符与具名归因），因为它们要数**真实发生的模型 HTTP 调用**——这件事
 * 在 WAL 里原理上不可见（一次没入账的调用恰好不会留下任何格）。
 *
 * ## 判据（逐条，编号即输出编号）
 *
 * | # | 判据 | 口径来源 |
 * |---|------|----------|
 * | J1 | 每一组端口往返（按 `payload.id` 分组）四要素齐整：`request` → `sent` → 终态（`response` \| `failed`），且**有且只有一次** `request`。**例外**：被拒的那一笔（`authz.denied`）没有 `sent` 是对的——没放行就没派发（e2e T78 的同一条判据）；反过来「拒了却派发了」判红 | DE-015 §4 五事实 |
 * | J2 | `user.message` 的 seq 先于本轮任何协议 `request` | DE-015 判据 2（溯源不悬空的时间面） |
 * | J3 | `sent` 的 seq 严格小于终态的 seq | DE-015 §4（引擎写 before，应答在后） |
 * | J4 | 收束格（`chat.assistant.final` \| `chat.assistant.fallback`，两种都算）的 `reason` 非空 | DE-015 §6 结局必须有名 |
 * | J5 | 每个事件的 `produced_by` 指向一条**已存在且更早**的 seq | DE-015 §4 溯源不悬空 |
 * | J6 | 孤名 = 0：`request` 的 `port` 必须在端口声明集里 | （装载期硬拒的实测对账） |
 * | J7 | 模型往返的**观测 ⇔ 三格**双向对账（`MODEL_PATHS` 逐条）：有观测没三格 ⇒ 一条绕开债环的调用；有三格没观测 ⇒ 记了账没人发起。观测一律读事实层（WAL） | DE-015 判据 2 全量口径 + [] |
 * | J8 | 给了 `--cli-log` 时：真实模型 HTTP 调用数 == `sent(port=model)` 数 + J7 判出的豁免未入账数 + J9 的**具名项**（轮外重试）。`--strict` 下没给日志即判红 | DE-015 §0「没有别的调度代码」 |
 * | J9 | 具名项不许超编：`[Compress] 压缩 LLM 请求开始` 数 − `sent(via=compress/summary)` 数 **≤** `[Wake] RetryCompaction` 数。多出来的 = 一条新长出的静默路径；负数 = 账比日志多 | DE-015 §5.2 / D-131（轮外重试不入账的**机器可判**边界） |
 *
 * ## 观测口径为什么必须是「这次调用打过模型」，不是「判决来自模型」
 *
 * `turn.stage{decide}` 的 `source` 记的是判决的**来源**（规则 / 模型 / 兜底），不是
 * 有没有调用：模型调了而应答不可解析 ⇒ `Escalate{unclassified}` ⇒ `source=fallback`，
 * 这次调用在事实层零痕迹。拿它当观测会同时假红（豁免额减不平、J8 数不拢）和假绿
 * （把「模型没答上」读成「没找模型」）。
 *
 * 所以观测落在同一格的**另一个字段**上：`model_calls`（这次判决打了几次模型）。
 * 它由**派发那一步**数出来（闭包里数，不是靠返回值推断），因此「派发被拒（执行体
 * 压根没跑）= 0」与「跑完但落格失败 = 1」分得开。
 *
 * ## J7/J8 为什么需要一张**豁免表**，以及它为什么不会变成长期的绿灯
 *
 * 豁免表是**临时桥**：某条模型路径还没进环时，先在这里记一笔「已知未入账」，带上
 * 任务号与期限，J8 的等式就暂时平得了。两条路径（判决 / 措辞）都进环之后，表是
 * **空的**——空表才是常态，豁免只许减不许加。三条机制保证它减得干净：
 *
 * ① 每条豁免必须带任务号与期限，期限过了就红；
 * ② `--strict` 下「有条目却无观测」也红（已进环就该删）；
 * ③ 差额一旦超过豁免额，J8 红。
 *
 * 用法：
 *   node scripts/obligation-ring-accept.mjs --homedir=.symbio --session=verify-ring-3 \
 *        [--session=verify-ring-2] [--cli-log=<file>] [--strict]
 * 退出码：0 = 全项通过；1 = 有失效；2 = 输入读不出（会话目录 / WAL 缺失 / JSON 不可解析）。
 *
 * ⚠️ `--cli-log` 那份日志必须是**一次 CLI 实测的产物**，且它的跨度 = 所列会话的跨度
 * （多会话时 J8 只能整轮判）。`[LLM] 请求发起` 是 **INFO** 级 ⇒ 不带 `-v` 也有；
 * 少了它 J8 / J9 数不出真实 HTTP 调用数，只能报「未判」——`--strict` 下那判红。
 */

import fs from 'node:fs'
import path from 'node:path'
import { green, red, dim } from './color.mjs'

// 判据码命名空间：本脚本不占 `GATE_CODES.md` 的编号空间（它不是门禁步骤，
// 判据编号 J1…J9 只在本文件与任务 evidence 里被引用）。

/** 五个协议事实名（DE-015 §4：事实名是闭集，端口是载荷）。 */
const PROTOCOL = new Set(['request', 'sent', 'response', 'failed'])
/** 终态：一次往返的两种结局。 */
const TERMINAL = new Set(['response', 'failed'])

/**
 * 端口声明集的**实测对账口径**：`loop_rows::PORTS` 的 `id` 列表。
 *
 * 这里写成一份常量而不是去解析 Rust 源码：J6 判的是「CLI 跑起来的这次装载里，
 * 有没有人往一个没人消费的端口上写事实」，它要的是**运行时事实**与声明的对账。
 * 声明侧的静态对账（行引用了未声明的端口 ⇒ 拒绝启动）已经在装载期判掉了，
 * 那部分是 [] 的 `ring.rs` 静态审计（`Decl` 引用不到就 Err，不成告警）。
 * 两侧名单若漂移，本常量会被 J6 判红（写了 `port=foo` 的事实却在声明集里查不到）。
 */
const DECLARED_PORTS = ['turn', 'model', 'tool']

/**
 * **模型往返的入账路径表**（J7 的判据本体）。
 *
 * 一条路径 = 「事实面上的某个观测 ⇒ 必然对应一组 `via` 相同的 port 三格」。
 * 两个方向都判，缺哪个都红：
 *  - 只有三格没有观测 ⇒ 一次没人发起的往返（不会发生，但形状上成立）；
 * - 只有观测没有三格 ⇒ **一条绕开债环的模型调用**——[] 登记的正是这一形态。
 *
 * `observed` 返回**权重**（这一格代表几次模型往返），求和即得观测次数。权重大于 1
 * 是真实情形：`turn.stage{decide}` 一格里记着这一轮判决打了几次模型。
 *
 * `exemption` 非空 = 这条路径**已知未入账**，豁免带任务号与期限（**只许减不许加**，
 * 见文件头）。期限过了、或观测从此消失（说明已进环）却没删条目，J7 都判红。
 */
const MODEL_PATHS = [
  {
    via: 'compose/wording',
    port: 'model',
    // `Answered{from_context}` 的含义是「答案已在对话里」⇒ 措辞必须由模型组织出来
    // （`Verdict::needs_wording_generation` 是同一条规则的唯一实现，两侧共用）。
    // 这条理由码只由**模型**产出（规则表只有寒暄 / 谢谢 / 确认 / 空话四档），所以
    // 「见过一次 `from_context`」= 「措辞产线打过一次模型」。
    observed: (f) =>
      f.kind === 'turn.stage' &&
      f.payload?.stage === 'decide' &&
      f.payload?.reason === 'from_context'
        ? 1
        : 0,
    exemption: null,
    why: '短路轮的答话组织：编排层发起的那一次模型往返（T-238）',
  },
  {
    via: 'classify/decide',
    port: 'model',
    // 判决往返的观测读**次数**，不读**来源**：`turn.stage{decide}.source` 记的是判决
    // 的来源（`rule` / `model` / `fallback` / `absent`），模型调了而应答不可解析 ⇒
    // 落 `Escalate{unclassified}` ⇒ `source=fallback`——那次调用在事实层不留痕迹。
    // `model_calls` 才是「这次判决打了几次模型」这个数（派发那一步数出来的），
    // 它与三格组数一一对应：规则短路 = 0 格 0 次。
    observed: (f) =>
      f.kind === 'turn.stage' && f.payload?.stage === 'decide'
        ? (f.payload?.model_calls ?? 0)
        : 0,
    exemption: null,
    why: '判决自身的那一次分类调用',
  },
  {
    via: 'compress/summary',
    port: 'model',
    // 自动语义压缩的摘要请求，可能**两笔**（快照校验不过时有一次纠正重试）——
    // 所以观测读的是 `turn.stage{compress}.model_calls`（派发那一步数出来的），
    // 而不是「见过一次 compress 格」：一格代表 0..2 次往返。
    // 续写幕会重跑压缩，那格按幕锚各落一份，数不会被幂等键吞掉。
    observed: (f) =>
      f.kind === 'turn.stage' && f.payload?.stage === 'compress'
        ? (f.payload?.model_calls ?? 0)
        : 0,
    exemption: null,
    why: '语义压缩的摘要往返（含纠正重试）',
  },
]

function parseArgs(argv) {
  const out = { homedir: null, sessions: [], cliLog: null, strict: false }
  for (const a of argv) {
    const [k, ...rest] = a.slice(2).split('=')
    const v = rest.join('=')
    if (k === 'homedir') out.homedir = v
    else if (k === 'session') out.sessions.push(v)
    else if (k === 'cli-log') out.cliLog = v
    else if (k === 'strict') out.strict = true
    else {
      console.error(red(`未知参数: ${a}`))
      process.exit(2)
    }
  }
  if (!out.homedir || out.sessions.length === 0) {
    console.error(red('用法: --homedir=<dir> --session=<sid> [--session=<sid> ...] [--cli-log=<file>]'))
    process.exit(2)
  }
  return out
}

/** 读一份 WAL（每行一个 JSON 事件）。撕裂尾行停止重放——与 `store/wal.rs::replay` 同口径。 */
function replayFile(file) {
  const events = []
  if (!fs.existsSync(file)) return events
  const text = fs.readFileSync(file, 'utf8')
  for (const line of text.split('\n')) {
    const t = line.trim()
    if (!t) continue
    try {
      events.push(JSON.parse(t))
    } catch (e) {
      // 撕裂尾行：后面的内容一律不可信（崩溃安全留的半行），到此为止。
      break
    }
  }
  return events
}

/**
 * 读一次会话的**全部事实**：归档段（按段名首号升序）+ live 段。
 *
 * 为什么不能只读 live：`consolidate` 落格成功后调 `WalStore::archive_before`，把
 * `seq < 快照号` 的前缀整段搬进 `<会话目录>/transcripts/wal-archive/v2-events.wal.<first>-<last>`
 * （`store/wal.rs::archive_events`）。只读 live 的读者会把每一段被归档的历史判成
 * 「produced_by 悬空」「本轮没有输入格」——长会话实测里 20 轮之后 live 只剩 12 格，
 * 连报 16 项假红，而真正的结论（压缩真发生、摘要往返入了账）全在归档段里。
 *
 * 读法与写侧逐字对齐：段名 `<wal 文件名>.<first>-<last>`，`first` 既是段内首号也是
 * 排序键；逐段重放后按 `first` 升序拼接，live 接在后面，**不重排**。目录不存在 /
 * 某段读不开 ⇒ 当作没有（与 Rust 侧「读不开就是没有」同一口径）。
 */
function readSessionEvents(walFile) {
  const events = []
  const dir = path.join(path.dirname(walFile), 'transcripts', 'wal-archive')
  const prefix = `${path.basename(walFile)}.`
  if (fs.existsSync(dir)) {
    const segments = []
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      if (!ent.isFile()) continue
      const rest = ent.name.startsWith(prefix) ? ent.name.slice(prefix.length) : null
      if (rest === null) continue
      const first = Number(rest.split('-')[0])
      if (!Number.isInteger(first)) continue
      segments.push([first, ent.name])
    }
    segments.sort((a, b) => a[0] - b[0])
    for (const [, name] of segments) events.push(...replayFile(path.join(dir, name)))
  }
  events.push(...replayFile(walFile))
  return events
}

/** 按 `payload.id` 把协议事实折成一组往返。 */
function groupRounds(events) {
  const byId = new Map()
  for (const e of events) {
    if (!PROTOCOL.has(e.kind)) continue
    const id = e.payload?.id
    if (typeof id !== 'string' || !id) continue
    if (!byId.has(id)) byId.set(id, [])
    byId.get(id).push(e)
  }
  return byId
}

/**
 * 一条路径的「观测 ⇔ 三格」双向对账（J7 的判据本体，两侧共用）。
 *
 * 返回 `unaccounted` = 命中豁免、确实没入账的往返笔数（J8 的账实减法用它）。
 */
function comparePath(p, observations, groups, strict, note) {
  let unaccounted = 0
  if (observations > groups) {
    if (!p.exemption) {
      note(`J7 ${p.via}: 观测 ${observations} 次、入账三格 ${groups} 组——有 ${observations - groups} 次模型往返没进债环`)
      return 0
    }
    if (new Date(p.exemption.until).getTime() < Date.now()) {
      note(`J7 ${p.via}: 豁免 ${p.exemption.task} 已过期限 ${p.exemption.until}，仍未闭`)
      return 0
    }
    unaccounted = observations - groups
  }
  if (groups > observations) {
    note(`J7 ${p.via}: 入账三格 ${groups} 组 > 观测 ${observations} 次——记了账却没人发起这条路径`)
  }
  // 豁免条目还在、观测却一次都没有 ⇒ 要么已进环，要么这次没走这条路径。
  // 只在 `--strict` 下判红：后者是合法运行形态（规则短路 / 未挂载 classify），
  // 判它红会把人训练成关掉 strict。
  if (strict && p.exemption && observations === 0) {
    note(`J7 ${p.via}: 豁免表（${p.exemption.task}）在本实测里没有观测——已进环就删条目，别留假绿灯`)
  }
  return unaccounted
}

/** J7（WAL 口径）：逐条判能在事实层观测的路径。 */
function judgePaths(events, rounds, strict, note) {
  let unaccounted = 0
  for (const p of MODEL_PATHS) {
    const groups = [...rounds.values()].filter((g) =>
      g.some((f) => f.kind === 'request' && f.payload?.via === p.via && f.payload?.port === p.port),
    ).length
    // 观测次数 = 各格的权重之和（权重大于 1 的情形见 `MODEL_PATHS` 的文档）。
    const observations = events.reduce((n, e) => n + (p.observed(e) || 0), 0)
    unaccounted += comparePath(p, observations, groups, strict, note)
  }
  return unaccounted
}

function judgeSession(homedir, sid, strict) {
  const wal = path.join(homedir, 'session', sid, 'v2-events.wal')
  if (!fs.existsSync(wal)) return { fatal: `读不到 WAL: ${wal}` }
  const events = readSessionEvents(wal)
  if (events.length === 0) return { fatal: `${wal} 是空的` }

  const rounds = groupRounds(events)
  const problems = []
  const note = (m) => problems.push(m)

  // J1 齐整（唯一 request、有 sent、恰一个终态）+ J3 顺序
  for (const [id, g] of rounds) {
    const requests = g.filter((f) => f.kind === 'request')
    const sent = g.find((f) => f.kind === 'sent')
    const terminals = g.filter((f) => TERMINAL.has(f.kind))
    // **被拒的那一笔没有 `sent` 是对的**：没放行就没派发（e2e T78 判的正是这一条）。
    // 所以「缺 sent」只在**没被拒**时判红；被拒的判据另取：拒了却派发了 ⇒ 判红。
    const denied = events.some((e) => e.kind === 'authz.denied' && e.payload?.id === id)
    if (requests.length !== 1) {
      note(`J1 ${id}: request 有 ${requests.length} 笔（应为 1）—— ${g.map((f) => f.kind).join('/')}`)
    }
    if (!sent && !denied) note(`J1 ${id}: 缺 sent`)
    if (denied && sent) note(`J1 ${id}: 授权被拒却派发了（拒 ⇒ 不得有 sent）`)
    if (terminals.length !== 1) note(`J1 ${id}: 终态（response|failed）有 ${terminals.length} 笔（应为 1）`)
    // J3 sent 先于终态
    if (sent && terminals.length > 0 && !terminals.every((t) => t.seq > sent.seq)) {
      note(`J3 ${id}: sent#${sent.seq} 不在终态之前`)
    }
  }
  // J2 输入格先于判决：每一组 `request` 所在轮，必须已有一条**更早**的同轮 `user.message`
  // （判据 12 的输入闭包：轮的一切下游事实都锚在它的输入格之后）
  const users = events.filter((e) => e.kind === 'user.message')
  if (users.length === 0) note('J2 没有 user.message 格（这次实测没有输入？）')
  for (const e of events) {
    if (e.kind !== 'request') continue
    const inTurn = users.filter((u) => u.turn === e.turn && u.seq < e.seq)
    if (inTurn.length === 0) {
      note(`J2 request#${e.seq}（turn=${e.turn}）之前没有同轮 user.message 输入格`)
    }
  }
  // J4 收束格 reason 非空
  //
  // 「收束格」是**两种**：`chat.assistant.final`（正常定稿）与 `chat.assistant.fallback`
  // （模型没答成 / 空答， 步 5c 起由 `close` 执行体独占写）。只认 final 会把
  // **每一条失败轮**都判成「没跑到定稿」——而失败轮恰恰是 D-122 决策 5 要求
  // 「照样有格」的那一轮。判据的口径必须与方案里的收束格集合一致。
  const CLOSING = ['chat.assistant.final', 'chat.assistant.fallback']
  const finals = events.filter((e) => CLOSING.includes(e.kind))
  if (finals.length === 0) note('J4 没有收束格（这次实测没跑到定稿）')
  for (const f of finals) {
    if (typeof f.payload?.reason !== 'string' || !f.payload.reason) note(`J4 收束格#${f.seq} 的 reason 为空`)
  }
  // J5 produced_by 不悬空
  const seqs = new Set(events.map((e) => e.seq))
  for (const e of events) {
    const pb = e.produced_by
    if (pb === null || pb === undefined) continue
    if (!seqs.has(pb)) note(`J5 ${e.kind}#${e.seq} 的 produced_by=${pb} 不存在`)
    else if (pb >= e.seq) note(`J5 ${e.kind}#${e.seq} 的 produced_by=${pb} 不早于自己`)
  }
  // J6 端口都在声明集里
  for (const e of events) {
    if (!PROTOCOL.has(e.kind)) continue
    const port = e.payload?.port
    if (!DECLARED_PORTS.includes(port)) note(`J6 ${e.kind}#${e.seq} 的端口 "${port ?? '(缺)'}" 未在声明集里`)
  }
  // J7 观测 ⇔ 三格（含豁免表）；返回值给 J8 做减法
  const unaccounted = judgePaths(events, rounds, strict, note)
  return { events, rounds, sid, problems, unaccounted }
}

/** 数真实模型 HTTP 调用：`[LLM] 请求发起 <endpoint> (model=..., msgs=..., ...)` 一次调用一行。 */
function countHttp(logLines) {
  return logLines.filter((l) => l.includes('[LLM] 请求发起')).length
}

/**
 * 数**用户主动重试压缩**的次数（J8 的具名项 / J9 的上界）。
 *
 * 这一行由 `pipeline.rs::retry_compaction` 在找到压缩节点之后立刻打——它是这条路径
 * 在日志上**唯一**的具名痕迹（WAL 里没有：轮外往返按定义不入账）。所以「具名」不是
 * 靠一张表，而是靠**这一行在代码里的位置**：删掉它，J9 立刻判「差额没人解释」。
 */
function countOutOfTurn(logLines) {
  return logLines.filter((l) => l.includes('[Wake] RetryCompaction')).length
}

/**
 * 数**真的打到模型的压缩摘要**次数（J9 的左边）。
 *
 * `compress_with_snapshot_core` 在调模型之前打这一行，**两个分支都打**（轮内自动
 * 压缩与轮外重试）——所以它是「这一次摘要往返发生过」的完整观测，而 WAL 里的
 * `sent(via=compress/summary)` 只是**入账了的那部分**。两者的差就是本判据要说的那件事。
 */
function countCompressStarts(logLines) {
  return logLines.filter((l) => l.includes('[Compress] 压缩 LLM 请求开始')).length
}

const args = parseArgs(process.argv.slice(2))
let logLines = null
if (args.cliLog) {
  try {
    logLines = fs.readFileSync(args.cliLog, 'utf8').split('\n')
  } catch (e) {
    console.log(red(`✗ 读不到 CLI 日志: ${args.cliLog}（${e.message}）`))
    process.exit(2)
  }
}
let totalProblems = 0
const summary = []
const judged = []
for (const sid of args.sessions) {
  const r = judgeSession(args.homedir, sid, args.strict)
  if (r.fatal) {
    console.log(red(`✗ ${sid}: ${r.fatal}`))
    process.exit(2)
  }
  r.sentModel = r.events.filter((e) => e.kind === 'sent' && e.payload?.port === 'model').length
  judged.push(r)
  summary.push(`${sid}: sent(model) ${r.sentModel} / 往返 ${r.rounds.size} 组 / 豁免未入账 ${r.unaccounted}`)
  if (r.problems.length === 0) {
    console.log(green(`✓ ${sid}: J1…J7（事实层口径）全项通过（${r.events.length} 格 / ${r.rounds.size} 组端口往返）`))
  } else {
    totalProblems += r.problems.length
    console.log(red(`✗ ${sid}: ${r.problems.length} 项失效（${r.events.length} 格 / ${r.rounds.size} 组往返）`))
    for (const p of r.problems) console.log(red(`    ${p}`))
  }
}
for (const s of summary) console.log(dim(`    ${s}`))

// J8 只能**整轮**判：一次 `--cli-log` 覆盖所列全部会话，而日志行内没有会话号——
// 把计数按会话摊开就是编数。文件头的输入约定（日志跨度 = 所列会话的跨度）在这里兑现。
if (logLines) {
  const whole = []
  const http = countHttp(logLines)
  const sentModel = judged.reduce((n, r) => n + r.sentModel, 0)
  const exempted = judged.reduce((n, r) => n + r.unaccounted, 0)

  // **具名项**（D-131）：用户主动重试压缩时那一笔摘要往返没有轮可挂 ⇒ 不入账。
  // 它不进 `MODEL_PATHS` 的豁免表（那张表只许减不许加），而是从日志里**数出来**：
  // `[Compress] 压缩 LLM 请求开始` 两个分支都打，而 WAL 里只有入账了的那部分有三格，
  // 两者之差 = 真的发生过却没入账的摘要往返数。
  const compressStarts = countCompressStarts(logLines)
  const sentSummary = judged.reduce(
    (n, r) =>
      n +
      r.events.filter(
        (e) => e.kind === 'sent' && e.payload?.port === 'model' && e.payload?.via === 'compress/summary',
      ).length,
    0,
  )
  const unaccountedSummary = compressStarts - sentSummary
  const outOfTurn = countOutOfTurn(logLines)

  // J8 的等式里多这一项之后，「差额」才有一个**具名**的解释对象；而这一项自己要被
  // J9 判「不超编」——两半合起来才是「这一笔没入账，且它只能是那一条路径」。
  const expect = sentModel + exempted + unaccountedSummary
  if (http !== expect) {
    whole.push(
      `J8 真实模型调用 ${http} 次 ≠ 入账 ${sentModel} + 豁免未入账 ${exempted} + 未入账摘要 ${unaccountedSummary} = ${expect}（差额 = 一条没人记账的模型路径）`,
    )
  }
  console.log(
    dim(
      `    整轮：HTTP ${http} / sent(model) ${sentModel} / 豁免未入账 ${exempted} / 未入账摘要 ${unaccountedSummary}`,
    ),
  )
  if (whole.length === 0) {
    console.log(green('✓ 整轮：J8 通过'))
  } else {
    totalProblems += whole.length
    for (const p of whole) console.log(red(`    ${p}`))
  }

  // ── J9：具名项不许超编（DE-015 §5.2 / D-131）──────────────────────
  //
  // J8 只说明「差额 = 未入账的摘要往返」，它没说这些摘要往返**从哪来**。J9 补这一半：
  // 每一次真的打到模型却没入账的摘要，都必须由一次**用户主动重试**解释。两个方向都判：
  //  - `unaccountedSummary > outOfTurn` ⇒ 有一条**没被点名**的未入账往返（新长出的静默路径）；
  //  - `unaccountedSummary < 0` ⇒ 三格比日志里的摘要请求还多（日志缺行，或有人拿
  //    `via=compress/summary` 记了不是摘要的往返）。
  //
  // 为什么不用 WAL 判这条边界：轮外那一笔**在事实层零痕迹**——这正是本判据要说的那件事。
  console.log(
    dim(
      `    整轮：摘要请求 ${compressStarts} / 入账三格 ${sentSummary} / 未入账 ${unaccountedSummary} / 轮外重试 ${outOfTurn}`,
    ),
  )
  if (unaccountedSummary > outOfTurn) {
    totalProblems += 1
    console.log(
      red(
        `    J9 未入账的摘要往返 ${unaccountedSummary} 次 > 轮外重试 ${outOfTurn} 次——` +
          `差额只能由「用户主动重试压缩」解释，多出来的是**新长出的静默路径**`,
      ),
    )
  } else if (unaccountedSummary < 0) {
    totalProblems += 1
    console.log(
      red(
        `    J9 入账三格 ${sentSummary} > 摘要请求 ${compressStarts}——账上比日志里多，` +
          '要么日志缺行，要么有人拿 via=compress/summary 记了不是摘要的往返',
      ),
    )
  } else {
    console.log(green('✓ 整轮：J9 通过（未入账的摘要往返全部由具名重试解释）'))
  }
} else if (args.strict) {
  // strict 的意义是「不判等于放行」：J8 / J9 要数真实发生的模型 HTTP 调用，那只在 CLI
  // 日志里——没给日志就是没判过，不能报成全过。
  console.log(red('✗ --strict：没给 --cli-log——J8 / J9（账实相符）未判'))
  totalProblems += 1
} else {
  console.log(dim('    J8 / J9 未判：没有 --cli-log（账实相符需要真实 HTTP 计数）'))
}

console.log(totalProblems === 0 ? green('义务环实测判定：全项通过') : red(`义务环实测判定：${totalProblems} 项失效`))
process.exit(totalProblems === 0 ? 0 : 1)
