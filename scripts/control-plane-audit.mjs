#!/usr/bin/env node
/**
 * control-plane-audit — 控制面形状（CP-001 / CP-004 / CP-006 / CP-009）
 *
 * 环由旧引擎换成 `ring/engine.rs` 的新形状之后，旧判据（阶段机、行表、守卫、
 * 收束格、声明 seal、六条不变量闭集）失去对象。本脚本只保留四条**与引擎无关**的
 * 形状判据，其余随旧引擎一起删除（见 `docs/007-task/034`）。
 *
 * | 码 | 判什么 | 范围 |
 * |---|---|---|
 * | `CP-001` | 唯一循环体：整个环 crate 的生产代码只有一段 `loop` / `while`，落在 `ring/engine.rs` | `crates/symbio-plugin-loop/src/**` |
 * | `CP-004` | 机制层无世界域名：`vocab.rs` 登记的事件名不得作为**代码**字面量出现在 kernel / sdk / loop；端口名不得出现在 **loop 的派发面**（注释与测试夹具不算） | `crates/symbio-kernel/src/**`、`crates/symbio-sdk/src/**`、`crates/symbio-plugin-loop/src/**` |
 * | `CP-006` | 机制面指纹：驱动循环体 + `fn plan` + `fn expire` 的**归一化 token 流**哈希（注释 / 空白 / import 顺序 / 尾逗号无关）与生产文件数冻结在基线里 | `crates/symbio-plugin-loop/src/**` |
 * | `CP-009` | 第二推进循环：除 `ring/engine.rs` 外，任何生产文件的循环体都不得调 `drive(` | 全部 crate 的生产源码 |
 *
 * ## 失效形态
 *
 * 这一族的老病是「读不出 ⇒ 通过」。故：
 * - 环 crate / 基线文件 / `vocab.rs` / 机制三段读不出 ⇒ exit 2（不亮绿灯）；
 * - 基线里的 `mechanism.file` 指向一个不存在的文件 ⇒ 红（陈旧基线）；
 * - `adr` 决策号不存在 ⇒ 红（不许匿名冻结）。
 *
 * ## 用法
 *
 *   node scripts/control-plane-audit.mjs [--root=<仓库根>] [--print-baseline] [--write-baseline]
 *
 * 退出码：0 = 通过；1 = 有红；2 = 读不出（不亮绿灯）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成）
// @ns CP 控制面形状（唯一循环体 / 机制层无域名 / 机制面指纹 / 第二推进循环）
// @codes CP-001 CP-004 CP-006 CP-009

import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { green, red, yellow } from './color.mjs'
import { adrExists } from './adr-index.mjs'
import { stripComments as stripCommentsKeepStrings } from './rust-scan.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')
const LOOP_REL = 'crates/symbio-plugin-loop'
const LOOP_SRC = `${LOOP_REL}/src`
const ENGINE_REL = `${LOOP_SRC}/ring/engine.rs`
const VOCAB_REL = `${LOOP_SRC}/vocab.rs`
const BASELINE_REL = 'scripts/baselines/control-plane.json'
const KERNEL_SRC = 'crates/symbio-kernel/src'
const SDK_SRC = 'crates/symbio-sdk/src'
const PORT_VOCAB_REL = 'crates/symbio-plugin-api/src/vocab.rs'

const PRINT = process.argv.includes('--print-baseline')
const WRITE = process.argv.includes('--write-baseline')

// 测试支持文件不是生产机制面：`test_fixtures.rs` 是 `#[cfg(any(test, feature = "test-fixtures"))]`
// 下的调用方拼写（sdk 的模块文档写明它「只在测试下存在」），与 `.*.test.rs` 同档。
const SKIP_FILE = /^(test_support\.rs|test_fixtures\.rs|.*\.test\.rs)$/
const LOOP_HEAD = /\bloop\s*\{|\bwhile\b[^{;]*\{|\bfor\b[^{;]*\bin\b[^{;]*\{/g
/** 驱动循环只算 `loop` / `while`；`for` 是遍历，不是「一圈一圈推进」。 */
const DRIVER_HEAD = /\bloop\s*\{|\bwhile\b[^{;]*\{/g
const ADVANCE_CALL = /(?<!fn )(?<!async fn )\bdrive\s*\(/
const EVENT_DECL = /EventDecl::(?:new|with_roles|host_with_roles)\(\s*(?:\r?\n\s*)?"([^"]+)"/g
/** 端口名只有一处登记：`plugin-api/vocab.rs` 的 `PORT_*` 常量。 */
const PORT_DECL = /pub const PORT_[A-Z0-9_]+:\s*&str\s*=\s*"([^"]+)"/g
const PART_LABELS = ['drive loop body', 'fn plan', 'fn expire']

function read(rel) {
  try {
    return fs.readFileSync(path.join(ROOT, rel), 'utf8')
  } catch {
    return null
  }
}

function walk(rel, out = []) {
  const abs = path.join(ROOT, rel)
  if (!fs.existsSync(abs)) return out
  for (const ent of fs.readdirSync(abs, { withFileTypes: true })) {
    const child = path.posix.join(rel, ent.name)
    if (ent.isDirectory()) walk(child, out)
    else if (ent.name.endsWith('.rs') && !SKIP_FILE.test(ent.name)) out.push(child)
  }
  return out
}

function cratesProd() {
  const out = []
  const crates = path.join(ROOT, 'crates')
  if (!fs.existsSync(crates)) return out
  for (const ent of fs.readdirSync(crates, { withFileTypes: true })) {
    if (ent.isDirectory()) walk(path.posix.join('crates', ent.name, 'src'), out)
  }
  return out
}

/** 剥注释并把字符串 / 字符字面量替换成 `""` / `''`：结构扫描只看代码。 */
function stripCode(text) {
  let out = ''
  let i = 0
  let lineComment = false
  let block = 0
  while (i < text.length) {
    const two = text.slice(i, i + 2)
    const ch = text[i]
    if (lineComment) {
      if (ch === '\n') {
        lineComment = false
        out += '\n'
      } else out += ' '
      i++
      continue
    }
    if (block > 0) {
      if (two === '*/') {
        block--
        out += '  '
        i += 2
        continue
      }
      out += ch === '\n' ? '\n' : ' '
      i++
      continue
    }
    if (two === '//') {
      lineComment = true
      out += '  '
      i += 2
      continue
    }
    if (two === '/*') {
      block++
      out += '  '
      i += 2
      continue
    }
    if (ch === '"') {
      out += '""'
      i++
      while (i < text.length) {
        if (text[i] === '\\') i += 2
        else if (text[i] === '"') {
          i++
          break
        } else i++
      }
      continue
    }
    if (ch === "'") {
      // 字符字面量 / 生命周期：只把明显是三字符的字符字面量剥掉。
      if (text[i + 2] === "'" && text[i + 1] !== '\\') {
        out += "''"
        i += 3
        continue
      }
      out += ch
      i++
      continue
    }
    out += ch
    i++
  }
  return out
}

/**
 * 只剥注释、**保留字符串字面量**：`CP-004` 要找的正是代码里的字符串，
 * 所以不能用 `stripCode`（它会把字符串换成 `""`）。注释里的「例如 `chat.assistant.final`」
 * 是散文，不是机制层持有世界域名；只有代码字面量才是。
 */
function stripComments(text) {
  let out = ''
  let i = 0
  let lineComment = false
  let block = 0
  while (i < text.length) {
    const two = text.slice(i, i + 2)
    const ch = text[i]
    if (lineComment) {
      if (ch === '\n') {
        lineComment = false
        out += '\n'
      } else out += ' '
      i++
      continue
    }
    if (block > 0) {
      if (two === '*/') {
        block--
        out += '  '
        i += 2
        continue
      }
      out += ch === '\n' ? '\n' : ' '
      i++
      continue
    }
    if (two === '//') {
      lineComment = true
      out += '  '
      i += 2
      continue
    }
    if (two === '/*') {
      block++
      out += '  '
      i += 2
      continue
    }
    if (ch === '"') {
      out += ch
      i++
      while (i < text.length) {
        out += text[i]
        if (text[i] === '\\') {
          i++
          if (i < text.length) out += text[i]
          i++
          continue
        }
        if (text[i] === '"') {
          i++
          break
        }
        i++
      }
      continue
    }
    if (ch === "'" && text[i + 2] === "'" && text[i + 1] !== '\\') {
      out += text.slice(i, i + 3)
      i += 3
      continue
    }
    out += ch
    i++
  }
  return out
}

function lineAt(text, index) {
  let line = 1
  for (let i = 0; i < index && i < text.length; i++) if (text[i] === '\n') line++
  return line
}

function matchBrace(text, open) {
  let depth = 0
  for (let i = open; i < text.length; i++) {
    if (text[i] === '{') depth++
    else if (text[i] === '}') {
      depth--
      if (depth === 0) return i
    }
  }
  return text.length
}

/** 取每个 `loop` / `while` / `for` 的块体。 */
function loopBodies(code, re = LOOP_HEAD) {
  const out = []
  for (const m of code.matchAll(re)) {
    const open = code.indexOf('{', m.index)
    if (open < 0) continue
    const end = matchBrace(code, open)
    out.push({ index: m.index, body: code.slice(open, end + 1) })
  }
  return out
}

function fnBody(code, name) {
  const re = new RegExp(`\\bfn\\s+${name}\\s*\\(`, 'g')
  const m = re.exec(code)
  if (!m) return null
  const open = code.indexOf('{', m.index)
  if (open < 0) return null
  return code.slice(open, matchBrace(code, open) + 1)
}

/**
 * 机制面指纹的**归一化**：只保留语义 token，抹掉格式噪声。
 *
 * 此前指纹是 `engine.rs` 三段拼接后的**原始字节**哈希，于是 `cargo fmt` 每动一次换行 /
 * import 顺序 / 尾逗号，基线就漂移一次、就要写一条 allow。漂移越频繁，allow 就越像
 * 橡皮图章——门最后只剩流程。归一化取三条：
 *
 * ① 注释已由 `stripComments` 去掉（保留字符串内容，不误伤 URL 里的 `//`）；
 * ② 空白全部折叠成一个分隔符：换行、缩进、import 排序都不进指纹；
 * ③ 逗号不计（rustfmt 会给多行参数 / 结构体补尾逗号）。
 *
 * 标识符、字符串 / 字符内容、数字与运算符逐字保留，所以加一个条件、改一处返回值、
 * 换一个比较符都会变。**指纹对格式不敏感，对语义敏感**——这正是基线该量的东西。
 */
function normalizeRust(text) {
  const tokens = []
  let i = 0
  while (i < text.length) {
    const ch = text[i]
    if (/\s/.test(ch)) {
      i++
      continue
    }
    // 原始字符串：r"…" / r#"…"# / br#"…"# / cr#"…"#（整个字面量是一个 token）
    const raw = /^(?:c|b)?r(#*)"/.exec(text.slice(i))
    if (raw) {
      const close = `"${raw[1]}`
      const end = text.indexOf(close, i + raw[0].length)
      const stop = end < 0 ? text.length : end + close.length
      tokens.push(text.slice(i, stop))
      i = stop
      continue
    }
    // 普通 / 字节字符串：内容逐字保留（语义的一部分）
    if (ch === '"') {
      let j = i + 1
      while (j < text.length) {
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
      tokens.push(text.slice(i, j))
      i = j
      continue
    }
    // 字符字面量与生命周期：前者是一个 token，后者拆成 `'` + 标识符
    if (ch === "'") {
      const chr = /^'(?:\\.|[^'\\])'/.exec(text.slice(i))
      if (chr) {
        tokens.push(chr[0])
        i += chr[0].length
        continue
      }
      tokens.push("'")
      i++
      continue
    }
    if (/[A-Za-z_]/.test(ch)) {
      const m = /^[A-Za-z_][A-Za-z0-9_]*/.exec(text.slice(i))
      tokens.push(m[0])
      i += m[0].length
      continue
    }
    if (/[0-9]/.test(ch)) {
      const m = /^[0-9][A-Za-z0-9_]*/.exec(text.slice(i))
      tokens.push(m[0])
      i += m[0].length
      continue
    }
    if (ch === ',') {
      i++
      continue
    }
    tokens.push(ch)
    i++
  }
  return tokens.join(' ')
}

function mechanismSource() {
  const engine = read(ENGINE_REL)
  if (engine === null) return null
  const code = stripCommentsKeepStrings(engine)
  const loops = loopBodies(code, DRIVER_HEAD)
  const plan = fnBody(code, 'plan')
  const expire = fnBody(code, 'expire')
  if (loops.length !== 1 || plan === null || expire === null) return null
  return [loops[0].body, plan, expire].join('\n')
}

const baseline = (() => {
  const raw = read(BASELINE_REL)
  if (raw === null) return null
  try {
    return JSON.parse(raw)
  } catch {
    return null
  }
})()

const errors = []

function report(code, rel, msg) {
  errors.push({ code, rel, msg })
}

// ───────────────────────────── CP-001 唯一循环体 ─────────────────────────────

const loopFiles = walk(LOOP_SRC)
if (loopFiles.length === 0) {
  console.error(red(`control-plane-audit：环 crate 生产源码读不出（${LOOP_SRC}）`))
  process.exit(2)
}

const cp001 = []
for (const rel of loopFiles) {
  const code = stripCode(read(rel) ?? '')
  for (const l of loopBodies(code, DRIVER_HEAD)) cp001.push({ rel, line: lineAt(code, l.index) })
}
if (cp001.length !== 1) {
  report('CP-001', LOOP_SRC, `唯一循环体：测得 ${cp001.length} 段（${cp001.map((x) => `${x.rel}:${x.line}`).join('、') || '无'}）`)
} else if (cp001[0].rel !== ENGINE_REL) {
  report('CP-001', cp001[0].rel, `循环体在 ${cp001[0].rel}:${cp001[0].line}，唯一允许位置是 ${ENGINE_REL}`)
}

// ───────────────────────────── CP-004 机制层无世界域名 ─────────────────────────────

const vocab = read(VOCAB_REL)
if (vocab === null) {
  console.error(red(`control-plane-audit：词汇表读不出（${VOCAB_REL}）`))
  process.exit(2)
}
const worldNames = [...new Set([...vocab.matchAll(EVENT_DECL)].map((m) => m[1]))]
if (worldNames.length === 0) {
  console.error(red(`control-plane-audit：词汇表里一个事件名都读不出（${VOCAB_REL}）`))
  process.exit(2)
}
const portVocab = read(PORT_VOCAB_REL)
if (portVocab === null) {
  console.error(red(`control-plane-audit：端口词汇表读不出（${PORT_VOCAB_REL}）`))
  process.exit(2)
}
const portNames = [...new Set([...portVocab.matchAll(PORT_DECL)].map((m) => m[1]))]
if (portNames.length === 0) {
  console.error(red(`control-plane-audit：端口词汇表里一个端口名都读不出（${PORT_VOCAB_REL}）`))
  process.exit(2)
}

// 世界事件名：kernel / sdk / loop 的**代码**都不许持有名字。`vocab.rs` 是声明表本身
// （`TC-2` 的「除声明表与测试外」），不算机制层。
for (const src of [KERNEL_SRC, SDK_SRC, LOOP_SRC]) {
  for (const rel of walk(src)) {
    if (rel === VOCAB_REL) continue
    const code = stripComments(read(rel) ?? '')
    for (const name of worldNames) {
      if (code.includes(`"${name}"`)) report('CP-004', rel, `机制层出现世界事件名字面量 "${name}"`)
    }
  }
}

// 端口名只在**派发面**（loop）判：loop 按世界端口名派发 / 选债，就是把世界名焊进机制。
// kernel / sdk 里的 `"model"` / `"tool"` 大量是**载荷取值**、能力家族名与消息角色
// （见 `plugin-api/vocab.rs` 的「多义拼写」注），文本扫描分不出概念，故不在此判。
for (const rel of walk(LOOP_SRC)) {
  if (rel === VOCAB_REL) continue
  const code = stripComments(read(rel) ?? '')
  for (const name of portNames) {
    if (code.includes(`"${name}"`)) report('CP-004', rel, `派发面出现端口名字面量 "${name}"`)
  }
}

// ───────────────────────────── CP-006 机制面指纹 ─────────────────────────────

if (baseline === null && !(PRINT || WRITE)) {
  console.error(red(`control-plane-audit：基线读不出（${BASELINE_REL}）`))
  process.exit(2)
}
const mech = baseline?.mechanism ?? {}
if (mech.file !== ENGINE_REL) report('CP-006', mech.file ?? BASELINE_REL, `机制面文件应为 ${ENGINE_REL}，基线写的是 ${mech.file ?? '(空)'}`)
if (!mech.reason || !String(mech.reason).trim()) report('CP-006', BASELINE_REL, '机制面理由为空')
if (!mech.adr || !adrExists(ROOT, mech.adr)) report('CP-006', BASELINE_REL, `机制面决策号不存在：${mech.adr ?? '(空)'}`)

const source = mechanismSource()
// `engine.rs` 整个读不出（文件被删 / 权限）才是「范围读不出」退 2；
// 文件在、但驱动循环不是恰好一段 / 没有 `fn plan` / 没有 `fn expire` 是**形状缺口**，
// 归 `CP-006` 判红——否则 `CP-001` 已经报出的「两段循环」会被一个 exit 2 吞掉。
if (source === null && read(ENGINE_REL) === null) {
  console.error(red(`control-plane-audit：机制面文件读不出（${ENGINE_REL}）`))
  process.exit(2)
}
const hash = source === null ? null : `sha256:${crypto.createHash('sha256').update(normalizeRust(source)).digest('hex')}`
const productionFiles = loopFiles.length

if (PRINT || WRITE) {
  if (source === null) {
    console.error(red(`control-plane-audit：机制三段读不出，无法打印基线（${ENGINE_REL}）`))
    process.exit(2)
  }
  const out = {
    mechanism: { file: ENGINE_REL, parts: PART_LABELS, hash, adr: mech.adr ?? '', reason: mech.reason ?? '' },
    extension: { productionFiles },
  }
  const text = JSON.stringify(out, null, 2)
  if (WRITE) fs.writeFileSync(path.join(ROOT, BASELINE_REL), `${text}\n`)
  else console.log(text)
  process.exit(0)
}

if (source === null) {
  report('CP-006', ENGINE_REL, `机制三段读不出（需要恰好 1 段驱动循环 + fn plan + fn expire）——形状缺口先看 CP-001`)
} else if (mech.hash !== hash) {
  report('CP-006', ENGINE_REL, `机制面归一化哈希变了：基线 ${mech.hash ?? '(空)'} ≠ 实测 ${hash} —— 语义变了才该变（格式 / 注释不进来）；机制演进必须同批更新基线并署名`)
}
if (Number(baseline?.extension?.productionFiles) !== productionFiles) {
  report('CP-006', LOOP_SRC, `生产文件数变了：基线 ${baseline?.extension?.productionFiles} ≠ 实测 ${productionFiles}`)
}

// ───────────────────────────── CP-009 第二推进循环 ─────────────────────────────

for (const rel of cratesProd()) {
  const code = stripCode(read(rel) ?? '')
  for (const l of loopBodies(code)) {
    if (!ADVANCE_CALL.test(l.body)) continue
    if (rel === ENGINE_REL) continue
    report('CP-009', rel, `第 ${lineAt(code, l.index)} 行的循环体调 drive(...) —— 推进只许由 ${ENGINE_REL} 的单一循环表达`)
  }
}

// ───────────────────────────── 输出 ─────────────────────────────

for (const e of errors) {
  console.error(`${red('[ERROR]')} ${e.code} ${e.rel} ${e.msg}`)
}

if (errors.length) {
  console.error(red(`control-plane-audit 红：${errors.length} 条`))
  process.exit(1)
}

console.log(
  green(
    `control-plane-audit 通过：CP-001 唯一循环体 1（${ENGINE_REL}）· CP-004 世界事件名 ${worldNames.length} 个、端口名 ${portNames.length} 个、机制层 0 处 · ` +
      `CP-006 机制面 ${hash ? `${hash.slice(7, 19)}…` : '(读不出)'}（${productionFiles} 文件）· CP-009 第二推进循环 0`,
  ),
)
console.log(yellow(`  ⊘ 旧引擎判据（选债口 / 守卫同处 / 收束格 / 判据集闭集 / 声明面封闭）已随旧形状删除（034）`))
process.exit(0)
