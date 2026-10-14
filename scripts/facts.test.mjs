/**
 * gen-facts（事实 runner）回归测试
 *
 * 跑法：node --test scripts/facts.test.mjs
 *
 * ## 它防的是哪种失效
 *
 * 这类 runner 的坏死方式**全是静默的**，而且方向一致——「什么都没生成」被当成
 * 「生成成功」：
 *
 *   - 目录读不出 / 被清空 ⇒ 零份事实 ⇒ 门禁退出 0、什么也没生成；
 *   - 某个事实模块的描述符坏了（缺 `out` / `render` 不是函数）⇒ 被静默跳过；
 *   - `out` 写了 `../../etc/x` ⇒ runner 变成一个任意写文件的口子，而它在 CI 里
 *     以仓库身份跑；
 *   - `render` 里掺了时间 ⇒ autoWork 的内容哈希每次都变，门禁永远报「修复了
 *     1 个文件」，而红的原因是生成器不确定，不是漂移；
 *   - 把回归测试当成一份事实 ⇒ 它没有描述符，于是「一个与事实无关的测试文件」
 *     把整轮生成拖红。
 *
 * 所以测试双向钉：**坏输入必须红**，**好输入必须绿且幂等**。只测后者的话，
 * 一个把所有事都吞掉的 runner 也能全绿。
 */
import { test } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { execFileSync } from 'node:child_process'

import { discover } from './gen-facts.mjs'
import { validateFact, stripStamp, emit, driftOf } from './facts/_lib.mjs'

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const RUNNER = path.join(REPO, 'scripts', 'gen-facts.mjs')

const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'gen-facts-'))

/** 一份最简、合法的事实模块源码（`id` 与文件名对齐）。 */
const goodSource = (name = 'good') =>
  [
    `export default {`,
    `  id: '${name}',`,
    `  title: '测试用事实',`,
    `  out: 'docs/${name}.md',`,
    `  render: () => 'hello\\n',`,
    `}`,
    '',
  ].join('\n')

/**
 * 在临时仓里搭一个**最小可跑**的 facts 目录并跑 runner，返回 `{ code, output }`。
 *
 * 只复制 runner 真正需要的三份文件（它自己 / color / 契约库），**不复制真实的事实
 * 模块**——真实模块 import `line-count.mjs`、`route-facts.mjs` 等本仓脚本，搬不干净
 * 会让「空目录」那个用例测到的其实是「模块加载失败」，而不是「目录里没有事实」。
 * 这类「测错了东西」的用例比没有更坏：它绿，而它声称防的那件事并未被防。
 */
function runIn(tmpRepo, facts = {}) {
  const scripts = path.join(tmpRepo, 'scripts')
  const dir = path.join(scripts, 'facts')
  fs.mkdirSync(dir, { recursive: true })
  fs.copyFileSync(RUNNER, path.join(scripts, 'gen-facts.mjs'))
  fs.copyFileSync(path.join(REPO, 'scripts', 'color.mjs'), path.join(scripts, 'color.mjs'))
  fs.copyFileSync(path.join(REPO, 'scripts', 'facts', '_lib.mjs'), path.join(dir, '_lib.mjs'))
  for (const [name, source] of Object.entries(facts)) {
    fs.writeFileSync(path.join(dir, `${name}.mjs`), source, 'utf8')
  }
  try {
    const stdout = execFileSync(process.execPath, [path.join(scripts, 'gen-facts.mjs')], {
      cwd: tmpRepo,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    return { code: 0, output: stdout }
  } catch (e) {
    return { code: e.status ?? -1, output: `${e.stdout ?? ''}${e.stderr ?? ''}` }
  }
}

// ── 契约校验（纯函数，逐条注入） ───────────────────────────────────

test('runner 被 import 时不执行（否则本文件会在 import 期被 process.exit 带走）', () => {
  // 这条不是形式：顶层 `process.exit(await main())` 会在 import 期结束测试进程，
  // 报出「1 个测试通过」而一条断言都没跑——本仓最难查的那种假绿。
  // 本测试能跑到自己，就说明 import 没有执行 runner。
  assert.equal(typeof discover, 'function')
})

test('discover：`_` 前缀不算一份事实（共享材料）', () => {
  const dir = tmp()
  for (const n of ['a.mjs', 'b.mjs', '_lib.mjs', 'notes.md']) {
    fs.writeFileSync(path.join(dir, n), '')
  }
  assert.deepEqual(discover(dir), ['a', 'b'])
})

test('discover：`.test.mjs` 也不算事实（回归测试住在同目录且没有描述符）', () => {
  const dir = tmp()
  for (const n of ['a.mjs', 'a.test.mjs', 'b.test.mjs']) {
    fs.writeFileSync(path.join(dir, n), '')
  }
  assert.deepEqual(discover(dir), ['a'])
})

test('discover：真实目录下每份事实都是 `_` 前缀之外的可执行模块', () => {
  const ids = discover(path.join(REPO, 'scripts', 'facts'))
  assert.ok(ids.length >= 3, `期望至少 3 份事实，实际 ${ids.length}`)
  assert.ok(!ids.includes('_lib'), '_lib 是共享材料，不该被当事实')
  assert.ok(!ids.some((i) => i.endsWith('.test')), '回归测试不该被当事实')
})

test('validateFact：完整描述符零违规', () => {
  const f = { id: 'x', title: 't', out: 'docs/x.md', render: () => 'x' }
  assert.deepEqual(validateFact(f, { id: 'x', outRoot: null }), [])
})

test('validateFact：缺 out / title / render 逐条判红', () => {
  assert.ok(validateFact({ id: 'x', title: 't', render: () => '' }, { id: 'x' }).some((e) => e.includes('out')))
  assert.ok(validateFact({ id: 'x', out: 'docs/x.md', render: () => '' }, { id: 'x' }).some((e) => e.includes('title')))
  assert.ok(validateFact({ id: 'x', title: 't', out: 'docs/x.md' }, { id: 'x' }).some((e) => e.includes('render')))
})

test('validateFact：id 与文件名不一致判红（错这一条就查不出「加了文件忘了接线」）', () => {
  const errs = validateFact({ id: 'other', title: 't', out: 'docs/x.md', render: () => '' }, { id: 'x' })
  assert.ok(errs.some((e) => e.includes('与文件名')))
})

test('validateFact：`out` 越界判红——runner 不是任意写文件的口子', () => {
  for (const out of ['../escape.md', 'docs/../../escape.md', 'C:/abs.md', '/abs.md', 'docs\\x.md']) {
    const errs = validateFact({ id: 'x', title: 't', out, render: () => '' }, { id: 'x' })
    assert.ok(errs.length > 0, `应当判红：${out}`)
  }
})

test('validateFact：没有 default 导出判红', () => {
  assert.ok(validateFact(undefined, { id: 'x' }).some((e) => e.includes('default')))
})

// ── 幂等与漂移 ────────────────────────────────────────────────────

test('stripStamp：溯源行不参与「内容变没变」的判定', () => {
  const a = 'x\n> 生成时间：2020-01-01 00:00:00 UTC\n'
  const b = 'x\n> 生成时间：2026-10-16 09:00:00 UTC\n'
  assert.equal(stripStamp(a), stripStamp(b))
  assert.notEqual(stripStamp(a).length, a.length, '溯源行本身应当被剔除')
})

test('emit：内容没变则整份沿用（连同原溯源行），文件不被改写', () => {
  const out = path.join(tmp(), 'x.md')
  assert.equal(emit({ out, content: 'body\n> 生成时间：A\n' }).changed, true)
  const before = fs.readFileSync(out, 'utf8')
  assert.equal(emit({ out, content: 'body\n> 生成时间：B\n' }).changed, false)
  assert.equal(fs.readFileSync(out, 'utf8'), before, '沿用必须是逐字节沿用')
})

test('emit：正文真变了就改写（把溯源行变化也当「没变」= 漂移永不判红）', () => {
  const out = path.join(tmp(), 'x.md')
  emit({ out, content: 'body1\n> 生成时间：A\n' })
  assert.equal(emit({ out, content: 'body2\n> 生成时间：B\n' }).changed, true)
  assert.match(fs.readFileSync(out, 'utf8'), /^body2/)
})

test('driftOf：文件不存在 / 内容不同都给出可读的一句话', () => {
  const dir = tmp()
  assert.match(driftOf({ out: path.join(dir, 'none.md'), content: 'x' }), /不存在/)
  const out = path.join(dir, 'y.md')
  emit({ out, content: 'a\n' })
  assert.equal(driftOf({ out, content: 'a\n' }), null)
  assert.match(driftOf({ out, content: 'b\n' }), /漂移/)
})

// ── 端到端：runner 本体 ───────────────────────────────────────────

test('端到端：空目录 ⇒ 退出码 2（绝不「什么都没生成」却退出 0）', () => {
  const r = runIn(tmp())
  assert.equal(r.code, 2, r.output)
  assert.match(r.output, /一份事实都没有/)
})

test('端到端：描述符破损 ⇒ 退出码 1 且指名是哪个文件', () => {
  const r = runIn(tmp(), { broken: 'export default { id: "broken" }\n' })
  assert.equal(r.code, 1, r.output)
  assert.match(r.output, /broken\.mjs/)
})

test('端到端：`out` 越界 ⇒ 退出码 1 且不写出任何文件', () => {
  const repo = tmp()
  const r = runIn(repo, { escape: "export default { id: 'escape', title: 't', out: '../pwned.md', render: () => 'x' }\n" })
  assert.equal(r.code, 1, r.output)
  assert.equal(fs.existsSync(path.join(repo, '..', 'pwned.md')), false, '越界目标不该被写出')
})

test('端到端：render 抛错 ⇒ 退出码 1（不是「生成成功但内容为空」）', () => {
  const r = runIn(tmp(), {
    boom:
      "export default { id: 'boom', title: 't', out: 'docs/boom.md', render: () => { throw new Error('源读不到') } }\n",
  })
  assert.equal(r.code, 1, r.output)
  assert.match(r.output, /源读不到/)
})

test('端到端：好事实会被写出，且第二次运行不改写', () => {
  const repo = tmp()
  const r1 = runIn(repo, { good: goodSource() })
  assert.equal(r1.code, 0, r1.output)
  assert.equal(fs.readFileSync(path.join(repo, 'docs', 'good.md'), 'utf8'), 'hello\n')
  const r2 = runIn(repo, { good: goodSource() })
  assert.equal(r2.code, 0, r2.output)
  assert.match(r2.output, /已是最新/)
})

test('端到端：`--check` 对漂移判红、对一致判绿', () => {
  const repo = tmp()
  const scripts = path.join(repo, 'scripts')
  fs.mkdirSync(path.join(scripts, 'facts'), { recursive: true })
  fs.copyFileSync(RUNNER, path.join(scripts, 'gen-facts.mjs'))
  fs.copyFileSync(path.join(REPO, 'scripts', 'color.mjs'), path.join(scripts, 'color.mjs'))
  fs.copyFileSync(path.join(REPO, 'scripts', 'facts', '_lib.mjs'), path.join(scripts, 'facts', '_lib.mjs'))
  fs.writeFileSync(path.join(scripts, 'facts', 'good.mjs'), goodSource(), 'utf8')
  const check = () => {
    try {
      execFileSync(process.execPath, [path.join(scripts, 'gen-facts.mjs'), '--check'], {
        cwd: repo,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'pipe'],
      })
      return 0
    } catch (e) {
      return e.status ?? -1
    }
  }
  assert.equal(check(), 1, '产物还不存在 ⇒ 应判红')
  fs.mkdirSync(path.join(repo, 'docs'), { recursive: true })
  fs.writeFileSync(path.join(repo, 'docs', 'good.md'), 'hello\n', 'utf8')
  assert.equal(check(), 0, '一致 ⇒ 应判绿')
  fs.writeFileSync(path.join(repo, 'docs', 'good.md'), 'drifted\n', 'utf8')
  assert.equal(check(), 1, '漂移 ⇒ 应判红')
})

test('能力矩阵（§6）：三张表的列数必须逐行一致', () => {
  // 这条不是洁癖：`vdfs_list` 的描述里写着 `type（directory|file）`，
  // 一个**未转义**的竖线会被 Markdown 读成新的一列 ⇒ 那一行其余列整体错位，
  // 而生成器只管吐文本、**看不见**这件事。语义取自代码 ⇒ 源码里有什么字符都会
  // 进表格，所以只能在这层判。
  const doc = fs.readFileSync(path.join(REPO, 'docs', 'CURRENT.md'), 'utf8')
  const lines = doc.split(/\r?\n/)
  let inSec = false
  let want = 0
  const bad = []
  lines.forEach((l, i) => {
    if (/^### 6\./.test(l)) {
      inSec = true
      want = 0
    }
    if (!inSec) return
    if (/^\|(?:---\s*\|)+$/.test(l)) {
      want = l.split('|').length - 2
      return
    }
    if (want && l.startsWith('|')) {
      const n = l.split(/(?<!\\)\|/).length - 2
      if (n !== want) bad.push(`第 ${i + 1} 行：${n} 列（表头 ${want} 列）`)
    }
  })
  assert.deepEqual(bad, [], `§6 有行列数不符：\n    ${bad.join('\n    ')}`)
})

test('能力矩阵（§6）：每条静态路由都有一句职责（缺一条就是「表里没写」）', () => {
  // 「抽不到就标空」对**描述**是对的（不臆造），但对**路由职责**是错的：
  // 补一句注释是一次性成本，此后由提取跟随。空着会让这张表永远缺一份清单，
  // 而那正是它要防的形态（RE-005 的行集缺席同款）。
  const doc = fs.readFileSync(path.join(REPO, 'docs', 'CURRENT.md'), 'utf8')
  const sec = doc.slice(doc.indexOf('### 6.3'))
  const missing = [...sec.matchAll(/^\| `([^`]+)` \| — \|$/gm)].map((m) => m[1])
  assert.deepEqual(missing, [], `这些路由在代码里还没有一句话职责：${missing.join('、')}`)
})

test('本仓自身：`--check` 全绿（三份事实与代码一致）', () => {
  try {
    const out = execFileSync(process.execPath, [RUNNER, '--check'], {
      cwd: REPO,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    assert.match(out, /docs\/CURRENT\.md/)
    assert.match(out, /docs\/ref\/GATE_CODES\.md/)
    assert.match(out, /tauri\/src\/constants\/routes\.gen\.ts/)
  } catch (e) {
    assert.fail(`--check 应当全绿，实际：${e.stdout ?? ''}${e.stderr ?? ''}`)
  }
})