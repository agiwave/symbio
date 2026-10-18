// `scripts/gate.mjs` 的回归测试。
//
// 这个脚本的失效方式不是「报错」而是「代价」：`--help` 不被识别时，它会照常跑完
// **全量门禁**（10+ 分钟），在这次运行里写下新的一轮日志——查一次用法，代价是
// 七分钟。与 `commit.mjs` 同类陷阱（那里 `--help` 会**真的提交一次**）。
//
// 另一半代价曾经更重：日志原先是平铺的 `.workbuddy-ai/gate-logs/<任务>.log`，
// 每轮覆盖同名文件 ⇒ 失败之后**再跑一次门禁，失败证据就没了**。现在每轮一个目录
// （`gate.d/_logs.mjs`），本文件末尾两条判据钉住这件事：失败详情必须印在失败处、
// 上一轮的日志不许被这一轮冲掉。
//
// 跑法：node --test scripts/gate.test.mjs

import { test } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { pathToFileURL } from 'node:url'
import { stripAnsi } from './color.mjs'
import { derive, fingerprintOf, describe, budget, machine } from './gate.d/_machine.mjs'

const repoRoot = path.resolve(import.meta.dirname, '..')
const gateScript = path.join(repoRoot, 'scripts', 'gate.mjs')

/**
 * 跑 `gate.mjs`：stdin 给 `'ignore'`（真交互 / 真等待会挂），并给**显式 timeout**——
 * 失败要快，不许把「白跑十分钟门禁」这个 bug 拖成一次十分钟的测试。
 *
 * `env` 用来给门禁加环境（现只有 `GATE_EXTRA_STAGES`，见下方 `withDemoStage`）。
 */
function runGate(args = [], env = {}) {
  return spawnSync(process.execPath, [gateScript, ...args], {
    cwd: repoRoot,
    encoding: 'utf8',
    timeout: 60_000,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, ...env },
  })
}

test('★ `--help` 只打印用法并退出 0，不跑任何阶段（否则查用法 = 白跑十分钟 + 覆盖日志）', () => {
  const r = runGate(['--help'])
  assert.equal(r.status, 0, `--help 应退出 0：${r.stdout}\n${r.stderr}`)
  const out = stripAnsi(r.stdout)
  assert.match(out, /用法：node scripts\/gate\.mjs/, '应打印用法')
  assert.match(out, /--only=/, '用法应列出主要开关')
  assert.match(out, /--list/, '用法应提到 --list')
  // 参数白名单里的每一个都必须在用法里登记（`--base=` 只被 `35-baseline.mjs` 直接读
  // `process.argv`，最容易在改白名单时漏登记——而漏登记的合法参数会变成「未知参数」）。
  assert.match(out, /--base=/, '用法应登记 --base=（它是白名单里的一员）')

  // 这两条才是补 `--help` 的**理由**：原先它会照常走完全量门禁。
  assert.doesNotMatch(out, /══ 门禁 ══/, '不得进入门禁主流程')
  assert.doesNotMatch(out, /── 阶段 \d+\/\d+/, '不得执行任何阶段')
})

test('`--list` 仍只列阶段与任务、不执行（用法自述不改变既有开关）', () => {
  const r = runGate(['--list'])
  assert.equal(r.status, 0, `--list 应退出 0：${r.stdout}\n${r.stderr}`)
  const out = stripAnsi(r.stdout)
  assert.match(out, /^1\. /m, `应列出阶段：\n${out}`)
  assert.doesNotMatch(out, /══ 门禁 ══/, '--list 不该进入主流程')
})

// 失效方式不是「报错」而是**静默假绿**：`--only=backedn`（拼错）会让 `enabled()`
// 对每个阶段都返回 false ⇒ **0 个阶段** ⇒ 报告「通过 0 / 0 全部通过」退出 0。
// CI 上等于悄悄少跑一整个 job，日志里一片绿。同理未识别的开关（`--only backend`
// 漏了 `=`、早已删除的 `--fix`）会静默变成一次全量门禁并覆盖上一轮日志。
// 三条都断言**没有进入主流程**——只断言退出码的话，「照跑默认动作」也满足。
test('★ `--only=<拼错的 id>` 拒绝：否则 0 个阶段也会报「全部通过」', () => {
  const r = runGate(['--only=backedn'])
  const out = stripAnsi(r.stdout + r.stderr)
  assert.equal(r.status, 2, `拼错的阶段 id 应拒绝（退出码 2）：${out}`)
  assert.match(out, /阶段 id 不存在/, `应指出是阶段 id 的问题：${out}`)
  assert.doesNotMatch(out, /══ 门禁 ══/, `不得进入门禁主流程：${out}`)
  assert.doesNotMatch(out, /── 阶段 \d+\/\d+/, `不得执行任何阶段：${out}`)
})

test('★ `--only=`（空清单）拒绝：空清单与「没给开关」是两件事', () => {
  const r = runGate(['--only='])
  const out = stripAnsi(r.stdout + r.stderr)
  assert.equal(r.status, 2, `空清单应拒绝：${out}`)
  assert.doesNotMatch(out, /══ 门禁 ══/, `不得进入门禁主流程：${out}`)
})

test('★ 未识别的参数拒绝：`--only backend`（漏 `=`）不得静默跑全量门禁', () => {
  const r = runGate(['--only', 'backend'])
  const out = stripAnsi(r.stdout + r.stderr)
  assert.equal(r.status, 2, `未知参数应拒绝：${out}`)
  assert.match(out, /无法识别的参数/, `应指出未知参数：${out}`)
  assert.match(out, /用法：node scripts\/gate\.mjs/, '应打印用法')
  assert.doesNotMatch(out, /── 阶段 \d+\/\d+/, `不得执行任何阶段：${out}`)
})

test('放行对照：合法阶段 id 通过校验并真的跑起来（不是把所有 `--only` 都拦掉）', () => {
  const r = runGate(['--only=baseline'])
  const out = stripAnsi(r.stdout + r.stderr)
  // 只断言「没被校验拦下」——baseline 自己的判定是另一条判据，不在这里再判一次。
  assert.notEqual(r.status, 2, `合法 id 不该被拒：${out}`)
  assert.match(out, /棘轮元守卫/, `合法 id 应真的跑起来：${out}`)
})

// 失效方式不是「报错」而是**静默作用在错误的仓库上**：门禁的每个阶段都以脚本
// 所在的仓库为准（阶段模块自己解析仓库根，docs 的守卫也各自锚定自己所在的位置），
// 所以从别的仓库调用它，跑的是**本仓**的门禁——写它的 `target/`、把它的
// `.workbuddy-ai/gate-logs/` 指到新的一轮，本地模式还替它 `git add`。
// 实测来源：`commit.test.mjs` 的 `--gate` 用例（在临时仓库里调 `commit.mjs --gate`）
// 让 symbio 的门禁真的跑了起来，起的 cargo 抢走 `.cargo-lock`，把**正在跑的门禁**
// 卡在 `Blocking waiting for file lock` 上。
test('★ 拒绝在**别的仓库**里跑：否则会静默门禁另一个仓库并写它的产物', () => {
  const foreign = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-foreign-repo-'))
  try {
    spawnSync('git', ['-c', 'init.defaultBranch=main', 'init', '-q'], { cwd: foreign })

    const r = spawnSync(process.execPath, [gateScript, '--list'], {
      cwd: foreign,
      encoding: 'utf8',
      timeout: 60_000,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    const out = stripAnsi(r.stdout + r.stderr)

    assert.equal(r.status, 2, `跨仓库调用应拒绝（退出码 2）：${out}`)
    assert.match(out, /拒绝执行/, `应说明拒绝理由：${out}`)
    // 反证：拒绝必须发生在**任何阶段之前**——照跑的实现同样会退出非 0。
    assert.doesNotMatch(out, /══ 门禁 ══/, `不得进入门禁主流程：${out}`)
    assert.doesNotMatch(out, /── 阶段 \d+\/\d+/, `不得执行任何阶段：${out}`)
    // 调用方仓库里不得留下门禁产物（日志目录就是「它在这儿跑过」的物证）
    assert.ok(!fs.existsSync(path.join(foreign, '.workbuddy-ai')), '门禁不得在调用方仓库里留下产物')
  } finally {
    fs.rmSync(foreign, { recursive: true, force: true })
  }
})

// ── 失败详情与日志保留 ──────────────────────────────────────────────────
//
// 这两条钉的是同一件事的两半：**失败之后不必为了看错误再跑一次门禁**。
//
//   ① 失败详情：汇总里必须出现失败项日志的**末几行正文**与**相对路径**。
//      只给一行「· 任务 → 路径」是不够的——正文在另一个文件里，人的下一个动作
//      就是再跑一轮（7 分钟），或者手动 cat（还得自己拼路径）。
//   ② 日志保留：每轮一个目录，**上一轮的日志不许被这一轮冲掉**。
//      曾经的病正是它：平铺 + `w` 覆盖 ⇒ 失败后再跑一次，证据就没了。
//
// 做法：造一个**必然失败**的阶段，用 `--only` 跑它，断言输出与落盘。用真阶段而不是
// mock——`report()` 的呈现逻辑只有端到端才走得到（`run` 的日志落盘、`record`、汇总全部参与）。
//
// ⚠️ 那个演示阶段**不住在 `scripts/gate.d/` 里**（2026-10-18 改）：它原先直接写进活目录，
// 等于让测试改门禁自己的配置，代价实测了两次——① `gate-wiring-audit` 扫那个目录时会
// 把演示阶段当成一个「排在 60-facts 之后的未登记阶段」判红（串行时不显形，一并发就随机红）；
// ② 清理被拦（批量删除守卫 / 文件占用）就把一个坏阶段永久留在仓里，此后每轮门禁都红，
// 而红的原因早就不在场。现在它落在 `os.tmpdir()` 下，由 `GATE_EXTRA_STAGES` 注入门禁
// （见 `gate.mjs` 阶段发现处的注释）。判据一条没减：仍然是**真阶段、真进程、真日志**。

/** 临时阶段的 id / 文件名（刻意用 `zz-` 前缀：排序在最后，且一眼能看出是注入的）。 */
const DEMO_STAGE = 'zz-failure-detail-demo'

/**
 * 造一个「一个命令失败、日志有正文」的阶段。
 *
 * 先刷 60 行噪声再报错：**噪声行数必须超过失败详情的默认尾行数**，否则断言
 * 「详情只取末几行」时它自己也在末几行里——那是判据自己的漏洞，与实现无关。
 */
const demoStageSource = () => `export default {
  id: '${DEMO_STAGE}',
  title: '失败详情演示（测试注入，勿留）',
  tasks: [
    {
      label: '演示任务：必然失败',
      cmd: process.execPath,
      args: [
        '-e',
        // ⚠️ 收尾用 \`process.exitCode = 1\` 而**不是** \`process.exit(1)\`：管道上的
        // stdout 是异步写，\`process.exit()\` 会**丢掉还没刷出去的尾部**。实测（2026-10-19，
        // 冷全量编译把机器压满时）：日志只写到「噪声行 47」，[ERROR] 那行没落盘，于是
        // 判据 ① 假红。判据自己的输入不能因为"没来得及 flush"而消失。
        "for (let i = 0; i < 60; i++) console.log('噪声行 ' + i);" +
          "console.log('[ERROR] 演示断言：期望 3 得到 4'); process.exitCode = 1",
      ],
    },
  ],
}
`

/**
 * 在**临时目录**里放好阶段文件，返回给 `runGate` 的 env。
 *
 * `files` = `{ '<文件名>.mjs': 源码 }`。目录里的非 `.mjs` 文件不会被加载（门禁的
 * 过滤条件与 `scripts/gate.d/` 完全一致），所以测试可以往同一个目录里放标记文件。
 *
 * 清理是 best-effort：`os.tmpdir()` 下的残留无害（系统会收），而 `rmSync` 本身在
 * 某些环境会被拦（批量删除守卫）——让清理失败去判红一份功能测试，就是让判据去
 * 测一件与被测对象无关的事。**真正要紧的是仓里不留东西**，而这一点由"不写进仓"
 * 结构性保证，不靠 `finally`。
 */
function withStageDir(files) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-gate-stages-'))
  for (const [name, source] of Object.entries(files)) fs.writeFileSync(path.join(dir, name), source, 'utf8')
  return {
    dir,
    env: { GATE_EXTRA_STAGES: dir },
    dispose: () => { try { fs.rmSync(dir, { recursive: true, force: true }) } catch { /* 见上 */ } },
  }
}
const withDemoStage = () => withStageDir({ [`${DEMO_STAGE}.mjs`]: demoStageSource() })

test('★ `long: true` 的任务先起：并发池按声明序发任务，长杆排在后面就是白等', () => {
  // 判据 = **执行顺序**，所以把并发度压到 1（`concurrency: 1`）：这样「取任务的下标
  // 顺序」直接等于「执行顺序」，可确定地断言，不必靠计时（计时会飘，飘了就假红）。
  // 声明序是 [短, 长]，期望执行序是 [长, 短] ⇒ 标记文件内容是 `BA`。
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-gate-stages-'))
  const marker = path.join(dir, 'order.log')
  /** 一个把首字母追加进标记文件的任务；`extra` 用来挂 `long: true`。 */
  const task = (tag, extra = '') => {
    const code = `require('fs').appendFileSync(${JSON.stringify(marker)}, '${tag}')`
    // `parallel: true` 是**必须**的：只有连续标记 `parallel` 的任务才会被交给并发池
    // （`runConcurrent`），而 `long` 只影响并发池的取任务顺序——不标 `parallel` 就
    // 走串行路径，那本来就已经是声明序，这条判据会变成在验一个空转。
    return `    { label: '${tag}', cmd: process.execPath, args: ['-e', ${JSON.stringify(code)}], parallel: true${extra} },\n`
  }
  const stage =
    `export default {\n` +
    `  id: 'zz-lpt-demo',\n` +
    `  title: '长杆先起演示（测试注入，勿留）',\n` +
    `  parallel: true,\n` +
    `  concurrency: 1,\n` +
    `  tasks: [\n` +
    task('A') +
    task('B', ', long: true') +
    `  ],\n` +
    `}\n`
  fs.writeFileSync(path.join(dir, 'zz-lpt-demo.mjs'), stage, 'utf8')
  try {
    const r = runGate(['--only=zz-lpt-demo'], { GATE_EXTRA_STAGES: dir })
    assert.equal(r.status, 0, `两个任务都应通过：${stripAnsi(r.stdout + r.stderr)}`)
    assert.equal(
      fs.readFileSync(marker, 'utf8'),
      'BA',
      '标了 `long: true` 的必须先跑——否则它要等前面所有任务跑完才开工，而它往往比其余加起来还长',
    )
  } finally {
    try { fs.rmSync(dir, { recursive: true, force: true }) } catch { /* 见 `withStageDir` */ }
  }
})

test('★ 失败项：日志末几行与相对路径直接印在汇总里（不必再跑一次门禁）', () => {
  const demo = withDemoStage()
  try {
    const r = runGate([`--only=${DEMO_STAGE}`], demo.env)
    const out = stripAnsi(r.stdout + r.stderr)
    assert.notEqual(r.status, 0, `必然失败的阶段不该退出 0：${out}`)

    assert.match(out, /══ 失败详情 ══/, `汇总里必须有失败详情段：${out}`)
    // 只看**失败详情段**：噪声行在任务运行时就被实时 echo 过，那是另一回事，
    // 混在一起判会把自己绕进去（本判据要的是「失败处有正文」）。
    const detail = out.slice(out.indexOf('══ 失败详情 ══'))
    assert.ok(detail.includes('[ERROR] 演示断言：期望 3 得到 4'), `失败详情里必须有日志正文：\n${detail}`)
    assert.ok(!detail.includes('噪声行 0'), '失败详情只取末几行，噪声行不该出现（否则等于全文糊屏）')
    assert.match(detail, /── 末 \d+ 行 ──/, '要说明截了多少行')
    // 本轮日志目录必须指名，且其中真有那份日志、内容就是正文
    const m = detail.match(/本轮日志目录：([^\s]+)/)
    assert.ok(m, `失败详情必须指出本轮日志目录：\n${detail}`)
    const runDir = path.join(repoRoot, m[1].replace(/\/$/, ''))
    assert.ok(fs.existsSync(runDir), `日志目录应已落盘：${runDir}`)
    const logs = fs.readdirSync(runDir).filter((f) => f.endsWith('.log'))
    assert.equal(logs.length, 1, `本轮应只有一份任务日志：${logs.join('、')}`)
    assert.ok(
      fs.readFileSync(path.join(runDir, logs[0]), 'utf8').includes('[ERROR] 演示断言'),
      '路径指向的文件里必须有同一段正文（路径不是装饰）',
    )
    assert.ok(detail.includes(`${m[1].replace(/\/$/, '')}/${logs[0]}`), `详情里应给出日志文件路径：${m[1]}/${logs[0]}`)
  } finally {
    demo.dispose()
  }
})

test('★ 每轮一个日志目录：跑第二轮不许冲掉第一轮的失败证据', () => {
  const demo = withDemoStage()
  const logRoot = path.join(repoRoot, '.workbuddy-ai', 'gate-logs')
  /** 从门禁自己的输出里读本轮目录——**不列目录**。
   *  列目录会被并发跑的门禁进程干扰（门内测试会 spawn 门禁，同一秒里可能多出目录），
   *  而门禁自己印的那一行就是权威（它印的就是它写的那个目录）。 */
  const runDirFrom = (out) => {
    const m = stripAnsi(out).match(/本轮日志目录：([^\s]+)/)
    assert.ok(m, `输出里应有本轮日志目录：\n${stripAnsi(out)}`)
    return m[1].replace(/\/$/, '')
  }
  try {
    const r1 = runGate([`--only=${DEMO_STAGE}`], demo.env)
    assert.notEqual(r1.status, 0, '第一轮应当失败')
    const firstRel = runDirFrom(r1.stdout + r1.stderr)
    const firstDir = path.join(repoRoot, firstRel)
    const firstLogs = fs.readdirSync(firstDir).filter((f) => f.endsWith('.log'))
    assert.equal(firstLogs.length, 1, `第一轮日志应已落盘：${firstLogs.join('、')}`)

    // 第二轮（换个阶段：判定不同、同样很快，且不会与第一轮同秒碰撞）
    const r2 = runGate(['--only=baseline'])
    assert.equal(r2.status, 0, `baseline 应当通过：${stripAnsi(r2.stdout + r2.stderr)}`)
    const secondRel = runDirFrom(r2.stdout + r2.stderr)
    assert.notEqual(secondRel, firstRel, '第二轮必须是**另一个**目录（否则就是覆盖）')

    // 关键断言：第一轮的目录与其中的失败日志**还在**
    assert.ok(fs.existsSync(firstDir), '第二轮不许冲掉第一轮的日志目录')
    assert.ok(
      fs.readFileSync(path.join(firstDir, firstLogs[0]), 'utf8').includes('[ERROR] 演示断言'),
      '第一轮的失败正文必须仍可读',
    )
    // 指针更新到第二轮，且指向真实存在的目录
    const ptr = fs.readFileSync(path.join(logRoot, 'latest-run.txt'), 'utf8').trim()
    assert.equal(ptr, path.basename(secondRel), '指针必须更新到最新一轮')
    assert.ok(fs.existsSync(path.join(logRoot, ptr)), '指针指向的目录必须存在')
  } finally {
    demo.dispose()
  }
})

test('★ 注入的阶段不进仓：演示阶段落在临时目录，`scripts/gate.d/` 里始终只有真阶段', () => {
  // 这条钉的是「测试不许改门禁自己的活配置」——它曾是真实的坑：演示阶段写进
  // `scripts/gate.d/`，于是 `gate-wiring-audit` 的 GW-005 会把它当成未登记阶段判红
  // （并发下随机），清理失败还会把它永久留在仓里。判据分两半：
  //   ① 注入**真的生效**（`--list` 里能看到它）——否则「干净」可能只是因为注入空转；
  //   ② 活目录**一个文件都没多**。
  const gateD = path.join(repoRoot, 'scripts', 'gate.d')
  const before = fs.readdirSync(gateD).sort()
  const demo = withDemoStage()
  try {
    const r = runGate(['--list'], demo.env)
    assert.equal(r.status, 0, `--list 应退出 0：${stripAnsi(r.stdout + r.stderr)}`)
    assert.match(stripAnsi(r.stdout), new RegExp(DEMO_STAGE), '注入的阶段必须真的被加载（否则这条判据在验一个空转）')
    assert.deepEqual(fs.readdirSync(gateD).sort(), before, '注入不许往 scripts/gate.d/ 写任何东西')
  } finally {
    demo.dispose()
  }
})

test('★ 懒建日志目录：`--help` 与被拒绝的调用不留空目录（否则门禁测试会撒一地空壳）', () => {
  const logRoot = path.join(repoRoot, '.workbuddy-ai', 'gate-logs')
  const snapshot = () =>
    fs.existsSync(logRoot)
      ? fs
          .readdirSync(logRoot, { withFileTypes: true })
          .filter((e) => e.isDirectory())
          .map((e) => e.name)
          .sort()
          .join(',')
      : ''
  const before = snapshot()
  runGate(['--help'])
  runGate(['--only=zz-not-a-stage']) // 拼错的 id ⇒ 退出 2，且必须发生在建目录之前
  runGate(['--only=']) // 空清单 ⇒ 同上
  assert.equal(snapshot(), before, '这些调用一个阶段都没跑，不该留下日志目录')
})

test('★ 阶段级并发必须连成**一批**：中间夹一个串行阶段就把批切断（docs 会整段串行在 backend 之后）', async () => {
  // 这条判据来自一次实测：`facts-write` 曾叫 `28-`，正夹在 `20-frontend-static`（并行）
  // 与 `30-docs`（并行）之间，于是门禁实际跑成
  //   [backend ∥ frontend-static] → facts-write → [docs ∥ baseline ∥ msrv]
  // 而 `gate.mjs` 文件头声明的是一整批。后果不是报错，是**白等一个 docs 的墙钟**
  // （本机实测 6m25s）——没有任何信号，只有把阶段墙钟打出来才看得见。
  //
  // 判据取「所有 `parallel: true` 的阶段必须连续」而不是「等于某个写死的名单」：
  // 名单会随阶段增删漂移，而「连续」是批语义本身。真需要第二个批时这条会红，
  // 那时该做的是**同时更新 `gate.mjs` 文件头声明的编排**，而不是把判据改宽。
  const dir = path.join(repoRoot, 'scripts', 'gate.d')
  const stages = []
  for (const f of fs.readdirSync(dir).filter((f) => f.endsWith('.mjs') && !f.startsWith('_')).sort()) {
    const mod = await import(pathToFileURL(path.join(dir, f)).href)
    if (mod.default?.id) stages.push({ file: f, id: mod.default.id, parallel: Boolean(mod.default.parallel) })
  }
  const flags = stages.map((s) => s.parallel)
  const first = flags.indexOf(true)
  const last = flags.lastIndexOf(true)
  assert.ok(first >= 0, `应有并发阶段：${stages.map((s) => s.file).join(', ')}`)
  const intruders = stages.slice(first, last + 1).filter((s) => !s.parallel)
  assert.deepEqual(
    intruders.map((s) => s.file),
    [],
    `并发批被串行阶段切断：${intruders.map((s) => s.file).join(', ')}——它把批分成两半，` +
      '两侧各自串行。要么把它移出这批（换文件名序），要么说明它为什么能并发（标 parallel）。',
  )
  // 具体锚点（上面那条是通用判据，这条钉住**当前编排的非显然部分**）：
  // docs 声明 `parallel: true` 的**唯一理由**是「藏在 backend 的墙钟里」——
  // 两者不在同一批，那 6m+ 就是纯加项。
  const idx = (id) => stages.findIndex((s) => s.id === id)
  assert.ok(idx('backend') >= 0 && idx('docs') >= 0, 'backend / docs 阶段必须存在')
  assert.ok(
    idx('backend') < idx('docs') && stages.slice(idx('backend'), idx('docs')).every((s) => s.parallel),
    'backend 与 docs 之间不得夹串行阶段（否则 docs 的墙钟直接加到总耗时上）',
  )
  // 生成器必须先于静态审计（`30-docs` 的 D-007 / RF-003 要看已展开的最终形态）
  assert.ok(idx('facts-write') < idx('docs'), 'facts-write 必须在 docs 之前')
  assert.equal(stages[0].id, 'fmt', 'fmt 是屏障，必须第一个跑')
})

// ── 机器能力档案（`gate.d/_machine.mjs`） ────────────────────────────────
//
// 这一组判据钉的是**「自适应」这件事本身不许退化**。它最危险的失效方式不是报错，
// 而是**悄悄退回常量**：门禁照样全绿、照样能跑，只是换一台机器就不再快了——
// 而没人会注意到，因为「慢」从来不触发任何红灯。上一版的 `concurrency: 8` /
// `SCENARIO_CONCURRENCY = 2` / 一批按本机拍的 timeout 就是这么做出来的。

test('★ 并发度随机器能力单调：核多不降、内存少要降、机器忙要降', () => {
  const base = { memGB: 16, freeMemGB: 5, cpuMs: 43, busy: 0.1 }
  const at = (n, extra = {}) => derive({ ...base, logicalCores: n, ...extra })

  const seq = [2, 4, 8, 16].map((n) => at(n).io)
  for (let i = 1; i < seq.length; i++) {
    assert.ok(seq[i] >= seq[i - 1], `io 并发必须随核数单调不减：${seq.join(' → ')}`)
  }
  // 内存钳：8 核但只有 8GB，不能按 8 核给满
  assert.ok(
    at(8, { memGB: 8, freeMemGB: 3 }).cpu < at(8).cpu,
    '8GB 机器的 cpu 并发必须低于 16GB 机器——否则 8 个 rustc 会 swap，比串行还慢',
  )
  // 负载钳：同一台机器已经被别的东西占满时，不能再开满
  assert.ok(
    at(8, { busy: 0.9 }).io < at(8, { busy: 0.1 }).io,
    '启动时负载 90% 必须降并发——「同一台机器不同时段」也是「不同性能的电脑」',
  )
})

test('★ 32 核机器上不许仍然只开 8 路', () => {
  const big = derive({ logicalCores: 32, memGB: 64, freeMemGB: 50, cpuMs: 20, busy: 0.1 })
  assert.ok(big.cpu > 8, `32 核机器 cpu 并发只有 ${big.cpu}——上限 8 是旧机器的核数，不是性质`)
  assert.ok(big.io > 8, `32 核机器 io 并发只有 ${big.io}`)
  // 但 e2e 的上限是**用例结构**决定的（最慢单例 ≈ 总工作量/5.7），不随核数涨
  assert.equal(big.e2e, 6, 'e2e 并发不该超过 6——那是用例结构，不是机器能力')
})

test('★ 换机器必须换指纹（否则另一台机器的耗时会被当成本机的预算）', () => {
  const fp = (o) => fingerprintOf({ platform: 'win32', arch: 'x64', cpuModel: 'X', ...o })
  const a = fp({ logicalCores: 8, memGB: 16 })
  const b = fp({ logicalCores: 16, memGB: 32 })
  assert.notEqual(a, b, '核数不同必须指纹不同')
  assert.notEqual(fp({ logicalCores: 8, memGB: 8 }), a, '内存不同必须指纹不同')
  assert.notEqual(fp({ logicalCores: 8, memGB: 16, cpuModel: 'Y' }), a, '换 CPU 必须指纹不同')
  // 同一个输入必须稳定（否则每轮都在丢历史，永远攒不到中位数）
  assert.equal(fp({ logicalCores: 8, memGB: 16 }), a, '同一台机器两次必须同指纹')
})

test('★ 超时预算：只放大不缩小，且有上限（防止「越超时越宽松」的正反馈）', () => {
  const withHistory = (ms) => ({ cpuScale: 1, calibration: { tasks: { k: { ms } } } })
  assert.equal(budget(withHistory([10]), 'k', 1000), 1000, '实测很快时不许把预算缩到 25ms')
  assert.equal(budget(withHistory([1000]), 'k', 1000), 2500, '实测 1s ⇒ 预算 2.5s')
  assert.equal(budget(withHistory([100000]), 'k', 1000), 6000, '再慢也不许超过静态基准的 6 倍')
  // 没有历史 ⇒ 按单核探针放大（只放大不缩小）
  assert.equal(budget({ cpuScale: 3, calibration: { tasks: {} } }, 'k', 1000), 3000)
  assert.equal(budget({ cpuScale: 0.5, calibration: { tasks: {} } }, 'k', 1000), 1000)
})

test('★ 机器档案必须能打印成人读的一行，且探测成本可忽略', async () => {
  const m = await machine()
  const line = describe(m)
  assert.match(line, /逻辑核/, `档案要写明核数：${line}`)
  assert.match(line, /并发 cpu \d+/, `档案要写明选定的并发：${line}`)
  assert.match(line, /GB/, `档案要写明内存：${line}`)
  // 探测是「为省时间而花的时间」，必须远小于它省下的时间（门禁一轮 4–5 分钟）。
  assert.ok(m.probeMs < 2000, `探测花了 ${m.probeMs}ms——它自己正在变成新的性能问题`)
})

test('★ 阶段的并发度必须声明**类型**，不许写死数字', async () => {
  // 「写死一个数」的失效方式最隐蔽：它在那台机器上是对的，换机器只是变慢，不报错。
  // 所以判据不是「值是否合理」（无法静态判断），而是「有没有留下类型声明」。
  for (const name of ['30-docs', '55-verify']) {
    const mod = await import(pathToFileURL(path.join(repoRoot, 'scripts', 'gate.d', `${name}.mjs`)).href)
    assert.ok(
      ['cpu', 'io'].includes(mod.default.concurrencyKind),
      `${name} 没声明 concurrencyKind——并发度必须由机器档案解析，不能是常量`,
    )
    assert.equal(
      mod.default.concurrency,
      undefined,
      `${name} 仍然写死了 concurrency: ${mod.default.concurrency}——那是某台机器的快照，不是性质`,
    )
  }
})
