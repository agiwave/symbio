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
import { capabilityFacts } from './facts/_capability.mjs'
import { pluginSources, extractRouteArms, vdfsOps } from './route-facts.mjs'

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

/**
 * 能力矩阵三张表所在的节次（截到下一个 `## `，不含 §3 及以后）。
 *
 * **刻意不写死编号**：这张表被提过位（§6 → §2，因为它覆盖了原 §2 的工具清单）。
 * 写死编号的判据在提位那一刻**集体失效且不报红**——`/^### 6\./` 匹配不到 ⇒
 * 循环体一次都不进 ⇒ 断言 `bad === []` **恒真**。恒真的守卫比没有守卫更坏：
 * 它占着判据码，让人以为这一族已被守住。改按**内容**定位。
 */
const CAPABILITY_SEC = (() => {
  const doc = fs.readFileSync(path.join(REPO, 'docs', 'CURRENT.md'), 'utf8')
  const i = doc.search(/^### \d+\.1 LLM 可见工具$/m)
  assert.notEqual(i, -1, 'CURRENT.md 里找不到「LLM 可见工具」小节——判据自己失效了')
  const rest = doc.slice(i)
  const j = rest.search(/^## [^#]/m)
  return j === -1 ? rest : rest.slice(0, j)
})()

test('能力矩阵：三张表的列数必须逐行一致', () => {
  // 这条不是洁癖：`vdfs_list` 的描述里写着 `type（directory|file）`，
  // 一个**未转义**的竖线会被 Markdown 读成新的一列 ⇒ 那一行其余列整体错位，
  // 而生成器只管吐文本、**看不见**这件事。语义取自代码 ⇒ 源码里有什么字符都会
  // 进表格，所以只能在这层判。
  //
  // 口径是**逐张表**：每张表拿**自己**的分隔行当基准，跨表不比。
  // （曾经按「整节共用一个 want」写，那让 §2.2 的 2 列表被拿去和 §2.1 的 6 列表
  // 比 —— 判据自己红自己。列数一致是**表内**性质，不是跨表性质。）
  //
  // 还有一处必须豁免：**新表的头行**。头行在**自己**的分隔行**之前**，所以在它那
  // 一行上 `want` 还是**上一张表**的值——拿它去比就会把「2 列表的头」判成「6 列表
  // 少列」。判据红自己是最快的失守方式：修法是让「表的开始」显式清零，而不是
  // 给每一处不合意的行加豁免。
  const lines = CAPABILITY_SEC.split(/\r?\n/)
  let want = 0
  let prevWasPipe = false
  let table = 0
  const bad = []
  lines.forEach((l, i) => {
    const isPipe = l.startsWith('|')
    if (isPipe && !prevWasPipe) want = 0 // 新表开始，头行还没有基准
    prevWasPipe = isPipe
    if (!isPipe) return
    if (/^\|(?:---\s*\|)+$/.test(l)) {
      want = l.split('|').length - 2
      table++
      return
    }
    if (want) {
      const n = l.split(/(?<!\\)\|/).length - 2
      if (n !== want) bad.push(`第 ${i + 1} 行：${n} 列（表头 ${want} 列）`)
    }
  })
  assert.equal(table, 3, `能力矩阵应当是 3 张表，实际认出 ${table} 张——判据自己失效了`)
  assert.deepEqual(bad, [], `能力矩阵有行列数不符：\n    ${bad.join('\n    ')}`)
})

test('能力矩阵：每条静态路由都有一句职责（缺一条就是「表里没写」）', () => {
  // 「抽不到就标空」对**描述**是对的（不臆造），但对**路由职责**是错的：
  // 补一句注释是一次性成本，此后由提取跟随。空着会让这张表永远缺一份清单，
  // 而那正是它要防的形态（RE-005 的行集缺席同款）。
  const sec = CAPABILITY_SEC.slice(CAPABILITY_SEC.indexOf('前端 / 宿主路由'))
  assert.notEqual(sec, '', '找不到「前端 / 宿主路由」小节——判据自己失效了')
  const missing = [...sec.matchAll(/^\| `([^`]+)` \| — \|$/gm)].map((m) => m[1])
  assert.deepEqual(missing, [], `这些路由在代码里还没有一句话职责：${missing.join('、')}`)
})

/**
 * 三个**封闭集**（成员直接从代码提取，与 `CURRENT.md` §2 用的是同一份来源）。
 *
 * ⚠️ 判据必须按**集合成员身份**判，不能按**字面形状**判。
 * 第一版用正则形状（`` | `x_y` | `` 算工具、`` | `a/b` | `` 算路由），
 * 在本仓**当场三处误报**：
 *   - 插件总表的第一列是**插件目录名**（`agent`、`web`…），形状与工具短名无差别；
 *   - VDFS 操作（`vdfs/read`）形状与路由**完全一样**（`词/词`）；
 *   - 代码规模表的 `symbio/src`、`tauri/src` 也是 `词/词`。
 * 守卫一旦开始误报，下场是被整体豁免——比没有守卫更糟。
 * 按成员身份判则天然分开：`vdfs/read` 不在路由集里，`symbio/src` 哪儿都不在。
 */
const CLOSED_SETS = (() => {
  const cap = capabilityFacts(REPO)
  // 插件目录**扫出来**而不是写死一份名单：写死的名单会与新增插件分叉，而分叉的
  // 方向是「静默漏判」——新插件的路由不进集合，于是它在别处被复述时这条判据看不见。
  const base = path.join(REPO, 'symbio', 'src', 'plugins')
  const routes = new Set()
  for (const p of fs.readdirSync(base, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name)) {
    for (const src of pluginSources(path.join(base, p))) {
      for (const a of extractRouteArms(src, p)) routes.add(a)
    }
  }
  return {
    'LLM 可见工具': new Set([...cap.tools.keys()]),
    'VDFS 操作': new Set(cap.vdfsOps),
    '前端 / 宿主路由': routes,
  }
})()

/**
 * 把一份 Markdown 切成若干「集中列举处」，返回每处的成员列表。
 *
 * ## 为什么不止「表」，也不止「行」
 *
 * 当年那份重复的 §3.2 有两种可能写法，两种都真实出现过：
 *   - **一行**用 ` · ` 串起来：`- 前端链路（13 个）：`vdfs/root` · `vdfs/list` · …`
 *   - **一个列表块**：`- `vdfs/root`` / `- `vdfs/list`` / …
 *
 * 只判表格对它完全失明；只判单行只能抓前一种。注入验证当场各漏过一次——
 * 这就是「按**块**（空行分隔的连续非表格行）」而不是按行归集的理由。
 *
 * 两个阈值不同，因为**误报代价不同**：
 *   - 表格里 ≥3 个成员 ⇒ 几乎必然是在列清单（表的第一列是标识符的形状要求）；
 *   - 散文块里 ≥5 个成员 ⇒ 同样几乎必然是清单；顺手提三四个工具名是正常论述，
 *     压到 3 会开始误报，而守卫一旦开始误报就是被整体豁免（docs/README.md D-004 那条）。
 */
function tableKeys(md, { prose = true } = {}) {
  const out = []
  const lines = md.split(/\r?\n/)
  let table = null
  let block = null
  const flushBlock = () => {
    if (prose && block && block.length >= 5) out.push(block)
    block = null
  }
  const flushTable = () => {
    if (table) out.push(table)
    table = null
  }
  for (const l of lines) {
    if (l.startsWith('|')) {
      flushBlock()
      if (!table) table = []
      const first = l.split('|')[1] ?? ''
      const m = first.match(/^\s*`([^`]+)`\s*$/)
      if (m) table.push(m[1])
      continue
    }
    flushTable()
    if (!l.trim()) {
      flushBlock()
      continue
    }
    if (!block) block = []
    for (const m of l.matchAll(/`([^`]+)`/g)) block.push(m[1])
  }
  flushTable()
  flushBlock()
  return out
}

test('一个封闭集只在一张表里成表（同一批事实不许写两遍）', () => {
  // 本条防的是**已发生过**的那种腐化：CURRENT.md 曾同时有「§2 工具清单」与
  // 「§6.1 工具语义」两张表（行集交集 25/25），§3.2 与 §6.2 两处列同一批 VDFS
  // 操作，§1 与 §6.3 两处列同一批路由。§2 那张表还带一列**人写的**备注，与代码
  // 提取的描述逐条讲同一件事——于是它既冗余又会漂移。
  //
  // 只判「成表」：**同一集合的成员各自出现在 N 张不同表里**。散文提到某个工具名、
  // 文档引用本表，都不算——那是指路不是复述。
  // 口径：每个集合**覆盖最全**的那处是 owner；其余任何一处若也集中列了 ≥3 个成员判红。
  // 「处」不限于表格——当年那份重复的 §3.2 是一行 ` · ` 串起来的清单，见 `tableKeys`。
  const doc = fs.readFileSync(path.join(REPO, 'docs', 'CURRENT.md'), 'utf8')
  const tables = tableKeys(doc)
  assert.ok(tables.length >= 5, `只认出 ${tables.length} 张表——判据自己失效了`)

  const bad = []
  for (const [setName, members] of Object.entries(CLOSED_SETS)) {
    // 每张表命中了哪些成员
    const hits = tables.map((keys, i) => ({ i, n: keys.filter((k) => members.has(k)).length }))
    const hit = hits.filter((h) => h.n > 0)
    if (hit.length === 0) {
      bad.push(`${setName}：CURRENT.md 一张表里都没有——owner 不见了`)
      continue
    }
    const owner = hit.reduce((a, b) => (b.n > a.n ? b : a))
    if (owner.n < members.size) {
      bad.push(
        `${setName}：owner 表只覆盖 ${owner.n}/${members.size} 个成员` +
          `（表头 \`${tables[owner.i][0] ?? '?'}\`）`,
      )
    }
    for (const h of hit) {
      if (h.i === owner.i) continue
      if (h.n < 3) continue // 两三行的表不构成复述
      bad.push(
        `${setName}：除 owner 外，第 ${h.i + 1} 张表还列了 ${h.n} 个成员` +
          `（首个 \`${tables[h.i][0]}\`）——改成指路`,
      )
    }
  }
  assert.deepEqual(
    bad,
    [],
    `同一批事实被写进了多张表。处置不是「把两张表合并」，是先问**哪张是 owner**、其余改成指路：\n    ${bad.join('\n    ')}`,
  )
})

test('跨文档：CURRENT.md 的封闭集不得在别处成表复述', () => {
  // `docs/README.md` 的「文档职责边界」是一张**承诺表**——它写着「不手抄，需要就引用」。
  // 承诺本身此前无人守：D-001…D-010 全是查链 / 查措辞 / 查形态，没有一条查
  // 「同一事实被写了两遍」。RE-003 当年就有过两张路由表并存的形态（各自漂移）。
  //
  // 这条把上条判据**推广到全仓**。口径按**成员身份**判，不按字面形状（理由见
  // `CLOSED_SETS` 上面那段）。
  //
  // ## 为什么跨文档只判「成表」，不判散文
  //
  // 首版跨文档也判散文块，当场**三处误报**，而三处理由各不相同：
  //   - `AR-013-vdfs.md` 在讲每个操作的**机制**（`t` 位递归、访问层组合）——那是规范
  //     正文的活，它点 7 个操作名是在**解释**，不是在列清单；
  //   - `RE-001-CONFIGURATION.md` 那处是 gateway **只读白名单**——一个配置值碰巧用
  //     操作名拼成；
  //   - `T-056.md` 在描述要删哪些 e2e 用例。
  // 三处的共同点：**散文里点名 N 个成员是在讲机制**，而机制归它们各自的 owner。
  // 真正的复述形态历来都是**表**——把一张表抄成第二张才会漂移。
  // 而本文件内（生成物）散文块照样判：我控制生成器，写成散文块一样是多余的。
  //
  // 另外**放行 owner 自己**与**明确指路**：RE-003 是路由**契约**的 owner，它按插件
  // 分节重复列地址是那本书的组织方式，删掉就毁掉它的可查性——按**白名单**放行，
  // 白名单写明它保的是什么，不做笼统豁免。
  //
  // ⚠️ 判据只覆盖 CURRENT.md 投影出来的三个封闭集。别的 owner（错误码 / 配置 /
  // prompt 形状）各有一张表，**尚未**纳入这条判据——那是欠账，见 T-100。
  //
  // ## 已知边界：**成表不足以判定复述**
  //
  // 一张表可以**以封闭集成员为键**而讲另一件事。session README 那张
  // 「工具名 → 结果取哪个字段」就是：四个工具名出现，但它答的是「结果形状」，
  // CURRENT.md 答的是「这条能力是什么」。判据分不出这两者——它只能数成员出现次数。
  //
  // 这条边界写在这里而不是靠多堆白名单掩盖：白名单里每一条都写明**它保的是什么**，
  // 新增一条必须同样写清；判据抓的是「抄一张清单」，抓不到「拿它当索引」。
  const ALLOW = new Map([
    ['docs/ref/RE-003-ROUTES.md', '路由**契约**的 owner：按插件分节重复列地址是它的组织方式'],
    ['docs/CURRENT.md', '代码投影，本判据的 owner'],
    [
      'symbio/src/plugins/session/README.md',
      '「工具名 → 结果取哪个字段」映射表：工具名是那张表的**键**，讲的是结果形状，' +
        '不是工具清单（见下面「已知边界」）',
    ],
  ])
  const files = []
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      if (e.name === '.git' || e.name === 'node_modules' || e.name === 'target') continue
      const p = path.join(d, e.name)
      if (e.isDirectory()) walk(p)
      else if (e.name.endsWith('.md')) files.push(p)
    }
  }
  walk(REPO)

  const bad = []
  let scanned = 0
  for (const f of files) {
    const rel = path.relative(REPO, f).replace(/\\/g, '/')
    if (rel.startsWith('docs/archive/') || ALLOW.has(rel)) continue
    // 生成物与门禁自己的日志不是文档
    if (rel.includes('.workbuddy-ai/') || rel.endsWith('docs/CURRENT.md')) continue
    scanned++
    const text = fs.readFileSync(f, 'utf8')
    // `prose: false` —— 跨文档只判「成表」，理由见上。
    for (const [setName, members] of Object.entries(CLOSED_SETS)) {
      for (const [i, keys] of tableKeys(text, { prose: false }).entries()) {
        const hit = keys.filter((k) => members.has(k))
        if (hit.length < 3) continue
        bad.push(`${rel} 第 ${i + 1} 张表：列了 ${hit.length} 个「${setName}」成员（${hit.slice(0, 3).join('、')}…）`)
      }
    }
  }
  assert.ok(scanned > 80, `只扫了 ${scanned} 份文档——判据自己失效了`)
  assert.deepEqual(
    bad,
    [],
    `这些文档在复述 CURRENT.md 已经投影的事实。处置：留一行指路（\`[CURRENT.md §2.x](../CURRENT.md#…\`），` +
      `不要复制清单——同一句事实存两份，改一次要动两处，且必然漂移：\n    ${bad.join('\n    ')}`,
  )
})

test('本仓自身：runner 认得三份事实（本测试不要求此刻无漂移）', async () => {
  // ⚠️ 这里**刻意**不断言 `--check` 全绿：门禁的 `30-docs` 阶段跑本测试，而事实文件由
  // **更靠后的** `60-facts` 阶段重生成。于是「代码改了、事实还没重生成」是本测试运行时
  // 的**正常状态**，断言它全绿等于断言一件还没发生的事——首次接入时就因此假红过一次。
  // 漂移的判定归它的自然位置：重生成之后（`60-facts`）与 CI 的 `--ci` 模式。
  //
  // 本测试只钉「目录被遍历到」这一件与时序无关的事——漏接线时它才红。
  const ids = discover(path.join(REPO, 'scripts', 'facts'))
  for (const id of ['current', 'gate-codes', 'routes-ts']) {
    assert.ok(ids.includes(id), `runner 应当认得事实 \`${id}\`（实际：${ids.join('、')}）`)
  }
  const mods = await Promise.all(
    ['current', 'gate-codes', 'routes-ts'].map((id) => import(`./facts/${id}.mjs`)),
  )
  for (const [i, mod] of mods.entries()) {
    const errs = validateFact(mod.default, { id: ['current', 'gate-codes', 'routes-ts'][i] })
    assert.deepEqual(errs, [], `本仓事实的描述符必须合契约：${errs.join('；')}`)
  }
})