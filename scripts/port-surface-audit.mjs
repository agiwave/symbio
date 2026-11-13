#!/usr/bin/env node
/**
 * port-surface-audit — **端口面**：DE-051 / DE-050 的端口判据可执行形态
 *
 * ## 它判什么
 *
 * | 编号 | 判据 | 它拦的是哪一种走样 |
 * |---|---|---|
 * | PTS-001 | 带端口载荷的**事实名** ⊆ 协议族（`request` / `sent` / `response` / `failed` / `retry`），且端口字段名唯一 | **加端口顺手加事实名**——端口是载荷，不是事实种类（DE-050 §0） |
 * | PTS-002 | 求解器（`owed` 所在文件）里**不出现端口字段名与端口取值** | **端口渗进引擎**——引擎一旦认识端口，加端口就得改引擎 |
 * | PTS-003 | 求解函数 `owed` 的**字节**与基线一致 | 判据 1 的不变量：「加端口不改引擎」 |
 * | PTS-004 | 基线引用的裁决锚点 **能打开**、理由**非空** | **基线失去决策锚点**——重算基线要挂得住既有裁决；改判据 / 协议本身才需新裁决 |
 * | PTS-005 | 协议格的**写点只在一个文件里**（两种形态：`symbio/src` 的 `Event::pending` 名字实参 ⊆ 协议常量 / `PortDecl::before`；`impl PortFactWriter` 里的 `store.append`——即 effects / `PortFactWriter` 链路） | **协议格到处有人写**——「四格由引擎独占」从此靠自觉（DE-050 §0） |
 * | PTS-006 | DE-050 §3.3 的**重试条文**写明 `retry(attempt+1)` 且**不含「未启用」** | **条文与机制分叉**——重试已进债键，条文若仍说「未启用」就是把在跑的路说成没跑 |
 * | PTS-007 | **架构层与骨架**的读侧（`kernel` / `sdk` / `plugin-api` / `loop` / `session`）**不得比较端口名**（`port == "model"` / `port == PORT_TOOL`）；能力插件比对自己端口的名字属**所有权**，不判（边界见文件末「已知边界」） | **读侧的端口特判回流**——收束载荷已经盖上章，读侧再认端口名就等于把「加端口不改读侧」作废 |
 *
 * ## 为什么是这几格（而不是「机制键守恒」那种计数）
 *
 * DE-051 的端口面判据是本方案最硬的一条：**加一个端口 = 0 个新事实名 + 1 个
 * `PortDecl` + 债行 / 行声明 + 执行体，求解函数 `owed` 的源码 diff = 0**。
 * 它不是「键数不变」，是**那段代码的字节不变**——因为端口一旦渗进引擎，
 * 「加端口」就变成「改引擎」，机制面随端口数线性增长，而端口数没有上界。
 *
 * 判据 1 此前**只有散文**（DE-050 / DE-051 的条文），没有执行方。于是三件事都能悄悄发生：
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
 *   而是「引擎动过了 ⇒ 同批重算基线、在 `reason` 写清为什么」（PTS-004 判这条
 *   基线的决策锚点与理由挂不挂得住）。**重算基线是记账，不是改判据**：`adr`
 *   沿用既有裁决即可，不必为它另造 `D-NNN`；只有改协议 / 判据本身（影响面
 *   巨大）才需要先落一条新裁决。于是「改引擎」不是顺手的事，但也不是每次都要立法。
 * - **PTS-004 管「基线本身」**：基线是一处**判据面**，`adr` 必须指向能打开的
 *   既有裁决（按本仓纪律 `SP-007`），`reason` 必须解释本次为何重算；它判的是
 *   **锚点还在不在**，不是「这次改动配不配拥有一条新决策」。「能打开」的读法
 *   不在本脚本自定，统一走 `scripts/decision-anchors.mjs` 的 `hasDecisionAnchor`：
 *   设计文档（含模块 `README.md`）的「关键决策」节里的 `### D-NNN` 锚点。
 * - **PTS-005 管「谁在写协议格」**：前四格判的是声明与引擎，这一格判**写侧**——
 *   `sent` / `response` / `failed` / `retry` 由引擎独占（DE-050 §0），装载期
 *   `ENGINE_ONLY_CELLS` 拦的是「执行体**声明**了它们」，而「另一个文件直接落一条
 *   `sent`」声明面上查不到。这条判据不钉文件名：它钉**写方唯一**（名单长度 == 1），
 *   因为「唯一」才是判据，「是 `port.rs`」只是今天的实测值。写点为 0 同样判红——
 *   协议格没人写就是环不转，而不是「没什么可判」。
 *   判定面认两种写点形态，因为链路现在分两段：
 *   - **落库写点**：`impl PortFactWriter` 体里的 `store.append`（宿主把执行体给的
 *     `FactDraft` 落成事实——这是协议格真正进账的唯一一处）；
 *   - **直写**：`Event::pending(` 的名字实参是协议常量或 `PortDecl::before`（老形状，
 *     也是「有人绕过出口直接构造协议事实」的形状，仍要看见）。
 *   环里的 `CollectingWriter` 只**收集**效果、不落库，不算写点——它是链路的中间段，
 *   不是第二个写方。
 *   边界（写在绿灯里，别把它当证明）：判定基于文本。本仓 `Event` 的**字段是 `pub`**，
 *   所以手搓结构体字面量在类型上合法、本条看不见（今天全仓零处；要关掉这个缝得先
 *   收窄 `Event` 字段，那是 core 形状的事，挂 `arch-shape-audit` 而不挂本条）。
 *   把名字先存进一个认不出来的变量同样能绕，这一处按 `decl.before` 补判
 *   （`sent` 的名字正是它，全仓只在端口层读一次）。
 * - **PTS-006 管「条文与机制对不对得上」**：前五格判声明、引擎与写侧，这一格判**散文**——
 *   重试已进债键，`DE-050` §3.3 若继续写「未启用」，读者会以为重试不在生产里跑。
 *   机制侧由用例 `only_the_tool_port_has_headroom_and_only_for_rebuildable_waiters`
 *   （真表取值）与 `a_retried_request_reopens_authorization_and_dispatch`（行层重新开账）
 *   钉住，条文侧由本条钉「写了 `retry(attempt+1)`、不再写未启用」，两侧各钉一头。
 *   边界：它只认 §3.3 那一段的文字，判不出别的段落里的同类说法。
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
 * - 判定基于**文本**而非 AST：求解器的字节取「注释剥掉后的 `owed` 函数体」，
 *   所以**只改注释不算改动**（散文不是算法），但改函数签名 / 体里的任何一行动红。
 * - PTS-001 只判**声明层**（`decl/mod.rs` 的 `FactDecl`）。事实真的落不落这个字段、
 *   载荷里有没有真的带上端口，是**装载期 / 运行期**的事（`seal` 的字段检查与
 *   写侧闸），本脚本不重复判。
 * - PTS-002 判**字段名 + 取值**两侧（口径 ②，把原来自陈的边界关掉）：
 *   字段名在整份求解器文件里查（剥注释前都会红），取值在 `owed`
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
// @ns PTS 端口面（协议名闭集 / 引擎对端口无感 / 求解器字节基线 / 协议格写方唯一 / 重试条文 / 读侧不认端口名）
// @codes PTS-001 PTS-002 PTS-003 PTS-004 PTS-005 PTS-006 PTS-007

import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'
import { blankComments, blankTestModules, matchBrace, skipString } from './rust-scan.mjs'
import { stringLiteralsOf } from './gate.d/_shared.mjs'
import { hasDecisionAnchor } from './decision-anchors.mjs'

const argv = process.argv.slice(2)
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const PRINT_BASELINE = argv.includes('--print-baseline')

/** 事件名字表（协议名的唯一 owner）与事实声明表 */
const EVENT_REL = 'crates/symbio-kernel/src/event/mod.rs'
const DECL_REL = 'crates/symbio-kernel/src/decl/mod.rs'
/** 基线（判据面之一，见文件头「基线在哪」） */
const BASELINE_REL = 'scripts/baselines/port-surface.json'
/** 重试条文所在（PTS-006 的唯一输入：§3.3 重放收敛） */
const PLAN_REL = 'docs/003-design/050-义务环事实词表与债务表.md'

/**
 * 协议族：**哪五个常量**是协议族（DE-050 §0 的「五个协议名」）。
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
 * 求解函数：判据 1 冻结的那段代码。选债归环之后内核只剩这一个求解口。
 *
 * ⚠️ 这是**判据的一部分**，不是配置：名单收窄（求解器归一）随代码同批做，
 * 加回第二个求解口等于把选债搬回内核，属**变宽**，要先落裁决。名单变化后
 * PTS-003 的指纹同批更新（基线 `scripts/baselines/port-surface.json`，
 * 理由写进 `reason`；既有 `adr` 沿用，不为改基线另造决策号）。
 */
const SOLVER_FNS = ['owed']

const errors = []
const report = (rule, msg) => errors.push({ rule, msg })

// 「这条 D-NNN 到底存不存在」不在本脚本再抄一遍扫描根与锚点正则：唯一读法是
// `scripts/decision-anchors.mjs` 的 `hasDecisionAnchor`——它同时认设计文档里的
// `### D-NNN` 锚点（设计文档或模块 README 的「关键决策」节）。
// `ROOT` 随 `--root=` 走，回归夹具因此在自己的临时树里判，不会串到真仓。
const decisionExists = (id) => hasDecisionAnchor(id, ROOT)


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
    report('PTS-001', `协议名常量 \`${c}\` 不在 ${EVENT_REL} 里 —— 协议族的名字必须单点定义（DE-050 §0 的五个协议名）`)
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
          report('PTS-001', `豁免 \`protocol ${m[1]}\` 引用了决策 \`D-${id}\`，但设计文档里没有 \`D-${id}\` 这条裁决 —— 指向不存在的决策的豁免不算豁免`)
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
    `事实 \`${name}\` 带端口字段，却不在协议族（${[...protocolNames].join(' / ')}）里 —— 端口是载荷不是事实种类：加一个端口不得新增事实名（判据 1）`,
  )
}

// ───────────────────────── 求解器：指纹与端口字段 ─────────────────────────

/**
 * 端口**取值**名单：谁声明端口，谁的声明就是名单来源。判定面认两种声明形状：
 * - `PortDecl { port: … }`（静态端口表，tool / telegram 这类）；
 * - `impl PortProvider for … { fn ports(&self) -> Vec<String> { vec![…] } }`
 *   （提供者自报端口，model 这类：端口名随能力目录注册，没有静态端口表）。
 * 只认第一种会让「没有 PortDecl 的端口」从名单里**静默消失**——名单一窄，
 * PTS-002 就不再拦「引擎按这个端口的值分支」。扫 `crates` 下所有 `.rs`，不钉某一张表：
 * 钉住文件会把插件路径写进这条 core 侧判据，而「再加一张端口表」就判不到了。
 *
 * 端口词可以走常量（`port: ports::MODEL`，端口词单点拼写后生产表就是这么写的）：
 * 常量链解析到字面值为止——`CapabilityFamily::(multi|single)("词")` 给出族词，
 * `pub const X: &str = <路径>::Y.name()` 把族词借成端口词，`pub const X: &str = "词"`
 * 是裸常量（`fn ports` 的数组元素就是这种）。跨 crate 的共享名走
 * `use <路径>::NAME as ALIAS`（D-364 的词汇层），别名也沿链解析到 `NAME` 的声明。
 * 带 `::` 的路径解析不出时**报红**而不是跳过，裸标识同理：跳过会让取值一侧
 * 静默变窄（宁红不静默）。
 */
function portValueLiterals() {
  const out = new Set()
  const unresolved = []
  const family = new Map()
  const plainConsts = new Map()
  const aliases = new Map()
  const strRefs = []
  const portExprs = []
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
      const code = blankComments(text).join('\n')
      for (const m of code.matchAll(
        /pub(?:\([^)]*\))?\s+const\s+([A-Z0-9_]+)\s*:\s*&str\s*=\s*"([^"]*)"/g,
      )) {
        plainConsts.set(m[1], m[2])
      }
      for (const m of code.matchAll(
        /pub\s+const\s+([A-Z0-9_]+)\s*:\s*CapabilityFamily\s*=\s*CapabilityFamily::(?:multi|single)\s*\(\s*"([^"]+)"\s*\)/g,
      )) {
        family.set(m[1], m[2])
      }
      for (const m of code.matchAll(
        /pub\s+const\s+([A-Z0-9_]+)\s*:\s*&str\s*=\s*(?:[A-Za-z0-9_]+::)+([A-Z0-9_]+)\.name\s*\(\s*\)/g,
      )) {
        strRefs.push([m[1], m[2]])
      }
      // 跨 crate 的共享名：`use <路径>::NAME as ALIAS;` / `use <路径>::NAME;`。
      // 别名本身不携带字面量，沿链解析到 `NAME` 的常量声明（D-364）。
      for (const m of code.matchAll(
        /(?:pub(?:\([^)]*\))?\s+)?use\s+(?:[A-Za-z0-9_]+::)*([A-Z][A-Z0-9_]*)\s+as\s+([A-Z][A-Z0-9_]*)\s*;/g,
      )) {
        aliases.set(m[2], m[1])
      }
      for (const m of code.matchAll(
        /(?:pub(?:\([^)]*\))?\s+)?use\s+(?:[A-Za-z0-9_]+::)+([A-Z][A-Z0-9_]*)\s*;/g,
      )) {
        aliases.set(m[1], m[1])
      }
      if (text.includes('PortDecl {')) {
        // 只看 `PortDecl { … }` 体里的 `port:`：整份文件里的 `port:` 还会命中
        // `PortInput { port: None }` 这类别的结构，把 `None` / `Some` 当成端口词。
        for (const m of code.matchAll(/\bPortDecl\s*\{/g)) {
          const open = code.indexOf('{', m.index)
          if (open < 0) continue
          const body = code.slice(open + 1, matchBrace(code, open))
          const p = /\bport\s*:\s*("([^"]+)"|[A-Za-z0-9_][A-Za-z0-9_:]*)/.exec(body)
          if (p) portExprs.push(p[1])
        }
      }
      // 提供者自报端口：`fn ports(&self) -> Vec<String> { vec![词, …] }`（D-440：
      // 端口名是运行时数据，不再是编译期常量切片）。
      for (const m of code.matchAll(
        /\bfn\s+ports\s*\(\s*&?\s*self\s*\)\s*->\s*Vec\s*<\s*String\s*>\s*\{/g,
      )) {
        const open = code.indexOf('{', m.index)
        if (open < 0) continue
        const body = code.slice(open + 1, matchBrace(code, open))
        const arr = /vec!\s*\[([\s\S]*?)\]/.exec(body)
        if (!arr) continue
        for (const item of arr[1].split(',')) {
          const token = item.trim().replace(/\.to_string\(\)$/, '')
          if (token) portExprs.push(token)
        }
      }
    }
  }
  walk(path.join(ROOT, 'crates'))
  const strConst = new Map()
  for (const [name, ref] of strRefs) {
    if (family.has(ref)) strConst.set(name, family.get(ref))
  }
  /** 沿 `use … as` 别名解析到常量声明（最多 8 跳，防环）。 */
  const resolveName = (name) => {
    let cur = name
    for (let i = 0; i < 8; i++) {
      if (plainConsts.has(cur)) return plainConsts.get(cur)
      if (family.has(cur)) return family.get(cur)
      const next = aliases.get(cur)
      if (!next || next === cur) return undefined
      cur = next
    }
    return undefined
  }
  for (const expr of portExprs) {
    if (expr.startsWith('"')) {
      out.add(expr.slice(1, -1))
      continue
    }
    const last = expr.split('::').pop()
    const resolved = resolveName(last) ?? strConst.get(last)
    if (resolved !== undefined) out.add(resolved)
    else unresolved.push(expr)
  }
  return { values: out, unresolved, resolveName }
}

const { values: portValues, unresolved: portUnresolved, resolveName } = portValueLiterals()
for (const expr of portUnresolved) {
  report(
    'PTS-002',
    `端口表里的 \`${expr}\` 解析不出取值 —— 常量链断在半路，取值一侧会静默变窄；要么把常量定义写成可解析的形状（\`CapabilityFamily::…("词")\` / \`…::NAME.name()\`），要么回字面量`,
  )
}

const baseline = readBaseline()
const solverRel = baseline?.solver?.file ?? 'crates/symbio-kernel/src/decl/ring.rs'
const solverText = readText(solverRel)

/**
 * 取求解函数的源码（**剥掉注释**）。
 *
 * 为什么剥注释：判据 1 说的是「这段代码的字节不变」，指的是**算法**；
 * 散文改动不该撞红（否则每写一句注释都要更新基线，基线就成了噪音）。
 * 同时也避免「文档里写了 `fn owed`」被当成函数定义。
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
  // 面收窄到**求解函数体**（剥注释后的那段，与 PTS-003 同一段文本）：整份
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
      `判据 1 说「加端口不改引擎」——若这次改动**与端口无关**（机制演进），同批把基线更新成当前指纹、在 reason 写清理由（adr 沿用既有决策号，改基线不必另造决策）；` +
      `若与端口有关，那就是端口渗进了引擎，改代码而不是改基线`,
  )
}

// PTS-004：基线的决策锚点挂得住（判锚点在不在，不判这次改动有没有配新决策）
if (baseline) {
  const s = baseline.solver ?? {}
  const reason = typeof s.reason === 'string' ? s.reason.trim() : ''
  if (reason === '') {
    report('PTS-004', `基线的 \`reason\` 为空 —— 重算基线是记账，也要在 reason 写清「为什么这次引擎可以动」`)
  }
  const refs = [...String(s.adr ?? '').matchAll(/(?:ADR|D)-(\d+[a-z]?)/g)].map((r) => r[1])
  if (refs.length === 0) {
    report('PTS-004', `基线的 \`adr\` 里没有 ADR 编号 —— 沿用一条能打开的既有裁决即可；改基线不必另造决策，改协议 / 判据本身才需要`)
  }
  for (const id of refs) {
    if (!decisionExists(id)) {
      report('PTS-004', `基线引用了决策 \`D-${id}\`，但设计文档里没有 \`D-${id}\` 这条裁决 —— 指向不存在的决策的豁免不算豁免`)
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
 * （`decl/mod.rs` 的 `ENGINE_ONLY_CELLS`），`request` 由**决策者**写（DE-050 §0），
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
      // effects / PortFactWriter 链路：宿主把执行体给的 `FactDraft` 落成事实的唯一一处。
      // 环里的 `CollectingWriter` 只收集效果、不 `append`，因此不进这个名单——它是链路
      // 的中间段，不是第二个写方（这一格的判据是「写方唯一」，不是「impl 唯一」）。
      for (const m of code.matchAll(/\bimpl\s+(?:<[^>]*>\s*)?PortFactWriter\s+for\s+/g)) {
        const open = code.indexOf('{', m.index)
        if (open < 0) continue
        const close = matchBrace(code, open)
        if (close < 0) continue
        const body = code.slice(open, close + 1)
        for (const a of body.matchAll(/\.\s*append\s*\(/g)) {
          const idx = open + a.index
          lines.push(code.slice(0, idx).split('\n').length)
        }
      }
      if (lines.length > 0) {
        hits.set(path.relative(ROOT, abs).split(path.sep).join('/'), lines)
      }
    }
  }
  walk(path.join(ROOT, 'crates'))
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
    `协议格有 ${writers.length} 个写点文件（${writers.join(' / ')}）—— 协议格由引擎独占（DE-050 §0）：` +
      `第二处写点会让「四格 = 引擎的账」变成自觉，端口层的幂等键与租约也不再是单点逻辑`,
  )
}

// ───────────────────────── PTS-006 重试条文 ─────────────────────────

/**
 * DE-050 §3.3：重试已启用。
 *
 * 判据只认 §3.3 那一段文字。真表上 `tool` 端口 `max_attempts = 2`、行层对
 * attempt+1 重新开账，条文说「未启用」就是把在跑的路说成没跑。缺 `retry(attempt+1)`
 * 同样判红——条文没有把重试写出来，读者会以为它不在生产里跑。
 *
 * 找不到 §3.3 同样判红——锚点漂了不是「没什么可判」。
 */
function contractRetrySection() {
  const text = readText(PLAN_REL)
  if (text === null) {
    report('PTS-006', `读不出 ${PLAN_REL} —— 重试条文是这条判据的唯一输入，宁红不静默`)
    return null
  }
  const start = text.indexOf('### 3.3 ')
  if (start < 0) {
    report('PTS-006', `${PLAN_REL} 里找不到「### 3.3 」小节 —— 重试条文的锚点没了，判据没有输入`)
    return null
  }
  const end = text.indexOf('\n### ', start + 1)
  return text.slice(start, end < 0 ? text.length : end)
}

const retrySection = contractRetrySection()
if (retrySection !== null && !/retry\(attempt\+1\)/.test(retrySection)) {
  report(
    'PTS-006',
    `§3.3 没有写 \`retry(attempt+1)\` —— 重试已启用（\`tool\` 端口 \`max_attempts = 2\`，行层对 attempt+1 重新开账）；条文不写等于把在跑的路说成没跑`,
  )
} else if (retrySection !== null && retrySection.includes('未启用')) {
  report(
    'PTS-006',
    `§3.3 仍写「未启用」—— 重试已启用（\`tool\` 端口 \`max_attempts = 2\`，行层对 attempt+1 重新开账）；条文与机制分叉`,
  )
}

// ───────────────────────── PTS-007 读侧不认端口名 ─────────────────────────

/**
 * 收束载荷已经**盖章**（`stamp.role` / `stamp.usage` / `stamp.spend` /
 * `stamp.failure`），机制层与读侧只认章与数据。读侧一旦再比较端口名
 * （`port == "model"` / `port == PORT_TOOL`），「新增一个世界端口 ⇒ 改架构文件数 = 0」
 * 这句话就立刻作废：读侧又变成按端口逐个打补丁的地方。
 *
 * 命中形状：一行里同时出现
 *   ① 端口上下文（小写词 `port`，或解析到端口取值的常量名）；
 *   ② 比较运算 `==` / `!=`；
 *   ③ 一个**端口取值**（`"model"` 这类字面量，或解析到端口取值的常量）。
 * 三者齐了才是「按端口名分支」。`stamp.role == Some("reader")` 不算——
 * `"reader"` 不在端口取值名单里，名单外的一律不判。
 *
 * 已知边界（刻意）：
 *   - **适用范围是架构层与骨架的读侧**（`kernel` / `sdk` / `plugin-api` / `loop` /
 *     `session` 里那些「读事实并决定语义」的代码）：它判的是**耦合**——读方依赖
 *     别人的端口名。**能力插件比对自己端口的名字是所有权**，不是耦合：
 *     `tool/round_done.rs` 的 `fact.key.port == TOOL`、`telegram/port.rs` 的
 *     `input.port != NOTIFY` 因此刻意不判。扫描落点当前是 `sdk` / `plugin-api` /
 *     `session` 三棵树；`kernel` 与 `loop` 的端口无感由 PTS-002 / PTS-003 判。
 *   - `loop/plugin_rows.rs` 的 `is_row_port(port) == plugin_row` **保留**：`plugin_row`
 *     是**机制端口**（环用它执行自己的行表），不是世界端口，不进「新增一个世界端口
 *     改动架构文件数 = 0」的统计。
 *   - 只认比较运算，不认 `matches!(port, "model")` / `match port { "model" => … }`；
 *     真出现时仍可人工看见（误判成本高于收益）。
 *   - `*.test.rs` 与 `#[cfg(test)]` 模块不判：夹具按生产形状造事件，其中
 *     `port == "model"` 是在布置被测对象，不是读侧分支。
 *   - 端口取值名单来自端口声明（与 PTS-002 同一份）；名单读不出时本格没有输入，
 *     与其余各格一样**宁红不静默**。
 */
const PTS7_ROOTS = [
  'crates/symbio-sdk/src',
  'crates/symbio-plugin-api/src',
  'crates/symbio-plugin-session/src',
]

function pts7Files(root) {
  const out = []
  const skip = new Set(['target', 'node_modules', '.git', '.symbio', '.workbuddy-ai'])
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        if (!skip.has(entry.name)) walk(path.join(dir, entry.name))
        continue
      }
      if (entry.name.endsWith('.rs') && !entry.name.endsWith('.test.rs')) {
        out.push(path.join(dir, entry.name))
      }
    }
  }
  walk(root)
  return out
}

/** 一行的常量 token（`PORT_TOOL` 这种形状；跳过字段/方法名的小写） */
function pt7ConstTokens(line) {
  return [...line.matchAll(/\b([A-Z][A-Z0-9_]{2,})\b/g)].map((m) => m[1])
}

/** 常量名是否解析到端口取值（`PORT_TOOL` / `MODEL_PORT` / `use … as` 别名） */
function pt7IsPortConst(name) {
  const value = resolveName(name)
  return value !== undefined && portValues.has(value)
}

const PTS7_FILES = PTS7_ROOTS.flatMap((rel) => pts7Files(path.join(ROOT, rel)))
if (PTS7_FILES.length === 0 || portValues.size === 0) {
  report(
    'PTS-007',
    `读侧判据没有输入（扫描面 ${PTS7_FILES.length} 个文件、端口取值名单 ${portValues.size} 条）——` +
      `三个读侧树或端口声明读不出时，最省事的实现就是「没什么可判 ⇒ 通过」，故宁红不静默`,
  )
} else {
  for (const abs of PTS7_FILES) {
    let code
    try {
      code = blankTestModules(blankComments(fs.readFileSync(abs, 'utf8').replace(/\r\n/g, '\n')).join('\n'))
    } catch {
      continue
    }
    const rel = path.relative(ROOT, abs).split(path.sep).join('/')
    code.split('\n').forEach((line, i) => {
      if (!/\bport\b/.test(line) && !pt7ConstTokens(line).some(pt7IsPortConst)) return
      if (!/(?:==|!=)/.test(line)) return
      const literals = stringLiteralsOf(line).filter((s) => portValues.has(s))
      const consts = pt7ConstTokens(line).filter(pt7IsPortConst)
      if (literals.length === 0 && consts.length === 0) return
      const name = literals[0] ?? resolveName(consts[0])
      report(
        'PTS-007',
        `${rel}:${i + 1}  读侧按端口名 \`${name}\` 分支 —— 收束载荷已经盖章（role / usage / spend / failure），` +
          `读侧只认章与数据；按端口名比较会让「新增一个世界端口 ⇒ 改架构文件数 = 0」作废`,
      )
    })
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
  ruleLine(
    true,
    'PTS-005',
    `协议格写方唯一：${writers.join(' / ')}（命中 ${[...writePoints.values()].reduce((n, l) => n + l.length, 0)} 处构造）`,
  )
  ruleLine(true, 'PTS-006', `重试条文已写出：${PLAN_REL} §3.3`)
  ruleLine(
    true,
    'PTS-007',
    `读侧不认端口名：${PTS7_FILES.length} 个文件里零处「端口取值比较」（取值名单 ${[...portValues].sort().join(' / ')}）`,
  )
  process.exit(0)
}

console.error(`\n== port-surface-audit：${errors.length} 处违规 ==`)
for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
console.error(
  `\n${yellow('提示')}：判据口径见本脚本文件头与 docs/003-design/051-机制形状规格.md 的端口面判据；` +
    `改引擎后更新基线：node scripts/port-surface-audit.mjs --print-baseline`,
)
process.exit(1)
