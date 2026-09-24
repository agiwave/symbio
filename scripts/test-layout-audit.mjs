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
 * **棘轮**（两个，管的是**不同维度**，都要维护）：
 *   - `tests`：生产文件里的**测试函数总数** —— 增量闸门。管的是「又往源代码文件里
 *     加了测试」，无论加在新建文件还是存量文件里。
 *   - `files`：**含测试函数的生产文件数** —— 拆分进度。把存量文件拆成 `X.test.rs`
 *     时测试函数数不变、文件数下降，只有它看得到这段进展。
 *
 * 两个都**只降不升**：存量不要求一次拆完，但新增即红；拆完存量要把对应数字下调。
 * 与 `gate.mjs` 的 `BASELINE` 同一约定。
 * （2026-09-20 前本脚本没有任何棘轮：内联不拆分也不构成违规，只要写在文件末尾就
 *  报 `✓ 布局符合约定` ⇒ 这条约定实际没有执行力。）
 *
 * ## 判据为什么是「测试函数」而不是「模块名」
 *
 * 2026-09-24 前的判定是 `^\s*mod tests\b[^{;]*\{` —— **按模块名找**。它有两个洞：
 *
 *   1. `symbio_core/turn.rs` 的测试写在 `mod tool_call_tests` / `mod turn_output_tests`
 *      里（14 个测试函数），**一个都数不到**；往里加多少测试都不会红。
 *   2. 棘轮只数**文件数**：往一个已经有内联 `mod tests` 的存量文件里继续加测试，
 *      文件数不变 ⇒ 完全不红。而「继续往里加」恰恰是最容易发生的路径。
 *
 * 故改为**按测试函数判**（`#[test]` / `#[tokio::test]` / `#[tokio::test(..)]`）——
 * 这才是「测试代码」的本质定义，与它叫哪个模块名、在第几个文件里无关。
 *
 * 用法：
 *   node scripts/test-layout-audit.mjs              # 审计全部 Rust crate
 *   node scripts/test-layout-audit.mjs --strict     # warning 也算失败
 *   ROOT=cli/src node scripts/test-layout-audit.mjs # 只审一个根（回归测试用）
 *
 * 退出码：0 = 通过；1 = 有 ERROR；2 = 仅 WARNING 且 --strict。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const rootArg = process.argv.find((a) => a.startsWith('ROOT='))

/**
 * 棘轮：**按根**分别设基线（各 crate 的存量不同，一个全局数字必然撒谎）。
 *
 * 不在表里的根（回归测试造的临时目录）取 `{ files: 0, tests: 0 }`——一个新目录
 * 本就不该有任何内联测试，这个默认值比"套用 symbio/src 的存量"严格得多也正确得多。
 */
const RATCHETS = {
  // 2026-09-24：存量内联测试已全部拆成独立的 *.test.rs（prod 文件 0 内联测试函数），
  // 故基线压到 0——此后**任何**新建/往源码里加测试函数都会红（见文件头「判据」说明）。
  'symbio/src': { files: 0, tests: 0 },
  'cli/src': { files: 0, tests: 0 },
  'tauri/src-tauri/src': { files: 0, tests: 0 },
}
const DEFAULT_RATCHET = { files: 0, tests: 0 }

/** `ROOT=` 给了就只审那一个；否则审全部 Rust crate（cli / tauri 此前完全没被覆盖） */
const ROOTS = rootArg
  ? [rootArg.slice(5)]
  : ['symbio/src', 'cli/src', 'tauri/src-tauri/src']
const STRICT = process.argv.includes('--strict')

/**
 * `--ratchet=files=N,tests=M`：临时覆盖基线，**只能与 `ROOT=` 一起用**。
 *
 * 存在的唯一理由是让回归测试能造出「基线刚好等于现状」的场景——不覆盖的话，
 * 任何临时目录的基线都是 0，一放测试就红，于是无法验证「在存量文件里**再加一个**
 * 测试也红」（那条路径的特征正是文件数不变）。多根时禁止覆盖：那会让每个根
 * 共用同一个数字，而基线本来就是按根不同的。
 */
const ratchetArg = process.argv.find((a) => a.startsWith('--ratchet='))
let ratchetOverride = null
if (ratchetArg) {
  if (!rootArg) {
    console.error(red('✗ --ratchet 只能与 ROOT=<单根> 一起使用'))
    process.exit(1)
  }
  ratchetOverride = {}
  for (const part of ratchetArg.slice('--ratchet='.length).split(',')) {
    const [k, v] = part.split('=')
    ratchetOverride[k.trim()] = Number(v)
  }
}

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

/** 测试函数属性：`#[test]` / `#[tokio::test]` / `#[tokio::test(start_paused = true)]` */
const TEST_FN_RE = /^[ \t]*#\[(?:tokio::)?test\b[^\]]*\]/gm
/** 宿主对拆分测试文件的声明（`mod tests;`，通常带 `#[path = "…"]`） */
const HOST_DECL_RE = /^\s*mod tests\s*;/m
/** 任何 `mod X {` 开块（含 turn.rs 那种 `mod tool_call_tests {`） */
const MOD_BLOCK_RE = /^[ \t]*mod\s+(\w+)\s*\{/gm

/** 数一个文件里的测试函数（测试代码的本质定义，与模块名无关） */
function countTests(src) {
  return (src.match(TEST_FN_RE) || []).length
}

for (const rootSpec of ROOTS) {
  const rootDir = path.resolve(repoRoot, rootSpec)
  const files = walk(rootDir)

  console.log(`\n== 测试布局审计（${rootSpec} 下 ${files.length} 个 .rs）==`)

  // ── 1. 已拆分的测试文件：宿主必须存在并声明 #[path] ────────────────────
  const testFiles = files.filter((f) => f.endsWith('.test.rs') || path.basename(f) === 'tests.rs')
  for (const tf of testFiles) {
    const base = path.basename(tf)
    const dir = path.dirname(tf)
    const host =
      base === 'tests.rs'
        ? path.join(dir, 'mod.rs')
        : path.join(dir, base.replace(/\.test\.rs$/, '.rs'))
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

  // ── 2. 生产文件里的测试代码 ────────────────────────────────────────────
  const prodFiles = files.filter(
    (f) => !f.endsWith('.test.rs') && path.basename(f) !== 'tests.rs'
  )
  let hostDecls = 0
  let testFns = 0
  const dirty = [] // 含测试函数的生产文件（= 未按约定拆分的文件）
  for (const f of prodFiles) {
    const src = fs.readFileSync(f, 'utf8')
    if (HOST_DECL_RE.test(src)) hostDecls += 1
    const n = countTests(src)
    if (n === 0) continue
    testFns += n
    dirty.push([f, n])

    // 内联测试块在文件**中部** ⇒ 拆分时按「取到文件尾」切会把生产代码搬进测试文件
    for (const m of src.matchAll(MOD_BLOCK_RE)) {
      const name = m[1]
      const before = src.slice(Math.max(0, m.index - 80), m.index)
      const isTestMod = name === 'tests' || /#\[cfg\(test\)\]/.test(before)
      if (!isTestMod) continue
      let depth = 0
      let seenOpen = false
      let end = -1
      for (let i = m.index; i < src.length; i += 1) {
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
      if (src.slice(end + 1).replace(/\s+/g, '').length > 0) {
        warn(
          `${rel(f)} 的测试模块 \`${name}\` 在文件**中部**（其后还有生产代码）` +
            `——拆分时勿按「取到文件尾」切`
        )
      }
    }
  }

  dirty.sort((a, b) => b[1] - a[1])
  console.log(
    `  已拆分的测试文件：${testFiles.length}（宿主声明 ${hostDecls}）` +
      `；含测试函数的生产文件：${dirty.length}；其内测试函数：${testFns}`
  )
  if (dirty.length > 0) {
    console.log(dim('  未拆分存量（前 10，拆分目标）：'))
    for (const [f, n] of dirty.slice(0, 10)) console.log(dim(`    ${String(n).padStart(3)}  ${rel(f)}`))
  }

  // ── 3. 棘轮：两个数字都只降不升 ────────────────────────────────────────
  const base = ratchetOverride ?? RATCHETS[rootSpec] ?? DEFAULT_RATCHET
  const key = rootSpec
  if (dirty.length > base.files) {
    error(
      `${key}：含测试函数的生产文件 ${dirty.length} > 基线 ${base.files}。` +
        `新增未拆分的测试文件不被接受——约定是拆成独立的 \`*.test.rs\`（见文件头）。`
    )
  } else if (dirty.length < base.files) {
    warn(`${key}：含测试函数的生产文件 ${dirty.length} < 基线 ${base.files} —— 请下调 RATCHETS[${key}].files`)
  }
  if (testFns > base.tests) {
    error(
      `${key}：生产文件里的测试函数 ${testFns} > 基线 ${base.tests}。` +
        `**往源代码文件里加测试不被接受**——无论加在新文件还是已有内联测试的文件里，` +
        `都要走 \`X.rs\` + \`X.test.rs\`。`
    )
  } else if (testFns < base.tests) {
    warn(`${key}：生产文件里的测试函数 ${testFns} < 基线 ${base.tests} —— 请下调 RATCHETS[${key}].tests`)
  }
}

console.log()
if (errors === 0 && warnings === 0) {
  console.log(green('  ✓ 布局符合约定'))
  process.exit(0)
}
console.log(`  Errors: ${errors} · Warnings: ${warnings}`)
if (errors > 0) process.exit(1)
if (STRICT) process.exit(2)
process.exit(0)
