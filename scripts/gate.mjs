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
 * 安全，写在它自己的文件头，不在本文件复述）。当前编排（2026-10-18）：
 *
 *   05-fmt（屏障，单独跑）
 *     → 06-facts-write（生成器，串行；docs 读它的产物，故必须是批的前置）
 *     → [10-backend ∥ 20-frontend-static ∥ 30-docs ∥ 35-baseline ∥ 50-msrv]
 *     → 55-verify → 56-frontend-build → 58-e2e → 60-facts（必须最后）
 *
 * ⚠️ **批只由「连续」的 `parallel: true` 阶段组成**：中间夹一个没标 `parallel` 的阶段，
 * 批就在那里断开，两侧各自成批、先后串行。这不是理论风险——`facts-write` 曾叫 `28-`，
 * 正夹在 `20-` 与 `30-` 之间，于是 docs 整段（6m25s）被串行在 backend（9m35s）之后，
 * 而本文件当时写的编排是「一整批」。**加阶段时先看它落在哪两个阶段之间**，再看它
 * 该不该标 `parallel`；两件事都不做，默认结果就是「悄悄地多串行一整段」。
 *
 * 三个「看着能并发、实际不能」的坑，判据各归其主文件：
 *   - verify 不进批：它的 `gen-verify-facts` 重写 `verify/facts/mod.rs`，而 docs 的
 *     doc-count-audit 读同一个文件（55-verify.mjs）；
 *   - frontend-build 不与 backend 并发：vite build 重写 `tauri/dist`，而 backend 编译
 *     symbio-tauri 时 `generate_context!` 在**编译期**读它（56-frontend-build.mjs）。
 *     前端另外三个任务**不碰 `dist`**，故拆到 `20-frontend-static.mjs` 进批——那是
 *     同一个阶段的另一半，拆开只是为了让不冲突的那一半不再串行；
 *   - e2e 不与任何重负载并发：用例对 CPU 竞争时序敏感（58-e2e.mjs）。
 *
 * 并发批内**不往 stdout 刷逐行输出**（会交错成乱码），详情落各自日志文件，
 * 控制台只留一行结论。**但失败不放过**：汇总的「失败详情」段把每个失败项日志的
 * 末 N 行印在失败处（`GATE_FAIL_TAIL` 调行数），路径一并给出——失败处的信息量与
 * 文件里的信息量之间，不该隔一次七分钟的重跑。
 *
 * ctx：{ repoRoot, scriptDir, logDir, ci, profile, run(o), log(name, text) }
 *   run(o) 执行命令：实时逐行转发（echo: filtered|all|none）+ 全文落日志 + 只信退出码。
 *   logDir = **本轮**的日志目录（`.workbuddy-ai/gate-logs/<runId>/`；每轮一个，
 *     保留最近 KEEP_RUNS 轮，见 gate.d/_logs.mjs）。
 *
 * 参数：
 *   node scripts/gate.mjs                       全量
 *   node scripts/gate.mjs --only=id1,id2        只跑指定阶段
 *   node scripts/gate.mjs --skip=id1,id2        跳过指定阶段
 *   node scripts/gate.mjs --ci                  CI 对齐模式（语义由任务模块自行解释）
 *   node scripts/gate.mjs --profile=<p>         附加构建档位（语义由任务模块自行解释）
 *   node scripts/gate.mjs --base=<ref>          棘轮基准版本（只对 baseline 阶段有意义）
 *   node scripts/gate.mjs --list                只列出阶段与任务，不执行
 *   node scripts/gate.mjs --help                打印用法并退出（**不跑任何阶段**）
 *
 * 参数是**白名单**：未识别的参数、`--only=` 的空清单、`--only` / `--skip` 里不存在的
 * 阶段 id 一律**拒绝**（退出码 2，用法见 `--help`）。理由：本脚本的默认动作是全量门禁
 * （10+ 分钟）——「未知参数不报错、照常跑默认动作」会让人以为只跑了一个阶段，白等七
 * 分钟；而拼错的阶段 id 更糟：它让 **0 个阶段**「全部通过」退出 0（静默假绿）。
 *
 * 从**别的仓库**调用会被**拒绝**（退出码 2）：门禁的每个阶段都以脚本所在的仓库为准，
 * 从别处调用会静默地门禁另一个仓库（写它的 target/、在它的日志目录里落一轮、替它 git add）。
 * 判据见下方「仓库身份」一节。
 *
 * ## 判定 vs 执行：门禁会**做掉**确定性的机械工作
 *
 * 格式化（`cargo fmt`）与事实文件生成（`facts/current`）是**函数**不是判断，
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
import { spawn, spawnSync } from 'node:child_process'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { red, green, yellow, dim, bold, stripAnsi } from './color.mjs'
import { machine, describe, budget, recordRun, measuredMs, setBatchLanes } from './gate.d/_machine.mjs'
import {
  LOGS_DIR_NAME,
  KEEP_RUNS,
  FAIL_TAIL_LINES,
  makeRunId,
  pruneOldRuns,
  writeRunPointer,
  logFileName,
  failureDetail,
} from './gate.d/_logs.mjs'

/**
 * 默认并发度（阶段可用 `concurrency` 覆盖）。**由机器档案决定，不是常量**。
 *
 * **只用于「互不干扰」的任务**：同一 `target/` 目录的 cargo 命令**必须串行**
 * （cargo 自己会争 `.cargo-lock`，并发跑只会排成 `Blocking waiting for file
 * lock`，白等一轮还扰乱日志）。因此并发是**按任务声明**（`parallel: true`）的，
 * 不是全局默认——没标的照旧一个一个跑。
 *
 * 这里的 `7` 只是「`machine()` 还没算出来之前」的占位值（`--list` 之类不跑任务
 * 的路径用不到它）。真正的值在 banner 之前由 `applyMachine()` 覆盖成
 * `slots.io`——任务级并发的绝大多数是审计脚本与回归测试，那是 IO 密集，
 * 超订有益（见 `_machine.mjs::derive`）。CPU 密集的阶段自己声明 `concurrency`。
 *
 * ⚠️ **不要在这里写死一个数**。上一版写的是 `min(核数-1, 8)`，看上去像自适应，
 * 其实上限 8 是「当时那台机器」的核数，换到 16/32 核机器上就是白丢一半并行度。
 */
let CONCURRENCY = 7

/** 机器档案；在 banner 处 `await machine()` 后填充。 */
let MACHINE = null

// realpathSync.**native** 规范化盘符大小写：启动 cwd 可能以小写盘符传入（会话
// 环境实测 `d:\...`；JS 版 realpathSync 与 libuv 一致地**保留**输入大小写，只有
// native 版走 GetFinalPathNameByHandle 返回 NTFS 真实大小写）。小写路径若一路
// 传给子任务，vitest v4 的 root 与模块真实路径大小写不一致 ⇒ 同一模块被当两个
// 副本，worker 单例失效，全部 spec 报「failed to find the runner」——2026-09-28 实测。
const scriptDir = fs.realpathSync.native(path.dirname(fileURLToPath(import.meta.url)))
const repoRoot = path.resolve(scriptDir, '..')

// ── 日志落点：**每轮一个目录** ────────────────────────────────────────────
//
// 早先一律写 `.workbuddy-ai/gate-logs/<任务名>.log`，每轮以 `w` 覆盖。后果不是
// 「旧日志没了」这么轻：失败之后**再跑一次门禁就会把失败证据冲掉**，而门禁一轮
// 7 分钟——「再跑一遍才能看到底哪错了」于是成了这里最贵的动作，且常常白跑
// （下一次跑的时候那个错误已经不出现 / 已经被覆盖）。
//
// 现在每轮一个 `<runId>` 目录，根下只留一份 `latest-run.txt` 指针（门外读者
// 定位用）。只保留最近 `KEEP_RUNS` 轮：失败可以活到下一轮，但不无限堆积。
//
// ## 为什么目录**懒建**（第一次真要写日志时才建）
//
// 早先这四行在模块作用域就跑，于是 `gate.mjs --help`、被拒绝的未知参数、
// 跨仓库拒绝——**每一次启动都留下一个空目录**。空目录不只是脏：门内的回归测试
// 会 spawn 门禁（`--help` / `--list` / 拼错 id），于是一次测试跑出一串空目录，
// 而「哪个目录有日志」从此不能靠列目录回答。懒建之后：没有落笔就没有目录。
//
// ## 为什么 mkdir 要带 EEXIST 重试
//
// `makeRunId` 的查重与建目录不是原子的：同一秒内两个门禁进程（门内测试 spawn 的
// 那个与正在跑的门禁本身）会拿到同一个 id。`fs.mkdirSync` 不带 recursive 时
// EEXIST 是**原子**信号，据此换后缀即可；若用 `recursive: true` 则永远不报错，
// 两个进程会写进同一个目录、日志互相覆盖——正是本轮改掉的那个病，换了个形态回来。
const logRoot = path.join(repoRoot, '.workbuddy-ai', LOGS_DIR_NAME)
const runId = makeRunId(new Date())
let logDir = path.join(logRoot, runId)
let ensured = false
/** 确保本轮日志目录存在并写下指针；返回该目录（同一轮内是稳定的）。 */
function ensureLogDir() {
  if (ensured) return logDir
  fs.mkdirSync(logRoot, { recursive: true })
  let id = runId
  for (let n = 2; ; n++) {
    try {
      fs.mkdirSync(path.join(logRoot, id))
      break
    } catch (e) {
      if (e?.code !== 'EEXIST') throw e
      id = `${runId}-${n}`
    }
  }
  logDir = path.join(logRoot, id)
  writeRunPointer(logRoot, id)
  pruneOldRuns(logRoot)
  ensured = true
  return logDir
}

const argv = process.argv.slice(2)
const hasFlag = (n) => argv.includes(n)
const valOf = (p) => {
  const a = argv.find((x) => x.startsWith(p))
  return a ? a.slice(p.length).trim() : null
}
const splitIds = (s) => s.split(',').map((x) => x.trim()).filter(Boolean)
const CI = hasFlag('--ci')
const profile = valOf('--profile=')
// `--only=` 给空值时解析成 `[]`（而不是 null）——**空清单与「没给开关」是两件事**，
// 前者是「一个阶段都不要跑」，必须被下面的校验拦下，不能静默退化成「跑全部」。
const onlyRaw = valOf('--only=')
const skipRaw = valOf('--skip=')
const only = onlyRaw === null ? null : splitIds(onlyRaw)
const skip = skipRaw === null ? [] : splitIds(skipRaw)
const listOnly = hasFlag('--list')

const USAGE = `用法：node scripts/gate.mjs [选项]

  （无参数）            跑全部阶段
  --only=id1,id2        只跑指定阶段（id 见 --list；拼错会报错，不会静默跑 0 个阶段）
  --skip=id1,id2        跳过指定阶段（id 同上）
  --ci                  CI 对齐模式（语义由各任务模块自行解释）
  --profile=<p>         附加构建档位（语义由各任务模块自行解释）
  --base=<ref>          棘轮基准版本（只对 baseline 阶段有意义；缺省取最近触碰该落点的提交）
  --list                只列出阶段与任务，不执行
  --help, -h            打印本用法并退出（**不跑任何阶段**）

  查用法一律用 --help。本脚本的默认动作是**全量门禁**（10+ 分钟），所以
  **未识别的参数一律拒绝**（退出码 2）：在默认动作昂贵的脚本上「未知参数不报错、
  照常跑默认动作」是最坏的组合——--only backend（漏了 =）会静默变成一次全量门禁，
  白等七分钟还以为只跑了一个阶段。

  从**别的仓库**调用会被拒绝（退出码 2）：门禁的每个阶段都以本脚本所在的仓库为准，
  从别处调用会静默门禁另一个仓库。要门禁本仓库就 cd 到它下面。`

// `--help` / `-h`：**在任何副作用之前**退出。
//
// 补它的理由：脚本原先不认 `--help`，于是 `node scripts/gate.mjs --help` 会**真的跑一遍
// 全量门禁**（10+ 分钟）——想查用法，代价是七分钟。与 `commit.mjs` 的 `--help`
// 同类陷阱（那里更贵：会真的提交一次）。判据 = `gate.test.mjs`：退出 0、打印用法、**不跑任何阶段**。
if (hasFlag('--help') || hasFlag('-h')) {
  console.log(USAGE)
  process.exit(0)
}

// ── 参数白名单：**未识别的参数一律拒绝** ────────────────────────────────────
//
// 这个脚本的默认动作是**全量门禁**（10+ 分钟）。在这样一个脚本上「未知参数不报错、
// 照常跑默认动作」是最坏的组合：`--only backend`（漏了 `=`）或 `--fix`（早已删除的
// 开关）都会静默变成一次全量门禁——而人以为自己只跑了一个阶段，白等七分钟。
// 所以未知参数在这里就停住，不进入任何阶段。
//
// `--base=<ref>` 由 `35-baseline.mjs` 直接读 `process.argv`（棘轮基准的显式覆盖口，
// 见该文件），它不经过本文件的解析，但**是**合法参数，故一并放行并登记进用法。
const KNOWN_FLAGS = new Set(['--ci', '--list', '--help', '-h'])
const KNOWN_PREFIXES = ['--only=', '--skip=', '--profile=', '--base=']
const unknownArgs = argv.filter(
  (a) => !KNOWN_FLAGS.has(a) && !KNOWN_PREFIXES.some((p) => a.startsWith(p)),
)
if (unknownArgs.length) {
  console.error(red(`✗ 无法识别的参数：${unknownArgs.join(' ')}`))
  console.error('')
  console.error(USAGE)
  process.exit(2)
}
if (onlyRaw !== null && only.length === 0) {
  console.error(red('✗ `--only=` 是空清单（一个阶段都没有）'))
  console.error(dim('  要跑全部就不要给这个开关；要跑指定阶段就写 id（见 --list）。'))
  process.exit(2)
}

// ── 仓库身份：拒绝在**别的仓库**上跑 ────────────────────────────────────────
//
// 门禁的每个阶段都以**脚本所在的仓库**为准：各阶段模块自己解析 `repoRoot`
// （`path.resolve(scriptDir, '..', '..')`），docs 的守卫也各自锚定自己所在的位置。
// 所以从另一个仓库调用它，跑的是**另一个仓库**的门禁——而且会真的写它的
// `target/`、在它的 `.workbuddy-ai/gate-logs/` 里留下本轮日志，本地模式还会替它
// `git add`。
//
// 这不是「参数用错了」，是**静默作用在错误的仓库上**，所以这里拒绝而不是照跑。
// 实测形态（2026-10-09）：`commit.test.mjs` 的 `--gate` 用例在一个临时 git 仓库里
// 跑 `commit.mjs --dry-run --gate`，于是门禁在 symbio 上真的跑了起来——不但白跑
// 60s（被测试自己的 timeout 砍断），它起的 cargo 还**抢走了 `.cargo-lock`**，
// 让**正在跑的门禁**的 `cargo test -p symbio` 卡在 `Blocking waiting for file lock`
// 上（实测日志首行即是它）。回归测试替被测系统制造锁争用，是这一条要断掉的。
//
// 判据 = 调用方 cwd 的 git 根 ≠ 本脚本所在仓库根。调用方**不在任何 git 仓库里**时
// 不判（没有「另一个仓库」可言，且此时不存在「静默门禁错仓库」这个失败模式）。
const callerRoot = (() => {
  const r = spawnSync('git', ['rev-parse', '--show-toplevel'], {
    cwd: process.cwd(),
    encoding: 'utf8',
    shell: false,
  })
  return r.status === 0 && r.stdout.trim() ? r.stdout.trim() : null
})()
/** 路径等价：realpath 规范化 + Windows 下大小写不敏感（`d:\` 与 `D:\` 是同一个目录）。 */
const samePath = (a, b) => {
  const norm = (p) => {
    let q = p
    try {
      q = fs.realpathSync.native(p)
    } catch {
      /* 路径不存在也照比，只是少一层规范化 */
    }
    q = path.resolve(q).replace(/\\/g, '/')
    return process.platform === 'win32' ? q.toLowerCase() : q
  }
  return norm(a) === norm(b)
}
if (callerRoot && !samePath(callerRoot, repoRoot)) {
  console.error(red('✗ 门禁拒绝执行：调用方在**别的仓库**里'))
  console.error(`    调用方仓库：${callerRoot}`)
  console.error(`    本门禁属于：${repoRoot}`)
  console.error('')
  console.error('  门禁的每个阶段都以**脚本所在的仓库**为准（阶段模块自己解析仓库根，')
  console.error('  docs 的守卫也各自锚定自己所在的位置），所以从这里跑它，跑的是上面')
  console.error('  那个仓库的门禁——会真的写它的 target/、在它的 .workbuddy-ai/')
  console.error('  gate-logs/ 里留下本轮日志，本地模式还会替它 git add。')
  console.error('')
  console.error('  要门禁本仓库：cd 到它下面再跑；要门禁别的仓库：那个仓库得有自己的')
  console.error('  scripts/gate.mjs。')
  process.exit(2)
}

const NOISE =
  /^(?:\s*$|.*\r$|\s*(Compiling|Checking|Downloading|Downloaded|Updating|Locking|Adding|Removing|Finished|Blocking|Waiting|Fresh|Documenting|Building)\b)/

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
    const dir = ensureLogDir()
    const logFile = path.join(dir, `${logFileName(label)}.log`)
    const sink = fs.createWriteStream(logFile, { flags: 'w' })
    // resolve 必须等 `sink` 刷完盘（见 `settle`）。`WriteStream` 的 `write()` 只是入队，
    // 而汇总的「失败详情」段是**再读那个文件**——满载时（冷编译抢盘）队尾还没落盘，
    // 读到的就是缺了末几行的半份日志，于是 `gate.test.mjs` 的「失败处必须有正文」判据
    // 在并发批次里随机假红。证据文件没写完就说「跑完了」，等于把最要紧的那份证据丢掉。
    let settled = false
    let pending = null
    const settle = (result) => {
      if (settled) return
      settled = true
      pending = result
      sink.end(() => resolve(result))
    }
    // 日志文件本身写不进去（本机实测过：批量删除守卫 / 文件被占用）不能让整轮门禁吊死，
    // 也不能让一个没人接的 `error` 事件把门禁炸掉——带结果返回，缺的那段证据由
    // `failureDetail` 的「读不到」分支如实呈现。
    sink.on('error', () => {
      if (pending) resolve(pending)
    })

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
      sink.write(err.message)
      settle({
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
      if (!quiet) {
        if (timedOut) console.log(yellow(`超时（已 kill，${fmtDuration(ms)}）`))
        else if (ok) console.log(green(`ok (${fmtDuration(ms)})`))
        else console.log(red(`失败 (exit=${code}${signal ? `, ${signal}` : ''}, ${fmtDuration(ms)})`))
      }
      settle({ ok, code, signal, output, timedOut, logFile, ms })
    })
  })
}

const ctx = {
  repoRoot,
  scriptDir,
  logDir,
  ci: CI,
  profile,
  // 棘轮向上自动上调：仅本地门禁写回并暂存（CI 无法提交，只亮绿灯）。见 `_shared.ratchetOrBump`。
  autoBump: !CI,
  run,
  /** 手写日志（自定义任务自己产出的文本）。返回落盘路径，便于结果里带回去。 */
  log(name, text) {
    const dir = ensureLogDir()
    const file = path.join(dir, `${logFileName(name)}.log`)
    fs.writeFileSync(file, text, 'utf8')
    return file
  },
}

// 阶段来源 = `scripts/gate.d/`，外加 `GATE_EXTRA_STAGES` 指向的目录（**测试注入口**）。
//
// 为什么需要这个口：`gate.test.mjs` 有两条判据要跑一个**必然失败**的阶段，才能端到端
// 验到「失败详情」与「每轮一个日志目录」。它原先直接把演示阶段写进 `scripts/gate.d/`
// ——那等于让测试**改门禁自己的活配置**，两个后果都实测到了：
//   ① 任何扫这个目录的判据（`gate-wiring-audit` 的 GW-005）会扫到它，红得毫无道理，
//      而且**随机**——谁先跑谁看见，串行时不显形，一并发就显形；
//   ② 清理失败（批量删除守卫 / 文件被占用）就把一个未登记的阶段**永久**留在仓里，
//      此后每一轮门禁都红，而红的原因早就不在场了。
// 注入口把「测试要一个坏阶段」与「仓里有哪些阶段」分开：临时目录里的阶段用完即弃，
// `scripts/gate.d/` 始终只包含**真的**阶段。
//
// ⚠️ 它是**注入**不是**豁免**：合并进来的阶段照常参与文件名排序、`--only` 校验、
// 并发分批，没有任何一条判据因为它的存在而放宽。不设它时行为与从前逐字一致。
const gateDir = path.join(scriptDir, 'gate.d')
const extraStageDir = process.env.GATE_EXTRA_STAGES
const stagePaths = new Map()
for (const dir of [gateDir, ...(extraStageDir ? [extraStageDir] : [])]) {
  for (const f of fs.readdirSync(dir).filter((f) => f.endsWith('.mjs') && !f.startsWith('_'))) {
    stagePaths.set(f, path.join(dir, f)) // 同名时后者（注入目录）覆盖前者
  }
}
const stages = []
// 来自注入目录的阶段 id：**它们不进校准文件**（见 `report()` 里 `recordRun` 的调用）。
// 测试夹具不是「这台机器跑门禁的真实成本」，把它记进历史会污染两件事：
// ① 超时预算（一个 300ms 的演示阶段会把同名任务的历史拉低）；
// ② LPT 起跑顺序（演示阶段有实测后，人工 `long` 标注就被实测覆盖——实测本该优先，
//    但那份「实测」测的是夹具，不是被测任务）。
const injectedStageIds = new Set()
for (const f of [...stagePaths.keys()].sort()) {
  const file = stagePaths.get(f)
  const mod = await import(pathToFileURL(file).href)
  if (mod.default?.id) {
    stages.push(mod.default)
    if (extraStageDir && path.resolve(path.dirname(file)) === path.resolve(extraStageDir)) {
      injectedStageIds.add(mod.default.id)
    }
  }
}

const enabled = (id) => (only ? only.includes(id) : true) && !skip.includes(id)

// ── 阶段 id 必须真的存在 ────────────────────────────────────────────────────
//
// **这是「静默假绿」唯一的入口**：`--only=backedn`（拼错）会让 `enabled()` 对每个
// 阶段都返回 false ⇒ 0 个阶段 ⇒ 报告「通过 0 / 0 全部通过」并退出 0。CI 上等于
// 悄悄少跑一整个 job，而日志里一片绿。同理 `--only=`（空清单，上面已拦）。
// 判据 = `gate.test.mjs`：拼错的 id 退出 2，且**不进入门禁主流程**。
const stageIds = new Set(stages.map((s) => s.id))
const unknownIds = [...new Set([...(only ?? []), ...skip])].filter((id) => !stageIds.has(id))
if (unknownIds.length) {
  console.error(red(`✗ --only / --skip 里的阶段 id 不存在：${unknownIds.join(', ')}`))
  console.error(dim(`  已知阶段：${stages.map((s) => s.id).join(', ')}`))
  console.error(dim('  拼错的 id 不会自己报错——它会让 0 个阶段「全部通过」，这正是这里要断掉的。'))
  process.exit(2)
}

// 结果按**阶段分组缓冲**，report 时按阶段顺序摊平。并发泳道下各阶段的完成时刻
// 交错，直接 push 会让汇总变成「谁快谁在前」；分组缓冲让中途（SIGINT）的汇总
// 也保持「按阶段归组、组内按任务序」。
const stageBuffers = new Map()
function bufferOf(stageId) {
  let buf = stageBuffers.get(stageId)
  if (!buf) stageBuffers.set(stageId, (buf = []))
  return buf
}
/**
 * 阶段 → **墙钟**耗时（ms）。`report` 据此给出「哪个阶段是瓶颈」。
 *
 * 为什么需要它：逐任务耗时在并发批里是相互争用后的数（同一批里 cargo 与 30 份
 * `node --test` 抢 CPU），加起来既不等于阶段耗时也不等于门禁耗时；而**阶段墙钟**
 * 才是可比的量。此前没有这一层，想回答「门禁的时间花在哪」只能重跑一遍并手工掐表
 * ——「重跑一遍才看得到」正是这份清单要消掉的东西（与失败日志同一条理由）。
 */
const stageMs = new Map()
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

/**
 * 把启用的阶段切成「串行段 / 并发批」。
 *
 * **调度与打印共用这一份**：两处各推一遍的话，打印出来的编排会和实际跑的悄悄分叉，
 * 而分叉不会报错——`facts-write` 夹在批中间那次就是这样：本文件头写着「一整批」，
 * 实际跑成两段，白等一个 docs 的墙钟（6m25s）且没有任何信号。
 * 判据 = `gate.test.mjs`「阶段级并发必须连成一批」。
 */
function planLanes(list) {
  const groups = []
  for (let i = 0; i < list.length; ) {
    if (list[i].s.parallel) {
      const batch = []
      while (i < list.length && list[i].s.parallel) batch.push(list[i++])
      groups.push({ parallel: true, lanes: batch })
    } else {
      groups.push({ parallel: false, lanes: [list[i++]] })
    }
  }
  return groups
}
/** 编排的一行渲染：并发批包在 `[...]` 里、组内用 `∥`，串行段用 `→` 连接。 */
const renderPlan = (groups) =>
  groups
    .map((g) => (g.lanes.length > 1 ? `[${g.lanes.map((l) => l.s.id).join(' ∥ ')}]` : g.lanes[0].s.id))
    .join(' → ')

if (listOnly) {
  for (const [i, s] of stages.entries()) {
    const tasks = materialize(s)
    console.log(`${i + 1}. ${bold(s.id)} — ${s.title}（${tasks.length} 项）`)
    for (const t of tasks) console.log(`   · ${t.label}`)
  }
  process.exit(0)
}

// 泳道计划在**打印之前**就算出来：下面那行编排与真正的调度共用同一个 `lanePlan`
// （两处各推一遍必然分叉，而分叉不报错——见 `planLanes` 的注释）。
const lanes = stages.map((s, idx) => ({ s, idx })).filter(({ s }) => enabled(s.id))
const lanePlan = planLanes(lanes)

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
console.log(dim(`  阶段：${renderPlan(lanePlan)}`))

// ── 机器能力档案：并发度与超时预算的唯一来源（`gate.d/_machine.mjs`） ──────
//
// **打印是机制的一部分，不是装饰**。自适应机制最大的风险不是算错，而是**没人
// 知道它在算**：慢机器上看到 `并发 cpu 2`，才知道门禁确实降级了（而不是「还是
// 那么慢，优化没用」）；快机器上看到 `并发 cpu 15`，才知道升级没白升。
// 不打印的话，两种机器上的体验都是「一个黑盒，反正要等」。
//
// 放在这里（而不是文件头）是为了让 `--help` / `--list` / 被拒绝的调用**不付**
// 那 170ms 的探测成本——它们走不到这一行。
MACHINE = await machine()
CONCURRENCY = MACHINE.slots.io
console.log(dim(`  机器：${describe(MACHINE)}`))
{
  const rounds = Math.max(0, ...Object.values(MACHINE.calibration.tasks ?? {}).map((t) => t.ms?.length ?? 0))
  if (MACHINE.discardedFingerprint) {
    console.log(
      dim(`  校准：机器指纹已变（上一份来自 ${MACHINE.discardedFingerprint}），历史作废、按本机重新起算`),
    )
  } else if (rounds > 0) {
    console.log(dim(`  校准：沿用本机历史（最多 ${rounds} 轮）⇒ 超时预算取实测中位数 × 2.5，长任务按实测耗时排序`))
  } else {
    console.log(
      dim(`  校准：本机首跑，暂无实测 ⇒ 超时预算按静态值 × ${MACHINE.cpuScale.toFixed(2)}（单核探针）`),
    )
  }
}
// 到这里参数已全部校验通过、阶段即将开跑 ⇒ 这一轮**真的会发生**，此时建目录、
// 写指针都名实相符（`--help` 与被拒绝的调用走不到这里，所以不会留空目录）。
const relLog = (path.relative(repoRoot, ensureLogDir()) || '.').replace(/\\/g, '/')
console.log(dim(`  本轮日志：${relLog}/（每轮一个目录，留最近 ${KEEP_RUNS} 轮；失败项的末 ${FAIL_TAIL_LINES} 行会直接印在汇总里）`))

/**
 * 并发跑一批任务（连续标记 `parallel: true` 的那些），并发度 `limit`。
 *
 * **结果按原顺序落**：`out` 预分配 + worker 从共享游标取任务，因此汇总表的顺序
 * 恒等于任务声明顺序，不受完成先后影响（否则「谁快谁排前面」，两次运行的日志
 * 顺序都不同，对比就白费力气）。
 *
 * 批内强制 `quiet`：多个子进程同时往 stdout 写会交错成乱码，详情改由各自日志
 * 承载（失败时汇总会直接列出文件名），控制台只保留一行结论。
 *
 * **`summary: true` 的任务补印结论行**（见 `lastLogLine`）：批内静音是为了不交错，
 * 不是为了丢掉结论——「N 条规则全部通过」这类最后一行本来就在日志里，批结束后按
 * 声明序补印即可。此前守卫本体因为要保住这一行而被排除在并发之外，整段串行 32s；
 * 「结论行」与「并发」从来不是二选一。
 *
 * **`long: true` 的任务先起**（LPT：最长者先做）。并发池按声明序发任务，于是长杆
 * 排在第 38 位就得等前几波短任务跑完才开工——而它的时长往往比其余全部任务加起来
 * 还长，那几波等待是纯加项。实测（docs 阶段 61 项 / 8 泳道）：`windows-restart`
 * 排在第 38 位时 docs 墙钟 4m30s，其中 209s 是它、前 50s 是"等它被轮到"；
 * 先起之后这段等待消失。**只影响起跑顺序，不影响判定、不影响汇总顺序**（`out`
 * 仍按声明序落）。它是个**时长提示**不是判据：写错了最多白排一次序。
 */
function lastLogLine(file) {
  if (!file) return null
  try {
    const lines = stripAnsi(fs.readFileSync(file, 'utf8'))
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean)
    return lines.length ? lines[lines.length - 1] : null
  } catch {
    return null // 日志读不到不该改变判定，只是少一行提示
  }
}

/**
 * 任务的「起跑权重」：越大越先起（LPT，最长者先做）。
 *
 * 两个来源，**实测优先**：
 *
 * 1. **本机上一轮的实测耗时**（`_machine.mjs::measuredMs`，中位数窗口 5 轮）。
 *    这是最准的——「谁慢」本来就是一个只有跑过才知道的问题。
 * 2. 没有实测时退回**人工标注** `long: true`。它只在**首跑**（或换机后的首跑）
 *    有用，是冷启动的拐杖：新加一个慢测试时，标注能立刻让它在第一批就起，
 *    而不必先慢一轮、下一轮才被排对。
 *
 * 为什么不是「只用实测」：换机器后历史被丢弃，那一刻没人知道谁慢，若不给人工
 * 标注留位置，新机器上的**每一轮首跑**都会退化成「按声明序」，而声明序里长杆
 * 往往排在中间——那正是这一轮要修掉的浪费。
 *
 * 为什么不是「只用标注」：标注会过时。慢测试被优化掉之后，标注还挂着，于是它
 * 一直抢第一批的位子；反过来，某个测试变慢了而没人补标注，就永远排不上。
 * 实测会自己跟上。
 */
function taskWeight(stageId, t) {
  const measured = measuredMs(MACHINE, stageId, t.label)
  if (measured != null) return measured
  return t.long ? 1e9 : 0
}

async function runConcurrent(stageId, tasks, limit) {
  const out = new Array(tasks.length)
  // 起跑顺序：按 `taskWeight` 降序（见上）。**只是取任务的下标顺序变了**，
  // `out` 仍按原下标落 ⇒ 汇总表的顺序与判定完全不受影响。`sort` 在 V8 里是稳定的，
  // 所以同权重任务的相对次序仍是声明序。
  const order = tasks
    .map((_, i) => i)
    .sort((a, b) => taskWeight(stageId, tasks[b]) - taskWeight(stageId, tasks[a]))
  let cursor = 0
  const worker = async () => {
    for (;;) {
      if (cursor >= order.length) return
      const idx = order[cursor++]
      out[idx] = await executeTask(stageId, tasks[idx], { quiet: true })
    }
  }
  await Promise.all(Array.from({ length: Math.min(limit, tasks.length) }, worker))

  for (const [i, r] of out.entries()) {
    record(stageId, r)
    const mark = r.pass === true ? green('✓') : r.pass === 'skipped' ? yellow('⊘') : red('✗')
    const took = r.ms != null ? dim(` (${fmtDuration(r.ms)})`) : ''
    console.log(`  ${mark} ${r.label}${took}${r.note ? yellow(` — ${r.note}`) : ''}`)
    if (tasks[i].summary) {
      const file = Array.isArray(r.logFile) ? r.logFile[0] : r.logFile
      const line = lastLogLine(file)
      if (line) console.log(dim(`      ${line}`))
    }
  }
}

/**
 * 阶段声明的并发度 → 具体数字。
 *
 * 阶段模块是**同步 import** 的（加载时就要求 `concurrency` 是个数），而机器档案
 * 是异步探测的（要等 120ms 的负载采样）。所以阶段不能直接写数字，只能声明
 * **任务类型**，由这里在运行期解析：
 *
 * - `concurrency: <number>` —— 写死的数，仅用于「这个阶段的并发与机器无关」的
 *   极少数情形（目前没有）。**不要用它来表达「我实测过 8 挺好」**——那正是这次
 *   要清掉的东西：8 是某台机器的快照，不是性质。
 * - `concurrencyKind: 'cpu'` —— 吃满每个核（rustc / cargo / vitest worker）。
 * - `concurrencyKind: 'io'`  —— 大部分时间在等文件系统，超订有益（审计脚本）。
 * - 都不写 —— 用默认值（`slots.io`，任务级并发绝大多数是审计与测试）。
 */
function resolveConcurrency(s) {
  if (typeof s.concurrency === 'number') return s.concurrency
  if (!MACHINE) return CONCURRENCY
  if (s.concurrencyKind === 'cpu') return MACHINE.slots.cpu
  if (s.concurrencyKind === 'io') return MACHINE.slots.io
  return CONCURRENCY
}

async function runStage(s, index) {
  stageHeader(index + 1, s.title)
  const stageStart = Date.now()
  const tasks = materialize(s)
  const limit = resolveConcurrency(s)
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
  // 阶段**墙钟**耗时（不是任务耗时之和）：瓶颈定位要的是「这一阶段占了多少墙钟」。
  // 并发批下逐任务耗时是相互争用后的数，加起来没有意义；阶段墙钟才是可比的量。
  stageMs.set(s.id, (stageMs.get(s.id) ?? 0) + (Date.now() - stageStart))
}

/**
 * 并发跑一批「相互无数据依赖」的阶段（连续标 `parallel: true` 的那些）。
 * 与任务级批同构：worker 从共享游标取泳道。
 *
 * 并发度**不按核数线性给**：每个阶段内部自己有限流（docs 的 io 路数、backend 的
 * 单路 cargo、msrv 的独立 target），阶段之间再乘一遍就是重复放大。实测本机
 * （8 逻辑核）整批 5 个阶段同时开是有效的，所以 `mid`/`high` 不限制。
 *
 * **低配机器要限制**：`low` 档（≤4 核 或 ≤8GB）上 5 个阶段同时跑，等于把各阶段
 * 已经降过一轮的并发再叠 5 层，swap 与调度开销会把「并发」吃回去。这类机器上
 * 批内只放 2 条泳道，剩下的排队——总墙钟反而更短。
 */
async function runStageBatch(batch) {
  const cap = MACHINE?.tier === 'low' ? 2 : Math.max(2, CONCURRENCY)
  const limit = Math.min(batch.length, cap)
  // 批内争用进超时预算：本批有几条泳道，就按几条分机器来算预算
  // （见 `_machine.setBatchLanes`）。为什么是**整批**而不是只数吃 CPU 的那几条：
  // 实测那轮超时里，docs 的 8 路 io 审计与 cargo 一起把磁盘压满，把 `docs` 排除
  // 在外会低估一半的争用。
  setBatchLanes(batch.length)
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
// 泳道调度：连续标 `parallel` 的阶段合成一批并发，其余照旧串行。
// `parallel` 语义见文件头「阶段级同构」一节；哪些阶段标了、为什么（以及为什么
// 某些阶段**不能**标），判据在各阶段自己的文件头。计划在打印处已算出（`lanePlan`）。
for (const group of lanePlan) {
  if (group.lanes.length > 1) await runStageBatch(group.lanes)
  else {
    // 单泳道 = 独占机器：预算回到「不乘任何争用系数」的原点。不重置的话，
    // 批后那些串行阶段（verify / vite build / e2e）会继承批内的放大系数。
    setBatchLanes(1)
    await runStage(group.lanes[0].s, group.lanes[0].idx)
  }
}

/** 汇总并给退出码。**中断时也走这里**（见下方 SIGINT/SIGTERM）——已跑完的结果、
 *  各自日志文件、失败归属都不该因为按了一次 Ctrl+C 就丢掉。 */
function report(interrupted = false) {
  const results = collectedResults()
  const failed = results.filter((r) => r.pass === false)
  const skipped = results.filter((r) => r.pass === 'skipped')
  const passed = results.length - failed.length - skipped.length

  // 阶段耗时（墙钟）：瓶颈定位的第二层。逐任务耗时在并发批里是**争用后**的数，
  // 加起来没有意义；阶段墙钟才是可比的量。并发批里的阶段相互重叠，故合计可能
  // 大于总耗时——这一点写在表头，免得读者拿它去加。
  const stageRows = [...stageMs.entries()].sort((a, b) => b[1] - a[1])

  // 把本轮实测并进 per-machine 校准文件：**下一轮的超时预算与起跑顺序靠它**。
  // 中断路径也记——已经跑完的那些任务耗时是真实数据，没有理由因为按了 Ctrl+C 丢掉。
  // 写失败不抛（见 `recordRun`）：记不下这次多快，不该改变这次的结论。
  //
  // **注入的阶段（`GATE_EXTRA_STAGES`）不记**：那是测试夹具，不是这台机器跑门禁的
  // 真实成本。实测踩过——`gate.test.mjs` 的 LPT 判据第二轮起就红，因为演示阶段的
  // 两个任务已经攒出了「实测耗时」，于是「人工标了 long 的先起」被实测覆盖了，
  // 而那份实测测的是 `-e "appendFileSync"`，与真实长杆毫无关系。
  if (MACHINE) {
    const measured = injectedStageIds.size ? results.filter((r) => !injectedStageIds.has(r.stage)) : results
    const wall = Object.fromEntries([...stageMs].filter(([id]) => !injectedStageIds.has(id)))
    recordRun(MACHINE, measured, { stageWall: wall })
  }

  // 运行清单落盘：终端的滚动缓冲会丢，文件不会。**中断路径也写**——正是那条
  // 路径最可能只剩这一个物证。逐项带日志文件名，供事后核对。
  //
  // 失败项**连末 N 行一起落盘**：清单是给人事后读的，只给路径等于让人再跑一次
  // （这正是 `_logs.mjs` 要治的病②）。清单与终端的失败详情同源同文。
  const failTailLines = Number(process.env.GATE_FAIL_TAIL) || FAIL_TAIL_LINES
  try {
    const lines = [
      `# 门禁运行清单${interrupted ? '（中断，未跑完）' : ''}`,
      `时间：${new Date().toISOString()}`,
      `轮次目录：${relLog}/`,
      `结论：通过 ${passed} / ${results.length}${skipped.length ? `（另有 ${skipped.length} 项未判定）` : ''}`,
      `总耗时：${fmtDuration(Date.now() - gateStarted)}`,
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
      ...(failed.length
        ? [
            '## 失败详情（各失败项日志的末 ' + failTailLines + ' 行）',
            '',
            ...failed.flatMap((r) => {
              const { header, body } = failureDetail(r, { repoRoot, lines: failTailLines })
              return [header, ...body, '']
            }),
          ]
        : []),
      ...(stageRows.length
        ? [
            '## 阶段耗时（墙钟，降序）',
            '',
            '| 阶段 | 墙钟 |',
            '|---|---|',
            ...stageRows.map(([id, ms]) => `| ${id} | ${fmtDuration(ms)} |`),
            '',
            '> 并发批里的阶段相互重叠，故合计可能大于总耗时；逐任务耗时在批内是争用后的数。',
            '',
          ]
        : []),
    ]
    fs.writeFileSync(path.join(ensureLogDir(), '_summary.md'), lines.join('\n'), 'utf8')
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
  // 阶段墙钟（降序）：一眼看出瓶颈在哪，不必重跑。与清单里那张表同源。
  if (stageRows.length) {
    console.log(
      dim(
        `  阶段耗时（墙钟，降序）：${stageRows.map(([id, ms]) => `${id} ${fmtDuration(ms)}`).join(' · ')}`,
      ),
    )
  }

  // 失败逐项：**日志末 N 行直接印在这里**，路径一并给出。
  //
  // 早先这里只列一行「· 任务 → 路径」，正文要自己去那个文件里翻——而门禁一轮
  // 7 分钟，人的下一个动作往往是「再跑一遍看看」（最坏：日志已被下一轮覆盖，
  // 白跑）。失败处的信息量与文件里的信息量之间不该隔一次重跑。
  //
  // 截断额度的三个旋钮都在 `gate.d/_logs.mjs`，`GATE_FAIL_TAIL` 覆盖行数。
  if (failed.length) {
    const lines = Number(process.env.GATE_FAIL_TAIL) || FAIL_TAIL_LINES
    console.log()
    console.log(bold('══ 失败详情 ══'))
    console.log(dim(`  以下取各失败项日志的末 ${lines} 行（GATE_FAIL_TAIL=<行数> 可调；完整输出在路径指向的文件里）`))
    for (const r of failed) {
      const { header, body } = failureDetail(r, { repoRoot, lines })
      console.log()
      console.log(red(`  ${header}`))
      for (const l of body) console.log(dim(l))
    }
  }

  console.log(dim(`  本轮日志目录：${path.relative(repoRoot, logDir).replace(/\\/g, '/') || '.'}/`))
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
