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
import { closedSets, NO_MEMBER_SET } from './facts/_sets.mjs'

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const RUNNER = path.join(REPO, 'scripts', 'gen-facts.mjs')

const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'gen-facts-'))

/**
 * 收集一棵树里的 `.md` 语料——「散文指路」类判据的扫描面。
 *
 * ## 为什么必须跳过 `.workbuddy-ai/`（T-108 之后）
 *
 * 门禁的失败详情会把**失败项日志的末 N 行**写进
 * `.workbuddy-ai/gate-logs/<runId>/_summary.md`，而那段正文就是**被扫文档的原文引用**。
 * 把引用当「指路」判，结果是：一条早已修好的错误引用会被**它自己那次失败的记录**
 * 永久喂红——而且红在一个日志文件上、与源码无关（症状：改完文档重跑仍红，
 * 指出的行号落在一个 `_summary.md` 里，读者以为判据坏了）。
 *
 * 这不是假设：本轮修 `T-110` 的 `RE-006 §6` 时正是如此——源文档改对了，
 * 上一轮的失败详情仍在语料里，判据继续红。
 *
 * ## 范围收窄到哪里为止
 *
 * 跳过的只有**工具产物目录**（`node_modules` / `target` / `.git` / `archive` /
 * `.workbuddy-ai`），**一个文档目录都没跳**——收窄的方向是「把不是文档的东西排除」，
 * 不是「把判据够不着的地方缩小」。回归测试双向钉住这一点（见文件末尾那条）。
 */
export function proseDocs(root) {
  const SKIP_DIRS = new Set(['.git', 'node_modules', 'target', 'archive', '.workbuddy-ai'])
  const out = []
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      if (SKIP_DIRS.has(e.name)) continue
      const p = path.join(d, e.name)
      if (e.isDirectory()) walk(p)
      else if (e.name.endsWith('.md')) {
        out.push({
          path: p,
          rel: path.relative(root, p).replace(/\\/g, '/'),
          text: fs.readFileSync(p, 'utf8'),
        })
      }
    }
  }
  walk(root)
  return out
}

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
  const doc = fs.readFileSync(path.join(REPO, 'docs', 'ref', 'RE-006-当前事实表.md'), 'utf8')
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
 * 封闭集 —— 「同一批事实只许在一处成表」的**成员集合 + owner** 声明。
 *
 * 声明住在 `scripts/facts/_sets.mjs`（与 `RE-007-清单归属总账` 同源地读各文档
 * front-matter 的 `lists: [<id>=owner]`），本文件只**读**它：判据的住所与被判的
 * 东西分开，新增一类 owner 投影不用再改测试。
 *
 * ⚠️ 判据必须按**集合成员身份**判，不能按**字面形状**判。
 * 第一版用正则形状（`` | `x_y` | `` 算工具、`` | `a/b` | `` 算路由），
 * 在本仓**当场三处误报**：
 *   - 插件总表的第一列是**插件目录名**（`agent`、`web`…），形状与工具短名无差别；
 *   - VDFS 操作（`vdfs/read`）形状与路由**完全一样**（`词/词`）；
 *   - 代码规模表的 `symbio/src`、`tauri/src` 也是 `词/词`。
 * 守卫一旦开始误报，下场是被整体豁免——比没有守卫更糟。
 * 按成员身份判则天然分开：`vdfs/read` 不在路由集里，`symbio/src` 哪儿都不在。
 *
 * 覆盖面对账：职责边界表里每个**可能有成员集合**的 owner 投影，要么在上面这份声明里，
 * 要么在 `_sets.mjs` 的 `NO_MEMBER_SET` 里带一条「为什么没有集合可判」的理由
 * （PROMPT_SHAPE 是运行时形状、AR-013 操作族与 VDFS 操作同源）。不写理由的那一栏
 * 留空，就等于把「都覆盖了吗」推给人读一遍表——正是判据要省掉的那件事。
 */
const SETS = closedSets(REPO)

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
  const doc = fs.readFileSync(path.join(REPO, 'docs', 'ref', 'RE-006-当前事实表.md'), 'utf8')
  const tables = tableKeys(doc)
  assert.ok(tables.length >= 5, `只认出 ${tables.length} 张表——判据自己失效了`)

  const bad = []
  // 只查 CURRENT.md 自己拥有的三个集合（owner = RE-006 的那几个）。
  // **刻意不推广到手写 owner**：文件内判据连散文块一起判（生成物里写成散文块也是多余的），
  // 而手写 owner 的散文点名多个成员是在讲机制——RE-001 的 gateway 只读白名单一次点 7 个
  // VDFS 操作，正是跨文档判据首版误报的同一处。同一文档内的重复归这条，但它只对
  // 「我控制生成器」的文档成立。
  for (const [setName, set] of SETS) {
    if (set.ownerRel !== 'docs/ref/RE-006-当前事实表.md') continue
    const members = set.members
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

test('跨文档：封闭集不得在别处成表复述（覆盖职责边界表的每一个 owner 投影）', () => {
  // `docs/README.md` 的「文档职责边界」是一张**承诺表**——它写着「不手抄，需要就引用」。
  // 承诺本身此前无人守：D-001…D-010 全是查链 / 查措辞 / 查形态，没有一条查
  // 「同一事实被写了两遍」。RE-003 当年就有过两张路由表并存的形态（各自漂移）。
  //
  // 这条把上条判据**推广到全仓**。口径按**成员身份**判，不按字面形状（理由见
  // `SETS` 上面那段）。
  //
  // ## 覆盖面（T-100）
  //
  // 首版只查 CURRENT.md 投影的三个封闭集；职责边界表里其余的 owner 投影
  // （错误码 / 配置项 / 判据码）各有一张表而无人守。现在集合声明住在
  // `scripts/facts/_sets.mjs`，判据遍历**全部**集合：每个集合的 owner 从各文档
  // front-matter 的 `lists: [<id>=owner]` 声明里读（与 RE-007 同源），成员从
  // 代码 / Rust 枚举 / 脚本声明 / owner 自己的被标记清单里提。没有成员集合可判的
  // 两个投影（PROMPT_SHAPE、AR-013 操作族）在 `_sets.mjs` 的 `NO_MEMBER_SET` 里
  // 各带一条理由——「覆盖了吗」因此是一句可查的话，不是一篇散文。
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
  // 白名单写明它保的是什么，不做笼统豁免。owner 自己按集合逐个放行（一份文档可以是
  // 某个集合的 owner 同时抄了另一个集合的清单——那正是要判的）。
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
    [
      'docs/ref/RE-003-ROUTES.md',
      new Map([
        [
          '前端 / 宿主路由',
          '契约表以地址为**键**：讲的是调用方 / 返回类型 / 备注（另一件事），不声称拥有地址清单——' +
            '同 session README 的已知边界；键集合另有判据（「RE-003 契约表的键必须是 §2.3 的静态路由」）',
        ],
      ]),
    ],
    [
      'symbio/src/plugins/session/README.md',
      new Map([
        [
          'LLM 可见工具',
          '「工具名 → 结果取哪个字段」映射表：工具名是那张表的**键**，讲的是结果形状，' +
            '不是工具清单（见上面「已知边界」）',
        ],
      ]),
    ],
    [
      'docs/plan/PL-007-对话面插件拆分实施方案.md',
      new Map([
        [
          '配置项',
          '§7.1「配置平凡值」表以键为**键**：答的是完整值 / 平凡值 / 平凡值下的行为，不是键清单。' +
            '默认值的 owner 是各插件自己的 `Default` 实现（RE-001 明写「默认值以代码为准」），' +
            '与 classify/README 同一形状（见上面「已知边界」）',
        ],
      ]),
    ],
    [
      'symbio/src/plugins/classify/README.md',
      new Map([
        [
          '配置项',
          '「键 / 默认 / 平凡值 / 平凡值下」表：答的是本插件三个键的**默认值**，不是键清单' +
            '（同 PL-007 §7.1，见上面「已知边界」）',
        ],
      ]),
    ],
  ])

  /** 收全仓 `.md`（生成物与门禁日志除外）⇒ `[{ rel, text }]`。 */
  const repoDocs = () => {
    const docs = []
    const walk = (d) => {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        if (e.name === '.git' || e.name === 'node_modules' || e.name === 'target') continue
        const p = path.join(d, e.name)
        if (e.isDirectory()) walk(p)
        else if (e.name.endsWith('.md')) docs.push(p)
      }
    }
    walk(REPO)
    return docs.map((f) => ({ file: f, rel: path.relative(REPO, f).replace(/\\/g, '/') }))
  }

  /**
   * 对给定的文档集跑跨文档判据，返回违规列表。
   *
   * 抽成函数是为了**反向验证**：把 owner 的表复制一份塞进一个合成文档，
   * 判据必须红。不抽的话，反向用例只能在真仓里落盘再删——反向验证做不到「必然红」。
   */
  const scanDocs = (docs) => {
    const bad = []
    let scanned = 0
    for (const { rel, file } of docs) {
      if (rel.startsWith('docs/archive/') || rel.includes('.workbuddy-ai/')) continue
      scanned++
      const text = fs.readFileSync(file, 'utf8')
      // `prose: false` —— 跨文档只判「成表」，理由见上。
      for (const [setName, set] of SETS) {
        if (rel === set.ownerRel) continue // owner 自己放行（按集合逐个放行）
        if (ALLOW.get(rel)?.has(setName)) continue // 白名单逐条写明它保什么
        for (const [i, keys] of tableKeys(text, { prose: false }).entries()) {
          const hit = keys.filter((k) => set.members.has(k))
          if (hit.length < 3) continue
          bad.push(`${rel} 第 ${i + 1} 张表：列了 ${hit.length} 个「${setName}」成员（${hit.slice(0, 3).join('、')}…）`)
        }
      }
    }
    return { bad, scanned }
  }

  // ── ① owner 必须逐字收留每个成员（「owner 不见了」的另一半：成员被真源加了而表没收）──
  //
  // 弱口径（字面出现，不要求成表）：错误码与判据码的 owner 表按自己的列形状渲染
  // （判据码是 `**C-001**` 加粗列，不是反引号列），逐字比不误报也不漏；成表口径的
  // 严格版归文件内判据（CURRENT.md 那份）。
  const ownerMiss = []
  for (const [setName, set] of SETS) {
    const ownerText = fs.readFileSync(path.join(REPO, set.ownerRel), 'utf8')
    const missing = [...set.members].filter((m) => !ownerText.includes(m))
    if (missing.length) {
      ownerMiss.push(`${setName}：owner（${set.ownerRel}）没收这些成员：${missing.slice(0, 5).join('、')}${missing.length > 5 ? '…' : ''}`)
    }
  }
  assert.deepEqual(
    ownerMiss,
    [],
    'owner 文档漏收成员——真源（代码 / 声明）加了东西而表没跟上，读表的人会漏掉它：\n    ' +
      ownerMiss.join('\n    '),
  )

  // ── ①b 没有成员集合可判的投影，理由必须写实 ──
  //
  // 「覆盖职责边界表的每一个 owner 投影」这句话要能查，就得让**不设集合**也成为一种
  // 显式声明：每条带一个为什么（且指向的文档真在），不留空话。
  for (const { owner, why } of NO_MEMBER_SET) {
    assert.ok(why && why.length > 10, `${owner} 声明了「无成员集合」，但没写为什么`)
    assert.ok(fs.existsSync(path.join(REPO, owner)), `${owner} 不在仓里——「无成员集合」声明指向了一份不存在的文档`)
    assert.ok(
      ![...SETS.values()].some((s) => s.ownerRel === owner),
      `${owner} 既在 NO_MEMBER_SET 里又拥有一个集合——两边声明互相矛盾`,
    )
  }

  // ── ② 全仓扫描：非 owner 的表里 ≥3 个成员即复述 ──
  const docs = repoDocs()
  const { bad, scanned } = scanDocs(docs)
  assert.ok(scanned > 80, `只扫了 ${scanned} 份文档——判据自己失效了`)
  assert.deepEqual(
    bad,
    [],
    '这些文档在复述别人的 owner 已经投影的事实。处置：留一行**编号指路**（`[[编号|…]]`，' +
      '地址由 gen-doc-refs 重算），不要复制清单——同一句事实存两份，改一次要动两处，且必然漂移。' +
      '若它「以成员为键讲另一件事」（默认值 / 契约 / 结果形状），按 ALLOW 登记并写明**它保的是什么**：\n    ' +
      bad.join('\n    '),
  )

  // ── ③ 反向验证：每个集合都必须能被「复制 owner 的表到别处」触发 ──
  //
  // 「全绿」只说明判据没被触发，不说明它能触发。三个首版集合当年注入过三种重复形态；
  // T-100 新接的三个集合（错误码 / 配置项 / 判据码）各注入一次，缺一次就是恒真。
  //
  // 注入的是**合成文档**（不进真仓），与真仓其余文档一起过同一个 `scanDocs`：
  // 豁免名单、集归属、阈值全走真实路径，只有这一份是假的。
  const reverse = [
    { setName: '错误码', why: '错误码字面量表' },
    { setName: '配置项', why: '配置键清单表' },
    { setName: '判据码', why: '判据码表' },
  ]
  for (const { setName, why } of reverse) {
    const set = SETS.get(setName)
    const members = [...set.members].slice(0, 6)
    assert.ok(members.length >= 3, `集合「${setName}」成员不足 3 个，反向用例无从注入`)
    const injected = [
      `# 注入的反向用例（${why}）`,
      '',
      '| 成员 | 说明 |',
      '|---|---|',
      ...members.map((m) => `| \`${m}\` | 复制自 owner 的清单。 |`),
      '',
    ].join('\n')
    const dir = tmp()
    const injectedPath = path.join(dir, 'injected.md')
    fs.writeFileSync(injectedPath, injected, 'utf8')
    const withInject = scanDocs([{ rel: 'docs/ref/RE-999-injected.md', file: injectedPath }, ...docs])
    assert.ok(
      withInject.bad.length > bad.length,
      `反向用例失效：把 owner 的${why}复制到别处，判据不红——集合「${setName}」是恒真的`,
    )
    assert.ok(
      withInject.bad.some((l) => l.includes(`「${setName}」`)),
      `反向用例失效：红了但不是「${setName}」那一条——判据认错了集合`,
    )
  }
})

test('散文里的 RE-006 节次指路：节次必须存在', () => {
  // ## 它防的是哪种失效
  //
  // `CURRENT.md` 的能力矩阵从 §6 提到 §2 那一次，迁移脚本改了三组**锚点**
  // （`#6-*`/`#61-*`/`#63-*`）与带 `CURRENT.md ` 前缀的散文，漏了两类：
  //
  //   1. **链接的显示文字**：`[§6.3](../CURRENT.md#23-…)` 的目标锚点改了，
  //      文字还是 `§6.3`。D-007 判的是 `#锚点`——它只看见目标，**看不见文字**，
  //      于是 13 处「点进去是对的、读起来是错的」判绿放行。
  //   2. **语义指路**：`CURRENT.md §3.2`（指 VDFS 操作闭集）。那一节的内容在同一次
  //      去重里被改写成了「两条链路不一一对应」——节次**还在**，所以任何按锚点判的
  //      守卫都看不出它已经指错了东西。
  //
  // 后者是这类失效的一般形态：**位置没变，位置上的东西换了**。锚点判据结构上抓不到
  // 它，只能抓「节次不存在」这一半。
  //
  // ## 口径为什么收窄到「指向 CURRENT.md 的指路」
  //
  // 第一版按「同一行里出现过 CURRENT.md」一刀切，当场**三处误判**：ADR-039 的 `§0`、
  // T-077 里同句并列的 `AR-010 §5.5`、T-099 evidence 里的 `§7`（那指 ST 文档的节次，
  // 与 CURRENT 无关）。守卫一旦开始误报就是被整体豁免，比没有守卫更糟。
  // 改成只判**明确指向 CURRENT.md 的那两种形态**（链接目标含 CURRENT.md / 散文写全
  // `CURRENT.md §x`），那之外一律放过。
  //
  // ## 刻意漏掉的形态（不是疏忽，是取舍）
  //
  // 「CURRENT.md」与「§N.M」之间**隔着一个词**的散文——如 T-098 evidence 里的
  // 「CURRENT.md 新增 §6 能力矩阵」——**不判**。要判它就得允许中间是任意散文，
  // 那必然误伤「CURRENT.md 与 §7 顺延为 §8」这类**指向别的文档**的同句并列。
  // 取舍：宁可漏一种形态，也不要一条会误报的判据。漏掉的这一种靠**位置中立写法**
  // 消解——登记条目的 evidence 不该用节号指路（节号是位置，会动；「能力矩阵」是
  // 那件东西，不会动）。T-098 已按此改写。
  const doc = fs.readFileSync(path.join(REPO, 'docs', 'ref', 'RE-006-当前事实表.md'), 'utf8')
  const actual = new Set([...doc.matchAll(/^#{2,3}\s+([\d.]+)\.?\s/gm)].map((m) => m[1].replace(/\.$/, '')))

  const bad = []
  let scanned = 0
  for (const f of proseDocs(REPO)) {
    if (f.rel.endsWith('docs/ref/RE-006-当前事实表.md')) continue
    scanned++
    f.text.split(/\r?\n/).forEach((l0, i) => {
      // **先剥代码跨**：`[§6.3](…)` 写在反引号里是**举例**（「当年长这样」），
      // 不是指路。不剥会当场误报本判据自己在注释里写的反例——判据红自己，
      // 最快的失守方式。加豁免不如加正确性：代码跨里的 `§` 本来就不是引用。
      const l = l0.replace(/`[^`]*`/g, '``')
      // 形态 1（**已退休**，2026-10-14）：链接显示文字里的 §N.M。
      //   RE-006 纳入编号体系后，文档内的链接一律写成 `[[RE-006-当前事实表#…|…]]`，
      //   由 gen-doc-refs 展开成真实链接——锚点存在性归 **D-007**、地址重算归 **RF-002**，
      //   两条都已在门禁里。这条形态不再出现在文档里，判它就是判一段不存在的东西。
      // 形态 2：散文写全 `CURRENT.md §N.M`，含 markdown 链接形态
      // （`[CURRENT.md](../CURRENT.md) §3.9` —— 中间隔着一个 `)`，
      //  第一版正则漏了它，注入验证当场报「漏过」）
      for (const m of l.matchAll(/RE-006[\w\u4e00-\u9fa5-]*`?\)?\s*§(\d+(?:\.\d+)*)/g)) {
        if (!actual.has(m[1])) {
          bad.push(`${f.rel}:${i + 1} 散文指 RE-006 §${m[1]}，但没有这一节`)
        }
      }
    })
  }
  assert.ok(scanned > 80, `只扫了 ${scanned} 份文档——判据自己失效了`)
  assert.deepEqual(bad, [], `散文里的 RE-006 节次指路失效：\n    ${bad.join('\n    ')}`)
})

test('散文判据的扫描面：文档目录全进语料，工具产物目录一律不进', () => {
  // 反向钉住 `proseDocs` 的范围。两个方向都要判，缺一个都不算钉住：
  //   · 只判「`.workbuddy-ai/` 不进」⇒ 把整个语料砍空也照样绿（判据退化成装饰）；
  //   · 只判「`docs/` 进」⇒ 门禁失败详情（引用被扫文档的原文）会把判据喂成永久红。
  const root = tmp()
  fs.mkdirSync(path.join(root, 'docs', 'ref'), { recursive: true })
  fs.mkdirSync(path.join(root, '.workbuddy-ai', 'gate-logs', '20261015-120438'), { recursive: true })
  fs.mkdirSync(path.join(root, 'node_modules', 'x'), { recursive: true })
  fs.writeFileSync(path.join(root, 'docs', 'ref', 'a.md'), '见 RE-006 §99 那一节\n')
  fs.writeFileSync(
    path.join(root, '.workbuddy-ai', 'gate-logs', '20261015-120438', '_summary.md'),
    '失败详情里引用了「见 RE-006 §99 那一节」这句原文\n',
  )
  fs.writeFileSync(path.join(root, 'node_modules', 'x', 'readme.md'), '见 RE-006 §99\n')
  const rels = proseDocs(root).map((f) => f.rel).sort()
  assert.deepEqual(
    rels,
    ['docs/ref/a.md'],
    `扫描面应恰好是文档目录（实得 ${JSON.stringify(rels)}）——多一个就是把引用当指路，少一个就是把判据砍空`,
  )
})

test('RE-003 契约表的键必须是 §2.3 的静态路由（多一个键判红）', () => {
  // 分工（T-102 定，view 取消后仍成立）：**地址与一句话职责的唯一表是 RE-006 §2.3**
  // （代码投影）；RE-003 只放**契约**（调用方 / 返回类型 / 备注）——以地址为**键**，
  // 与 session README「工具名 → 结果字段」同一种形状（以封闭集成员为键讲另一件事，
  // 见上面判据的「已知边界」）。**没有第二张地址表**，也就没有「两个渲染器分叉」可判。
  //
  // 于是这条判据只守一个方向：**键可以缺**（新路由还没人写契约——缺行不判红），
  // **不能多**（退役路由的残留 / 手写腐化）。`_root` 就曾以这种形态在旧表里：
  // 它在这仓代码里从未存在（`git log -S '"_root"' -- symbio/` 零命中）。
  //
  // 口径：两边都取 `| \`地址\` | …` 形态的表行；表头与分隔行没有反引号地址，天然排除。
  const cur = fs.readFileSync(path.join(REPO, 'docs', 'ref', 'RE-006-当前事实表.md'), 'utf8')
  const re003 = fs.readFileSync(path.join(REPO, 'docs', 'ref', 'RE-003-ROUTES.md'), 'utf8')

  const extraKeysOf = (currentText, routesText) => {
    const i = currentText.indexOf('### 2.3')
    if (i < 0) throw new Error('CURRENT.md 里找不到 §2.3 —— 判据自己失效了')
    const sec = currentText.slice(i, currentText.indexOf('## 3.', i))
    const fromCurrent = new Set([...sec.matchAll(/^\| `([^`]+)` \|/gm)].map((m) => m[1]))
    const j = routesText.indexOf('## 路由契约表')
    if (j < 0) throw new Error('RE-003 里找不到「## 路由契约表」节 —— 判据自己失效了')
    const tbl = routesText.slice(j, routesText.indexOf('\n## ', j + 10))
    const fromRe003 = new Set([...tbl.matchAll(/^\| `([^`]+)` \|/gm)].map((m) => m[1]))
    return [...fromRe003].filter((r) => !fromCurrent.has(r))
  }

  const extra = extraKeysOf(cur, re003)
  assert.ok(
    [...cur.matchAll(/^\| `([^`]+)` \|/gm)].length >= 15,
    '§2.3 静态路由行数异常少 —— 判据自己失效了',
  )
  assert.deepEqual(
    extra,
    [],
    `RE-003 契约表多出这些键——它们不是 §2.3 里的静态路由（退役残留 / 手写腐化）：` +
      `\n    ${extra.join('\n    ')}\n  处置：删掉这一行；确属退役地址的只进 §退役路由索引，不进契约表。`,
  )

  // 反向验证（两条注入都必须被上面同一个 extraKeysOf 抓住）：
  // ① 插一条假静态地址 —— 手写腐化的形态；
  const withFake = re003.replace(
    /(\| 地址 \| 调用方[^\n]*\n\|[-|]+\|\n)/,
    `$1| \`fake/never_exists\` | 注入的反向用例。 | — | — |\n`,
  )
  assert.ok(withFake !== re003, '反向用例失效：假键没插进去（表头正则失配）')
  assert.equal(extraKeysOf(cur, withFake).length, 1, '反向用例失效：插一条假键抓不到，判据是恒真的')
  // ② `_root` —— 旧表的真实病史：它在本仓代码里从未存在，却成表过。
  const withRoot = re003.replace(
    /(\| 地址 \| 调用方[^\n]*\n\|[-|]+\|\n)/,
    `$1| \`_root\` | 注入的病史用例。 | — | — |\n`,
  )
  assert.equal(extraKeysOf(cur, withRoot).length, 1, '反向用例失效：_root 那条病史抓不到，判据是恒真的')
})

test('本仓自身：runner 认得三份事实（本测试不要求此刻无漂移）', async () => {
  // ⚠️ 这里**刻意**不断言 `--check` 全绿：门禁的 `30-docs` 阶段跑本测试，而事实文件由
  // **更靠后的** `60-facts` 阶段重生成。于是「代码改了、事实还没重生成」是本测试运行时
  // 的**正常状态**，断言它全绿等于断言一件还没发生的事——首次接入时就因此假红过一次。
  // 漂移的判定归它的自然位置：重生成之后（`60-facts`）与 CI 的 `--ci` 模式。
  //
  // 本测试只钉「目录被遍历到」这一件与时序无关的事——漏接线时它才红。
  const REQUIRED = ['current', 'gate-codes', 'routes-ts']
  const ids = discover(path.join(REPO, 'scripts', 'facts'))
  for (const id of REQUIRED) {
    assert.ok(ids.includes(id), `runner 应当认得事实 \`${id}\`（实际：${ids.join('、')}）`)
  }
  const mods = await Promise.all(REQUIRED.map((id) => import(`./facts/${id}.mjs`)))
  for (const [i, mod] of mods.entries()) {
    const errs = validateFact(mod.default, { id: REQUIRED[i] })
    assert.deepEqual(errs, [], `本仓事实的描述符必须合契约：${errs.join('；')}`)
  }
})

test('生成物里不得留下字面 refLink(——${} 没插值的静默形态', async () => {
  // 病历（9f49d454）：提交消息写着「新增 `refLink(id, label)`」，diff 里却**只有用法没有
  // 定义**——六处调用写进双引号字符串，`${refLink(...)}` 原样落进 RE-006。两类守卫同时
  // 失效：链接审计看不见它（那不是 markdown 链接），`--check` 也看不见它（生成器确定性
  // 地产出同一份坏内容，永远「一致」）。**声称存在而代码里不存在的函数，比没有函数更坏**。
  //
  // 口径：扫每份事实的**落盘产物**（`out` 字段）。生成先于静态审计（28-facts-write），
  // 所以测试看到的产物就是审计将面对的终态。
  const ids = discover(path.join(REPO, 'scripts', 'facts'))
  const mods = await Promise.all(ids.map((id) => import(`./facts/${id}.mjs`)))
  for (const [i, mod] of mods.entries()) {
    const out = path.join(REPO, mod.default.out)
    const text = fs.readFileSync(out, 'utf8')
    assert.ok(
      !text.includes('refLink('),
      `${mod.default.out} 里有字面 \`refLink(\`——生成器把 \`\${refLink(...)}\` 写进了非模板字符串` +
        `（没插值）。处置：改用模板字面量或字符串拼接，并确认 \`createRefLink\` 真的被 import。`,
    )
  }

  // 反向：病史原文必命中——连提交前的 RE-006 片段都判不出，这条判据就是恒真的。
  const preFix = "> 路由权威清单看 ${refLink('RE-003-ROUTES', 'RE-003-ROUTES.md')}（与本表交叉核对）。"
  assert.ok(preFix.includes('refLink('), '反向用例失效：连病史原文都判不出，真 bug 时它不可能红')
})