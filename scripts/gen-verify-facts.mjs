#!/usr/bin/env node
/**
 * gen-verify-facts — 把 verify 程序的**期望数据**从程序里搬到文档里
 *
 * ## 它治的病
 *
 * `docs/plan/verify/*.rs` 是 v2 方案的硬证据（`docs/plan/README.md` §4）。但在本批之前，
 * 那些证据的**输入**全是程序里手填的常量：机制表 10 键、13 阶的赋值向量、能力条目数 54。
 * 于是程序验的是「计划自洽」，不是「计划 == 权威文档」——文档改了，程序照绿。
 * 而「改了文档要有人记得同步改程序」恰恰是这类体系里最先断的那环（`_shared.mjs` 的
 * 基线漏改在同一天发生了两次，就是这个机制缺失的直接后果）。
 *
 * ## 数据的 owner（生成器只搬运，不判断）
 *
 * | 生成物 | 真源 |
 * |---|---|
 * | `MECHANISMS` | [01 §8 权威参数表](../docs/plan/01-核心架构.md) 的「参数键」列 |
 * | `STAGE_CLAIMS` | [路线图总览 §1 阶梯总表](../docs/plan/roadmap/00-路线图总览.md) |
 * | `STAGE_DOCS` | 每阶 `S0N-*.md` §3 的 ```capability-assign``` 块 + §4 的加粗平凡值行 |
 *
 * **顶层键 vs 子键**：`projection.param` 在 §8 里与顶层键同列一张表，但 01 §8 自己写明
 * 「它不是第 11 个机制键，是 `projection` 的子键」。判据因此是形状而非名单：
 * 键名带 `.` 且首段本身也是一个登记的键 ⇒ 子键。这条规则同时终结了「机制键 10 还是 11」
 * 的口径分歧（plan/13 批 D3 数到的那处），因为它把答案交给了表本身。
 *
 * **加粗的平凡值行**：§4 每阶列 2–3 行参数，其中**恰好一行**的平凡值加粗——那是本阶的
 * 退路口（S01 退回 `decider`、S03 退回 `scope=root`）。加粗是文档里已有的记号，
 * 不是为本批新发明的语法。读到 0 行或 >1 行 ⇒ **抛错**，不猜。
 *
 * ## 读不出来必须红
 *
 * 表头列数不对、块缺失、数字解析不出来——一律 throw。生成器静默产出一份「少几行」的
 * facts，比不产出更糟：程序会拿着残缺数据照常断言，并给出绿灯。
 *
 * ## 用法
 *
 *   node scripts/gen-verify-facts.mjs            # 写 docs/plan/verify/facts/mod.rs
 *   node scripts/gen-verify-facts.mjs --check    # 只比对，不一致 exit 1（门禁档）
 *   node scripts/gen-verify-facts.mjs --stdout   # 打到标准输出（测试用，不落盘）
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { tableWithHeader, plain, fencedBlock } from './md-table.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const OUT_REL = path.join('docs', 'plan', 'verify', 'facts', 'mod.rs')

const ARCH_REL = path.join('docs', 'plan', '01-核心架构.md')
const LADDER_REL = path.join('docs', 'plan', 'roadmap', '00-路线图总览.md')
const STAGE_DIR_REL = path.join('docs', 'plan', 'roadmap')

const read = (rel) => fs.readFileSync(path.join(repoRoot, rel), 'utf8')

/** 机制键的形状：小写字母段用 `.` 连接（`actor.budget_ms`）；不符合的单元格不是键 */
const KEY_RE = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*$/

/**
 * 01 §8 → `{ all: 全部参数键（含子键）, top: 顶层机制键 }`。
 *
 * `top` 的判据是**子键规则**：`X.Y` 且 `X` 也在表里 ⇒ `X.Y` 是 `X` 的子键，不另计一个机制。
 * 名单会漂移（谁记得 `projection.param` 算不算），规则不会。
 */
export function mechanismsOf(archMd) {
  const table = tableWithHeader(archMd, '参数键')
  if (!table) throw new Error(`${ARCH_REL}：找不到「参数键」表头的那张表——01 §8 的表头改了？`)
  if (table.header.length < 4 || plain(table.header[2]) !== '平凡值') {
    throw new Error(`${ARCH_REL} §8：表头不是「参数键 / 取值域 / 平凡值 / 用于」四列，实际是 ${table.header.join(' | ')}`)
  }
  const all = table.rows.map((r) => plain(r[0])).filter((k) => KEY_RE.test(k))
  if (all.length === 0) throw new Error(`${ARCH_REL} §8：表里没有解析出任何参数键`)
  const known = new Set(all)
  const top = all.filter((k) => !(k.includes('.') && known.has(k.split('.')[0])))
  return { all, top }
}

/**
 * 一阶的 §3 赋值块 → `[[键, 取值], …]`。
 *
 * 块里只允许 `key = value` 与 `#` 注释行；出现别的行 ⇒ 抛错。放宽成「跳过读不懂的行」
 * 就等于让一个打错的键名静静消失，而那个键从此无人审计。
 */
export function assignsOf(stageMd, where) {
  const block = fencedBlock(stageMd, 'capability-assign')
  if (!block) throw new Error(`${where}：§3 缺少 \`\`\`capability-assign 赋值块`)
  const pairs = []
  for (const line of block) {
    const t = line.trim()
    if (!t || t.startsWith('#')) continue
    const m = t.match(/^([a-z0-9_.]+)\s*=\s*(.+)$/)
    if (!m) throw new Error(`${where}：赋值块里有一行既不是 \`键 = 取值\` 也不是注释：${t}`)
    pairs.push([m[1], m[2].trim()])
  }
  if (!pairs.length) throw new Error(`${where}：赋值块没有任何 \`键 = 取值\` 行`)
  return pairs
}

/**
 * 一阶的 §4 表 → 本阶的退路口 `(键, 平凡值)`。
 *
 * 只认**平凡值单元格带 `**`** 的那一行；先按机制表（`mechanisms`）滤掉不是机制键的行
 * （S12 的 `ConationPolicy.enabled` 是策略字段，不是机制键，它加粗是因为那一行是「欲」
 * 的开关说明——不是阶梯的退路口）。0 行或 >1 行都抛错：退路口是每阶唯一的一处。
 */
export function fallbackOf(stageMd, mechanisms, where) {
  // 只在 §4 那一节里找：文档里 `| 参数 | … |` 开头的表不止一张（S10 的 §3.1 也有一张），
  // 而「退路口」是 §4 定义的，按节限定比按出现顺序猜稳。
  const heading = stageMd.split(/\r?\n/).findIndex((l) => /^## 4\./.test(l))
  if (heading < 0) throw new Error(`${where}：没有「## 4. 平凡值与回退」这一节`)
  const table = tableWithHeader(stageMd, '参数', { fromLine: heading })
  if (!table) throw new Error(`${where}：§4 里找不到「参数 / 完整值 / 平凡值」表`)
  const rows = table.rows
    .map((r) => ({ key: plain(r[0]), trivial: (r[2] || '').trim() }))
    .filter((x) => mechanisms.includes(x.key))
  const bolded = rows.filter((x) => x.trivial.includes('**'))
  if (bolded.length !== 1) {
    throw new Error(
      `${where}：§4 里平凡值加粗的机制键行应有且只有 1 行（那是本阶的退路口），实际 ${bolded.length} 行：` +
        `${rows.map((x) => x.key).join(', ')}`,
    )
  }
  return [bolded[0].key, plain(bolded[0].trivial)]
}

/** 路线图总览 §1 → 13 阶的**声明**（新增机制键数 / 参数变化数都来自这里，是被验的结论） */
export function claimsOf(ladderMd) {
  const table = tableWithHeader(ladderMd, '阶')
  if (!table) throw new Error(`${LADDER_REL}：找不到阶梯总表（表头首列「阶」）`)
  const newIdx = table.header.findIndex((h) => h.includes('新增机制键'))
  const chgIdx = table.header.findIndex((h) => h.includes('参数变化'))
  if (newIdx < 0 || chgIdx < 0) {
    throw new Error(`${LADDER_REL}：总表缺少「新增机制键」或「参数变化」列，表头是 ${table.header.join(' | ')}`)
  }
  return table.rows.map((r) => ({
    id: plain(r[0]),
    name: plain(r[1]),
    tier: plain(r[2]),
    newKeys: Number(plain(r[newIdx])),
    paramChanges: Number(plain(r[chgIdx])),
  }))
}

/** roadmap/ 目录里真正存在的阶文件（`S01-xxx.md`），**不含**总览 */
export function stageFilesOf(dirEntries) {
  return dirEntries
    .filter((f) => /^S\d{2}-.+\.md$/.test(f))
    .sort()
    .map((f) => ({ id: f.slice(0, 3), file: f }))
}

/** 汇总成一份 facts 文本（导出给测试用：同一份代码，测试喂假文档） */
export function buildFacts({ archMd, ladderMd, stageMds, dirEntries }) {
  const { all, top } = mechanismsOf(archMd)
  const claims = claimsOf(ladderMd)
  const files = stageFilesOf(dirEntries)

  const byId = new Map(claims.map((c) => [c.id, c]))
  const missing = files.filter((f) => !byId.has(f.id)).map((f) => f.id)
  const absent = claims.filter((c) => !files.some((f) => f.id === c.id)).map((c) => c.id)
  if (missing.length || absent.length) {
    throw new Error(
      `阶梯总表与 roadmap 的阶文件不是一对一同名：只在总表里 ${absent.join(', ') || '无'}；` +
        `只在目录里 ${missing.join(', ') || '无'}`,
    )
  }

  const docs = files.map(({ id, file }) => {
    const md = stageMds[id]
    if (md === undefined) throw new Error(`${file}：读不到正文`)
    const where = `${STAGE_DIR_REL}/${file}`
    const assigns = assignsOf(md, where)
    const claim = byId.get(id)
    if (assigns.length !== claim.paramChanges) {
      throw new Error(
        `${where}：§3 赋值块有 ${assigns.length} 项，总表声明 S${id.slice(1)} 的参数变化是 ${claim.paramChanges} 项` +
          `——两处必须同改，否则总表那一列在说假话`,
      )
    }
    for (const [k] of assigns) {
      if (!all.includes(k)) {
        throw new Error(`${where}：赋值块用了不在 01 §8 参数表里的键 \`${k}\`——新增机制键要走 ADR，不是走文档笔误`)
      }
    }
    return { id, file, assigns, fallback: fallbackOf(md, top, where) }
  })

  const rs = (s) => `"${String(s).replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`
  const out = []
  out.push('// 生成物：由 `scripts/gen-verify-facts.mjs` 从 docs/plan 抽取，**不要手改**。')
  out.push('// 要改这些数据就改文档（01 §8 / 路线图总表 / 各阶 §3·§4），然后重跑生成脚本。')
  out.push('//')
  out.push('// 每个 verify 程序都是一个**独立的 crate**，各自只 `use` 下面的一部分；')
  out.push('// 没被某个程序读到的那些不是死码，是另一个程序在读。这里判死码只会制造噪音，')
  out.push('// 所以整模块关掉这条 lint——纯数据模块没有逻辑，关掉不掩盖任何真实缺陷。')
  out.push('#![allow(dead_code)]')
  out.push('')
  out.push('/// 01 §8 的顶层机制键（`projection.param` 这类子键不计，判据见生成器注释）')
  out.push(`pub const MECHANISMS: &[&str] = &[${top.map((k) => `\n    ${rs(k)},`).join('')}\n];`)
  out.push('')
  out.push('/// 01 §8 参数表里登记的全部键（含子键）——赋值块的合法词表')
  out.push(`pub const PARAM_KEYS: &[&str] = &[${all.map((k) => `\n    ${rs(k)},`).join('')}\n];`)
  out.push('')
  out.push('/// 路线图总表 §1 对某一阶的**声明**——是被验的结论，不是输入')
  out.push(`pub struct StageClaim {
    pub id: &'static str,
    pub name: &'static str,
    pub tier: &'static str,
    /// 总表声明的「新增机制键」
    pub claimed_new_keys: usize,
    /// 总表声明的「参数变化」——等于该阶 §3 赋值块的行数
    pub claimed_param_changes: usize,
}`)
  out.push('')
  out.push(`pub const STAGE_CLAIMS: &[StageClaim] = &[${claims
    .map(
      (c) =>
        `\n    StageClaim {\n        id: ${rs(c.id)},\n        name: ${rs(c.name)},\n        tier: ${rs(c.tier)},\n` +
        `        claimed_new_keys: ${c.newKeys},\n        claimed_param_changes: ${c.paramChanges},\n    },`,
    )
    .join('')}\n];`)
  out.push('')
  out.push('/// 一阶的 §3 赋值块 + §4 退路口——阶梯审计的**输入**')
  out.push(`pub struct StageDoc {
    pub id: &'static str,
    /// roadmap 目录里的那篇（文件名即 owner 声明）
    pub file: &'static str,
    pub assigns: &'static [(&'static str, &'static str)],
    /// §4 里平凡值加粗的那一行：\`(机制键, 平凡值)\`
    pub fallback: (&'static str, &'static str),
}`)
  out.push('')
  const docLiterals = docs
    .map(
      (d) =>
        `\n    StageDoc {\n        id: ${rs(d.id)},\n        file: ${rs(d.file)},\n` +
        `        assigns: &[${d.assigns.map(([k, v]) => `\n            (${rs(k)}, ${rs(v)}),`).join('')}\n        ],\n` +
        `        fallback: (${rs(d.fallback[0])}, ${rs(d.fallback[1])}),\n    },`,
    )
    .join('')
  out.push(`pub const STAGE_DOCS: &[StageDoc] = &[${docLiterals}\n];`)
  out.push('')
  return out.join('\n')
}

/** 从真实仓库读输入并生成（测试直接调 `buildFacts` 喂内存里的假文档） */
export function generate(root = repoRoot) {
  const stageDir = path.join(root, STAGE_DIR_REL)
  const dirEntries = fs.readdirSync(stageDir)
  const claims = claimsOf(fs.readFileSync(path.join(root, LADDER_REL), 'utf8'))
  const stageMds = {}
  for (const c of claims) {
    const f = dirEntries.find((x) => x.startsWith(`${c.id}-`) && /^S\d{2}-.+\.md$/.test(x))
    if (f) stageMds[c.id] = fs.readFileSync(path.join(stageDir, f), 'utf8')
  }
  return buildFacts({
    archMd: fs.readFileSync(path.join(root, ARCH_REL), 'utf8'),
    ladderMd: fs.readFileSync(path.join(root, LADDER_REL), 'utf8'),
    stageMds,
    dirEntries,
  })
}

const MAIN = process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))
if (MAIN) {
  const args = process.argv.slice(2)
  let text
  try {
    text = generate()
  } catch (e) {
    console.error(`gen-verify-facts: ${e.message}`)
    process.exit(1)
  }
  const outPath = path.join(repoRoot, OUT_REL)
  if (args.includes('--stdout')) {
    process.stdout.write(text)
  } else if (args.includes('--check')) {
    const onDisk = fs.existsSync(outPath) ? fs.readFileSync(outPath, 'utf8') : ''
    if (onDisk === text) {
      console.log(`gen-verify-facts --check: ${OUT_REL} 与文档一致`)
    } else {
      console.error(
        `gen-verify-facts --check: ${OUT_REL} 已与文档不一致。\n` +
          '  改了 01 §8 / 阶梯总表 / 各阶 §3·§4 就要重跑 `node scripts/gen-verify-facts.mjs`——\n' +
          '  让 verify 程序继续吃旧数据，它验的就还是旧计划（绿灯是假的）。',
      )
      process.exit(1)
    }
  } else {
    fs.mkdirSync(path.dirname(outPath), { recursive: true })
    fs.writeFileSync(outPath, text)
    console.log(`✓ 已生成 ${OUT_REL}（${(text.match(/\n/g) || []).length + 1} 行）`)
  }
}
