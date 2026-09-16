#!/usr/bin/env node
/**
 * gate — 改动后的**统一检查入口**
 *
 * 用途：把「改动后必跑」的那一串命令与它们的坑**收进代码**，而不是记在
 * 文档 / 记忆里靠人背。跑法只有一条：
 *
 *   node scripts/gate.mjs              # 全量（后端 → 前端 → 审计 → 事实文件）
 *   node scripts/gate.mjs --only=frontend
 *   node scripts/gate.mjs --skip=backend
 *   node scripts/gate.mjs --fix        # 先自动格式化 / 重生成，再检查
 *   node scripts/gate.mjs --ci         # CI 对齐：cargo test --workspace（含集成测试）
 *   node scripts/gate.mjs --profile=release   # 额外跑 cargo build（本地默认不跑）
 *
 * ## 与 .github/workflows/ci.yml 的关系
 *
 * CI 目前仍是手写命令（backend / frontend 两个 job 并行）。本脚本的 `--ci` 与
 * `--profile=` 就是为对齐它准备的，后续可把 CI 改成
 * `node scripts/gate.mjs --only=backend --ci --profile=release` 与
 * `node scripts/gate.mjs --only=frontend`，让「检查什么」只有一处真相。
 * ⚠️ 改 CI 前请在 CI 环境验证过再合并（本地无法证明 Actions 上跑得通）。
 *
 * ## 阶段与顺序（顺序有语义，不要随意调）
 *
 *   1. backend   cargo check --tests / test --lib / clippy / fmt --check（在 `symbio/`）
 *   2. frontend  vue-tsc --noEmit / vitest run（在 `tauri/`）
 *   3. docs      grep-audit / style-audit / doc-link-audit
 *   4. facts     gen-current-facts --check（**必须最后**：它由代码生成，
 *                前面任何自动修复都可能改动代码）
 *
 * ## 封装进去的坑（改本脚本前请先读这些，它们都是踩出来的）
 *
 * - **不接管道**：cargo / git 的输出一旦接 `| tail` 就缓冲到 EOF ⇒ 全程零输出，
 *   与卡死无法区分；且管道会**吞掉错误** ⇒ 失败的命令看起来成功。本脚本用
 *   spawn 实时读流：既逐行转发（有进度），又把全文落到日志文件（可回溯），
 *   判定**只信退出码**。
 * - **沙箱误报**：clippy 之后常见的 `[sandbox] target/… 拒绝` / `os error 5`
 *   是沙箱拦截，**不是失败**——看退出码，不要grep 文本。
 * - **rustfmt 只用 `cargo fmt`**：工具链锁 1.93.1（rustfmt 1.8.0），裸 `rustfmt`
 *   走 rustup default ⇒ 格式漂移。CI 与本脚本都用 `cargo fmt --all -- --check`。
 * - **vitest 不能后台跑**：后台会卡死近一小时。本脚本前台跑 + 超时 kill。
 *   它还有两类**假失败**（与 cargo 并发 / 缓存 `EPERM rename .tmp-…`），
 *   判据都是「用例数没少」：退出码非 0 但用例数 ≥ 基线 ⇒ 记为疑似假失败，
 *   打印原因并继续（真失败会用数��变少暴露出来）。
 * - **基线只增不减**：`cargo test --lib` / vitest 的通过数低于基线即失败。
 *   高于基线时提示更新本文件顶部的 `BASELINE`——那是刻意要人看一眼的地方。
 *
 * 退出码：0 = 全通过；1 = 有阶段失败。
 *
 * 平台无关（Windows / macOS / Linux 通用）：不依赖 bash / npx，子进程一律
 * `shell: false` + 参数数组；node 侧脚本用 `process.execPath` 启动。
 */

import fs from 'node:fs'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const backendDir = path.join(repoRoot, 'symbio')
const frontendDir = path.join(repoRoot, 'tauri')
const logDir = path.join(repoRoot, '.workbuddy-ai', 'gate-logs')

/** 通过数基线（**只增不减**；跑高了请更新这里并说明理由） */
const BASELINE = {
  rustTests: 647,
  vitestFiles: 19,
  vitestTests: 156,
}

/** vitest 前台最长等待（毫秒）——超时即 kill，判定交给「用例数是否达标」 */
const VITEST_TIMEOUT_MS = 180_000

// ── 参数 ───────────────────────────────────────────────────────────────
const argv = process.argv.slice(2)
const FIX = argv.includes('--fix')
/** CI 对齐模式：`cargo test --workspace`（含集成测试）而非 `--lib`，且不做基线比较 */
const CI = argv.includes('--ci')
/** `cargo build --profile <p>`：给了就跑构建（本地默认不跑，省几分钟） */
const profileArg = argv.find((a) => a.startsWith('--profile='))
const profile = profileArg ? profileArg.slice(10).trim() : null
const onlyArg = argv.find((a) => a.startsWith('--only='))
const skipArg = argv.find((a) => a.startsWith('--skip='))
const only = onlyArg ? onlyArg.slice(7).split(',').map((s) => s.trim()) : null
const skip = skipArg ? skipArg.slice(7).split(',').map((s) => s.trim()) : []

// ── 输出 ───────────────────────────────────────────────────────────────
const useColor = Boolean(process.stdout.isTTY) && process.env.NO_COLOR === undefined
const paint = (code) => (s) => (useColor ? `\x1b[${code}m${s}\x1b[0m` : s)
const red = paint('0;31')
const green = paint('0;32')
const yellow = paint('0;33')
const dim = paint('2')
const bold = paint('1')

const enabled = (id) => (only ? only.includes(id) : true) && !skip.includes(id)

function stageHeader(n, total, title) {
  console.log()
  console.log(bold(`── 阶段 ${n}/${total} · ${title} ${'─'.repeat(Math.max(0, 44 - title.length))}`))
}

function fmtDuration(ms) {
  const s = Math.round(ms / 1000)
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`
}

/** cargo 的进度噪音（刷屏且无信息量） */
const NOISE = /^(?:\s*$|.*\r$|\s*(Compiling|Checking|Downloading|Downloaded|Updating|Locking|Adding|Removing|Finished|Blocking|Waiting|Fresh|Documenting|Building)\b)/

/**
 * 跑一条命令：**实时逐行转发 + 全文落日志 + 只信退出码**。
 *
 * @param {object} o
 * @param {string} o.label      控制台显示名
 * @param {string} o.cmd        可执行文件
 * @param {string[]} o.args
 * @param {string} [o.cwd]
 * @param {number} [o.timeoutMs]
 * @param {'filtered'|'all'|'none'} [o.echo] 控制台转发策略：`filtered` = 过滤 cargo
 *        进度噪音后转发（默认，长任务用它），`all` = 全转发（短任务用它），
 *        `none` = 不转发（只看结果）。**无论哪种，全文都会落日志。**
 * @returns {Promise<{ok: boolean, code: number|null, signal: string|null, output: string, timedOut: boolean}>}
 */
function run(o) {
  const { label, cmd, args, cwd = repoRoot, timeoutMs = 0, echo = 'filtered' } = o
  process.stdout.write(`  ▸ ${label} … `)

  return new Promise((resolve) => {
    const started = Date.now()
    let output = ''
    let timedOut = false

    const child = spawn(cmd, args, { cwd, shell: false, env: process.env })
    let timer = null
    if (timeoutMs > 0) {
      timer = setTimeout(() => {
        timedOut = true
        child.kill('SIGTERM')
      }, timeoutMs)
    }

    // 「跑完了但进程不自己退出」的子进程（如 vitest）：总结行一出现就等一小会儿
    // 再收尾，不必干等到超时。
    let settled = false
    let settleTimer = null
    const maybeSettle = () => {
      if (settled || !o.settleAfter) return
      if (!o.settleAfter.re.test(stripAnsi(output))) return
      settled = true
      settleTimer = setTimeout(() => child.kill('SIGTERM'), o.settleAfter.delayMs ?? 1500)
    }

    const consume = (chunk) => {
      const text = chunk.toString()
      output += text
      maybeSettle()
      if (echo !== 'none') {
        for (const raw of text.split('\n')) {
          const line = raw.replace(/\r/g, '').trimEnd()
          if (!line) continue
          if (echo === 'filtered' && NOISE.test(line)) continue
          console.log(`      ${dim(line)}`)
        }
      }
    }
    child.stdout?.on('data', consume)
    child.stderr?.on('data', consume)

    child.on('error', (err) => {
      if (timer) clearTimeout(timer)
      console.log(red('启动失败'))
      console.log(red(`      ${err.message}`))
      resolve({ ok: false, code: null, signal: null, output: output + err.message, timedOut })
    })

    child.on('close', (code, signal) => {
      if (timer) clearTimeout(timer)
      if (settleTimer) clearTimeout(settleTimer)
      const ms = Date.now() - started
      fs.mkdirSync(logDir, { recursive: true })
      fs.writeFileSync(path.join(logDir, `${slug(label)}.log`), output, 'utf8')
      if (timedOut) {
        console.log(yellow(`超时（已 kill，${fmtDuration(ms)}）`))
      } else if (code === 0) {
        console.log(green(`ok (${fmtDuration(ms)})`))
      } else if (settled) {
        // 总结行已出、进程不自退：退出码无意义，**结果由用例数判定**（见调用方）
        console.log(green(`ok (${fmtDuration(ms)}；进程未自退，已收尾)`))
      } else {
        console.log(red(`失败 (exit=${code}${signal ? `, ${signal}` : ''}, ${fmtDuration(ms)})`))
      }
      resolve({ ok: code === 0 || settled, code, signal, output, timedOut, settled })
    })
  })
}

function slug(s) {
  return s.replace(/[^a-zA-Z0-9]+/g, '-').replace(/^-|-$/g, '').toLowerCase() || 'step'
}

/** ANSI 转义序列（子进程带颜色输出时，正则会被转义码打断，必须先剥离） */
const ANSI_RE = /\x1b\[[0-9;]*[A-Za-z]/g
const stripAnsi = (s) => s.replace(ANSI_RE, '')

/** 从输出里抓一个整数（第一个捕获组） */
function grabInt(output, re) {
  const m = stripAnsi(output).match(re)
  return m ? Number(m[1]) : null
}

// ── 阶段 ───────────────────────────────────────────────────────────────
const results = []
function record(stage, label, pass, note) {
  results.push({ stage, label, pass, note })
}

async function stageBackend() {
  stageHeader(1, 4, '后端（cargo）')
  if (!fs.existsSync(path.join(backendDir, 'Cargo.toml'))) {
    console.log(yellow(`  跳过：未找到 ${backendDir}/Cargo.toml`))
    return
  }
  if (FIX) {
    const f = await run({ label: 'cargo fmt --all（--fix）', cmd: 'cargo', args: ['fmt', '--all'], cwd: backendDir })
    record('backend', 'cargo fmt --all（--fix）', f.ok)
  }

  const check = await run({
    label: 'cargo check --tests',
    cmd: 'cargo',
    args: ['check', '--tests'],
    cwd: backendDir,
  })
  record('backend', 'cargo check --tests', check.ok)

  const testArgs = CI ? ['test', '--workspace'] : ['test', '--lib']
  const test = await run({
    label: `cargo ${testArgs.join(' ')}`,
    cmd: 'cargo',
    args: testArgs,
    cwd: backendDir,
  })
  const passed = grabInt(test.output, /test result: ok\. (\d+) passed/)
  if (CI) {
    // CI 跑全量（含集成测试），通过数与 `--lib` 基线不同 ⇒ 只信退出码
    record('backend', `cargo ${testArgs.join(' ')}`, test.ok)
  } else if (passed === null) {
    record('backend', 'cargo test --lib', test.ok, '未能解析通过数')
  } else if (passed < BASELINE.rustTests) {
    record('backend', 'cargo test --lib', false, `通过数 ${passed} < 基线 ${BASELINE.rustTests}（有测试被删或失败）`)
  } else {
    if (passed > BASELINE.rustTests) {
      console.log(yellow(`      ⚠ 通过数 ${passed} > 基线 ${BASELINE.rustTests}：请更新 scripts/gate.mjs 的 BASELINE.rustTests`))
    } else {
      console.log(dim(`      ${passed} passed（基线 ${BASELINE.rustTests}）`))
    }
    record('backend', 'cargo test --lib', test.ok || passed >= BASELINE.rustTests, passed > BASELINE.rustTests ? `通过数 ${passed}（基线待更新）` : '')
  }

  const clippy = await run({
    label: 'cargo clippy --all-targets -- -D warnings',
    cmd: 'cargo',
    args: ['clippy', '--all-targets', '--', '-D', 'warnings'],
    cwd: backendDir,
  })
  if (!clippy.ok && /\[sandbox\]|os error 5/.test(clippy.output)) {
    console.log(yellow('      ⚠ 输出含沙箱拦截字样——那是环境限制，不是 lint 失败；以退出码为准'))
  }
  record('backend', 'cargo clippy', clippy.ok)

  const fmt = await run({
    label: 'cargo fmt --all -- --check',
    cmd: 'cargo',
    args: ['fmt', '--all', '--', '--check'],
    cwd: backendDir,
  })
  if (!fmt.ok) console.log(yellow('      ↳ 格式不符：跑 `node scripts/gate.mjs --fix` 自动格式化'))
  record('backend', 'cargo fmt --check', fmt.ok)

  if (profile) {
    const build = await run({
      label: `cargo build --profile ${profile}`,
      cmd: 'cargo',
      args: ['build', '--profile', profile],
      cwd: backendDir,
    })
    record('backend', `cargo build --profile ${profile}`, build.ok)
  }

  // `cli/` 是**独立 workspace**（仓库根没有 Cargo.toml）：存在就一并检查，
  // 免得手工只跑 `symbio/` 而漏掉它。
  const cliDir = path.join(repoRoot, 'cli')
  if (fs.existsSync(path.join(cliDir, 'Cargo.toml'))) {
    if (FIX) {
      await run({ label: 'cli: cargo fmt --all（--fix）', cmd: 'cargo', args: ['fmt', '--all'], cwd: cliDir })
    }
    for (const [label, args] of [
      ['cli: cargo check --tests', ['check', '--tests']],
      ['cli: cargo clippy --all-targets -- -D warnings', ['clippy', '--all-targets', '--', '-D', 'warnings']],
      ['cli: cargo fmt --all -- --check', ['fmt', '--all', '--', '--check']],
    ]) {
      const r = await run({ label, cmd: 'cargo', args, cwd: cliDir })
      record('backend', label, r.ok)
    }
  }
}

async function stageFrontend() {
  stageHeader(2, 4, '前端（vue-tsc / vitest）')
  const pkg = path.join(frontendDir, 'package.json')
  if (!fs.existsSync(pkg)) {
    console.log(yellow(`  跳过：未找到 ${frontendDir}/package.json`))
    return
  }

  const tsc = await run({
    label: 'vue-tsc --noEmit',
    cmd: process.execPath,
    args: [path.join(frontendDir, 'node_modules', 'vue-tsc', 'bin', 'vue-tsc.js'), '--noEmit', '-p', 'tsconfig.json'],
    cwd: frontendDir,
  })
  record('frontend', 'vue-tsc --noEmit', tsc.ok)

  const vitest = await run({
    label: 'vitest run',
    cmd: process.execPath,
    args: [path.join(frontendDir, 'node_modules', 'vitest', 'vitest.mjs'), 'run'],
    cwd: frontendDir,
    timeoutMs: VITEST_TIMEOUT_MS,
    // vitest 跑完测试**不会自己退出**（已知行为）：总结行一出就收尾，不必干等超时
    settleAfter: { re: /Tests\s+\d+ passed/, delayMs: 1500 },
  })
  const files = grabInt(vitest.output, /Test Files\s+(\d+) passed/)
  const tests = grabInt(vitest.output, /Tests\s+(\d+) passed/)
  const enough =
    files !== null && tests !== null && files >= BASELINE.vitestFiles && tests >= BASELINE.vitestTests

  if (tests !== null) {
    if (tests < BASELINE.vitestTests) {
      record('frontend', 'vitest run', false, `用例数 ${tests} < 基线 ${BASELINE.vitestTests}`)
    } else if (!vitest.ok) {
      // 判据是「用例数没少」：进程不自退 / 与 cargo 并发 / 缓存 EPERM 都长这样
      const why = vitest.settled ? '进程未自退（已收尾）' : '退出码非 0'
      console.log(yellow(`      ⚠ ${why}，但用例数没少（${files} 文件 / ${tests} 用例）——不判失败`))
      if (!vitest.settled) {
        console.log(yellow('        常见原因：与 cargo 并发 / 缓存 EPERM rename / 跑满超时被 SIGTERM'))
      }
      record('frontend', 'vitest run', true, vitest.settled ? '用例数达标' : '疑似假失败（用例数达标）')
    } else {
      if (tests > BASELINE.vitestTests) {
        console.log(yellow(`      ⚠ 用例数 ${tests} > 基线 ${BASELINE.vitestTests}：请更新 scripts/gate.mjs 的 BASELINE.vitestTests`))
      } else {
        console.log(dim(`      ${files} 文件 / ${tests} 用例（基线 ${BASELINE.vitestFiles}/${BASELINE.vitestTests}）`))
      }
      record('frontend', 'vitest run', true)
    }
  } else {
    record('frontend', 'vitest run', vitest.ok && !vitest.timedOut, enough ? '' : '未能解析用例数')
  }
}

async function stageDocs() {
  stageHeader(3, 4, '静态审计')
  for (const name of ['grep-audit', 'style-audit', 'doc-link-audit', 'test-layout-audit']) {
    const r = await run({
      label: `scripts/${name}.mjs`,
      cmd: process.execPath,
      args: [path.join(scriptDir, `${name}.mjs`)],
      cwd: repoRoot,
      echo: 'all',
    })
    record('docs', `${name}`, r.ok)
  }
  console.log(dim('      doc-link-audit 的失效链接若全在 docs/archive/ 属历史形态，可不处理'))
}

async function stageFacts() {
  stageHeader(4, 4, '事实文件（必须最后）')
  if (FIX) {
    const gen = await run({
      label: 'gen-current-facts.mjs（--fix 写入）',
      cmd: process.execPath,
      args: [path.join(scriptDir, 'gen-current-facts.mjs')],
      cwd: repoRoot,
    })
    record('facts', 'gen-current-facts（写入）', gen.ok)
  }
  const chk = await run({
    label: 'gen-current-facts.mjs --check',
    cmd: process.execPath,
    args: [path.join(scriptDir, 'gen-current-facts.mjs'), '--check'],
    cwd: repoRoot,
  })
  if (!chk.ok) console.log(yellow('      ↳ 漂移：跑 `node scripts/gate.mjs --fix` 重新生成'))
  record('facts', 'gen-current-facts --check', chk.ok)
}

// ── 主流程 ─────────────────────────────────────────────────────────────
console.log(bold('══ 门禁 ══'))
console.log(dim(`  仓库根：${repoRoot}`))
console.log(dim(`  模式：${FIX ? '--fix（先格式化 / 重生成，再检查）' : '只读检查'}`))
console.log(dim(`  完整日志：${path.relative(repoRoot, logDir) || '.'}/`))

if (enabled('backend')) await stageBackend()
if (enabled('frontend')) await stageFrontend()
if (enabled('docs')) await stageDocs()
if (enabled('facts')) await stageFacts()

// ── 汇总 ───────────────────────────────────────────────────────────────
const failed = results.filter((r) => !r.pass)
console.log()
console.log(bold('══ 汇总 ══'))
for (const r of results) {
  const mark = r.pass ? green('✓') : red('✗')
  console.log(`  ${mark} ${r.label}${r.note ? yellow(` — ${r.note}`) : ''}`)
}
console.log()
console.log(`  通过 ${results.length - failed.length} / ${results.length}`)

if (failed.length > 0) {
  console.log(red(`  失败 ${failed.length} 项，逐项日志见 ${path.relative(repoRoot, logDir) || '.'}/`))
  process.exit(1)
}
console.log(green('  全部通过'))
process.exit(0)
