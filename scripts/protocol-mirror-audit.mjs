#!/usr/bin/env node
/**
 * protocol-mirror-audit — 跨栈协议常量的**镜像一致性**守卫
 *
 * ## 它守的是什么（三组）
 *
 * **A. 常量镜像**（后端与前端必须**逐字相等**）
 *   前端持有的 `VDFS_*` 常量是后端协议词的**副本**——它拿这些词拼地址、认目录、
 *   选渲染器、判状态。副本漂移不会有任何测试变红，只会在运行期表现为
 *   「消息读不到 / 渲染器选错 / 状态判反」，所以要有专门的检查。
 *
 *   本组**自动发现**：扫后端两个常量源 + 前端 `schemas/vdfs.ts`，取同名交集逐条
 *   比对。新增一个常量即自动进入守卫——**不需要改本脚本**。此前是手工登记 3 条，
 *   剩下 26 条无人看守（`docs/design/architecture-health-check-2026-09.md` F-5）。
 *   名字不同的镜像登记在 `ALIASES`；前端自持（后端无对应）的常量必须登记在
 *   `LOCAL_ONLY` 并写明理由——**"没登记"会报错**，所以不会有静默的漏网。
 *
 * **B. 缺席检查**（前端**不得再出现**）
 *   会话的两个地址段（挂载段 `session`、转写段 `消息`）已改为**运行期发现**
 *   （见 `tauri/src/services/vdfsScheme.ts`），前端不再持有它们。这两条保证
 *   它们不会悄悄回来——回来就是第二份真相。
 *
 * **C. 闭集词表**（取值**集合**必须相等）
 *   后端 `#[serde(rename_all = "snake_case")]` 的**闭集枚举**是跨栈契约：前端按
 *   这些词分发渲染与业务规则。枚举加一个变体而前端词表没跟，那条变更会被前端
 *   当成「未知词」静默丢弃——`aborted` 曾因此让中止后的 Turn 显示成已完成、
 *   重试入口不出现。
 *
 *   比对的是**取值集合**，不是顺序（顺序对前端无意义，故不比）。
 *   只查 `rename_all = "snake_case"` 的枚举：少了这个声明，serde 会用变体名原名
 *   （`ToolCall`），下面那套转换就会「看起来正确」而实际全错——所以声明本身也是
 *   被检查项。
 *
 * ## 它**不**声称什么
 *
 * 判定基于正则读源码，不是 AST：注释掉的常量同样会命中（缺席检查因此偏严，
 * 这符合它的意图——连注释里都不该教人写回去）。
 *
 * ## 用法
 *
 *   node scripts/protocol-mirror-audit.mjs                # 审计本仓库
 *   node scripts/protocol-mirror-audit.mjs --repo=<dir>   # 换仓库根（回归测试用）
 *
 * 退出码：0 = 全部通过；1 = 存在不一致 / 缺失 / 不应出现的常量。
 */

import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRepo = path.resolve(scriptDir, '..')

function argValue(name) {
  const hit = process.argv.find((a) => a.startsWith(`--${name}=`))
  return hit ? hit.slice(name.length + 3) : null
}

const REPO = argValue('repo') ? path.resolve(argValue('repo')) : defaultRepo

const NO_COLOR = process.env.NO_COLOR === '1'
const red = (s) => (NO_COLOR ? s : `[31m${s}[0m`)
const green = (s) => (NO_COLOR ? s : `[32m${s}[0m`)
const yellow = (s) => (NO_COLOR ? s : `[33m${s}[0m`)
const dim = (s) => (NO_COLOR ? s : `[2m${s}[0m`)

const OK = '✓'
const BAD = '✗'

// ==================== A. 常量镜像 ====================

/** 后端常量源：自动发现其中 `VDFS_*` 前缀的 `&str` 常量 */
const RUST_CONST_FILES = [
  'symbio/src/symbio_core/vdfs_provider.rs',
  'symbio/src/plugins/vdfs/protocol.rs',
]

/** 前端常量源 */
const TS_CONST_FILES = ['tauri/src/schemas/vdfs.ts']

/**
 * 名字不同、但确实是同一份协议词的镜像对。
 *
 * 为什么会有别名：前端有时要在名字里区分**用途**——`VDFS_ROOT_OP` 是"取根地址的
 * 那个 op"，名字里带 `OP` 才不会与 `schemas/vdfsRoot` 的根**地址**混淆；而后端
 * 只有一份常量名。名字不同 ⇒ 自动发现看不见它们，因此必须显式登记。
 */
const ALIASES = [
  {
    ts: 'VDFS_ROOT_OP',
    rust: { file: 'symbio/src/plugins/vdfs/protocol.rs', name: 'VDFS_ROOT' },
    what: '进入地址空间：列出虚拟根（不给地址）',
  },
  {
    ts: 'VDFS_EVENT_KIND',
    rust: { file: 'symbio/src/symbio_core/event_bus.rs', name: 'KIND_VDFS' },
    what: 'VDFS 变更的总线频道名',
  },
]

/**
 * 前端**自持**的 `VDFS_*` 常量（后端没有对应协议词）。
 *
 * 这是逃生舱：能进这里说明它**不是**跨栈契约，而是前端自己的概念。
 * 每条必须写明理由——「后端没有对应」这件事本身要经得起复核。
 */
const LOCAL_ONLY = []

// ==================== B. 缺席检查 ====================

/**
 * 前端**不得再出现**的常量。
 *
 * 会话地址段已改为运行期发现（`services/vdfsScheme.ts`）：挂载段按「挂载点声明
 * 可新建 `ext = session`」认出来，转写段按 `kind = VDFS_KIND_MESSAGES` 认出来。
 * 前端持有它们就是第二份真相——这两条防止它们悄悄回来。
 */
const ABSENT = [
  {
    name: 'VDFS_SESSION_DIR',
    what: '会话挂载段常量（应由 vdfsScheme 运行期发现）',
    pattern: /\bVDFS_SESSION_DIR\b/,
  },
  {
    name: 'VDFS_SEG_MESSAGES',
    what: '转写段常量（应由 vdfsScheme 按 kind 发现）',
    pattern: /\bVDFS_SEG_MESSAGES\b/,
  },
]

// ==================== C. 闭集词表 ====================

/**
 * 后端**闭集枚举** ↔ 前端**词表数组**。
 *
 * 对应关系无法自动推断（`MessageRole` ↔ `CHAT_ROLES` 名字不同），故显式登记；
 * 但登记后**取值**是自动提取比对的——枚举加变体、词表加取值，两边立刻对上账。
 */
const CHAT_MESSAGE_RS = 'symbio/src/symbio_core/schemas/session/chat_message.rs'
const CHAT_MESSAGE_TS = 'tauri/src/schemas/chat_message.ts'

const ENUM_SETS = [
  {
    what: '消息角色',
    rust: { file: CHAT_MESSAGE_RS, enum: 'MessageRole' },
    ts: { file: CHAT_MESSAGE_TS, array: 'CHAT_ROLES' },
  },
  {
    what: '消息类型',
    rust: { file: CHAT_MESSAGE_RS, enum: 'MessageType' },
    ts: { file: CHAT_MESSAGE_TS, array: 'MESSAGE_TYPES' },
  },
  {
    what: '消息状态',
    rust: { file: CHAT_MESSAGE_RS, enum: 'MessageStatus' },
    ts: { file: CHAT_MESSAGE_TS, array: 'MESSAGE_STATUSES' },
  },
  {
    what: '会话恢复动作',
    rust: { file: CHAT_MESSAGE_RS, enum: 'ResumeAction' },
    ts: { file: CHAT_MESSAGE_TS, array: 'RESUME_ACTIONS' },
  },
]

// ==================== 提取 ====================

/** 后端：全部 `pub const NAME: &str = "v"`（含 `pub(crate)`） */
function rustConsts(src) {
  const out = new Map()
  const re = /(?:pub|pub\(crate\))\s+const\s+(\w+)\s*:\s*&str\s*=\s*"([^"]*)"/g
  let m
  while ((m = re.exec(src))) out.set(m[1], m[2])
  return out
}

/**
 * 前端：全部 `export const NAME = 'v'`。
 *
 * 右侧**必须**是字面量：`export const VDFS_STATUS_FAILED = MESSAGE_STATUS_FAILED`
 * 这类别名不算镜像——它没有第二份真相，值随被别名者走。别名正是**消除**镜像的
 * 手段，不该被当成镜像来守（也因此 `VDFS_STATUS_*` 里只有字面量的两条进 A 组，
 * 其余六条由 C 组经 `MessageStatus` 覆盖）。
 */
function tsConsts(src) {
  const out = new Map()
  const re = /export\s+const\s+(\w+)\s*=\s*'([^']*)'/g
  let m
  while ((m = re.exec(src))) out.set(m[1], m[2])
  return out
}

/** 从 `from` 起第一个 `{` 到配对 `}` 之间的内容（不含花括号本身） */
function braceBlock(src, from) {
  const start = src.indexOf('{', from)
  if (start < 0) return null
  let depth = 0
  for (let i = start; i < src.length; i++) {
    if (src[i] === '{') depth++
    else if (src[i] === '}') {
      depth--
      if (depth === 0) return src.slice(start + 1, i)
    }
  }
  return null
}

/** 找到 `pub enum <name>` 的位置，没有则 null */
function enumHead(src, name) {
  return src.match(new RegExp(`(?:pub|pub\\(crate\\))\\s+enum\\s+${name}\\b`))
}

/**
 * 闭集枚举的变体名。
 *
 * 只认「行首（去空白）大写字母开头、后跟 `,` / `{` / `(`」的行——属性行
 * （`#[default]` / `#[serde(..)]`）与注释行因此自动跳过。枚举体内不会有别的
 * 以大写字母开头的语句，所以不需要 AST。
 */
function rustEnumVariants(src, name) {
  const m = enumHead(src, name)
  if (!m) return null
  const body = braceBlock(src, m.index)
  if (body === null) return null
  const out = []
  for (const raw of body.split(/\r?\n/)) {
    const t = raw.trim()
    if (!t || t.startsWith('#') || t.startsWith('//')) continue
    const vm = t.match(/^([A-Z][A-Za-z0-9]*)\s*(?:,|\{|\()/)
    if (vm) out.push(vm[1])
  }
  return out
}

/**
 * 枚举声明**正上方**的连续属性里，是否有 `rename_all = "snake_case"`。
 *
 * 从 `enum` 往前逐行收集 `#[...]`，遇到第一个非属性、非空、非注释行即停——
 * 不能只往上看固定字符数，那会跨到上一个枚举的属性上去。
 */
function hasRenameAllSnakeCase(src, name) {
  const m = enumHead(src, name)
  if (!m) return false
  const lines = src.slice(0, m.index).split(/\r?\n/)
  const attrs = []
  for (let i = lines.length - 1; i >= 0; i--) {
    const t = lines[i].trim()
    if (t === '' || t.startsWith('//')) continue
    if (t.startsWith('#[')) {
      attrs.push(t)
      continue
    }
    break
  }
  return attrs.some((a) => /rename_all\s*=\s*"snake_case"/.test(a))
}

/** Rust 变体名 → `serde(rename_all = "snake_case")` 的线格式词 */
function snakeCase(name) {
  return name
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2')
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .toLowerCase()
}

/** 前端词表数组的**取值**（元素是常量名，到本文件的字面量常量表里解引用） */
function tsArrayValues(src, name) {
  const m = src.match(new RegExp(`const\\s+${name}\\s*=\\s*\\[([\\s\\S]*?)\\]\\s*as\\s+const`))
  if (!m) return { values: null, problem: `未找到词表数组 ${name}` }
  const consts = tsConsts(src)
  const values = []
  for (const raw of m[1].split(',')) {
    const id = raw.split('//')[0].trim()
    if (!id) continue
    if (!consts.has(id)) {
      return { values: null, problem: `${name} 的元素 ${id} 不是本文件里的字符串常量` }
    }
    values.push(consts.get(id))
  }
  if (values.length === 0) return { values: null, problem: `词表数组 ${name} 是空的` }
  return { values, problem: null }
}

/**
 * 递归列出目录下的 .ts / .vue 文件。
 *
 * **排除 `__tests__`**：与 mechanism-audit 的既有口径一致——测试持有的是
 * **协议夹具**（把地址钉成字面量正是它的职责），生产代码才不许持有。
 */
function walk(dir) {
  const out = []
  if (!fs.existsSync(dir)) return out
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const abs = path.join(dir, e.name)
    if (e.isDirectory()) {
      if (e.name === '__tests__') continue
      out.push(...walk(abs))
    } else if (/\.(ts|vue)$/.test(e.name)) {
      out.push(abs)
    }
  }
  return out
}

// ==================== 主流程 ====================

let errors = 0

const readIfExists = (relPath) => {
  const abs = path.join(REPO, relPath)
  return fs.existsSync(abs) ? fs.readFileSync(abs, 'utf8') : null
}
const relOf = (abs) => path.relative(REPO, abs).replace(/\\/g, '/')

console.log('=== protocol-mirror-audit：跨栈协议常量 ===')
console.log(dim(`    仓库根：${REPO}`))
console.log()

// ---------- A ----------
console.log('--- A. 常量镜像（自动发现同名 `VDFS_*`，逐字比对）---')

const rustAll = new Map()
for (const f of RUST_CONST_FILES) {
  const src = readIfExists(f)
  if (src === null) {
    errors++
    console.log(`  ${red(BAD)} 后端常量源不存在：${f}`)
    continue
  }
  for (const [k, v] of rustConsts(src)) rustAll.set(k, { value: v, file: f })
}

const tsAll = new Map()
for (const f of TS_CONST_FILES) {
  const src = readIfExists(f)
  if (src === null) {
    errors++
    console.log(`  ${red(BAD)} 前端常量源不存在：${f}`)
    continue
  }
  for (const [k, v] of tsConsts(src)) tsAll.set(k, { value: v, file: f })
}

let mirrorCount = 0
for (const [name, ts] of tsAll) {
  if (!name.startsWith('VDFS_')) continue
  if (LOCAL_ONLY.some((l) => l.name === name)) continue

  const alias = ALIASES.find((a) => a.ts === name)
  let rustVal = null
  let rustWhere = ''

  if (rustAll.has(name)) {
    rustVal = rustAll.get(name).value
    rustWhere = `${rustAll.get(name).file}::${name}`
  } else if (alias) {
    const src = readIfExists(alias.rust.file)
    const hit = src === null ? null : (rustConsts(src).get(alias.rust.name) ?? null)
    rustWhere = `${alias.rust.file}::${alias.rust.name}`
    if (hit === null) {
      errors++
      console.log(`  ${red(BAD)} [镜像] ${name} —— 别名指向的后端常量未找到`)
      console.log(dim(`      ${rustWhere}`))
      continue
    }
    rustVal = hit
  } else {
    errors++
    console.log(`  ${red(BAD)} [镜像] ${name} —— 前端持有，后端无同名常量，也未登记`)
    console.log(dim('      它确是协议词镜像（只是名字不同）→ 登记到 ALIASES；'))
    console.log(dim('      它是前端自持概念 → 登记到 LOCAL_ONLY 并写明理由。'))
    continue
  }

  mirrorCount++
  if (rustVal === ts.value) {
    const via = alias ? dim(`（后端 ${alias.rust.name}）`) : ''
    console.log(`  ${green(OK)} ${name} = ${JSON.stringify(rustVal)}${via}`)
    continue
  }
  errors++
  console.log(
    `  ${red(BAD)} ${name} —— 取值不一致：后端 ${JSON.stringify(rustVal)} ≠ 前端 '${ts.value}'`,
  )
  console.log(dim(`      ${rustWhere} ↔ ${ts.file}::${name}`))
}

// ---------- B ----------
console.log()
console.log('--- B. 缺席检查（前端不得再持有会话地址段）---')
const frontendFiles = walk(path.join(REPO, 'tauri', 'src'))
for (const a of ABSENT) {
  const hits = []
  for (const f of frontendFiles) {
    const lines = fs.readFileSync(f, 'utf8').split(/\r?\n/)
    lines.forEach((line, i) => {
      if (a.pattern.test(line)) hits.push(`${relOf(f)}:${i + 1}`)
    })
  }
  if (hits.length === 0) {
    console.log(`  ${green(OK)} ${a.name} —— ${a.what}`)
    continue
  }
  errors += hits.length
  console.log(`  ${red(BAD)} ${a.name} —— 出现 ${hits.length} 处（${a.what}）`)
  for (const h of hits.slice(0, 10)) console.log(`      ${red(h)}`)
  console.log(dim('      这两个段名由后端 provider 决定，前端应运行期发现'))
}

// ---------- C ----------
console.log()
console.log('--- C. 闭集词表（后端枚举取值 ↔ 前端词表，集合相等）---')
for (const e of ENUM_SETS) {
  const rustSrc = readIfExists(e.rust.file)
  const tsSrc = readIfExists(e.ts.file)
  const problems = []
  let rustWords = null
  let tsWords = null

  if (rustSrc === null) {
    problems.push(`后端文件不存在：${e.rust.file}`)
  } else {
    const variants = rustEnumVariants(rustSrc, e.rust.enum)
    if (variants === null) {
      problems.push(`后端未找到枚举 ${e.rust.enum}`)
    } else {
      rustWords = variants.map(snakeCase)
      if (!hasRenameAllSnakeCase(rustSrc, e.rust.enum)) {
        problems.push(`${e.rust.enum} 缺 #[serde(rename_all = "snake_case")]`)
      }
    }
  }

  if (tsSrc === null) {
    problems.push(`前端文件不存在：${e.ts.file}`)
  } else {
    const got = tsArrayValues(tsSrc, e.ts.array)
    if (got.problem) problems.push(got.problem)
    else tsWords = got.values
  }

  if (rustWords !== null && tsWords !== null) {
    const rset = new Set(rustWords)
    const tset = new Set(tsWords)
    const missingInTs = rustWords.filter((w) => !tset.has(w))
    const extraInTs = tsWords.filter((w) => !rset.has(w))
    if (missingInTs.length) problems.push(`前端词表缺少：${missingInTs.join(', ')}`)
    if (extraInTs.length) problems.push(`前端词表多出：${extraInTs.join(', ')}`)
  }

  if (problems.length === 0) {
    console.log(
      `  ${green(OK)} ${e.rust.enum} ↔ ${e.ts.array} —— ${e.what}（${rustWords.length} 个取值）`,
    )
    continue
  }

  errors += problems.length
  console.log(`  ${red(BAD)} ${e.rust.enum} ↔ ${e.ts.array} —— ${e.what}`)
  for (const msg of problems) console.log(`      ${red(msg)}`)
  console.log(dim(`      ${e.rust.file} ↔ ${e.ts.file}`))
}

// ---------- 收尾 ----------
console.log()
console.log(`Errors: ${errors}`)

if (errors > 0) {
  console.log()
  console.log(
    yellow(
      '  A 组：前端持有的 VDFS_* 常量必须与后端同名常量逐字相等（改后端就要改前端镜像）。\n' +
        '  B 组：会话地址段不该出现在前端——它们是运行期发现的数据，不是常量。\n' +
        '  C 组：后端闭集枚举的取值集合必须与前端词表相等（枚举加变体就要加词）。',
    ),
  )
  process.exit(1)
}

console.log(
  green(
    `  A 组 ${mirrorCount} 条常量镜像 + B 组 ${ABSENT.length} 项缺席检查 + ` +
      `C 组 ${ENUM_SETS.length} 张闭集词表，全部一致` +
      `（扫描 ${frontendFiles.length} 个前端文件）`,
  ),
)
process.exit(0)
