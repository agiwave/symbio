#!/usr/bin/env node
/**
 * slo-exit-audit — SLO 出口表里每个「用户可见指标」必须指得到一列读数（SX-001…003）
 *
 * ## 它治的病
 *
 * [plan/13 批 P1](./docs/plan/13-质量改进总纲.md) 登记的原始问题：**出口判据是
 * 「测试有没有过」，不是「用户看见的数变了没有」**。`04 §1.3` 那张逐阶出口表有 14 行
 * 指标， measured 方式一栏全是「埋点 / 日志扫描 / 投影比对 / 单测」——**没有一行说得出
 * 用户在界面上读哪里**。而 `session/stats` 这个只读口早就存在（八列 + 四列），
 * 读数列与出口指标之间**没有任何东西在对账**。
 *
 * 本脚本就是那一层对账。它不判「产品判断对不对」（纪律 4：那不可机械化），
 * 只判一条可核对的锚：**表里指认的每一列，代码里真有一列**。
 *
 * T-006 起，这套口径已登记为条目 [`registry/spec/S31-2.md`](../registry/spec/S31-2.md)
 * （owner = 本脚本；条目只固化「指认必须落到真列 + `—` 是待建」两条对账关系，不复述规则正文）。
 *
 * ## 判据
 *
 * | 码 | 判什么 |
 * |---|---|
 * | SX-001 | 出口表指认了 `session/stats` 里不存在的列 ⇒ 红（指认漂移 / 列被改名） |
 * | SX-002 | 规则源读不出（出口表或 `stats.rs` 的列集合提取失败）⇒ exit 2 |
 * | SX-003 | 反向自检用：出口表本身形状不对（没有那一列 / 没有 `阶` 头）⇒ exit 2 |
 *
 * ## 为什么 `—` 不是豁免
 *
 * `—` = 这一项**还没有用户可见读数**。它不是「不用有」，是「待建」——所以脚本每次
 * 都把 `—` 的行数打进日志，让「还有多少出口是用户看不见的」是一个**每次门禁都看得见
 * 的数**，而不是一份躺在文档里的自我原谅。真要给某一行配上读数，改的是表里的 `—`。
 *
 * 用法：node scripts/slo-exit-audit.mjs [--root=<仓库根>] [--strict]
 * 退出码：0 = 通过；1 = SX-001；2 = 范围读不出（**不退化成「什么都没检查」**）。
 */
// 判据码命名空间（登记表 docs/reference/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns SX SLO 出口表 vs session/stats 读数列（用户可见出口）
// @codes SX-001 SX-002 SX-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red } from './color.mjs'
import { plain, tableWithHeader } from './md-table.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? path.resolve(rootArg.slice(7)) : path.resolve(scriptDir, '..')

const PLAN04_REL = path.join('docs', 'plan', '04-工程落地.md')
const STATS_REL = path.join('symbio', 'src', 'plugins', 'session', 'stats.rs')

/** 出口表的表头第一格——用它定位 §1.3 那张表（全文只有一张以此为头的表）。 */
const TABLE_ANCHOR = '阶'
/** 新列的名字。改名要连本脚本一起改，否则 SX-003 会红——那是刻意的。 */
const EXIT_HEAD = '用户可见出口'

/**
 * `session/stats` 的读数列集合。
 *
 * 从 `stats.rs` 的两个 `#[derive(Serialize)]` 结构体提 `pub` 字段：`SessionStats`
 * （顶层列）与 `TierRow`（`tiers` 列的子列）。**不硬编码一份列名**——硬编码的那份
 * 会在加列时静默变成假话（表里指认得对、代码里没有，守卫却照绿）。
 *
 * 提取失败一律返回 null ⇒ 调用方 exit 2。宁可我读不出，不可我漏读。
 */
export function readColumns(root) {
  const abs = path.join(root, STATS_REL)
  if (!fs.existsSync(abs)) return null
  const txt = fs.readFileSync(abs, 'utf8')

  const fieldsOf = (structName) => {
    const at = txt.indexOf(`struct ${structName}`)
    if (at < 0) return null
    // 结构体字面量：从 `{` 到与之配对的 `}`。depth 计数避免正文里的 `}` 提前收尾。
    let i = txt.indexOf('{', at)
    if (i < 0) return null
    let depth = 0
    for (let j = i; j < txt.length; j++) {
      if (txt[j] === '{') depth++
      else if (txt[j] === '}') {
        depth--
        if (depth === 0) {
          i = j
          break
        }
      }
    }
    const body = txt.slice(at, i)
    // 只取 `pub 名字:` 形体；`pub(crate)` 不在本文件出现（出口结构体字段全是 pub）。
    const names = []
    for (const m of body.matchAll(/^\s*pub\s+([a-z_][a-z0-9_]*)\s*:/gm)) names.push(m[1])
    return names.length ? names : null
  }

  const top = fieldsOf('SessionStats')
  const tier = fieldsOf('TierRow')
  if (!top || !tier) return null
  // 顶层列 + `tiers.<子列>`：出口表用点号指子列，故这里一次摊平。
  return new Set([...top, ...tier.map((f) => `tiers.${f}`)])
}

/**
 * 出口表的行：`[{ stage, raw }]`。`raw` 是「用户可见出口」格的原文（未去反引号）。
 *
 * 表读不出 ⇒ null。**区分「表不在」与「表在但形状不对」没有意义**：两者都让本守卫
 * 失去规则源，都该 exit 2（SX-003）。
 */
export function readExitRows(root) {
  const abs = path.join(root, PLAN04_REL)
  if (!fs.existsSync(abs)) return null
  const txt = fs.readFileSync(abs, 'utf8')
  const t = tableWithHeader(txt, TABLE_ANCHOR)
  if (!t) return null
  const col = t.header.findIndex((h) => plain(h).startsWith(EXIT_HEAD))
  if (col < 0) return null
  if (t.rows.length === 0) return null
  return t.rows.map((r) => ({ stage: plain(r[0] ?? ''), raw: plain(r[col] ?? '') }))
}

/** 一格指认 → 列名清单。`—` / 空 ⇒ []（这一项没有用户可见读数）。 */
function columnsOf(cell) {
  const out = []
  for (const part of cell.split(/[；;，,、]/)) {
    const p = plain(part).trim()
    if (!p || p === '—' || p === '-' || p === '－') continue
    out.push(p)
  }
  return out
}

console.log('--- slo-exit-audit: SLO 出口表的用户可见指标 vs session/stats 读数列 ---')

const rows = readExitRows(REPO_ROOT)
const columns = readColumns(REPO_ROOT)
if (!rows || !columns) {
  console.log(
    red(
      '[ERROR] SX-002/SX-003 审计范围读不出：' +
        `出口表=${rows ? 'ok' : 'null'} 读数列=${columns ? columns.size : 'null'} ⇒ ` +
        '来源读不出来，此时退化成「什么都没检查」是最坏的一种失败',
    ),
  )
  process.exit(2)
}

let errors = 0
let declared = 0
let missing = 0
const declaredColumns = new Set()

for (const row of rows) {
  for (const name of columnsOf(row.raw)) {
    declared++
    declaredColumns.add(name)
    if (!columns.has(name)) {
      errors++
      console.log(
        red(
          `[ERROR] SX-001 ${row.stage} 的「${EXIT_HEAD}」指认了 \`${name}\`，` +
            `而 session/stats 没有这一列 ⇒ 指认漂移（列被改名 / 出口还没建就写上了）`,
        ),
      )
    }
  }
  if (columnsOf(row.raw).length === 0) missing++
}

if (errors) {
  console.log(`slo-exit-audit 未通过：${errors} 处指认对不上（扫 ${rows.length} 行）`)
  process.exit(1)
}
console.log(
  green(
    `slo-exit-audit 通过：出口表 ${rows.length} 行，${declared} 处指认全部指得到列` +
      `（${declaredColumns.size} 个不同列）；${missing} 行**没有**用户可见读数（—）`,
  ),
)
