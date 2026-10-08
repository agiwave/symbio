#!/usr/bin/env node
/**
 * doc-symbol-audit — 文档符号指认审计（D-005，全仓级，跨插件）
 *
 * 守的是「现行文档指认的 Rust 符号确实存在」。这类失效没人看得见：读者按图
 * 索骥找不到定义，而 doc-link-audit 只管链接、gen-current-facts 只从代码生成
 * 结构表，都不校验散文里的 `module::symbol`。判据在真实数据上校准过：77 篇
 * md 初版命中 8 条，逐条人工裁决后固定为下列规则。
 *
 * 判据：
 *   提取所有 *.md 中反引号包裹、以 `::` 分段且**每段为标识符**的路径
 *   （≥2 段，剥尾部 `()`），取最后一段查它是否以整词出现在 Rust 源码
 *   （symbio/src、cli/src、tauri/src-tauri/src）的 .rs 语料里；查不到 ⇒ ERROR。
 *
 * 为什么是这条判据：
 *   - 只查 ≥2 段：单段反引号词（`PayloadKey`、CLI 选项、配置键）与普通词
 *     无法区分，查了必然误报一片、最后被豁免喂到失效；
 *   - 只验最后一段：中段模块路径因 re-export 天生多解（顶层 `pub use *`、
 *     模块内 use），逐一解析必然误报；
 *   - 整词集合而非子串：`register_option` 不会误匹配 `register_option_field`。
 *
 * 豁免（都对应「文档说的是历史」）：
 *   1. `docs/DECISIONS.md`、`docs/archive/`、`docs/decisions/` 整体排除——
 *      ADR（索引与分域正文）与归档都是历史快照，当时的符号指认不随代码
 *      改名回改（回改反而篡改历史）；
 *   2. 行内含历史词（已删除 / 已退役 / 已废弃 / deprecated / 取代）⇒ 跳过，
 *      如迁移表里「已删除」的行；
 *   3. 承认通道：行内 `<!-- doc-symbol-allow: <理由> -->`，**理由不可为空**
 *      （空理由视为未承认，与 grep-audit / dead-code-audit 同一约定）。
 *
 * 失效形态（回归测试逐条钉住）：
 *   - 审计范围读不出来（没有 md 或没有 .rs 语料）却亮绿灯 ⇒ 必须 exit 1；
 *   - 判据写宽（注释 / 普通词 / DECISIONS / 单段词误报）⇒ 断言仍 exit 0；
 *   - 承认通道空理由不能放行。
 *
 * 用法： node scripts/doc-symbol-audit.mjs [--root=<仓库根>]
 * 退出码：0 = 通过；1 = 有失效指认（或审计范围异常）。
 *
 * 已知边界：只查 Rust 侧语料。外部 crate 符号若一次都没在本仓 .rs 出现过
 * 会被误报（当前全仓数据 0 条）——真出现时把文档改成本仓可见的完整指认，
 * 或用承认通道。
 */
// 判据码命名空间（登记表 docs/reference/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns D 文档正文与链接
// @codes D-005

import { readFileSync, readdirSync, existsSync } from 'node:fs'
import { join, dirname, resolve, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
// `--root=` 只为回归测试开的口子：测试在临时目录造一棵最小仓库，把判据用
// 「注入真实违规并断言变红」验证（判定型守卫的教条：只会亮绿灯 = 没有守卫）。
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? resolve(rootArg.slice(7)) : resolve(__dirname, '..')

/** Rust 语料根：任一缺失可容忍，全缺即「范围读不出」 */
const RS_ROOTS = ['symbio/src', 'cli/src', 'tauri/src-tauri/src']
/** walk 时整树跳过的目录 */
const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai'])
/** 整文件排除（历史快照，指认不随代码改名回改） */
const EXCLUDE_MD = new Set(['docs/DECISIONS.md'])
/** 整目录排除：ADR 正文分册与索引同源——ADR 记的是**当时**的设计，
 *  其中被指认的符号可能早已改名 / 删除，按现行代码判它会永久误报。 */
const EXCLUDE_MD_DIRS = ['docs/archive', 'docs/decisions']
/** 行内历史词：这行说的是过去，符号现在不存在是正常的 */
const HISTORY_RE = /已删除|已退役|已废弃|deprecated|取代/
/** 承认通道，理由非空才生效 */
const ALLOW_RE = /<!--\s*doc-symbol-allow:\s*([^>]*?)\s*-->/

let errors = 0
function report (file, line, msg) {
  errors++
  console.log(`${red('[ERROR]')} D-005 ${file}:${line}  ${msg}`)
}

/** 递归收集 md（相对路径），排除历史快照与垃圾目录 */
function collectMd (dir, prefix = '') {
  const out = []
  for (const ent of readdirSync(dir, { withFileTypes: true })) {
    const rel = prefix ? `${prefix}/${ent.name}` : ent.name
    if (ent.isDirectory()) {
      if (SKIP_DIRS.has(ent.name)) continue
      if (EXCLUDE_MD_DIRS.includes(rel)) continue
      out.push(...collectMd(join(dir, ent.name), rel))
    } else if (ent.name.endsWith('.md') && !EXCLUDE_MD.has(rel)) {
      out.push(rel)
    }
  }
  return out
}

/** 收集 .rs 语料（单个文件缺失容忍，全缺即异常） */
function collectRs (relRoot) {
  const abs = join(REPO_ROOT, relRoot)
  if (!existsSync(abs)) return []
  const out = []
  const walk = (dir) => {
    for (const ent of readdirSync(dir, { withFileTypes: true })) {
      if (ent.isDirectory()) {
        if (SKIP_DIRS.has(ent.name)) continue
        walk(join(dir, ent.name))
      } else if (ent.name.endsWith('.rs')) out.push(join(dir, ent.name))
    }
  }
  walk(abs)
  return out
}

// ── 主流程 ──────────────────────────────────────────────────────────────
console.log('--- doc-symbol-audit: 文档符号指认审计（D-005） ---')

const mdFiles = existsSync(REPO_ROOT) ? collectMd(REPO_ROOT).sort() : []
if (mdFiles.length === 0) {
  console.log(`${red('[ERROR]')} D-005 审计范围读不出：0 篇候选 md（${REPO_ROOT}）`)
  process.exit(1)
}

// Rust 语料 → 整词集合（一次构建，O(1) 查询）
const rsFiles = RS_ROOTS.flatMap((r) => collectRs(r))
if (rsFiles.length === 0) {
  console.log(`${red('[ERROR]')} D-005 审计范围读不出：0 个 .rs 语料根（${RS_ROOTS.join(', ')}）`)
  process.exit(1)
}
const idents = new Set()
const identRe = /[A-Za-z_][A-Za-z0-9_]*/g
for (const f of rsFiles) {
  for (const m of readFileSync(f, 'utf8').matchAll(identRe)) idents.add(m[0])
}

let scanned = 0
let refs = 0
for (const rel of mdFiles) {
  const lines = readFileSync(join(REPO_ROOT, rel), 'utf8').split(/\r?\n/)
  lines.forEach((line, i) => {
    const allow = line.match(ALLOW_RE)
    if (allow && allow[1].trim()) return // 承认通道：理由非空
    if (HISTORY_RE.test(line)) return // 这行说的是历史
    const tickRe = /`([^`\n]+)`/g
    let m
    while ((m = tickRe.exec(line)) !== null) {
      let inner = m[1].trim().replace(/\(\)$/, '')
      const segs = inner.split('::')
      if (segs.length < 2) continue // 单段不是符号指认（普通词 / CLI 选项）
      if (!segs.every((s) => /^[A-Za-z_][A-Za-z0-9_]*$/.test(s))) continue
      scanned++
      const sym = segs[segs.length - 1]
      if (!idents.has(sym)) {
        report(rel, i + 1, `指认的符号在 Rust 源码中不存在：\`${inner}\`——符号改名了，还是路径写错？`)
      } else {
        refs++
      }
    }
  })
}

if (errors > 0) {
  console.log(red(`\ndoc-symbol-audit 未通过：${errors} 处失效指认（扫描 ${scanned} 条候选，${mdFiles.length} 篇 md）`))
  process.exit(1)
}
console.log(green(`\ndoc-symbol-audit 通过：${mdFiles.length} 篇 md、${rsFiles.length} 个 .rs 语料，${refs} 条符号指认全部存在${dim(`（候选 ${scanned} 条）`)}`))
