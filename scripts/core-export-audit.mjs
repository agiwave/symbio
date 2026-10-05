#!/usr/bin/env node
/**
 * core-export-audit — `symbio_core` **架构出口的有效性**（判定型）
 *
 * ## 它判什么
 *
 * 「symbio_core 的代码全放子目录里，但子目录**不许对外 `pub` 输出**，必须在**根目录
 * 显式输出**；根输出的每一件东西必须被**两个以上模块**使用——没人用的不许存在，
 * 只有一个模块用的必须下沉到那个模块里（只有一个消费方的不算架构元素）。」
 *
 * 三条规则，一一对应：
 *
 * | 规则 | 判什么 | 为什么必须判 |
 * |---|---|---|
 * | C-001 | 根 `mod.rs` 不得有 `pub mod <域>;` | 域目录一旦 `pub mod`，整个子树就是公共面：`core-surface` 枚举的「公开面 = 根重导出」当场失真，外部还能直接深引绕过根 |
 * | C-002 | core 外的**代码**不得出现 `symbio_core::<域>`（含裸 `use crate::symbio_core::<域>;`） | 深引 = 消费方绕过根出口：符号从根导出移除后深引点照样编译通过，公开面变成两套。裸模块导入此前**没有**任何审计在拦（旧 E-010 的正则要求尾 `::`） |
 * | C-003 | 根导出符号的消费方数必须 ≥2 | ADR-023 的判据（依赖方数量）此前只有**报告型** `core-surface-audit` 在数、判定权在人手里，于是长期停在 0 消费方 139 / 单消费方 64 上不动 |
 *
 * ## 与既有审计的分工（别在两处写两套判据）
 *
 * - **`core-surface-audit.mjs`**（报告型）：同一份公开面（`collectCoreSurface`）与
 *   同一份消费方计数（`collectConsumers`）。报告负责**列出候选**并提示 README §4 四问；
 *   本脚本负责**判定**：豁免账本（必须写非空理由）+ 棘轮基线（只许降）。
 * - **原 E-010**（`plugin-entry-audit.mjs` 的「不得深引 `symbio_core::<域>::`」）：**已退役，
 *   由 C-002 取代**——深引是**内核出口**的事、不是插件门面的事，两处各判一遍必然分叉出两套
 *   域清单与豁免通道。C-002 是它的全量版：域目录动态取自 core（含 `schemas`，不再豁免），
 *   并额外拦裸 `use crate::symbio_core::<域>;`（原正则要求尾 `::`，漏了这种形态）。
 * - **`dead-code-audit.mjs`**：判「全仓零引用的 `pub` 声明」（core 自身也算引用）。
 *   C-003 判的是「名字没跨出 core 的根导出」——**core 内部在用、对外零消费**的符号归这里：
 *   它不该占着根出口（收窄 `pub(crate)` 或下沉）。互补不重叠。
 *
 * ## 存量怎么办：棘轮基线
 *
 * 今天全仓 0 消费方 140 个、单消费方 67 个。一次性下沉 200+ 符号等于把 core 翻一遍，
 * 不可独立回退；故按本仓既有棘轮口径（`_shared.mjs` 的 `BASELINE.rustTests`、
 * `test-layout-audit` 基线）：**计数只许降，超了就红**。降到基线以下时脚本会提示同步
 * `BASELINE`。
 *
 * ## 豁免（对应 `symbio_core/README.md` §4 四问）
 *
 * 单消费方不等于该下沉——宿主接缝（`ExecTranscriptWriter`）、跨 crate 契约、trait 形参
 * 类型都只有一个消费方而必须留 core。把符号加进 `WAIVERS` 并写**非空理由**即豁免；
 * 空理由、以及**失效豁免**（符号已不在公开面 / 已 ≥2 消费方）都判红——豁免要随代码一起退役。
 *
 * C-001 / C-002 的行内豁免：同一行（或其上 3 行内）的注释写 `core-export-allow: <理由>`。
 *
 * ## 用法
 *   node scripts/core-export-audit.mjs            # 判定
 *   node scripts/core-export-audit.mjs --verbose  # 附存量清单（0 / 1 消费方逐条）
 *   node scripts/core-export-audit.mjs --root=<dir>  # 指向别处的仓库（测试用）
 * 退出码：0 = 通过，1 = 违规（源头可改）。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'
import { collectCoreSurface, collectConsumers, isSelfReference } from './core-surface.mjs'
import { blankComments } from './rust-scan.mjs'

const argv = process.argv.slice(2)
const VERBOSE = argv.includes('--verbose')
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const CORE_REL = 'symbio/src/symbio_core'
/** 深引扫描范围：与 E-010 的 CODE_ROOTS 同源（壳侧是独立 crate，但同属消费方） */
const CODE_ROOTS = ['symbio/src', 'cli/src', 'tauri/src', 'tauri/src-tauri/src']

/**
 * C-003 豁免账本：符号 → **非空**理由（README §4 四问）。
 * 空字符串不算豁免；符号离开公开面或消费方变 ≥2 ⇒ 本条失效（判红，要求同步删除）。
 *
 * `CORE_EXPORT_WAIVERS`（JSON 对象）/ `CORE_EXPORT_BASELINE`（`zero:one`）是回归测试的
 * 注入通道——真仓门禁不设这两个变量（设了就等于把棘轮调到零，见 `.test.mjs`）。
 */
const WAIVERS = process.env.CORE_EXPORT_WAIVERS
  ? JSON.parse(process.env.CORE_EXPORT_WAIVERS)
  : {
      // 治理矩阵与可见域参数（04 §3.1 批⑥ 接线）。core 外唯一消费方是 `plugins/session`
      // 的两道闸，但它们**不能**下沉：core 内 `authz` / `governance` 自己也在用（core 不
      // 依赖插件），而插件又够不着 `governance` 域目录的深引（C-002）⇒ 必须留根出口。
      // 属 README §4 四问的「接缝」而非「单消费方该下沉」的情形（`authz` 迁入 core 后
      // 它们掉了原本由 `crate::authz` 充当的第 2 个消费方，见 `symbio_core/mod.rs` 注）。
      PermissionMatrix:
        '插件-facing 治理契约类型：E-009/C-002 禁止插件深引 core 域目录，必须留根出口；core 内 authz 与 governance 亦用，无法下沉到唯一消费方',
      VisScope:
        '同上：读侧可见域参数类型，插件经 session/stats 读闸传入，core 内 authz 亦用，无法下沉',
    }

/**
 * 棘轮基线：0 消费方 / 单消费方的存量上限（自引用不计）。
 * 只许降——超了说明新符号进了根出口却没跨出 core，或某个消费方被删掉了。
 *
 * `0` 是规则 2 首批整改的结果：140 个「名字没跨出 core」的符号已从根出口收窄
 * （定义留在原域 `pub`，core 内走域内路径），只被测试/无人使用的存量另由
 * `dead-code-audit` R-002 逐项承认。`61` 是宏展开消费计入口径后的单消费方存量——
 * 授权表迁入 core（`authz` 不再充当第 2 个消费方）后从 62 降为 61，掉出的
 * `PermissionMatrix` / `VisScope` 两个走 `WAIVERS`（见上）。
 */
const BASELINE = process.env.CORE_EXPORT_BASELINE
  ? (() => {
      const [zero, one] = process.env.CORE_EXPORT_BASELINE.split(':').map((n) => Number(n))
      return { zero, one }
    })()
  : { zero: 0, one: 61 }

const errors = []
const report = (rule, msg, loc = '') => errors.push({ rule, msg, loc })

/** 行内豁免：本行或其上 3 行内的注释写 `core-export-allow: <非空理由>`（看**原始行**，注释在 blank 后就没了） */
function waived(rawLines, i) {
  for (let j = Math.max(0, i - 3); j <= i; j++) {
    const m = rawLines[j].match(/core-export-allow:\s*(.*)$/)
    if (m) return m[1].trim().length > 0
  }
  return false
}

// ==================== 读仓库 ====================

let surface
try {
  surface = collectCoreSurface(ROOT)
} catch (e) {
  console.error(red(`✗ ${e.message}`))
  process.exit(1)
}
const { coreDir, domains, symbols, modules } = surface

function* walk(dir) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of entries) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) yield* walk(p)
    else yield p
  }
}

const relOf = (abs) => path.relative(ROOT, abs).split(path.sep).join('/')

// ==================== C-001 根出口唯一 ====================

const rootRaw = fs.readFileSync(path.join(coreDir, 'mod.rs'), 'utf8')
const rootLines = rootRaw.split(/\r?\n/)
rootLines.forEach((line, i) => {
  const m = line.match(/^\s*pub\s+mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*[;{]/)
  if (!m) return
  if (waived(rootLines, i)) return
  report(
    'C-001',
    `根 mod.rs 出现 \`pub mod ${m[1]};\` —— 子目录不得对外开模块门；改成 \`mod ${m[1]};\`，` +
      `把需要出现在公共面上的东西在根 \`pub use\` 显式列出`,
    `${CORE_REL}/mod.rs:${i + 1}`,
  )
})

// ==================== C-002 core 外无深引 ====================

const deepRefRe = /\bsymbio_core\s*::\s*([a-z_][a-z0-9_]*)\b/
for (const r of CODE_ROOTS) {
  for (const file of walk(path.join(ROOT, r))) {
    if (!file.endsWith('.rs')) continue
    const abs = path.resolve(file)
    if (abs.startsWith(path.resolve(coreDir) + path.sep)) continue // core 内部深引是模块内部的事
    const raw = fs.readFileSync(file, 'utf8')
    const rawLines = raw.split(/\r?\n/)
    const codeLines = blankComments(raw) // 与原始行一一对应（豁免注释看原行）
    codeLines.forEach((line, i) => {
      const m = line.match(deepRefRe)
      if (!m || !domains.has(m[1])) return
      if (waived(rawLines, i)) return
      report(
        'C-002',
        `\`symbio_core::${m[1]}::…\` 深引 core 子目录（或裸 \`use …::symbio_core::${m[1]};\`）` +
          ` —— 根平铺是唯一公共面，写 \`symbio_core::<符号>\``,
        `${relOf(file)}:${i + 1}`,
      )
    })
  }
}

// ==================== C-003 双消费方 ====================

const consumers = collectConsumers(ROOT, symbols, CORE_REL)
const byName = (a, b) => a.domain.localeCompare(b.domain) || a.name.localeCompare(b.name)
const rows = [...symbols.keys()]
  .map((name) => ({
    name,
    domain: symbols.get(name),
    units: [...(consumers.get(name) ?? [])].sort(),
    isModule: modules.has(name),
  }))
  .sort(byName)

const waivedNow = new Set()
for (const [name, reason] of Object.entries(WAIVERS)) {
  if (!String(reason ?? '').trim()) {
    report('C-003', `豁免账本里 \`${name}\` 的理由为空 —— 空理由不算豁免`)
    continue
  }
  const row = rows.find((r) => r.name === name)
  if (!row) {
    report('C-003', `豁免账本里的 \`${name}\` 已不在公开面 —— 该条豁免已失效，请删除`)
    continue
  }
  if (row.units.length >= 2) {
    report('C-003', `豁免 \`${name}\` 已失效（现有 ${row.units.length} 个消费方）—— 请从账本删除`)
    continue
  }
  waivedNow.add(name)
}

const selfRef = rows.filter((r) => isSelfReference(r))
const zero = rows.filter((r) => r.units.length === 0 && !waivedNow.has(r.name))
const one = rows.filter(
  (r) => r.units.length === 1 && !isSelfReference(r) && !waivedNow.has(r.name),
)

if (zero.length > BASELINE.zero) {
  report(
    'C-003',
    `0 消费方 ${zero.length} 个 > 基线 ${BASELINE.zero} —— 名字没跨出 core 的符号不许占根出口` +
      `（收窄为 \`pub(crate)\` 或删除）。逐条：--verbose`,
  )
}
if (one.length > BASELINE.one) {
  report(
    'C-003',
    `单消费方 ${one.length} 个 > 基线 ${BASELINE.one} —— 只有一个模块消费的不算架构元素，` +
      `下沉到该模块；确属 README §4 四问的接缝，写进 WAIVERS 并给理由。逐条：--verbose`,
  )
}

// ==================== 输出 ====================

const ruleLine = (ok, rule, desc) =>
  console.log(`${ok ? green('✓') : red('✗')} ${rule}  ${dim(desc)}`)

if (errors.length === 0) {
  console.log('\n== core-export-audit ==')
  ruleLine(true, 'C-001', '根出口唯一：根 mod.rs 无 `pub mod <域>;`')
  ruleLine(true, 'C-002', `core 外无深引（${domains.size} 个域目录动态取自 core）`)
  ruleLine(
    true,
    'C-003',
    `双消费方棘轮：0 消费方 ${zero.length}/${BASELINE.zero} · 单消费方 ${one.length}/${BASELINE.one}` +
      ` · 自引用 ${selfRef.length} · ≥2 ${rows.length - zero.length - one.length - selfRef.length}`,
  )
  if (zero.length < BASELINE.zero || one.length < BASELINE.one) {
    console.log(
      yellow('提示') +
        `：已降到基线下方（${zero.length}/${one.length}），请同步本脚本的 ` +
        `BASELINE = { zero: ${zero.length}, one: ${one.length} }`,
    )
  }
  if (VERBOSE) {
    for (const r of zero) console.log(`  [ 0] ${(r.domain ?? '?').padEnd(12)} ${r.name}`)
    for (const r of one) console.log(`  [ 1] ${(r.domain ?? '?').padEnd(12)} ${r.name} -> ${r.units.join(', ')}`)
  }
  process.exit(0)
}

console.error(`\n== core-export-audit：${errors.length} 处违规 ==`)
for (const e of errors) {
  console.error(`  ${red(e.rule)}  ${e.msg}`)
  if (e.loc) console.error(dim(`        ${e.loc}`))
}
console.error(
  `\n${yellow('提示')}：C-003 存量走棘轮（只许降），处置判据见 ` +
    `${green('symbio_core/README.md §4 四问')}；逐条清单跑 ` +
    `node scripts/core-export-audit.mjs --verbose`,
)
if (VERBOSE) {
  for (const r of zero) console.error(`  [ 0] ${(r.domain ?? '?').padEnd(12)} ${r.name}`)
  for (const r of one) console.error(`  [ 1] ${(r.domain ?? '?').padEnd(12)} ${r.name} -> ${r.units.join(', ')}`)
}
process.exit(1)
