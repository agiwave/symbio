#!/usr/bin/env node
/**
 * obligation-ring-accept — 义务环符合性的 **CLI 实测判定**（[T-236] 批口径 1 的第三件套）
 *
 * ## 它治的病
 *
 * 「跑了一遍 CLI，看着日志挺全」不是证据。判据必须能在**没有人读日志**的情况下重跑，
 * 否则同一次实测在不同人手里得出不同结论——[T-238] 登记的缺口正是这样活了很久：
 * 短路轮两次打模型、协议格为 0，而代码里到处是「每次往返都有三格」的注释。
 *
 * ## 输入 = 一份 CLI 实测产物，不是构造的夹具
 *
 * 读 `<homedir>/session/<sid>/v2-events.wal`（事实层唯一真相）。`--cli-log` 可选：
 * 给了它才能判 J8（账实相符），因为它要数**真实发生的模型 HTTP 调用**——这件事
 * 在 WAL 里原理上不可见（一次没入账的调用恰好不会留下任何格）。
 *
 * ## 判据（逐条，编号即输出编号）
 *
 * | # | 判据 | 口径来源 |
 * |---|------|----------|
 * | J1 | 每一组端口往返（按 `payload.id` 分组）四要素齐整：`request` → `sent` → 终态（`response` \| `failed`），且**有且只有一次** `request` | PL-016 §4 五事实 |
 * | J2 | `user.message` 的 seq 先于本轮任何协议 `request` | PL-017 判据 2（溯源不悬空的时间面） |
 * | J3 | `sent` 的 seq 严格小于终态的 seq | PL-016 §4（引擎写 before，应答在后） |
 * | J4 | 收束格（`chat.assistant.final`）的 `reason` 非空 | PL-016 §6 结局必须有名 |
 * | J5 | 每个事件的 `produced_by` 指向一条**已存在且更早**的 seq | PL-016 §4 溯源不悬空 |
 * | J6 | 孤名 = 0：`request` 的 `port` 必须在端口声明集里 | T-239（装载期硬拒的实测对账） |
 * | J7 | 模型往返的**观测 ⇔ 三格**双向对账（`MODEL_PATHS` 逐条）：有观测没三格 ⇒ 一条绕开债环的调用；有三格没观测 ⇒ 记了账没人发起 | PL-017 判据 2 全量口径 + [T-238] |
 * | J8 | 给了 `--cli-log` 时：真实模型 HTTP 调用数 == `sent(port=model)` 数 + J7 判出的豁免未入账数 | PL-016 §0「没有别的调度代码」 |
 *
 * ## J7/J8 为什么需要一张**豁免表**，以及它为什么不会变成长期的绿灯
 *
 * `classify` 的模型调用发生在**插件内部**：编排层在调用之前无法知道这次会不会打模型
 * （规则表命中 ⇒ 零往返），而 `sent` 的语义是「已经派出去了」——调用前补一笔
 * `request/sent` 就是把没发生的往返记成发生了，日志从此数不出模型调用。
 * 把它接进环需要一次契约裁决，登记在 [T-247]（`next` · 期限见该任务）。
 *
 * 压缩摘要有模型调用时同样未入账，但它在事实层**没有观测**（`turn.stage{compress}`
 * 记的是「这一步跑过」，不是「打了模型」），J7 判不到它——J8 判得到：多出来的那次
 * HTTP 调用减不平 ⇒ 红。
 *
 * 豁免表**只许减不许加**由三条机制保证：① 每条豁免必须带任务号与期限，期限过了就红；
 * ② `--strict` 下「有条目却无观测」也红（已进环就该删）；③ 差额一旦超过豁免额，J8 红。
 *
 * 用法：
 *   node scripts/obligation-ring-accept.mjs --homedir=.symbio --session=verify-ring-3 \
 *        [--session=verify-ring-2] [--cli-log=<file>] [--strict]
 * 退出码：0 = 全项通过；1 = 有失效；2 = 输入读不出（会话目录 / WAL 缺失 / JSON 不可解析）。
 */

import fs from 'node:fs'
import path from 'node:path'
import { green, red, dim } from './color.mjs'

// 判据码命名空间：本脚本不占 `GATE_CODES.md` 的编号空间（它不是门禁步骤，
// 判据编号 J1…J8 只在本文件与任务 evidence 里被引用）。

/** 五个协议事实名（PL-016 §4：事实名是闭集，端口是载荷）。 */
const PROTOCOL = new Set(['request', 'sent', 'response', 'failed'])
/** 终态：一次往返的两种结局。 */
const TERMINAL = new Set(['response', 'failed'])

/**
 * 端口声明集的**实测对账口径**：`loop_rows::PORTS` 的 `id` 列表。
 *
 * 这里写成一份常量而不是去解析 Rust 源码：J6 判的是「CLI 跑起来的这次装载里，
 * 有没有人往一个没人消费的端口上写事实」，它要的是**运行时事实**与声明的对账。
 * 声明侧的静态对账（行引用了未声明的端口 ⇒ 拒绝启动）已经在装载期判掉了，
 * 那部分是 [T-239] 的 `ring.rs` 静态审计（`Decl` 引用不到就 Err，不成告警）。
 * 两侧名单若漂移，本常量会被 J6 判红（写了 `port=foo` 的事实却在声明集里查不到）。
 */
const DECLARED_PORTS = ['turn', 'model', 'tool']

/**
 * **模型往返的入账路径表**（J7 的判据本体）。
 *
 * 一条路径 = 「事实面上的某个观测 ⇒ 必然对应一组 `via` 相同的 port 三格」。
 * 两个方向都判，缺哪个都红：
 *  - 只有三格没有观测 ⇒ 一次没人发起的往返（不会发生，但形状上成立）；
 *  - 只有观测没有三格 ⇒ **一条绕开债环的模型调用**——[T-238] 登记的正是这一形态。
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
    observed: (f) => f.kind === 'turn.stage' && f.payload?.stage === 'decide' && f.payload?.reason === 'from_context',
    exemption: null,
    why: '短路轮的答话组织：编排层发起的那一次模型往返（T-238）',
  },
  {
    via: 'classify/decide',
    port: 'model',
    observed: (f) => f.kind === 'turn.stage' && f.payload?.stage === 'decide' && f.payload?.source === 'model',
    exemption: {
      task: 'T-247',
      until: '2026-11-05',
      why: '判决往返在 classify 插件内部发起：编排层在调用前不知道规则表会不会命中，而 `sent` 的语义是「已派出」，调用前补记等于把没发生的往返记成发生了',
    },
    why: '判决自身的那一次分类调用',
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

/** 读一份 WAL（每行一个 JSON 事件）。读不出 ⇒ 抛错，由调用方转成退出码 2。 */
function readWal(file) {
  const text = fs.readFileSync(file, 'utf8')
  const events = []
  for (const [i, line] of text.split('\n').entries()) {
    const t = line.trim()
    if (!t) continue
    try {
      events.push(JSON.parse(t))
    } catch (e) {
      throw new Error(`${file} 第 ${i + 1} 行不是合法 JSON: ${e.message}`)
    }
  }
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
 * J7：按 `MODEL_PATHS` 逐条判「观测 ⇔ 三格」。
 *
 * 返回 `unaccounted` = 命中豁免、确实没入账的往返笔数（J8 的账实减法用它）。
 */
function judgePaths(events, rounds, strict, note) {
  let unaccounted = 0
  for (const p of MODEL_PATHS) {
    const observations = events.filter((e) => p.observed(e))
    const groups = [...rounds.values()].filter((g) =>
      g.some((f) => f.kind === 'request' && f.payload?.via === p.via && f.payload?.port === p.port),
    )
    if (observations.length > groups.length) {
      if (!p.exemption) {
        note(`J7 ${p.via}: 观测 ${observations.length} 次、入账三格 ${groups.length} 组——有 ${observations.length - groups.length} 次模型往返没进债环`)
        continue
      }
      if (new Date(p.exemption.until).getTime() < Date.now()) {
        note(`J7 ${p.via}: 豁免 ${p.exemption.task} 已过期限 ${p.exemption.until}，仍未闭`)
        continue
      }
      unaccounted += observations.length - groups.length
    }
    if (groups.length > observations.length) {
      note(`J7 ${p.via}: 入账三格 ${groups.length} 组 > 观测 ${observations.length} 次——记了账却没人发起这条路径`)
    }
    // 豁免条目还在、观测却一次都没有 ⇒ 要么已进环，要么这次没走判决路径。
    // 只在 `--strict` 下判红：后者是合法运行形态（规则短路 / 未挂载 classify），
    // 判它红会把人训练成关掉 strict。
    if (strict && p.exemption && observations.length === 0) {
      note(`J7 ${p.via}: 豁免表（${p.exemption.task}）在本实测里没有观测——已进环就删条目，别留假绿灯`)
    }
  }
  return unaccounted
}

function judgeSession(homedir, sid, strict) {
  const wal = path.join(homedir, 'session', sid, 'v2-events.wal')
  if (!fs.existsSync(wal)) return { fatal: `读不到 WAL: ${wal}` }
  let events
  try {
    events = readWal(wal)
  } catch (e) {
    return { fatal: e.message }
  }
  if (events.length === 0) return { fatal: `${wal} 是空的` }

  const rounds = groupRounds(events)
  const problems = []
  const note = (m) => problems.push(m)

  // J1 齐整（唯一 request、有 sent、恰一个终态）+ J3 顺序
  for (const [id, g] of rounds) {
    const requests = g.filter((f) => f.kind === 'request')
    const sent = g.find((f) => f.kind === 'sent')
    const terminals = g.filter((f) => TERMINAL.has(f.kind))
    if (requests.length !== 1) {
      note(`J1 ${id}: request 有 ${requests.length} 笔（应为 1）—— ${g.map((f) => f.kind).join('/')}`)
    }
    if (!sent) note(`J1 ${id}: 缺 sent`)
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
  const finals = events.filter((e) => e.kind === 'chat.assistant.final')
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

function countHttp(logFile) {
  const text = fs.readFileSync(logFile, 'utf8')
  // `[LLM] 请求发起 <endpoint> (model=..., msgs=..., ...)` —— 每一次真实 HTTP 调用一行。
  return text.split('\n').filter((l) => l.includes('[LLM] 请求发起')).length
}

const args = parseArgs(process.argv.slice(2))
let totalProblems = 0
const summary = []
for (const sid of args.sessions) {
  const r = judgeSession(args.homedir, sid, args.strict)
  if (r.fatal) {
    console.log(red(`✗ ${sid}: ${r.fatal}`))
    process.exit(2)
  }
  // J8 账实相符（需要 CLI 日志）
  if (args.cliLog) {
    const http = countHttp(args.cliLog)
    const sentModel = r.events.filter((e) => e.kind === 'sent' && e.payload?.port === 'model').length
    const expect = sentModel + r.unaccounted
    if (http !== expect) {
      r.problems.push(`J8 真实模型调用 ${http} 次 ≠ 入账 ${sentModel} + 未入账判决 ${r.unaccounted} = ${expect}（差额 = 一条没人记账的模型路径）`)
    }
    summary.push(`${sid}: HTTP ${http} / sent(model) ${sentModel} / 未入账判决 ${r.unaccounted}`)
  } else {
    summary.push(`${sid}: HTTP - / sent(model) ${r.events.filter((e) => e.kind === 'sent' && e.payload?.port === 'model').length} / 往返 ${r.rounds.size} 组`)
  }
  if (r.problems.length === 0) {
    console.log(green(`✓ ${sid}: J1…J7 全项通过（${r.events.length} 格 / ${r.rounds.size} 组端口往返）`))
  } else {
    totalProblems += r.problems.length
    console.log(red(`✗ ${sid}: ${r.problems.length} 项失效（${r.events.length} 格 / ${r.rounds.size} 组往返）`))
    for (const p of r.problems) console.log(red(`    ${p}`))
  }
}
for (const s of summary) console.log(dim(`    ${s}`))
if (!args.cliLog) console.log(dim('    J8 未判：没有 --cli-log（账实相符需要真实 HTTP 计数）'))

console.log(totalProblems === 0 ? green('义务环实测判定：全项通过') : red(`义务环实测判定：${totalProblems} 项失效`))
process.exit(totalProblems === 0 ? 0 : 1)
