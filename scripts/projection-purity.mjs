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
 * | PP-001 | 产投影文件不得 `use` 导入**产投影的兄弟模块**（不含 `Projection::new` 的共享读侧——`mutations` / `rebuild` / `turns` / `prompt_fidelity` / `consolidate` / `recall_score`——谁都可以导） | 投影 A 读投影 B 的输出（L2，DE-001 §13.2）：视图叠视图，外壳 `degraded` / 预算语义在层间静默丢失 |
 * | PP-002 | `projection/*.rs` 生产文件禁 `SystemTime::now` / `rand::` / `static mut` / `std::time::Instant` | 隐式全局依赖：时钟、随机数、全局可变状态——确定性（N1）的口径外漏洞 |
 * | PP-003 | 每个 `pub fn X(..) -> Projection<` 构造器必须有 `projection/` 之外的**生产消费方**，或同文件里一行 `// projection-purity-allow <X>: 非空理由` | 死构造器：抽了纯函数视图却没有调用方——「为将来准备」的投机抽象（本仓只认消费方定的契约） |
 *
 * 「产投影文件」的判定：`symbio_core/projection/` 下（排除 `*.test.rs`）含
 * `Projection::new` 的文件。`mod.rs` 是框架本身（`new` 的定义处），不算产投影。
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
// @codes PP-001 PP-002 PP-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'
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

const prodFiles = loadProduction(PROJ_DIR)
const projFiles = prodFiles.filter((f) => path.dirname(path.join(ROOT, f.rel)) === PROJ_DIR)

/** 产投影文件：含 `Projection::new`（mod.rs 是框架定义处，天然含它——按路径排除）。 */
const producers = projFiles.filter(
  (f) => path.basename(f.rel) !== 'mod.rs' && f.text.some((l) => l.includes('Projection::new')),
)
const producerNames = new Set(producers.map((f) => path.basename(f.rel, '.rs')))

// ── PP-001：产投影不导入产投影兄弟 ─────────────────────────────────────
// 只认 `use` 行的两种形态：`use super::<name>` 与 `use crate::symbio_core::projection::<name>`。
for (const f of producers) {
  for (const [i, line] of f.text.entries()) {
    const m = line.match(
      /use\s+(?:super|crate::symbio_core::projection)::(\w+)(::\{([^}]*)\})?\s*;/,
    )
    if (!m) continue
    const head = m[1]
    const items = m[3] ? m[3].split(',').map((s) => s.trim().split(/\s+as\s+/)[0]) : []
    for (const name of [head, ...items]) {
      const mod = name.split('::')[0]
      if (producerNames.has(mod)) {
        fail(
          `PP-001 ${f.rel}:${i + 1} 导入产投影兄弟 \`${mod}\`——投影 A 不读投影 B（L2，DE-001 §13.2）`,
        )
      }
    }
  }
}

// ── PP-002：projection 生产文件禁隐式全局依赖 ──────────────────────────
for (const f of projFiles) {
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
          `PP-002 ${f.rel}:${i + 1} 出现 \`${token}\`——投影体内不得读时钟 / 随机数 / 全局可变状态（N1 的口径外漏洞）`,
        )
      }
    }
  }
}

// ── PP-003：构造器必须有投影外的生产消费方（或显式豁免） ───────────────
const outside = loadProduction(path.join(ROOT, 'crates')).filter(
  (f) => !f.rel.startsWith(PROJ_REL + '/'),
)
for (const f of producers) {
  // 构造器：`pub fn NAME` 与 `-> Projection<` 出现在同一签名里（跨行宽容）。
  const sigs = [...f.text.join('\n').matchAll(/pub fn (\w+)\b[^;{}]{0,300}?->\s*Projection</g)]
  for (const sig of sigs) {
    const name = sig[1]
    const exempt = [...f.raw.split('\n')].some((line) => {
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
        `PP-003 ${f.rel} 构造器 \`${name}\` 在 projection/ 之外没有生产消费方——死抽象（删掉，或写 projection-purity-allow 说明设计意图）`,
      )
    }
  }
}

// ── 输出 ───────────────────────────────────────────────────────────────
if (errors.length > 0) {
  for (const e of errors) console.log(red('[ERROR]'), e)
  console.log(dim(`\n共 ${errors.length} 处违规（判据见 docs/003-design/001-核心架构.md）`))
  process.exit(1)
}

console.log(
  green('[OK]   '),
  dim(
    `PP-001 产投影 ${producers.length} 文件零兄弟导入；` +
      `PP-002 全目录 ${projFiles.length} 生产文件零隐式全局依赖；` +
      `PP-003 构造器全部有投影外生产消费方`,
  ),
)
