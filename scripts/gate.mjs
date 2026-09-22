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

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
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
      const ok = code === 0 && signal === null && !timedOut
      const ms = Date.now() - started
      fs.mkdirSync(logDir, { recursive: true })
      fs.writeFileSync(path.join(logDir, `${slug(label)}.log`), output, 'utf8')
      if (timedOut) console.log(yellow(`超时（已 kill，${fmtDuration(ms)}）`))
      else if (ok) console.log(green(`ok (${fmtDuration(ms)})`))
      else console.log(red(`失败 (exit=${code}${signal ? `, ${signal}` : ''}, ${fmtDuration(ms)})`))
      resolve({ ok, code, signal, output, timedOut })
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
  log(name, text) {
    fs.mkdirSync(logDir, { recursive: true })
    fs.writeFileSync(path.join(logDir, `${slug(name)}.log`), text, 'utf8')
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
const record = (stage, label, pass, note) => results.push({ stage, label, pass, note })

function normalize(outcome) {
  if (outcome === 'skipped') return { pass: 'skipped', note: '' }
  if (outcome == null || outcome === true) return { pass: true, note: '' }
  if (typeof outcome === 'boolean') return { pass: outcome, note: '' }
  return { pass: outcome.ok !== false, note: outcome.note ?? '' }
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
    )
    return
  }
  const outcome = normalize(await t.run(ctx))
  record(stageId, t.label, outcome.pass, outcome.note)
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
console.log(dim(`  完整日志：${path.relative(repoRoot, logDir) || '.'}/`))

for (const [i, s] of stages.entries()) {
  if (!enabled(s.id)) continue
  stageHeader(i + 1, s.title)
  for (const t of materialize(s)) await executeTask(s.id, t)
}

const failed = results.filter((r) => r.pass === false)
const skipped = results.filter((r) => r.pass === 'skipped')
console.log()
console.log(bold('══ 汇总 ══'))
for (const r of results) {
  const mark = r.pass === true ? green('✓') : r.pass === 'skipped' ? yellow('⊘') : red('✗')
  console.log(`  ${mark} ${r.label}${r.note ? yellow(` — ${r.note}`) : ''}`)
}
console.log()
console.log(
  `  通过 ${results.length - failed.length - skipped.length} / ${results.length}${skipped.length ? `（另有 ${skipped.length} 项未判定）` : ''}`,
)

if (failed.length > 0) {
  console.log(red(`  失败 ${failed.length} 项，逐项日志见 ${path.relative(repoRoot, logDir) || '.'}/`))
  process.exit(1)
}
console.log(green('  全部通过'))
process.exit(0)
