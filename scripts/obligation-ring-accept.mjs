#!/usr/bin/env node
// @gate test
/**
 * obligation-ring-accept — 义务环符合性的 **CLI 实测判定**（口径 1 的第三件套）
 *
 * ## 它治的病
 *
 * 「跑了一遍 CLI，看着日志挺全」不是证据。判据必须能在**没有人读日志**的情况下重跑，
 * 否则同一次实测在不同人手里得出不同结论——这条缺口正是这样活了很久：
 * 短路轮两次打模型、协议格为 0，而代码里到处是「每次往返都有三格」的注释。
 *
 * ## 输入 = 一份 CLI 实测产物，不是构造的夹具
 *
 * 读 `<homedir>/session/<sid>/v2-events.wal`（事实层唯一真相）**及其归档段**
 * （`transcripts/wal-archive/`，`archive_before` 搬走的前缀；见 `readSessionEvents`）。
 * 只读 live 会把每一段被归档的历史判成悬空引用——会话一长就满屏假红。`--cli-log` 可选：
 * 给了它才能判 J8（账实相符），因为它要数**真实发生的模型 HTTP 调用**——这件事
 * 在 WAL 里原理上不可见（一次没入账的调用恰好不会留下任何格）。
 *
 * ## 读法锚点：`ring:` 身份，不是载荷（2026-11 重锚）
 *
 * 本脚本曾经按 `payload.id` 分组、按 `payload.port` 取端口、按 `turn.stage{decide}.model_calls`
 * 取观测。那套读法对应的是上一代事实本体（`entity: Turn`、`payload: {id, port, via}`、
 * 端口集 `turn/model/tool`）。现行 WAL 里这些**全部不存在**，于是它在一份完全合规的
 * 实测上报 29 项失效（28×J6「端口缺失」+ 1×J8），**而 J7 因为「0 观测 ⇔ 0 三格」
 * 静默判绿**——判据自己的两种最坏失效（假红 + 假绿）同时出现。
 *
 * 现行协议事实的身份在 **`event_id`**：`ring:{port}:{id}:{attempt}:{kind}[:{shard}]`
 * （`crates/symbio-host/src/lib.rs:594-607` 的 `ring_event_id`，解析口径同
 * `:641-673` 的 `ring_key_of`：`id` 自身可含 `:`，从右往左定界）。由此：
 *
 * - **端口是行名 / 端口名**（`plugin_row` / `model.assist` / `model` / `tool` / …），
 *   不是「五事实的类别」；
 * - `sent` 载荷只有 `deadline_ms`（挂起时再带 `shard`），**没有** `id` / `port`；
 * - 顺序是 **sent → request → response**：宿主先领租约（`before`）再问行（`request`）。
 *   旧判据假设的 `request → sent` 是上一代派发器的顺序；
 * - 一次往返的 `sent` 可能**多于一条**：执行体判「要挂起等人」时，`finish` 会补一条
 *   `{deadline_ms: null, shard: 1}` 的 `sent`（`symbio-plugin-loop/src/ring/engine.rs:713-726`），
 *   它不是第二次派发，也不是第二个分片。
 *
 * ## 判据（逐条，编号即输出编号）
 *
 * | # | 判据 | 口径来源 |
 * |---|------|----------|
 * | J1 | 按 `(port,id,attempt)` 分组的每一笔：`request` **恰一次**、`sent` **≥1**、终态（`response` \| `failed`）**恰一个**；带挂起 `sent`（`deadline_ms: null`）的笔允许终态为 0；授权被拒（`authz.denied`）且**没派发**时不要求 `sent`，且拒的笔不得以 `response` 收束 | DE-050 §0 五事实 |
 * | J2 | 每一条协议 `request` 都在某个**已开且未关**的 scope 里：它之前必须有一条 `user.message`，且这两者之间没有同一 scope 的 `session.terminated` | DE-001 I2（溯源不悬空的时间面） |
 * | J3 | 一笔的最小 `sent.seq` 严格小于它的终态 `seq`（挂起补的 `sent` 若晚于终态不算） | DE-050 §0（引擎写 before，应答在后） |
 * | J4 | 收束格（`chat.assistant.final` \| `chat.assistant.fallback`）的 `reason` 非空 | DE-050 §3.2 结局必须有名 |
 * | J5 | 每个事件的 `produced_by` 指向一条**已存在且更早**的 seq（含归档段） | DE-001 I2 溯源不悬空 |
 * | J6 | 孤名 = 0：协议事实必须带 `ring:` 身份（形状判据，判红）。端口准入改读**实测端口集**；静态 `DECLARED_PORTS` 降级为漂移对照（「新端口」/「声明未出现」都只提示、不判红） | D-447 ①（装载期硬拒的实测对账） |
 * | J7 | 模型往返的**观测 ⇔ 三格**双向对账，逐条路径（见 `MODEL_PATHS`）：有观测没三格 ⇒ 一条绕开债环的调用；有三格没观测 ⇒ 记了账没人发起 | DE-001 I2 全量口径 |
 * | J8 | 给了 `--cli-log` 时：`[LLM] 请求发起` 行数 == `sent`（模型端口、剔除挂起补条）数。`--strict` 下没给日志即判红 | SP-033 TC-1「没有别的调度代码」 |
 * | J9 | 压缩产物 ⇔ 摘要往返：每一条 `session.consolidated` 都必须指名到一笔**已收束**的 `compact-summarize` 往返（`model.assist` 端口，`id` 尾缀 = 产物指纹） | DE-053 D-336 的**产物侧** |
 *
 * ## 四条退场（SP-033 §5：写明退场 + 承接判据，不静默删）
 *
 * ① **J7 的 `compress/summary` 路径退场**。旧读法用 `turn.stage{compress}.model_calls`
 *    当观测；现行事实里没有这一格，也没有任何**独立于环**的事实能说「这一轮该压」——
 *    压缩是阈值触发的内部状态，不由某条事实宣告。承接判据是 **J9**：压缩的产物
 *    （`session.consolidated`）必须指名到一笔入环的摘要往返，产物与往返的配对关系
 *    比「观测次数」更强（它连**哪一笔**都钉住）。「有一次静默的摘要调用」由 **J8**
 *    兜住（摘要往返是 `model.assist` 的 HTTP 调用，没入环就数不平）。
 *
 * ② **J9 的「轮外重试具名项」退场**。旧读法数日志里的 `[Compress] 压缩 LLM 请求开始`
 *    与 `[Wake] RetryCompaction`；这两个标记在现行代码里**都不存在**——用户点
 *    `retry_compaction` 现在落一条 `message.removed`（`symbio-plugin-compact/src/debt.rs:37`、
 *    `current_owe` 的纪元推进），重试走的是**环内**的新债键，不再有「轮外那一笔」。
 *    所以那条边界不是被放宽，而是**结构上消失了**；承接判据仍是 J8（每一次摘要
 *    HTTP 调用都必须有 `sent`）+ J9 的产物配对。
 *
 * ③ **J7/J8 的豁免表退场（`MODEL_PATHS[].exemption` + J8 的 `exempted` 项）**。
 *    豁免表是「某条模型路径还没入环」时的临时桥，条目必须带任务号与期限。现行架构里
 *    分类与措辞两条产线**都已入环**（各自有 `model.assist` 的三格），没有可豁免的对象；
 *    留一张空表等于留一个「以后往里加一条就绿」的口子。所以 J8 的等式收敛成
 *    `HTTP == sent(模型端口)`——这不是放宽：原来的豁免项本身就是**已知债**，现在债没了。
 *    「有一条静默的模型调用」仍由 J8 的差额判红。
 *
 * ④ **J6 的静态端口白名单拒绝 + 配套的「表 ↔ 源码声明」漂移守卫退场（D-447 ①）**。
 *    白名单要求「加端口先改验收表」，与 D-440「端口名不进声明表」冲突；守卫又把
 *    「新增源码声明不改表就红」钉成了门禁。承接判据是**动态端口集 + 漂移对照提示**：
 *    端口准入读实测端口集，静态表只报「新端口 / 声明未出现」，两侧都不判红。形状侧
 *    （`ring:` 身份、J1/J3/J7/J9）一条没放宽——退场的是「名单当拒绝依据」，不是判据。
 *
 * ## 端口准入为什么不再查静态表（D-447 ①）
 *
 * J6 曾经判「这次运行有没有人往一个没声明的端口上写事实」，用一张带出处的常量表
 * `DECLARED_PORTS` 当白名单。那与 D-440「端口名不进声明表」正面冲突：**加端口先改
 * 验收表**才能不判红，而端口本就是装配期动态收集的结果。旧版还吃过一次反方向的亏——
 * 常量停在 `['turn','model','tool']`、声明侧早换成了行名，J6 把整份合规 WAL 判成
 * 28 项假红；后来配了一条「扫源码声明 ↔ 表逐项比对」的漂移守卫，多一个端口不改表就红，
 * 反而把「加端口要改表」钉得更死。
 *
 * 现行口径：协议事实必须带 `ring:` 身份（这一条是形状，缺了它端口无从判定，仍判红）；
 * 身份里的端口**就是本次实测的端口**，不再回表匹配。`DECLARED_PORTS` 保留、降级为
 * **漂移对照**：实测端口不在表里 ⇒ 提示「新端口」；表里声明但本次没出现 ⇒ 提示一声。
 * 两侧都只提示、不进失效计数，也不影响退出码。判红的仍是协议形状类问题（J1 四格 /
 * 终态 / J3 顺序 / J7 配对 / J9 产物配对）。回归夹具 `obligation-ring-accept.test.mjs`
 * 里有一条：含未在静态表中的自定义端口 `custom_probe` 的完整事实序列，J6 仍通过且输出
 * 带「新端口」。
 *
 * ## 「孤名 = 0」的拦截 owner（静态白名单退场后）
 *
 * 名单不当拒绝依据之后，孤儿端口名由**三处各自判得出**的地方拦住；J6 是它们的实测
 * 对账，不是拦截面本身：
 *
 * 1. **装配期（唯一入口）**：端口集是本次装配新建的 `PortBindings`
 *    （`crates/symbio-sdk/src/capability/port.rs`），只有插件经 `register_ports` /
 *    `register_tool` 登记过的端口才有执行体，host `gather` 折 `snapshot()` 成交执行体表。
 *    没人登记的名字在装配结果里根本不存在；同名两绑定 / 无执行体由 `bind` 落错误，
 *    `snapshot()` 任一冲突**整份** `Err` ⇒ 装配失败（`DE-056 D-468`）。
 * 2. **运行期环（失败不静默）**：`crates/symbio-plugin-loop/src/ring/engine.rs` 派发时
 *    取不到执行体，仍按协议写全 `sent` / `request` / `failed`，`reason` 是
 *    `no provider for port ...`；不是跳过这一笔，也不是用 `response` 冒充成功。反例
 *    `crates/symbio-plugin-loop/src/ring/engine.test.rs` 的
 *    `absent_executor_lands_failed_and_names_the_port` 钉住三格齐全与点名。
 * 3. **运行期判定器（形状对账）**：本文件 J6 要求每条协议事实带 `ring:` 身份，端口因此
 *    可观测；J1 同时要求每笔有终态。一个没有归属的端口名只要真的产生了协议事实，就会
 *    以第 2 条的 `failed` 出现在 WAL 里，不会从判定面消失。静态 `DECLARED_PORTS` 只回答
 *    「这次跑出了哪些端口」，不回答「准不准」。
 *
 * owner 是第 1 / 2 条：判红点分别在 `PortBindings::snapshot()`（装配失败）与
 * `engine.rs` 的 `failed` 事实（非静默终态）。J6 的 `ring:` 形状判据与 J1 的终态判据
 * 让这两处拦得住、也看得见。
 *
 * ## 观测口径为什么读「模型端口上的 sent」而不是「某种来源字段」
 *
 * J8 只数**入账的派发**：`sent` 是引擎在执行体之前落的 before 事实，一次执行体
 * 调用恰一条（挂起补条除外，它不是派发）。读 `response` 会把「失败但有终态」的
 * 往返漏掉；读规则/兜底字段会把「调了模型但答不可解析」读成没调。
 *
 * 用法：
 *   node scripts/obligation-ring-accept.mjs --homedir=.symbio --session=verify-ring-3 \
 *        [--session=verify-ring-2] [--cli-log=<file>] [--strict]
 * 退出码：0 = 全项通过；1 = 有失效；2 = 输入读不出（会话目录 / WAL 缺失 / JSON 不可解析）。
 *
 * ⚠️ `--cli-log` 那份日志必须是**一次 CLI 实测的产物**，且它的跨度 = 所列会话的跨度
 * （多会话时 J8 只能整轮判）。`[LLM] 请求发起` 是 **INFO** 级 ⇒ 不带 `-v` 也有；
 * 少了它 J8 数不出真实 HTTP 调用数，只能报「未判」——`--strict` 下那判红。
 */

import fs from 'node:fs'
import path from 'node:path'
import { green, red, dim } from './color.mjs'

// 判据码命名空间：本脚本不占 `GATE_CODES.md` 的编号空间（它不是门禁步骤，
// 判据编号 J1…J9 只在本文件与任务 evidence 里被引用）。

/** 五个协议事实名（DE-050 §0：事实名是闭集，端口是载荷）。 */
const PROTOCOL = new Set(['request', 'sent', 'response', 'failed'])
/** 终态：一次往返的两种结局。 */
const TERMINAL = new Set(['response', 'failed'])
/** 协议事实的入库身份前缀（`crates/symbio-host/src/lib.rs:591`）。 */
const RING = 'ring:'

/**
 * **静态漂移对照表**（D-447 ① 之后不再是准入白名单）。
 *
 * 端口准入改读**实测端口集**（见 J6）：新增端口不必先在这里登记也能通过 J6。
 * 这张表只用来报两种**漂移提示**（都不判红）：实测出现而表里没有 ⇒ 「新端口」；
 * 表里声明而本次没出现 ⇒ 只是没跑到。每一项保留出处，便于对照时知道它从哪来；
 * 名字的出处有两种形态：端口提供者的 `PortProvider::ports()`，或 `PortDecl { port }`。
 */
const DECLARED_PORTS = [
  // `model` 插件的主往返端口
  { port: 'model', source: 'crates/symbio-plugin-model/src/port.rs:69（PortProvider::ports → PORT_MODEL，api/vocab.rs:31）' },
  // `model` 插件的一次性问答端口（classify / compose / compact 的模型往返都走它）
  { port: 'model.assist', source: 'crates/symbio-plugin-model/src/assist.rs:67（PortProvider::ports → PORT_MODEL_ASSIST，api/vocab.rs:21）' },
  // 工具执行端口
  { port: 'tool', source: 'crates/symbio-plugin-tool/src/declarations.rs:79（PortDecl{port: TOOL} → PORT_TOOL，api/vocab.rs:24）' },
  // 行执行端口（所有 `by: plugin_row` 的债行都走它；`plugin_row` 是宿主词汇）
  { port: 'plugin_row', source: 'crates/symbio-plugin-loop/src/plugin_rows.rs:58（port: PLUGIN_ROW，sdk/src/plugin/rows.rs:56）' },
  // 上下文压缩的落格端口（摘要走 model.assist，产物由这条落）
  { port: 'compact', source: 'crates/symbio-plugin-compact/src/port.rs:38（PortProvider::ports → COMPACT_PORT）' },
  // 意向提升端口
  { port: 'conation', source: 'crates/symbio-plugin-conation/src/port.rs:43（PortProvider::ports → PROMOTE_PORT，debt.rs:32）' },
  // 记忆端口
  { port: 'memory', source: 'crates/symbio-plugin-memory/src/port.rs:29（PortProvider::ports → MEMORY_PORT）' },
  // 探针端口的回声往返（自检用）
  { port: 'probe_echo', source: 'crates/symbio-plugin-probe/src/declarations.rs:19（PortDecl{port: PROBE_PORT}，probe/src/port.rs:10）' },
  // 演示探针（demo 插件）
  { port: 'demo', source: 'crates/symbio-demo-probe/src/port.rs:9（PortProvider::ports → DEMO_PORT）' },
  // 电报外发端口
  { port: 'notify', source: 'crates/symbio-plugin-telegram/src/declarations.rs:22（PortDecl{port: NOTIFY}，telegram/src/port.rs:20）' },
  // 网页探活端口
  { port: 'web_probe', source: 'crates/symbio-plugin-web/src/probe.rs:31（PortProvider::ports → "web_probe"）' },
]
const DECLARED = new Set(DECLARED_PORTS.map((p) => p.port))

/**
 * **会发 HTTP 的端口**（J8 的左边）。
 *
 * 判据是「这一条 `sent` 背后必然有一次模型往返」，所以名单只收 `model` 声明的两个
 * 端口：主往返 `model` 与一次性问答 `model.assist`。`tool` 会发网络请求（web_fetch
 * 等），但它的调用不是**模型**往返，日志里也没有 `[LLM] 请求发起`。
 */
const MODEL_PORTS = new Set(['model', 'model.assist'])

/** 分类行的名字：`classify.decide:<root_id>`（`crates/symbio-plugin-classify/src/`）。 */
const CLASSIFY_ROW = 'classify.decide'
/** 措辞产线的用途名，落在 `model.assist` 的 `request.payload.via`（`compose/src/debt.rs:102`）。 */
const COMPOSE_VIA = 'compose/wording'
/** 摘要行的名字：`compact-summarize:<指纹>`（`crates/symbio-plugin-compact/src/debt.rs:79`）。 */
const SUMMARIZE_ROW = 'compact-summarize'
/** 压缩产物事实名；`event_id` 形如 `session.consolidated:<指纹>`。 */
const CONSOLIDATED = 'session.consolidated'
/** 判决事实（模型 / 规则两条路径共用这两个种类，靠 `reason` 分来源）。 */
const CLASSIFY_ESCALATE = 'classify.escalate'
const CLASSIFY_ANSWERED = 'classify.answered'

/**
 * **只有模型能产出的理由码**（J7 的观测口径）。
 *
 * 规则表只有问候 / 致谢 / 确认 / 空输入四档，全部落 `Answered`；模型四词
 * （`direct` / `clarify` / `refuse` / `work`）映射为 `from_context` / `clarify` /
 * `refuse` / `needs_work`，解析失败落 `unclassified`。所以下面这集合里的理由码
 * 出现一次 = 分类产线**真的打过一次模型**（`crates/symbio-plugin-classify/src/decide.rs:62-100`）。
 * 反过来，规则四档不在集合里——它们 0 次调用、0 组三格，双向都平。
 */
const MODEL_VERDICT_REASONS = new Set([
  'from_context',
  'clarify',
  'refuse',
  'needs_work',
  'unclassified',
])

/** 收束格：两种都算（`chat.assistant.fallback` 由 `close` 执行体独占写）。 */
const CLOSING = ['chat.assistant.final', 'chat.assistant.fallback']

/**
 * **模型往返的入账路径表**（J7 的判据本体）。
 *
 * 一条路径 = 「事实面上的某个观测 ⇒ 必然对应一组同端口的往返三格」。两个方向都判，
 * 缺哪个都红：
 *  - 只有三格没有观测 ⇒ 一次没人发起的往返；
 *  - 只有观测没有三格 ⇒ **一条绕开债环的模型调用**。
 *
 * `observed` 返回**权重**（这一格代表几次模型往返），求和即得观测次数。
 * `rounds` 是这条路径的「三格组」判据：读**逻辑笔**（`port + id`，忽略 `attempt`），
 * 因为一次失败重试会换尝试号，但观测侧仍只有一次判决。
 *
 * `guard` 非空 = 这条路径只在配置允许时才判：`compose` 关掉时 `from_context` 照样
 * 会落（分类不知道下游裁掉了措辞产线），此时强求三格是**假红**。
 */
const MODEL_PATHS = [
  {
    via: 'classify/decide',
    // 观测：经模型产出的判决事实（规则四档不在集合里，见 `MODEL_VERDICT_REASONS`）。
    observed: (e) => {
      if (e.kind === CLASSIFY_ESCALATE) return 1
      if (e.kind === CLASSIFY_ANSWERED) {
        const reason = e.payload?.reason
        return typeof reason === 'string' && MODEL_VERDICT_REASONS.has(reason) ? 1 : 0
      }
      return 0
    },
    rounds: (r) => r.port === 'model.assist' && r.id.startsWith(`${CLASSIFY_ROW}:`),
    guard: () => true,
    why: '轮首判决的那一次分类调用',
  },
  {
    via: 'compose/wording',
    // 观测：`Answered{from_context}` 是**唯一**要组织措辞的判决
    // （`Verdict::needs_wording_generation` 是这条规则的唯一实现，两侧共用）。
    observed: (e) =>
      e.kind === CLASSIFY_ANSWERED && e.payload?.reason === 'from_context' ? 1 : 0,
    // 三格：`request.payload.via` 是这条产线自报的用途名（旧本体里它在三格载荷上，
    // 现在仍在 `request` 载荷上——这是 compose 唯一保留、也是唯一需要的自报位）。
    rounds: (r) => r.port === 'model.assist' && r.request?.payload?.via === COMPOSE_VIA,
    // 措辞产线被关掉时，`from_context` 仍会落，但没有三格可要——不是缺口。
    guard: (cfg) => cfg.composeEnabled !== false,
    why: '短路轮的答话组织：编排层发起的那一次模型往返',
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
    console.error(red('用法: --homedir=<dir> --session=<sid> [--session=<sid> ...] [--cli-log=<file>] [--strict]'))
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
 * 排序键；逐段重放后按 `first` 升序拼接，live 接在后面。目录不存在 / 某段读不开 ⇒ 当作
 * 没有（与 Rust 侧「读不开就是没有」同一口径）。最后按 `seq` 稳定排序：后续所有判据
 * 都假设数组单调递增，段与 live 的拼接顺序不能成为判据的一部分。
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
  return events.sort((a, b) => (a.seq ?? -1) - (b.seq ?? -1))
}

/**
 * 协议事实的 `ring:` 身份 → `{ port, id, attempt, shard }`；不是协议事实 / 身份坏了 ⇒ null。
 *
 * 形状 `ring:{port}:{id}:{attempt}:{kind}[:{shard}]`，`id` 自身可含 `:` ⇒ 从右定界：
 * 最右可能是分片号，往左依次是种类与尝试号，剩下的中段整体是 `id`。这是
 * `crates/symbio-host/src/lib.rs:641-673`（`ring_key_of`）的同一口径；两侧若漂移，
 * J6 的「缺 ring 身份」会把它判出来。
 */
function ringKey(e) {
  const raw = e.event_id
  if (typeof raw !== 'string' || !raw.startsWith(RING)) return null
  const parts = raw.slice(RING.length).split(':')
  const kindAt = parts.lastIndexOf(e.kind)
  // `kind` 右侧只允许一位分片号；左侧必须还有端口、至少一段 id、尝试号。
  if (kindAt < 3 || parts.length - kindAt > 2) return null
  const attempt = Number(parts[kindAt - 1])
  if (!Number.isInteger(attempt) || attempt < 0) return null
  const port = parts[0]
  if (!port) return null
  return {
    port,
    id: parts.slice(1, kindAt - 1).join(':'),
    attempt,
    shard: parts.length > kindAt + 1 ? parts[kindAt + 1] : null,
  }
}

/** 一笔派发的挂起补条：`{deadline_ms: null, shard: N}`（`engine.rs:713-726`），不是新派发。 */
function suspended(e) {
  return e.kind === 'sent' && e.payload?.deadline_ms === null
}

/** 按 `(port,id,attempt)` 把协议事实折成一笔往返。 */
function groupRounds(events) {
  const byKey = new Map()
  for (const e of events) {
    if (!PROTOCOL.has(e.kind)) continue
    const k = ringKey(e)
    if (!k) continue
    const key = `${k.port}\u0000${k.id}\u0000${k.attempt}`
    if (!byKey.has(key)) byKey.set(key, { ...k, facts: [] })
    byKey.get(key).facts.push(e)
  }
  return byKey
}

/**
 * 按 `(port,id)` 把协议事实折成**逻辑笔**（忽略尝试号）。
 *
 * J7 用它：一次失败重试换的是尝试号，不是「第二次判决」，观测侧仍然只有一格；
 * 拿 `(port,id,attempt)` 组数当三格数会把重试判成「记了账没人发起」。
 */
function logicalRounds(events) {
  const byKey = new Map()
  for (const e of events) {
    if (!PROTOCOL.has(e.kind)) continue
    const k = ringKey(e)
    if (!k) continue
    const key = `${k.port}\u0000${k.id}`
    if (!byKey.has(key)) byKey.set(key, { port: k.port, id: k.id, facts: [] })
    byKey.get(key).facts.push(e)
  }
  const out = []
  for (const r of byKey.values()) {
    out.push({
      port: r.port,
      id: r.id,
      facts: r.facts,
      request: r.facts.find((f) => f.kind === 'request') ?? null,
    })
  }
  return out
}

/** 会话配置里与 J7 守卫有关的两个开关（缺省视为开：出厂默认就是开）。 */
function sessionFlags(events) {
  let classifyEnabled = true
  let composeEnabled = true
  for (const e of events) {
    if (e.kind !== 'session.config') continue
    const p = e.payload ?? {}
    if (typeof p.classify_enabled === 'boolean') classifyEnabled = p.classify_enabled
    if (typeof p.compose_enabled === 'boolean') composeEnabled = p.compose_enabled
  }
  return { classifyEnabled, composeEnabled }
}

/**
 * 双向对账（J7 的判据本体，逐条路径）。
 *
 * `observations` / `groups` 都按**逻辑笔**计：同一笔的重试不制造第二个观测，也不
 * 制造第二组三格。
 */
function comparePath(p, observations, groups, note) {
  if (observations > groups) {
    note(`J7 ${p.via}: 观测 ${observations} 次、入账三格 ${groups} 组——有 ${observations - groups} 次模型往返没进债环`)
  }
  if (groups > observations) {
    note(`J7 ${p.via}: 入账三格 ${groups} 组 > 观测 ${observations} 次——记了账却没人发起这条路径`)
  }
}

/** J7（WAL 口径）：逐条判能在事实层观测的路径。 */
function judgePaths(events, logical, cfg, note) {
  for (const p of MODEL_PATHS) {
    if (!p.guard(cfg)) continue
    const groups = logical.filter((r) => p.rounds(r)).length
    const observations = events.reduce((n, e) => n + (p.observed(e) || 0), 0)
    comparePath(p, observations, groups, note)
  }
}

function judgeSession(homedir, sid) {
  const wal = path.join(homedir, 'session', sid, 'v2-events.wal')
  if (!fs.existsSync(wal)) return { fatal: `读不到 WAL: ${wal}` }
  const events = readSessionEvents(wal)
  if (events.length === 0) return { fatal: `${wal} 是空的` }

  const rounds = groupRounds(events)
  const logical = logicalRounds(events)
  const cfg = sessionFlags(events)
  const problems = []
  const note = (m) => problems.push(m)

  // 授权被拒的笔：`authz.denied` 是世界事实（无 `ring:` 前缀），载荷带
  // `{port,id,attempt}`（`symbio-plugin-tool/src/port.rs:301-312`）。
  const deniedKeys = new Set()
  for (const e of events) {
    if (e.kind !== 'authz.denied') continue
    const p = e.payload ?? {}
    if (typeof p.port === 'string' && typeof p.id === 'string' && Number.isInteger(p.attempt)) {
      deniedKeys.add(`${p.port}\u0000${p.id}\u0000${p.attempt}`)
    }
  }

  // 可重试失败的「终态」是一条 `retry`：它写在**下一次尝试的键**上，`payload.from`
  // 指回本次尝试号（`engine.rs:727-737`）。所以按 `(port,id,attempt)` 分组时，被重试
  // 掉的那一笔自己没有 response|failed，**这是对的**——它的结局是下一笔。
  const retriedFrom = new Map()
  for (const e of events) {
    if (e.kind !== 'retry') continue
    const k = ringKey(e)
    const from = e.payload?.from
    if (!k || !Number.isInteger(from)) continue
    retriedFrom.set(`${k.port}\u0000${k.id}\u0000${from}`, e)
  }

  // J1 齐整（唯一 request、≥1 sent、恰一个终态）+ J3 顺序
  for (const [key, g] of rounds) {
    const requests = g.facts.filter((f) => f.kind === 'request')
    const sents = g.facts.filter((f) => f.kind === 'sent')
    const terminals = g.facts.filter((f) => TERMINAL.has(f.kind))
    const denied = deniedKeys.has(key)
    // 执行体判「要挂起等人」时会补一条 `{deadline_ms: null, shard}` 的 sent；那一笔
    // 在新触发到来前没有终态是**对的**，所以终态允许为 0。
    const isSuspended = sents.some((f) => suspended(f))
    const retried = retriedFrom.get(key) ?? null
    if (requests.length !== 1) {
      note(`J1 ${g.port}/${g.id}#${g.attempt}: request 有 ${requests.length} 笔（应为 1）—— ${g.facts.map((f) => f.kind).join('/')}`)
    }
    // **没放行就没派发**：`authz.denied` 的笔可以没有 sent（e2e T78 的同一条判据）。
    if (sents.length === 0 && !denied) note(`J1 ${g.port}/${g.id}#${g.attempt}: 缺 sent`)
    if (terminals.length > 1) {
      note(`J1 ${g.port}/${g.id}#${g.attempt}: 终态（response|failed）有 ${terminals.length} 笔（应为 1）`)
    }
    if (terminals.length === 0 && !isSuspended && !retried) {
      note(`J1 ${g.port}/${g.id}#${g.attempt}: 缺终态（response|failed / retry / 挂起 sent）`)
    }
    // 拒的笔必须按失败收束：拒不是一种成功结局。
    if (denied && terminals.some((t) => t.kind === 'response')) {
      note(`J1 ${g.port}/${g.id}#${g.attempt}: 授权被拒却以 response 收束（应落 failed）`)
    }
    // J3 sent 先于终态：取**最早**那条 sent（挂起补条可能晚于终态，它不是派发锚点）。
    // 被重试掉的一笔，它的「终态」是那条 retry 事实。
    const ends = retried ? [...terminals, retried] : terminals
    if (sents.length > 0 && ends.length > 0) {
      const firstSent = Math.min(...sents.map((s) => s.seq))
      if (!ends.every((t) => t.seq > firstSent)) {
        note(`J3 ${g.port}/${g.id}#${g.attempt}: sent#${firstSent} 不在终态之前`)
      }
    }
  }

  // J2 输入格先于判决：每一条协议 `request` 都必须落在某个**已开且未关**的 scope 里。
  //
  // 现行事实没有 `turn` 字段（scope 由 `user.message` 开启、`session.terminated` 关闭），
  // 所以判据改成「同一条时间线」的形式：request 之前有 user.message，且这两者之间
  // 没有**同一个 root** 的终止格。少读这一条，「scope 关了还在派发」就看不见。
  const users = events.filter((e) => e.kind === 'user.message')
  if (users.length === 0) note('J2 没有 user.message 格（这次实测没有输入？）')
  const terms = events.filter((e) => e.kind === 'session.terminated')
  for (const e of events) {
    if (e.kind !== 'request') continue
    const opened = users.filter((u) => u.seq < e.seq)
    if (opened.length === 0) {
      note(`J2 request#${e.seq} 之前没有 user.message 输入格`)
      continue
    }
    const opener = opened[opened.length - 1]
    const root = opener.payload?.id ?? opener.event_id
    if (terms.some((t) => t.seq > opener.seq && t.seq < e.seq && t.payload?.root_id === root)) {
      note(`J2 request#${e.seq} 落在已终止的 scope（root=${root}）之后，此前没有新的输入格`)
    }
  }
  // J4 收束格 reason 非空
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
  // J6 形状判据：协议事实必须带 ring: 身份（缺了它端口无从判定，判红）。
  // 端口准入不再查静态白名单（D-447 ①）：实测端口集就是本次运行的动态端口集，
  // 静态 DECLARED_PORTS 降级为**漂移对照**，两侧差异只提示、不进 problems。
  const observedPorts = new Set()
  for (const e of events) {
    if (!PROTOCOL.has(e.kind)) continue
    const k = ringKey(e)
    if (!k) {
      note(`J6 ${e.kind}#${e.seq} 的 event_id "${e.event_id}" 不是 ring: 身份——端口无从判定`)
      continue
    }
    observedPorts.add(k.port)
  }
  // 漂移对照（只提示，不判红）：
  //  - 实测出现但不在静态表里 ⇒ 新端口（新增端口不必先改验收表）；
  //  - 静态表里声明但本次没出现 ⇒ 只是没跑到，不是失效。
  const drift = []
  for (const p of [...observedPorts].sort()) {
    if (!DECLARED.has(p)) {
      drift.push(`新端口 "${p}"：本次实测出现，但不在静态漂移对照表里（D-447 ①：新增端口无需改验收表）`)
    }
  }
  const absent = [...DECLARED].filter((p) => !observedPorts.has(p)).sort()
  if (absent.length > 0) {
    drift.push(`静态漂移对照表：${absent.length} 个声明端口本次未出现（${absent.join(' / ')}）——仅对照，非失效`)
  }
  // J7 观测 ⇔ 三格（逐条路径；退场的 compress 路径见文件头）
  judgePaths(events, logical, cfg, note)

  return { events, rounds, logical, cfg, sid, problems, drift }
}

/** 数真实模型 HTTP 调用：`[LLM] 请求发起 <endpoint> (model=..., msgs=..., ...)` 一次调用一行。 */
function countHttp(logLines) {
  return logLines.filter((l) => l.includes('[LLM] 请求发起')).length
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
  const r = judgeSession(args.homedir, sid)
  if (r.fatal) {
    console.log(red(`✗ ${sid}: ${r.fatal}`))
    process.exit(2)
  }
  // J8 的左边：模型端口的 sent，剔除挂起补条（它不是派发）。
  r.sentModel = r.events.filter((e) => {
    if (e.kind !== 'sent' || suspended(e)) return false
    const k = ringKey(e)
    return k !== null && MODEL_PORTS.has(k.port)
  }).length
  judged.push(r)
  summary.push(
    `${sid}: sent(模型端口) ${r.sentModel} / 往返 ${r.rounds.size} 笔（(port,id,attempt)）/ 逻辑债 ${r.logical.length} 条`,
  )
  if (r.problems.length === 0) {
    console.log(green(`✓ ${sid}: J1…J7（事实层口径）全项通过（${r.events.length} 格 / ${r.rounds.size} 笔端口往返）`))
  } else {
    totalProblems += r.problems.length
    console.log(red(`✗ ${sid}: ${r.problems.length} 项失效（${r.events.length} 格 / ${r.rounds.size} 笔往返）`))
    for (const p of r.problems) console.log(red(`    ${p}`))
  }
  // 漂移提示只输出 dim，不进 problems、不影响退出码（D-447 ①）。
  for (const d of r.drift) console.log(dim(`    ${d}`))
}
for (const s of summary) console.log(dim(`    ${s}`))

// J8 只能**整轮**判：一次 `--cli-log` 覆盖所列全部会话，而日志行内没有会话号——
// 把计数按会话摊开就是编数。文件头的输入约定（日志跨度 = 所列会话的跨度）在这里兑现。
if (logLines) {
  const http = countHttp(logLines)
  const sentModel = judged.reduce((n, r) => n + r.sentModel, 0)
  console.log(dim(`    整轮：HTTP ${http} / sent(模型端口) ${sentModel}`))
  if (http !== sentModel) {
    totalProblems += 1
    console.log(
      red(
        `    J8 真实模型调用 ${http} 次 ≠ 入账 ${sentModel} 次（模型端口的 sent，剔除挂起补条）——` +
          '差额 = 一条没人记账的模型路径',
      ),
    )
  } else {
    console.log(green('✓ 整轮：J8 通过'))
  }
} else if (args.strict) {
  // strict 的意义是「不判等于放行」：J8 要数真实发生的模型 HTTP 调用，那只在 CLI
  // 日志里——没给日志就是没判过，不能报成全过。
  console.log(red('✗ --strict：没给 --cli-log——J8（账实相符）未判'))
  totalProblems += 1
} else {
  console.log(dim('    J8 未判：没有 --cli-log（账实相符需要真实 HTTP 计数）'))
}

// ── J9：压缩产物 ⇔ 摘要往返（DE-053 D-336 的产物侧）────────────────────
//
// 这一条**只读事实**，不需要日志：压缩产物的 `event_id` 是
// `session.consolidated:<指纹>`，摘要往返的债键是 `compact-summarize:<同一个指纹>`
// （两侧的指纹来自同一把 `key`：`crates/symbio-plugin-compact/src/debt.rs:143-186`
// 与 `port.rs:113`）。
//
// 只看一个方向：**产物必须指名到一笔已收束的往返**。反方向（有往返没产物）不判：
// 写压缩产物的那一行可能在会话结束时还没跑完（摘要成功、落格未到），那不是静默
// 路径。真正的「有往返没账」由 J8 兜住。
// 配对在**会话内**做：两个会话各自压各自的段，指纹跨会话撞车不该被当成同一笔。
let j9Consolidated = 0
let j9Summaries = 0
let j9Closed = 0
let j9Bad = 0
for (const r of judged) {
  const consolidated = r.events.filter((e) => e.kind === CONSOLIDATED)
  const summarize = r.logical.filter(
    (x) => x.port === 'model.assist' && x.id.startsWith(`${SUMMARIZE_ROW}:`),
  )
  const closed = summarize.filter((x) => x.facts.some((f) => f.kind === 'response'))
  j9Consolidated += consolidated.length
  j9Summaries += summarize.length
  j9Closed += closed.length
  for (const c of consolidated) {
    const prefix = `${CONSOLIDATED}:`
    const fingerprint = String(c.event_id).startsWith(prefix) ? String(c.event_id).slice(prefix.length) : null
    const hit = fingerprint ? closed.find((x) => x.id === `${SUMMARIZE_ROW}:${fingerprint}`) : null
    if (!hit) {
      j9Bad += 1
      console.log(
        red(
          `    J9 ${r.sid} 压缩产物 ${c.event_id}（seq#${c.seq}）找不到配对的摘要往返：` +
            `应有一笔 ${SUMMARIZE_ROW}:${fingerprint ?? '(身份缺失)'} 的 response —— 产物不是模型产出的`,
        ),
      )
    }
  }
}
console.log(
  dim(`    整轮：压缩产物 ${j9Consolidated} / 摘要往返 ${j9Summaries}（已收束 ${j9Closed}）`),
)
if (j9Bad > 0) totalProblems += j9Bad
else console.log(green('✓ 整轮：J9 通过（每笔压缩产物都指名到一次入环的摘要往返）'))

console.log(totalProblems === 0 ? green('义务环实测判定：全项通过') : red(`义务环实测判定：${totalProblems} 项失效`))
process.exit(totalProblems === 0 ? 0 : 1)
