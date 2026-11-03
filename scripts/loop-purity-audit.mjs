#!/usr/bin/env node
/**
 * loop-purity-audit — 义务环纯度（LP-001…LP-005）
 *
 * ## 它治的病
 *
 * `DE-047` 把「什么属于环」写成五条不变量与一条判定式。不可机械判的那一半留在判定表
 * （`DE-047` §2），**可机械判的这一半**必须落成门禁——否则「环不碰世界语义」只是一句
 * 散文，下一批顺手把 gate 读数或批遍历加回 loop 时没有人说话。
 *
 * ## 五个面
 *
 * | 码 | 判什么 | 范围 |
 * |---|---|---|
 * | `LP-001` | loop 生产源码不得 import 端口**载荷类型**（只许端口名 / 事实行 / 债） | `crates/symbio-plugin-loop/src/**`（`.rs`，排除 `*.test.rs` 与 `test_support.rs`） |
 * | `LP-002` | `symbio-plugin-loop` 的 Cargo 清单不得依赖 provider / 插件 crate | `crates/symbio-plugin-loop/Cargo.toml` |
 * | `LP-003` | loop 生产源码不得出现世界语义词根（`batch` / `gate` / `llm` / `tool` / `model` / `Tool*` / `Model*` / `Turn*`）与硬编码端口行名 | 同上 |
 * | `LP-004` | 端口受理面（`plugin-api/src/port.rs`）的 trait / 类型 / 方法名不得含世界词根（`llm` / `tool` / `model` / `batch` / `gate`） | `crates/symbio-plugin-api/src/port.rs` |
 * | `LP-005` | `PortProvider` / `PortExecutor` 的签名只许通用行 / 值类型，具名世界载荷（`ToolCall*` / `LlmAdapter` / `ModelProvider` / `tool_batch`）入签名即红 | 同上 |
 *
 * 五个面都是**只许降的棘轮**：基线高于实测 ⇒ 红（收窄了却没在同批把基线下调）；
 * 实测高于基线 ⇒ 红（新增）。这是本仓既有审计（`turn-root-audit` / `core-shape-audit`）
 * 的同一口径——难点不在「数得准」，而在「数出来之后不许悄悄变宽」。
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
// @codes LP-001 LP-002 LP-003 LP-004 LP-005

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow } from './color.mjs'

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
  for (const full of files) {
    const rel = path.relative(srcDir, full).split(path.sep).join('/')
    const text = readOrDie(full)
    const a = payloadImports(text)
    const b = worldRoots(text)
    if (a > 0) lp001[rel] = a
    if (b > 0) lp003[rel] = b
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

  return { lp001, lp002, lp003, lp004, lp005 }
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

console.log(
  green(
    `loop-purity-audit 通过：LP-001 载荷类型 ${total(measured.lp001)} · ` +
      `LP-002 违禁依赖 ${measured.lp002.length} · LP-003 世界词根 ${total(measured.lp003)} · ` +
      `LP-004 端口具名 ${measured.lp004} · LP-005 具名载荷 ${measured.lp005}` +
      `（基线 ${total(base.lp001PayloadImports)} / ${(base.lp002ForbiddenDeps || []).length} / ` +
      `${total(base.lp003WorldRoots)} / ${base.lp004WorldNamedContracts ?? 0} / ${base.lp005WorldPayloadSig ?? 0}）`,
  ),
)
process.exit(0)
