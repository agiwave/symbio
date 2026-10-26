#!/usr/bin/env node
/**
 * internal-model-audit — 「内部请求」绕过档位闸门的审计（IM-001…IM-004）
 *
 * ## 为什么需要它
 *
 * 档位闸门只有一个挂点：`LlmAdapter::generate(&FullModel, …)`。`FullModel` 是胶囊令牌
 * （与 `ClassifyOnly` / `RuleOnly` 同族，私有构造），拿得到它才受档位约束。而
 * `ModelProvider::execute_turn` 是对所有插件开放的 core trait——**直调它就绕过闸门**
 * （相应批次）。
 *
 * 绕过本身不是罪：内部请求（无工具 + `silent()` 出口 + 固定 system prompt）本来就不该
 * 走对话链路的闸门。问题在于**没人枚举这些调用点**——闸门的覆盖范围因此无法复查，
 * 第四条内部请求可以悄悄溜进来而不留痕。本守卫把「调用点清单」变成机器判据。
 *
 * ## 判定口径
 *
 * | 码 | 判据 | 为什么它需要登记 |
 * |---|---|---|
 * | IM-001 | 生产代码里 `\.execute_turn\(` 未带豁免标记 | 新增一个绕过点必须当场写理由；只写代码不写理由，闸门的边界就没人说得清 |
 * | IM-002 | 豁免标记**空理由**（`// internal-model-allow IM-001:` 后面没字） | 写了像登记、实际不算 ⇒ 不单独报出来的话，人以为已经登记过 |
 * | IM-003 | 豁免标记**悬空**（本行与下一行都没有对应调用点） | 代码搬走而标记还在 ⇒ 判据对这一处永久失效（`plugin-special-case-audit` 的 SC-005 同族） |
 * | IM-004 | 登记的形状与 [D-106](../crates/symbio-plugin-model/README.md "D-106-模型消费端口归一") 决策 2 的定义不符 | D-106 重述了「内部请求」的**形状**（tools 空切片 + `silent()` 出口 + 就地构造 system prompt + 调用方中止）；只有第一条能机械核，就核这一条——声明悄悄腐烂时门禁变红，而不是烂在文档里 |
 *
 * ## 为什么逐处豁免，不用数字基线
 *
 * 与 SC 族同一条理由：基线是源码常量，改小它不需要动测试、门禁照样全绿且不留痕迹。
 * 逐处豁免把「多一处」变成「多写一条理由」，理由不可空；标记悬空由 IM-003 兜住。
 *
 * ## 存量（2026-10-20，两处，逐处登记）
 *
 * 1. `symbio_core/adapters/provider_adapter.rs` —— 它就是闸门自己的方法体
 *    （`LlmAdapter::generate` 拿到 `&FullModel` 之后才调它）。记为 `IM-001/gate`：
 *    它**转发调用方的工具集**，这正是 IM-004 允许（且要求）它唯一拥有的形状。
 * 2. `session/context/pipeline.rs::run_compression_llm` —— 压缩摘要（内部请求）。
 *
 * 2026-10-20 之前还有 classify / compose 两处；相应批次
 * 把它们改经 `ProviderLlmAdapter`，直调点随之消失（[D-106](../crates/symbio-plugin-model/README.md "D-106")）。
 * 压缩摘要仍是唯一登记豁免（[D-106](../crates/symbio-plugin-model/README.md "D-106") 决策 2 重述了
 * 「内部请求」的形状）；
 * 本守卫不判它该不该在，只判「在，且有人说过为什么」，外加 IM-004 那一刀**形状**。
 *
 * ## 判据自身的失效形态（回归测试钉的就是这几条）
 *
 * - **扫描根没有 `.rs`** ⇒ 一条都不命中而日志照样「通过」。故直接 exit 1。
 * - **注释没剥** ⇒ 注释里提一句 `execute_turn` 就红，人只能把豁免写到满屏。
 * - **测试模块没排除** ⇒ 测试本来就按名造 provider，红线画错了地方。
 *   （两层：`*.test.rs` / `tests.rs` 文件 + 文件内的 `#[cfg(test)] mod`。）
 * - **豁免只认本行** ⇒ 「代码在上、理由在下」的写法会全红，逼出满屏多余空行。
 * - **IM-004 取实参取了别的文件的文本** ⇒ 形状判成绿的（或判错人）。实参表必须跟着
 *   每一处调用点自己存（存 `stem`），不能设模块级缓存——跑判据时缓存里只剩最后一个文件。
 * - **实参按单行切** ⇒ 实参表一换行就切出半个表。故 `callArgs` 从 `(` 起做括号配平，
 *   顶层逗号切分（回归测试 ⑭ 钉）。
 *
 * 用法：
 *   node scripts/internal-model-audit.mjs
 *   node scripts/internal-model-audit.mjs SRC_ROOT=<dir>
 *
 * 退出码：0 = 通过；1 = 发现违规（含判据失效）。
 * 平台无关（纯 Node），带回归测试（internal-model-audit.test.mjs）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns IM 内部请求模型调用（档位闸门的覆盖范围）
// @codes IM-001 IM-002 IM-003 IM-004

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, yellow } from './color.mjs'
import { blankTestModules } from './rust-scan.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

/** 扫描根：整个 crate。绕过点可能出现在任何插件，也包括 core（闸门自己就在 core）。 */
const DEFAULT_SRC = path.join('crates')

/**
 * 豁免标记：`// internal-model-allow IM-001: 理由`。
 * 可带限定 `IM-001/gate`（闸门本体——它要透传调用方的工具集，见 IM-004）。
 * 理由单独切出来判空：空理由不算登记，且要**显式报出来**（否则人会以为登记过）。
 */
const MARKER_RE = /\/\/\s*internal-model-allow\s+(IM-\d{3})(\/gate)?\s*:/
/** 调用点（剥注释与测试模块之后）。 */
const CALL_RE = /\.execute_turn\s*\(/
/** 空工具切片的规范化形状（`&[]` / `& []` 都算）。 */
const EMPTY_TOOLS = /^&\s*\[\s*\]$/

/** 递归收集 `.rs`（测试文件排除）。 */
function walk(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walk(p, out)
    else if (e.name.endsWith('.rs') && !e.name.endsWith('.test.rs') && !e.name.endsWith('tests.rs')) {
      out.push(p)
    }
  }
  return out
}

/**
 * 把注释抹成空白、**保留每一个换行**。
 *
 * 为什么不用 `rust-scan.stripComments`：它把跨行的块注释整段吃掉（连换行一起），
 * 后面的行号整体上移——而本守卫的豁免是按行号挂的，行号一错位，豁免就挂到别人身上，
 * 比不豁免更坏（日志会显示「已登记」）。
 */
function blankComments(text) {
  let out = ''
  let i = 0
  const n = text.length
  while (i < n) {
    const two = text[i + 1]
    if (text[i] === '/' && two === '/') {
      while (i < n && text[i] !== '\n') {
        out += ' '
        i++
      }
      continue
    }
    if (text[i] === '/' && two === '*') {
      out += '  '
      i += 2
      while (i < n && !(text[i] === '*' && text[i + 1] === '/')) {
        out += text[i] === '\n' ? '\n' : ' '
        i++
      }
      if (i < n) {
        out += '  '
        i += 2
      }
      continue
    }
    // 字符串字面量：里面的 `//` 与 `/*` 不是注释。形状近似即可——本判据不看字符串内容，
    // 只需要不让字符串里的注释记号把后面整段吃掉。
    if (text[i] === '"') {
      out += '"'
      i++
      while (i < n && text[i] !== '"') {
        if (text[i] === '\\' && i + 1 < n) {
          out += text[i] + text[i + 1]
          i += 2
          continue
        }
        out += text[i] === '\n' ? '\n' : text[i]
        i++
      }
      if (i < n) {
        out += '"'
        i++
      }
      continue
    }
    out += text[i]
    i++
  }
  return out
}

/**
 * 审计主体（与源根解耦，供回归测试注入夹具）。
 *
 * @returns {{violations: Array, allowed: Map, usedAllows: Set, fatal: string|null, registered: number}}
 */
export function audit(srcRoot) {
  const files = walk(srcRoot)
  if (files.length === 0) {
    return { violations: [], allowed: new Map(), usedAllows: new Set(), fatal: '扫描根下没有 .rs 文件', registered: 0 }
  }

  /** 调用点清单：`rel:line` → {text, offset, stem}（offset = `.execute_turn(` 在 stem 里的下标）。 */
  const calls = new Map()
  /** 扫描期看到的豁免标记（含空理由），`rel:line` → {code, gate, reason}。 */
  const seenAllows = new Map()
  const dangling = []

  for (const file of files) {
    const rel = path.relative(repoRoot, file).split(path.sep).join('/')
    const raw = fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n')
    const lines = raw.split('\n')

    // ① 先收标记（在**原文**上收——理由就在注释里，剥了就没了）
    lines.forEach((l, i) => {
      const m = MARKER_RE.exec(l)
      if (!m) return
      const rest = l.slice(l.indexOf(m[0]) + m[0].length).trim()
      if (!rest) {
        // IM-002：写了像登记、实际不算。必须显式报出来，否则它会退化成 IM-001
        // 而人以为自己已经登记过。
        dangling.push({
          code: 'IM-002',
          key: `${rel}:${i + 1}`,
          text:
            `豁免标记没有理由：\n        ${l.trim()}\n` +
            '        空理由不算登记（与 grep-audit / doc-link-audit 同一约定）——补一句「为什么这条内部请求可以不走闸门」',
        })
        return
      }
      seenAllows.set(`${rel}:${i + 1}`, { code: m[1], gate: m[2] === '/gate', reason: rest })
    })

    // ② 再找调用点（剥注释与测试模块，行号保留）
    const stem = blankTestModules(blankComments(raw))
    let offset = 0
    stem.split('\n').forEach((l, i) => {
      const at = l.indexOf('.execute_turn(')
      if (at < 0) {
        offset += l.length + 1
        return
      }
      // stem 跟着每一处调用点存（不能设模块级缓存：等 IM-004 跑到时，
      // 缓存里只剩**最后一个文件**的 stem，前面文件的实参会按别人的文本切）。
      calls.set(`${rel}:${i + 1}`, { text: lines[i].trim(), offset: offset + at, stem })
      offset += l.length + 1
    })
  }

  // ③ 标记生效判定：本行或**下一行**有调用点（标记写在代码上方）。
  //    「理由在代码下面」不算——人读代码时先看到代码、后看到理由，顺序正好反过来
  //    （与 SC 族 / grep-audit 的位置约定同源）。
  const allowed = new Map()
  const usedAllows = new Set()
  for (const [markerKey, meta] of seenAllows) {
    const next = nextKeyLine(markerKey)
    const hit = calls.has(next) ? next : calls.has(markerKey) ? markerKey : null
    if (hit) {
      allowed.set(hit, meta)
      usedAllows.add(hit)
    } else {
      dangling.push({
        code: 'IM-003',
        key: markerKey,
        text:
          `豁免悬空：登记了 ${meta.code}（理由：${meta.reason}），但本行与下一行都没有调用点\n` +
          '        代码搬走或判据已不适用 ⇒ 删掉这条标记；留着它等于让判据对这一处永久失效',
      })
    }
  }

  const violations = []
  for (const [key, site] of calls) {
    if (!allowed.has(key)) {
      violations.push({
        code: 'IM-001',
        key,
        text:
          `绕过档位闸门的模型调用未登记：\n        ${site.text}\n` +
          '        直调 `ModelProvider::execute_turn` 不经 `LlmAdapter::generate(&FullModel, …)`。' +
          '要么改走 `ProviderLlmAdapter::for_turn`，要么在上一行写 `// internal-model-allow IM-001: 理由`（标记写在调用点上方）',
      })
      continue
    }
    // ④ IM-004 形状判据：D-106 决策 2 定义的「内部请求」四条里，`tools` 空切片
    //    是唯一能机械核的一条（其余三条要跨函数读）。内部请求点必须是 `&[]`；
    //    闸门本体（`/gate`）必须**转发**调用方的工具集，而不是自己造一个空的。
    const args = callArgs(site.stem, site.offset)
    const tools = args[2] ?? ''
    const shapeOk = allowed.get(key).gate ? !EMPTY_TOOLS.test(tools) : EMPTY_TOOLS.test(tools)
    if (!shapeOk) {
      violations.push({
        code: 'IM-004',
        key,
        text:
          `登记的形状与 D-106 决策 2 不符（tools 实参 = ${tokens(tools)}）：\n        ${site.text}\n` +
          (allowed.get(key).gate
            ? '        闸门本体要**透传**调用方的工具集；自己造一个空的说明它不再是闸门'
            : '        内部请求必须把 tools 传空切片——不空就不是内部请求，得重新裁'),
      })
    }
  }
  violations.push(...dangling)

  return { violations, allowed, usedAllows, fatal: null, registered: calls.size }
}

/**
 * 从 `.execute_turn(` 起取出**完整实参表**（括号可跨行），按顶层逗号切分。
 *
 * 为什么必须配平而不是切一行：**豁免标记自己就可能落在方法链中间**（`provider_adapter.rs`
 * 那处就是 `self.provider` 与 `.execute_turn(` 之间夹一条注释），而实参表一旦换行写就跨出
 * 调用点所在行——按单行切会切出半个表，形状判成「空」或判成别人的实参。
 */
function callArgs(stem, offset) {
  const open = stem.indexOf('(', offset)
  if (open < 0) return []
  let depth = 0
  let i = open
  for (; i < stem.length; i++) {
    const c = stem[i]
    if (c === '(' || c === '[' || c === '{') depth++
    else if (c === ')' || c === ']' || c === '}') {
      depth--
      if (depth === 0) break
    }
  }
  const inner = stem.slice(open + 1, i)
  const args = []
  let buf = ''
  let d = 0
  for (const c of inner) {
    if (c === '(' || c === '[' || c === '{') d++
    if (c === ')' || c === ']' || c === '}') d--
    if (c === ',' && d === 0) {
      args.push(buf.trim())
      buf = ''
      continue
    }
    buf += c
  }
  if (buf.trim()) args.push(buf.trim())
  return args
}

const tokens = (s) => s.replace(/\s+/g, ' ')

/** `rel:line` ⇒ `rel:(line+1)`（下一行——标记写在代码上方）。 */
function nextKeyLine(key) {
  const at = key.lastIndexOf(':')
  const line = Number(key.slice(at + 1))
  return `${key.slice(0, at)}:${line + 1}`
}

// ── 入口 ──────────────────────────────────────────────────────────────
const arg = (name) => process.argv.find((a) => a.startsWith(`${name}=`))?.slice(name.length + 1)
const srcRoot = path.resolve(arg('SRC_ROOT') ?? path.join(repoRoot, DEFAULT_SRC))
if (!fs.existsSync(srcRoot)) {
  console.error(red(`✗ SRC_ROOT 不存在：${srcRoot}`))
  process.exit(1)
}

const { violations, usedAllows, fatal, registered } = audit(srcRoot)
if (fatal) {
  console.error(red(`✗ 内部请求模型调用审计：${fatal} ⇒ 本项失去判据`))
  process.exit(1)
}
if (violations.length > 0) {
  console.error(red(`✗ 内部请求模型调用审计：${violations.length} 处违规`))
  for (const v of violations) console.error(`  ${v.code} ${v.key}\n        ${v.text}`)
  process.exit(1)
}
console.log(
  green(
    `✓ 内部请求模型调用审计：直调 execute_turn 的 ${registered} 处全部登记过理由、形状核过` +
      `（口径：生产代码，测试文件与 #[cfg(test)] 模块不判）`,
  ),
)
if (usedAllows.size > 0) {
  console.log(yellow(`  ↳ 存量豁免 ${usedAllows.size} 处：每一处都在代码旁写明理由，新增一处就得多写一条`))
}
