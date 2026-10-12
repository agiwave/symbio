#!/usr/bin/env node
/**
 * prompt-shape-audit — 出厂 `full` 路径的 **prompt 形状基线**（判定 + 基线文件的读写）
 *
 * ## 它判什么
 *
 * 「结构化 prompt 对模型是变好还是变坏」这个问题，在有一份形状基线之前**无法回答**——
 * 连「现在送出去的是什么形状」都没有一个可核对的记录，任何「更有效」的说法都是拍板
 * （[plan/13 终点第 2 条](../docs/archive/13-质量改进总纲.md)、
 * [T-046](../docs/task/T-046.md)、[ADR-048a](../docs/adr/ADR-048a.md)）。
 * 本脚本把那份记录立起来：从**真实链路**（CLI × mock LLM）抓到的请求体里提取形状，
 * 与基线文件逐字段比，改了就红。
 *
 * | 编号 | 判什么 | 它拦的是哪一种失效 |
 * |---|---|---|
 * | PS-001 | 形状逐字段漂移（请求数 / 信封 / 消息序列与结构载体 / system 小节标签序列 / 工具面） | 装配代码改了而没人知道——ADR-048a 判「拍平是退步」，但拍平在文本上**就是**一次消息数变化 |
 * | PS-002 | 基线文件缺失或其中的 JSON 块读不出 | 基线退化成「散文里提一句」，于是没有任何东西在判 |
 * | PS-003 | 提取到的请求数为 0 | **假绿灯最坏的一种**：链路没跑通 ⇒ 空形状与空基线「一致」⇒ 报通过 |
 *
 * ## 什么进基线，什么只作观测（口径见 [ADR-057](../docs/adr/ADR-057.md)）
 *
 * **形状 = 由装配代码决定的部分**，它进判据：
 * 信封参数、消息序列（`role` + `tool_calls` 条数 + 工具结果是否带 `tool_call_id` +
 * 正文形态）、system 前缀的**小节标签序列**、工具面的**数量与名字顺序**（原序直比）。
 *
 * **字节数不进判据**，只作观测：正文规模同时由**负载**（用例里的固定文本、召回的记忆内容）
 * 决定，把它钉进判据 ⇒ 改一个用例措辞就红，而那与 prompt 装配无关。判据管「结构在不在」，
 * 规模随基线重生成留一份带日期的快照，两次快照之差就是「这次结构化改动花了多少」。
 *
 * ## 为什么形状从运行时抓，不静态读代码
 *
 * 静态读 `render_messages` / `request_view_prefix` 只能读出**打算**发什么；本基线要记的是
 * **实际**发出去什么——中间隔着投影、截断、记忆召回、协议适配器（四种 wire 形态各有一套
 * flatten）。ADR-048a 的教训正在这里：它判的是「送进模型的是消息数组还是拼平的文本」，
 * 这是一条**运行时事实**。
 *
 * ## 与既有判据的分工（一个事实只有一个 owner）
 *
 * | 谁 | 判什么 |
 * |---|---|
 * | 本脚本 | 出厂路径**送出去的形状**（PS-001…003） |
 * | `symbio_core/projection/prompt_fidelity.rs` | 网格 → prompt 的**内容完整性**（该进的没进、进的没出处） |
 * | `e2e/cases/t46-*.mjs` | 把真实请求体喂给本脚本（判据的执行方在门内，口径在本脚本） |
 *
 * 完整性与形状是两个面：fidelity 红说明「少了内容」，本脚本红说明「形态变了」。
 *
 * 用法：
 *   node scripts/prompt-shape-audit.mjs --from=<requests.json> [--check|--write]
 *   `--from` 是 mock-llm `/_requests` 的原样落盘（`[{ body: {...} }]`）；
 *   `--check`（默认）比对基线，`--write` 用抓到的形状重写基线文件。
 *
 * 退出码：0 = 一致；1 = 有漂移（PS-001）；2 = 基线读不出（PS-002）或链路没跑出请求（PS-003）。
 *
 * 与仓库约定一致：纯 Node，不依赖 bash / ripgrep。
 */

// @ns PS prompt 形状基线
// @codes PS-001 PS-002 PS-003

import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { join, dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
export const BASELINE_REL = join('docs', 'ref', 'PROMPT_SHAPE.md')
export const SCHEMA_VERSION = 1
const FENCE_RE = /```json:prompt-shape\n([\s\S]*?)\n```/

// ---------- 提取 ----------

/** 信封：只取装配代码/配置决定的这几个键，`messages` / `tools` 另有归属。 */
function envelopeOf (body) {
  const out = {}
  for (const k of ['model', 'stream', 'temperature', 'tool_choice', 'max_tokens']) {
    out[k] = body[k] === undefined ? null : body[k]
  }
  return out
}

/**
 * system 正文的**小节标签序列**。
 *
 * 口径：`【…】` 只有**出现在行首**才算小节标题——正文里也会引用别的标签
 * （「与【智能体记忆】相互独立」），那一句在行中，按标签切会把一段切成三节。
 * 首个标签之前的残留记成 `null`（出厂前缀里确实有这种无标签正文：委派判定那段）。
 */
export function sectionsOf (text) {
  const out = []
  const re = /^【([^】]+)】/gm
  let last = 0
  let lastTag = null
  let m
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) out.push({ tag: lastTag, chars: text.slice(last, m.index).length })
    lastTag = m[1]
    last = m.index
    // 标签本身并入所属小节：下一处标签或文末才收尾
    re.lastIndex = m.index + m[0].length
  }
  if (lastTag === null) {
    // 一个标签都没有：整段是一节（无标签）
    if (text.length) out.push({ tag: null, chars: text.length })
  } else {
    out.push({ tag: lastTag, chars: text.slice(last).length })
  }
  return out
}

/** 单条消息的形状：role + 结构载体，不含任何正文字节判据。 */
function messageShapeOf (m) {
  const content = m.content
  const kind =
    typeof content === 'string' ? (content.length ? 'text' : 'empty')
      : Array.isArray(content) ? `parts:${content.map((p) => p.type).join(',')}`
        : content == null ? 'absent' : 'other'
  return {
    role: m.role ?? null,
    tool_calls: Array.isArray(m.tool_calls) ? m.tool_calls.length : 0,
    tool_call_id: m.tool_call_id != null,
    content: kind,
  }
}

/**
 * 从 mock-llm 记录的原样请求体提取形状。
 *
 * @param {Array<{body: object}>} requests `/_requests` 的返回
 * @param {{scenario?: string}} [meta]
 */
export function extractShape (requests, meta = {}) {
  const seen = (requests ?? []).map((q) => q?.body ?? {})
  return {
    schema: SCHEMA_VERSION,
    scenario: meta.scenario ?? 'unnamed',
    requests: seen.map((body, i) => {
      const msgs = Array.isArray(body.messages) ? body.messages : []
      const tools = Array.isArray(body.tools) ? body.tools : []
      // 工具面记**送出的原始顺序**：顺序本身就是装配的产物（注册序 = 能力收集的广播序）。
      // 曾经在这里 `.sort()`——那不是口径，是遮丑：当时 `tools` 表是哈希表，数组顺序
      // **每起一个进程重洗一次**，比原序必然天天红。表改成保序的（`IndexMap`）之后，
      // 这条判据就把「谁先谁后」一起钉住了（ADR-057 的口径随本次一并收紧）。
      const toolNames = tools.map((t) => t?.function?.name ?? t?.name ?? '?')
      return {
        index: i,
        envelope: envelopeOf(body),
        message_shape: msgs.map(messageShapeOf),
        // 只有 system 消息带小节结构；其余消息的小节列表为空。
        system_sections: msgs.map((m) => (m.role === 'system' ? sectionsOf(stringContent(m)) : [])),
        tools: {
          count: toolNames.length,
          names: toolNames,
          schema_bytes: tools.reduce((n, t) => n + Buffer.byteLength(JSON.stringify(t)), 0),
        },
      }
    }),
  }
}

/** 观测（不进判据）：正文规模，随基线重生成留一次快照。 */
export function extractObservations (requests) {
  return (requests ?? []).map((q, i) => {
    const body = q?.body ?? {}
    const msgs = Array.isArray(body.messages) ? body.messages : []
    return {
      index: i,
      messages: msgs.length,
      message_bytes: msgs.reduce((n, m) => n + Buffer.byteLength(stringContent(m)), 0),
      request_bytes: Buffer.byteLength(JSON.stringify(body)),
      system_sections: msgs.map((m) =>
        m.role === 'system' ? sectionsOf(stringContent(m)).map((s) => s.chars) : [],
      ),
    }
  })
}

function stringContent (m) {
  const c = m?.content
  if (typeof c === 'string') return c
  if (c == null) return ''
  return JSON.stringify(c)
}

// ---------- 比对 ----------

/**
 * 逐字段比对，返回「差异」字符串数组（空数组 = 一致）。
 *
 * 只比 `shape` 里的判据列；调用方须传同一 `scenario` 的两份形状。
 */
export function diffShapes (baseline, live) {
  const diffs = []
  if (!baseline || !live) return ['基线或现场形状缺失']
  if (baseline.scenario !== live.scenario) diffs.push(`scenario：基线 ${baseline.scenario} ≠ 现场 ${live.scenario}`)
  if (baseline.requests.length !== live.requests.length) {
    diffs.push(`请求数：基线 ${baseline.requests.length} ≠ 现场 ${live.requests.length}`)
  }
  const n = Math.min(baseline.requests.length, live.requests.length)
  for (let i = 0; i < n; i++) {
    const b = baseline.requests[i]
    const l = live.requests[i]
    const tag = `req${i}`
    const eb = JSON.stringify(b.envelope)
    const el = JSON.stringify(l.envelope)
    if (eb !== el) diffs.push(`${tag} 信封：${eb} ≠ ${el}`)
    const mb = b.message_shape.map(describeMessage).join(' > ')
    const ml = l.message_shape.map(describeMessage).join(' > ')
    if (mb !== ml) diffs.push(`${tag} 消息序列：\n      基线 ${mb}\n      现场 ${ml}`)
    const sb = b.system_sections.map(sectionTags).join(' | ')
    const sl = l.system_sections.map(sectionTags).join(' | ')
    if (sb !== sl) diffs.push(`${tag} system 小节标签：\n      基线 ${sb}\n      现场 ${sl}`)
    if (b.tools.count !== l.tools.count) diffs.push(`${tag} 工具数：${b.tools.count} ≠ ${l.tools.count}`)
    if (b.tools.names.join() !== l.tools.names.join()) {
      const missing = b.tools.names.filter((x) => !l.tools.names.includes(x))
      const added = l.tools.names.filter((x) => !b.tools.names.includes(x))
      diffs.push(`${tag} 工具面：新增 [${added.join(', ')}] 移除 [${missing.join(', ')}]`)
    }
  }
  return diffs
}

function describeMessage (m) {
  const carriers = []
  if (m.tool_calls) carriers.push(`calls=${m.tool_calls}`)
  if (m.tool_call_id) carriers.push('id')
  return `${m.role}${carriers.length ? `[${carriers.join(' ')}]` : ''}${m.content === 'empty' ? '(空)' : ''}`
}

function sectionTags (list) {
  return list.map((s) => `【${s.tag ?? '无标签'}】`).join('')
}

// ---------- 基线文件 ----------

/**
 * 渲染基线文件（生成物，不手改）。
 *
 * 判据列 = ```json:prompt-shape``` 块，由 `e2e/cases/t46-*.mjs` 每次门禁复算；
 * 散文表是同一份 JSON 的**投影**，规模列是重生成当时的快照（不进判据，故标明日期）。
 */
export function renderBaselineDoc (shape, observations, { capturedAt = '' } = {}) {
  const rows = shape.requests.map((r, i) => {
    const o = observations[i] ?? {}
    return `| req${i} | ${r.message_shape.length} | ${r.message_shape.map(describeMessage).join(' → ')} | ${r.system_sections.map(sectionTags).join(' ') || '—'} | ${r.tools.count} | ${o.message_bytes ?? '—'} | ${r.tools.schema_bytes} | ${o.request_bytes ?? '—'} |`
  })
  return `# Prompt 形状基线（出厂 \`full\` 路径）

> ⚠️ **生成物，不手改。** 唯一写者是 \`node e2e/cases/t46-prompt-shape-baseline.mjs\`
> 带 \`PROMPT_SHAPE_UPDATE=1\`；判据由那条用例在门禁里每次复算（改了装配就红），
> 口径与「什么进基线、什么只作观测」见 [ADR-057](../adr/ADR-057.md)，
> 提取实现在 \`scripts/prompt-shape-audit.mjs\`。

场景 \`${shape.scenario}\` · 重生成于 ${capturedAt || '（未标时间）'} · 请求数 ${shape.requests.length}

## 判据列（逐字段比，PS-001）

| 请求 | 消息数 | 消息序列（role + 结构载体） | system 小节标签序列 | 工具数 |
|---|---|---|---|---|
${shape.requests.map((r, i) => `| req${i} | ${r.message_shape.length} | ${r.message_shape.map(describeMessage).join(' → ')} | ${r.system_sections.map(sectionTags).join(' ') || '—'} | ${r.tools.count} |`).join('\n')}

## 规模观测（**不进判据**：同一列里既有装配代码的字，也有负载的字）

正文规模同时被**用例文本**与**召回内容**决定，钉进判据会让「改用例措辞」变红，
而那只与装配无关。下表是重生成当时的一次快照，两次快照之差 = 那次结构改动的代价。

| 请求 | 消息数 | 消息序列 | system 小节标签 | 工具数 | 正文字节 | 工具 schema 字节 | 请求体总字节 |
|---|---|---|---|---|---|---|---|
${rows.join('\n')}

## 基线（判据本体）

\`\`\`json:prompt-shape
${JSON.stringify(shape, null, 2)}
\`\`\`
`
}

/** 从基线文件里取出判据 JSON；读不出即 PS-002。 */
export function parseBaselineDoc (md) {
  const m = FENCE_RE.exec(md)
  if (!m) return null
  try {
    const parsed = JSON.parse(m[1])
    if (!parsed || typeof parsed !== 'object' || !Array.isArray(parsed.requests)) return null
    return parsed
  } catch {
    return null
  }
}

// ---------- CLI（人工复核 / 用例调用） ----------

const argv = process.argv.slice(2)
const argOf = (name, dflt) => {
  const i = argv.indexOf(`--${name}`)
  if (i >= 0 && argv[i + 1] && !argv[i + 1].startsWith('--')) return argv[i + 1]
  const eq = argv.find((a) => a.startsWith(`--${name}=`))
  return eq ? eq.slice(name.length + 3) : dflt
}

const invokedAs = process.argv[1] && resolve(process.argv[1])
if (invokedAs === fileURLToPath(import.meta.url)) {
  main()
}

function main () {
  const from = argOf('from')
  const write = argv.includes('--write')
  const baselinePath = resolve(REPO_ROOT, argOf('baseline', BASELINE_REL))

  if (!from) {
    console.error('用法：node scripts/prompt-shape-audit.mjs --from=<requests.json> [--check|--write]')
    console.error('（判据的日常执行方是 `e2e/cases/t46-prompt-shape-baseline.mjs`，本 CLI 供人工复核。）')
    process.exit(2)
  }
  let requests
  try {
    requests = JSON.parse(readFileSync(from, 'utf8'))
  } catch (e) {
    console.error(red('[ERROR] PS-003') + ` 现场请求体读不出：${from}（${e.message}）`)
    process.exit(2)
  }
  if (!Array.isArray(requests) || requests.length === 0) {
    console.error(
      red('[ERROR] PS-003') +
        ` 提取到的请求数为 0 ⇒ 链路没跑通，此时「与基线一致」是假绿灯（空比空必然相等）。`,
    )
    process.exit(2)
  }

  const scenario = argOf('scenario', 'unnamed')
  const shape = extractShape(requests, { scenario })
  const observations = extractObservations(requests)

  if (write) {
    const doc = renderBaselineDoc(shape, observations, { capturedAt: new Date().toISOString().slice(0, 10) })
    writeFileSync(baselinePath, doc)
    console.log(`${green('[OK]')} PS 基线重生成：${BASELINE_REL}（${shape.requests.length} 个请求）`)
    process.exit(0)
  }

  if (!existsSync(baselinePath)) {
    console.error(red('[ERROR] PS-002') + ` 基线文件不存在：${BASELINE_REL}`)
    process.exit(2)
  }
  const baseline = parseBaselineDoc(readFileSync(baselinePath, 'utf8'))
  if (!baseline) {
    console.error(red('[ERROR] PS-002') + ` 基线文件里的 \`\`\`json:prompt-shape\`\`\` 块读不出：${BASELINE_REL}`)
    process.exit(2)
  }
  const diffs = diffShapes(baseline, shape)
  if (diffs.length) {
    console.error(red(`[FAIL] PS-001`) + ` prompt 形状与基线不一致（${diffs.length} 处）：`)
    for (const d of diffs) console.error(`  ${d}`)
    console.error(
      `\n${yellow('提示')}：形状确实**该**变 ⇒ 跑 \`PROMPT_SHAPE_UPDATE=1 node e2e/cases/t46-prompt-shape-baseline.mjs\` 重生成，` +
        `\n      并在改动的那条 ADR / 提交信息里说清「结构上多了什么、代价是多少」。\n` +
        `      规模列不进判据的原因与口径见 ${dim('docs/adr/ADR-057.md')}。`,
    )
    process.exit(1)
  }
  console.log(`${green('[OK]')} PS-001 ${shape.requests.length} 个请求的形状与基线逐字段一致`)
  console.log(`${green('[OK]')} PS-002 基线可读：${BASELINE_REL}`)
  console.log(`${green('[OK]')} PS-003 现场请求数非零（链路真的跑通了）`)
  process.exit(0)
}
