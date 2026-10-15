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
import { stripAnsi } from './color.mjs'

const repoRoot = path.resolve(import.meta.dirname, '..')
const gateScript = path.join(repoRoot, 'scripts', 'gate.mjs')

/**
 * 跑 `gate.mjs`：stdin 给 `'ignore'`（真交互 / 真等待会挂），并给**显式 timeout**——
 * 失败要快，不许把「白跑十分钟门禁」这个 bug 拖成一次十分钟的测试。
 */
function runGate(args = []) {
  return spawnSync(process.execPath, [gateScript, ...args], {
    cwd: repoRoot,
    encoding: 'utf8',
    timeout: 60_000,
    stdio: ['ignore', 'pipe', 'pipe'],
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
// 做法：往 `scripts/gate.d/` 里临时放一个**必然失败**的阶段，用 `--only` 跑它，
// 断言输出与落盘；`finally` 里删掉。用真阶段而不是 mock——`report()` 的呈现逻辑
// 只有端到端才走得到（`run` 的日志落盘、`record`、汇总全部参与）。

/** 临时阶段的 id / 文件名（刻意用 `zz-` 前缀：排序在最后，且一眼能看出是注入的）。 */
const DEMO_STAGE = 'zz-failure-detail-demo'
const DEMO_STAGE_FILE = path.join(repoRoot, 'scripts', 'gate.d', `${DEMO_STAGE}.mjs`)

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
        "for (let i = 0; i < 60; i++) console.log('噪声行 ' + i);" +
          "console.log('[ERROR] 演示断言：期望 3 得到 4'); process.exit(1)",
      ],
    },
  ],
}
`

test('★ 失败项：日志末几行与相对路径直接印在汇总里（不必再跑一次门禁）', () => {
  fs.writeFileSync(DEMO_STAGE_FILE, demoStageSource(), 'utf8')
  try {
    const r = runGate([`--only=${DEMO_STAGE}`])
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
    fs.rmSync(DEMO_STAGE_FILE, { force: true })
  }
})

test('★ 每轮一个日志目录：跑第二轮不许冲掉第一轮的失败证据', () => {
  fs.writeFileSync(DEMO_STAGE_FILE, demoStageSource(), 'utf8')
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
    const r1 = runGate([`--only=${DEMO_STAGE}`])
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
    fs.rmSync(DEMO_STAGE_FILE, { force: true })
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
