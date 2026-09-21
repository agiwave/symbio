#!/usr/bin/env node
/**
 * test-layout-audit — Rust 测试文件布局审计（取代「记在记忆里的布局约定」）
 *
 * 约定（原先靠人记，现在由本脚本判）：
 *   - 测试独立成文件、同级同名加 `.test`：`workdir.rs` + `workdir.test.rs`，
 *     宿主文件**末尾**声明 `#[cfg(test)] #[path = "workdir.test.rs"] mod tests;`
 *   - 宿主是 `mod.rs` 的，测试放同级 `tests.rs`（`#[path = "tests.rs"]`）
 *   - 拆出测试文件前**必须**确认 `mod tests` 是否在文件末尾——在中部时按
 *     「取到文件尾」切会把生产代码搬进测试文件（`model/plugin.rs` 踩过）
 *
 * **棘轮**：真内联 `mod tests` 的文件数**只降不升**（`INLINE_TEST_BASELINE`）。
 * 存量 53 个不要求一次性拆完，但**新增一个即红**。下调基线是拆分进度的一部分——
 * 与 `gate.mjs` 的 `BASELINE` 同一约定。
 * （2026-09-20 前本脚本没有任何棘轮：内联不拆分也不构成违规，只要写在文件末尾就
 *  报 `✓ 布局符合约定` ⇒ 这条约定实际没有执行力。）
 *
 * 用法：
 *   node scripts/test-layout-audit.mjs              # 审计 symbio/src
 *   node scripts/test-layout-audit.mjs --strict     # warning 也算失败
 *   ROOT=cli/src node scripts/test-layout-audit.mjs
 *
 * 退出码：0 = 通过；1 = 有 ERROR；2 = 仅 WARNING 且 --strict。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const rootArg = process.argv.find((a) => a.startsWith('ROOT='))
const rootDir = path.resolve(repoRoot, rootArg ? rootArg.slice(5) : 'symbio/src')
const STRICT = process.argv.includes('--strict')

let errors = 0
let warnings = 0
const error = (s) => {
  errors += 1
  console.log(red(`  ERROR: ${s}`))
}
const warn = (s) => {
  warnings += 1
  console.log(yellow(`  WARN : ${s}`))
}

function walk(dir, out = []) {
  if (!fs.existsSync(dir)) return out
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.name === 'target' || e.name.startsWith('.')) continue
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walk(p, out)
    else if (e.name.endsWith('.rs')) out.push(p)
  }
  return out
}

const rel = (p) => path.relative(repoRoot, p).replace(/\\/g, '/')
const files = walk(rootDir)

console.log(`== 测试布局审计（${rel(rootDir)} 下 ${files.length} 个 .rs）==`)

// ── 1. 已拆分的测试文件：宿主必须存在并声明 #[path] ────────────────────
const testFiles = files.filter((f) => f.endsWith('.test.rs') || path.basename(f) === 'tests.rs')
for (const tf of testFiles) {
  const base = path.basename(tf)
  const dir = path.dirname(tf)
  const host =
    base === 'tests.rs' ? path.join(dir, 'mod.rs') : path.join(dir, base.replace(/\.test\.rs$/, '.rs'))
  if (!fs.existsSync(host)) {
    error(`${rel(tf)} 找不到宿主文件（期望 ${rel(host)}）`)
    continue
  }
  const src = fs.readFileSync(host, 'utf8')
  // 两种合法形式：
  //  - 宿主是 `mod.rs`、测试是 `tests.rs` ⇒ `mod tests;` 即可（Rust 默认找同名文件）
  //  - 宿主是 `X.rs`、测试是 `X.test.rs` ⇒ 必须 `#[path = "X.test.rs"]`，否则编译不到
  const declared =
    base === 'tests.rs'
      ? /\bmod\s+tests\s*;/.test(src) || src.includes('"tests.rs"')
      : src.includes(`"${base}"`)
  if (!declared) {
    error(
      base === 'tests.rs'
        ? `${rel(host)} 未声明 mod tests;（${base} 不会被编译）`
        : `${rel(host)} 未声明 #[path = "${base}"]（测试文件不会被编译）`
    )
  }
}

// ── 2. 内联 mod tests：① 报告「不在文件末尾」的 ② **棘轮**：数量不得增长 ──
//
// 为什么这里要有棘轮：本脚本的约定是「测试独立成文件」，但在此前，新建一个带内联
// `mod tests` 的文件**不会让任何东西变红**——只要内联块写在文件末尾就报
// `✓ 布局符合约定`。于是这条约定**没有棘轮**，存量推不动、增量也拦不住。
// 修法是 `gate.mjs` 的 `BASELINE` 同一手法：**数量只降不升**。它不要求立刻把现有
// 53 个文件全拆掉（那是另一次改动），但从此刻起**新增一个即红**——这正是"约定"
// 与"建议"的区别。
//
// ⚠️ 判定必须区分两种 `mod tests`：
//   - 宿主文件的 `#[path = "X.test.rs"] mod tests;`（**合规**，正是约定要求的写法）
//   - 真正的内联 `mod tests { … }`（才是"未拆分"）
// 旧版用 `/^\s*mod tests\b/m` 一把抓，于是「含内联 mod tests 的文件 106」这个数字
// 把两者混算（53 个宿主声明 + 53 个真内联），既说不清已拆多少、也说不清剩多少。
const INLINE_TEST_BASELINE = 52 // 2026-09-22 实测（真内联，非宿主声明）；自 53 下调（local/shell.rs 拆分）

/** 真·内联测试：`mod tests {` 或 `mod tests\n{`；宿主声明 `mod tests;` 不算 */
const INLINE_RE = /^\s*mod tests\b[^{;]*\{/m
/** 宿主对拆分测试文件的声明（`mod tests;`，通常带 `#[path = "…"]`） */
const HOST_DECL_RE = /^\s*mod tests\s*;/m

const inlineTestFiles = files.filter((f) => !f.endsWith('.test.rs') && path.basename(f) !== 'tests.rs')
let withInline = 0
let hostDecls = 0
for (const f of inlineTestFiles) {
  const src = fs.readFileSync(f, 'utf8')
  if (HOST_DECL_RE.test(src)) hostDecls += 1
  const at = src.search(INLINE_RE)
  if (at < 0) continue
  withInline += 1
  // 从 `mod tests` 起做括号配对，闭合之后若还有实质内容 ⇒ 它在文件**中部**
  let depth = 0
  let seenOpen = false
  let end = -1
  for (let i = at; i < src.length; i += 1) {
    const c = src[i]
    if (c === '{') {
      depth += 1
      seenOpen = true
    } else if (c === '}') {
      depth -= 1
      if (seenOpen && depth === 0) {
        end = i
        break
      }
    }
  }
  if (end < 0) continue
  const after = src.slice(end + 1).replace(/\s+/g, '')
  if (after.length > 0) {
    warn(`${rel(f)} 的 mod tests 在文件**中部**（其后还有生产代码）——拆分时勿按「取到文件尾」切`)
  }
}

console.log()
console.log(
  `  已拆分的测试文件：${testFiles.length}（宿主声明 ${hostDecls}）` +
    `；真内联 mod tests：${withInline}（基线 ${INLINE_TEST_BASELINE}）`
)

// 棘轮：只降不升。低于基线时提示下调（与 gate.mjs 的 BASELINE 同一约定）。
if (withInline > INLINE_TEST_BASELINE) {
  error(
    `真内联 mod tests 的文件 ${withInline} > 基线 ${INLINE_TEST_BASELINE}：` +
      `新增内联测试不被接受——约定是拆成独立的 \`*.test.rs\`（见文件头）。` +
      `若本次拆分了存量文件，应同时把本文件的 INLINE_TEST_BASELINE 下调。`
  )
} else if (withInline < INLINE_TEST_BASELINE) {
  warn(
    `真内联 mod tests 的文件 ${withInline} < 基线 ${INLINE_TEST_BASELINE}：` +
      `请把本文件的 INLINE_TEST_BASELINE 下调——棘轮**只降不升**，留着旧数字等于放弃了这段进展。`
  )
}

if (errors === 0 && warnings === 0) {
  console.log(green('  ✓ 布局符合约定'))
  process.exit(0)
}
console.log(`  Errors: ${errors} · Warnings: ${warnings}`)
if (errors > 0) process.exit(1)
if (STRICT) process.exit(2)
process.exit(0)
