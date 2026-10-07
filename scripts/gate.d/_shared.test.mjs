// `gate.d/_shared.mjs` 的回归测试。
//
// 这里只覆盖 `autoWork`——门禁的「自动执行的工作」原语。它值得有回归测试的理由，
// 与 `30-docs.mjs` 开头那句是同一个：**一个只会亮绿灯的机制等于没有机制**。
// `autoWork` 的失效方式恰恰是「看起来在修、其实没把修复带进提交」，而这一点
// 在正常流程里看不出来（本地跑门禁 → 文件确实被格式化了 → 一切正常），
// 只在「修复前就已脏/已暂存」时暴露。这个设计第一版就写反了方向，见下。
//
// 跑法：node --test scripts/gate.d/_shared.test.mjs

import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { autoWork, cargoTestRatchet } from './_shared.mjs'

const repoRoot = path.resolve(import.meta.dirname, '../..')

// ================= 一次性临时仓库 =================
//
// 用真实 `git` 而不是打桩：`autoWork` 的全部行为都建立在
// `git status --porcelain -z` 与 `git add` 的真实语义上，打桩等于把被测对象换掉。
//
// 所有用例共用**一个**仓库、各自用**不同的文件**：`dirtyPaths` 是仓库级的，
// 但只要某用例的 `run` 只写自己的文件，别人的脏文件既不会被改写、也就不会被暂存。

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-autowork-'))

const git = (args) =>
  spawnSync('git', args, { cwd: root, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 })

git(['-c', 'init.defaultBranch=main', 'init', '-q'])
git(['config', 'user.email', 'test@example.com'])
git(['config', 'user.name', 'test'])
git(['config', 'core.autocrlf', 'false'])
// 被测文件**先以「已格式化」的样子进初始提交**，这样它们都是**已跟踪**文件，
// 「已暂存 / 未暂存」才有明确含义。（若用新建文件，`git diff --cached` 会因为
// 「相对 HEAD 是新增」而恒为真，判据就废了。）
const SEED = {
  't1.rs': 'fn a() {}\n',
  't2.rs': 'fn b() {}\n',
  't4-untouched.rs': 'fn u() {}\n',
  't7.rs': 'fn g() {}\n',
  't8.rs': 'fn h() {}\n',
}
for (const [p, c] of Object.entries(SEED)) fs.writeFileSync(path.join(root, p), c)
git(['add', '-A'])
git(['commit', '-q', '-m', 'init'])

after(() => {
  // 尽力清理。沙箱的 safe-delete 垫片在「单回合删除数过多」时会拒删并抛——
  // 那是环境限制，不是测试失败；临时目录交给 OS 回收。
  try {
    fs.rmSync(root, { recursive: true, force: true, maxRetries: 3 })
  } catch {
    /* 环境限制，忽略 */
  }
})

// ================= 小工具 =================

const abs = (p) => path.join(root, p)
const write = (p, c) => fs.writeFileSync(abs(p), c)
const read = (p) => fs.readFileSync(abs(p), 'utf8')
const porcelain = (p) => (git(['status', '--porcelain', '--', p]).stdout || '').replace(/\r?\n$/, '')
/** 索引 vs HEAD：这次提交**会**带上这个文件的改动。 */
const isStaged = (p) => Boolean((git(['diff', '--cached', '--name-only', '--', p]).stdout || '').trim())
/** 工作区 vs 索引：还有**未暂存**的改动（即「脏」）。 */
const worktreeDiffers = (p) => Boolean((git(['diff', '--name-only', '--', p]).stdout || '').trim())
const stagedContent = (p) => git(['show', `:${p}`]).stdout

/** 模拟一个「确定性的机械工作」：把 writes 里的内容写进文件。 */
const worker = (writes) => async () => {
  for (const [p, c] of Object.entries(writes)) write(p, c)
  return { ok: true, code: 0, signal: null, output: '', timedOut: false }
}
const brokenWorker = () => async () => ({
  ok: false,
  code: 1,
  signal: null,
  output: 'boom',
  timedOut: false,
})

const work = (run, ci = false) =>
  autoWork({ repoRoot: root, ci, run }, { label: 'probe', cmd: 'noop', args: [], cwd: root })

// ================= 用例 =================

test('不改写任何文件 ⇒ 通过、不暂存、无 note', async () => {
  const f = 't1.rs'
  write(f, 'fn a(){}\n') // 脏、未暂存（正常流程里提交前的常态）
  assert.equal(worktreeDiffers(f), true, '前置：工作区有未暂存改动')
  assert.equal(isStaged(f), false, '前置：索引里没有它')

  const r = await work(worker({ [f]: 'fn a(){}\n' })) // 修复结果 == 现状

  assert.equal(r.ok, true)
  assert.equal(r.note, undefined, '没有差异就不该有 note')
  assert.equal(isStaged(f), false, '没有差异就不该动索引')
  assert.equal(worktreeDiffers(f), true, '未暂存的改动仍留在工作区，不该被吞掉')
})

test('修复前就已脏（未暂存）+ 被改写 ⇒ 当场暂存（第一版就是在这里写反了方向）', async () => {
  const f = 't2.rs'
  write(f, 'fn b(){ }\n') // 脏、未暂存
  assert.equal(worktreeDiffers(f), true)
  assert.equal(isStaged(f), false)

  // ⚠️ 修复结果必须**不同于 HEAD**（`'fn b() {}\n'`）。若相同，文件修完就变干净、
  // 不再出现在「修复后」那一侧，也就没有东西可暂存——那时断言失败是数据的问题，不是实现的问题。
  const FIXED = 'fn b() {\n}\n'
  const r = await work(worker({ [f]: FIXED }))

  assert.equal(r.ok, true)
  assert.match(r.note ?? '', /自动修复/)
  assert.equal(isStaged(f), true, '⚠️ 修复前已脏的文件也必须进索引——否则提交里留下的是未格式化那一版')
  assert.equal(stagedContent(f), FIXED)
  assert.equal(worktreeDiffers(f), false, '修复后索引与工作区应一致')
})

test('修复前干净 + 修复产生新文件 ⇒ 暂存', async () => {
  const f = 't3.rs'
  assert.equal(porcelain(f), '', '前置：不存在')

  const r = await work(worker({ [f]: 'fn c() {}\n' }))

  assert.equal(r.ok, true)
  assert.equal(isStaged(f), true)
  assert.equal(stagedContent(f), 'fn c() {}\n')
})

test('修复前就脏、但修复没碰它 ⇒ 不暂存（门禁不替人决定哪些改动进本次提交）', async () => {
  const untouched = 't4-untouched.rs'
  const touched = 't4-touched.rs'
  write(untouched, 'fn u(){ }\n') // 人为的、与格式化无关的未提交改动
  assert.equal(worktreeDiffers(untouched), true)
  assert.equal(isStaged(untouched), false)

  const r = await work(worker({ [touched]: 'fn t() {}\n' }))

  assert.equal(r.ok, true)
  assert.equal(isStaged(touched), true, '被修复改写的要暂存')
  assert.equal(isStaged(untouched), false, '没被碰的脏文件不该被顺手暂存')
  assert.equal(read(untouched), 'fn u(){ }\n', '也不该被改写')
})

test('幂等：第二次运行不再产生修复', async () => {
  const f = 't5.rs'
  write(f, 'fn e(){}\n')
  await work(worker({ [f]: 'fn e() {}\n' }))
  const r2 = await work(worker({ [f]: 'fn e() {}\n' }))
  assert.equal(r2.ok, true)
  assert.equal(r2.note, undefined)
})

test('命令本身报错 ⇒ 不通过，且带上退出码', async () => {
  const r = await work(brokenWorker())
  assert.equal(r.ok, false)
  assert.match(r.note ?? '', /exit=1/)
})

test('CI：有差异 ⇒ 报红、提示回本地、不动索引', async () => {
  const f = 't7.rs'
  write(f, 'fn g(){}\n')
  // 同 t2：修复结果要 ≠ HEAD，否则「跑完没有差异」，本用例就没在测 CI 分支。
  const r = await work(worker({ [f]: 'fn g() {\n}\n' }), true)

  assert.equal(r.ok, false, 'CI 无法提交，只能判红')
  assert.match(r.note ?? '', /本地/)
  assert.equal(isStaged(f), false, 'CI 不得动索引')
})

test('CI：无差异 ⇒ 通过（不误报）', async () => {
  const f = 't8.rs'
  write(f, 'fn h() {}\n')
  const r = await work(worker({ [f]: 'fn h() {}\n' }), true)
  assert.equal(r.ok, true)
})

// ⚠️ 这条守卫针对的失效模式很具体：`autoWork` 的「CI 判红」完全依赖 `--ci`，
// 而 facts 阶段在 ci.yml 里跑的是 `--only=docs,facts`——若那里漏了 `--ci`，
// 这一步就变成「自动修复 + 暂存」并**静默放过漂移**，即一个只亮绿灯的检查项。
// 参数漏写不会报错、不会变慢、本地也复现不出来，只能靠断言钉住。
test('ci.yml 里包含 facts 阶段的 gate 调用必须传 --ci', () => {
  const yml = fs.readFileSync(path.join(repoRoot, '.github/workflows/ci.yml'), 'utf8')
  const calls = yml.split('\n').filter((l) => l.includes('gate.mjs') && !l.trim().startsWith('#'))
  assert.ok(calls.length > 0, '没找到 gate.mjs 调用——守卫失效了')
  const factsCalls = calls.filter((l) => /only=[^\s]*facts/.test(l))
  assert.ok(factsCalls.length > 0, '没找到含 facts 阶段的 gate 调用——守卫失效了')
  for (const l of factsCalls) {
    assert.match(l, /--ci/, `facts 阶段必须传 --ci，否则漂移会静默通过：${l.trim()}`)
  }
})

// ⚠️ 下面两条钉的是**同一种失效模式：没有信号的漏跑**——漏跑时门禁与日志都一片绿，
// 事后只能靠人记住「今天本该跑的没跑」。
//
// 2026-10-07 实测：触发分支只有 `[ main, master ]`，而 `v2-plan` 领先 `main` 80 个
// 提交，`gh run list --branch v2-plan` **一条 run 都没有、也没有 PR**——1339 个 Rust
// 单测、42 个 e2e、14 个守卫的绿灯**只存在于开发者本机**。这与 ci.yml 文件头记的
// 「白名单漏掉 `*.css` ⇒ 四个 job 都不跑」是同一课的第二次：第一次漏的是文件类型，
// 这次漏的是分支。
//
// 为什么只能靠断言：分支名单写错不报错、不变慢、本地复现不出来（本地跑 gate 与
// CI 触发毫无关系）。且 `--ci` 只认这个命令行开关、**不读 `process.env.CI`**
// （gate.mjs `const CI = hasFlag('--ci')`），所以连「CI 环境里会自动生效」的想当然
// 都不成立。
test('ci.yml：v2-plan 必须在 push 与 pull_request 的触发分支里，且保留手动触发', () => {
  const yml = fs.readFileSync(path.join(repoRoot, '.github/workflows/ci.yml'), 'utf8')
  const branchLines = yml.split('\n').filter((l) => /^\s*branches:\s*\[/.test(l))
  assert.equal(
    branchLines.length,
    2,
    `期望 push / pull_request 各一个 branches 行，实得 ${branchLines.length}——守卫失效了`,
  )
  for (const l of branchLines) {
    assert.match(
      l,
      /v2-plan/,
      `触发分支里没有 v2-plan ⇒ 该分支的 push 永远不触发 CI，且**没有任何提示**：${l.trim()}`,
    )
  }
  // 手动触发是第三条路：分支不在名单里时，连补跑都做不到。
  assert.match(yml, /^ {2}workflow_dispatch:/m, '没有 workflow_dispatch ⇒ 分支名写错时连手动补跑都不可能')
})

// verify 阶段是 `docs/plan/verify/` 那 14 个「架构证据」的自动化 runner（C29 兑现）。
// 在此之前它们只能手敲 rustc 跑、结果抄进 OUTPUT.md 提交——**快照不是守卫**。
// 少了这个 job，证据是否仍然成立取决于有没有人记得去手跑一遍。
test('ci.yml：verify 阶段必须上 CI', () => {
  const yml = fs.readFileSync(path.join(repoRoot, '.github/workflows/ci.yml'), 'utf8')
  const calls = yml.split('\n').filter((l) => l.includes('gate.mjs') && !l.trim().startsWith('#'))
  const verifyCalls = calls.filter((l) => /only=[^\s]*verify/.test(l))
  assert.ok(
    verifyCalls.length > 0,
    '没有含 verify 阶段的 gate 调用 ⇒ 14 个验证程序退回「人手跑过一次」的状态',
  )
})

// ── cargoTestRatchet：两个独立 workspace 共用的「跑测试 + 通过数棘轮」 ──────
//
// 抽成共享函数就是为了让 `symbio` 与 `cli` 是**同一套判据**，所以这里钉的是
// **判据本身**，不是某一侧的调用：三条分支（低于 / 等于 / 高于基线）加上两条
// 退化路径（解析不到 / 命令失败）。其中「解析不到不判红」尤其要钉住——那是
// 刻意的：宁可漏报，也不因为**解析**失败把门禁变红（必然红的门禁比没有门禁更糟）。
const fakeCtx = (output, { ci = false, ok = true } = {}) => ({
  ci,
  run: async () => ({ ok, output, code: ok ? 0 : 1, timedOut: false }),
})
const ratchet = (output, opts, { baseline = 10, ciBaseline } = {}) =>
  cargoTestRatchet(fakeCtx(output, opts), {
    label: 'cargo test',
    cwd: '.',
    args: ['test'],
    baseline,
    baselineName: 'rustTests',
    // 关键：**不传就把这个键整个省掉**（而不是传 undefined 占位）——被测函数判的是
    // `ciBaseline === undefined`，两种「没给」必须走同一条路径。
    ...(ciBaseline === undefined ? {} : { ciBaseline, ciBaselineName: 'ciRustTestsTotal' }),
  })

test('cargoTestRatchet：通过数等于基线 ⇒ 通过且无 note', async () => {
  const r = await ratchet('test result: ok. 10 passed; 0 failed').run()
  assert.equal(r.ok, true)
  assert.equal(r.note, undefined)
})
test('cargoTestRatchet：通过数低于基线 ⇒ 判红（有测试被删或失败）', async () => {
  const r = await ratchet('test result: ok. 9 passed; 0 failed').run()
  assert.equal(r.ok, false)
  assert.match(r.note, /9 < 基线 10/)
})
test('cargoTestRatchet：通过数高于基线 ⇒ 通过但提示同步基线', async () => {
  const r = await ratchet('test result: ok. 11 passed; 0 failed').run()
  assert.equal(r.ok, true)
  assert.match(r.note, /基线待更新/)
})
test('cargoTestRatchet：解析不到通过数 ⇒ 通过并标注，不因解析失败判红', async () => {
  const r = await ratchet('no summary here').run()
  assert.equal(r.ok, true)
  assert.match(r.note, /未能解析/)
})
test('cargoTestRatchet：命令失败 ⇒ 判红并带退出码', async () => {
  const r = await ratchet('', { ok: false }).run()
  assert.equal(r.ok, false)
  assert.match(r.note, /exit=1/)
})
// ── CI 分支：2026-10-07 从「只信退出码」改为与 `ciBaseline` 比对 ────────────
//
// 原实现是 `if (ctx.ci) return { ok: true }`（注释还写着「数字仅作信息展示」）——
// **CI 这一侧从来没有棘轮**。本地删掉测试会被分包基线拦下，推上去 CI 却只信退出码，
// 照样绿。这正是「测试失败」与「测试消失」的分界：后者**不留任何痕迹**，退出码恒为 0。
// 而两条分支判据必须一致（抽成共享函数就是为了这个），否则本地与 CI 各自演化成两条规范。
test('cargoTestRatchet：CI 全量等于基线 ⇒ 通过且无 note', async () => {
  const r = await ratchet('test result: ok. 10 passed; 0 failed', { ci: true }, { ciBaseline: 10 }).run()
  assert.equal(r.ok, true)
  assert.equal(r.note, undefined)
})
test('cargoTestRatchet：CI 全量低于基线 ⇒ 判红（测试消失在 CI 也拦得住）', async () => {
  const r = await ratchet('test result: ok. 10 passed; 0 failed', { ci: true }, { ciBaseline: 12 }).run()
  assert.equal(r.ok, false)
  assert.match(r.note, /10 < 基线 12/)
})
test('cargoTestRatchet：CI 全量高于基线 ⇒ 通过但提示同步基线', async () => {
  const r = await ratchet('test result: ok. 11 passed; 0 failed', { ci: true }, { ciBaseline: 10 }).run()
  assert.equal(r.ok, true)
  assert.match(r.note, /基线待更新/)
})
// ⚠️ 这一条是本组的重点：**少写一个参数不该让守卫消失**。
// 「退回只信退出码」是这条守卫唯一会**静默**失效的方式——调用方漏写 `ciBaseline`，
// 不报错、不变慢、日志里只少一行字，CI 就此变回抓不住「测试消失」的状态。
test('cargoTestRatchet：CI 模式缺 ciBaseline ⇒ 判红（省略参数不让棘轮消失）', async () => {
  const r = await ratchet('test result: ok. 10 passed; 0 failed', { ci: true }).run()
  assert.equal(r.ok, false)
  assert.match(r.note, /缺 ciBaseline/)
})
// 解析不到 ⇒ 仍不判红，与本地同一口径（宁可漏报，也不因**解析**失败变红）。
test('cargoTestRatchet：CI 解析不到通过数 ⇒ 通过并标注', async () => {
  const r = await ratchet('no summary here', { ci: true }, { ciBaseline: 10 }).run()
  assert.equal(r.ok, true)
  assert.match(r.note, /未能解析/)
})
// CI 的实际输出形状是**多个测试目标各一行**，判据按求和而非首行——口径不同于本地
// 分包（那条取首个 result 行）。求和算错会让 1347 这类基线判在错误的数上。
test('cargoTestRatchet：CI 全量按各行求和（不是首行）', async () => {
  const out = 'test result: ok. 7 passed; 0 failed\ntest result: ok. 3 passed; 0 failed'
  const sum = await ratchet(out, { ci: true }, { ciBaseline: 10 }).run()
  assert.equal(sum.ok, true)
  assert.equal(sum.note, undefined)
  const low = await ratchet(out, { ci: true }, { ciBaseline: 11 }).run()
  assert.equal(low.ok, false)
  assert.match(low.note, /10 < 基线 11/)
})
