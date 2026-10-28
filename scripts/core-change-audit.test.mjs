//! `core-change-audit` 的判据回归：两条规则各配正例与反例，外加喂料层的解析。
//!
//! 判据本身是**纯函数**（`judge(sha, files, message)`），`parseCommitRecords(out)` 是它
//! 上游那层「把 git 的 `--raw` 输出变成三元组」的解析，`requiresRange(args, env)` 是
//! 它下游那层「没有 base 可判时算跳过还是算判红」，`validateBaseline(entries, decisions)`
//! 则是「豁免名单自己合不合法」——四者都在这里直接喂样本，
//! 不起临时仓库、不跑 git。
//!
//! 解析层原来没有测试，注释写的是「喂进来的形状由门禁实跑覆盖」。批量化改的恰恰就是
//! 这一层，第一版把正文切到只剩标题、`Core-Change:` trailer 全丢，23 条提交被误判红——
//! 只有真跑一遍才露得出来，而真跑一遍的代价是整轮门禁。所以下面钉的是**形状本身**，
//! 夹具取自真实 `git log --raw` 输出，不是想象出来的。

import { test } from 'node:test'
import assert from 'node:assert/strict'
import { judge, parseCommitRecords, requiresRange, validateBaseline } from './core-change-audit.mjs'

const SHA = 'deadbeefdeadbeefdeadbeefdeadbeefdeadbeef'
const CORE = 'crates/symbio-core/src/lib.rs'
const KERNEL = 'crates/symbio-kernel/src/event/mod.rs'
const PLUGIN = 'crates/symbio-plugin-web/src/plugin.rs'

/** 一条合规的提交信息（标题 + 门禁段），用来隔离出「trailer 有无」这一件事 */
function msg(extra = '') {
  return `feat(x): 标题\n\n1. 改了什么\n\n门禁：全绿\n${extra}`
}

test('CC-001 不触发：没动 core/kernel 的提交不需要任何说明', () => {
  const r = judge(SHA, [PLUGIN], msg())
  assert.deepEqual(r.errors, [])
})

test('CC-001 触发：动 core 却没有 `Core-Change:`', () => {
  const r = judge(SHA, [CORE], msg())
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /^CC-001/)
})

test('CC-001 触发：动 kernel 同样算 core', () => {
  const r = judge(SHA, [KERNEL], msg())
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /^CC-001/)
})

// 口径边界：只判源码。内核的 `Cargo.toml` 每轮依赖整理都会变，把它算进来会让这条判据
// 退化成「每次都填表」；而清单膨胀的后果由 C-002 / C-004 从结果面判。
test('CC-001 不触发：只改了 core 的清单 / README（没有 `.rs`）', () => {
  const r = judge(
    SHA,
    ['crates/symbio-core/Cargo.toml', 'crates/symbio-core/README.md'],
    msg(),
  )
  assert.deepEqual(r.errors, [])
})

test('CC-001 触发：清单与源码同批改动，仍按源码算', () => {
  const r = judge(SHA, ['crates/symbio-core/Cargo.toml', CORE], msg())
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /^CC-001/)
})

test('CC-002 触发：既动插件又动 core，理由里没有决策号', () => {
  const r = judge(SHA, [CORE, PLUGIN], msg('\nCore-Change: 顺手补了个分支\n'))
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /^CC-002/)
})

test('通过：既动插件又动 core，理由挂决策号', () => {
  const r = judge(SHA, [CORE, PLUGIN], msg('\nCore-Change: D-195 共享类型只能住机制层\n'))
  assert.deepEqual(r.errors, [])
})

test('通过：只动 core，理由可以是自由文本（决策号只在「扩能力」这一档强制）', () => {
  const r = judge(SHA, [CORE], msg('\nCore-Change: 纯注释整理，无语义变化\n'))
  assert.deepEqual(r.errors, [])
})

test('认全角冒号：`Core-Change：D-195` 不是漏网的另一种写法', () => {
  const r = judge(SHA, [CORE, PLUGIN], msg('\nCore-Change：D-195 理由\n'))
  assert.deepEqual(r.errors, [])
})

test('决策号允许三位以上（D-1000 不该被当成没写）', () => {
  const r = judge(SHA, [CORE, PLUGIN], msg('\nCore-Change: D-1000 理由\n'))
  assert.deepEqual(r.errors, [])
})

test('provider / testkit 也算「扩能力」侧的证据', () => {
  for (const f of ['crates/symbio-provider-vdfs_service/src/lib.rs', 'crates/symbio-testkit/src/lib.rs']) {
    const r = judge(SHA, [CORE, f], msg('\nCore-Change: 随手改\n'))
    assert.equal(r.errors.length, 1, f)
    assert.match(r.errors[0], /^CC-002/)
  }
})

test('reason 里只写「D-」不算：必须是完整决策号形态', () => {
  const r = judge(SHA, [CORE, PLUGIN], msg('\nCore-Change: D- 理由\n'))
  assert.equal(r.errors.length, 1)
  assert.match(r.errors[0], /^CC-002/)
})

// ---------------------------------------------------------------- 解析层（喂料）

/**
 * 造一条真实形状的 `git log --raw` 记录：`\x01<sha>\x00<正文>\n\n<文件行…>`。
 *
 * 空行是 git 自己给的（`%B` 之后接 raw 块），不是夹具的装饰——正文与文件记录的
 * 分界正是这一层唯一需要判定的事。
 */
function rec(sha, message, files = []) {
  const lines = files.map(
    (f) =>
      typeof f === 'string'
        ? `:100644 100644 3479b6d5 a34de8f1 M\t${f}`
        : `:${f.m1} ${f.m2} 3479b6d5 a34de8f1 ${f.st}\t${f.path}`,
  )
  return `\x01${sha}\x00${message}${lines.length ? `\n\n${lines.join('\n')}` : ''}`
}

const FULL_SHA = 'a1b2c3d4e5f6a7b8c9d0a1b2c3d4e5f6a7b8c9d0'

test('解析：`Core-Change:` trailer 住在正文末尾，必须完整读进 message', () => {
  // 这一条就是批量化第一版踩到的坑：正文被切到只剩标题 ⇒ trailer 全丢 ⇒ 判红。
  const body = `feat(kernel): 标题\n\n1. 改了什么\n   续行。\n\n门禁：全绿\n\nCore-Change: D-195 共享类型只能住机制层`
  const [c] = parseCommitRecords(rec(SHA, body, [CORE, PLUGIN]))
  assert.equal(c.sha, SHA)
  assert.match(c.message, /Core-Change: D-195/)
  assert.deepEqual(judge(c.sha, c.files, c.message).errors, [])
})

test('解析：多段正文逐字保留，不被压成标题', () => {
  const body = 'refactor(x): 标题\n\n第一段\n\n第二段\n   缩进的续行\n\n结论。\n\n门禁：全绿'
  const [c] = parseCommitRecords(rec(SHA, body, [KERNEL]))
  // 真实输出里 raw 块前有一行空行分隔，解析后正文尾部会留下它；判据只看 trailer 在不在。
  assert.equal(c.message.replace(/\n+$/, ''), body)
})

test('解析：文件清单取自 `:` 起头的 raw 行，按 `\t` 切出路径', () => {
  const [c] = parseCommitRecords(
    rec(SHA, 'chore: t', [KERNEL, 'docs/003-design/015-义务环.md']),
  )
  assert.deepEqual(c.files, [KERNEL, 'docs/003-design/015-义务环.md'])
})

test('解析：删除记录（源模式全 0、状态 D）照样算改动文件', () => {
  const [c] = parseCommitRecords(
    rec(SHA, 'chore: t', [{ m1: '100644', m2: '000000', st: 'D', path: 'docs/007-task/016-x.md' }]),
  )
  assert.deepEqual(c.files, ['docs/007-task/016-x.md'])
})

test('解析：没有文件记录的提交 `files` 为空数组（不是 undefined）', () => {
  const [c] = parseCommitRecords(rec(SHA, 'docs: 只改信息'))
  assert.deepEqual(c.files, [])
  assert.equal(c.message, 'docs: 只改信息')
})

test('解析：正文里以 `:` 开头的行不是文件记录，不得截断 message', () => {
  // `:列表项` 与 `:100644 100644 …` 的区别只在「两个定长八进制模式字段」；
  // 只认行首 `:` 的实现会把 trailer 连同后半正文一起当成文件清单丢掉。
  const body = 'feat(x): 标题\n\n口径：\n:1 不是文件行\n: 也不是\n\n门禁：全绿\n\nCore-Change: 纯注释整理'
  const [c] = parseCommitRecords(rec(SHA, body, [CORE]))
  assert.match(c.message, /Core-Change: 纯注释整理/)
  assert.deepEqual(c.files, [CORE])
  assert.deepEqual(judge(c.sha, c.files, c.message).errors, [])
})

test('解析：多条记录按序全部解出，空片段跳过', () => {
  const out = rec(SHA, 'a: 一', [CORE]) + rec('f'.repeat(40), 'b: 二')
  const all = parseCommitRecords(out)
  assert.deepEqual(all.map((c) => c.sha), [SHA, 'f'.repeat(40)])
  assert.equal(all.length, 2)
  assert.equal(parseCommitRecords('').length, 0)
})

test('解析→判定：历史样本（动 kernel、正文无 trailer）判 CC-001', () => {
  // 正文与文件行取自当年真实触发 CC-001 的那一笔（后来补了 `Core-Change:` trailer，
  // 原样保留在这里作反例）；sha 换成形状合法的占位值——判据不看 sha 指向谁。
  const body =
    'feat(loop): tool 端口的在途可观测兑现——派发与收应答分处两个环步\n\n' +
    '1. 裁决（D-124 订正 1，落 DE-015）：装载期第 6 条后半对「出口全是引擎格」的行改判\n' +
    '   「有没有交出去的凭据」。\n\n' +
    '门禁：node scripts/gate.mjs 161/162 通过、e2e 86/86。\n\n' +
    '结论落 DE-015（D-124 订正 1）。'
  const files = [
    'crates/symbio-kernel/src/decl/ring.rs',
    'crates/symbio-plugin-loop/src/tool_dispatch.rs',
    'docs/003-design/015-义务环.md',
  ]
  const [c] = parseCommitRecords(rec(FULL_SHA, body, files))
  assert.equal(c.files.length, 3)
  const r = judge(c.sha, c.files, c.message)
  assert.deepEqual(r.errors.map((e) => e.slice(0, 6)), ['CC-001'])
})

// ── 范围缺失时算跳过还是算判红 ──
//
// 这条判据的失效形态不是「判错」，是「在自动化里静默不判」：浅克隆 / 没拉 base 的
// checkout 走的是同一条出路，而它退 0 的样子和通过一模一样。红线原先只挂在
// `--require-range` 上，而 workflow 按脚本名跑门禁、从没拼过这个参数。
test('CI 环境里「没有 base 可判」判红，不需要调用方传参', () => {
  assert.equal(requiresRange([], { CI: 'true' }), true)
  assert.equal(requiresRange([], { GITHUB_ACTIONS: 'true' }), true)
})

test('本地无 CI 变量仍然只是跳过（不逼开发者配 base ref）', () => {
  assert.equal(requiresRange([], {}), false)
  // 值为 false / 0 不算自动化：有人在本地导出过 CI=false 来跑脚本
  assert.equal(requiresRange([], { CI: 'false' }), false)
  assert.equal(requiresRange([], { CI: '0' }), false)
})

test('显式 `--require-range` 与 CI 变量取并集，本地也能强制', () => {
  assert.equal(requiresRange(['--require-range'], {}), true)
  assert.equal(requiresRange(['--require-range'], { CI: 'false' }), true)
})

// ── 追溯豁免名单自己的形状（`validateBaseline`） ──
//
// 「不追改已推送的历史」是 CC-001 唯一诚实的退路，而退路会被滥用：一条不解释自己的
// 豁免三个月后就变成永久后门，且没人看得出它豁免的是什么。所以名单的形状由脚本判。
const D = new Set(['D-124'])

test('晚于判据的豁免没写裁决号 ⇒ 红（豁免顶替的就是那条缺失的 Core-Change）', () => {
  const errors = validateBaseline(
    [{ sha: 'abc1234', beforeGuard: false, why: '已推到 origin/v2，不改历史', decision: '' }],
    D,
  )
  assert.equal(errors.length, 1, JSON.stringify(errors))
  assert.match(errors[0], /裁决/)
})

test('裁决号在设计文档里打不开 ⇒ 红（写错的决策号等于没写）', () => {
  const errors = validateBaseline(
    [{ sha: 'abc1234', beforeGuard: false, why: '已推送', decision: '裁决 D-99999' }],
    D,
  )
  assert.equal(errors.length, 1, JSON.stringify(errors))
  assert.match(errors[0], /打不开/)
})

test('豁免没有理由正文 ⇒ 红；一条带决策号的合法条目 ⇒ 无错', () => {
  assert.match(
    validateBaseline([{ sha: 'abc1234', beforeGuard: true, why: '   ' }], D)[0] ?? '',
    /永久后门/,
  )
  const ok = validateBaseline(
    [
      { sha: 'abc1234', beforeGuard: true, why: '判据诞生于它之后，不可能自带 trailer' },
      { sha: 'def5678', beforeGuard: false, why: '已推到 origin/v2 ⇒ 补 trailer 要重写历史', decision: '裁决 D-124' },
    ],
    D,
  )
  assert.deepEqual(ok, [])
})

test('sha 形状不合法（漏写 / 写错）⇒ 红，且不继续查它的理由', () => {
  const errors = validateBaseline([{ sha: '', beforeGuard: true, why: '' }], D)
  assert.equal(errors.length, 1, JSON.stringify(errors))
  assert.match(errors[0], /提交号/)
})
