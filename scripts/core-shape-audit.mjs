#!/usr/bin/env node
/**
 * core-shape-audit — `symbio_core` 的**公共形状**只许收窄（判定型）
 *
 * ## 它判什么
 *
 * 三格棘轮，全部来自**内核**（DE-023 A2）的五个封闭语法 trait：
 *
 * | 编号 | 格 | 判什么 | 它拦的是哪一种膨胀 |
 * |---|---|---|---|
 * | CS-001 | `traits` | **内核五 trait**（Store / Capability / ModelProvider / VdfsProvider / Registry）的声明数（冻结 5） | **新增接缝**：每多一个内核抽象，就多一处「所有实现方都得知道」的地方。域 trait 不占此格（DE-023 A2 收窄口径） |
 * | CS-001 | `methods` | 上述五 trait 的 `fn` 成员总数（冻结 17） | **契约变宽**：既有接缝上挂新方法，影响面 = 全部实现方 |
 * | CS-001 | `defaults` | 其中**带默认体**的成员数（冻结 2） | **接口上长实现**：行为代码住进架构模块 |
 * | CS-002 | —— | 放宽上面任何一个数字，必须写 `core-shape-allow <格>: ADR-NNN <非空理由>`，且那个 ADR 文件在 `docs/003-design/` **能打开** | **改一个数字**看起来像维护，而它实际是在改架构 |
 * | CS-003 | —— | core 生产代码里 `crate::plugins` 引用数 = 0（**全 core**） | **实现依赖插件**：内核一旦认某个插件的实现坐标，停用/替换那个插件就不再是纯装配动作。依赖方向不随内核边界变化，故不收窄 |
 * | CS-004 | —— | **内核文件集**（`kernel.rs` 的 `core-kernel-files:` 标记）里插件 id 字面量数只许降 | **词汇回流**：在内核写 `"session"` / `"vdfs"` 就是在语法里认一个插件名字——加插件/改 id 因此又变成内核改动（[DE-023](../docs/003-design/023-内核边界与扩展局部性.md "DE-023-内核边界与扩展局部性") D-179） |
 *
 * CS-004 的判定范围是 `kernel.rs` 声明的**内核文件集**（A2 之后 core ≠ 内核）：
 * 域文件（authz / chat / plugin / event_bus / projection …）的插件 id 字面量不是内核
 * 词汇，随 C 的阶段迁出，只在汇总行里报数、不设门禁。文件集之外的 `pub trait`
 * 也不占 CS-001（域 trait 不占格）；`crate::plugins` 引用仍按全 core 判（CS-003）。
 *
 * 表里的冻结值是**现值**，owner 是本文件的 `SHAPE_BASELINE`（下面那一行）；放宽一格
 * 必须按 CS-002 在它旁边留一条 `core-shape-allow`，并有一条能打开的 ADR 论证「必需」。
 *
 * ## 为什么需要它（2026-10-12 的实测缺口）
 *
 * 2026-10-12 那一批「`VdfsProvider` 去样板」的处方（把单点分发推广成默认实现）要把
 * `symbio_core::VdfsProvider` 从**一个方法**扩成**十个方法**。按当时全部门禁的口径算一遍：
 *
 * - `core-export-audit` C-001 / C-002：动的是那个符号**内部**，根出口一个字符没改；
 * - C-003：`VdfsProvider` 早就是多消费方导出，它数的是**符号表**（Map 的键），
 *   方法数不在其视野内——符号 +0，形状 ×10；
 * - `dead-code` / `core-naming` / `no-direct-call` / `mechanism-audit`：判据里都没有
 *   「一个 trait 有几个成员」这一项。
 *
 * ⇒ 一次把核心契约面扩到十倍、并在接口上多挂 9 个默认体的改动，**门禁全绿也能提交**。
 * 而这条纪律本仓早就写了：`08 §0 纪律 3`「架构改动必须逐条论证『必需』」，登记条目
 * [SP-007](../docs/005-spec/007.md)——它的判据写的是「论证不出即删（**人工核对**）」。
 * 于是「架构改动」的全部约束，取决于执行的人**记得**去核对；而批次执行时最不缺的就是
 * 合理化：「加 9 个方法是实现层的样板收敛，不算架构改动」。
 *
 * **本脚本把这条纪律从「记得去看」变成「改了就红」。** 它不判某个 trait 该不该宽——那是
 * 论证的事；它只保证：让核心形状变宽，必须显式触碰一处基线数字，并留下一条能被人打开的
 * 决策。红不红不由执行者判断。
 *
 * ## 为什么只有这三格（口径边界，别顺手加）
 *
 * - **不判行数**：`08 §3.3` 写着「加了数据 / 加了调用点 / 改了文档，都不是架构改动」，
 *   而契约加字段是数据变化——行棘轮会对它红，那是误报；一个只会误报的守卫最后一定被人
 *   喂豁免喂到失效（本仓为此写过一整节，见 `core-export-audit.mjs` 的 W1 注）。
 * - **不判枚举变体数**：变体**是**形状量（`08 §3.5` 把「`NodeKind` 新取值」列为不新增），
 *   但它与「trait 宽度」不是同一个失效面。要加就单独论证一次，不在本批顺手扩判据面。
 * - **不判 `impl` 数 / 宏**：宏展开的形状静态读不到（`08 §3` 的判据本来也不看这个）。
 *
 * ## 与既有判据的分工（一个事实只有一个 owner）
 *
 * | 谁 | 判什么 |
 * |---|---|
 * | 本脚本 | core 公共**形状**的量（三格）+ 放宽痕迹必须挂得住一条决策 |
 * | `core-export-audit`（C-001…C-003） | 根出口的**归属**：什么能出现在公共面上、谁在用 |
 * | `gate.d/35-baseline` | 基线数字本身**不得比上一版更松**（比对 git 基准版本）；本脚本的三格登记在它的 `RATCHETS` 清单里，`CORE_SHAPE_BASELINE` 的环境变量覆盖由它兜 |
 * | [SP-007](../docs/005-spec/007.md) | 纪律**条文**本身（owner 在 `08 §3`），本脚本是它的判据执行方 |
 *
 * CS-002 与 `35-baseline` 各判半条：那边只发现「数字变松了」（红不红取决于有没有豁免注释），
 * 这边判「这条豁免**够不够格**」。两边合起来才是完整的「放宽 = 写决策」。
 *
 * ## 已知边界（写清楚，免得把绿灯当成证明）
 *
 * 判定基于**去注释、剔测试模块后的文本**，不是 AST：
 * - `pub trait` 必须写成 `pub trait <名>`（`pub unsafe trait` 读不到）；
 * - 宏生成的 trait / 成员读不到（本仓的 `define_string_key!` 生成的是 impl，不是 trait）；
 * - 成员签名扫描**不跟踪尖括号**（`->` 里那个 `>` 会把括号计数拖成负数）——代价是
 *   「泛型参数里带 `{`」这种罕见形状会被看成带默认体，**只会多数不会少数**，棘轮方向上安全；
 * - 它挡的是「顺手改宽」，不挡「刻意放宽」——后者正是 CS-002 要求留决策的原因。
 *
 * ## 用法
 *   node scripts/core-shape-audit.mjs              # 判定
 *   node scripts/core-shape-audit.mjs --verbose    # 逐 trait 形状（成员数 / 其中默认体）
 *   node scripts/core-shape-audit.mjs --churn      # 只出「近 50 次提交的内核改动占比」报告
 *   node scripts/core-shape-audit.mjs --root=<dir> # 指向别处的仓库（测试用）
 * 退出码：0 = 通过，1 = 违规。
 *
 * 环境变量 `CORE_SHAPE_BASELINE=<traits>:<methods>:<defaults>` **只给回归测试用**（真仓门禁
 * 不设）；它不动任何文件就能把上限放到天边，所以 `gate.d/35-baseline.mjs` 另判一次
 * 「覆盖值不得比默认值更松」。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns CS core 形状
// @codes CS-001 CS-002 CS-003 CS-004

import fs from 'node:fs'
import path from 'node:path'
import { execSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'
import { isTestFile, walk } from './core-surface.mjs'
import { blankComments, blankTestModules, matchBrace } from './rust-scan.mjs'

const argv = process.argv.slice(2)
const VERBOSE = argv.includes('--verbose')
const CHURN = argv.includes('--churn')
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

/**
 * `--churn`：最近 N 次非 merge 提交里，`symbio/src/symbio_core/` 的改动占比。
 *
 * **报告，不设门禁**（[DE-023](../docs/003-design/023-内核边界与扩展局部性.md "DE-023-内核边界与扩展局部性") A1）：
 * 单次提交的占比没有阈值可定——修内核 bug 的提交本就该 100% 落在内核；判据是**走向**
 * （扩展类提交占比趋 0）。形状由 CS-001…CS-004 判，走向由本报告看，两者不互相冒充。
 *
 * 不落 `CURRENT.md`：那份事实表只收「离线可重生成的代码投影」（`facts/README.md`），
 * 而 git 历史每次提交都变——写进去会让门禁每轮都「修复 1 个文件」，把漂移判据淹掉。
 */
function printChurn(windowSize = 50) {
  let raw
  try {
    raw = execSync(`git log -${windowSize} --no-merges --numstat --format=%x1e`, {
      cwd: ROOT,
      encoding: 'utf8',
      maxBuffer: 32 * 1024 * 1024,
    })
  } catch {
    console.log(yellow('内核改动占比：`git log` 不可读（浅克隆 / 无 git），本节缺数'))
    return
  }
  let commits = 0
  let coreCommits = 0
  let total = 0
  let core = 0
  let counted = false
  for (const line of raw.split('\n')) {
    if (line.trim() === '\x1e') {
      commits += 1
      counted = false
      continue
    }
    const m = line.match(/^(\d+|-)\t(\d+|-)\t(.+)$/)
    if (!m) continue
    const n = (m[1] === '-' ? 0 : Number(m[1])) + (m[2] === '-' ? 0 : Number(m[2]))
    total += n
    if (m[3].startsWith('symbio/src/symbio_core/')) {
      core += n
      if (!counted) {
        coreCommits += 1
        counted = true
      }
    }
  }
  if (commits === 0) {
    console.log(yellow('内核改动占比：窗口内没有提交，本节缺数'))
    return
  }
  const pct = (a, b) => (b === 0 ? '0.0' : ((a / b) * 100).toFixed(1))
  console.log(`内核改动占比（最近 ${commits} 次非 merge 提交，报告不设门禁）`)
  console.log(
    `  动过内核的提交：${coreCommits} / ${commits}（${pct(coreCommits, commits)}%）`,
  )
  console.log(`  内核改动行：${core} / ${total}（${pct(core, total)}%）`)
  console.log(dim('  判据是走向：扩展类提交的内核占比应逐步趋 0。'))
}

if (CHURN) {
  printChurn()
  process.exit(0)
}

const CORE_REL = 'symbio/src/symbio_core'
/** 本脚本自己：CS-002 读它源码里的放宽痕迹 */
const SELF_REL = 'scripts/core-shape-audit.mjs'

/**
 * 决策条目是否存在：`D-NNN` 是设计文档内的锚点（`### D-NNN <标题>`），不是独立文件。
 * 全量扫一遍设计目录（`docs/003-design/*.md` + 组件级就地文档）后缓存。
 */
let decisionSet = null
function hasDecisionAnchor(id) {
  if (decisionSet === null) {
    decisionSet = new Set()
    const roots = [
      path.join(ROOT, 'docs', '003-design'),
      path.join(ROOT, 'symbio', 'src', 'plugins'),
      path.join(ROOT, 'tauri', 'docs'),
      path.join(ROOT, 'cli', 'docs'),
    ]
    const walk = (dir) => {
      let list
      try {
        list = fs.readdirSync(dir, { withFileTypes: true })
      } catch {
        return
      }
      for (const ent of list) {
        const abs = path.join(dir, ent.name)
        if (ent.isDirectory()) {
          if (ent.name === 'node_modules' || ent.name === 'target' || ent.name === 'archive') continue
          walk(abs)
          continue
        }
        if (!ent.name.endsWith('.md')) continue
        let text = ''
        try {
          text = fs.readFileSync(abs, 'utf8')
        } catch {
          continue
        }
        for (const line of text.split('\n')) {
          const m = /^#{2,6}\s+(D-\d{3}a?)\b/.exec(line)
          if (m) decisionSet.add(m[1].slice(2))
        }
      }
    }
    for (const r of roots) walk(r)
  }
  return decisionSet.has(id)
}

/**
 * 形状基线（2026-10-12 冻结，决策与理由见 [D-055](../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
 * 三格都是**上限**，只许降。数字本身不是判据，**改数字才是**：任何让形状变宽的改动都会
 * 撞红，而把它继续推下去只有一条路——改这一行的数字，那又会要求 CS-002 的一条
 * `core-shape-allow <格>: ADR-NNN <理由>`，且那个 ADR 文件必须真实存在。
 * 于是「顺手把架构改宽一点」在机制上等于「写一份决策」，而不是「改一个数字」。
 *
 * ⚠️ 键序就是 `CORE_SHAPE_BASELINE` 的位置口径（`35-baseline` 的 env 覆盖判定按它 zip），
 * 也必须是 `35-baseline` 那条提取函数认的键名；任何一边单改都会红。
 */
// 内核五 trait 是**封闭语法**（DE-023 §1）：它们的形状只许收窄。
// 其余 trait 是域协议（Plugin / LlmAdapter / DeltaSink / …），随 B2/C 迁出内核，
// 不占这条棘轮——在那里加成员不等于「内核接缝变宽」。
const KERNEL_TRAITS = ['Store', 'Capability', 'ModelProvider', 'VdfsProvider', 'Registry']

const SHAPE_BASELINE = process.env.CORE_SHAPE_BASELINE
  ? (() => {
      const [traits, methods, defaults] = process.env.CORE_SHAPE_BASELINE.split(':').map((n) => Number(n))
      return { traits, methods, defaults }
    })()
  : { traits: 5, methods: 17, defaults: 2 }

const CELL_LABEL = {
  traits: '`pub trait` 数（接缝数）',
  methods: '`fn` 成员总数（契约面宽度）',
  defaults: '带默认体的成员数（接口上的实现）',
}

/** 放宽痕迹的读法：与 `35-baseline` 的 `waiverReader('core-shape-allow')` 同一条拼写约定 */
const WAIVER_RE = /^ {0,2}\/\/\s*core-shape-allow\s+([A-Za-z0-9_]+)\s*:\s*(\S[^\n]*)$/gm
const ADR_REF_RE = /(?<!-)(?:D|ADR)-(\d+[a-z]?)/g

const errors = []
const report = (rule, msg) => errors.push({ rule, msg })

// ==================== 读形状 ====================

/**
 * 数一个 trait 体的形状（体已剥掉外层花括号）。
 *
 * 成员 = 深度 0 上的 `fn`；「带默认体」= 该 `fn` 的签名以 `{` 收尾而不是 `;`。命中默认体后
 * 用 `matchBrace` 跳过整个体，否则体内的 `fn`（闭包 / 局部函数）会被当成成员。
 */
function traitShape(body) {
  let fns = 0
  let defaults = 0
  let depth = 0
  for (let i = 0; i < body.length; i++) {
    const c = body[i]
    if (c === '{' || c === '(' || c === '[') {
      depth++
      continue
    }
    if (c === '}' || c === ')' || c === ']') {
      depth--
      continue
    }
    if (depth !== 0 || c !== 'f' || !body.startsWith('fn', i)) continue
    if (!/[^A-Za-z0-9_]/.test(body[i - 1] ?? ' ')) continue
    const m = /^fn\s+([A-Za-z0-9_]+)/.exec(body.slice(i))
    if (!m) continue
    fns++
    let d = 0
    let open = -1
    for (let j = i + 2; j < body.length; j++) {
      const k = body[j]
      if (k === '(' || k === '[') d++
      else if (k === ')' || k === ']') d--
      else if (d === 0 && k === '{') {
        open = j
        break
      } else if (d === 0 && k === ';') break
    }
    if (open >= 0) {
      defaults++
      i = matchBrace(body, open)
    }
  }
  return { fns, defaults }
}

/**
 * 内核文件集：`kernel.rs` 的 `core-kernel-files:` 标记是**唯一声明处**。
 *
 * 为什么按文件而不是按目录：内核是**语法**的集合，语法声明散在域文件里
 * （`capability/mod.rs` 有 `Capability`/`Registry`，`vdfs/provider.rs` 有 `VdfsProvider`），
 * 按目录划会把整个域的行为也算进内核。文件集只回答「哪几个文件是内核边界」。
 *
 * 夹具仓库没有门面 ⇒ 返回 null，退回全 core 口径（夹具只验棘轮本身）。
 */
function readKernelScope(coreDir) {
  const file = path.join(coreDir, 'kernel.rs')
  if (!fs.existsSync(file)) return null
  const m = fs.readFileSync(file, 'utf8').match(/^[ \t]*\/\/[ \t]*core-kernel-files:[ \t]*(.+)$/m)
  if (!m) return null
  const rels = m[1].trim().split(/\s+/).filter(Boolean)
  const abs = new Set()
  const missing = []
  for (const rel of rels) {
    const p = path.join(coreDir, rel)
    if (!fs.existsSync(p)) missing.push(rel)
    abs.add(p)
  }
  return { rels, abs, missing }
}

/**
 * core 生产侧每个 `pub trait` 的形状。
 *
 * 测试文件与 `#[cfg(test)]` 模块都不算——口径与 `core-export-audit` C-003 一致：
 * **测试不是架构消费方**，测试里的 trait 形状也不是长期接口。
 * 传入内核文件集时只量内核（`kernelScope` 之外的文件是域行为，随 C 迁出）。
 */
function measureCoreShape(coreDir, kernelFiles = null) {
  const all = []
  for (const file of walk(coreDir)) {
    if (!file.endsWith('.rs') || isTestFile(file)) continue
    if (kernelFiles && !kernelFiles.has(file)) continue
    const txt = blankTestModules(blankComments(fs.readFileSync(file, 'utf8')).join('\n'))
    for (const m of txt.matchAll(/\bpub\s+trait\s+([A-Za-z0-9_]+)/g)) {
      const open = txt.indexOf('{', m.index)
      if (open < 0) continue
      const body = txt.slice(open + 1, matchBrace(txt, open))
      all.push({
        name: m[1],
        file: path.relative(ROOT, file).split(path.sep).join('/'),
        ...traitShape(body),
      })
    }
  }
  const rows = all.filter((r) => KERNEL_TRAITS.includes(r.name))
  rows.sort((a, b) => b.fns - a.fns || a.name.localeCompare(b.name))
  return {
    rows,
    all,
    values: {
      traits: rows.length,
      methods: rows.reduce((s, r) => s + r.fns, 0),
      defaults: rows.reduce((s, r) => s + r.defaults, 0),
    },
  }
}

const coreDir = path.join(ROOT, CORE_REL)
if (!fs.existsSync(path.join(coreDir, 'mod.rs'))) {
  console.error(red(`✗ 读不出 core 范围：${CORE_REL}/mod.rs 不存在（root=${ROOT}）—— 判据没有输入，宁红不静默`))
  process.exit(1)
}
const kernelScope = readKernelScope(coreDir)
if (kernelScope && kernelScope.missing.length > 0) {
  report(
    'CS-004',
    `内核文件集指向不存在的文件：${kernelScope.missing.join(' / ')} —— 门面声明的边界必须可解析，` +
      '否则 CS-001 / CS-004 会在一个缩小了的范围上恒绿（宁红不静默）',
  )
}
const shape = measureCoreShape(coreDir, kernelScope?.abs)

// 封闭语法里少一个 trait 不是「收窄成功」，是判据失去输入：宁红不静默。
// 夹具仓库（没有 plugins/mod.rs）只造一两个 trait 验棘轮本身，不要求凑齐五格。
const hasPlugins = fs.existsSync(path.join(ROOT, 'symbio/src/plugins/mod.rs'))
if (hasPlugins) {
  for (const name of KERNEL_TRAITS) {
    if (!shape.rows.some((r) => r.name === name)) {
      report('CS-001', `内核 trait \`${name}\` 读不出来——封闭语法被删/改名，棘轮失去一格输入`)
    }
  }
}

// ==================== CS-001 形状棘轮 ====================

for (const [key, cap] of Object.entries(SHAPE_BASELINE)) {
  if (!Object.hasOwn(shape.values, key)) {
    report('CS-001', `基线里有一格 \`${key}\` 不是形状量（只有 ${Object.keys(shape.values).join(' / ')}）`)
    continue
  }
  if (!Number.isFinite(cap)) {
    report('CS-001', `基线格 \`${key}\` 不是数字（${cap}）—— 判据读不出即失效，宁红不静默`)
    continue
  }
  const now = shape.values[key]
  if (now > cap) {
    report(
      'CS-001',
      `核心形状变宽：${CELL_LABEL[key]} 实测 ${now} > 基线 ${cap} —— symbio_core 的公共形状只许收窄。` +
        `要让这一步成立，先在 \`docs/003-design/\` 立一条 ADR 逐条论证「必需」（08 §0 纪律 3，登记条目 SP-007；` +
        `被这份棘轮钉住的先例见 D-055），再在基线旁写 core-shape-allow ${key}: ADR-NNN <理由>。逐 trait 清单：--verbose`,
    )
  }
}

// ==================== CS-002 放宽必须挂得住一条决策 ====================

/**
 * 读本脚本**自己**源码里那几行放宽痕迹。
 *
 * 为什么要读自己：棘轮的放宽口就是「把 SHAPE_BASELINE 的数字改大」这一行，而那一行的
 * 合法性（有没有挂得住的决策）只有它自己知道。`35-baseline` 比对的是基准版本的数字，
 * 它只知道「变松了、旁边有没有注释」；注释**够不够格**在这里判——两半合起来才是
 * 「放宽 = 写一份能打开的决策」。
 *
 * 夹具仓库里可能没有本脚本 ⇒ 读不到就跳过（与 `core-surface` 的 `walk` 同一约定：
 * 路径不存在不是违规）。
 */
const waivers = []
const selfPath = path.join(ROOT, SELF_REL)
if (fs.existsSync(selfPath)) {
  const selfText = fs.readFileSync(selfPath, 'utf8').replace(/\r\n/g, '\n')
  for (const m of selfText.matchAll(WAIVER_RE)) {
    const [, key, rawReason] = m
    const reason = rawReason.trim()
    waivers.push({ key, reason })
    if (!Object.hasOwn(SHAPE_BASELINE, key)) {
      report('CS-002', `\`core-shape-allow ${key}\` 放宽的不是形状基线的格（只有 ${Object.keys(SHAPE_BASELINE).join(' / ')}）`)
      continue
    }
    const refs = [...reason.matchAll(ADR_REF_RE)].map((r) => r[1])
    if (refs.length === 0) {
      report(
        'CS-002',
        `放宽 \`${key}\` 的理由里没有 ADR 编号（\`${key}: ${reason}\`）—— 动棘轮数字必须挂一条**决策**。` +
          `「批次需要 / 顺手 / 待补 / 见对话」都不算`,
      )
      continue
    }
    for (const id of refs) {
      if (!hasDecisionAnchor(id)) {
        report(
          'CS-002',
          `放宽 \`${key}\` 引用了 \`D-${id}\`，但设计文档里没有任何 \`### D-${id}\` 条目 —— 指向不存在的决策的豁免不算豁免`,
        )
      }
    }
  }
}

// ==================== CS-003 / CS-004 kernel 纯度 ====================

/**
 * 插件的**目录名**就是插件 id 的词表来源：新增插件 = 新目录，不需要改本脚本。
 * 目录不存在（夹具仓库）⇒ 词表为空 ⇒ CS-004 不适用（CS-003 照判）。
 */
const PLUGINS_REL = 'symbio/src/plugins'
const pluginsDir = path.join(ROOT, PLUGINS_REL)
const pluginIds = fs.existsSync(pluginsDir)
  ? fs
      .readdirSync(pluginsDir, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .map((e) => e.name)
      .sort()
  : []

/** core 生产侧（剔测试模块 / 注释）的全文；CS-003 / CS-004 的输入。 */
const coreTexts = []
for (const file of walk(coreDir)) {
  if (!file.endsWith('.rs') || isTestFile(file)) continue
  const text = blankTestModules(blankComments(fs.readFileSync(file, 'utf8')).join('\n'))
  coreTexts.push({ rel: path.relative(ROOT, file).split(path.sep).join('/'), text })
}

/**
 * CS-004 的判定范围：内核文件集（门面 `core-kernel-files:` 标记）。
 * CS-003 仍扫**全部 core**——依赖方向（core 不得认插件实现坐标）不随内核边界变化；
 * 域行为里出现插件 id 字面量不是内核纯度问题，随 C 迁出，只做报告。
 */
const purityTexts = kernelScope
  ? coreTexts.filter((f) => kernelScope.abs.has(path.join(ROOT, f.rel)))
  : coreTexts


// ── CS-003 `crate::plugins` 引用 0 ──
const pluginRefs = coreTexts.flatMap((f) =>
  [...f.text.matchAll(/crate\s*::\s*plugins\b/g)].map(() => f.rel),
)
if (pluginRefs.length > 0) {
  report(
    'CS-003',
    `core 生产代码引用了插件实现 ${pluginRefs.length} 处（${[...new Set(pluginRefs)].join(' / ')}）——` +
      '内核只该经 trait / 声明与插件交互（DE-023 D-179）；注释里提到不算，但代码路径算',
  )
}

// ── CS-004 插件 id 字面量只许降 ──
/**
 * 逐文件冻结存量（2026-10-26 实测值）。**只许降**：数字变小就同步改这里，
 * 变大 = 内核又认了一个插件名字。整张表的目标是空表。
 *
 * 口径：`"<id>"` / `"<id>:` / `"<id>/` 三种字面量起点，逐个插件目录名匹配；
 * 测试模块与注释已剥离（测试里按名造插件是对的）。
 */
const NOUN_BASELINE = {
  // 2026-10-26（DE-023 A2）：判定范围收窄到**内核文件集**（`kernel.rs` 的
  // `core-kernel-files:` 标记）。此前 10 个文件 44 处是「core = 内核」时代的账；
  // 内核文件集当前 0 处。域文件（authz / chat / plugin / event_bus / projection …）
  // 的字面量随 C 迁出，只做报告、不设门禁——它们不是内核词汇（D-177）。
}

const nounCounts = new Map()
const purityRels = new Set(purityTexts.map((f) => f.rel))
let domainNounTotal = 0
if (hasPlugins && pluginIds.length > 0) {
  for (const f of purityTexts) {
    let n = 0
    for (const id of pluginIds) {
      const re = new RegExp(`"${id}"|"${id}[:/]`, 'g')
      n += (f.text.match(re) || []).length
    }
    if (n > 0) nounCounts.set(f.rel, n)
  }
  for (const f of coreTexts) {
    if (purityRels.has(f.rel)) continue
    for (const id of pluginIds) {
      const re = new RegExp(`"${id}"|"${id}[:/]`, 'g')
      domainNounTotal += (f.text.match(re) || []).length
    }
  }
  const ratchetDown = []
  for (const [rel, n] of nounCounts) {
    const cap = NOUN_BASELINE[rel] ?? 0
    if (n > cap) {
      report(
        'CS-004',
        `插件 id 字面量回流：${rel} 有 ${n} 处 > 冻结 ${cap} —— ` +
          '词汇（插件 id / 路由 / 装配位）由 owner 声明，内核只 seal（DE-023 D-177 / D-179）',
      )
    } else if (n < cap) {
      ratchetDown.push(`${rel} ${n}<${cap}`)
    }
  }
  for (const rel of Object.keys(NOUN_BASELINE)) {
    if (!nounCounts.has(rel)) ratchetDown.push(`${rel} 0<${NOUN_BASELINE[rel]}`)
  }
  if (ratchetDown.length && errors.length === 0) {
    console.log(
      yellow('提示') +
        `：插件 id 字面量已收到冻结值下方（${ratchetDown.join(' · ')}），` +
        '请同步本脚本的 NOUN_BASELINE——留着旧数字等于把余量白留给下一次',
    )
  }
}


const ruleLine = (ok, rule, desc) =>
  console.log(`${ok ? green('✓') : red('✗')} ${rule}  ${dim(desc)}`)

const cells = Object.keys(SHAPE_BASELINE)
const shapeLine = cells
  .map((k) => `${k} ${shape.values[k]}/${SHAPE_BASELINE[k]}`)
  .join(' · ')

if (errors.length === 0) {
  console.log('\n== core-shape-audit ==')
  ruleLine(true, 'CS-001', `形状棘轮：${shapeLine}`)
  ruleLine(
    true,
    'CS-002',
    waivers.length
      ? `放宽 ${waivers.length} 格，引用的 ADR 全部能打开：${waivers.map((w) => `${w.key}←${w.reason.match(ADR_REF_RE)?.[0]}`).join('、')}`
      : '没有放宽痕迹（基线就是当前形状）',
  )
  const nounTotal = [...nounCounts.values()].reduce((s, n) => s + n, 0)
  ruleLine(true, 'CS-003', '内核纯度：core 生产代码零 `crate::plugins` 引用')
  ruleLine(
    true,
    'CS-004',
    pluginIds.length
      ? `内核纯度：内核文件集插件 id 字面量 ${nounTotal} 处（冻结 ${Object.values(NOUN_BASELINE).reduce((s, n) => s + n, 0)} 处 / ${Object.keys(NOUN_BASELINE).length} 个文件，只许降）；` +
        `域文件 ${domainNounTotal} 处（C 迁出账，不设门禁）`
      : '内核纯度：夹具仓库没有插件目录，CS-004 不适用',
  )
  const under = cells.filter((k) => shape.values[k] < SHAPE_BASELINE[k])
  if (under.length) {
    console.log(
      yellow('提示') +
        `：已收到基线下方（${under.map((k) => `${k} ${shape.values[k]}<${SHAPE_BASELINE[k]}`).join(' · ')}），` +
        `请同步本脚本的 SHAPE_BASELINE——留着旧数字等于把余量白留给下一次`,
    )
  }
  if (VERBOSE) {
    console.log('  —— pub trait 形状（成员数 / 其中带默认体）')
    for (const r of shape.rows)
      console.log(`  [T] ${r.name.padEnd(26)} ${String(r.fns).padStart(2)}/${String(r.defaults).padEnd(2)} ${r.file}`)
  }
  process.exit(0)
}

console.error(`\n== core-shape-audit：${errors.length} 处违规 ==`)
for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
console.error(
  `\n${yellow('提示')}：形状量的口径与不判什么见本脚本文件头；放宽口径见 ` +
    `${green('docs/003-design/001-核心架构.md')}；逐 trait 清单跑 node scripts/core-shape-audit.mjs --verbose`,
)
if (VERBOSE) {
  for (const r of shape.rows)
    console.error(`  [T] ${r.name.padEnd(26)} ${String(r.fns).padStart(2)}/${String(r.defaults).padEnd(2)} ${r.file}`)
}
process.exit(1)
