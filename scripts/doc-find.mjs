#!/usr/bin/env node
/**
 * doc-find —— 全仓文档检索（唯一入口）
 *
 * 为什么有这个脚本：
 *   项目文档是「下沉」的——系统级规范在 docs/，单模块机制在各模块 README.md / docs/，
 *   还有相当一部分写在源码的 `//!` 模块文档里（如 symbio_core::memory）。散落是对的，
 *   但「某条约定写在哪」就必须可检索，否则它会以摘要形式回流进记忆，而摘要必然漂移。
 *
 * 用法：
 *   node scripts/doc-find.mjs 三层记忆
 *   node scripts/doc-find.mjs canonicalize_loose --rs      # 只搜源码注释
 *   node scripts/doc-find.mjs "会话标题" --limit 5
 *   node scripts/doc-find.mjs 闸门 --context 2             # 多带 2 行上下文
 *
 * 覆盖：
 *   - *.md（docs/、各模块 README.md / docs/、CHANGELOG 等）
 *   - *.rs 的 `//!`（模块文档）与 `///`（项文档）
 *
 * 排除：archive/、node_modules/、target/、.git/、.symbio/、以及 .workbuddy 开头的目录
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { dim, cyan, yellow, green, bold } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

const argv = process.argv.slice(2)
const limitArg = argv.find((a) => a.startsWith('--limit='))
const ctxArg = argv.find((a) => a.startsWith('--context='))
const LIMIT = limitArg ? Number(limitArg.slice(8)) : 20
const CONTEXT = ctxArg ? Number(ctxArg.slice(10)) : 0
const ONLY_RS = argv.includes('--rs')
const ONLY_MD = argv.includes('--md')
const QUERY = argv.filter((a) => !a.startsWith('--'))[0]

const EXCLUDED = /(^|[\/\\])(archive|node_modules|target|\.git|\.symbio)(\/|\\|$)|(^|[\/\\])\.workbuddy[^/\\]*([\/\\]|$)/
const MD_MAX_BYTES = 2 * 1024 * 1024
const RS_MAX_BYTES = 1024 * 1024

if (!QUERY) {
  console.error(yellow('用法：node scripts/doc-find.mjs <关键词> [--md|--rs] [--limit=N] [--context=N]'))
  process.exit(2)
}

/**
 * Rust 里值得看的行：文档注释（`//!` / `///`）+ 定义行本身。
 *
 * 只搜文档注释会漏掉「搜符号名」这个最常见的用法——`canonicalize_loose` 的
 * 说明写在它上方的 `///` 里，而那三行并不包含这个词，命中行是 `fn canonicalize_loose(...)`。
 */
const RS_DEF_RE = /^\s*(pub(\([^)]*\))?\s+)?(async\s+)?(fn|struct|enum|const|static|type|trait|impl|mod)\b/
const isRsInteresting = (line) => /^\s*\/\/[!/]/.test(line) || RS_DEF_RE.test(line)

/** 递归收集文件，跳过排除目录 */
function walk(dir, out = []) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    const full = path.join(dir, e.name)
    const rel = path.relative(repoRoot, full).split(path.sep).join('/')
    if (EXCLUDED.test(rel + '/')) continue
    if (e.isDirectory()) walk(full, out)
    else if (e.isFile()) out.push(full)
  }
  return out
}

const needle = QUERY.toLowerCase()

const hits = []
let scanned = 0

for (const file of walk(repoRoot)) {
  const rel = path.relative(repoRoot, file).split(path.sep).join('/')
  const isMd = file.endsWith('.md')
  const isRs = file.endsWith('.rs')
  if (!isMd && !isRs) continue
  if (ONLY_RS && !isRs) continue
  if (ONLY_MD && !isMd) continue

  let stat
  try {
    stat = fs.statSync(file)
  } catch {
    continue
  }
  if (stat.size > (isMd ? MD_MAX_BYTES : RS_MAX_BYTES)) continue

  let text
  try {
    text = fs.readFileSync(file, 'utf8')
  } catch {
    continue
  }
  scanned++

  const lines = text.split('\n')
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    if (isRs && !isRsInteresting(line)) continue // Rust：文档注释 + 定义行（符号本体）
    if (!line.toLowerCase().includes(needle)) continue

    const ctx = []
    if (CONTEXT > 0) {
      for (let k = Math.max(0, i - CONTEXT); k <= Math.min(lines.length - 1, i + CONTEXT); k++) {
        if (k !== i) ctx.push(`${String(k + 1).padStart(4)}  ${dim(lines[k].trimEnd())}`)
      }
    }
    hits.push({ rel, line: i + 1, text: line.trimEnd(), ctx })
    if (hits.length >= LIMIT * 4) break // 够用即可，避免扫全仓
  }
  if (hits.length >= LIMIT * 4) break
}

// 排序：CHANGELOG 靠后（它是历史，不是「现在是什么」），其余按路径
hits.sort((a, b) => {
  const aCh = a.rel.includes('CHANGELOG') ? 1 : 0
  const bCh = b.rel.includes('CHANGELOG') ? 1 : 0
  if (aCh !== bCh) return aCh - bCh
  return a.rel.localeCompare(b.rel) || a.line - b.line
})

const shown = hits.slice(0, LIMIT)

console.log(bold(`\n🔍 "${QUERY}" —— 扫过 ${scanned} 个 md / rs 文件，命中 ${hits.length} 处`))
if (!shown.length) {
  console.log(yellow('  无命中。试试更短的词，或去掉 --md / --rs 限制。'))
} else {
  let lastFile = null
  for (const h of shown) {
    if (h.rel !== lastFile) {
      console.log(`\n${cyan(h.rel)}`)
      lastFile = h.rel
    }
    console.log(`  ${dim(String(h.line).padStart(4))}  ${h.text.trimEnd()}`)
    for (const c of h.ctx) console.log(`  ${c}`)
  }
  if (hits.length > shown.length) {
    console.log(dim(`\n  … 还有 ${hits.length - shown.length} 处，加 --limit=${hits.length} 看全部`))
  }
}
console.log(green('\n提示：知识应写回文档，不要回流进记忆——搜到就顺手更新那一处。\n'))
