#!/usr/bin/env node
/**
 * port-surface-audit — **端口面**：PL-016 §6 判据 1 的可执行形态
 *
 * ## 它判什么
 *
 * | 编号 | 判据 | 它拦的是哪一种走样 |
 * |---|---|---|
 * | PTS-001 | 带端口载荷的**事实名** ⊆ 协议族（`request` / `sent` / `response` / `failed` / `retry`），且端口字段名唯一 | **加端口顺手加事实名**——端口是载荷，不是事实种类（PL-016 §2.4 裁决） |
 * | PTS-002 | 求解器（`owed` / `drive` 所在文件）里**不出现端口字段名与端口取值** | **端口渗进引擎**——引擎一旦认识端口，加端口就得改引擎 |
 * | PTS-003 | 求解器那两个函数的**字节**与基线一致 | 判据 1 的原文口径：「`drive` 与 `owed` 源码 diff = 0」 |
 * | PTS-004 | 基线引用的 ADR **能打开**、理由**非空** | **改判据而不留决策**——基线是判据的一面，改它必须有决策挂得住 |
 *
 * ## 为什么是这四格（而不是「机制键守恒」那种计数）
 *
 * PL-016 §6 判据 1 是本方案最硬的一条：**加一个端口 = 0 个新事实名 + 1 个
 * `PortDecl` + 适配器 + 2 行模板，`drive` 与 `owed` 的源码 diff = 0**。
 * 它不是「键数不变」，是**那两段代码的字节不变**——因为端口一旦渗进引擎，
 * 「加端口」就变成「改引擎」，机制面随端口数线性增长，而端口数没有上界。
 *
 * 判据 1 此前**只有散文**（PL-016 §6 那一行），没有执行方。于是三件事都能悄悄发生：
 * 新端口带出新事实名（协议族闭集被稀释）、引擎里长出 `if port == "…"`（端口渗进）、
 * 引擎为某个端口加了特判（字节变了而没人说得清是谁的锅）。
 *
 * ## 四格的关系（一句话说清谁管什么）
 *
 * - **PTS-001 管「事实名这一侧」**：端口是**载荷**，协议名全局只有 5 条。
 *   新端口 ⇒ 新 `PortDecl` + 适配器 + 2 行，**事实名一个都不加**。
 * - **PTS-002 管「引擎这一侧」**：引擎不读端口字段 ⇒ 它对端口**无感** ⇒
 *   新端口在引擎那边是零成本。这条是**结构性**的，不依赖任何基线。
 * - **PTS-003 管「字节这一侧」**：判据 1 的原话。它比前两条更严也更脆——引擎
 *   因为**别的**原因改了（机制演进），它也会红。所以它的红不是「你错了」，
 *   而是「引擎动过了 ⇒ 同批更新基线并挂一条 ADR」（PTS-004 判那条 ADR 挂不挂得住）。
 *   于是「改引擎」从此不是顺手的事，而是一份决策。
 * - **PTS-004 管「基线本身」**：基线是一处**判据面**，改它等于改判据，
 *   按本仓纪律（`08 §0 纪律 3`）必须有能打开的 ADR。
 *
 * ## 基线在哪、为什么是文件而不是常量
 *
 * `scripts/baselines/port-surface.json`。理由有二：① 基线是**数据**，判据是**判据**，
 *   两者混在一个文件里就分不清「谁改的」；② 回归测试需要**夹具树**——
 *   夹具里的求解器与真仓的字节不同，基线若写死在脚本里，每一份夹具都会红在
 *   PTS-003 上，测试就只剩「怎么都红」一种形态。基线随树走 ⇒ 夹具自带基线。
 *
 * ## 已知边界（写清楚，免得把绿灯当成证明）
 *
 * - 判定基于**文本**而非 AST：求解器的字节取「注释剥掉后的两个函数体」，
 *   所以**只改注释不算改动**（散文不是算法），但改函数签名 / 体里的任何一行动红。
 * - PTS-001 只判**声明层**（`decl/mod.rs` 的 `FactDecl`）。事实真的落不落这个字段、
 *   载荷里有没有真的带上端口，是**装载期 / 运行期**的事（`seal` 的字段检查与
 *   写侧闸），本脚本不重复判。
 * - PTS-002 判**字段名 + 取值**两侧（T-245 口径 ②，把原来自陈的边界关掉）：
 *   字段名在整份求解器文件里查（剥注释前都会红），取值在 `owed` / `drive` 两个
 *   **函数体**里查（同一份剥注释后的文本，与 PTS-003 比对的字节同源）。取值为什么不
 *   查整份文件：`ring.rs` 的信封字段名里就有 `"turn"`（轮号，`fact_field` 的特判），
 *   它与端口 `turn` 的取值同名——那是命名巧合，不是引擎读端口。
 *   还剩的边界：`if port == PORT_TOOL` 这种**经常量**的分支看不见（字面量不在
 *   函数体里）。常量本身会在别处出现，而把「常量名不许出现在求解器」判成红会
 *   连 `MAX_STEPS` 这类无关常量一起红——宁可按字面量判，也不造一条常年需要豁免的规则。
 * - PTS-002 判的是「引擎不**命名**端口字段」。若引擎通过别的键名读同一个值
 *   （`e.payload.get("endpoint")`），本脚本看不见——但那已经违反 PTS-001 的
 *   「端口字段名唯一」，而端口字段名只能由 `FactDecl.port` 声明。
 *
 * ## 用法
 *
 *   node scripts/port-surface-audit.mjs                # 判定
 *   node scripts/port-surface-audit.mjs --print-baseline # 打印当前求解器指纹（改引擎后据此更新基线）
 *   node scripts/port-surface-audit.mjs --root=<dir>    # 指向别处的仓库（回归测试用）
 *
 * 退出码：0 = 通过，1 = 违规。
 */

// 判据码命名空间（登记表 docs/006-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns PTS 端口面（协议名闭集 / 引擎对端口无感 / 求解器字节基线）
// @codes PTS-001 PTS-002 PTS-003 PTS-004

import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'
import { blankComments, matchBrace } from './rust-scan.mjs'
import { stringLiteralsOf } from './gate.d/_shared.mjs'

const argv = process.argv.slice(2)
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const PRINT_BASELINE = argv.includes('--print-baseline')

/** 事件名字表（协议名的唯一 owner）与事实声明表 */
const EVENT_REL = 'symbio/src/symbio_core/event/mod.rs'
const DECL_REL = 'symbio/src/symbio_core/decl/mod.rs'
/** 基线（判据面之一，见文件头「基线在哪」） */
const BASELINE_REL = 'scripts/baselines/port-surface.json'

/**
 * 协议族：**哪五个常量**是协议族（PL-016 §2.4 的「五个协议名」）。
 *
 * 这里钉的是**常量名**，不是字面量——字面量的 owner 是 `event/mod.rs`，
 * 抄一份「request / sent / …」到脚本里就是第二处真相（改了协议名的拼写，
 * 脚本会拿旧名单去判，于是判错）。
 */
const PROTOCOL_CONSTS = [
  'EVENT_REQUEST',
  'EVENT_SENT',
  'EVENT_RESPONSE',
  'EVENT_FAILED',
  'EVENT_RETRY',
]

/**
 * 求解器的两个函数：判据 1 点名的那两段代码。
 *
 * ⚠️ 这是**判据的一部分**，不是配置：改它等于改判据（PL-016 §6 判据 1 的原话是
 * 「`drive` 与 `owed`」）。真要改，走 ADR + 基线更新（PTS-003 / PTS-004）。
 */
const SOLVER_FNS = ['owed', 'drive']

const errors = []
const report = (rule, msg) => errors.push({ rule, msg })

function readText(rel) {
  const abs = path.join(ROOT, rel)
  if (!fs.existsSync(abs)) return null
  return fs.readFileSync(abs, 'utf8').replace(/\r\n/g, '\n')
}

// ───────────────────────── 读取三份输入 ─────────────────────────

/** 事件名字表：`EVENT_*` 常量 → 名字的字面值 */
function eventConstants() {
  const text = readText(EVENT_REL)
  if (text === null) {
    report('PTS-001', `读不出事件名字表：${EVENT_REL} 不存在（root=${ROOT}）—— 判据没有输入，宁红不静默`)
    return null
  }
  const code = blankComments(text).join('\n')
  const map = new Map()
  for (const m of code.matchAll(/pub\s+const\s+(EVENT_[A-Z0-9_]+)\s*:\s*&str\s*=\s*"([^"]*)"/g)) {
    map.set(m[1], m[2])
  }
  return map
}

/** 事实声明表：`FactDecl { name, port, … }` 逐条 */
function factDecls() {
  const text = readText(DECL_REL)
  if (text === null) {
    report('PTS-001', `读不出事实声明表：${DECL_REL} 不存在（root=${ROOT}）—— 判据没有输入，宁红不静默`)
    return null
  }
  const code = blankComments(text).join('\n')
  const out = []
  for (const m of code.matchAll(/\bFactDecl\s*\{/g)) {
    const open = code.indexOf('{', m.index)
    if (open < 0) continue
    const body = code.slice(open + 1, matchBrace(code, open))
    const nameM = /\bname:\s*([A-Za-z0-9_]+|"[^"]*")/.exec(body)
    if (!nameM) continue
    const raw = nameM[1]
    const portM = /\bport:\s*Some\(\s*"([^"]*)"\s*\)/.exec(body)
    out.push({
      /** 常量名（`EVENT_REQUEST`）或字面量名（`"chat.assistant.final"`） */
      token: raw,
      /** 字面值：常量名要回事件名字表查一次 */
      name: raw.startsWith('"') ? raw.slice(1, -1) : null,
      /** 端口载荷字段名；`null` = 这条事实与端口无关 */
      portField: portM ? portM[1] : null,
    })
  }
  return out
}

function readBaseline() {
  const text = readText(BASELINE_REL)
  if (text === null) return null
  try {
    return JSON.parse(text)
  } catch (e) {
    report('PTS-003', `基线 ${BASELINE_REL} 解析失败：${e.message} —— 基线读不出即判据失效，宁红不静默`)
    return null
  }
}

// ───────────────────────── PTS-001 协议名闭集 ─────────────────────────

const consts = eventConstants()
const facts = factDecls()
const protocolNames = new Set()
for (const c of PROTOCOL_CONSTS) {
  const v = consts?.get(c)
  if (v === undefined) {
    report('PTS-001', `协议名常量 \`${c}\` 不在 ${EVENT_REL} 里 —— 协议族的名字必须单点定义（PL-016 §2.4 的五个协议名）`)
  } else {
    protocolNames.add(v)
  }
}

/**
 * 豁免口：第 6 个协议名是**机制变更**，必须挂一条能打开的 ADR。
 *
 * 豁免写在**本脚本自己的源码**里（读自己的源文件，不是读 `ROOT` 下那份）——豁免的
 * 载体是判据本身：它得跟着判据一起被 review、一起被改。放在别处就变成第二处真相。
 */
const WAIVER_RE = /^ {0,2}\/\/\s*port-surface-allow\s+protocol\s+([A-Za-z0-9_.-]+)\s*:\s*(\S[^\n]*)$/gm
const allowedProtocols = new Map()
{
  const selfPath = fileURLToPath(import.meta.url)
  const self = fs.existsSync(selfPath)
    ? fs.readFileSync(selfPath, 'utf8').replace(/\r\n/g, '\n')
    : null
  if (self !== null) {
    for (const m of self.matchAll(WAIVER_RE)) {
      allowedProtocols.set(m[1], m[2].trim())
      const refs = [...m[2].matchAll(/ADR-(\d+[a-z]?)/g)].map((r) => r[1])
      if (refs.length === 0) {
        report('PTS-001', `豁免 \`protocol ${m[1]}\` 的理由里没有 ADR 编号 —— 协议族是闭集，加一个成员是机制变更，必须挂一条决策`)
        continue
      }
      for (const id of refs) {
        if (!fs.existsSync(path.join(ROOT, 'docs', '010-adr', `${id}.md`))) {
          report('PTS-001', `豁免 \`protocol ${m[1]}\` 引用了 \`ADR-${id}\`，但 \`docs/010-adr/${id}.md\` 打不开 —— 指向不存在的决策的豁免不算豁免`)
        }
      }
    }
  }
}

const portFacts = (facts ?? []).filter((f) => f.portField !== null)
const portFields = new Set(portFacts.map((f) => f.portField))
if (portFields.size > 1) {
  report(
    'PTS-001',
    `带端口的事实声明了 ${portFields.size} 种端口字段名（${[...portFields].join(' / ')}）—— 端口字段名必须唯一，否则读侧无从知道该读哪个键`,
  )
}
for (const f of portFacts) {
  // 常量名回事件名字表查一次字面值——字面量的 owner 在那边，不在本脚本。
  const name = f.name ?? consts?.get(f.token)
  if (name === undefined || name === null) {
    report('PTS-001', `事实 \`${f.token}\` 的名字是常量形式却查不到字面值 —— 声明层引用了事件名字表里没有的名字`)
    continue
  }
  if (protocolNames.has(name)) continue
  if (allowedProtocols.has(name)) continue
  report(
    'PTS-001',
    `事实 \`${name}\` 带端口字段，却不在协议族（${[...protocolNames].join(' / ')}）里 —— 端口是载荷不是事实种类：加一个端口不得新增事实名（PL-016 §6 判据 1）`,
  )
}

// ───────────────────────── 求解器：指纹与端口字段 ─────────────────────────

/**
 * 端口**取值**名单：谁声明端口，谁的 `PortDecl` 就是名单来源——扫 `symbio/src` 下
 * 含 `PortDecl {` 的 `.rs`，不钉某一张表。钉住文件会把插件路径写进这条 core 侧
 * 判据，而「再加一张端口表」就判不到了。
 */
function portValueLiterals() {
  const out = new Set()
  const skip = new Set(['target', 'node_modules', '.git', '.symbio', '.workbuddy-ai'])
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        if (!skip.has(entry.name)) walk(path.join(dir, entry.name))
        continue
      }
      if (!entry.name.endsWith('.rs')) continue
      const text = fs
        .readFileSync(path.join(dir, entry.name), 'utf8')
        .replace(/\r\n/g, '\n')
      if (!text.includes('PortDecl {')) continue
      for (const m of blankComments(text).join('\n').matchAll(/\bport\s*:\s*"([^"]+)"/g)) {
        out.add(m[1])
      }
    }
  }
  walk(path.join(ROOT, 'symbio', 'src'))
  return out
}

const portValues = portValueLiterals()

const baseline = readBaseline()
const solverRel = baseline?.solver?.file ?? 'symbio/src/symbio_core/decl/ring.rs'
const solverText = readText(solverRel)

/**
 * 取求解器两个函数的源码（**剥掉注释**）。
 *
 * 为什么剥注释：判据 1 说的是「这两段代码的字节不变」，指的是**算法**；
 * 散文改动不该撞红（否则每写一句注释都要更新基线，基线就成了噪音）。
 * 同时也避免「文档里写了 `fn drive`」被当成函数定义。
 */
function solverSource(text) {
  if (text === null) return null
  const code = blankComments(text).join('\n')
  const parts = []
  for (const fn of SOLVER_FNS) {
    const m = new RegExp(`\\bfn\\s+${fn}\\b`).exec(code)
    if (!m) return { missing: fn }
    const open = code.indexOf('{', m.index)
    if (open < 0) return { missing: fn }
    parts.push(code.slice(m.index, matchBrace(code, open) + 1))
  }
  return { source: parts.join('\n') }
}

const solver = solverSource(solverText)
const solverHash =
  solver && solver.source
    ? 'sha256:' + crypto.createHash('sha256').update(solver.source, 'utf8').digest('hex')
    : null

if (PRINT_BASELINE) {
  console.log(
    JSON.stringify(
      {
        file: solverRel,
        fns: SOLVER_FNS,
        hash: solverHash,
        adr: baseline?.solver?.adr ?? null,
        reason: baseline?.solver?.reason ?? null,
      },
      null,
      2,
    ),
  )
  process.exit(solverHash ? 0 : 1)
}

// PTS-002：引擎不读端口字段 ⇒ 引擎对端口无感
if (solver && solver.missing) {
  report('PTS-002', `求解器文件 ${solverRel} 里找不到 \`fn ${solver.missing}\` —— 求解器搬走了或改了名，端口面的判据没有输入了`)
} else if (solverText !== null && portFacts.length > 0) {
  const field = [...portFields][0]
  const literals = new Set(stringLiteralsOf(solverText))
  if (literals.has(field)) {
    report(
      'PTS-002',
      `求解器（${solverRel}）里出现了端口字段名 \`${field}\` —— 引擎一旦读端口，端口数就会变成引擎的分支数（加端口 = 改引擎），判据 1 立刻破`,
    )
  }
  // **取值**一侧（T-245 口径 ②）：`if port == "tool"` 这种分支不命名 `port`
  // 字段也照样是「引擎认识端口」，而且它比字段名更隐蔽——字段名会出现在
  // 任何一次读法里，取值只出现在特判里。判据面因此要同时问名单。
  // 面收窄到**两个函数体**（剥注释后的那段，与 PTS-003 同一段文本）：整份
  // `ring.rs` 里有信封字段名 `"turn"`（轮号，`fact_field` 的特判），它与
  // 端口 `turn` 的取值同名——那是信封与端口的命名巧合，不是引擎读端口。
  if (solver && solver.source) {
    const engineLiterals = new Set(stringLiteralsOf(solver.source))
    for (const value of [...portValues].sort()) {
      if (engineLiterals.has(value)) {
        report(
          'PTS-002',
          `求解器（${solverRel} 的 ${SOLVER_FNS.join(' / ')}）里出现了端口取值 \`${value}\` —— 按端口值分支就是「加端口 = 改引擎」的另一种写法，判据 1 立刻破`,
        )
      }
    }
  }
}

// PTS-003：求解器字节与基线一致
if (!baseline) {
  report('PTS-003', `基线 ${BASELINE_REL} 读不出 —— 判据 1 的「字节不变」没有比对基准，判据没有输入，宁红不静默`)
} else if (solver && solver.missing) {
  report('PTS-003', `基线指向的求解器 ${solverRel} 里没有 \`fn ${solver.missing}\` —— 基线与代码已经对不上`)
} else if (!solverHash) {
  report('PTS-003', `求解器 ${solverRel} 的源码取不出来 —— 判据没有输入，宁红不静默`)
} else if (solverHash !== baseline.solver?.hash) {
  report(
    'PTS-003',
    `求解器（${solverRel} 的 ${SOLVER_FNS.join(' / ')}）的字节与基线不符：实测 ${solverHash} ≠ 基线 ${baseline.solver?.hash ?? '(无)'}。` +
      `判据 1 说「加端口不改引擎」——若这次改动**与端口无关**（机制演进），同批把基线更新成当前指纹并写清 ADR 与理由；` +
      `若与端口有关，那就是端口渗进了引擎，改代码而不是改基线`,
  )
}

// PTS-004：基线挂得住一条决策
if (baseline) {
  const s = baseline.solver ?? {}
  const reason = typeof s.reason === 'string' ? s.reason.trim() : ''
  if (reason === '') {
    report('PTS-004', `基线的 \`reason\` 为空 —— 基线是判据的一面，改它必须写清「为什么这次引擎可以动」`)
  }
  const refs = [...String(s.adr ?? '').matchAll(/ADR-(\d+[a-z]?)/g)].map((r) => r[1])
  if (refs.length === 0) {
    report('PTS-004', `基线的 \`adr\` 里没有 ADR 编号 —— 改求解器是一次架构决策，必须挂一条能打开的决策`)
  }
  for (const id of refs) {
    if (!fs.existsSync(path.join(ROOT, 'docs', '010-adr', `${id}.md`))) {
      report('PTS-004', `基线引用了 \`ADR-${id}\`，但 \`docs/010-adr/${id}.md\` 打不开 —— 指向不存在的决策的豁免不算豁免`)
    }
  }
}

// ───────────────────────── 输出 ─────────────────────────

const ruleLine = (ok, rule, desc) =>
  console.log(`${ok ? green('✓') : red('✗')} ${rule}  ${dim(desc)}`)

if (errors.length === 0) {
  console.log('\n== port-surface-audit ==')
  ruleLine(
    true,
    'PTS-001',
    `协议名闭集：带端口的事实 ${portFacts.length} 条，全部落在协议族（${[...protocolNames].join(' / ')}）内`,
  )
  ruleLine(
    true,
    'PTS-002',
    portFacts.length > 0
      ? `求解器对端口无感：${solverRel} 里没有端口字段名 \`${[...portFields][0]}\`，${SOLVER_FNS.join(' / ')} 里也没有端口取值（名单 ${[...portValues].sort().join(' / ') || '空'}）`
      : '求解器对端口无感：当前没有带端口的事实声明（判据待首个端口落地时生效）',
  )
  ruleLine(true, 'PTS-003', `求解器字节与基线一致：${solverHash}`)
  ruleLine(true, 'PTS-004', `基线挂得住决策：${baseline?.solver?.adr ?? '(无)'}`)
  process.exit(0)
}

console.error(`\n== port-surface-audit：${errors.length} 处违规 ==`)
for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
console.error(
  `\n${yellow('提示')}：判据口径见本脚本文件头与 docs/004-plan/016-义务环.md §6 判据 1；` +
    `改引擎后更新基线：node scripts/port-surface-audit.mjs --print-baseline`,
)
process.exit(1)
