// gate.d 共享库：基线、输出解析、环境探针。任务模块按需引入。
//
// ## 基线棘轮：向上自动、向下才红
//
// 三态判定只有**一处**定义（`ratchetVerdict`，纯函数）：低于基线红、高出但在
// `BASELINE_GRACE` 内黄字提示、超出容差红。`BASELINE` 本身被下调由
// `gate.d/35-baseline.mjs`（元守卫）判红——那是向下方向唯一的放宽口，且必须配
// `// baseline-allow <key>: <理由>` 一行豁免。
//
// 但**应用到门禁时**走 `ratchetOrBump`（带副作用的网关版）：实测 > 基线就**就地把数字
// 写回本文件**（只增、不写注释、本地顺手 `git add`），判绿；实测 < 基线才判红。
// 理由：棘轮的真实价值只在向下方向（测试被删 ⇒ 红，逼人查）。向上是预期内的正常增长，
// 不该拦人、更不该每次手写一段论证——那论证正是被本仓规则禁止的「源码内变更史」。
// 故本文件里 `BASELINE` 对象**不应再堆各批次的变更注释**；要追变更看提交历史。

import fs from 'node:fs'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { stripAnsi, yellow, green, red } from '../color.mjs'

// 本文件自身路径：自动上调 `BASELINE` 时就地重写这里（`acceptBaseline`）。
const SHARED_MJS = fileURLToPath(import.meta.url)
const SHARED_DIR = path.dirname(SHARED_MJS)
const REPO_ROOT = path.resolve(SHARED_DIR, '..', '..')

/**
 * 通过数基线（**只增不减**；跑高了请更新这里并说明理由；**跑低了要说明理由**）
 *
 * S27（916）：VdfsChange 信封 = `{path, data?}`、操作枚举整个退役——信封构造
 *      守卫（`bare` 无载荷 / `with_data` 带载荷 / 无 `change` 键）、门面原样透传、
 *      transcript 四帧生命周期（首帧全量副本 / delta 窄载荷 / removed 状态帧 /
 *      运行态随节点视图）。`transcript_stream` 已随 ADR-025 退役，908 中相关
 *      用例随之删除，由 VDFS 变更通道的新用例接替。
 * 908：流式链路评审 A–D 批的回归锚点——`event_bus` 满通道语义（不摘除订阅 /
 *      `Closed` 才摘除 / resync 标记形状 / 满通道后标记必达）与**扇出不复制载荷**
 *      （`PluginPayload::Data(Arc<Value>)` 的 `Arc::ptr_eq` 断言），外加
 *      `transcript_stream` 的扇出共享与信封可解回事件（该模块已退役）。逐条见
 *      `docs/archive/streaming-chain-review-2026-09-22.md` §0.1。
 * 901：转写帧日志分级——`FrameLogLevel`（骨架 INFO / 细节 DEBUG）与
 *      `frame_log_of` 的相位判据，折行器新增 `foldable`（骨架帧与首帧不可折）。
 *      新增「骨架帧不可折」「相位分界」「行尾不落空格」用例（3）。
 * 898：控制台日志降噪——新增 `logger` 级别闸门用例（2）与 telegram 配置「缺键不是错误」
 *      用例（2）。闸门：无 subscriber 路径默认 INFO，debug 静默（`--verbose` / `SYMBIO_LOG`
 *      放开）；telegram 结构级 `#[serde(default)]` 使「只有身份键的 PLUGIN.yml」解析为默认值
 *      而非 Err（此前每次启动一条假 WARN）。
 * 894：S24 收口——转写流帧收成一条 `ChatMessage`：删除 `NodeOp` / `NodeChange`，
 *      `delta`（增量，与 `content` 互斥）与 `status = removed`（删除状态迁移）落到
 *      `ChatMessage` 上，告警下沉为 `TranscriptWriter::warn`；转写核心日志的「纯增量」
 *      折行（`DeltaLogCoalescer`）与请求级会话快照的回退判据一并落地。相应新增/重排了
 *      消息合并、删除帧、压缩终态、转写往返、折行边界（`seq`/图不受影响）、快照回退
 *      判据等用例（含一处 `llm_state_frame` → `llm_message_frame` 回归修复的锁定用例）。
 * 881：v1→v2 迁移 + 节点状态流 S20~S23 + 压缩消息流化 + 中止收口终态化 + 协议
 *      增量提取器逐字节回归 + tool_name 线上名投影 + MCP camelCase/载荷语义网 +
 *      extract_result 判定顺序。逐批明细见对应提交（`git log --grep=<批次/主题>`；
 *      本仓库不维护变更日志，变更历史即提交历史）。
 */
export const BASELINE = {
  // 各测试套件的计数基线（**只增不减**）。
  // 向下（删测试）必须配 `// baseline-allow <key>: <理由>`（见 `35-baseline.mjs` 元守卫，防『悄悄下调基线』）。
  // 向上：门禁实测超过即自动把数字写回本文件，不在此堆各批次变更注释——变更原因看提交历史。
  // baseline-allow rustTests: 三次已归因的下修非侵蚀——批 P3 退役 `bridge` 转写路径（`v2_bridge.test.rs` 15 条缩到 `v2_facts.test.rs` 5 条 + 11 条迁与新增，逐条归属见上方注释）、批 T-051 删 `v2_mode_defaults_to_full_and_roundtrips` 1 条（被测对象 `V2Mode` 随 ADR-051 退役）、批 T-053 删 `v2_reconcile.test.rs` 9 条（被测对象 `v2_reconcile` 双源对账随单源切换退役，判据随之消失；`v2_worker_progress.test.rs` 由「两读法对照」改写为「直接钉 WAL 读法」，条数仍 5）。
  rustTests: 1556,
  // baseline-allow ciRustTestsTotal: 批 T-051 的同一增量在 CI 口径下修一格（1425 → 1424，删 `v2_mode_defaults_to_full_and_roundtrips`），非侵蚀。
  ciRustTestsTotal: 1462,
  cliRustTests: 8,
  tauriRustTests: 0,
  // doc-link-allow D-010: 下一行是退役文件的历史叙述（T-070 已把这两个文件删掉），不是入链
  // baseline-allow vitestFiles: 批 T-070（`76324817`，ADR-063 会话呈现合流）退役 `tauri/src/schemas/conversation_line.ts` 与其 185 行 spec `tauri/src/schemas/__tests__/conversation_line.spec.ts`（整文件删除 ⇒ 测试文件 55→54）。被测对象随「对话/工作分栏退役」消失，判据随之下沉：新形态由 `e2e/cases/t24-panel-split.mjs` 与 `symbio/src/plugins/session/context/conversation_view.rs` 侧覆盖，不是丢掉覆盖。
  // baseline-allow vitestTests: 同一批 T-070：上述 spec 的用例 + `tauri/src/stores/__tests__/appearance.spec.ts` 随分栏退役收窄的那部分，合计 856→838（-18）。`appearance` 的其余行为仍由该 spec 的保留用例守着（只删了分栏相关的那一段），非侵蚀。
  vitestFiles: 54,
  vitestTests: 838,
  verifyPrograms: 16,
  verifyNegatives: 3,
  // baseline-allow e2eCases: 批 T-056 读通道禁令（e2e 不得读任何内部存储文件——`messages.json` / `v2-events.wal` / `session.json` 一并禁）下删 3 个用例：t33 熔断（存储层判据，已由 `wal.test.rs` / `active.test.rs` 覆盖）、t40（断言全为 WAL 白盒入格，可见效果已由 t29 记忆召回 / t35 技能编译覆盖）、t43（messages-json-is-rebuildable，判据「真源已换、缓存可弃」已下沉 `store/tests.rs`，且 messages.json 已停写）。45 → 42，非侵蚀：三条的判据都下沉到了 Rust 侧，不是丢掉覆盖。
  e2eCases: 51,
  verifyAsserts: 189,
  // 唯一一处**向上**的 panic 面（实测 71）：S2.7-b2 段归档新增 `WalStore::archive_before`
  // 与 `WalStore::history`，两者各要拿一次同一个 `RwLock`，沿用本模块既有的「锁当无中毒用」
  // 约定（同 `len`/`range`/`head` 的 `expect("WalStore 锁中毒")`）。两处均已按 PN-001 登记，
  // 理由同构：改成毒后恢复是行为变更，需单独 ADR。这不是放宽棘轮——两处都是既有约定的
  // 复制，不是新引入的健壮性缺口。
  panicSites: 71,
}


/**
 * 棘轮的**回填容差**：实测高出基线、差值在此格数以内只提示，超出即判红。
 *
 * 为什么要有上限：「高于基线」不是无害的——基线不跟着涨，删掉同样多的测试仍然全绿，
 * 棘轮就被削掉了同样的格数。而提示是可以被无限忽略的（日志天天有、门禁天天绿）。
 *
 * **适用边界（2026-10-10 调整）**：测试数 / 断言数这类「只增」棘轮已改为
 * **向上自动写回**（见 `ratchetOrBump`），不再靠本容差逼人回填，故本常量对它们
 * 实际不再触发红。它仍服务于 `35-baseline.mjs` 里的**其他**棘轮（前端覆盖率阈值、
 * 死码存量、core 单消费方符号数），以及 `20-frontend-static` 的覆盖率阈值提示。
 */
export const BASELINE_GRACE = 5

/**
 * 棘轮三态判定的**唯一定义**——所有棘轮（本地 / CI / verify）共用一条判据，
 * 各自决定红不红必然演化成两套口径。
 *
 * | 实测 vs 基线 | 结论 |
 * |---|---|
 * | 低于 | 红：有测试或证据被移除（棘轮存在的唯一理由） |
 * | 高出但在 `BASELINE_GRACE` 内 | 绿 + `warn`：提示回填 |
 * | 高出且超出容差 | 红：没回填的基线等于没有棘轮 |
 * | 等于 | 绿，无 note |
 *
 * `kind` / `unit` 只影响措辞，不影响判定。
 */
export function ratchetVerdict({ actual, baseline, name, kind = '通过数', unit = '测试' }) {
  if (actual < baseline) {
    return { ok: false, note: `${kind} ${actual} < 基线 ${baseline}（有${unit}被删或失败）` }
  }
  if (actual > baseline + BASELINE_GRACE) {
    return {
      ok: false,
      note: `${kind} ${actual} > 基线 ${baseline}：超出回填容差 ${BASELINE_GRACE} ⇒ 更新 BASELINE.${name}`,
    }
  }
  if (actual > baseline) return { ok: true, warn: true, note: `${kind} ${actual}（基线待更新）` }
  return { ok: true }
}

/**
 * BASELINE 的**只增**判据：当前值只要低于任一基准版本里的值，就是削判据。
 *
 * `BASELINE` 是源码常量，而它是「有测试被删就红」这条判据本身——把它改小不需要动
 * 任何测试、任何守卫代码，门禁照样全绿，且**不留痕迹**（与「测试消失」同形）。
 *
 * **基准是「触碰过该文件的最近两个版本」，不是分支根**：远历史里基线本来就低，拿它比
 * 会让每一跑都红，于是豁免被喂到失效；只比最近一版又漏掉「已经提交的那次下调」。
 * 两版足够让**下调发生的当次**变红，无论它还在工作区还是已进 HEAD。
 *
 * ⚠️ 只判 `BASELINE` 一个对象是**覆盖面**问题而非正确性问题——plan/13 批 M12 把它
 * 泛化成 {@link ratchetErosion}（多落点 + 两种方向），本函数保留为 `dir: 'floor'`
 * 的那一格，语义一字未改。
 */
export function baselineErosion(currentText, baseTexts) {
  return ratchetErosion({ dir: 'floor' }, parseBaselineCells, currentText, baseTexts, baselineWaivers(currentText))
}

/**
 * **棘轮元判据**：把「当前值不得比基准版本更松」抽成一处。
 *
 * 两种方向，都是「只许往紧的方向改」：
 * - `floor`（**只增**，下限）：当前值不得**小于**基准值。用于「测试数 / 覆盖率阈值」
 *   这类**越多越严**的判据——调低就是削判据。
 * - `ceiling`（**只减**，上限）：当前值不得**大于**基准值。用于「死码存量 / 未跨出
 *   core 的符号数」这类**越少越严**的判据——调高就是削判据。
 *
 * 两者共用这一处，是为了让「什么算削判据」只定义一次：`35-baseline.mjs` 的
 * `RATCHETS` 清单每加一条落点，不必重新决定一次红不红——那正是 M2 把
 * `ratchetVerdict` 收进单点的同一条理由。
 *
 * `cellsOf(text)` 是**提取函数**，由调用方给（各落点的形状不同：`BASELINE` 是对象
 * 字面量、`CORE_EXPORT_BASELINE` 是 `zero:one` 对、`vitest.config.ts` 是 `thresholds`
 * 嵌套块）。取不到任何格时返回空 Map，**由调用方决定**是红还是跳过——本函数不替它猜。
 *
 * ⚠️ **契约：`cellsOf` 必须返回 `Number`，不能返回字符串。** 两个方向都靠 `<` / `>`
 * 判定，而 JS 的字符串比较是**字典序**：`'9' < '10'` 为 **false**（`'9'` > `'1'`）。
 * 一个返回字符串的提取器会让 `9 → 10` 这类**下调**静默通过——方向恰好是最危险的那一侧。
 */
export function ratchetErosion({ dir = 'floor' } = {}, cellsOf, currentText, baseTexts, waivers) {
  const current = cellsOf(currentText)
  const waivedKeys = waivers ? new Set(waivers.keys()) : new Set()
  const loosened = (now, then) => (dir === 'ceiling' ? now > then : now < then)
  const worst = new Map()
  for (const { ref, text } of baseTexts) {
    for (const [key, then] of cellsOf(text)) {
      const now = current.get(key)
      if (now !== undefined && !loosened(now, then)) continue
      const hit = worst.get(key)
      if (hit && hit.from >= then) continue
      worst.set(key, { key, from: then, to: now ?? null, waived: waivedKeys.has(key), source: ref })
    }
  }
  return [...worst.values()]
}

/**
 * 已登记的基线豁免（键 → 理由）。豁免形态与 `dead-code-allow R-002: 理由` 同一条
 * 约定：**理由非空**，否则不算豁免——空理由的豁免等于没有判据。
 */
/**
 * 自动上调 `BASELINE.<key>` 的数值并重写本文件（**只增不减，不写注释**）。
 *
 * 棘轮的真实价值只在向下方向（实测掉下去 ⇒ 红，逼人查「是不是丢了测试」）。
 * 向上是预期内的正常增长：门禁检测到「实测 > 基线」就**就地写回新数**，不写任何
 * 注释——自动发生的，无需解释。这把原设计里「每次加测试都要手写一段论证再回填」
 * 的仪式整个去掉（那段论证就是被本仓规则禁止的「源码内变更史」）。
 *
 * 仅当 `value > 当前值` 才写，绝不下调（下沉由 `35-baseline.mjs` 的元守卫拦）。
 * 本进程单线程，同步读写不会与同批并发的 `35-baseline` 读产生撕裂。
 *
 * @param stage 仅本地模式传 true：写回后一并 `git add` 本文件，使上调随本次提交走
 *              （与 `autoWork` 对 fmt / 事实文件生成的处理同一条约定）。
 * @returns true 表示确实写回（数值上调）；false 表示无需改或写入失败。
 */
export function acceptBaseline(key, value, { stage = false } = {}) {
  let text
  try {
    text = fs.readFileSync(SHARED_MJS, 'utf8')
  } catch {
    return false
  }
  const re = new RegExp(`^  ${key}: (-?\\d+),$`, 'm')
  const m = re.exec(text)
  if (!m) return false
  const current = Number(m[1])
  if (!(value > current)) return false
  const next = text.replace(re, `  ${key}: ${value},`)
  try {
    fs.writeFileSync(SHARED_MJS, next, 'utf8')
  } catch {
    return false
  }
  if (stage) {
    try {
      spawnSync('git', ['add', '--', SHARED_MJS], { cwd: REPO_ROOT })
    } catch {
      /* 暂存失败不阻断门禁 */
    }
  }
  return true
}

/**
 * 棘轮判定 + 向上自动上调（网关版）。`ratchetVerdict` 保持纯函数（供单测），
 * 本函数在其上叠加「向上即写回」的副作用：
 * - 实测 > 基线：自动上调（只增，无注释），判绿；
 * - 实测 < 基线：判红（向下是唯一要守的方向，对应「测试被删」）；
 * - 相等：判绿。
 *
 * 仅当 `ctx.autoBump` 为真（本地门禁）才真的写回；CI 只亮绿灯、不落盘（CI 无法提交，
 * 且上调让它变绿正是预期）。`ctx.autoBump` 由 `gate.mjs` 设为 `!ctx.ci`。
 */
export function ratchetOrBump({ actual, baseline, name, kind = '通过数', unit = '测试', ctx }) {
  if (actual > baseline) {
    const bump = Boolean(ctx?.autoBump)
    if (bump) acceptBaseline(name, actual, { stage: true })
    return { ok: true, note: bump ? `基线自动上调至 ${actual}（无注释）` : '基线待更新（本地门禁会自动上调）' }
  }
  return ratchetVerdict({ actual, baseline, name, kind, unit })
}

export function baselineWaivers(text) {
  const out = new Map()
  for (const m of text.matchAll(/^ {0,2}\/\/ baseline-allow ([A-Za-z][A-Za-z0-9_]*): (\S[^\n]*)$/gm)) {
    out.set(m[1], m[2].trim())
  }
  return out
}

/**
 * 从 `_shared.mjs` 源码文本里取出 BASELINE 的数值格。
 *
 * 只认对象体内的 `  键: 数字,` 一种形状，且**只扫对象体**——不然注释里那句
 * 「回填 `1339 → 1341`」这类文本会被当成一格，而它描述的恰恰是历史值。
 */
export function parseBaselineCells(text) {
  const norm = text.replace(/\r\n/g, '\n')
  const start = norm.indexOf('export const BASELINE = {')
  const cells = new Map()
  if (start < 0) return cells
  const body = norm.slice(start)
  const end = body.indexOf('\n}')
  const lines = (end < 0 ? body : body.slice(0, end)).split('\n')
  for (const line of lines) {
    const m = /^ {2}([A-Za-z][A-Za-z0-9_]*): (-?\d+),$/.exec(line)
    if (m) cells.set(m[1], Number(m[2]))
  }
  return cells
}

/**
 * 解析脚本里的判据码声明（`@ns` 命名空间 / `@codes` 本脚本判的号）。
 *
 * 声明**只认行注释形态**，写在文件头之后——登记表的唯一真源是这些行，
 * `docs/ref/GATE_CODES.md` 由它们生成（`gen-gate-codes.mjs`）。
 */
export function parseGateCodeDecls(text) {
  const ns = new Map()
  const codes = new Map()
  for (const m of text.matchAll(/^ *\/\/ @ns ([A-Z]{1,4}) (\S[^\n]*)$/gm)) {
    ns.set(m[1], m[2].trim())
  }
  for (const m of text.matchAll(/^ *\/\/ @codes ((?:[A-Z]{1,4}-\d{3}\s*)+)$/gm)) {
    for (const code of m[1].trim().split(/\s+/)) codes.set(code, code.slice(0, code.indexOf('-')))
  }
  return { ns, codes }
}

/** 正则字面量可以出现在这些字符之后（其余情况下 `/` 是除号，不是正则的开头） */
const REGEX_PRECEDERS = new Set([
  '', '(', ',', '=', ':', '[', '!', '&', '|', '?', '{', '}', ';', '+', '-', '*', '%', '~', '^', '<', '>',
])

/**
 * 扫一段源码，分出两类区间：
 * - `literals`：字符串字面量的**内容**区间（不含引号）；
 * - `blanks`：应当从「代码标识符」里剔除的区间——字符串内容 **与注释**。
 *
 * 为什么只要字符串：判据码被「用到」的唯一硬形态是它出现在输出语句里；注释里的
 * 「原 E-010 已退役」讲的是历史，要求它进登记表就把叙述当成了事实——而这类叙述
 * 在头注释里成段存在，误报会直接把守卫喂成豁免。
 *
 * 为什么注释要一起剔：注释是**文档的另一处**，它跟着代码一起腐烂。按标识符统计时
 * 留下注释，等于让一个早已删除的常量继续替它那一族「作证」，把合法的改名判成假红。
 */
export function codeSpansOf(src) {
  const literals = []
  const blanks = []
  let i = 0
  let last = ''
  while (i < src.length) {
    const c = src[i]
    const d = src[i + 1]
    if (c === '/' && d === '/') {
      const nl = src.indexOf('\n', i)
      const stop = nl < 0 ? src.length : nl
      blanks.push({ start: i + 2, end: stop })
      i = stop < src.length ? stop + 1 : src.length
      continue
    }
    if (c === '/' && d === '*') {
      const end = src.indexOf('*/', i + 2)
      blanks.push({ start: i + 2, end: end < 0 ? src.length : end })
      i = end < 0 ? src.length : end + 2
      continue
    }
    if (c === '/' && REGEX_PRECEDERS.has(last)) {
      i++
      let inClass = false
      while (i < src.length && src[i] !== '\n') {
        if (src[i] === '\\') {
          i += 2
          continue
        }
        if (src[i] === '[') inClass = true
        else if (src[i] === ']') inClass = false
        else if (src[i] === '/' && !inClass) break
        i++
      }
      last = '/'
      i++
      continue
    }
    if (c === '"' || c === "'" || c === '`') {
      let j = i + 1
      while (j < src.length) {
        if (src[j] === '\\') {
          j += 2
          continue
        }
        if (src[j] === c) break
        if (c !== '`' && src[j] === '\n') break
        j++
      }
      literals.push({ start: i + 1, end: j })
      blanks.push({ start: i + 1, end: j })
      last = c
      i = j + 1
      continue
    }
    if (!/\s/.test(c)) last = c
    i++
  }
  return { literals, blanks }
}

export function stringLiteralsOf(src) {
  return codeSpansOf(src).literals.map(({ start, end }) => src.slice(start, end))
}

/**
 * 取「代码自己写下的标识符」那一份文本：字符串内容与注释整段抹成空格。
 *
 * 为什么需要：`env!("CARGO_PKG_VERSION")` 里的 `CARGO_PKG_VERSION` 是**外部约定**的
 * 名字，不是本仓声明的常量；一条讲历史改名注释里的 `ROUTE_OLD` 也只是叙述。按标识符
 * 统计时留下它们，文档里合法的 `CARGO_TARGET_DIR` 就成了假红，而早已删除的常量还能
 * 替自己那一族「作证」。判据的「这一族存在」只能由代码本体回答。
 */
export function codeTextOf(src) {
  let out = ''
  let at = 0
  for (const { start, end } of codeSpansOf(src).blanks) {
    out += src.slice(at, start) + ' '
    at = end
  }
  return out + src.slice(at)
}

export const VITEST_TIMEOUT_MS = 180_000

/** cargo 的进度噪音（刷屏且无信息量） */
export const cargoNoiseRe =
  /^(?:\s*$|.*\r$|\s*(Compiling|Checking|Downloading|Downloaded|Updating|Locking|Adding|Removing|Finished|Blocking|Waiting|Fresh|Documenting|Building)\b)/

/** 从输出里抓一个整数（第一个捕获组） */
export function grabInt(output, re) {
  const m = stripAnsi(output).match(re)
  return m ? Number(m[1]) : null
}

/** 把所有匹配的捕获组相加（`cargo test --workspace` 每个目标各打一行 `test result:`） */
export function sumInt(output, re) {
  const text = stripAnsi(output)
  let total = 0
  let found = false
  for (const m of text.matchAll(new RegExp(re.source, re.flags.includes('g') ? re.flags : `${re.flags}g`))) {
    total += Number(m[1])
    found = true
  }
  return found ? total : null
}

/**
 * `cargo test` + **通过数棘轮**（`symbio` 与 `cli` 共用）。
 *
 * 抽成共享函数而不是各写一遍：`symbio` 与 `cli` 是**两个独立 workspace**
 * （仓库根没有 `Cargo.toml`），而「跑测试并比对通过数基线」这件事必须对两者是
 * **同一套判据**——各写一份的结果是两边各自演化，最后变成两条不同的规范
 * （`check-commit-msg.mjs` 把「单条」与「一段范围」抽成同一个 `validate()` 就是为此）。
 *
 * 棘轮语义（与 `BASELINE` 头部的说明一致）：
 * - 低于基线 ⇒ **失败**（有测试被删，或有测试失败而退出码没反映出来）；
 * - 高于基线 ⇒ 通过，但打印提示要求同步基线（那是刻意要人看一眼的地方）；
 * - 解析不到通过数 ⇒ 通过但标注 —— 宁可漏报，也不因为**解析**失败把门禁变红。
 *
 * **CI 分支同样有棘轮**（2026-10-07 补上，此前是 `return { ok: true }`）：靠 `ciBaseline`
 * 这一格，口径是 `--workspace` **各行求和**，与下面的本地口径不同。调用方在 CI 模式
 * **必须**传它——不传不降级、直接判红，理由见函数体内注释：省略参数不该让守卫消失。
 *
 * **口径（本地分包时）**：取输出里**第一个** `test result: ok. N passed`——那是 lib
 * 目标的**通过数**（`cargo test -p <pkg>` 先跑 lib，再跑集成测试 / doctest）。所以基线
 * 是「通过数」**不是「用例总数」**：`ignored` 的不计入（实测 `symbio` lib 总 1299、
 * `2 ignored` ⇒ 口径数 1297）。拿「总用例数」去定基线会差掉 ignored 那几个。
 *
 * 自定义任务而非声明式命令：同一次运行既判退出码又解析通过数，
 * 避免为了拿输出再跑一遍测试。
 */
export function cargoTestRatchet(ctx, { label, cwd, args, baseline, baselineName, ciBaseline, ciBaselineName }) {
  return {
    label,
    run: async () => {
      const r = await ctx.run({ label, cmd: 'cargo', args, cwd })
      if (!r.ok) {
        const note = r.timedOut ? '超时终止' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`
        return { ok: false, note, logFile: r.logFile }
      }
      if (ctx.ci) {
        // ⚠️ **没给 `ciBaseline` ⇒ 判红**，而不是退回「只信退出码」。
        //
        // 退回是这条守卫唯一会**静默**失效的方式：调用方少写一个参数，CI 就变回
        // 只抓「测试失败」、抓不住「测试消失」——后者不留任何痕迹（本地 1339 与基线
        // 比，CI 却只是求和打印一行数字）。宁可让漏写的人当场看见红，也不要一个
        // 只亮绿灯的检查项。真要「这个 crate 在 CI 没有可比基线」，显式传 `ciBaseline: 0`，
        // 那是**声明**，不是省略。
        if (ciBaseline === undefined) {
          return {
            ok: false,
            note: 'CI 模式缺 ciBaseline ⇒ 本轮次没有棘轮（调用方少写了一个参数）',
            logFile: r.logFile,
          }
        }
        // CI 跑一次 `--workspace`：输出是**所有测试目标各行求和**，与本地分包「各取
        // 首个 result 行」不是同一个口径（见 `BASELINE.ciRustTestsTotal`）。
        const total = sumInt(r.output, /test result: ok\. (\d+) passed/)
        if (total === null) return { ok: true, note: '未能解析通过数（--workspace 全量；只信退出码）' }
        const v = ratchetOrBump({
          actual: total,
          baseline: ciBaseline,
          name: ciBaselineName ?? baselineName,
          kind: 'CI 全量',
          ctx,
        })
        if (v.ok) {
          console.log(green(`      ${total} passed（CI 全量，基线 ${ciBaseline}）${v.note ? ` · ${v.note}` : ''}`))
        } else {
          console.log(red(`      ↳ ${v.note}`))
        }
        return { ...v, logFile: r.logFile }
      }
      const passed = grabInt(r.output, /test result: ok\. (\d+) passed/)
      if (passed === null) return { ok: true, note: '未能解析通过数' }
      const v = ratchetOrBump({ actual: passed, baseline, name: baselineName, ctx })
      if (v.ok) {
        console.log(green(`      ${passed} passed（基线 ${baseline}）${v.note ? ` · ${v.note}` : ''}`))
      } else {
        console.log(red(`      ↳ ${v.note}`))
      }
      return { ...v, logFile: r.logFile }
    },
  }
}

/** vitest --coverage 表格里「All files」行的行覆盖率（第 4 列，%） */
export function coverageLinesPct(output) {
  const m = stripAnsi(output).match(
    /^All files\s*\|\s*([\d.]+)\s*\|\s*([\d.]+)\s*\|\s*([\d.]+)\s*\|\s*([\d.]+)\s*\|/m,
  )
  return m ? Number(m[4]) : null
}

/** `tauri/vitest.config.ts` 里 `thresholds.lines` 的当前取值 */
export function coverageThreshold(frontendDir) {
  const cfg = path.join(frontendDir, 'vitest.config.ts')
  if (!fs.existsSync(cfg)) return null
  const m = fs.readFileSync(cfg, 'utf8').match(/thresholds:\s*\{[^}]*?\blines:\s*(\d+)/)
  return m ? Number(m[1]) : null
}

/** 沙箱「批量删除守卫」特征（vite 清 dist/ / vitest 清 coverage/ 会触发） */
export const sandboxDeleteRe = /safe-delete|SAFE_DELETE/

/**
 * 沙箱拦截导致的「未判定」：不记失败，记 skipped。
 * 一个**必然红**的门禁比没有门禁更糟 —— 人会学会忽略它。
 */
export function blockedBySandboxDelete(output) {
  return sandboxDeleteRe.test(output)
}

export function maybeSandboxDeleteHint(output) {
  if (!sandboxDeleteRe.test(output)) return
  console.log(
    '      ⚠ 输出含「批量删除被拦」字样：这是沙箱限制，不是构建/测试失败。'.padStart(0),
  )
  console.log('        确认方法：在沙箱外跑同一条命令，或先手工清掉 dist/ 与 coverage/。')
}

/** 从 `Cargo.toml` 读 `rust-version`，补全成 x.y.z（rustup 工具链名带 patch） */
export function readMsrv(dir) {
  const toml = path.join(dir, 'Cargo.toml')
  if (!fs.existsSync(toml)) return null
  const m = fs.readFileSync(toml, 'utf8').match(/^\s*rust-version\s*=\s*"([^"]+)"/m)
  if (!m) return null
  const parts = m[1].trim().split('.')
  while (parts.length < 3) parts.push('0')
  return parts.join('.')
}

// CLI release 二进制的路径与新鲜度判定**不在这里**——统一在
// `scripts/cli-binary.mjs`（门禁与 e2e 共用的唯一真相）。曾经这里只有
// 「文件在不在」两个函数，而「在」不等于「对应当前源码」，于是过期产物被一直用下去。

// ==================== 「自动执行的工作」（不是检查项） ====================

/**
 * `git status --porcelain` → 脏路径集合（含已暂存与未暂存）。
 *
 * 只取路径，不区分状态：本模块关心的是「这个路径的内容在修复前后有没有变」。
 *
 * 用 `-z`（NUL 分隔）而非默认的行分隔：默认输出会把非 ASCII 路径**转义**成
 * `"\346\226\207.md"`（`core.quotepath=true` 是默认值），那样的串既读不了文件
 * （`contentHashes` 拿到 null），也不能直接喂给 `git add`（暂存失败）。
 * `-z` 下路径原样给出、不加引号，这两处一并消失。
 */
function dirtyPaths(repoRoot) {
  const r = spawnSync('git', ['status', '--porcelain', '-z'], { cwd: repoRoot, encoding: 'utf8' })
  const out = new Set()
  if (r.status !== 0) return out
  const tokens = r.stdout.split('\0')
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i]
    if (!t) continue
    // 形如 `XY path`；重命名/复制时**紧随其后还有一个「旧路径」token**，跳过它。
    const xy = t.slice(0, 2)
    out.add(t.slice(3))
    if (xy[0] === 'R' || xy[0] === 'C' || xy[1] === 'R' || xy[1] === 'C') i++
  }
  return out
}

/** 工作区内容的哈希（文件不存在 → null）。用于判定「修复是否真的改写了它」。 */
function contentHashes(repoRoot, paths) {
  const m = new Map()
  for (const p of paths) {
    try {
      m.set(p, createHash('sha256').update(fs.readFileSync(path.join(repoRoot, p))).digest('hex'))
    } catch {
      m.set(p, null)
    }
  }
  return m
}

/**
 * 把一件**确定性的机械工作**交给门禁自己做完，而不是判它「有没有做过」。
 *
 * ## 为什么不判「有没有做过」
 *
 * 格式化与事实文件生成是**函数**，不是判断：`fmt(code) → code'`、`gen(code) → facts`
 * 对同一份输入永远给同一个输出。把它们写成 `--check` 等于让门禁因为
 * **人忘了按一次按钮**而红——它报的不是代码有问题，是流程有问题。而修复动作
 * 完全确定、零风险，没有任何理由等人来按。
 *
 * ## 语义
 *
 * - 命令**跑成功** ⇒ 通过（`ok: true`）。产物被改写不算失败，那是它该做的事。
 * - 命令**本身报错** ⇒ 不通过（真失败：工具坏了 / 输入不可解析）。
 * - 本地：被改写的路径**当场暂存**，使修复与「本次提交」是同一份内容。
 * - CI：CI 不能提交，所以「跑完仍有差异」只能报红——那是唯一能保住不变量的信号。
 *
 * ⚠️ **「CI 报红」靠 `ctx.ci`，而 `ctx.ci` 来自 `--ci`。** 调用本原语的任务
 * （`10-backend` 的两处 fmt、`60-facts` 的生成）必须在 CI 侧被以 `--ci` 调用，
 * 否则会退化成「自动修复 + 暂存」而**静默放过漂移**——一个只亮绿灯的检查项。
 * 回归测试里有一条专门断言 `.github/workflows/ci.yml` 的对应步骤传了 `--ci`。
 *
 * ## 怎么认出「被修复改写的路径」（⚠️ 这里错过一次）
 *
 * 第一版按「修复前干净、修复后变脏」判定，**方向反了**：日常流程是
 * 「改文件 → `git add` → 提交」，所以修复前就已脏（甚至已暂存）才是**常态**，
 * 而那样会漏掉它们 ⇒ 提交里留下**未格式化**的那一版。
 *
 * 正确判据是**内容哈希**：修复前给所有脏路径记哈希，修复后重算，
 * 哈希变了就是被改写过（无论它此前是干净、已暂存、还是已有未提交改动）。
 * 只比较脏路径即可——干净路径修复后若变脏，它自然进入「修复后」这一侧。
 *
 * 这样「修复前就脏、修复没碰」的路径**不会被暂存**：门禁没有立场替人决定
 * 「那些改动该不该进本次提交」（`commit.mjs` 的模型是「先 git add 你要的，
 * 再提交索引」）。
 */
export async function autoWork(ctx, { label, cmd, args = [], cwd }) {
  const before = dirtyPaths(ctx.repoRoot)
  const beforeHash = contentHashes(ctx.repoRoot, before)

  const r = await ctx.run({ label, cmd, args, cwd })
  if (!r.ok) {
    return {
      ok: false,
      note: r.timedOut ? '超时终止' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`,
    }
  }

  const after = dirtyPaths(ctx.repoRoot)
  const afterHash = contentHashes(ctx.repoRoot, after)
  const touched = [...after].filter(
    (p) => !before.has(p) || beforeHash.get(p) !== afterHash.get(p),
  )

  if (touched.length === 0) return { ok: true }

  const shown = touched.slice(0, 5).join('、') + (touched.length > 5 ? ` 等 ${touched.length} 个` : '')
  if (ctx.ci) {
    return {
      ok: false,
      note: `已执行但有 ${touched.length} 处差异（${shown}）—— CI 无法提交，请在本地跑一次门禁（会自动修复并暂存）`,
    }
  }

  const add = spawnSync('git', ['add', '--', ...touched], { cwd: ctx.repoRoot, encoding: 'utf8' })
  if (add.status !== 0) {
    return { ok: false, note: `已修复 ${touched.length} 个文件但暂存失败：${(add.stderr || '').trim()}` }
  }
  console.log(yellow(`      ⚠ 门禁已自动修复并暂存 ${touched.length} 个文件：${shown}`))
  return { ok: true, note: `自动修复 ${touched.length} 个文件（已暂存）` }
}
