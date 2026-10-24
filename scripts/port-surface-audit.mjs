#!/usr/bin/env node
/**
 * port-surface-audit — **端口面**：DE-015 §6 判据 1 的可执行形态
 *
 * ## 它判什么
 *
 * | 编号 | 判据 | 它拦的是哪一种走样 |
 * |---|---|---|
 * | PTS-001 | 带端口载荷的**事实名** ⊆ 协议族（`request` / `sent` / `response` / `failed` / `retry`），且端口字段名唯一 | **加端口顺手加事实名**——端口是载荷，不是事实种类（DE-015 §2.4 裁决） |
 * | PTS-002 | 求解器（`owed` / `drive` 所在文件）里**不出现端口字段名与端口取值** | **端口渗进引擎**——引擎一旦认识端口，加端口就得改引擎 |
 * | PTS-003 | 求解器那两个函数的**字节**与基线一致 | 判据 1 的原文口径：「`drive` 与 `owed` 源码 diff = 0」 |
 * | PTS-004 | 基线引用的 ADR **能打开**、理由**非空** | **改判据而不留决策**——基线是判据的一面，改它必须有决策挂得住 |
 * | PTS-005 | 协议格的**写点只在一个文件里**（扫 `symbio/src` 的 `Event::pending`，名字位上是协议常量或 `PortDecl::before`） | **协议格到处有人写**——「四格由引擎独占」从此靠自觉（DE-015 §2.3 / 判据 5） |
 * | PTS-006 | DE-015 §2.6 契约表的第 7 行**引 D-138**（重试已启用：键含 attempt，重试前提两条）且**不含「未启用」** | **条文与机制分叉**——重试已进债键（D-138），条文若仍说「未启用」就是把在跑的路说成没跑 |
 *
 * ## 为什么是这几格（而不是「机制键守恒」那种计数）
 *
 * DE-015 §6 判据 1 是本方案最硬的一条：**加一个端口 = 0 个新事实名 + 1 个
 * `PortDecl` + 适配器 + 2 行模板，`drive` 与 `owed` 的源码 diff = 0**。
 * 它不是「键数不变」，是**那两段代码的字节不变**——因为端口一旦渗进引擎，
 * 「加端口」就变成「改引擎」，机制面随端口数线性增长，而端口数没有上界。
 *
 * 判据 1 此前**只有散文**（DE-015 §6 那一行），没有执行方。于是三件事都能悄悄发生：
 * 新端口带出新事实名（协议族闭集被稀释）、引擎里长出 `if port == "…"`（端口渗进）、
 * 引擎为某个端口加了特判（字节变了而没人说得清是谁的锅）。
 *
 * ## 各格的关系（一句话说清谁管什么）
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
 * - **PTS-005 管「谁在写协议格」**：前四格判的是声明与引擎，这一格判**写侧**——
 *   `sent` / `response` / `failed` / `retry` 由引擎独占（DE-015 §2.3），装载期
 *   `ENGINE_ONLY_CELLS` 拦的是「执行体**声明**了它们」，而「另一个文件直接落一条
 *   `sent`」声明面上查不到。这条判据不钉文件名：它钉**写方唯一**（名单长度 == 1），
 *   因为「唯一」才是判据，「是 `port.rs`」只是今天的实测值。写点为 0 同样判红——
 *   协议格没人写就是环不转，而不是「没什么可判」。
 *   边界（写在绿灯里，别把它当证明）：判定基于 `Event::pending(` 的**名字实参**文本。
 *   本仓 `Event` 的**字段是 `pub`**，所以手搓结构体字面量在类型上合法、本条看不见
 *   （今天全仓零处；要关掉这个缝得先收窄 `Event` 字段，那是 core 形状的事，
 *   挂 `core-shape-audit` 而不挂本条）。把名字先存进一个认不出来的变量同样能绕，
 *   这一处按 `decl.before` 补判（`sent` 的名字正是它，全仓只在端口层读一次）。
 * - **PTS-006 管「条文与机制对不对得上」**：前五格判声明、引擎与写侧，这一格判**散文**——
 *   重试已进债键（D-138），契约 7 若继续写「未启用」，读者会以为重试不在生产里跑。
 *   机制侧由用例 `only_the_tool_port_has_headroom_and_only_for_rebuildable_waiters`
 *   （真表取值）与 `a_retried_request_reopens_authorization_and_dispatch`（行层重新开账）
 *   钉住，条文侧由本条钉「引了 D-138、不再写未启用」，两侧各钉一头；任何一侧单独
 *   放宽都足以让「重试通道未启用」的说法复活。边界：它只认 §2.6 契约表第 7 行的文字，
 *   判不出别的段落里的同类说法。
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
 * - PTS-002 判**字段名 + 取值**两侧（口径 ②，把原来自陈的边界关掉）：
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

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns PTS 端口面（协议名闭集 / 引擎对端口无感 / 求解器字节基线 / 协议格写方唯一 / 契约 7 的重试限定）
// @codes PTS-001 PTS-002 PTS-003 PTS-004 PTS-005 PTS-006

import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'
import { blankComments, blankTestModules, matchBrace, skipString } from './rust-scan.mjs'
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
/** 契约 7 的条文所在（PTS-006 的唯一输入：§2.6 契约表的第 7 行） */
const PLAN_REL = 'docs/003-design/015-义务环.md'

/**
 * 协议族：**哪五个常量**是协议族（DE-015 §2.4 的「五个协议名」）。
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
 * ⚠️ 这是**判据的一部分**，不是配置：改它等于改判据（DE-015 §6 判据 1 的原话是
 * 「`drive` 与 `owed`」）。真要改，走 ADR + 基线更新（PTS-003 / PTS-004）。
 */
const SOLVER_FNS = ['owed', 'drive']

const errors = []
const report = (rule, msg) => errors.push({ rule, msg })

// 决策的存在形态是设计文档里的 `### D-NNN` 条目——adr 目录已废除，决策折入设计正文。
// 全量扫一遍设计文档与组件级就地文档后缓存：一次读、多次判。
let decisionSet = null
function decisionExists(id) {
  if (decisionSet === null) {
    decisionSet = new Set()
    const roots = [path.join(ROOT, 'docs', '003-design'), path.join(ROOT, 'symbio', 'src'), path.join(ROOT, 'cli', 'docs'), path.join(ROOT, 'tauri', 'docs')]
    const walk = (d) => {
      let list
      try {
        list = fs.readdirSync(d, { withFileTypes: true })
      } catch {
        return
      }
      for (const e of list) {
        const abs = path.join(d, e.name)
        if (e.isDirectory()) {
          if (e.name === 'archive' || e.name === 'target' || e.name === 'node_modules') continue
          walk(abs)
          continue
        }
        if (!e.name.endsWith('.md')) continue
        for (const line of fs.readFileSync(abs, 'utf8').split(String.fromCharCode(10))) {
          const m = /^#{2,6}\s+(D-\d{3}a?)\b/.exec(line)
          if (m) decisionSet.add(m[1])
        }
      }
    }
    for (const r of roots) walk(r)
  }
  return decisionSet.has(`D-${id}`)
}


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
    report('PTS-001', `协议名常量 \`${c}\` 不在 ${EVENT_REL} 里 —— 协议族的名字必须单点定义（DE-015 §2.4 的五个协议名）`)
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
      const refs = [...m[2].matchAll(/(?:ADR|D)-(\d+[a-z]?)/g)].map((r) => r[1])
      if (refs.length === 0) {
        report('PTS-001', `豁免 \`protocol ${m[1]}\` 的理由里没有 ADR 编号 —— 协议族是闭集，加一个成员是机制变更，必须挂一条决策`)
        continue
      }
      for (const id of refs) {
        if (!decisionExists(id)) {
          report('PTS-001', `豁免 \`protocol ${m[1]}\` 引用了决策 \`D-${id}\`，但设计文档里没有 \`### D-${id}\` 条目 —— 指向不存在的决策的豁免不算豁免`)
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
    `事实 \`${name}\` 带端口字段，却不在协议族（${[...protocolNames].join(' / ')}）里 —— 端口是载荷不是事实种类：加一个端口不得新增事实名（DE-015 §6 判据 1）`,
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
  // **取值**一侧（口径 ②）：`if port == "tool"` 这种分支不命名 `port`
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
  const refs = [...String(s.adr ?? '').matchAll(/(?:ADR|D)-(\d+[a-z]?)/g)].map((r) => r[1])
  if (refs.length === 0) {
    report('PTS-004', `基线的 \`adr\` 里没有 ADR 编号 —— 改求解器是一次架构决策，必须挂一条能打开的决策`)
  }
  for (const id of refs) {
    if (!decisionExists(id)) {
      report('PTS-004', `基线引用了决策 \`D-${id}\`，但设计文档里没有 \`### D-${id}\` 条目 —— 指向不存在的决策的豁免不算豁免`)
    }
  }
}

// ───────────────────────── PTS-005 协议格写方唯一 ─────────────────────────

/**
 * 协议四格里**由引擎写**的那四格（判据 5 的名单）。
 *
 * 钉常量名不钉字面量：字面量的 owner 在 `event/mod.rs`（与 PTS-001 同一口径）。
 *
 * `request` **不在名单里**，这是口径不是遗漏：协议五格里只有这四格是引擎独占
 * （`decl/mod.rs` 的 `ENGINE_ONLY_CELLS`），`request` 由**决策者**写（DE-015 §2.4），
 * 把它算进「引擎的账」就把契约写反了。代价说清楚：今天它的写点也在 `port.rs`，
 * 但第二个 `request` 写点本条看不见——那条缝归装载期与运行期对账（判据 2）。
 */
const WRITE_CONSTS = ['EVENT_SENT', 'EVENT_RESPONSE', 'EVENT_FAILED', 'EVENT_RETRY']

/**
 * 取调用表达式的一个实参文本：`openIdx` 指向左括号，`n` 从 0 起。
 *
 * 为什么按实参而不是按行：`Event::pending(` 的名字实参与它隔着换行是常态，
 * 按行匹配会把行表里的 `source: EVENT_SENT`（那是**模式**，不是写）读成写点，
 * 也会漏掉名字与构造点不同行的真写点。嵌套括号 / 方括号里的逗号不算分隔，
 * 字符串里的都不算（`skipString` 认原始串）。
 */
function argumentAt(code, openIdx, n) {
  let depth = 0
  let cur = ''
  const args = []
  for (let i = openIdx; i < code.length; i++) {
    const ch = code[i]
    if (ch === '"') {
      const end = skipString(code, i)
      cur += code.slice(i, end + 1)
      i = end
      continue
    }
    if (ch === '(' || ch === '[' || ch === '{') {
      depth++
      if (depth > 1) cur += ch
      continue
    }
    if (ch === ')' || ch === ']' || ch === '}') {
      depth--
      if (depth === 0) {
        args.push(cur)
        return args[n]
      }
      cur += ch
      continue
    }
    if (ch === ',' && depth === 1) {
      args.push(cur)
      cur = ''
      continue
    }
    if (depth >= 1) cur += ch
  }
  return undefined
}

/**
 * `symbio/src` 下的协议格写点，按文件聚合（值是命中行号）。
 *
 * 排除测试：`.test.rs` 文件与 `#[cfg(test)]` 模块里造一条 `sent` 是**布置事实**，
 * 不是建立第二个写方（与 PTS-002 剥注释同一取向——散文与排演都不算算法）。
 * `blankComments` + `blankTestModules` 都保留换行，所以报告里的行号对得上原文件。
 */
function protocolWritePoints() {
  const hits = new Map()
  const skip = new Set(['target', 'node_modules', '.git', '.symbio', '.workbuddy-ai'])
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        if (!skip.has(entry.name)) walk(path.join(dir, entry.name))
        continue
      }
      if (!entry.name.endsWith('.rs') || entry.name.endsWith('.test.rs')) continue
      const abs = path.join(dir, entry.name)
      const code = blankTestModules(
        blankComments(fs.readFileSync(abs, 'utf8').replace(/\r\n/g, '\n')).join('\n'),
      )
      const lines = []
      for (const m of code.matchAll(/\bEvent\s*::\s*pending\s*\(/g)) {
        const open = code.indexOf('(', m.index)
        const name = argumentAt(code, open, 1)
        if (name === undefined) continue
        const hit =
          WRITE_CONSTS.some((c) => new RegExp(`\\b${c}\\b`).test(name)) ||
          /\bbefore\b/.test(name)
        if (!hit) continue
        const line = code.slice(0, m.index).split('\n').length
        lines.push(line)
      }
      if (lines.length > 0) {
        hits.set(path.relative(ROOT, abs).split(path.sep).join('/'), lines)
      }
    }
  }
  walk(path.join(ROOT, 'symbio', 'src'))
  return hits
}

const writePoints = protocolWritePoints()
const writers = [...writePoints.keys()].sort()
if (writers.length === 0) {
  report(
    'PTS-005',
    `全仓找不到协议格的写点（扫 \`Event::pending\` 的名字实参 ⊆ ${WRITE_CONSTS.join(' / ')} / before）——` +
      `协议格没人写就是环不转，不是「没什么可判」；判据没有输入，宁红不静默`,
  )
} else if (writers.length > 1) {
  report(
    'PTS-005',
    `协议格有 ${writers.length} 个写点文件（${writers.join(' / ')}）—— 协议格由引擎独占（DE-015 §2.3）：` +
      `第二处写点会让「四格 = 引擎的账」变成自觉，端口层的幂等键与租约也不再是单点逻辑`,
  )
}

// ───────────────────────── PTS-006 契约 7 的重试限定 ─────────────────────────

/**
 * DE-015 §2.6 契约表第 7 行：重试已启用（D-138）。
 *
 * 判据只认契约表第 7 行那一段文字。重试进债键的裁决落纸后，条文必须**引 D-138**
 * （键含 attempt、重试前提两条），且不得再写「未启用」——真表上 `tool` 端口
 * `max_attempts = 2`、行层对 attempt+1 重新开账，条文说「未启用」就是把在跑的路
 * 说成没跑。缺重试裁决的锚 = 改了机制而不留决策（本仓纪律）；留着「未启用」= 条文
 * 与机制分叉（「条文与真表不一致」的旧病反向复发）。
 *
 * 找不到 §2.6 / 第 7 行同样判红——锚点漂了不是「没什么可判」。
 */
const CONTRACT_RETRY_ROW_RE = /^\|\s*7\s*\|/

function contractRetryRow() {
  const text = readText(PLAN_REL)
  if (text === null) {
    report('PTS-006', `读不出 ${PLAN_REL} —— 契约 7 的条文是这条判据的唯一输入，宁红不静默`)
    return null
  }
  const start = text.indexOf('### 2.6 ')
  if (start < 0) {
    report('PTS-006', `${PLAN_REL} 里找不到「### 2.6 」小节 —— 契约表的锚点没了，判据没有输入`)
    return null
  }
  const end = text.indexOf('\n### ', start + 1)
  const section = text.slice(start, end < 0 ? text.length : end)
  const rows = section.split('\n').filter((l) => CONTRACT_RETRY_ROW_RE.test(l))
  if (rows.length !== 1) {
    report('PTS-006', `§2.6 契约表的第 7 行找到 ${rows.length} 条（应恰好 1 条）—— 锚点漂了，判据没有输入`)
    return null
  }
  return rows[0]
}

const contractRow = contractRetryRow()
// 引用决策编号用正则字面量而不是字符串字面量：`'D-138'` 整串等值会被
// gate-codes-audit 判成「发射了未登记的判据码」（D 前缀下决策编号与判据码共用
// 一个字母，只认整串等值）；这里是对文档散文的**引用**，不是发射码。
if (contractRow !== null && !/D-138/.test(contractRow)) {
  report(
    'PTS-006',
    `§2.6 契约 7 那一行不引重试进债键的裁决 —— 重试的键口径是裁决过的机制改动（键含 attempt、前提两条）；改判据不留决策 = 本仓纪律 08 §0 纪律 3 要拦的形状`,
  )
} else if (contractRow !== null && contractRow.includes('未启用')) {
  report(
    'PTS-006',
    `§2.6 契约 7 那一行仍写「未启用」—— 重试已启用（\`tool\` 端口 \`max_attempts = 2\`，行层对 attempt+1 重新开账，见重试进债键裁决）；条文与机制分叉`,
  )
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
  ruleLine(
    true,
    'PTS-005',
    `协议格写方唯一：${writers.join(' / ')}（命中 ${[...writePoints.values()].reduce((n, l) => n + l.length, 0)} 处构造）`,
  )
  ruleLine(true, 'PTS-006', `契约 7 引 D-138（重试已启用）：${PLAN_REL} §2.6`)
  process.exit(0)
}

console.error(`\n== port-surface-audit：${errors.length} 处违规 ==`)
for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
console.error(
  `\n${yellow('提示')}：判据口径见本脚本文件头与 docs/003-design/015-义务环.md §6 判据 1；` +
    `改引擎后更新基线：node scripts/port-surface-audit.mjs --print-baseline`,
)
process.exit(1)
