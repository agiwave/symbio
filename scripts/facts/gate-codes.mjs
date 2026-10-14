#!/usr/bin/env node
/**
 * 事实 · `docs/ref/GATE_CODES.md` —— 判据码登记表
 *
 * 存在理由同 `facts/current.mjs`：**能生成的表不手抄**。判据码的归属只有
 * 脚本自己知道，人在别处抄一份就必然出现「表说有这条、脚本里早删了」；而这张表
 * 唯一的用途（加新判据时挑一个不撞的号）恰好最怕抄错。
 *
 * 真源是各脚本文件头之后的两行声明（`// @ns <前缀> <归属>`、`// @codes <码>…`），
 * 口径与对账见 `gate-codes-audit.mjs`；每条判据**判什么**归该脚本自己的头注释，
 * 本表只给地址地图，不复述。
 *
 * 由 `scripts/gen-facts.mjs` 遍历 `scripts/facts/` 时调用，不要直接跑。
 */

import { collect } from "../gate-codes-audit.mjs";
import { fileURLToPath } from "node:url";
import path from "node:path";

// 本文件住 `scripts/facts/`，故仓根要上溯两级（原来在 `scripts/` 下只需一级）。
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const OUT_REL = "docs/ref/GATE_CODES.md";

const num = (code) => Number(code.slice(code.indexOf('-') + 1))

/** `D-001 D-002 D-003 D-006` ⇒ `D-001–003、D-006`（连续段折叠，空洞一眼可见） */
function formatRanges(prefix, codes) {
  const sorted = [...codes].sort((a, b) => num(a) - num(b))
  if (!sorted.length) return '—'
  const out = []
  let run = [sorted[0]]
  for (const code of sorted.slice(1)) {
    if (num(code) === num(run[run.length - 1]) + 1) run.push(code)
    else {
      out.push(run)
      run = [code]
    }
  }
  out.push(run)
  return out
    .map((r) =>
      r.length > 1 ? `${r[0]}–${String(num(r[r.length - 1])).padStart(3, '0')}` : r[0],
    )
    .join('、')
}

function render() {
  const scripts = collect(ROOT)
  const prefixes = new Map() // 前缀 → {desc, files:Set, codes:Set}
  const rows = []
  for (const s of scripts) {
    for (const [prefix, desc] of s.ns) {
      if (!prefixes.has(prefix)) prefixes.set(prefix, { desc, files: new Set(), codes: new Set() })
      prefixes.get(prefix).files.add(s.rel)
    }
    for (const code of s.codes.keys()) {
      const entry = prefixes.get(code.slice(0, code.indexOf('-')))
      if (entry) entry.codes.add(code) // 前缀没声明的码由 GC-002 判红，这里不参与渲染
      rows.push({ code, file: s.rel })
    }
  }
  const byPrefix = (a, b) => a.code.localeCompare(b.code)
  rows.sort(byPrefix)

  const L = []
  L.push('# 判据码登记表（自动生成）')
  L.push('')
  L.push('> ⚠️ **本表由 `scripts/facts/gate-codes.mjs` 从各脚本的 `// @ns` / `// @codes` 声明生成，请勿手改。**')
  L.push('> 改了声明不用手动重跑——门禁会**自动重新生成并暂存**（`scripts/gen-facts.mjs`，见 `scripts/gate.d/60-facts.mjs`）。')
  L.push('>')
  L.push('> 判据码是每条机械判定的**地址**：豁免注释、门禁日志、文档指认都指向它。')
  L.push('> 每条判据**判什么**归声明它的那个脚本的头注释；撞号与漏登记由')
  L.push('> [`scripts/gate-codes-audit.mjs`](../../scripts/gate-codes-audit.mjs) 判（GC-001–006）。')
  L.push('')
  L.push('## 1. 命名空间')
  L.push('')
  L.push('| 前缀 | 归属 | 声明脚本 | 已用号段 | 下一个可用号 |')
  L.push('|---|---|---|---|---|')
  for (const prefix of [...prefixes.keys()].sort()) {
    const { desc, files, codes } = prefixes.get(prefix)
    const next = codes.size
      ? `${prefix}-${String(Math.max(...[...codes].map(num)) + 1).padStart(3, '0')}`
      : `${prefix}-001`
    const owners = [...files].sort().map((f) => '`' + f + '`').join(' · ')
    L.push(
      `| **${prefix}** | ${desc} | ${owners} | ${formatRanges(prefix, codes)} | \`${next}\` |`,
    )
  }
  L.push('')
  L.push('## 2. 逐码归属')
  L.push('')
  L.push('| 判据码 | 声明脚本 |')
  L.push('|---|---|')
  for (const r of rows) L.push(`| **${r.code}** | \`${r.file}\` |`)
  L.push('')
  return L.join('\n')
}

// ================= 事实描述符（由 `scripts/gen-facts.mjs` 遍历调用） =================
//
// 本表**不带溯源行**：它的全部输入是各脚本里的 `@ns` / `@codes` 声明，
// 同输入必然同输出——加一行时间戳只会让门禁每次都报「修复了 1 个文件」。
export default {
  id: 'gate-codes',
  title: '判据码登记表（各脚本 `@ns` / `@codes` 声明的投影）',
  out: OUT_REL,
  stamp: false,
  render,
}
