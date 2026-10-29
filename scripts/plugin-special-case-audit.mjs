#!/usr/bin/env node
/**
 * plugin-special-case-audit — 插件层的「特判」审计（SC-001…SC-006）
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
 * | SC-001 | 工具名以**字面量**出现在 `crates/symbio-plugin-session/src/**`（非测试）的**比较位置** | 工具名是注册表里的事实，比较它就是「按名分流」；分流该由工具自声明的能力给出 |
 * | SC-002 | 协议层的**预算 wire 键**直接跟数字字面量 | 预算是协议约束的**取值**，唯一的家是 `protocols/profile.rs`；写死在调用点就是第二份 |
 * | SC-003 | 档案表里的**思考字段名**在协议层被当作 delta 读字段 | 整行解析与前缀提取必须认同一份名单，抄第二份的代价是前缀截断重复/吃字 |
 * | SC-004 | **供应商名**字面量出现在 `if` 式判据（`==` / `contains` / `starts_with` …） | 「这家网关要特殊处理」是数据（落档案表），不是分支；`match` 臂不判——那是别名的登记表 |
 * | SC-005 | 豁免注释**悬空**（写了 `special-case-allow` 而那一行没有命中） | 登记了而代码没了，等于这条判据对那一处永久失效（`panic-audit` 的 PN-003 同族） |
 * | SC-006 | 工具名以**字面量**出现在 `core` / `kernel` / `loop` / `session` 的**非测试代码**（不限于比较位置） | 这四棵树是「加能力不该动」的架构面；能力名落进去，要么是按名分流，要么是新能力在逼架构记住它。D-212 的反向判据 |
 *
 * ## 为什么 SC-001…004 不用数字基线，用**逐处理由**
 *
 * 棘轮惯例是「存量压进 `BASELINE`，只许降」。这四条刻意不走那条路：基线是源码常量，
 * 改小它不需要动测试、不需要动守卫、门禁照样全绿且不留痕迹。这里改成**逐处豁免**：
 * 每一处存量必须在代码旁写 `// special-case-allow SC-00X: 理由`，理由非空才算数。
 * 于是「多一处」= 「多写一条理由」，改数字绕不过去；而理由消失时 SC-005 会红——
 * 豁免本身也进了棘轮。
 *
 * 今天这四条的存量都是 **0**：工具名判定早换成了能力自声明，供应商差异收成了档案表。
 * 也就是说它们不是「清账清单」，是**从地板起**的防回流线。
 *
 * ## 为什么 SC-006 反向：逐处基线，每处挂一条决策
 *
 * 四棵树里「工具名一次都不出现」在迁移期做不到——存量只有一处（会话写臂读
 * `heartbeat.include_history`，那是心跳插件的 metadata 命名空间，D-076 #5），
 * 但判据不能为它开一个「比较位置」的例外。故 SC-006 用**逐处基线**
 * （`scripts/baselines/tool-names.json`：文件 + 工具名 + 处数），并且：
 *
 * - 新出现的名字 / 位置 ⇒ 红（不在基线里就没有额度）；
 * - 基线里记了而代码里少了 ⇒ 红（基线只许降，清掉的行必须从账上划掉）；
 * - 每一条基线、每一条代码内豁免都必须挂**能打开的** `### D-NNN`——指向不存在的决策
 *   的豁免不算豁免（与 CS-002 / PTS-004 同一条资格线）。
 *
 * ## 判据自身的失效形态（回归测试钉的就是这几条）
 *
 * - **工具名集合读不出来** ⇒ 一条都不会命中，而日志照样「通过」。故空集合直接 exit 1。
 * - **比较位置的识别写宽**（把任意字面量都当比较）⇒ 误报一片，最后被豁免喂到失效。
 * - **注释没剥** ⇒ 注释里提一句 `agent_run` 就红，人只能把豁免写到满屏。
 * - **测试文件没排除** ⇒ 测试本来就该按名造工具/事件，红线画错了地方。
 * - **SC-006 基线读不出来** ⇒ 所有命中都被当成「有额度」或一律跳过。故基线缺失直接 exit 1。
 *
 * 用法：
 *   node scripts/plugin-special-case-audit.mjs
 *   node scripts/plugin-special-case-audit.mjs SESSION_ROOT=<dir> PROTOCOLS_ROOT=<dir> \
 *     CORE_ROOT=<dir> KERNEL_ROOT=<dir> LOOP_ROOT=<dir> SC006_BASELINE=<file> DOCS_ROOT=<dir> REL_ROOT=<dir>
 *
 * 退出码：0 = 通过；1 = 发现违规（含判据失效）。
 * 平台无关（纯 Node，不依赖 bash / ripgrep），带回归测试（plugin-special-case-audit.test.mjs）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns SC 插件层特判（工具名 / 供应商差异）
// @codes SC-001 SC-002 SC-003 SC-004 SC-005 SC-006

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, yellow } from './color.mjs'
import { hasDecisionAnchor as anchorExists } from './decision-anchors.mjs'
import { blankTestModules } from './rust-scan.mjs'
import { toolFacts } from './facts/_capability.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

/** 会话插件：工具执行流程的所在地（SC-001 的扫描根）。 */
const DEFAULT_SESSION = path.join('crates', 'symbio-plugin-session', 'src')
/** 模型协议层（SC-002 / SC-003 / SC-004 的扫描根）。 */
const DEFAULT_PROTOCOLS = path.join('crates', 'symbio-plugin-model', 'src', 'protocols')

/**
 * SC-006 的扫描根：**能力不该逼架构认识自己的名字**的四棵树。
 *
 * 为什么是这四棵：`notes` 夹具（EX-001）证明新增能力包只改自己与生成物；反过来，
 * 工具名一旦落到这四棵的任何一处，就说明某条通用链路开始认识某个具体能力。
 * 测试文件不在内（它本来就该按名造工具）。
 */
const MSG_ROOTS = [
  ['core', path.join('crates', 'symbio-core', 'src')],
  ['kernel', path.join('crates', 'symbio-kernel', 'src')],
  ['loop', path.join('crates', 'symbio-plugin-loop', 'src')],
  ['session', path.join('crates', 'symbio-plugin-session', 'src')],
]

/** SC-006 的逐处基线（数据，不是判据）：`文件 → 工具名 → { count, adr, reason }`。 */
const SC006_BASELINE_REL = path.join('scripts', 'baselines', 'tool-names.json')

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
    else if (e.name.endsWith('.rs') && !e.name.endsWith('.test.rs') && e.name !== 'tests.rs') {
      out.push(p)
    }
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
 * 决策条目是否存在：`### D-NNN` 是设计文档里的条目，不是独立文件。
 * 扫描口径（根目录集合、跳过目录、锚点正则）住 `decision-anchors.mjs`——本审计与
 * CS-002 / PP-005 判的是同一句话，抄一份早晚两边不一致。
 */
const decisionExists = (id, docsRoot) => anchorExists(id, repoRoot, docsRoot)

/**
 * SC-006 逐处基线：`{ "<仓库相对路径>": { "<工具名>": { count, adr, reason } } }`。
 *
 * 基线是**数据**，判据住本文件；`adr` 必须是一条能打开的 `### D-NNN`（资格由审计判，
 * 与本函数无关）。形状读不出 ⇒ `fatal`：没有处数上限的基线等于没有判据。
 */
function loadSc006Baseline(file) {
  let text
  try {
    text = fs.readFileSync(file, 'utf8')
  } catch {
    return { fatal: `SC-006 基线读不出来：${file}（没有存量账本就没法判「只许降」）` }
  }
  let data
  try {
    data = JSON.parse(text)
  } catch (e) {
    return { fatal: `SC-006 基线不是合法 JSON：${file}（${e.message}）` }
  }
  if (!data || typeof data !== 'object' || Array.isArray(data)) {
    return { fatal: `SC-006 基线的顶层必须是「文件 → 条目」的对象：${file}` }
  }
  const cells = new Map()
  for (const [rel, names] of Object.entries(data)) {
    if (!names || typeof names !== 'object' || Array.isArray(names)) {
      return { fatal: `SC-006 基线 \`${rel}\` 的值必须是「工具名 → 条目」的对象` }
    }
    for (const [tool, cell] of Object.entries(names)) {
      if (!cell || typeof cell !== 'object' || Array.isArray(cell)) {
        return { fatal: `SC-006 基线 \`${rel}\` / \`${tool}\` 的条目必须是对象` }
      }
      if (!Number.isInteger(cell.count) || cell.count < 1) {
        return { fatal: `SC-006 基线 \`${rel}\` / \`${tool}\` 的 count 必须是正整数（读不出处数 = 没有上限）` }
      }
      cells.set(`${rel}|${tool}`, {
        count: cell.count,
        adr: typeof cell.adr === 'string' ? cell.adr : '',
        reason: typeof cell.reason === 'string' ? cell.reason : '',
      })
    }
  }
  return { cells }
}

/**
 * 审计。
 *
 * 各扫描根与基线都可被参数覆盖（回归测试用）。**工具名与档案表名单都从代码读**，
 * 不在这里另写一份——「哪些名字是工具名」是关于整个仓库的事实，抄一份清单就是本守卫
 * 自己要消灭的那种形态；SC-006 的存量同理，账本里只记「哪处、几个」，名字仍由
 * `toolFacts` 给出。
 */
function audit(sessionRoot, protocolsRoot, toolsRoot, opts) {
  const { msgRoots, baselineFile, docsRoot, relRoot } = opts
  const violations = []
  const allowed = new Map() // `${code}|${file}:${line}` → 理由
  const usedAllows = new Set()

  const toolNames = new Set(toolFacts(toolsRoot).keys())
  if (toolNames.size === 0) {
    return { violations, allowed, usedAllows, fatal: `工具名集合读不出来（capability 提取失效，根=${toolsRoot}）` }
  }

  const relOf = (file) => path.relative(relRoot, file).split(path.sep).join('/')
  const lineCache = new Map()
  const linesOf = (file) => {
    if (!lineCache.has(file)) lineCache.set(file, codeLines(file))
    return lineCache.get(file)
  }

  // ── SC-001 工具名字面量出现在比较位置 ────────────────────────────────
  for (const file of walk(sessionRoot)) {
    const rel = relOf(file)
    linesOf(file).forEach((line, i) => {
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
    const rel = relOf(file)
    linesOf(file).forEach((line, i) => {
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
    const rel = relOf(file)
    linesOf(file).forEach((line, i) => {
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
    const rel = relOf(file)
    linesOf(file).forEach((line, i) => {
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

  // ── SC-006 工具名字面量落在架构面（core / kernel / loop / session）────
  //
  // 与 SC-001 的分工：SC-001 只判**比较位置**（按名分流），SC-006 判**出现**——
  // 这四棵树是「加能力不该动」的面，能力名哪怕只是被记下来（日志、默认值、配置键），
  // 也说明某条通用链路开始认识某个具体能力。测试文件与内联测试模块都不算。
  const occurrences = []
  for (const { label, dir } of msgRoots) {
    for (const file of walk(dir)) {
      const rel = relOf(file)
      linesOf(file).forEach((line, i) => {
        for (const m of line.matchAll(LITERAL_RE)) {
          if (!toolNames.has(m[1])) continue
          occurrences.push({ rel, line: i + 1, tool: m[1], where: label, text: line.trim() })
        }
      })
    }
  }

  // ── 豁免收集（SC-005 反向判悬空）───────────────────────────────────
  //
  // 位置约定与 `doc-link-audit` / `grep-audit` 同一条：标记写在**本行**（行尾）或**上一行**，
  // 只豁免那一处。挂在下一行不算——那会让「注释在代码下面」变成一种隐式作用域，
  // 而人读代码时先看到代码、后看到理由，判断顺序正好反过来。
  const keptDangling = []
  const allows = []
  const seen = new Set()
  const allFiles = [
    ...walk(sessionRoot),
    ...walk(protocolsRoot),
    ...msgRoots.flatMap((r) => walk(r.dir)),
  ]
  for (const file of allFiles) {
    if (seen.has(file)) continue
    seen.add(file)
    const rel = relOf(file)
    fs.readFileSync(file, 'utf8')
      .replace(/\r\n/g, '\n')
      .split('\n')
      .forEach((raw, i) => {
        const m = ALLOW_RE.exec(raw)
        if (!m) return
        const reason = m[2].trim()
        if (!reason) return // 空理由不算豁免（与 grep-audit / dead-code-audit 同一约定）
        allows.push({ code: m[1], reason, rel, line: i + 1 })
      })
  }

  // SC-006 的豁免在**计数之前**扣掉：基线记的是「还欠着几处」，一处被就地豁免了，
  // 账上就不该再占额度。资格线比另外四条严——理由里必须有一条能打开的 `### D-NNN`
  // （D-212：能力名进架构面，只有决策能开这个口）。
  const waivedOcc = new Set()
  let sc006Inline = 0
  for (const a of allows) {
    if (a.code !== 'SC-006') continue
    const here = `${a.rel}:${a.line}`
    const adr = /D-(\d{3}a?)/.exec(a.reason)
    if (!adr) {
      violations.push({
        code: 'SC-006',
        key: here,
        text:
          `SC-006 的豁免没有挂决策：${a.reason}\n` +
          '        能力名进架构面只有一种合法理由，即一条写下来的裁决——写成 `// special-case-allow SC-006: D-NNN 理由`',
      })
      continue
    }
    if (!decisionExists(adr[1], docsRoot)) {
      violations.push({
        code: 'SC-006',
        key: here,
        text:
          `SC-006 的豁免引用了 D-${adr[1]}，但设计文档里没有 \`### D-${adr[1]}\` 条目\n` +
          '        指向不存在的决策的豁免不算豁免（与 CS-002 / PTS-004 同一条资格线）',
      })
      continue
    }
    const idx = occurrences.findIndex(
      (o, k) =>
        !waivedOcc.has(k) && o.rel === a.rel && (o.line === a.line || o.line === a.line - 1),
    )
    if (idx < 0) {
      keptDangling.push({
        code: 'SC-005',
        key: here,
        text:
          `豁免悬空：登记了 SC-006（理由：${a.reason}），但本行与上一行都没有工具名字面量\n` +
          '        代码搬走或判据已不适用 ⇒ 删掉这条豁免；留着它等于让判据对这一处永久失效',
      })
      continue
    }
    waivedOcc.add(idx)
    sc006Inline++
    usedAllows.add(here)
  }

  // SC-001…004 的豁免按「同码 + 本行/上一行」匹配；SC-006 已在上面扣账。
  const hitsByPos = new Map() // `${code}|${rel}:${line}` → violation
  for (const v of violations) {
    if (v.code !== 'SC-006') hitsByPos.set(`${v.code}|${v.key}`, v)
  }
  for (const a of allows) {
    if (a.code === 'SC-006') continue
    const target = [`${a.rel}:${a.line}`, `${a.rel}:${a.line - 1}`]
      .map((k) => hitsByPos.get(`${a.code}|${k}`))
      .find(Boolean)
    if (!target) {
      // SC-005：写了豁免、本行与上一行都没有命中 ⇒ 登记的东西已经不在了
      keptDangling.push({
        code: 'SC-005',
        key: `${a.rel}:${a.line}`,
        text:
          `豁免悬空：登记了 ${a.code}（理由：${a.reason}），但本行与上一行都没有对应命中\n` +
          '        代码搬走或判据已不适用 ⇒ 删掉这条豁免；留着它等于让判据对那一处永久失效',
      })
      continue
    }
    allowed.set(`${target.code}|${target.key}`, { code: target.code, reason: a.reason })
    usedAllows.add(`${a.rel}:${a.line}`)
  }

  // ── SC-006 逐处基线：只许降 ────────────────────────────────────────
  const baseline = loadSc006Baseline(baselineFile)
  if (baseline.fatal) return { violations, allowed, usedAllows, fatal: baseline.fatal }

  const byKey = new Map() // `${rel}|${tool}` → 未豁免的命中
  occurrences.forEach((o, i) => {
    if (waivedOcc.has(i)) return
    const key = `${o.rel}|${o.tool}`
    if (!byKey.has(key)) byKey.set(key, [])
    byKey.get(key).push(o)
  })

  for (const [key, list] of byKey) {
    const cap = baseline.cells.get(key)?.count ?? 0
    for (const o of list.slice(cap)) {
      violations.push({
        code: 'SC-006',
        key: `${o.rel}:${o.line}`,
        text:
          `工具名 \`${o.tool}\` 出现在 ${o.where} 的非测试代码：${o.text}\n` +
          '        工具的身份归注册表，判动作读它自声明的行为（D-212）。' +
          '这四棵树是「加能力不该动」的架构面；确实需要留一处的，写进 `scripts/baselines/tool-names.json` 并挂一条能打开的决策',
      })
    }
  }
  for (const [key, cell] of baseline.cells) {
    const actual = byKey.get(key)?.length ?? 0
    if (actual < cell.count) {
      violations.push({
        code: 'SC-006',
        key,
        text:
          `SC-006 基线已过期：\`${key}\` 记了 ${cell.count} 处，代码里只剩 ${actual} 处\n` +
          '        基线只许降——把这一行改小或删掉，别让已经清掉的存量继续占额度',
      })
    }
    if (!cell.reason) {
      violations.push({
        code: 'SC-006',
        key,
        text: `SC-006 基线 \`${key}\` 没有理由——没有理由的存量等于没登记`,
      })
    }
    const adr = /D-(\d{3}a?)/.exec(cell.adr)
    if (!adr) {
      violations.push({
        code: 'SC-006',
        key,
        text: `SC-006 基线 \`${key}\` 没有挂决策——每一处存量都必须有一条能打开的 \`### D-NNN\``,
      })
    } else if (!decisionExists(adr[1], docsRoot)) {
      violations.push({
        code: 'SC-006',
        key,
        text:
          `SC-006 基线 \`${key}\` 引用了 D-${adr[1]}，但设计文档里没有 \`### D-${adr[1]}\` 条目\n` +
          '        指向不存在的决策的豁免不算豁免（与 CS-002 / PTS-004 同一条资格线）',
      })
    }
  }

  const kept = []
  for (const v of violations) {
    if (allowed.has(`${v.code}|${v.key}`)) continue
    kept.push(v)
  }
  kept.push(...keptDangling)

  return {
    violations: kept,
    allowed,
    usedAllows,
    baselineCells: baseline.cells.size,
    inlineWaived: allowed.size + sc006Inline,
    fatal: null,
  }
}

// ── 入口 ──────────────────────────────────────────────────────────────
const arg = (name) => process.argv.find((a) => a.startsWith(`${name}=`))?.slice(name.length + 1)
const relRoot = path.resolve(arg('REL_ROOT') ?? repoRoot)
const sessionRoot = path.resolve(arg('SESSION_ROOT') ?? path.join(repoRoot, DEFAULT_SESSION))
const protocolsRoot = path.resolve(arg('PROTOCOLS_ROOT') ?? path.join(repoRoot, DEFAULT_PROTOCOLS))
const toolsRoot = path.resolve(arg('TOOLS_ROOT') ?? repoRoot)
const docsRoot = arg('DOCS_ROOT') ? path.resolve(arg('DOCS_ROOT')) : null
const baselineFile = path.resolve(arg('SC006_BASELINE') ?? path.join(repoRoot, SC006_BASELINE_REL))
const msgRoots = MSG_ROOTS.map(([label, rel]) => ({
  label,
  dir: path.resolve(arg(`${label.toUpperCase()}_ROOT`) ?? path.join(repoRoot, rel)),
}))
for (const [label, dir] of [
  ['SESSION_ROOT', sessionRoot],
  ['PROTOCOLS_ROOT', protocolsRoot],
  ['TOOLS_ROOT', toolsRoot],
  ...msgRoots.map((r) => [`${r.label.toUpperCase()}_ROOT`, r.dir]),
]) {
  if (!fs.existsSync(dir)) {
    console.error(red(`✗ ${label} 不存在：${dir}`))
    process.exit(1)
  }
}
if (docsRoot && !fs.existsSync(docsRoot)) {
  console.error(red(`✗ DOCS_ROOT 不存在：${docsRoot}`))
  process.exit(1)
}

const {
  violations,
  allowed,
  usedAllows,
  baselineCells = 0,
  inlineWaived = 0,
  fatal,
} = audit(sessionRoot, protocolsRoot, toolsRoot, { msgRoots, baselineFile, docsRoot, relRoot })
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
console.log(
  green(
    '✓ 插件层特判审计：工具名不按字面量分流、架构面零工具名、预算与思考字段名只在档案表、供应商差异不写成 if' +
      `（代码内豁免 ${inlineWaived} 处，其中已用 ${usedAllows.size} 处；SC-006 存量 ${baselineCells} 处）`,
  ),
)
if (inlineWaived > 0) {
  console.log(yellow(`  ↳ 存量豁免 ${inlineWaived} 处：每一处都在基线或代码旁写明理由，新增一处就得多写一条`))
}
