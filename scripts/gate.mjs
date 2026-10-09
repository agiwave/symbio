#!/usr/bin/env node
/**
 * gate — 通用门控框架。
 *
 * 本体只做三件事：解析参数、按序执行 `scripts/gate.d/*.mjs` 声明的任务、汇总退出码。
 * 「查什么、怎么判定」全部在 gate.d 任务模块里（`_` 前缀文件是共享库，不是阶段）。
 *
 * 任务模块契约（默认导出）：
 *   { id, title, tasks }   tasks 为任务数组，或 `async (ctx) => 任务数组`
 * 任务形态：
 *   { label, cmd, args?, cwd?, env?, timeoutMs?, echo? }        声明式命令，判定 = 退出码 0
 *   { label, run: async (ctx) => 结果, skipNote? }              自定义判定
 *   { label, when: (ctx) => bool, ... }                          条件不满足记 skipped
 * 结果取值：true / false / 'skipped' / { ok, note }
 *
 * ## 并发：默认串行，`parallel: true` 才并发
 *
 * 任务加 `parallel: true` 表示「与同批其他任务互不干扰」，连续的一批会以
 * `concurrency`（阶段级，缺省取核数-1 且 ≤8）并发执行。**默认串行**是刻意的：
 * 同一 `target/` 目录的 cargo 命令并发只会争 `.cargo-lock`（排成
 * `Blocking waiting for file lock`），收益为零还要多写一轮日志。
 *
 * 适合并发的：只读审计脚本（扫文件）、`rustc` 独立编译、彼此隔离的 e2e 用例。
 * 不适合的：任何 cargo 命令、写同一个输出目录的任务。
 *
 * 阶段级同构：阶段对象同样可以标 `parallel: true`——连续标了的**阶段**合成一批
 * 并发跑，约束与任务级相同：**只有相互无数据依赖的阶段才可标**（每个阶段为什么
 * 安全，写在它自己的文件头，不在本文件复述）。当前编排（2026-10-09）：
 *
 *   05-fmt（屏障，单独跑）→ [10-backend ∥ 30-docs ∥ 35-baseline ∥ 50-msrv]
 *     → 55-verify → 56-frontend → 58-e2e → 60-facts（必须最后）
 *
 * 三个「看着能并发、实际不能」的坑，判据各归其主文件：
 *   - verify 不进批：它的 `gen-verify-facts` 重写 `verify/facts/mod.rs`，而 docs 的
 *     doc-count-audit 读同一个文件（55-verify.mjs）；
 *   - frontend 不与 backend 并发：vite build 重写 `tauri/dist`，而 backend 编译
 *     symbio-tauri 时 `generate_context!` 在**编译期**读它（56-frontend.mjs）；
 *   - e2e 不与任何重负载并发：用例对 CPU 竞争时序敏感（58-e2e.mjs）。
 *
 * 并发批内**不往 stdout 刷逐行输出**（会交错成乱码），详情落各自日志文件，
 * 控制台只留一行结论。
 *
 * ctx：{ repoRoot, scriptDir, logDir, ci, profile, run(o), log(name, text) }
 *   run(o) 执行命令：实时逐行转发（echo: filtered|all|none）+ 全文落日志 + 只信退出码。
 *
 * 参数：
 *   node scripts/gate.mjs                       全量
 *   node scripts/gate.mjs --only=id1,id2        只跑指定阶段
 *   node scripts/gate.mjs --skip=id1,id2        跳过指定阶段
 *   node scripts/gate.mjs --ci                  CI 对齐模式（语义由任务模块自行解释）
 *   node scripts/gate.mjs --profile=<p>         附加构建档位（语义由任务模块自行解释）
 *   node scripts/gate.mjs --list                只列出阶段与任务，不执行
 *   node scripts/gate.mjs --help                打印用法并退出（**不跑任何阶段**）
 *
 * ## 判定 vs 执行：门禁会**做掉**确定性的机械工作
 *
 * 格式化（`cargo fmt`）与事实文件生成（`gen-current-facts`）是**函数**不是判断，
 * 对同一份输入永远给同一个输出。把它们写成「检查你有没有跑过」等于让门禁因为
 * **人忘了按一次按钮**而红——报的不是代码有问题，是流程有问题。因此它们由门禁
 * **自动执行**：命令跑成功即通过，命令本身报错才不通过；本地还会把改写的文件
 * **当场暂存**，使修复与本次提交是同一份内容。见 `gate.d/_shared.autoWork`。
 *
 * （`--fix` 这个开关已删除：它当时的作用就是「允许跑那两个修复动作」，
 * 而它们现在无条件执行。留一个没有任何效果的开关比没有更糟。）
 */

import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { red, green, yellow, dim, bold, stripAnsi } from './color.mjs'

/**
 * 默认并发度（阶段可用 `concurrency` 覆盖）。
 *
 * **只用于「互不干扰」的任务**：同一 `target/` 目录的 cargo 命令**必须串行**
 * （cargo 自己会争 `.cargo-lock`，并发跑只会排成 `Blocking waiting for file
 * lock`，白等一轮还扰乱日志）。因此并发是**按任务声明**（`parallel: true`）的，
 * 不是全局默认——没标的照旧一个一个跑。
 *
 * 上限取 `核数 - 1` 且不超过 8：审计类脚本是 IO 密集（扫文件），给满核会与
 * 主进程和其他任务抢调度；`-1` 留一个核给 spawn / 日志落盘。
 */
const CONCURRENCY = Math.max(2, Math.min((os.availableParallelism?.() ?? os.cpus().length) - 1, 8))

// realpathSync.**native** 规范化盘符大小写：启动 cwd 可能以小写盘符传入（会话
// 环境实测 `d:\...`；JS 版 realpathSync 与 libuv 一致地**保留**输入大小写，只有
// native 版走 GetFinalPathNameByHandle 返回 NTFS 真实大小写）。小写路径若一路
// 传给子任务，vitest v4 的 root 与模块真实路径大小写不一致 ⇒ 同一模块被当两个
// 副本，worker 单例失效，全部 spec 报「failed to find the runner」——2026-09-28 实测。
const scriptDir = fs.realpathSync.native(path.dirname(fileURLToPath(import.meta.url)))
const repoRoot = path.resolve(scriptDir, '..')
const logDir = path.join(repoRoot, '.workbuddy-ai', 'gate-logs')

const argv = process.argv.slice(2)
const hasFlag = (n) => argv.includes(n)
const valOf = (p) => {
  const a = argv.find((x) => x.startsWith(p))
  return a ? a.slice(p.length).trim() : null
}
const CI = hasFlag('--ci')
const profile = valOf('--profile=')
const only = valOf('--only=') ? valOf('--only=').split(',').map((s) => s.trim()).filter(Boolean) : null
const skip = valOf('--skip=') ? valOf('--skip=').split(',').map((s) => s.trim()).filter(Boolean) : []
const listOnly = hasFlag('--list')

// `--help` / `-h`：**在任何副作用之前**退出。
//
// 补它的理由：脚本原先不认 `--help`，于是 `node scripts/gate.mjs --help` 会**真的跑一遍
// 全量门禁**（10+ 分钟）并把 `.workbuddy-ai/gate-logs/*.log` 覆盖掉——想查用法，代价是
// 丢掉上一次的失败日志（最需要它的那一刻它没了，只能重跑）。与 `commit.mjs` 的 `--help`
// 同类陷阱（那里更贵：会真的提交一次）。判据 = `gate.test.mjs`：退出 0、打印用法、**不跑任何阶段**。
if (hasFlag('--help') || hasFlag('-h')) {
  console.log(`用法：node scripts/gate.mjs [选项]

  （无参数）            跑全部阶段
  --only=id1,id2        只跑指定阶段（id 见 --list）
  --skip=id1,id2        跳过指定阶段
  --ci                  CI 对齐模式（语义由各任务模块自行解释）
  --profile=<p>         附加构建档位（语义由各任务模块自行解释）
  --list                只列出阶段与任务，不执行
  --help, -h            打印本用法并退出（**不跑任何阶段**）

  查用法一律用 --help：本脚本只认已识别的开关，未知参数不会报错——--help 被当未知
  参数时会照常跑完全量门禁并覆盖日志。`)
  process.exit(0)
}

const NOISE =
  /^(?:\s*$|.*\r$|\s*(Compiling|Checking|Downloading|Downloaded|Updating|Locking|Adding|Removing|Finished|Blocking|Waiting|Fresh|Documenting|Building)\b)/

const slug = (s) => s.replace(/[^a-zA-Z0-9]+/g, '-').replace(/^-|-$/g, '').toLowerCase() || 'step'

const fmtDuration = (ms) => {
  const s = Math.round(ms / 1000)
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`
}

function run(o) {
  const { label, cmd, args = [], cwd = repoRoot, timeoutMs = 0, echo = 'filtered', env, quiet = false } = o
  // `quiet`：并发批里**不往 stdout 写任何东西**——多个任务同时 echo 会交错成
  // 不可读的乱码（`▸ a … ▸ b … ok (1s) ok (2s)`）。详情仍完整落在各自的日志
  // 文件里，批跑完由调用方按原顺序打印一行结论。单任务（非并行）行为不变。
  if (!quiet) process.stdout.write(`  ▸ ${label} … `)

  return new Promise((resolve) => {
    const started = Date.now()
    let output = ''
    let timedOut = false

    // **流式落盘**：边跑边写，进程被 kill / 超时 / Ctrl+C 时该任务的日志也已完整。
    // 早先只在 close 时一次性写，于是「跑挂了」恰恰等于「没有日志」——最需要它的
    // 那一刻它不存在（只能重跑一遍）。
    const logFile = path.join(logDir, `${slug(label)}.log`)
    fs.mkdirSync(logDir, { recursive: true })
    const sink = fs.createWriteStream(logFile, { flags: 'w' })

    const child = spawn(cmd, args, { cwd, shell: false, env: { ...process.env, ...env } })
    let timer = null
    if (timeoutMs > 0) {
      timer = setTimeout(() => {
        timedOut = true
        child.kill('SIGKILL')
      }, timeoutMs)
    }

    const consume = (chunk) => {
      const text = chunk.toString()
      output += text
      sink.write(text)
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
      if (!quiet) {
        console.log(red('启动失败'))
        console.log(red(`      ${err.message}`))
      }
      sink.end(err.message)
      resolve({
        ok: false,
        code: null,
        signal: null,
        output: output + err.message,
        timedOut,
        logFile,
        ms: Date.now() - started,
      })
    })

    child.on('close', (code, signal) => {
      if (timer) clearTimeout(timer)
      const ok = code === 0 && signal === null && !timedOut
      const ms = Date.now() - started
      sink.end()
      if (!quiet) {
        if (timedOut) console.log(yellow(`超时（已 kill，${fmtDuration(ms)}）`))
        else if (ok) console.log(green(`ok (${fmtDuration(ms)})`))
        else console.log(red(`失败 (exit=${code}${signal ? `, ${signal}` : ''}, ${fmtDuration(ms)})`))
      }
      resolve({ ok, code, signal, output, timedOut, logFile, ms })
    })
  })
}

const ctx = {
  repoRoot,
  scriptDir,
  logDir,
  ci: CI,
  profile,
  run,
  /** 手写日志（自定义任务自己产出的文本）。返回落盘路径，便于结果里带回去。 */
  log(name, text) {
    fs.mkdirSync(logDir, { recursive: true })
    const file = path.join(logDir, `${slug(name)}.log`)
    fs.writeFileSync(file, text, 'utf8')
    return file
  },
}

const gateDir = path.join(scriptDir, 'gate.d')
const stages = []
for (const f of fs.readdirSync(gateDir).filter((f) => f.endsWith('.mjs') && !f.startsWith('_')).sort()) {
  const mod = await import(pathToFileURL(path.join(gateDir, f)).href)
  if (mod.default?.id) stages.push(mod.default)
}

const enabled = (id) => (only ? only.includes(id) : true) && !skip.includes(id)

// 结果按**阶段分组缓冲**，report 时按阶段顺序摊平。并发泳道下各阶段的完成时刻
// 交错，直接 push 会让汇总变成「谁快谁在前」；分组缓冲让中途（SIGINT）的汇总
// 也保持「按阶段归组、组内按任务序」。
const stageBuffers = new Map()
function bufferOf(stageId) {
  let buf = stageBuffers.get(stageId)
  if (!buf) stageBuffers.set(stageId, (buf = []))
  return buf
}
// 每条结果都记下自己的日志文件（可能为 null：跳过项没跑命令），失败汇总据此
// 直接列出**这一项的文件名**——早先只给目录，68 个文件里要自己猜是哪个。
const record = (stageId, r) => bufferOf(stageId).push(r)
const collectedResults = () =>
  stages.filter((s) => stageBuffers.has(s.id)).flatMap((s) => stageBuffers.get(s.id))

function normalize(outcome) {
  if (outcome === 'skipped') return { pass: 'skipped', note: '' }
  if (outcome == null || outcome === true) return { pass: true, note: '' }
  if (typeof outcome === 'boolean') return { pass: outcome, note: '' }
  return { pass: outcome.ok !== false, note: outcome.note ?? '', logFile: outcome.logFile ?? null }
}

/**
 * 执行单个任务并**返回**结果（由调用方经 `record` 落入所属阶段的缓冲）——并发批要按**原顺序**
 * 落结果，只能由调用方统一 `record`。`quiet` 透传给 `run`（见其说明）。
 */
async function executeTask(stageId, t, { quiet = false } = {}) {
  if (t.when && !t.when(ctx)) {
    return {
      stage: stageId,
      label: t.label,
      pass: 'skipped',
      note: t.skipNote ?? '条件不满足',
      logFile: null,
      ms: null,
    }
  }
  if (t.cmd) {
    const r = await run({ ...t, env: t.env, quiet })
    return {
      stage: stageId,
      label: t.label,
      pass: r.ok,
      note: r.timedOut ? '超时终止' : r.ok ? '' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`,
      logFile: r.logFile,
      ms: r.ms,
    }
  }
  const started = Date.now()
  const outcome = normalize(await t.run(ctx))
  return {
    stage: stageId,
    label: t.label,
    pass: outcome.pass,
    note: outcome.note,
    logFile: outcome.logFile ?? null,
    ms: Date.now() - started,
  }
}

function stageHeader(index, title) {
  console.log()
  console.log(bold(`── 阶段 ${index}/${stages.length} · ${title} ${'─'.repeat(Math.max(0, 44 - title.length))}`))
}

const materialize = (s) => Array.from(typeof s.tasks === 'function' ? s.tasks(ctx) : s.tasks)

if (listOnly) {
  for (const [i, s] of stages.entries()) {
    const tasks = materialize(s)
    console.log(`${i + 1}. ${bold(s.id)} — ${s.title}（${tasks.length} 项）`)
    for (const t of tasks) console.log(`   · ${t.label}`)
  }
  process.exit(0)
}

console.log(bold('══ 门禁 ══'))
console.log(dim(`  仓库根：${repoRoot}`))
console.log(
  dim(
    `  模式：${
      CI
        ? '--ci（不能提交 ⇒ 自动执行后有差异即报红）'
        : '本地（自动执行 fmt / 事实文件生成，并把改写当场暂存）'
    }${profile ? ` · --profile=${profile}` : ''}`,
  ),
)
console.log(dim(`  阶段：${stages.filter((s) => enabled(s.id)).map((s) => s.id).join(' → ')}`))
const relLog = (path.relative(repoRoot, logDir) || '.').replace(/\\/g, '/')
console.log(dim(`  完整日志：${relLog}/（每次运行都写；失败项下方会直接列出文件名）`))

/**
 * 并发跑一批任务（连续标记 `parallel: true` 的那些），并发度 `limit`。
 *
 * **结果按原顺序落**：`out` 预分配 + worker 从共享游标取任务，因此汇总表的顺序
 * 恒等于任务声明顺序，不受完成先后影响（否则「谁快谁排前面」，两次运行的日志
 * 顺序都不同，对比就白费力气）。
 *
 * 批内强制 `quiet`：多个子进程同时往 stdout 写会交错成乱码，详情改由各自日志
 * 承载（失败时汇总会直接列出文件名），控制台只保留一行结论。
 */
async function runConcurrent(stageId, tasks, limit) {
  const out = new Array(tasks.length)
  let cursor = 0
  const worker = async () => {
    for (;;) {
      const idx = cursor++
      if (idx >= tasks.length) return
      out[idx] = await executeTask(stageId, tasks[idx], { quiet: true })
    }
  }
  await Promise.all(Array.from({ length: Math.min(limit, tasks.length) }, worker))

  for (const r of out) {
    record(stageId, r)
    const mark = r.pass === true ? green('✓') : r.pass === 'skipped' ? yellow('⊘') : red('✗')
    const took = r.ms != null ? dim(` (${fmtDuration(r.ms)})`) : ''
    console.log(`  ${mark} ${r.label}${took}${r.note ? yellow(` — ${r.note}`) : ''}`)
  }
}

async function runStage(s, index) {
  stageHeader(index + 1, s.title)
  const tasks = materialize(s)
  const limit = s.concurrency ?? CONCURRENCY
  let i = 0
  while (i < tasks.length) {
    // 连续标记 `parallel` 的任务合成一批并发跑；其余照旧串行——cargo 命令
    // **必须**串行（同 target 目录会争 `.cargo-lock`，见 CONCURRENCY 处说明）。
    if (tasks[i].parallel) {
      const batch = []
      while (i < tasks.length && tasks[i].parallel) batch.push(tasks[i++])
      await runConcurrent(s.id, batch, limit)
    } else {
      record(s.id, await executeTask(s.id, tasks[i++]))
    }
  }
}

/**
 * 并发跑一批「相互无数据依赖」的阶段（连续标 `parallel: true` 的那些）。
 * 与任务级批同构：worker 从共享游标取泳道。并发度沿用 `CONCURRENCY`
 * （CI 的 4 核 runner 收到 3；本地 20 核足以让整批同时开跑）。
 *
 * 控制台输出是**行级交错**：所有输出都经主进程的单事件循环，每次
 * `console.log` 是一整行、不会撕裂，各任务行自带标签可以辨认归属；
 * 汇总不交错——`report` 按阶段顺序摊平（见 `stageBuffers`）。
 */
async function runStageBatch(batch) {
  const limit = Math.min(batch.length, Math.max(2, CONCURRENCY))
  let cursor = 0
  const worker = async () => {
    for (;;) {
      const i = cursor++
      if (i >= batch.length) return
      await runStage(batch[i].s, batch[i].idx)
    }
  }
  await Promise.all(Array.from({ length: limit }, worker))
}

const gateStarted = Date.now()
// 泳道调度：连续标 `parallel` 的阶段合成一批并发，其余照旧串行。
// `parallel` 语义见文件头「阶段级同构」一节；哪些阶段标了、为什么（以及为什么
// 某些阶段**不能**标），判据在各阶段自己的文件头。
const lanes = stages.map((s, idx) => ({ s, idx })).filter(({ s }) => enabled(s.id))
for (let i = 0; i < lanes.length; ) {
  const lane = lanes[i]
  if (lane.s.parallel) {
    const batch = []
    while (i < lanes.length && lanes[i].s.parallel) batch.push(lanes[i++])
    await runStageBatch(batch)
  } else {
    await runStage(lane.s, lane.idx)
    i++
  }
}

/** 汇总并给退出码。**中断时也走这里**（见下方 SIGINT/SIGTERM）——已跑完的结果、
 *  各自日志文件、失败归属都不该因为按了一次 Ctrl+C 就丢掉。 */
function report(interrupted = false) {
  const results = collectedResults()
  const failed = results.filter((r) => r.pass === false)
  const skipped = results.filter((r) => r.pass === 'skipped')
  const passed = results.length - failed.length - skipped.length

  // 运行清单落盘：终端的滚动缓冲会丢，文件不会。**中断路径也写**——正是那条
  // 路径最可能只剩这一个物证。逐项带日志文件名，供事后核对。
  try {
    const lines = [
      `# 门禁运行清单${interrupted ? '（中断，未跑完）' : ''}`,
      `时间：${new Date().toISOString()}`,
      `结论：通过 ${passed} / ${results.length}${skipped.length ? `（另有 ${skipped.length} 项未判定）` : ''}`,
      '',
      ...results.map((r) => {
        const mark = r.pass === true ? '✓' : r.pass === 'skipped' ? '⊘' : '✗'
        // 耗时进清单：瓶颈定位只看这一个文件就够，不必重跑（控制台会滚动丢失）。
        const took = r.ms != null ? ` (${fmtDuration(r.ms)})` : ''
        const list = Array.isArray(r.logFile) ? r.logFile : r.logFile ? [r.logFile] : []
        const logs = list.map((f) => path.relative(repoRoot, f).replace(/\\/g, '/')).join(', ')
        return `${mark} [${r.stage}] ${r.label}${took}${r.note ? ` — ${r.note}` : ''}${logs ? `\n    日志：${logs}` : ''}`
      }),
      '',
    ]
    fs.writeFileSync(path.join(logDir, '_summary.md'), lines.join('\n'), 'utf8')
  } catch {
    /* 清单只是副产品：写不进去也不该改变门禁结论 */
  }

  console.log()
  console.log(bold('══ 汇总 ══'))
  for (const r of results) {
    const mark = r.pass === true ? green('✓') : r.pass === 'skipped' ? yellow('⊘') : red('✗')
    const took = r.ms != null ? dim(` (${fmtDuration(r.ms)})`) : ''
    console.log(`  ${mark} ${r.label}${took}${r.note ? yellow(` — ${r.note}`) : ''}`)
  }
  console.log()
  console.log(
    `  通过 ${passed} / ${results.length}${skipped.length ? `（另有 ${skipped.length} 项未判定）` : ''}${interrupted ? '（中断，未跑完）' : ''}`,
  )
  console.log(dim(`  总耗时 ${fmtDuration(Date.now() - gateStarted)}`))

  // 失败逐项直接给**文件**（早先只给目录，68 个日志里得自己猜）；聚合任务可以
  // 一次带回多个子日志（数组），没跑上命令的项（自己的断言失败、非子进程）如实说明。
  for (const r of failed) {
    const list = Array.isArray(r.logFile) ? r.logFile : r.logFile ? [r.logFile] : []
    if (list.length === 0) {
      console.log(red(`  · ${r.label} → （无子进程日志，见上方输出）`))
      continue
    }
    for (const f of list) {
      console.log(red(`  · ${r.label} → ${path.relative(repoRoot, f).replace(/\\/g, '/')}`))
    }
  }

  console.log(dim(`  本次运行日志目录：${path.relative(repoRoot, logDir).replace(/\\/g, '/') || '.'}/`))
  if (interrupted) {
    console.log(yellow('  已中断——以上为已跑完部分；本次运行日志已落盘，无需重跑即可查看。'))
    process.exit(130)
  }
  if (failed.length > 0) {
    console.log(red(`  失败 ${failed.length} 项。`))
    process.exit(1)
  }
  console.log(green('  全部通过'))
  process.exit(0)
}

// 中断兜底：Ctrl+C / 被 kill 时把已有结果与日志归属打出来，别让人重跑一遍才看得到。
for (const sig of ['SIGINT', 'SIGTERM']) {
  process.on(sig, () => {
    console.log(yellow(`\n  收到 ${sig}，收尾中…`))
    report(true)
  })
}

report()
