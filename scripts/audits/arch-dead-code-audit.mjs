#!/usr/bin/env node
/**
 * arch-dead-code-audit — 架构树的「公开但零生产调用」判据（AD-001…004）
 *
 * ## 它治的病
 *
 * 2026-11-09 的复盘发现：`capability_port_check`（端口装配判重）与 `fit_sections`
 * （上下文段裁剪）**不是死代码，是判据被删丢了**——`16ad2f4d` / `60cedd50` 删旧编排时
 * 把调用点一起删了，函数还留在 `plugin-api` 里。两件事同时成立：判据静默失效，
 * 而所有门禁照旧全绿——**没人调用就没人报错**，`cargo` 也不报（它们是 `pub`）。
 *
 * 这正是本仓最贵的一类腐烂：不是「代码写错」，是「判据不再跑」。只要函数形状还在、
 * 符号还在，`doc-symbol-audit` 的指认照样成立、架构文档照样引它；唯一的读数
 * **「生产里有没有人调它」**此前没有任何脚本去数。
 *
 * ## 口径（三条，写死在这里）
 *
 * 1. **扫描面**：`ARCH_TREES`（`kernel` / `sdk` / `plugin-api`）的**生产** `.rs` 里
 *    `pub fn`（含关联函数、`async` / `const` / `unsafe` / `extern` 形态）。trait 里
 *    无 `pub` 的成员不算——它们由 trait 调用，是契约实现，不是可被遗忘的判据。
 * 2. **生产 = 非测试**：`*.test.rs` / `tests.rs` / `#[cfg(test)] mod` 一律剔除
 *    （`isTestFile` 与 `blankTestModules` 是共用实现）。测试里的调用**不算**生产调用——
 *    否则「写一条测试」就能给一个没人用的函数续命，而那正是要防的形态。
 * 3. **调用 = 名字出现在生产语料里**（跨 crate 与经 `pub use` 转出后的调用都算）。
 *    `use` / `mod` 行不计——转出本身不是调用，它只是把路径挪了个地方；数它会让
 *    「`pub use` 一条没人用的符号」看起来活着。`#[allow(dead_code)]` 也不是豁免。
 *
 * 同名多定义的符号（`new` / `default` 这类构造器）**不判**：名字级计数分不清是哪一处的
 * 调用，硬判会把「另一处的同名调用」算成这一处的存活。判据宁可漏判，也不报假红——
 * 假红最后一定会被豁免喂死。
 *
 * ## 基线
 *
 * `scripts/baselines/arch-dead-code.json` 逐条登记 `symbol` / `path` / `why` / `evidence`。
 * `why` 是哪一类合法零调用：**契约实现**（留给插件实现/宿主调用）/ **由 trait 调用** /
 * **生成物入口** / **待接线**（有既有调用点、正在恢复）。`evidence` 必须能打开：
 * 一条 `### D-NNN`，或一条**存在的**仓内路径（模块契约文档）——「为什么它可以
 * 零调用」和「这条判断由谁裁决 / 登记」是两件事。
 *
 * 基线**只许降**：新出现的零调用符号即红；删掉/接线的符号若不同批下调基线也红
 * （`AD-002`），否则「删一条、再挂一条」能把棘轮抹平。
 *
 * ## 历史回放（`--at=<rev>`）
 *
 * 本判据的验收方式不是「今天是对的」，而是「能不能指出它从哪一刻开始零调用」：
 * 在同一条提交的前后两版源码上各跑一次，符号集合的差集就是失去的调用点。该模式
 * **不判基线**（历史版本没有这份基线），只输出读数；范围仍必须读得出（`AD-004` 照样生效）。
 *
 * ## 失效形态（回归测试钉的是这几条）
 *
 * - 把测试代码算进生产语料 ⇒ 写一条测试即可续命（`AD-001` 永不命中）；
 * - 把 `use` / `pub use` 行算成调用 ⇒ 转出即存活，判据对「没人用的门面」失效；
 * - 同名符号按名字判定 ⇒ 构造器假绿一片，豁免写满之后没人再看；
 * - 基线只判「新增」不判「删除」 ⇒ 删了符号基线留余量，下一次新增正好填进去（`AD-002`）；
 * - `why` 空着、`evidence` 指一个不存在的 D 号 ⇒ 基线退化成一张「已批准」的空表（`AD-003`）；
 * - 扫描面读不出（目录不在 / 0 个符号）却按「0 处违规」通过（`AD-004`）。
 *
 * 用法：
 * ```bash
 * node scripts/audits/arch-dead-code-audit.mjs            # 判当前树（门禁用）
 * node scripts/audits/arch-dead-code-audit.mjs --report   # 只列零调用符号（校准基线用）
 * node scripts/audits/arch-dead-code-audit.mjs --at=16ad2f4d^   # 历史回放（不判基线）
 * node scripts/audits/arch-dead-code-audit.mjs --root=<dir>     # 回归测试夹具
 * ```
 * 退出码：0 = 通过；1 = 有违规；2 = 范围读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns AD 架构树零调用判据
// @codes AD-001 AD-002 AD-003 AD-004
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, dim } from '../color.mjs'
import { walk, isTestFile } from '../fs-walk.mjs'
import { blankComments, blankTestModules } from '../rust-scan.mjs'
import { hasDecisionAnchor } from '../decision-anchors.mjs'
import { gitRaw } from '../gate.d/_shared.mjs'

const git = (args) => gitRaw(args, { cwd: ROOT, maxBuffer: 256 * 1024 * 1024 })

const argv = process.argv.slice(2)
const rootArg = argv.find((a) => a.startsWith('--root='))
const atArg = argv.find((a) => a.startsWith('--at='))
const REPORT = argv.includes('--report')
const REPLAY = atArg ? atArg.slice('--at='.length) : null
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')

export const BASELINE_REL = 'scripts/baselines/arch-dead-code.json'

/**
 * 扫描面：架构树（与 `capability-migration-audit` 的 `ARCH_TREES` 同一份口径）。
 *
 * ⚠️ 两处必须同时改：这份列表与 `capability-migration-audit.ARCH_TREES` 是同一件事的
 * 两个读者（同一个常量被两个脚本各抄一遍，`D-372` 治过一次「名单抄本分叉」）。
 * `D-486` 把 `symbio-contract` 立为冻结 ABI 层之后，它一度只进了受保护树与 CS-009 的
 * 数据面，**没进本扫描面**——契约层的零调用 `pub fn` 因此是 AD-001…004 的盲区。
 */
export const ARCH_TREES = [
  'crates/symbio-sdk',
  'crates/symbio-kernel',
  'crates/symbio-contract',
  'crates/symbio-plugin-api',
]

/** 调用语料：整个 `crates` 树 + Tauri 宿主的 Rust 源码（插件与宿主都可能调它们）。 */
export const CALL_TREES = ['crates', 'tauri/src-tauri/src']

/** `pub fn` 的声明面。`pub(crate)` 不算公开面（`pub` 后必须紧跟空白）。 */
const FN_DECL_RE =
  /^\s*pub\s+(?:async\s+)?(?:const\s+|unsafe\s+|extern\s+"[^"]*"\s+)*fn\s+([A-Za-z_][A-Za-z0-9_]*)/

/** `use` 行（含 `pub use`）——转出不是调用，见模块文档口径 3。 */
const USE_START_RE = /^\s*(?:pub(?:\s*\(\s*[^)]*\s*\))?\s+)?use\s/
/** `mod` 声明行——接线不是调用。 */
const MOD_DECL_RE = /^\s*(?:pub(?:\s*\(\s*[^)]*\s*\))?\s+)?mod\s+[A-Za-z_][A-Za-z0-9_]*\s*;\s*$/

/** 把注释抹成空白、剔除 `#[cfg(test)]` 模块，**保留行结构**（行号可回指原文件）。 */
export function blankSource(text) {
  return blankTestModules(blankComments(text).join('\n'))
}

/**
 * 生产语料：在 [`blankSource`] 之上**删掉 `use` / `mod` 行**（多行 `use` 整块删）。
 * 删行不影响「名字有没有出现」这一判定（本判据不报行号）。
 */
export function callCorpus(text) {
  const lines = blankSource(text).split('\n')
  const out = []
  let inUse = false
  for (const line of lines) {
    if (inUse) {
      if (line.includes(';')) inUse = false
      continue
    }
    if (USE_START_RE.test(line)) {
      if (!line.includes(';')) inUse = true
      continue
    }
    if (MOD_DECL_RE.test(line)) continue
    out.push(line)
  }
  return out.join('\n')
}

/** 从一份源码提取 `pub fn` 声明（带行号）。 */
export function extractSymbols(text, rel) {
  const out = []
  const lines = blankSource(text).split('\n')
  for (let i = 0; i < lines.length; i++) {
    const m = FN_DECL_RE.exec(lines[i])
    if (m) out.push({ symbol: m[1], path: rel, line: i + 1 })
  }
  return out
}

/** 名字整词出现次数。 */
function countName(corpus, name) {
  const re = new RegExp(`\\b${name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`, 'g')
  return (corpus.match(re) ?? []).length
}

/**
 * 判定：单定义符号在语料里**除了自己那一行声明**之外还有没有出现。
 *
 * @param {{symbol: string, path: string, line: number}[]} symbols
 * @param {Map<string, string>} corpusByRel  每个生产文件的语料（已剥注释/测试/use）
 */
export function zeroCallSymbols(symbols, corpusByRel) {
  const byName = new Map()
  for (const s of symbols) {
    if (!byName.has(s.symbol)) byName.set(s.symbol, [])
    byName.get(s.symbol).push(s)
  }
  const corpus = [...corpusByRel.values()].join('\n')
  const dead = []
  for (const [name, defs] of byName) {
    if (defs.length !== 1) continue // 同名多定义：名字级计数分不清，宁可漏判
    const total = countName(corpus, name)
    // 声明行本身在语料里出现 1 次（定义文件也是生产文件）；其余出现即调用/引用。
    if (total <= 1) dead.push(defs[0])
  }
  return dead
}

function readTreeFromDisk(root) {
  // `ARCH_TREES` 是 `crates` 的子路径，两次遍历会重复读同一文件——按 rel 去重，
  // 否则同一符号被数成两个定义，`zeroCallSymbols` 会把它当「同名多定义」跳过（实测踩坑）。
  const files = new Map()
  for (const tree of [...ARCH_TREES, ...CALL_TREES]) {
    const dir = path.join(root, tree)
    if (!fs.existsSync(dir)) continue
    for (const abs of walk(dir)) {
      if (!abs.endsWith('.rs') || isTestFile(abs)) continue
      const rel = path.relative(root, abs).split(path.sep).join('/')
      if (!files.has(rel)) files.set(rel, fs.readFileSync(abs, 'utf8'))
    }
  }
  return [...files].map(([rel, text]) => ({ rel, text }))
}

function readTreeFromGit(rev) {
  const lr = git(['ls-tree', '-r', '--name-only', rev, '--', ...ARCH_TREES, ...CALL_TREES])
  if (lr.status !== 0) return { error: `git ls-tree ${rev} 失败：${(lr.stderr || '').trim()}` }
  const paths = lr.stdout
    .split('\n')
    .map((s) => s.trim())
    .filter((p) => p.endsWith('.rs'))
  const files = []
  const seen = new Set()
  for (const rel of paths) {
    if (seen.has(rel)) continue
    seen.add(rel)
    const base = rel.split('/').pop()
    if (base.endsWith('.test.rs') || base === 'tests.rs') continue
    const r = git(['show', `${rev}:${rel}`])
    if (r.status !== 0) return { error: `git show ${rev}:${rel} 失败：${(r.stderr || '').trim()}` }
    files.push({ rel, text: r.stdout })
  }
  return { files }
}

/** 扫描面读不出 ⇒ `AD-004`（退 2）；0 个符号也绿是这条判据最坏的失效形态。 */
export function surfaceErrors(files) {
  const errors = []
  const corpus = files.filter((f) => f.rel.startsWith('crates/'))
  if (corpus.length < 50) {
    errors.push({ code: 'AD-004', message: `生产语料只有 ${corpus.length} 个 .rs（< 50）—— 扫描面读不出` })
  }
  const symbols = []
  for (const tree of ARCH_TREES) {
    const inTree = files.filter((f) => f.rel.startsWith(`${tree}/`))
    if (inTree.length === 0) {
      errors.push({ code: 'AD-004', message: `架构树 ${tree} 下没有生产 .rs —— 扫描面读不出` })
      continue
    }
    const found = inTree.flatMap((f) => extractSymbols(f.text, f.rel))
    if (found.length === 0) {
      errors.push({ code: 'AD-004', message: `架构树 ${tree} 下没有任何 pub fn —— 扫描面读不出` })
    }
    symbols.push(...found)
  }
  return { errors, symbols }
}

/** 读基线；缺失 / 坏 JSON / 缺 `symbols` 数组都返回 `error`（调用方退 2）。 */
export function readBaseline(root = ROOT) {
  const file = path.join(root, BASELINE_REL)
  if (!fs.existsSync(file)) return { error: `基线缺失：${BASELINE_REL}` }
  let obj
  try {
    obj = JSON.parse(fs.readFileSync(file, 'utf8'))
  } catch (e) {
    return { error: `基线不是合法 JSON：${e.message}` }
  }
  if (!Array.isArray(obj?.symbols)) return { error: `基线缺少 symbols 数组：${BASELINE_REL}` }
  return { note: obj.note ?? '', symbols: obj.symbols }
}

/**
 * 证据可打开：`D-NNN` 决策号，或一条**存在的**仓内路径（模块文档 / 契约文档）。
 *
 * 为什么允许路径：这一面的「为什么可以零调用」大多是**类型自带的契约**（构造器 /
 * 查询口），它由模块文档登记，而不是某一条 D 号裁决。要求每条都挂 D 号只会逼人
 * 给同一件事写七十份理由；而要求「能打开」仍然把「写个编号占位」挡在外面。
 */
export function evidenceErrors(root, evidence) {
  if (typeof evidence !== 'string' || evidence.trim() === '') {
    return ['缺少 evidence（D-NNN 或存在的仓内路径）']
  }
  const ev = evidence.trim()
  if (/^D-\d{3}$/.test(ev)) {
    return hasDecisionAnchor(ev.slice(2), root) ? [] : [`挂的决策 ${ev} 没有「### ${ev}」锚点 ⇒ 证据要能打开`]
  }
  return fs.existsSync(path.join(root, ev)) ? [] : [`证据路径 ${ev} 不存在 ⇒ 证据要能打开`]
}

/** 基线条目的形状与署名（`AD-003`）。决策号存在性走 `decision-anchors`（唯一扫描实现）。 */
export function checkBaseline(baseline, root = ROOT) {
  const errors = []
  const seen = new Set()
  for (const [i, e] of baseline.symbols.entries()) {
    const where = `${BASELINE_REL}#symbols[${i}]`
    if (!e || typeof e !== 'object' || Array.isArray(e)) {
      errors.push({ code: 'AD-003', message: `${where} 不是对象` })
      continue
    }
    if (typeof e.symbol !== 'string' || e.symbol.trim() === '') {
      errors.push({ code: 'AD-003', message: `${where} 缺少 symbol` })
      continue
    }
    const key = `${e.symbol}@${e.path ?? ''}`
    if (seen.has(key)) errors.push({ code: 'AD-003', message: `${where} 重复登记 ${key}` })
    seen.add(key)
    if (typeof e.path !== 'string' || e.path.trim() === '') {
      errors.push({ code: 'AD-003', message: `${where}（${e.symbol}）缺少 path` })
    }
    if (typeof e.why !== 'string' || e.why.trim() === '') {
      errors.push({ code: 'AD-003', message: `${where}（${e.symbol}）why 为空 —— 零调用必须说清属于哪一类` })
    }
    for (const msg of evidenceErrors(root, e.evidence)) {
      errors.push({ code: 'AD-003', message: `${where}（${e.symbol}）${msg}` })
    }
  }
  return errors
}

/**
 * 逐条对账：现在零调用而基线没有 ⇒ `AD-001`；基线有而现在已经不零调用 ⇒ `AD-002`。
 *
 * @param {{symbol: string, path: string}[]} dead
 * @param {{symbols: object[]}} baseline
 */
export function judge(dead, baseline) {
  const errors = []
  const key = (s) => `${s.symbol}@${s.path}`
  const live = new Set(dead.map(key))
  const known = new Set()
  for (const e of baseline.symbols) {
    if (!e || typeof e.symbol !== 'string') continue
    known.add(key(e))
    if (!live.has(key(e))) {
      errors.push({
        code: 'AD-002',
        message: `${e.symbol}（${e.path}）在基线里，但生产里已经不再零调用（已接线或已删）—— 必须同批下调 ${BASELINE_REL}，否则基线留出的余量会被下一次新增正好填进来`,
      })
    }
  }
  for (const s of dead) {
    if (!known.has(key(s))) {
      errors.push({
        code: 'AD-001',
        message: `${s.symbol}（${s.path}:${s.line}）在生产里零调用，且不在基线 —— 它是被删丢了调用点的判据（接线 + 补用例），不是死代码`,
      })
    }
  }
  return errors
}

// ── 主流程 ────────────────────────────────────────────────────────────

export function main() {
  const read = REPLAY ? readTreeFromGit(REPLAY) : { files: readTreeFromDisk(ROOT) }
  if (read.error) {
    console.error(red(`范围读不出：${read.error} —— 判据不许在看不见源码时通过`))
    process.exit(2)
  }

  const surface = surfaceErrors(read.files)
  if (surface.errors.length) {
    for (const e of surface.errors) console.log(red(`[ERROR] ${e.code} ${e.message}`))
    process.exit(2)
  }

  const corpusByRel = new Map(
    read.files.filter((f) => CALL_TREES.some((t) => f.rel.startsWith(`${t}/`))).map((f) => [f.rel, callCorpus(f.text)]),
  )
  const dead = zeroCallSymbols(surface.symbols, corpusByRel)

  if (REPORT || REPLAY) {
    const where = REPLAY ? ` @ ${REPLAY}` : ''
    console.log(`arch-dead-code-audit${where}：${surface.symbols.length} 个 pub fn / ${dead.length} 个零生产调用`)
    for (const s of dead) console.log(dim(`  · ${s.symbol}  ${s.path}:${s.line}`))
    process.exit(0)
  }

  const baseline = readBaseline(ROOT)
  if (baseline.error) {
    console.error(red(`范围读不出：${baseline.error} —— 判据不许在看不见基线时通过`))
    process.exit(2)
  }

  const errors = [...checkBaseline(baseline, ROOT), ...judge(dead, baseline)]
  if (errors.length) {
    for (const e of errors) console.log(red(`[ERROR] ${e.code} ${e.message}`))
    console.log(
      `arch-dead-code-audit 未通过：${errors.length} 处违规（零调用 ${dead.length} 个 / 基线 ${baseline.symbols.length} 条）`,
    )
    process.exit(1)
  }

  console.log(
    green(
      `arch-dead-code-audit 通过：${surface.symbols.length} 个 pub fn / 零生产调用 ${dead.length} 个，` +
        `基线 ${baseline.symbols.length} 条（只许降）`,
    ),
  )
  for (const e of baseline.symbols) console.log(dim(`  · ${e.symbol}  ${e.why}（${e.evidence}）`))
}

const invokedDirectly = fileURLToPath(import.meta.url) === path.resolve(process.argv[1] ?? '')
if (invokedDirectly) main()
