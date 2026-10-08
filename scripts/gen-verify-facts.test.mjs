import { test } from 'node:test'
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

import {
  mechanismsOf,
  frameOf,
  assignsOf,
  fallbackOf,
  claimsOf,
  buildFacts,
  generate,
} from './gen-verify-facts.mjs'

const REPO = path.resolve(import.meta.dirname, '..')

// ── 最小可用夹具：01 §8 参数表 + 02 坐标系七张表 + 阶梯总表 + 两阶文档 ──────
//
// 夹具刻意只给两阶、两维：反向用例注入的是**形状**（少一行 / 多一个加粗 / 未知键 /
// 表头改名），与阶数、维数无关；两阶足以暴露「名册配对」「累计点亮全表」的两端。

const ARCH = `
## 8. 权威参数表（唯一一份）

| 参数键 | 取值域 | **平凡值** | 用于 |
|---|---|---|---|
| \`store\` | \`memory\` / \`wal\` | \`memory\` | 部署 |
| \`projection\` | \`snapshot\` / \`recall\` | 仅 \`eval\` | 视图定义 |
| \`projection.param\` | \`recall:tag\` … | — | 投影的取值 |
| \`event.entity\` | \`turn\` / \`task\` | — | 事件目录 |
| \`scope\` | \`root\` / \`child:<id>\` | \`root\` | 递归 |
`

// 02 的七张表：三张定义（§2 维度 / §3 轴 / §3.1 天梯）+ 一张名册（§6.1）
// + 三张声明（§2 条数列、§5 说法、§6.2 空格、§7 社交）。声明与名册在这里刻意自洽——
// 自洽与否由 verify 程序判，生成器只管读得出读不出。
const FRAME = `
## 2. 横向：五维

| 维度 | 回答 | 内涵 | 条数 |
|---|---|---|:-:|
| **知** Episteme | 我知道什么 | 记忆、因果 | 2 |
| **行** Action | 我能做什么 | 工具、承诺 | 2 |

## 3. 纵向：五轴

| 轴 | 低阶 | 高阶 |
|---|---|---|
| **主动性** | 只在被要求时行动 | 自己发起 |
| **时域跨度** | 单轮、当下 | 跨会话 |
| **抽象层级** | 具体指令 | 归纳规则 |

### 3.1 天梯 T1–T3 = 五轴的递进

| 级 | 名称 | 提升的轴 |
|---|---|---|
| T1 | 能对话 | 抽象层级 |
| T2 | 能干活 | 主动性 |
| T3 | 能记住 | 时域跨度 |

## 5. 伪高阶识别

| 说法 | 为什么是伪高阶 |
|---|---|
| 工具数量多 | 是「行」的宽度 |
| 会写代码 | 是技能 |

## 6. 覆盖矩阵（实测）

### 6.1 能力名册

| 能力 | 维 | 轴 | 天梯 |
|---|---|---|:-:|
| 多轮对话 | 知 | 抽象层级 | T1 |
| 情景记忆 | 知 | 时域跨度 | T3 |
| 工具接入 | 行 | 主动性 | T2 |
| 对等协商 | 行 | 抽象层级 | T2 |

### 6.2 覆盖矩阵与生长位

| 空格 | 可能的生长方向 |
|---|---|
| 知 × 主动性 | 自主决定记什么 |
| 行 × 时域跨度 | 长时任务 |

## 7. 社会性

| 能力 | 提升的轴 |
|---|---|
| 对等协商 | 抽象层级 |
`

const LADDER = `
## 1. 阶梯总表

| 阶 | 场景 | 天梯 | 新能力 | **新增机制键** | 参数变化 | 文件 |
|---|---|:-:|---|:-:|:-:|---|
| S01 | 最小闭环 | T1 | 能对话 | **2** | 2 | [→](./S01-a.md) |
| S02 | 工具与产物 | T2 | 能动手 | **2** | 2 | [→](./S02-b.md) |

**机制键累计**：2 → 4
`

const S01 = `
## 3. 增量清单（只加数据）

| 类型 | 内容 |
|---|---|
| 事件格子 | 新增 \`turn/opened\` |

**本阶点亮的机制键**：

\`\`\`capability-assign
event.entity = turn
projection = snapshot
\`\`\`

## 4. 平凡值与回退（J2）

| 参数 | 完整值 | **平凡值** | 平凡值下 |
|---|---|---|---|
| \`projection\` | \`snapshot\` | **\`eval\`** | 仍完整运行 |
| \`event.entity\` | \`turn\` | \`turn\` | 不适用 |
`

const S02 = `
## 3. 增量清单（只加数据）

\`\`\`capability-assign
store = wal
scope = child:<id>
\`\`\`

## 4. 平凡值与回退（J2）

| 参数 | 完整值 | **平凡值** | 平凡值下 |
|---|---|---|---|
| \`store\` | \`wal\` | **\`memory\`** | 仍完整运行 |
`

const fixture = (over = {}) => ({
  archMd: ARCH,
  frameMd: FRAME,
  ladderMd: LADDER,
  stageMds: { S01, S02 },
  dirEntries: ['00-路线图总览.md', 'S01-a.md', 'S02-b.md'],
  ...over,
})

// ── 正向：读得出来，且形状对 ────────────────────────────────────────────

test('§8 的顶层键判据是形状：projection.param 是子键，不另计一个机制', () => {
  const { all, top } = mechanismsOf(ARCH)
  assert.deepEqual(all, ['store', 'projection', 'projection.param', 'event.entity', 'scope'])
  assert.deepEqual(top, ['store', 'projection', 'event.entity', 'scope'])
})

test('生成物带出机制表、名册与每阶赋值——程序侧不需要任何手填计划数字', () => {
  const text = buildFacts(fixture())
  assert.match(text, /pub const MECHANISMS: &\[&str\] = &\[\n {4}"store",/)
  assert.match(text, /claimed_new_keys: 2,/)
  assert.match(text, /\("store", "wal"\),/)
  assert.match(text, /fallback: \("store", "memory"\),/)
  assert.ok(!text.includes('projection.param"\n];'), '子键不得混进 MECHANISMS 那一段')
})

test('新增一个真正的顶层键，MECHANISMS 才会变长——「10 还是 11」由表本身回答', () => {
  const withNew = ARCH.replace(
    '| `scope` | `root` / `child:<id>` | `root` | 递归 |',
    '| `scope` | `root` / `child:<id>` | `root` | 递归 |\n| `vis_scope` | `shared` | `root` | 可见域 |',
  )
  assert.equal(mechanismsOf(withNew).top.length, 5)
})

test('02 的七张表各归各位：定义、名册、三处声明都按所在小节读，不吃错表', () => {
  const f = frameOf(FRAME)
  assert.deepEqual(f.dimensions, ['知', '行'])
  assert.deepEqual(f.axes, ['主动性', '时域跨度', '抽象层级'])
  assert.equal(f.tierMax, 3)
  assert.deepEqual(f.dimCounts, [['知', 2], ['行', 2]])
  assert.deepEqual(f.caps[3], { name: '对等协商', dim: '行', axis: '抽象层级', tier: 2 })
  assert.deepEqual(f.pseudo, ['工具数量多', '会写代码'])
  assert.deepEqual(f.growth, [['知', '主动性'], ['行', '时域跨度']])
  assert.deepEqual(f.social, [['对等协商', '抽象层级']])
})

test('名册与三处声明一起进生成物——program 侧不再有一份 54 行抄本', () => {
  const text = buildFacts(fixture())
  assert.match(text, /pub const DIMENSIONS: &\[&str\] = &\[\n {4}"知",/)
  assert.match(text, /pub const CAPABILITIES: &\[Capability\] = &\[\n {4}Capability \{/)
  assert.match(text, /pub const DECLARED_DIM_COUNTS: &\[\(&str, usize\)\] = &\[\n {4}\("知", 2\),/)
  assert.match(text, /pub const PSEUDO: &\[&str\] = &\[\n {4}"工具数量多",/)
  assert.match(text, /pub const DECLARED_GROWTH: &\[\(&str, &str\)\] = &\[\n {4}\("知", "主动性"\),/)
  assert.match(text, /pub const DECLARED_SOCIAL: &\[\(&str, &str\)\] = &\[\n {4}\("对等协商", "抽象层级"\),/)
})

// ── 反向用例：读不出来 / 两处各说各话，都必须抛，不得静默产出残缺 facts ────

test('反向：删掉总表的一行而阶文件还在 ⇒ 名册与目录不配对', () => {
  const trimmed = LADDER.replace(/^.*S02.*$/m, '')
  assert.throws(() => buildFacts(fixture({ ladderMd: trimmed })), /不是一对一同名/)
})

test('反向：删掉一篇阶文档而总表还声明着它 ⇒ 同一条不配对', () => {
  assert.throws(
    () => buildFacts(fixture({ dirEntries: ['00-路线图总览.md', 'S01-a.md'] })),
    /不是一对一同名/,
  )
})

test('反向：§3 赋值块少一行 ⇒ 与总表声明的「参数变化」数不符', () => {
  const short = S02.replace('store = wal\n', '')
  assert.throws(
    () => buildFacts(fixture({ stageMds: { S01, S02: short } })),
    /§3 赋值块有 1 项，总表声明 S02 的参数变化是 2 项/,
  )
})

test('反向：赋值块用了一个不在 §8 表里的键 ⇒ 指认它是要走 ADR 的新机制', () => {
  const bad = S02.replace('scope = child:<id>', 'wizard.mode = on')
  assert.throws(
    () => buildFacts(fixture({ stageMds: { S01, S02: bad } })),
    /不在 01 §8 参数表里的键 `wizard.mode`/,
  )
})

test('反向：§3 赋值块整块不见 ⇒ 抛错而不是当成「这一阶没有参数变化」', () => {
  const bare = S02.replace(/```capability-assign[\s\S]*?```/, '')
  assert.throws(() => assignsOf(bare, 'S02'), /§3 缺少 ```capability-assign 赋值块/)
})

test('反向：赋值块里一行不是「键 = 取值」⇒ 抛错，不悄悄跳过那个键', () => {
  const odd = S02.replace('store = wal', 'store wal')
  assert.throws(() => assignsOf(odd, 'S02'), /既不是 .键 = 取值. 也不是注释/)
})

test('反向：§4 出现两行加粗平凡值 ⇒ 退路口不唯一，抛错', () => {
  const twice = S02.replace(
    '| `store` | `wal` | **`memory`** | 仍完整运行 |',
    '| `store` | `wal` | **`memory`** | 仍完整运行 |\n| `scope` | `child:<id>` | **`root`** | 这一行也算加粗 |',
  )
  assert.throws(() => fallbackOf(twice, ['store', 'scope'], 'S02'), /应有且只有 1 行/)
})

test('反向：§4 一行加粗都没有 ⇒ 退路口读不出来，抛错', () => {
  const none = S02.replace('**`memory`**', '`memory`')
  assert.throws(() => fallbackOf(none, ['store'], 'S02'), /应有且只有 1 行/)
})

test('反向：§4 只在本文那一节里找表（S02 前面另有一张「参数」表时不误吃）', () => {
  const withEarlier = '## 3.\n\n| 参数 | 今天 |\n|---|---|\n| `projection` | `x` |\n\n' + S02.replace('## 3. 增量清单（只加数据）\n', '')
  assert.deepEqual(fallbackOf(withEarlier, ['store'], 'S02'), ['store', 'memory'])
})

test('反向：§8 表头第三列不叫「平凡值」⇒ 判定列位读歪，抛错', () => {
  assert.throws(() => mechanismsOf(ARCH.replace('**平凡值**', '退路')), /表头不是/)
})

test('反向：§8 那张表整个不见 ⇒ 抛错，绝不返回一张空表', () => {
  assert.throws(() => mechanismsOf('# 01\n\n正文里没有参数表\n'), /找不到「参数键」表头/)
})

test('反向：总表缺「新增机制键」那一列 ⇒ 声明读不出来，抛错', () => {
  assert.throws(
    () => claimsOf(LADDER.replace(/\*\*新增机制键\*\*/g, '新键')),
    /缺少「新增机制键」或「参数变化」列/,
  )
})

test('反向：§6.1 名册少一列 ⇒ 四列的形状不成立，抛错', () => {
  const three = FRAME.replace('| 能力 | 维 | 轴 | 天梯 |\n|---|---|---|:-:|', '| 能力 | 维 | 轴 |\n|---|---|---|')
  assert.throws(() => frameOf(three), /名册必须是「能力 \/ 维 \/ 轴 \/ 天梯」四列/)
})

test('反向：名册里有重名行 ⇒ 计数会静默翻倍，抛错', () => {
  const dup = FRAME.replace('| 对等协商 | 行 | 抽象层级 | T2 |', '| 情景记忆 | 行 | 抽象层级 | T2 |')
  assert.throws(() => frameOf(dup), /名册有重名行/)
})

test('反向：名册的天梯列写成汉字 ⇒ 读不出整数，抛错（不当成 T0）', () => {
  const odd = FRAME.replace('| 多轮对话 | 知 | 抽象层级 | T1 |', '| 多轮对话 | 知 | 抽象层级 | 第一级 |')
  assert.throws(() => frameOf(odd), /§6\.1「多轮对话」的天梯 读不出整数/)
})

test('反向：§2 的条数列写成文字 ⇒ 声明读不出来，抛错', () => {
  const odd = FRAME.replace('| **知** Episteme | 我知道什么 | 记忆、因果 | 2 |', '| **知** Episteme | 我知道什么 | 记忆、因果 | 十余条 |')
  assert.throws(() => frameOf(odd), /§2「知」的条数 读不出整数/)
})

test('反向：§6.2 的空格不写「维 × 轴」⇒ 生长位读不出归属，抛错', () => {
  const odd = FRAME.replace('| 知 × 主动性 | 自主决定记什么 |', '| 自我模型精度 | 自主决定记什么 |')
  assert.throws(() => frameOf(odd), /空格必须写成「维 × 轴」/)
})

test('反向：§3.1 的小节标题改名 ⇒ 天梯表按节定位失败，抛错而不是读成 §3 的五轴表', () => {
  const renamed = FRAME.replace('### 3.1 天梯 T1–T3 = 五轴的递进', '### 3.1 天梯与递进')
    .replace(/^### 3\.1/m, '### 3-1')
  assert.throws(() => frameOf(renamed), /找不到 §3\.1 天梯表/)
})

test('反向：§5 的表不见而小节还在 ⇒ 抛错，绝不在程序里退回一份词表', () => {
  const gone = FRAME.replace('| 说法 | 为什么是伪高阶 |\n|---|---|\n| 工具数量多 | 是「行」的宽度 |\n| 会写代码 | 是技能 |\n\n', '')
  assert.throws(() => frameOf(gone), /§5 伪高阶表里找不到表头首列为「说法」/)
})

test('反向：§6.1 名册整张不见 ⇒ 抛错，绝不返回空名册（空名册会让所有维都变空）', () => {
  const gone = FRAME.replace(/\| 能力 \| 维 \| 轴 \| 天梯 \|\n(\|[^\n]*\n)+\n/, '')
  assert.throws(() => frameOf(gone), /§6\.1 名册里找不到表头首列为「能力」的表/)
})

// ── dogfood：真实仓库的文档必须能被同一份代码读出来，且生成物未漂移 ────────

test('真实 docs/plan 读得出 10 个顶层键与 13 阶名册', () => {
  const text = generate(REPO)
  assert.equal([...text.matchAll(/^ {4}"/gm)].length >= 10, true)
  assert.match(text, /pub const MECHANISMS/)
  assert.equal([...text.matchAll(/^ {4}StageClaim \{$/gm)].length, 13)
  assert.equal([...text.matchAll(/^ {4}StageDoc \{$/gm)].length, 13)
  assert.match(text, /"projection\.param"/, '§8 的子键必须出现在 PARAM_KEYS 里')
})

test('真实 02 读得出坐标系：名册、定义表与三处声明吃的是同一个集合', () => {
  const f = frameOf(fs.readFileSync(path.join(REPO, 'docs', 'plan', '02-能力坐标系.md'), 'utf8'))
  assert.equal(f.dimensions.length, f.dimCounts.length)
  assert.equal(
    f.dimCounts.reduce((s, [, n]) => s + n, 0),
    f.caps.length,
    '§2 的条数声明加起来必须等于名册行数——名册少一行这里就对不上',
  )
  assert.ok(f.caps.every((c) => c.tier >= 1 && c.tier <= f.tierMax), '名册的天梯都落在 §3.1 定义的级数内')
  assert.equal(
    f.growth.length,
    f.dimensions.length * f.axes.length - new Set(f.caps.map((c) => `${c.dim}×${c.axis}`)).size,
    '生长位数 = 总格数 − 名册已占的格数',
  )
  assert.ok(f.pseudo.length > 0 && f.social.length > 0, '词表与社交声明都读得出内容（空表说明读歪了）')
})

test('生成物与 docs/plan/verify/facts/mod.rs 逐字一致（漂移由门禁自动修复，单测钉住判据）', () => {
  const onDisk = fs.readFileSync(path.join(REPO, 'docs', 'plan', 'verify', 'facts', 'mod.rs'), 'utf8')
  assert.equal(onDisk, generate(REPO))
})

test('--check 档：文档与生成物一致 ⇒ exit 0', () => {
  const out = execFileSync(process.execPath, [path.join(REPO, 'scripts', 'gen-verify-facts.mjs'), '--check'], {
    encoding: 'utf8',
    cwd: REPO,
  })
  assert.match(out, /与文档一致/)
})
