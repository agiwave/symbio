#!/usr/bin/env node
/**
 * plugin-special-case-audit — 插件层的「特判」审计（SC-001…SC-005）
 *
 * ## 为什么需要它
 *
 * core 侧有六个可执行审计（形状 / 出口 / 命名 / 死代码 / 登记 / 棘轮），插件层此前**零个**。
 * 后果不是理论风险：同一轮架构复盘里，三条回流各自照出过一次——工具执行流程按工具名
 * 字面量判定（`tool_executor.rs` / `delegation.rs`）、三个协议各写各的 `max_tokens` 下限、
 * 冻结字段有定义无读方。机制面纪律锋利、实现面无闸，这就是「不要拿机制面去付实现面的账」
 * 说的那件事。
 *
 * 本守卫只覆盖**能机械化**的那两条（第三条「字段有定义无读方」需要跨文件消费方统计，
 * 形态接近 `core-export-audit`，另议）。判据都是「同一份逻辑被抄成特判」的**形态**，
 * 不是「有人这么写过」的历史。
 *
 * ## 判定口径
 *
 * | 码 | 判据 | 为什么它一定是特判 |
 * |---|---|---|
 * | SC-001 | 工具名以**字面量**出现在 `plugins/session/**`（非测试）的**比较位置** | 工具名是注册表里的事实，比较它就是「按名分流」；分流该由工具自声明的能力给出 |
 * | SC-002 | 协议层的**预算 wire 键**直接跟数字字面量 | 预算是协议约束的**取值**，唯一的家是 `protocols/profile.rs`；写死在调用点就是第二份 |
 * | SC-003 | 档案表里的**思考字段名**在协议层被当作 delta 读字段 | 整行解析与前缀提取必须认同一份名单，抄第二份的代价是前缀截断重复/吃字 |
 * | SC-004 | **供应商名**字面量出现在 `if` 式判据（`==` / `contains` / `starts_with` …） | 「这家网关要特殊处理」是数据（落档案表），不是分支；`match` 臂不判——那是别名的登记表 |
 * | SC-005 | 豁免注释**悬空**（写了 `special-case-allow` 而那一行没有命中） | 登记了而代码没了，等于这条判据对那一处永久失效（`panic-audit` 的 PN-003 同族） |
 *
 * ## 为什么不用数字基线，用**逐处理由**
 *
 * 棘轮惯例是「存量压进 `BASELINE`，只许降」。本条刻意不走那条路：基线是源码常量，
 * 改小它不需要动测试、不需要动守卫、门禁照样全绿且不留痕迹（`` 指出的弱点）。
 * 这里改成**逐处豁免**：每一处存量必须在代码旁写 `// special-case-allow SC-00X: 理由`，
 * 理由非空才算数。于是「多一处」= 「多写一条理由」，改数字绕不过去；而理由消失时
 * SC-005 会红——豁免本身也进了棘轮。
 *
 * 今天四条判据的存量都是 **0**： 把工具名判定换成了能力自声明， 把供应商差异
 * 收成档案表。也就是说这不是一份「清账清单」，是一条**从地板起**的防回流线。
 *
 * ## 判据自身的失效形态（回归测试钉的就是这几条）
 *
 * - **工具名集合读不出来** ⇒ 一条都不会命中，而日志照样「通过」。故空集合直接 exit 1。
 * - **比较位置的识别写宽**（把任意字面量都当比较）⇒ 误报一片，最后被豁免喂到失效。
 * - **注释没剥** ⇒ 注释里提一句 `agent_run` 就红，人只能把豁免写到满屏。
 * - **测试文件没排除** ⇒ 测试本来就该按名造工具，红线画错了地方。
 *
 * 用法：
 *   node scripts/plugin-special-case-audit.mjs
 *   node scripts/plugin-special-case-audit.mjs SESSION_ROOT=<dir> PROTOCOLS_ROOT=<dir>
 *
 * 退出码：0 = 通过；1 = 发现违规（含判据失效）。
 * 平台无关（纯 Node，不依赖 bash / ripgrep），带回归测试（plugin-special-case-audit.test.mjs）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns SC 插件层特判（工具名 / 供应商差异）
// @codes SC-001 SC-002 SC-003 SC-004 SC-005

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, yellow } from './color.mjs'
import { blankTestModules } from './rust-scan.mjs'
import { toolFacts } from './facts/_capability.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

/** 会话插件：工具执行流程的所在地（SC-001 的扫描根）。 */
const DEFAULT_SESSION = path.join('crates', 'symbio-plugin-session', 'src')
/** 模型协议层（SC-002 / SC-003 / SC-004 的扫描根）。 */
const DEFAULT_PROTOCOLS = path.join('crates', 'symbio-plugin-model', 'src', 'protocols')

/** 档案表文件名：SC-003 的名单来源就是它，它自己不参与扫描。 */
const PROFILE_FILE = 'profile.rs'

/**
 * 预算 wire 键。
 *
 * 这份名单**只能住在这里**：档案表存的是「每个协议取什么值」，不存「这个键叫什么」——
 * 键名是协议语法的一部分，由协议实现决定；而「哪些键算预算」是本判据的定义，归本文件。
 */
const BUDGET_KEYS = [
  'max_tokens',
  'max_output_tokens',
  'maxOutputTokens',
  'max_completion_tokens',
  'budget_tokens',
]

/**
 * 供应商名。判据只看**分支形态**（`==` / `contains` / `starts_with` …），不看 `match` 臂：
 * `protocols/mod.rs` 的别名表就是一张 `match`，那是「同一份数据换个写法」，不是特判。
 *
 * 名单不追求穷尽——它挡的是**回流**：新增一家网关时，人写 `if model.contains("新家")`
 * 的那一刻要停一下，改成落表。漏网的名字不会让判据变成假绿灯，只会让这一次回流滑过去
 * （下一轮架构复盘仍会看见）。
 */
const VENDOR_NAMES = [
  'glm', 'qwen', 'mimo', 'deepseek', 'anthropic', 'openai', 'gemini', 'claude',
  'ollama', 'vllm', 'lmstudio', 'moonshot', 'kimi', 'ernie', 'hunyuan', 'doubao',
  'zhipu', 'baichuan', 'spark', 'llama', 'mixtral', 'gpt',
]

/** 豁免标记：`// special-case-allow SC-002: 理由`（理由不可为空）。 */
const ALLOW_RE = /\/\/\s*special-case-allow\s+(SC-\d{3})\s*:\s*(\S[^\n]*)$/
/** 任意字符串字面量（含转义）。 */
const LITERAL_RE = /"((?:[^"\\]|\\.)*)"/g

const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/** 递归收集 `.rs`（测试文件与测试模块排除：测试本来就按名造工具）。 */
function walk(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walk(p, out)
    else if (e.name.endsWith('.rs') && !e.name.endsWith('.test.rs')) out.push(p)
  }
  return out
}

/**
 * 把注释抹成空白、**保留每一个换行**。
 *
 * 为什么不用 `rust-scan.stripComments`：它把跨行的 `/* … *\/` 整段吃掉（连换行一起），
 * 后面的行号整体上移。本守卫的豁免是**按行号**挂的（`// special-case-allow`），
 * 行号一错位，豁免就挂到别人身上——比不豁免更坏，因为日志会显示「已登记」。
 *
 * 字符字面量（`'"'`）必须跳过，否则一个引号字符会把后面整段代码当成字符串；
 * 生命周期（`&'a str`）**不**当字面量——按「单字符 + 收尾引号」的形状判别。
 */
function blankComments(text) {
  const out = text.split('')
  const n = text.length
  const blank = (from, to) => {
    for (let k = from; k < to && k < n; k++) if (out[k] !== '\n') out[k] = ' '
  }
  let i = 0
  while (i < n) {
    const c = text[i]
    if (c === '/' && text[i + 1] === '/') {
      let j = i
      while (j < n && text[j] !== '\n') j++
      blank(i, j)
      i = j
      continue
    }
    if (c === '/' && text[i + 1] === '*') {
      // 块注释在 Rust 里可嵌套
      let depth = 0
      let j = i
      while (j < n) {
        if (text[j] === '/' && text[j + 1] === '*') {
          depth++
          j += 2
          continue
        }
        if (text[j] === '*' && text[j + 1] === '/') {
          depth--
          j += 2
          if (depth === 0) break
          continue
        }
        j++
      }
      blank(i, j)
      i = j
      continue
    }
    if (c === '"') {
      let j = i + 1
      while (j < n) {
        if (text[j] === '\\') {
          j += 2
          continue
        }
        if (text[j] === '"') {
          j++
          break
        }
        j++
      }
      i = j
      continue
    }
    if (c === "'") {
      const m = /^'(?:\\.|[^'\\])'/.exec(text.slice(i, i + 4))
      i += m ? m[0].length : 1
      continue
    }
    i++
  }
  return out.join('')
}

/** 读一份源码：注释与内联测试模块抹成空白（**行号与原文一一对应**）。 */
function codeLines(file) {
  const raw = fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n')
  return blankTestModules(blankComments(raw)).split('\n')
}

/**
 * 该字面量是不是处在**比较位置**。
 *
 * 三种形态，各自对应一种真实写法：
 *   - `x == "agent_run"` / `x != "agent_run"`——直接按名分流；
 *   - `matches!(x, "a" | "b")`——同上，换了语法；
 *   - `x.starts_with("glm")` / `.contains(` / `.ends_with(`——按名嗅探（前缀族）。
 *
 * 刻意**不含** `=>`（match 臂）：别名表与闭集映射都写成 match 臂，那是数据表的另一种写法。
 * 刻意**不含** `unwrap_or("ask_user")` / `.get("heartbeat")`：那是缺省值与键名，
 * 不是「按名字判身份」——把它们算进来，豁免会写满整个仓（两处现成实例，见 SC-001 的注）。
 */
function inComparison(line, start, end) {
  const before = line.slice(0, start).trimEnd()
  if (/(?:==|!=)$/.test(before)) return true
  if (/(?:starts_with|ends_with|contains)\($/.test(before)) return true
  // `matches!(` 的开括号在字面量之前，且同一行内没闭合 ⇒ 字面量在模式里
  const macroAt = before.lastIndexOf('matches!')
  return macroAt >= 0 && !before.slice(macroAt).includes(')')
}

/** 档案表里的字段名名单：`profile.rs` 里所有字符串字面量（剥注释后）。 */
function profileKeys(protocolsRoot) {
  const file = path.join(protocolsRoot, PROFILE_FILE)
  if (!fs.existsSync(file)) return new Set()
  const keys = new Set()
  for (const line of codeLines(file)) {
    for (const m of line.matchAll(LITERAL_RE)) keys.add(m[1])
  }
  return keys
}

/**
 * 审计。
 *
 * `sessionRoot` / `protocolsRoot` / `toolsRoot` 可被参数覆盖（回归测试用）。**工具名与
 * 档案表名单都从代码读**，不在这里另写一份——「哪些名字是工具名」是关于整个仓库的事实，
 * 抄一份清单就是本守卫自己要消灭的那种形态。
 */
function audit(sessionRoot, protocolsRoot, toolsRoot) {
  const violations = []
  const allowed = new Map() // `${file}:${line}` → 理由
  const usedAllows = new Set()

  const toolNames = new Set(toolFacts(toolsRoot).keys())
  if (toolNames.size === 0) {
    return { violations, allowed, usedAllows, fatal: `工具名集合读不出来（capability 提取失效，根=${toolsRoot}）` }
  }

  // ── SC-001 工具名字面量出现在比较位置 ────────────────────────────────
  for (const file of walk(sessionRoot)) {
    const rel = path.relative(repoRoot, file).split(path.sep).join('/')
    codeLines(file).forEach((line, i) => {
      for (const m of line.matchAll(LITERAL_RE)) {
        if (!toolNames.has(m[1])) continue
        const start = m.index
        const end = start + m[0].length
        if (!inComparison(line, start, end)) continue
        violations.push({
          code: 'SC-001',
          key: `${rel}:${i + 1}`,
          text:
            `工具名以字面量出现在比较位置：${line.trim()}\n` +
            '        工具的身份由注册表给出，分流该读工具自声明的能力（`CapabilityMeta` 的声明位），' +
            '不是在执行流程里按名比字符串',
        })
      }
    })
  }

  // ── SC-002 预算 wire 键直接写数字 ────────────────────────────────────
  const budgetRes = BUDGET_KEYS.map((k) => ({
    key: k,
    // `"max_tokens": 8192` 与 `req["max_tokens"] = json!(8192)`
    re: new RegExp(`"${esc(k)}"\\s*:\\s*\\d|"${esc(k)}"\\]\\s*=\\s*json!\\(\\s*\\d`),
  }))
  for (const file of walk(protocolsRoot)) {
    const rel = path.relative(repoRoot, file).split(path.sep).join('/')
    codeLines(file).forEach((line, i) => {
      for (const { key, re } of budgetRes) {
        if (!re.test(line)) continue
        violations.push({
          code: 'SC-002',
          key: `${rel}:${i + 1}`,
          text:
            `预算键 \`${key}\` 直接跟数字字面量：${line.trim()}\n` +
            '        预算是协议约束的取值，唯一的家是 `protocols/profile.rs`——加一行数据，不写在调用点',
        })
      }
    })
  }

  // ── SC-003 思考字段名绕开档案表 ──────────────────────────────────────
  const keys = profileKeys(protocolsRoot)
  const keyRes = [...keys].map((k) => ({
    key: k,
    // `delta.get("x")` / `delta["x"]` / `path.is(&[… "x" …])`
    re: new RegExp(`\\.get\\(\\s*"${esc(k)}"\\s*\\)|delta\\s*\\[\\s*"${esc(k)}"\\s*\\]|path\\.is\\(&\\[[^\\]]*"${esc(k)}"`),
  }))
  for (const file of walk(protocolsRoot)) {
    if (path.basename(file) === PROFILE_FILE) continue
    const rel = path.relative(repoRoot, file).split(path.sep).join('/')
    codeLines(file).forEach((line, i) => {
      for (const { key, re } of keyRes) {
        if (!re.test(line)) continue
        violations.push({
          code: 'SC-003',
          key: `${rel}:${i + 1}`,
          text:
            `思考字段名 \`${key}\` 在协议层被直接读：${line.trim()}\n` +
            '        整行解析与前缀提取必须认同一份名单——走 `first_str` + 档案表的键，' +
            '抄第二份的代价是前缀截断重复或吃字',
        })
      }
    })
  }

  // ── SC-004 供应商名字面量做 if 式判据 ────────────────────────────────
  const vendorSet = new Set(VENDOR_NAMES)
  for (const file of walk(protocolsRoot)) {
    const rel = path.relative(repoRoot, file).split(path.sep).join('/')
    codeLines(file).forEach((line, i) => {
      for (const m of line.matchAll(LITERAL_RE)) {
        if (!vendorSet.has(m[1].toLowerCase())) continue
        const start = m.index
        const end = start + m[0].length
        if (!inComparison(line, start, end)) continue
        violations.push({
          code: 'SC-004',
          key: `${rel}:${i + 1}`,
          text:
            `供应商名出现在 if 式判据里：${line.trim()}\n` +
            '        「这家网关要特殊处理」是数据——落 `protocols/profile.rs`，不是在解析代码里开分支',
        })
      }
    })
  }

  // ── 豁免收集（SC-005 反向判悬空）───────────────────────────────────
  //
  // 位置约定与 `doc-link-audit` / `grep-audit` 同一条：标记写在**本行**（行尾）或**上一行**，
  // 只豁免那一处。挂在下一行不算——那会让「注释在代码下面」变成一种隐式作用域，
  // 而人读代码时先看到代码、后看到理由，判断顺序正好反过来。
  const hitKeys = new Set(violations.map((v) => v.key))
  const keptDangling = []
  const allFiles = [...walk(sessionRoot), ...walk(protocolsRoot)]
  for (const file of allFiles) {
    const rel = path.relative(repoRoot, file).split(path.sep).join('/')
    fs.readFileSync(file, 'utf8')
      .replace(/\r\n/g, '\n')
      .split('\n')
      .forEach((raw, i) => {
        const m = ALLOW_RE.exec(raw)
        if (!m) return
        const reason = m[2].trim()
        if (!reason) return // 空理由不算豁免（与 grep-audit / dead-code-audit 同一约定）
        const here = `${rel}:${i + 1}`
        const target = [here, `${rel}:${i}`].find((k) => hitKeys.has(k))
        if (target) {
          allowed.set(target, { code: m[1], reason })
          usedAllows.add(target)
          return
        }
        // SC-005：写了豁免、本行与上一行都没有命中 ⇒ 登记的东西已经不在了
        keptDangling.push({
          code: 'SC-005',
          key: here,
          text:
            `豁免悬空：登记了 ${m[1]}（理由：${reason}），但本行与上一行都没有对应命中\n` +
            '        代码搬走或判据已不适用 ⇒ 删掉这条豁免；留着它等于让判据对这一处永久失效',
        })
      })
  }

  const kept = []
  for (const v of violations) {
    if (allowed.has(v.key)) continue
    kept.push(v)
  }
  kept.push(...keptDangling)

  return { violations: kept, allowed, usedAllows, fatal: null }
}

// ── 入口 ──────────────────────────────────────────────────────────────
const arg = (name) => process.argv.find((a) => a.startsWith(`${name}=`))?.slice(name.length + 1)
const sessionRoot = path.resolve(arg('SESSION_ROOT') ?? path.join(repoRoot, DEFAULT_SESSION))
const protocolsRoot = path.resolve(arg('PROTOCOLS_ROOT') ?? path.join(repoRoot, DEFAULT_PROTOCOLS))
const toolsRoot = path.resolve(arg('TOOLS_ROOT') ?? repoRoot)
for (const [label, dir] of [
  ['SESSION_ROOT', sessionRoot],
  ['PROTOCOLS_ROOT', protocolsRoot],
  ['TOOLS_ROOT', toolsRoot],
]) {
  if (!fs.existsSync(dir)) {
    console.error(red(`✗ ${label} 不存在：${dir}`))
    process.exit(1)
  }
}

const { violations, allowed, usedAllows, fatal } = audit(sessionRoot, protocolsRoot, toolsRoot)
if (fatal) {
  // 判据失效与「全部通过」在日志里长得一样，宁红不静默（`panic-audit` 的同一条教训）
  console.error(red(`✗ 插件层特判审计：${fatal} ⇒ 本项失去判据`))
  process.exit(1)
}
if (violations.length > 0) {
  console.error(red(`✗ 插件层特判审计：${violations.length} 处违规`))
  for (const v of violations) console.error(`  ${v.code} ${v.key}\n        ${v.text}`)
  process.exit(1)
}
const waived = allowed.size
console.log(
  green(
    `✓ 插件层特判审计：工具名不按字面量分流、预算与思考字段名只在档案表、供应商差异不写成 if` +
      `（豁免 ${waived} 处，其中已用 ${usedAllows.size} 处）`,
  ),
)
if (waived > 0) {
  console.log(yellow(`  ↳ 存量豁免 ${waived} 处：每一处都在代码旁写明理由，新增一处就得多写一条`))
}
