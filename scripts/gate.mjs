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
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { red, green, yellow, dim, bold, stripAnsi } from './color.mjs'

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
  const { label, cmd, args = [], cwd = repoRoot, timeoutMs = 0, echo = 'filtered', env } = o
  process.stdout.write(`  ▸ ${label} … `)

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
      console.log(red('启动失败'))
      console.log(red(`      ${err.message}`))
      sink.end(err.message)
      resolve({ ok: false, code: null, signal: null, output: output + err.message, timedOut, logFile })
    })

    child.on('close', (code, signal) => {
      if (timer) clearTimeout(timer)
      const ok = code === 0 && signal === null && !timedOut
      const ms = Date.now() - started
      sink.end()
      if (timedOut) console.log(yellow(`超时（已 kill，${fmtDuration(ms)}）`))
      else if (ok) console.log(green(`ok (${fmtDuration(ms)})`))
      else console.log(red(`失败 (exit=${code}${signal ? `, ${signal}` : ''}, ${fmtDuration(ms)})`))
      resolve({ ok, code, signal, output, timedOut, logFile })
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

const results = []
// 每条结果都记下自己的日志文件（可能为 null：跳过项没跑命令），失败汇总据此
// 直接列出**这一项的文件名**——早先只给目录，68 个文件里要自己猜是哪个。
const record = (stage, label, pass, note, logFile = null) =>
  results.push({ stage, label, pass, note, logFile })

function normalize(outcome) {
  if (outcome === 'skipped') return { pass: 'skipped', note: '' }
  if (outcome == null || outcome === true) return { pass: true, note: '' }
  if (typeof outcome === 'boolean') return { pass: outcome, note: '' }
  return { pass: outcome.ok !== false, note: outcome.note ?? '', logFile: outcome.logFile ?? null }
}

async function executeTask(stageId, t) {
  if (t.when && !t.when(ctx)) {
    record(stageId, t.label, 'skipped', t.skipNote ?? '条件不满足')
    return
  }
  if (t.cmd) {
    const r = await run({ ...t, env: t.env })
    record(
      stageId,
      t.label,
      r.ok,
      r.timedOut ? '超时终止' : r.ok ? '' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`,
      r.logFile,
    )
    return
  }
  const outcome = normalize(await t.run(ctx))
  record(stageId, t.label, outcome.pass, outcome.note, outcome.logFile ?? null)
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

for (const [i, s] of stages.entries()) {
  if (!enabled(s.id)) continue
  stageHeader(i + 1, s.title)
  for (const t of materialize(s)) await executeTask(s.id, t)
}

/** 汇总并给退出码。**中断时也走这里**（见下方 SIGINT/SIGTERM）——已跑完的结果、
 *  各自日志文件、失败归属都不该因为按了一次 Ctrl+C 就丢掉。 */
function report(interrupted = false) {
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
        const list = Array.isArray(r.logFile) ? r.logFile : r.logFile ? [r.logFile] : []
        const logs = list.map((f) => path.relative(repoRoot, f).replace(/\\/g, '/')).join(', ')
        return `${mark} [${r.stage}] ${r.label}${r.note ? ` — ${r.note}` : ''}${logs ? `\n    日志：${logs}` : ''}`
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
    console.log(`  ${mark} ${r.label}${r.note ? yellow(` — ${r.note}`) : ''}`)
  }
  console.log()
  console.log(
    `  通过 ${passed} / ${results.length}${skipped.length ? `（另有 ${skipped.length} 项未判定）` : ''}${interrupted ? '（中断，未跑完）' : ''}`,
  )

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
