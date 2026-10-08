#!/usr/bin/env node
/**
 * gate-codes-audit — 判据码命名空间对账
 *
 * 判据码是每条机械判定的**地址**（`D-006`、`E-009`、`R-002`…），豁免注释、门禁日志、
 * 文档指认都靠它。此前它只存在于各脚本的散文里，于是有两类静默失效：
 *
 *   - **撞号**：两个脚本各自启用同一码。`D-` 已被两个脚本共用（doc-link-audit 与
 *     doc-symbol-audit），共用本身不报错，但加规则时谁也不知道对方用到几号——
 *     撞上后豁免注释会同时命中两处，人只在其中一个脚本里看到「已豁免」。
 *   - **登记漏了**：新增一条判定没进登记表 ⇒ 后来人从表里挑「下一个可用号」，
 *     恰好挑到那条没登记的号上。
 *
 * 所以登记表不手抄：**每个脚本在自己文件里声明**（`@ns` 前缀 + `@codes` 号），
 * `gen-gate-codes.mjs` 从声明渲染 `docs/reference/GATE_CODES.md`，本脚本判声明本身。
 * 抄一份表就有两处真相，而它必然漂移——本仓同类事故已记在 `dead-code-audit` 的 R-002 旁。
 *
 * 判定口径：
 *   - 「用到」只算**字符串字面量**（`stringLiteralsOf`）。注释里的「原 E-010 已退役」
 *     是历史叙述，要求它进登记表就把叙述当事实，而这类叙述在头注释里成段存在；
 *     一条天天误报的守卫最后只会被人用豁免喂死。
 *   - 前缀没被任何 `@ns` 声明过的码**不看**（`ADR-023` 是决策记录编号，不是判据码）：
 *     这样「是不是判据码」由登记表自己定义，不需要本脚本里再养一份例外名单。
 *
 * 不在本脚本范围内：给没有编号的判定型脚本（style-audit / test-layout-audit 等）
 * 补判据码——那是输出契约的变更，不属命名空间卫生。
 *
 * 用法：node scripts/gate-codes-audit.mjs
 * 退出码：1 = 有违规；2 = 审计范围读不出（0 个脚本）。
 */

// 判据码命名空间（登记表 docs/reference/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns GC 判据码命名空间本身
// @codes GC-001 GC-002 GC-003 GC-004 GC-005 GC-006

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red } from './color.mjs'
import { parseGateCodeDecls, stringLiteralsOf } from './gate.d/_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

/** 被扫的脚本范围：`scripts/*.mjs` 与 `scripts/gate.d/*.mjs`（回归测试文件不判） */
function auditFiles(root) {
  const out = []
  for (const dir of ['scripts', path.join('scripts', 'gate.d')]) {
    if (!fs.existsSync(path.join(root, dir))) continue
    for (const name of fs.readdirSync(path.join(root, dir)).sort()) {
      if (!name.endsWith('.mjs') || name.endsWith('.test.mjs')) continue
      out.push(path.posix.join(dir.split(path.sep).join('/'), name))
    }
  }
  return out
}

/** 一个脚本的声明 + 它字符串里用到的码 */
function readScript(root, rel) {
  const text = fs.readFileSync(path.join(root, rel), 'utf8')
  const { ns, codes } = parseGateCodeDecls(text)
  // 声明行自己不算「用到了这个码」——否则 GC-003 恒真，登记错到别人名下也看不见
  const declBody = text.replace(/^ *\/\/ @(?:ns|codes) .*$/gm, '')
  const used = new Set()
  for (const str of stringLiteralsOf(text)) {
    for (const m of str.matchAll(/\b([A-Z]{1,4})-(\d{3})\b/g)) used.add(m[0])
  }
  return { rel, text, declBody, ns, codes, used }
}

export function collect(root = repoRoot) {
  return auditFiles(root).map((rel) => readScript(root, rel))
}

/**
 * @returns {{error: string[], warning: string[]}} 违规逐条给人读的一行话
 */
export function checkDecls(scripts) {
  const error = []
  const owner = new Map() // 码 → 声明它的脚本
  const nsOwner = new Map() // 前缀 → {desc, files[]}

  for (const s of scripts) {
    for (const [prefix, desc] of s.ns) {
      if (!nsOwner.has(prefix)) nsOwner.set(prefix, { desc, files: [] })
      nsOwner.get(prefix).files.push(s.rel)
    }
    for (const code of s.codes.keys()) {
      const prev = owner.get(code)
      if (prev) {
        error.push(`GC-001 撞号：${code} 同时由 ${prev} 与 ${s.rel} 声明`)
        continue
      }
      owner.set(code, s.rel)
      const prefix = code.slice(0, code.indexOf('-'))
      if (!s.ns.has(prefix)) {
        error.push(`GC-002 未登记前缀：${s.rel} 声明了 ${code}，却没有 \`// @ns ${prefix} <归属>\``)
      }
      if (!s.declBody.includes(code)) {
        error.push(`GC-003 登记错位：${s.rel} 的 @codes 写了 ${code}，本文件里再没有这个码（号写错或登记到别人名下）`)
      }
    }
  }

  // 「是不是判据码」由登记表自己定义：只有已声明前缀下的码才参与对账
  const knownPrefixes = new Set(nsOwner.keys())
  for (const s of scripts) {
    for (const code of s.used) {
      if (!knownPrefixes.has(code.slice(0, code.indexOf('-')))) continue
      if (owner.has(code)) continue
      error.push(`GC-004 输出了未登记的码：${s.rel} 用到 ${code}，没有任何脚本声明它（加判定要先登记 @codes）`)
    }
  }

  for (const [prefix, entry] of nsOwner) {
    const claimed = [...owner.entries()].filter(([code]) => code.startsWith(`${prefix}-`))
    if (claimed.length === 0) {
      error.push(`GC-005 空命名空间：${prefix}（${entry.desc}）被 ${entry.files.join('、')} 声明，却没有任何 @codes 用到它`)
    }
    for (const other of entry.files.slice(1)) {
      const desc = scripts.find((s) => s.rel === other)?.ns.get(prefix)
      if (desc !== entry.desc) {
        error.push(`GC-006 同一前缀两种说法：${prefix} 在 ${entry.files[0]} 是「${entry.desc}」，在 ${other} 是「${desc}」`)
      }
    }
  }
  return { error, warning: [] }
}

export function formatReport(scripts, { error }) {
  const lines = []
  lines.push(`--- gate-codes-audit: 判据码命名空间对账（${scripts.length} 个脚本） ---`)
  const total = scripts.reduce((n, s) => n + s.codes.size, 0)
  const prefixes = new Set(scripts.flatMap((s) => [...s.ns.keys()]))
  lines.push(`   前缀 ${prefixes.size} 个 · 判据码 ${total} 条`)
  for (const e of error) lines.push(`${red('[ERROR]')} ${e}`)
  if (!error.length) lines.push(green('   无撞号、无未登记前缀、无未登记的码'))
  return lines.join('\n')
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const scripts = collect()
  if (scripts.length === 0) {
    console.log(`${red('[ERROR]')} 审计范围读不出：0 个脚本（${repoRoot}）`)
    process.exit(2)
  }
  const result = checkDecls(scripts)
  console.log(formatReport(scripts, result))
  process.exit(result.error.length ? 1 : 0)
}
