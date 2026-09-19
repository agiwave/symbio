#!/usr/bin/env node
/**
 * grep-audit — 静态审计检查脚本（对应 PLAN §S-4）
 *
 * 用途：拦截常见异步/同步错误模式，防止 v27-v28 修复过的 bug 复发
 *   - S-002:        std::sync::Mutex 在 async 上下文中持锁跨 await
 *   - S-002-bonus:  业务路径 `let _ = ...await` 吞错
 *   - S-007:        CHANGELOG 缺关键修复条目（v25-N6 案例）
 *   - S-008:        VdfsNode.status 用裸字面量赋值（词表只有 `VDFS_STATUS_*`）
 *   - S-009:        事件总线 kind 用裸字面量（词表只有 `KIND_*`）
 *
 * 用法：
 *   node scripts/grep-audit.mjs            # 审计 symbio/src/plugins（全部插件）
 *   node scripts/grep-audit.mjs --strict   # 严格模式：warning 也算失败
 *   SCOPE=<dir> node scripts/grep-audit.mjs
 *
 * 退出码：
 *   0 = 全部通过
 *   1 = 发现 ERROR（必须修复）
 *   2 = 仅 WARNING（建议修复，--strict 才会失败）
 *
 * 由 scripts/grep_audit.sh 迁移而来：纯 Node 实现，**不依赖 bash / ripgrep**，
 * Windows / macOS / Linux 通用（仓库约定：脚本一律平台无关）。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const cwd = process.cwd()

const STRICT = process.argv.includes('--strict')

// ── 输出配色（非 TTY / NO_COLOR 时自动关闭，CI 日志保持纯净）──────────────
const useColor = Boolean(process.stdout.isTTY) && process.env.NO_COLOR === undefined
const paint = (code) => (s) => (useColor ? `\x1b[${code}m${s}\x1b[0m` : s)
const red = paint('0;31')
const yellow = paint('0;33')
const green = paint('0;32')

let errors = 0
let warnings = 0

const err = (m) => {
  console.log(`${red('[ERROR]')} ${m}`)
  errors++
}
const warn = (m) => {
  console.log(`${yellow('[WARN] ')} ${m}`)
  warnings++
}
const ok = (m) => console.log(`${green('[OK]   ')} ${m}`)

// ── SCOPE 推断 ─────────────────────────────────────────────────────────
// 环境变量 > cwd 相对 > 仓库根相对 > 默认值
const isDir = (p) => {
  try {
    return fs.statSync(p).isDirectory()
  } catch {
    return false
  }
}

function resolveScope() {
  if (process.env.SCOPE) return path.resolve(cwd, process.env.SCOPE)
  for (const p of [
    path.resolve(cwd, 'src/plugins'),
    path.resolve(cwd, 'symbio/src/plugins'),
    path.resolve(repoRoot, 'symbio/src/plugins'),
  ]) {
    if (isDir(p)) return p
  }
  return path.resolve(cwd, 'src/plugins')
}

const scopeAbs = resolveScope()
/** 显示用路径：相对 cwd，并统一为正斜杠（跨平台一致） */
const disp = (p) => path.relative(cwd, p).split(path.sep).join('/') || '.'

// ── 收集 .rs 源文件 ────────────────────────────────────────────────────
function walk(dir) {
  const out = []
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) out.push(...walk(p))
    else if (e.isFile() && e.name.endsWith('.rs')) out.push(p)
  }
  return out
}

const files = walk(scopeAbs)
const linesOf = new Map()
for (const f of files) {
  try {
    linesOf.set(f, fs.readFileSync(f, 'utf8').split(/\r?\n/))
  } catch {
    /* 读不到就跳过 */
  }
}

console.log('=== grep-audit.mjs ===')
console.log(`Scope: ${disp(scopeAbs)}`)
console.log(`Strict: ${STRICT}`)
console.log()

// ── S-002: std::sync::Mutex + .lock() 跨 await ─────────────────────────
// 启发式：某行出现 .lock()，且同文件后续 5 行内出现 .await，
//         且该文件引用了 std::sync::Mutex → ERROR
console.log('--- S-002: std::sync::Mutex 跨 await 检查 ---')

if (files.length === 0) {
  warn(`scope 不存在或无 .rs 文件（跳过 S-002 检查）：${disp(scopeAbs)}`)
} else {
  let asyncCount = 0
  for (const lines of linesOf.values()) {
    for (const l of lines) if (/^\s*(pub\s+)?async\s+fn\s+\w+/.test(l)) asyncCount++
  }

  if (asyncCount === 0) {
    ok('未发现 async fn（跳过 S-002 检查）')
  } else {
    let bad = 0
    for (const [file, lines] of linesOf) {
      if (!/use std::sync::Mutex|std::sync::Mutex</.test(lines.join('\n'))) continue

      const lockLines = []
      const awaitLines = []
      lines.forEach((l, i) => {
        const n = i + 1
        if (/\.lock\(\)/.test(l)) lockLines.push(n)
        if (/\.await\b/.test(l)) awaitLines.push(n)
      })
      if (lockLines.length === 0 || awaitLines.length === 0) continue

      for (const ln of lockLines) {
        // 仅允许带理由的行级人工豁免；测试代码同样参与检查。
        if (/\/\/\s*grep-audit-allow S-002:\s*\S/.test(lines[ln - 1])) continue
        if (awaitLines.some((a) => a > ln && a <= ln + 5)) {
          err(
            `${disp(file)}:${ln}  std::sync::Mutex 持锁后 5 行内出现 .await` +
              `（潜在跨 await 持锁，请改用 tokio::sync::Mutex）`,
          )
          bad++
        }
      }
    }
    if (bad === 0) ok('S-002 通过：未发现 std::sync::Mutex 跨 await 持锁')
  }
}
console.log()

// ── S-002-bonus: 业务路径 let _ = ...await ─────────────────────────────
// 命中行若位于 #[test] / #[tokio::test] / fn test_ / mod tests 之后 200 行内，
// 视为测试代码并跳过。
console.log('--- S-002-bonus: 业务路径 let _ = ...await 检查 ---')

const SUSPECT_RE = /let _ = .*\.await/
const TEST_MARKER_RE = /#\[(tokio::)?test\]|fn test_|mod tests/

const suspects = []
for (const [file, lines] of linesOf) {
  const markers = []
  const hits = []
  lines.forEach((l, i) => {
    const n = i + 1
    if (TEST_MARKER_RE.test(l)) markers.push(n)
    if (SUSPECT_RE.test(l) && !l.includes('.tx.send')) hits.push(n)
  })
  for (const h of hits) {
    // 同一行可能既是 marker 又是命中，此时 markers 含 h 自身，需排除 m === h
    const inTest = markers.some((m) => m < h && h - m <= 200)
    if (!inTest) suspects.push(`${disp(file)}:${h}:${lines[h - 1].trim()}`)
  }
}

if (suspects.length > 0) {
  for (const s of suspects) {
    warn(`${s}  业务路径疑似吞错，请人工 review（应改为 if let Err(e) = ... + plugin_warn）`)
  }
} else {
  ok('S-002-bonus 通过：业务路径无新增 let _ = ...await')
}
console.log()

// ── S-007: CHANGELOG 维护检查 ──────────────────────────────────────────
console.log('--- S-007: CHANGELOG 维护检查 ---')

// 约定（见 CONTRIBUTING.md「提交规范」）：对外可见的变更统一记入**仓库级**
// `docs/CHANGELOG.md`。agent 插件自身的 `docs/` 树（含其插件级 CHANGELOG）
// 已在 OAB 简化时并入 `docs/design/open-agent-bundle-spec.md`，不再单独维护。
// 因此：scope 下若仍有插件级 CHANGELOG 则优先检查它，否则回退到仓库级。
// 标题格式按实际约定放宽为 `## vNN` 或 `## YYYY-MM-DD`。
const scopedChangelog = path.join(scopeAbs, 'docs', 'CHANGELOG.md')
const changelog = fs.existsSync(scopedChangelog)
  ? scopedChangelog
  : path.join(repoRoot, 'docs', 'CHANGELOG.md')
if (!fs.existsSync(changelog)) {
  err(`${disp(changelog)} 不存在`)
} else {
  const head = fs
    .readFileSync(changelog, 'utf8')
    .split(/\r?\n/)
    .find((l) => /^## (v\d+|\d{4}-\d{2}-\d{2})/.test(l))
  if (!head) err(`${disp(changelog)} 无版本/日期标题（## vNN 或 ## YYYY-MM-DD）`)
  else ok(`CHANGELOG 最新标题：${head}`)
}
console.log()

// ── S-008: VdfsNode.status 不得用裸字面量 ──────────────────────────────
//
// 词表只有一套：`symbio_core::vdfs_provider::VDFS_STATUS_*`（`docs/design/vdfs.md`
// §3.2）。裸字面量的危险不是拼错（那会立刻看见），而是**改名时不会编译失败**——
// `status` 是跨进程边界的字符串，改了常量而漏掉字面量，前端只会静默认不出状态。
//
// 该词表**曾经**把「以错误结束」从 `error` 改名为 `failed`（理由见
// `VDFS_STATUS_FAILED` 的文档：与消息层 `MessageStatus::Failed` 同词），而
// `symbio_core::schemas::options.rs` 里留了一枚 `OPTION_STATUS_ERROR = "error"`
// 的化石、`mcp` / `skill` / `model` 三个插件各自手写 `"unknown"` / `"active"` /
// `"disabled"`——共 6 处。三处都**没有任何测试会因此变红**，故立此规则。
//
// 判据（只认两处无歧义的位置，宁可漏报）：
//   · `.status = "<字面量>"`，含 `if` / `match` 分支里写字面量的形态；
//   · `with_status("<字面量>")`。
// **不**认结构体字面量的 `status: "…"`——响应信封（`SimpleResponse`）的
// `status: "success"` 是另一套词表，按位置判会误报，故留作已知边界。
console.log('--- S-008: VdfsNode.status 字面量检查 ---')

const STATUS_ASSIGN_RE = /\.status\s*=\s*([^;]{0,240});/g
const STATUS_SETTER_RE = /with_status\(\s*"/
const WAIVER_S008_RE = /\/\/\s*grep-audit-allow S-008:\s*\S/

let s008 = 0
for (const [file, lines] of linesOf) {
  const text = lines.join('\n')
  for (const m of text.matchAll(STATUS_ASSIGN_RE)) {
    const rhs = m[1].trim()
    // 只在「值位置直接是字面量」时判：`= "x"` 或 `= if/match … { "x" … }`
    const bad = rhs.startsWith('"') || (/^(if|match)\b/.test(rhs) && rhs.includes('"'))
    if (!bad) continue
    const ln = text.slice(0, m.index).split('\n').length
    if (WAIVER_S008_RE.test(lines[ln - 1])) continue
    err(`${disp(file)}:${ln}  status 用裸字面量赋值；改引 VDFS_STATUS_* 常量（改名才不会静默失效）`)
    s008++
  }
  lines.forEach((l, i) => {
    if (!STATUS_SETTER_RE.test(l) || WAIVER_S008_RE.test(l)) return
    err(`${disp(file)}:${i + 1}  with_status 收到裸字面量；改引 VDFS_STATUS_* 常量`)
    s008++
  })
}
if (s008 === 0) ok('S-008 通过：status 一律取自 VDFS_STATUS_* 常量')
console.log()

// ── S-009: 事件总线 kind 不得用裸字面量 ────────────────────────────────
//
// 词表只有一套：`symbio_core::event_bus::KIND_*`（文档见
// `docs/architecture/PROTOCOLS.md` §事件总线频道）。与 S-008 同源：`kind` 是
// **跨进程**字符串，改名不会编译失败，只会让消费方的「按 kind 分派」静默失效。
//
// 为什么单独立一条：真实事故就是「发布点写裸字面量 → 常量声明出来却无人引用」，
// 被 `dead-code-audit` 的 R-001 当成死代码报了出来（`KIND_SESSION` / `KIND_SYSTEM`
// 的家史）。而 R-001 只在常量**全仓零引用**时才响——只要别处引用过一次，写裸字面量
// 的发布点就再也无人发现。故按位置立此规则。
//
// 判据（只认 `publish` / `try_publish` 首参直接是字面量的形态，宁可漏报）：
//   · `EventBus::publish("<字面量>"` / `EventBus::try_publish("<字面量>"`
//   · `<receiver>.publish("<字面量>"` / `.try_publish("<字面量>"`
console.log('--- S-009: 事件总线 kind 字面量检查 ---')

const KIND_LITERAL_RE = /(?:try_)?publish\(\s*"/
const WAIVER_S009_RE = /\/\/\s*grep-audit-allow S-009:\s*\S/

let s009 = 0
for (const [file, lines] of linesOf) {
  lines.forEach((l, i) => {
    if (!KIND_LITERAL_RE.test(l) || WAIVER_S009_RE.test(l)) return
    err(
      `${disp(file)}:${i + 1}  publish 首参是裸 kind 字面量；` +
        `改引 KIND_* 常量（kind 是跨进程字符串，改名不会编译失败）`,
    )
    s009++
  })
}
if (s009 === 0) ok('S-009 通过：事件 kind 一律取自 KIND_* 常量')
console.log()

// ── 汇总 ───────────────────────────────────────────────────────────────
console.log('=== 汇总 ===')
console.log(`Errors:   ${errors}`)
console.log(`Warnings: ${warnings}`)

if (errors > 0) process.exit(1)
if (STRICT && warnings > 0) process.exit(2)
process.exit(0)
