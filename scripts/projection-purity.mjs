#!/usr/bin/env node
/**
 * projection-purity — 投影目录的**二级纯净性**与 L2「投影 A 不读投影 B」（判定型）
 *
 * ## 它判什么
 *
 * `Projection::new` 的泛型约束是**一级**强制（句柄进不来，编译期）——但它拦不住
 * 「形参表合法、体内偷看全局」（DE-001 §1.4.1 的诚实划界）。本脚本是**二级**：
 *
 * | 编号 | 规则 | 它拦的是哪一种回潮 |
 * |---|---|---|
 * | PP-001 | 产投影文件不得 `use` 导入**同根产投影的兄弟**（不含 `Projection::new` 的共享读侧——`mutations` / `rebuild` / `turns` / `prompt_fidelity` / `tiers`——谁都可以导） | 投影 A 读投影 B 的输出（L2，DE-001 §13.2）：视图叠视图，外壳 `degraded` / 预算语义在层间静默丢失 |
 * | PP-002 | 折叠目录里的生产文件禁 `SystemTime::now` / `rand::` / `static mut` / `std::time::Instant` | 隐式全局依赖：时钟、随机数、全局可变状态——确定性（N1）的口径外漏洞 |
 * | PP-003 | 每个 `pub fn X(..) -> Projection<` 构造器必须有**折叠目录之外**的生产消费方，或同文件里一行 `// projection-purity-allow <X>: 非空理由` | 死构造器：抽了纯函数视图却没有调用方——「为将来准备」的投机抽象（本仓只认消费方定的契约） |
 * | PP-004 | core 投影面的两格存量——**产投影文件数**与**投影构造器数**——只许降；放宽要按 PP-005 在本脚本旁留一条挂得住的决策 | **投影长回 core**：能力折叠一旦住在 `symbio_core/projection/`，「加一个能力」就又成了改架构模块。下沉判据是消费方分布（只有一个 owner 在读 ⇒ 归那个 owner），而这条把「又长回来」变成机器可判的红（[DE-025](../docs/003-design/025-能力包与扩展面.md "DE-025-能力包与扩展面") D-221） |
 * | PP-005 | 上面两格的放宽注释 `// projection-purity-allow <格>: D-NNN <非空理由>` 里的 `D-NNN` 必须在设计文档里有 `### D-NNN` 锚点 | 「改一个数字」看起来像维护：注释写得再像理由，指向不存在的决策就不算豁免（与 CS-002 同一条判据形状） |
 *
 * ## 判定面：折叠搬到哪儿，判据跟到哪儿
 *
 * PP-001 / PP-002 / PP-003 扫的是**所有折叠根**：`crates/symbio-core/src/projection/`
 * 加上每个 crate 的 `src/projections/`（D-221 把只被一个 owner 读的折叠交还 owner 之后，
 * 判据不能跟着目录一起留下——否则「迁出 core」顺部就变成「逃出纯净性判定」）。
 * PP-004 只判 **core 那一根**：它是横向扩展的棘轮，不是「仓库里一共有几条折叠」。
 *
 * ## PP-004 的口径边界（写清楚，免得把绿灯当成证明）
 *
 * 它数的是**core 这一侧的投影面**，不是「core 有多少 `.rs`」：`vocab.rs` / `task_vocab.rs`
 * 是折叠的**参数表**（不产 `Projection`），`mod.rs` 是 trait 本身——它们长宽都不判。
 * 目标是把两格继续往下推：今天还留在 core 的六个构造器（`calibration_for` / `cost_ledger`
 * / `readyset_for` / `reputation` / `transcript` / `turnstate`）各有 ≥2 个非 core 的 crate
 * 在读（`loop` / `session`），读者迁完它们就该跟着走。跨插件想读别人的折叠，走
 * `symbio_plugin_api::views` 的按名字读口，**不要**在 core 里再写一份。
 *
 * 环境变量 `CORE_PROJECTION_BASELINE=<files>:<ctors>` **只给回归测试用**（真仓门禁不设）；
 * 它能一次调用把上限放到天边，所以 `gate.d/35-baseline.mjs` 另判一次「覆盖值不得比
 * 默认值更松」——与 `CORE_SHAPE_BASELINE` 同一条分工。
 *
 * 「产投影文件」的判定：折叠根下（排除 `*.test.rs`）含 `Projection::new` 的文件。
 * `mod.rs` 是框架本身（`new` 的定义处），不算产投影。
 *
 * ## 为什么是文本扫描而不是 AST
 *
 * 与 `facts-boundary-audit` 同一口径：判据是「某个 token 出没」，文本扫描的
 * 漏报面（宏拼接、路径别名）已知且可接受——它挡的是「顺手写回去」，不挡
 * 「刻意开新口」；刻意者得先动这份判据或写豁免，那正是要的人工步骤。
 *
 * ## 用法
 *   node scripts/projection-purity.mjs              # 判定真实仓库
 *   node scripts/projection-purity.mjs --root=<dir> # 指向别处的仓库（回归测试用）
 * 退出码：0 = 通过；1 = 违规。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns PP 投影纯净性
// @codes PP-001 PP-002 PP-003 PP-004 PP-005

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim, yellow } from './color.mjs'
import { hasDecisionAnchor as anchorExists } from './decision-anchors.mjs'
import { isTestFile, walk } from './core-surface.mjs'
import { blankComments } from './rust-scan.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(scriptDir, '..')

const PROJ_REL = 'crates/symbio-core/src/projection'
const PROJ_DIR = path.join(ROOT, PROJ_REL)

const IMPURITY_TOKENS = [
  'SystemTime::now',
  'rand::',
  'static mut',
  'std::time::Instant',
]

/** PP-002 豁免：逐行 `// projection-purity-allow <码或构造器>: 非空理由`。 */
const ALLOW_RE = /^.*\/\/\s*projection-purity-allow\s+(\S+)\s*:\s*(\S.*)$/

const errors = []
const fail = (m) => errors.push(m)

/** 决策锚点是否存在（扫描口径在 `decision-anchors.mjs`，三个审计共用一份）。 */
const hasDecisionAnchor = (id) => anchorExists(id, ROOT)

const relPosix = (abs) => path.relative(ROOT, abs).split(path.sep).join('/')

/** 生产 .rs（排除 *.test.rs / tests.rs），返回 { rel, text(去注释), raw }。 */
function loadProduction(dir) {
  return [...walk(dir)]
    .filter((abs) => abs.endsWith('.rs') && !isTestFile(abs))
    .sort()
    .map((abs) => {
      const rel = relPosix(abs)
      const raw = fs.readFileSync(abs, 'utf8')
      return { rel, raw, text: blankComments(raw) }
    })
}

if (!fs.existsSync(PROJ_DIR)) {
  console.error(red('✗ 读不出投影目录：PROJ_DIR 不存在——判据没有输入，宁红不静默'))
  process.exit(1)
}

/** 折叠根：core 的 `projection/`（PP-004 的棘轮面）+ 每个 crate 的 `src/projections/`。 */
function foldRoots() {
  const roots = [{ rel: PROJ_REL, abs: PROJ_DIR, core: true }]
  const cratesDir = path.join(ROOT, 'crates')
  let entries = []
  try {
    entries = fs.readdirSync(cratesDir, { withFileTypes: true })
  } catch {
    return roots
  }
  for (const ent of entries) {
    if (!ent.isDirectory()) continue
    const abs = path.join(cratesDir, ent.name, 'src', 'projections')
    const rel = relPosix(abs)
    if (rel !== PROJ_REL && fs.existsSync(abs)) roots.push({ rel, abs, core: false })
  }
  return roots
}

/** 一个折叠根扫出来的三样：目录里的生产文件、产投影的那些、以及构造器清单。 */
const scopes = foldRoots().map((root) => {
  const files = loadProduction(root.abs).filter((f) => path.dirname(f.rel) === root.rel)
  const producers = files.filter(
    (f) =>
      path.basename(f.rel) !== 'mod.rs' && f.text.some((l) => l.includes('Projection::new')),
  )
  return {
    ...root,
    files,
    producers,
    // 「同根兄弟」的范围：跨根导入不在本判据面里——core 的协议投影本来就是给各 crate 读的，
    // 判 L2 要判的是**同一层里视图叠视图**。
    producerNames: new Set(producers.map((f) => path.basename(f.rel, '.rs'))),
    ctors: producers.flatMap((f) =>
      [...f.text.join('\n').matchAll(/pub fn (\w+)\b[^;{}]{0,300}?->\s*Projection</g)].map((sig) => ({
        rel: f.rel,
        raw: f.raw,
        name: sig[1],
      })),
    ),
  }
})

const foldFiles = scopes.flatMap((s) => s.files)
const foldProducers = scopes.flatMap((s) => s.producers)
const foldRels = new Set(scopes.map((s) => s.rel))
const coreScope = scopes.find((s) => s.core)

// ── PP-001：产投影不导入同根产投影的兄弟 ───────────────────────────────
// 认 `use` 行里指向折叠目录的那一段：`use super::<name>[::item]`、
// `use super::{<a>, <b>}`、`use <…>::projection[s]::<name>`。
// **首段就是模块名**：`use super::fallback::declared_tier;` 与 `use super::fallback;`
// 是同一件事——只看整行结尾的形态会漏掉「按条目导入」这一半写法。
const USE_RE = /use\s+(?:super|[\w:]*\bprojections?)::(\{[^}]*\}|\w+)/
for (const s of scopes) {
  for (const f of s.producers) {
    for (const [i, line] of f.text.entries()) {
      const m = line.match(USE_RE)
      if (!m) continue
      const names = m[1].startsWith('{')
        ? m[1]
            .slice(1, -1)
            .split(',')
            .map((x) => x.trim().split(/\s+as\s+/)[0])
        : [m[1]]
      for (const name of names) {
        const mod = name.split('::')[0]
        if (s.producerNames.has(mod)) {
          fail(
            `PP-001 ${f.rel}:${i + 1} 导入同根产投影兄弟 \`${mod}\`——投影 A 不读投影 B（L2，DE-001 §13.2）；要共用的判据请放进不产投影的共享读侧（例：tiers.rs）`,
          )
        }
      }
    }
  }
}

// ── PP-002：折叠文件禁隐式全局依赖 ─────────────────────────────────────
for (const f of foldFiles) {
  const allowed = new Set()
  for (const line of f.raw.split('\n')) {
    const m = line.match(ALLOW_RE)
    if (m && ['PP-001', 'PP-002', 'PP-003'].includes(m[1])) allowed.add(m[1])
  }
  if (allowed.has('PP-002')) continue
  for (const [i, line] of f.text.entries()) {
    for (const token of IMPURITY_TOKENS) {
      if (line.includes(token)) {
        fail(
          `PP-002 ${f.rel}:${i + 1} 出现 \`${token}\`——折叠体内不得读时钟 / 随机数 / 全局可变状态（N1 的口径外漏洞）`,
        )
      }
    }
  }
}

// ── PP-003：构造器必须有折叠外的生产消费方（或显式豁免） ───────────────
const outside = loadProduction(path.join(ROOT, 'crates')).filter(
  (f) => !foldRels.has(path.dirname(f.rel)),
)
/** 构造器清单：PP-003 逐条判消费方，PP-004 数 core 那一根的条数——同一次扫描，两份判据同源。 */
const ctorRows = scopes.flatMap((s) => s.ctors)
for (const { rel, raw, name } of ctorRows) {
  const exempt = [...raw.split('\n')].some((line) => {
    const m = line.match(ALLOW_RE)
    return m && m[1] === name && m[2]
  })
  if (exempt) continue
  const consumed = outside.some(
    (g) =>
      // use 导入：`projection::{recall, ...}` / `projection::recall`
      new RegExp(`projection::(?:\\{[^}]*\\b${name}\\b|\\w*\\b${name}\\b)`).test(g.text) ||
      // 调用：词边界 `name(`
      new RegExp(`\\b${name}\\s*\\(`).test(g.text),
  )
  if (!consumed) {
    fail(
      `PP-003 ${rel} 构造器 \`${name}\` 在折叠目录之外没有生产消费方——死抽象（删掉，或写 projection-purity-allow 说明设计意图）`,
    )
  }
}

// ── PP-004：core 投影面只许降（DE-025 D-221）────────────────────────────
/**
 * 两格的**现值**（2026-10-28 下沉六条之后冻结）：
 * - `files`：`projection/` 下产 `Projection` 的文件数；
 * - `ctors`：其中 `pub fn X(..) -> Projection<` 的构造器条数。
 *
 * 放宽口就是改这一行的数字，而改数字要按 PP-005 挂一条能打开的决策。
 * ⚠️ 键序就是 `CORE_PROJECTION_BASELINE=<files>:<ctors>` 的位置口径，
 * 也必须是 `gate.d/35-baseline.mjs` 读的那两格——任何一边单改，这里读不出格 ⇒ 判红。
 */
const PROJ_BASELINE = process.env.CORE_PROJECTION_BASELINE
  ? (() => {
      const [files, ctors] = process.env.CORE_PROJECTION_BASELINE.split(':').map((n) => Number(n))
      return { files, ctors }
    })()
  : { files: 6, ctors: 6 }

const PROJ_CELL_LABEL = {
  files: 'core 产投影文件数',
  ctors: 'core 投影构造器条数',
}

const projValues = { files: coreScope.producers.length, ctors: coreScope.ctors.length }
for (const [key, cap] of Object.entries(PROJ_BASELINE)) {
  if (!Object.hasOwn(projValues, key)) {
    fail(
      `PP-004 基线里有一格 \`${key}\` 不是投影面量（只有 ${Object.keys(projValues).join(' / ')}）——判据读不出即失效，宁红不静默`,
    )
    continue
  }
  if (!Number.isFinite(cap)) {
    fail(`PP-004 基线格 \`${key}\` 不是数字（${cap}）——判据读不出即失效，宁红不静默`)
    continue
  }
  const now = projValues[key]
  if (now > cap) {
    fail(
      `PP-004 ${PROJ_CELL_LABEL[key]}变宽：实测 ${now} > 基线 ${cap} —— ` +
        `能力折叠住进 \`symbio_core/projection/\` 就等于「加一个能力要改架构模块」（DE-025 D-221）。` +
        '唯一合法的路径按序走：① 数它有几个 crate 在读——只有一个 owner 在读就**下沉给那个 owner**' +
        '（先例：checkpoint / fallback / slo / span / changes 归 session，consolidate 归 loop）；' +
        '② 别的插件想读 owner 的折叠，走 `symbio_plugin_api::views` 的按名字读口 + ' +
        '`PLUGIN.yml` 的 `shared_views`，不在 core 里再写一份；③ 三条路都不通才谈放宽：' +
        '先在 `docs/003-design/` 立一条 `### D-NNN` 逐条论证「必需」，再在本脚本基线旁写 ' +
        `projection-purity-allow ${key}: D-NNN <理由>`,
    )
  }
}

// ── PP-005：放宽注释必须挂得住一条真实决策 ─────────────────────────────
/**
 * 读本脚本**自己**源码里的放宽痕迹（与 CS-002 同一分工：`35-baseline` 判「数字有没有
 * 比上一版更松」，这里判「这条豁免够不够格」）。夹具仓库里可能没有本脚本 ⇒ 读不到跳过。
 */
const PROJ_WAIVER_RE = /^ {0,2}\/\/\s*projection-purity-allow\s+([A-Za-z0-9_]+)\s*:\s*(\S[^\n]*)$/gm
const PROJ_WAIVABLE_KEYS = Object.keys(PROJ_BASELINE)
const selfPath = path.join(ROOT, 'scripts', 'projection-purity.mjs')
if (fs.existsSync(selfPath)) {
  const selfText = fs.readFileSync(selfPath, 'utf8').replace(/\r\n/g, '\n')
  for (const m of selfText.matchAll(PROJ_WAIVER_RE)) {
    const [, key, rawReason] = m
    const reason = rawReason.trim()
    if (!PROJ_WAIVABLE_KEYS.includes(key)) {
      fail(
        `PP-005 \`projection-purity-allow ${key}\` 放宽的不是投影面的格（只有 ${PROJ_WAIVABLE_KEYS.join(' / ')}）`,
      )
      continue
    }
    const refs = [...reason.matchAll(/\bD-(\d{3}a?)\b/g)].map((r) => r[1])
    if (refs.length === 0) {
      fail(
        `PP-005 放宽 \`${key}\` 的理由里没有决策编号（\`${key}: ${reason}\`）—— 动棘轮数字必须挂一条决策，` +
          '「批次需要 / 顺手 / 待补 / 见对话」都不算',
      )
      continue
    }
    for (const id of refs) {
      if (!hasDecisionAnchor(id)) {
        fail(
          `PP-005 放宽 \`${key}\` 引用了 \`D-${id}\`，但设计文档里没有任何 \`### D-${id}\` 条目 —— ` +
            '指向不存在的决策的豁免不算豁免',
        )
      }
    }
  }
}

const underBaseline = Object.keys(projValues).filter((k) => projValues[k] < PROJ_BASELINE[k])

// ── 输出 ───────────────────────────────────────────────────────────────
if (errors.length > 0) {
  for (const e of errors) console.log(red('[ERROR]'), e)
  console.log(dim(`\n共 ${errors.length} 处违规（判据见 docs/003-design/001-核心架构.md）`))
  process.exit(1)
}

console.log(
  green('[OK]   '),
  dim(
    `PP-001 ${scopes.length} 个折叠根、${foldProducers.length} 个产投影文件零兄弟互导；` +
      `PP-002 ${foldFiles.length} 个折叠生产文件零隐式全局依赖；` +
      `PP-003 构造器全部有折叠外生产消费方；` +
      `PP-004 core ${Object.keys(projValues)
        .map((k) => `${k} ${projValues[k]}/${PROJ_BASELINE[k]}`)
        .join(' · ')}`,
  ),
)
if (underBaseline.length > 0) {
  console.log(
    yellow(
      `：投影面已收到基线下方（${underBaseline
        .map((k) => `${k} ${projValues[k]}<${PROJ_BASELINE[k]}`)
        .join(' · ')}），` +
        '请同步本脚本的 PROJ_BASELINE——留着旧数字等于把余量白留给下一次下沉',
    ),
  )
}
