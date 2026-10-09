// `scripts/gate.mjs` 的回归测试。
//
// 这个脚本的失效方式不是「报错」而是「代价」：`--help` 不被识别时，它会照常跑完
// **全量门禁**（10+ 分钟）并把 `.workbuddy-ai/gate-logs/*.log` 覆盖掉——查一次用法，
// 代价是丢掉上一次的失败日志（最需要它的那一刻它没了，只能重跑）。
//
// 与 `commit.mjs` 同类陷阱（那里 `--help` 会**真的提交一次**）。两处的判据形态相同：
// 退出 0 + 打印用法 + **没有发生真实动作**（这里 = 没进门禁主流程 / 没跑任何阶段）。
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
// 所以从别的仓库调用它，跑的是**本仓**的门禁——写它的 `target/`、覆盖它的
// `.workbuddy-ai/gate-logs/`（冲掉上一轮的失败日志），本地模式还替它 `git add`。
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
