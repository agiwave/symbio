#!/usr/bin/env node
/**
 * loop-purity-audit — 义务环纯度（LP-001…LP-006）
 *
 * ## 它治的病
 *
 * `DE-047` 把「什么属于环」写成五条不变量与一条判定式。不可机械判的那一半留在判定表
 * （`DE-047` §2），**可机械判的这一半**必须落成门禁——否则「环不碰世界语义」只是一句
 * 散文，下一批顺手把 gate 读数或批遍历加回 loop 时没有人说话。
 *
 * ## 六个面
 *
 * | 码 | 判什么 | 范围 |
 * |---|---|---|
 * | `LP-001` | loop 生产源码不得 import 端口**载荷类型**（只许端口名 / 事实行 / 债） | `crates/symbio-plugin-loop/src/**`（`.rs`，排除 `*.test.rs` 与 `test_support.rs`） |
 * | `LP-002` | `symbio-plugin-loop` 的 Cargo 清单不得依赖 provider / 插件 crate | `crates/symbio-plugin-loop/Cargo.toml` |
 * | `LP-003` | loop 生产源码不得出现世界语义词根（`batch` / `gate` / `llm` / `tool` / `model` / `Tool*` / `Model*` / `Turn*`）与硬编码端口行名 | 同上 |
 * | `LP-004` | 端口受理面（`plugin-api/src/port.rs`）的 trait / 类型 / 方法名不得含世界词根（`llm` / `tool` / `model` / `batch` / `gate`） | `crates/symbio-plugin-api/src/port.rs` |
 * | `LP-005` | `PortProvider` / `PortExecutor` 的签名只许通用行 / 值类型，具名世界载荷（`ToolCall*` / `LlmAdapter` / `ModelProvider` / `tool_batch`）入签名即红 | 同上 |
 * | `LP-006` | **债表禁序号键 / 阶段值**：`key` 含 `round` / `turn` / `stage` / `phase`（无条件红），或 `anchor` / `seq` / `index` / `n`（默认红，行内 `baseline-allow: <理由> D-NNN` 方可认作事实身份）；`Decl` 的 `phase` 阶段号与 `anchor` 排序数值；`eq` 的字段名含序号词，或 `eq` 的值（字面量，或解析到常量的标识符）命中阶段名集合；生产函数名匹配 `(next\|open\|close)_stage` / `next_round`；`until` 里出现阶段事实名 `turn.stage` / `EVENT_TURN_STAGE` | `loop_rows.rs` / `control_decl.rs`（键 / 字段 / eq）、整个 loop crate 生产源码（函数名） |
 *
 * 前五个面是**只许降的棘轮**：基线高于实测 ⇒ 红（收窄了却没在同批把基线下调）；
 * 实测高于基线 ⇒ 红（新增）。这是本仓既有审计（`turn-root-audit` / `core-shape-audit`）
 * 的同一口径——难点不在「数得准」，而在「数出来之后不许悄悄变宽」。
 *
 * `LP-006` **不进基线**：它让 HEAD 立刻红，而这个红是真实状态，不许用 `baseline-allow`
 * 或抬高计数洗成绿。每一处红都必须登记在 `scripts/known-red.json`（红的位置 + 它证明的
 * 架构问题 + 由哪一刀清除 + 清除条件）。**known-red 不是豁免**：命中的红仍然 `exit 1`，
 * 台账只保证它被逐条列出、且只许往下走（实测少于登记 = 已清除未销账，也是红）。
 *
 * 唯一例外是效果端口声明域里的 `turn.stage`（`EffectFace::Stage` 的效果事实名，e2e
 * `t96` / `t97` 冻结断言）：它作为**效果事实名**出现不红，但回流成环的 `until` 判据或
 * 债键即红。例外的决策号是 `D-321`，登记在 `known-red.json` 的 `exemptions` 里——
 * 没有决策号的豁免在这里是红，不是「静默跳过」。
 *
 * 序号键分两类：`round` / `turn` / `stage` / `phase` 是**无条件红**（它们就是序号）；
 * `anchor` / `seq` / `index` / `n` 默认也红，只有行内 `baseline-allow: <理由> D-NNN`
 * （理由非空、决策真实存在）能承认它在这一处是「事实身份」而不是序号。默认不许，
 * 是为了不让「轮锚换个名字」直接绿。
 *
 * ## 失效形态
 *
 * 这一族的老病是「读不出 ⇒ 没什么可判 ⇒ 通过」。故：
 * - 源目录不存在 / 一个文件都没扫到 ⇒ exit 2（不亮绿灯）；
 * - 端口受理面文件缺失 / 两个端口 trait 找不到 ⇒ exit 2（不亮绿灯）；
 * - 基线文件缺失 ⇒ exit 2；
 * - 基线里有一格指向已不存在的文件 ⇒ 红（陈旧基线，`LP-003` 的同一形态）。
 *
 * ## 用法
 *
 *   node scripts/loop-purity-audit.mjs [--root=<仓库根>] [--print-baseline] [--write-baseline]
 *
 * 退出码：0 = 通过；1 = 有红格；2 = 读不出（不亮绿灯）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成）
// @ns LP 义务环纯度（类型 / 依赖 / 世界语义词根 / 端口受理面）
// @codes LP-001 LP-002 LP-003 LP-004 LP-005 LP-006

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow } from './color.mjs'
import { loadKnownRed, reconcile, adrExists, KNOWN_RED_REL } from './known-red.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')
const BASELINE_REL = 'scripts/loop-purity-baseline.json'
const LOOP_REL = 'crates/symbio-plugin-loop'
const SRC_REL = `${LOOP_REL}/src`
const API_PORT_REL = 'crates/symbio-plugin-api/src/port.rs'

const PRINT = process.argv.includes('--print-baseline')
const WRITE = process.argv.includes('--write-baseline')

/** 端口载荷类型：环只许认识端口名（`ports`）与协议事实，不认识载荷。 */
const PAYLOAD_TYPES = [
  'ToolCallRequest',
  'ToolCallOutcome',
  'ToolCallExecutor',
  'PendingPrompt',
  'PortPayload',
  'PortReading',
  'ToolBatchExecutor',
  'ToolBatchInput',
  'ToolBatchOutcome',
  'BatchGateReadings',
  'BreakerInputs',
]

/** 世界语义词根：名字出现即「环认识世界」。`ports::TOOL` / `"tool"` 按名字判，不在此列。
 *
 * 蛇形词根（`batch` / `gate`）的边界取「非字母数字」，而不是 `\b`：`\b` 把 `_` 当词内字符，
 * 于是 `run_batch` / `read_batch_gates` 这种**正是要禁的形状**恰好漏掉；而「非字母数字」
 * 又不会把 `aggregate` 里的 `gate` 算进来（`g` 是字母）。 */
const ROOT_BOUNDARY = (word) => new RegExp(`(?<![A-Za-z0-9])${word}(?![A-Za-z0-9])`, 'gi')

const WORLD_ROOTS = [
  [ROOT_BOUNDARY('batch'), 'batch'],
  [ROOT_BOUNDARY('gates?'), 'gate'],
  [ROOT_BOUNDARY('llm'), 'llm'],
  [ROOT_BOUNDARY('tool'), 'tool'],
  [ROOT_BOUNDARY('model'), 'model'],
  [/\bBatch[A-Z]/g, 'Batch*'],
  [/\bGate[A-Z]/g, 'Gate*'],
  [/\bTool[A-Z]/g, 'Tool*'],
  [/\bModel[A-Z]/g, 'Model*'],
  [/\bTurn[A-Za-z0-9_]*/g, 'Turn*'],
  [/\bturn_[a-z0-9_]+/g, 'turn_*'],
  [/\bturn\b(?!\s*\.)/g, 'turn'],
  [/\bTURN_[A-Z0-9_]+/g, 'TURN_*'],
  [/"(?:tool|model)"/g, '端口行名'],
  // `"session"` 在 loop 里多数是日志 target，不是端口名；只有端口客户端这一个常量
  // 是真正的**端口名字面量**。并入上一条字面量正则会把日志当债（宽判据），故单列。
  [/\bconst\s+PORT:\s*&str\s*=\s*"session"/g, '端口名常量'],
]

/** 端口受理面（`plugin-api/src/port.rs`）里的世界词根（`DE-047` I-6.1 / I-6.2）。
 *
 * 端口身份是数据不是类型：`ports::MODEL` / `ports::TOOL` 这类**常量**不在此列（它们是
 * 端口名字面值），而 `ModelProvider` / `tool_execute` 这类**类型与方法**入名即红。 */
const INTERFACE_WORLD_ROOTS = new Set(['llm', 'tool', 'model', 'batch', 'gate'])

/** 端口签名里的具名世界载荷（`DE-047` I-6.2）：通用接口只许行 / 值类型。 */
const WORLD_PAYLOADS = [
  'ToolCallRequest',
  'ToolCallOutcome',
  'ToolCallExecutor',
  'PendingPrompt',
  'ModelAsk',
  'ToolBatchInput',
  'ToolBatchOutput',
  'ToolBatchExecutor',
  'BatchGateReadings',
  'LlmAdapter',
  'ModelProvider',
]

/** 名字 → 词元：`ToolCallRequest` → `tool call request`，`tool_execute` → `tool execute`。
 *
 * 按词元判而不是按子串判：`aggregate` 里的 `gate`、`PortTool` 里的 `Tool` 都不该算，
 * 而 `run_batch` 里的 `batch` 与 `ToolCallRequest` 里的 `Tool` 都该算。 */
function nameTokens(name) {
  return name
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
    .toLowerCase()
    .split('_')
    .filter(Boolean)
}

const SKIP_FILE = /^(test_support\.rs|.*\.test\.rs)$/

function walk(dir, out = []) {
  for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, ent.name)
    if (ent.isDirectory()) walk(full, out)
    else if (ent.name.endsWith('.rs') && !SKIP_FILE.test(ent.name)) out.push(full)
  }
  return out
}

/** 注释与字符串里的词不算「环认识世界」：注释由 D-016 管，文档不扫。 */
function blankComments(text) {
  let out = ''
  let i = 0
  let block = 0
  while (i < text.length) {
    const two = text.slice(i, i + 2)
    const one = text[i]
    if (block === 0 && two === '//') {
      while (i < text.length && text[i] !== '\n') {
        out += ' '
        i++
      }
      continue
    }
    if (block === 0 && two === '/*') {
      block++
      out += '  '
      i += 2
      continue
    }
    if (block > 0) {
      if (two === '*/') {
        block--
        out += '  '
        i += 2
        continue
      }
      out += text[i] === '\n' ? '\n' : ' '
      i++
      continue
    }
    out += one
    i++
  }
  return out
}

function readOrDie(file) {
  try {
    return fs.readFileSync(file, 'utf8')
  } catch (e) {
    console.error(red(`读不出 ${path.relative(ROOT, file)}：${e.message}`))
    process.exit(2)
  }
}

/** LP-001：`use` 语句里出现的载荷类型名。 */
function payloadImports(text) {
  const stripped = blankComments(text)
  const uses = stripped.match(/\buse\s+[^;]+;/gs) || []
  const joined = uses.join('\n')
  let count = 0
  for (const name of PAYLOAD_TYPES) {
    const re = new RegExp(`\\b${name}\\b`, 'g')
    count += (joined.match(re) || []).length
  }
  return count
}

/** LP-002：Cargo 清单里的 provider / 插件依赖（`symbio-plugin-api` 是机制门面，放行）。 */
function forbiddenDeps(text) {
  const found = []
  const lines = text.replace(/\r\n/g, '\n').split('\n')
  let section = ''
  for (const line of lines) {
    const sec = /^\s*\[([^\]]+)\]\s*$/.exec(line)
    if (sec) {
      section = sec[1].trim()
      continue
    }
    if (!/^(dependencies|dev-dependencies|build-dependencies|target\..*\.dependencies)$/.test(section)) {
      continue
    }
    const key = /^\s*([A-Za-z0-9_-]+)\s*=/.exec(line)
    if (!key) continue
    const name = key[1]
    if (name === 'symbio-plugin-api') continue
    if (/^symbio-provider/.test(name) || /^symbio-plugin-/.test(name)) {
      found.push(`${section}:${name}`)
    }
  }
  return found.sort()
}

/** LP-003：世界语义词根计数。 */
function worldRoots(text) {
  const stripped = blankComments(text)
  let count = 0
  for (const [re] of WORLD_ROOTS) {
    count += (stripped.match(re) || []).length
  }
  return count
}

/** LP-004：端口受理面里带世界词根的 trait / 类型 / 方法名。 */
function worldNamedContracts(text) {
  const stripped = blankComments(text)
  const names = []
  // `pub trait|struct|enum|type`：类型面。
  for (const m of stripped.matchAll(/\bpub\s+(?:trait|struct|enum|type)\s+([A-Za-z_][A-Za-z0-9_]*)/g)) {
    names.push(m[1])
  }
  // `fn`：方法面（含 trait 方法与 `impl` 快捷方法；`pub fn` 也走这一条，不重复计）。
  for (const m of stripped.matchAll(/\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g)) {
    names.push(m[1])
  }
  return names.filter((name) => nameTokens(name).some((t) => INTERFACE_WORLD_ROOTS.has(t))).length
}

/** 按花括号配对取出每个 `trait` 的块体。 */
function traitBodies(text) {
  const stripped = blankComments(text)
  const bodies = []
  const head = /\btrait\s+([A-Za-z_][A-Za-z0-9_]*)[^\n{]*\{/g
  let m
  while ((m = head.exec(stripped))) {
    let i = head.lastIndex
    let depth = 1
    while (i < stripped.length && depth > 0) {
      if (stripped[i] === '{') depth++
      else if (stripped[i] === '}') depth--
      i++
    }
    bodies.push({ name: m[1], body: stripped.slice(head.lastIndex, i - 1) })
    head.lastIndex = i
  }
  return bodies
}

/** LP-005：两个端口 trait 的签名里出现的具名世界载荷。 */
function worldPayloadSignatures(text) {
  const ports = traitBodies(text).filter((b) => b.name === 'PortProvider' || b.name === 'PortExecutor')
  if (ports.length !== 2) return null
  let count = 0
  for (const { body } of ports) {
    for (const name of WORLD_PAYLOADS) {
      count += (body.match(new RegExp(`\\b${name}\\b`, 'g')) || []).length
    }
  }
  return count
}

/** LP-006 的判定面：债表键 / eq 阶段值只在两个声明文件里；函数名与 until 判据按整个 crate 判。 */
const DEBT_SRC = new Set(['loop_rows.rs', 'control_decl.rs'])
const STAGE_FACE_SRC = new Set(['loop_rows.rs', 'control_decl.rs', 'ring.rs'])
const KEY_SEQUENCE_TOKENS = new Set(['round', 'turn', 'stage', 'phase'])
/** 身份 vs 序号的歧义词：`anchor` / `seq` / `index` / `n`。默认红；只有行内
 * `baseline-allow: <理由> D-NNN`（理由非空、决策真实存在）能承认它是「事实身份」。 */
const KEY_IDENTITY_TOKENS = new Set(['anchor', 'seq', 'index', 'n'])
const FN_SEQUENCE = /(?:^|_)(?:next|open|close)_stage(?:$|_)|(?:^|_)next_round(?:$|_)/

/** 第 `index` 个字符所在的行（1 起）。 */
function lineAt(text, index) {
  let line = 1
  for (let i = 0; i < index && i < text.length; i++) if (text[i] === '\n') line++
  return line
}

/** 按 `code|file|kind|token` 归并；`lines` 只留前 8 个位置用于报告。 */
function addHit(map, file, kind, token, line) {
  const key = `${file}|${kind}|${token}`
  const cur = map.get(key) ?? { file, kind, token, count: 0, lines: [] }
  cur.count++
  if (cur.lines.length < 8) cur.lines.push(line)
  map.set(key, cur)
}

/** 从 `openIdx` 的 `{` / `[` 起找配对的 `}` / `]`（字符串感知）。找不齐返回 null。 */
function spanBlock(text, openIdx) {
  if (openIdx < 0 || text[openIdx] !== '{' && text[openIdx] !== '[') return null
  const open = text[openIdx]
  const close = open === '{' ? '}' : ']'
  let depth = 0
  let inStr = false
  for (let i = openIdx; i < text.length; i++) {
    const ch = text[i]
    if (inStr) {
      if (ch === '\\') {
        i++
        continue
      }
      if (ch === '"') inStr = false
      continue
    }
    if (ch === '"') {
      inStr = true
      continue
    }
    if (ch === open) depth++
    else if (ch === close) {
      depth--
      if (depth === 0) return text.slice(openIdx, i + 1)
    }
  }
  return null
}

const toSnake = (name) =>
  name
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
    .toLowerCase()

/**
 * 阶段名集合：**从代码里提**，不手抄一份清单。
 *
 * 四个来源按当前形态依次回退：`impl StageName` 的 match 臂 → `enum StageName` 的变体
 * （转蛇形）→ 阶段 id 常量 `SESSION_ID_*` → 行表步 id 常量 `STEP_*`。后两项是
 * `StageName` 退役后的同一身份来源，不是另一份词表——它们只是把「八个数」从类型里
 * 挪到常量里（`D-321` 的目标形状；但把同一个名字当 `until` 判据仍是本判据要红的债）。
 *
 * 一个都没提出来 ⇒ 调用方退 2（读不出），不许当成「没有阶段值」放行。
 */
function stageNamesOf(text) {
  const stripped = blankComments(text)
  const names = new Set()
  const impl = stripped.match(/impl\s+StageName\s*\{/)
  if (impl) {
    const body = spanBlock(stripped, stripped.indexOf('{', impl.index))
    if (body) for (const m of body.matchAll(/Self::\w+\s*=>\s*"([^"]*)"/g)) names.add(m[1])
  }
  if (names.size === 0) {
    const en = stripped.match(/enum\s+StageName\s*\{/)
    if (en) {
      const body = spanBlock(stripped, stripped.indexOf('{', en.index))
      if (body) {
        for (const m of body.matchAll(/\b([A-Z][A-Za-z0-9_]*)\s*(?:=\s*\d+)?\s*,/g)) names.add(toSnake(m[1]))
      }
    }
  }
  for (const m of stripped.matchAll(/const\s+SESSION_ID_[A-Z0-9_]+:\s*&str\s*=\s*"([^"]*)"/g)) names.add(m[1])
  for (const m of stripped.matchAll(/const\s+STEP_[A-Z0-9_]+:\s*&str\s*=\s*"([^"]*)"/g)) names.add(m[1])
  return names
}

/** 生产源码里的字符串常量：`NAME -> "value"`。用于把 `Lit::Str(STEP_X)` 解析回它的名字。 */
function constStringsOf(text) {
  const out = new Map()
  for (const m of blankComments(text).matchAll(/const\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]*)"/g)) {
    out.set(m[1], m[2])
  }
  return out
}

/** 行内 `baseline-allow: <理由> D-NNN`（只对歧义身份词开放；理由非空、决策必须存在）。
 *
 * 这是**唯一**能把 `anchor` / `seq` / `index` / `n` 认成「事实身份」的通道；`round` /
 * `turn` / `stage` / `phase` 没有这条通道——它们是序号本身，不是身份。
 * 注释行本身已在 `blankComments` 之外（这里读原文）。 */
function inlineAllow(text, line) {
  const lines = text.split('\n')
  for (const idx of [line - 1, line - 2]) {
    if (idx < 0) continue
    const m = /baseline-allow:\s*(.+)$/.exec(lines[idx] ?? '')
    if (!m) continue
    const parsed = /^(.+?)\s+(D-\d{3})\s*$/.exec(m[1].trim())
    if (!parsed) return { error: `行内 baseline-allow 形状不对（要「理由 D-NNN」）：${m[0].trim()}` }
    if (!adrExists(ROOT, parsed[2])) return { error: `行内 baseline-allow 指向不存在的决策 ${parsed[2]}` }
    return { adr: parsed[2] }
  }
  return null
}

/** LP-006.1：`key: &[...]` 里的序号键 / 歧义身份词。 */
function keyHits(text, file, hits, problems) {
  const stripped = blankComments(text)
  for (const m of stripped.matchAll(/key:\s*&\[([^\]]*)\]/g)) {
    const line = lineAt(stripped, m.index)
    const toks = new Set([...m[1].matchAll(/"([^"]*)"/g)].map((x) => x[1]))
    for (const t of toks) {
      if (KEY_SEQUENCE_TOKENS.has(t)) {
        addHit(hits, file, 'key', t, line)
        continue
      }
      if (!KEY_IDENTITY_TOKENS.has(t)) continue
      const allow = inlineAllow(text, line)
      if (allow?.error) problems.push(`${file}:${line} ${allow.error}`)
      if (!allow || allow.error) addHit(hits, file, 'key', t, line)
    }
  }
}

/** LP-006.1b：`Decl` 的 `phase` 阶段号与 `anchor` 排序数值。
 *
 * 字面就是「按序号推进」：行表自己按阶段号 / 排序数排座次，环再薄也还是阶段机。
 * 只取 `Decl { ... }` 块顶层的 `phase: <n>` / `anchor: <n>`（`Pattern` 里没有这两个字段）。 */
function declFieldHits(text, file, hits) {
  const stripped = blankComments(text)
  for (const m of stripped.matchAll(/\bDecl\s*\{/g)) {
    const openIdx = stripped.indexOf('{', m.index)
    const block = spanBlock(stripped, openIdx)
    if (!block) continue
    for (const f of block.matchAll(/\b(phase|anchor)\s*:\s*([0-9]+)/g)) {
      addHit(hits, file, 'decl-field', f[1], lineAt(stripped, openIdx + f.index))
    }
  }
}

/** LP-006.2：`eq` 的字段名 / 值命中序号键或阶段名集合。返回可解析的 eq 元组数。
 *
 * 三件事各算一处：
 * - `eq` 的**字段名**是 `round` / `turn` / `stage` / `phase` ⇒ `eq-key`（`("round", Lit::Uint(0))`
 *   与 `key: &["scope","round"]` 是同一笔债的两面：环按序号开账）；
 * - 字面量值 `Lit::Str("decide")` 命中阶段名 ⇒ `stage-eq`；
 * - 常量值 `Lit::Str(STEP_DECIDE)` 解析后命中阶段名 ⇒ 同样 `stage-eq`。后一种是同一形状的
 *   改名（把名字从类型挪到常量），不解析就等于给改名开了后门。解析不到的标识符
 *   （如 `ports::TOOL`）不在本判据的判定面，跳过。 */
function eqHits(text, file, stageNames, consts, hits) {
  const stripped = blankComments(text)
  let resolvable = 0
  for (const em of stripped.matchAll(/eq:\s*&\[/g)) {
    const openIdx = stripped.indexOf('[', em.index)
    const block = spanBlock(stripped, openIdx)
    if (!block) continue
    for (const tm of block.matchAll(/\(\s*"([A-Za-z0-9_.-]+)"\s*,/g)) {
      if (KEY_SEQUENCE_TOKENS.has(tm[1])) addHit(hits, file, 'eq-key', tm[1], lineAt(stripped, openIdx + tm.index))
    }
  }
  for (const m of stripped.matchAll(
    /\(\s*"([A-Za-z0-9_.-]+)"\s*,\s*Lit::Str\(\s*(?:"([^"]*)"|([A-Za-z_][A-Za-z0-9_:]*))\s*\)\s*\)/g,
  )) {
    const value = m[2] ?? consts.get(m[3])
    if (value === undefined) continue
    resolvable++
    if (stageNames.has(value)) addHit(hits, file, 'stage-eq', value, lineAt(stripped, m.index))
  }
  return resolvable
}

/** LP-006.3：`until` 判据里的阶段事实名（`turn.stage` / `EVENT_TURN_STAGE`）。 */
function untilHits(text, file, hits) {
  const stripped = blankComments(text)
  for (const m of stripped.matchAll(/until:\s*&\s*\[/g)) {
    const openIdx = stripped.indexOf('[', m.index)
    const block = spanBlock(stripped, openIdx)
    if (!block) continue
    for (const hit of block.matchAll(/turn\.stage|EVENT_TURN_STAGE/g)) {
      addHit(hits, file, 'stage-debt', 'turn.stage', lineAt(stripped, openIdx + hit.index))
    }
  }
}

/** LP-006.4：生产函数名匹配 `(next|open|close)_stage` / `next_round`。 */
function fnHits(text, file, hits) {
  const stripped = blankComments(text)
  for (const m of stripped.matchAll(/\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g)) {
    if (FN_SEQUENCE.test(m[1])) addHit(hits, file, 'fn', m[1], lineAt(stripped, m.index))
  }
}

/** 效果域 / 读面里的 `turn.stage` 计数——只用于报告 D-321 的冻结面，不参与判定。 */
function stageFactUses(text) {
  return [...blankComments(text).matchAll(/turn\.stage|EVENT_TURN_STAGE/g)].length
}

function scan() {
  const srcDir = path.join(ROOT, SRC_REL)
  if (!fs.existsSync(srcDir)) {
    console.error(red(`没有源目录 ${SRC_REL}——读不出就不亮绿灯`))
    process.exit(2)
  }
  const files = walk(srcDir)
  if (files.length === 0) {
    console.error(red(`扫到 0 个 loop 生产源文件——读不出就不亮绿灯`))
    process.exit(2)
  }
  const lp001 = {}
  const lp003 = {}
  const lp006 = new Map()
  const lp006Problems = []
  const seen = new Set()
  let eqLiterals = 0
  let stageFactCount = 0

  // 先读一遍：常量表（`Lit::Str(STEP_X)` 要解析回名字）与阶段名集合都跨文件。
  const texts = files.map((full) => ({
    rel: path.relative(srcDir, full).split(path.sep).join('/'),
    repoRel: `${SRC_REL}/${path.relative(srcDir, full).split(path.sep).join('/')}`,
    text: readOrDie(full),
  }))
  const consts = new Map()
  const stageNames = new Set()
  for (const { rel, text } of texts) {
    for (const [k, v] of constStringsOf(text)) consts.set(k, v)
    // 阶段名集合从 `stage_events.rs` 与行表 `loop_rows.rs` 提（后者是 `StageName`
    // 退役后的身份来源）；两处都提不出来 ⇒ 读不出（退 2）。
    if (rel === 'stage_events.rs' || rel === 'loop_rows.rs') {
      for (const n of stageNamesOf(text)) stageNames.add(n)
    }
  }
  const stageSourceFound = stageNames.size > 0

  for (const { rel, repoRel, text } of texts) {
    seen.add(rel)
    const a = payloadImports(text)
    const b = worldRoots(text)
    if (a > 0) lp001[rel] = a
    if (b > 0) lp003[rel] = b
    fnHits(text, repoRel, lp006)
    if (DEBT_SRC.has(rel)) {
      keyHits(text, repoRel, lp006, lp006Problems)
      declFieldHits(text, repoRel, lp006)
      eqLiterals += eqHits(text, repoRel, stageNames, consts, lp006)
    }
    if (STAGE_FACE_SRC.has(rel)) {
      untilHits(text, repoRel, lp006)
      stageFactCount += stageFactUses(text)
    }
  }

  for (const rel of DEBT_SRC) {
    if (!seen.has(rel)) {
      console.error(red(`没有 ${SRC_REL}/${rel}——债表判定面读不出就不亮绿灯`))
      process.exit(2)
    }
  }
  if (!seen.has('ring.rs')) {
    console.error(red(`没有 ${SRC_REL}/ring.rs——环的推进面读不出就不亮绿灯`))
    process.exit(2)
  }
  if (!stageSourceFound && eqLiterals > 0) {
    console.error(
      red(
        `阶段名集合读不出（stage_events.rs / loop_rows.rs 里既无 StageName 也无 SESSION_ID_* / STEP_* 常量），` +
          `但债表里有 ${eqLiterals} 处 eq 值需要比对——读不出就不亮绿灯`,
      ),
    )
    process.exit(2)
  }

  const cargoFile = path.join(ROOT, LOOP_REL, 'Cargo.toml')
  if (!fs.existsSync(cargoFile)) {
    console.error(red(`没有 ${LOOP_REL}/Cargo.toml——读不出就不亮绿灯`))
    process.exit(2)
  }
  const lp002 = forbiddenDeps(readOrDie(cargoFile))

  const portFile = path.join(ROOT, API_PORT_REL)
  if (!fs.existsSync(portFile)) {
    console.error(red(`没有 ${API_PORT_REL}——端口受理面读不出就不亮绿灯`))
    process.exit(2)
  }
  const portText = readOrDie(portFile)
  const lp004 = worldNamedContracts(portText)
  const lp005 = worldPayloadSignatures(portText)
  if (lp005 === null) {
    console.error(red(`没有找到 \`PortProvider\` / \`PortExecutor\` 两个端口 trait——读不出就不亮绿灯`))
    process.exit(2)
  }

  return { lp001, lp002, lp003, lp004, lp005, lp006, lp006Problems, stageFactCount }
}

function loadBaseline() {
  const file = path.join(ROOT, BASELINE_REL)
  if (!fs.existsSync(file)) {
    console.error(red(`没有基线 ${BASELINE_REL}——先跑 --write-baseline 并人工核对`))
    process.exit(2)
  }
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'))
  } catch (e) {
    console.error(red(`基线读不出：${e.message}`))
    process.exit(2)
  }
}

const measured = scan()
if (PRINT || WRITE) {
  const body = JSON.stringify(
    {
      note:
        'LP-001…005 的存量棘轮（只许降）。目标：LP-001 / LP-002 / LP-004 / LP-005 归零，LP-003 随 tools/** 迁出与 turn 拆除递减。',
      lp001PayloadImports: measured.lp001,
      lp002ForbiddenDeps: measured.lp002,
      lp003WorldRoots: measured.lp003,
      lp004WorldNamedContracts: measured.lp004,
      lp005WorldPayloadSig: measured.lp005,
    },
    null,
    2,
  )
  if (WRITE) {
    fs.writeFileSync(path.join(ROOT, BASELINE_REL), body + '\n')
    console.log(green(`已写入 ${BASELINE_REL}`))
  } else {
    console.log(body)
  }
  process.exit(0)
}

const base = loadBaseline()
const failures = []

function ratchet(label, code, measuredMap, baselineMap) {
  const keys = new Set([...Object.keys(measuredMap || {}), ...Object.keys(baselineMap || {})])
  for (const key of [...keys].sort()) {
    const now = measuredMap?.[key] ?? 0
    const was = baselineMap?.[key] ?? 0
    if (now > was) failures.push(`${code} ${label} 新增：${key} ${was} → ${now}`)
    else if (now < was) failures.push(`${code} ${label} 收窄未同批下调基线：${key} ${was} → ${now}`)
  }
}

ratchet('载荷类型 import', 'LP-001', measured.lp001, base.lp001PayloadImports)
ratchet('世界语义词根', 'LP-003', measured.lp003, base.lp003WorldRoots)

const depsNow = new Set(measured.lp002)
const depsWas = new Set(base.lp002ForbiddenDeps || [])
for (const dep of depsNow) {
  if (!depsWas.has(dep)) failures.push(`LP-002 新增依赖：${dep}`)
}
for (const dep of depsWas) {
  if (!depsNow.has(dep)) failures.push(`LP-002 依赖已移除但基线未同批下调：${dep}`)
}

ratchet(
  '端口受理面世界具名',
  'LP-004',
  { [API_PORT_REL]: measured.lp004 },
  { [API_PORT_REL]: base.lp004WorldNamedContracts ?? 0 },
)
ratchet(
  '端口签名具名载荷',
  'LP-005',
  { [API_PORT_REL]: measured.lp005 },
  { [API_PORT_REL]: base.lp005WorldPayloadSig ?? 0 },
)

const total = (m) => Object.values(m).reduce((a, b) => a + b, 0)

// ── LP-006：known-red 台账 ─────────────────────────────────────────────
// **不是豁免**：命中的红仍然 exit 1。台账只回答「这一处红被承认了没有、钉在哪一刀上」。
const known = loadKnownRed(ROOT)
if (!known) {
  console.error(red(`没有可读的 ${KNOWN_RED_REL}——known-red 台账读不出就不亮绿灯`))
  process.exit(2)
}
const lp006 = reconcile({
  code: 'LP-006',
  findings: measured.lp006,
  loaded: known,
  requireExemption: true,
  root: ROOT,
})
const lp006Sites = [...measured.lp006.values()].reduce((n, f) => n + f.count, 0)
const lp006Red =
  measured.lp006.size > 0 ||
  measured.lp006Problems.length > 0 ||
  lp006.stale.length > 0 ||
  lp006.malformed.length > 0 ||
  !!lp006.exemptionProblem

const short = (file) => file.slice(file.lastIndexOf('/') + 1)
console.error(`--- LP-006 债表禁序号键 / 阶段值：${measured.lp006.size} 类红 / ${lp006Sites} 处 ---`)
if (lp006.exemption) {
  console.error(
    `  ⊘ turn.stage ×${measured.stageFactCount} 在判定面外：效果事实名按 ${lp006.exemption.adr} 冻结保留` +
      `（${lp006.exemption.what}），只有出现在 until 判据或债键里才红；` +
      `清除条件：${lp006.exemption.clearWhen}`,
  )
}
if (lp006.exemptionProblem) console.error(red(`  ✗ ${lp006.exemptionProblem}`))
for (const f of lp006.known) {
  console.error(red(`  ✗ [known-red] ${short(f.file)} ${f.kind} "${f.token}" ×${f.count}（行 ${f.lines.join(', ')}）`))
  console.error(`      问题：${f.entry.problem}`)
  console.error(`      清除刀：${f.entry.knife}`)
  console.error(`      清除条件：${f.entry.clearWhen}`)
}
for (const f of lp006.unregistered) {
  console.error(red(`  ✗ [${f.reason}] ${short(f.file)} ${f.kind} "${f.token}" ×${f.count}（行 ${f.lines.join(', ')}）`))
}
for (const f of lp006.stale) {
  console.error(red(`  ✗ [${f.reason}] ${short(f.file)} ${f.kind} "${f.token}"`))
}
for (const f of lp006.malformed) {
  console.error(red(`  ✗ [台账形状] ${f.code} ${f.file} ${f.kind} "${f.token}"：${f.reason}`))
}
for (const p of measured.lp006Problems) {
  console.error(red(`  ✗ [行内 allow] ${p}`))
}
if (!measured.lp006.size) console.error(green('  ✓ 无序号键 / 阶段值 / next-round 族函数'))

if (failures.length) {
  console.error(red(`loop-purity-audit 红：${failures.length} 条`))
  for (const f of failures) console.error(`  ✗ ${f}`)
  console.error(
    yellow(
      '收窄/移除后请在同批把基线降下来（node scripts/loop-purity-audit.mjs --write-baseline），' +
        '新增则说明为什么新增的世界语义不能归 provider。',
    ),
  )
  process.exit(1)
}

if (lp006Red) {
  console.error(red(`loop-purity-audit 红：LP-006 见上（known-red 不是豁免，红就是红）`))
  process.exit(1)
}

console.log(
  green(
    `loop-purity-audit 通过：LP-001 载荷类型 ${total(measured.lp001)} · ` +
      `LP-002 违禁依赖 ${measured.lp002.length} · LP-003 世界词根 ${total(measured.lp003)} · ` +
      `LP-004 端口具名 ${measured.lp004} · LP-005 具名载荷 ${measured.lp005} · LP-006 序号键/阶段值 0` +
      `（基线 ${total(base.lp001PayloadImports)} / ${(base.lp002ForbiddenDeps || []).length} / ` +
      `${total(base.lp003WorldRoots)} / ${base.lp004WorldNamedContracts ?? 0} / ${base.lp005WorldPayloadSig ?? 0}）`,
  ),
)
process.exit(0)
