#!/usr/bin/env node
/**
 * grep-audit — 静态审计检查脚本（对应 PLAN §S-4）
 *
 * 用途：拦截常见异步/同步错误模式，防止 v27-v28 修复过的 bug 复发
 *   - S-002:        std::sync::Mutex 在 async 上下文中持锁跨 await
 *   - S-002-bonus:  业务路径 `let _ = ...await` 吞错
 *   - S-007:        CHANGELOG 缺关键修复条目（v25-N6 案例）
 *   - I-014-light:  CognitiveUnit 字段硬编码字符串键
 *
 * 用法：
 *   node scripts/grep-audit.mjs            # 审计 src/plugins/agent
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
    path.resolve(cwd, 'src/plugins/agent'),
    path.resolve(cwd, 'symbio/src/plugins/agent'),
    path.resolve(repoRoot, 'symbio/src/plugins/agent'),
  ]) {
    if (isDir(p)) return p
  }
  return path.resolve(cwd, 'src/plugins/agent')
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

const changelog = path.join(scopeAbs, 'docs', 'CHANGELOG.md')
if (!fs.existsSync(changelog)) {
  err(`${disp(changelog)} 不存在`)
} else {
  const head = fs
    .readFileSync(changelog, 'utf8')
    .split(/\r?\n/)
    .find((l) => /^## v\d+/.test(l))
  if (!head) err(`${disp(changelog)} 无 ## vNN 版本标题`)
  else ok(`CHANGELOG 最新版本标题：${head}`)
}
console.log()

// ── I-014-light: CognitiveUnit 字段硬编码字符串键抽查 ──────────────────
console.log('--- I-014-light: CognitiveUnit 字段硬编码字符串键抽查 ---')

const KEY_RE = /"(is_a|related|name|description|meta_belief|is_strategy|is_skill|is_meta|is_conflict)"/
const hardcoded = []
for (const [file, lines] of linesOf) {
  lines.forEach((l, i) => {
    if (l.includes('#[test]')) return
    if (KEY_RE.test(l)) hardcoded.push(`${disp(file)}:${i + 1}:${l.trim()}`)
  })
}

if (hardcoded.length > 0) {
  warn(`发现 ${hardcoded.length} 处 CognitiveUnit 字段硬编码字符串键（I-014 中期任务方向）`)
  warn('  完整方案见 PLAN M-1（typed_unit 强类型迁移）')
  // 显式限制前 5 条，避免刷屏
  for (const h of hardcoded.slice(0, 5)) console.log(`    ${h}`)
} else {
  ok('I-014-light 通过：未发现字段硬编码字符串键')
}
console.log()

// ── 汇总 ───────────────────────────────────────────────────────────────
console.log('=== 汇总 ===')
console.log(`Errors:   ${errors}`)
console.log(`Warnings: ${warnings}`)

if (errors > 0) process.exit(1)
if (STRICT && warnings > 0) process.exit(2)
process.exit(0)
