#!/usr/bin/env node
/**
 * doc-symbol-audit — 文档符号指认审计（D-005 / D-009，全仓级，跨插件）
 *
 * 守的是「现行文档指认的符号确实存在」。这类失效没人看得见：读者按图
 * 索骥找不到定义，而 doc-link-audit 只管链接、gen-current-facts 只从代码生成
 * 结构表，都不校验散文里的 `module::symbol` 与裸常量名。判据在真实数据上校准过。
 *
 * 判据：
 *   **D-005**（`module::symbol` 形态）：提取所有 *.md 中反引号包裹、以 `::` 分段且
 *     **每段为标识符**的路径（≥2 段，剥尾部 `()`），取最后一段查它是否以整词出现在
 *     Rust 源码（symbio/src、cli/src、tauri/src-tauri/src）的 .rs 语料里；查不到 ⇒ ERROR。
 *   **D-009**（裸常量名形态）：反引号里的单段全大写名（形如 `ROUTE_SESSION_CHAT_SEND`，
 *     `^[A-Z][A-Z0-9]*(_[A-Z0-9]+)+$`）必须在 Rust ∪ 前端（tauri/src）语料里逐字存在——
 *     但**仅当它的同族**（首段 + `_`，如 `ROUTE_`）在代码里另有成员时才判红。
 *
 * 为什么 D-009 要「同族」这道闸：全大写 + 下划线不只是常量的形态，也是**环境变量、CI
 * 密钥、线上错误码**的形态（`APPLE_CERTIFICATE`、`GITHUB_TOKEN`）。那些名字的正确出处
 * 不在本仓代码里，硬查就得为每一处写豁免——而豁免喂到判据失效是这类守卫最容易的死法。
 * 同族把范围收成「本仓自己在用的常量命名空间」：改名漂移（文档写 `ROUTE_TRIAGE_DECIDE`、
 * 代码里实际叫 `ROUTE_CLASSIFY_DECIDE`）正好落在闸内，环境变量族自动落在闸外。
 * 族名只由**代码本体**开出（`gate.d/_shared.mjs` 的 `codeTextOf` 抹掉字符串内容与注释）：
 * `env!("CARGO_PKG_VERSION")` 里的名字是外部约定、注释里的名字是叙述，都不算这一族存在。
 *
 * 为什么两条判据的语料不同：`a::b::c` 是 Rust 语法，D-005 只查 Rust 侧；裸常量名是跨栈
 * 事实（前端 `VDFS_PAGE_SIZE` 与后端 `ROUTE_*` 一样会被文档指认），D-009 两侧都算存在。
 *
 * 为什么 D-005 只查 ≥2 段、只验最后一段：单段反引号词（`PayloadKey`、CLI 选项、配置键）
 *   与普通词无法区分，查了必然误报一片、最后被豁免喂到失效（D-009 例外：全大写 + 下划线
 *   这个形态本身可判别，见上）；中段模块路径因 re-export 天生多解（顶层 `pub use *`、
 *   模块内 use），逐一解析必然误报。整词集合而非子串：`register_option` 不会误匹配
 *   `register_option_field`。
 *
 * 豁免（都对应「文档说的不是现状」）：
 *   1. `docs/DECISIONS.md`、`docs/archive/`、`docs/decisions/` 整体排除——
 *      ADR（索引与分域正文）与归档都是历史快照，当时的符号指认不随代码
 *      改名回改（回改反而篡改历史）；
 *   2. 行内含历史词（已删除 / 已退役 / 已废弃 / deprecated / 取代 / 判为删除）⇒ 跳过，
 *      如迁移表里「已删除」的行、台账里「`X` 判为删除」的行；
 *   3. 承认通道：行内 `<!-- doc-symbol-allow: <理由> -->`，**理由不可为空**
 *      （空理由视为未承认，与 grep-audit / dead-code-audit 同一约定）。代码里查无此名
 *      有两种合法情形，理由要写明是哪一种：**历史**（「批⑤ 已判删」）或**目标态**
 *      （「09 的 S2 才落这个常量」——施工方案指认未来名字是本分，不该被当成漂移）。
 *
 * 失效形态（回归测试逐条钉住）：
 *   - 审计范围读不出来（没有 md 或没有 .rs 语料）却亮绿灯 ⇒ 必须 exit 1；
 *   - 判据写宽（注释 / 普通词 / DECISIONS / 单段词误报）⇒ 断言仍 exit 0；
 *   - 判据写窄（同族闸失效、引号里或注释里的名字开出了族）⇒ 断言仍 exit 0；
 *   - 承认通道空理由不能放行。
 *
 * 用法： node scripts/doc-symbol-audit.mjs [--root=<仓库根>]
 * 退出码：0 = 通过；1 = 有失效指认（或审计范围异常）。
 *
 * 已知边界：D-005 只查 Rust 侧语料，外部 crate 符号若一次都没在本仓 .rs 出现过会被误报；
 * D-009 只在同族存在时才判，一个**整族都不存在**的杜撰名（`ROUTE_` 全没了还写 `ROUTE_X`）
 * 落在闸外——那是结构事实，归 CURRENT.md 的生成表管。
 */
// 判据码命名空间（登记表 docs/reference/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns D 文档正文与链接
// @codes D-005 D-009

import { readFileSync, readdirSync, existsSync } from 'node:fs'
import { join, dirname, resolve, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'
import { codeTextOf } from './gate.d/_shared.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
// `--root=` 只为回归测试开的口子：测试在临时目录造一棵最小仓库，把判据用
// 「注入真实违规并断言变红」验证（判定型守卫的教条：只会亮绿灯 = 没有守卫）。
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? resolve(rootArg.slice(7)) : resolve(__dirname, '..')

/** Rust 语料根：任一缺失可容忍，全缺即「范围读不出」。`docs/plan/verify` 是硬证据程序
 *  的 Rust 源（`rustc` 直接编译的那批文件），它们声明的常量同样是文档指认的对象。 */
const RS_ROOTS = ['symbio/src', 'cli/src', 'tauri/src-tauri/src', 'docs/plan/verify']
/** D-009 的第二份语料：裸常量名是跨栈事实，前端声明的常量同样被文档指认 */
const FE_ROOTS = ['tauri/src']
/** 裸常量名的形态：全大写、至少一段下划线——单段大写词与普通词无法区分，不查 */
const CONST_SHAPE = /^[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+$/
/** walk 时整树跳过的目录。`.symbio` 是运行时产物：每起一个会话就复制一份
 *  `AGENTS.md` / `SKILL.md`，把它们算进「活跃文档」会让命中数无上界。 */
const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai', '.symbio'])
/** 整文件排除（历史快照，指认不随代码改名回改） */
const EXCLUDE_MD = new Set(['docs/DECISIONS.md'])
/** 整目录排除：ADR 正文分册与索引同源——ADR 记的是**当时**的设计，
 *  其中被指认的符号可能早已改名 / 删除，按现行代码判它会永久误报。 */
const EXCLUDE_MD_DIRS = ['docs/archive', 'docs/decisions']
/** 行内历史词：这行说的是过去，符号现在不存在是正常的。
 *  「判为删除 / 已判：删除」是台账（04 §3.1）对被删符号的固定说法——它和「已删除」
 *  是同一类叙述：这行明说那个名字已经不在了，按现行代码判它必然是误红。 */
const HISTORY_RE = /已删除|已退役|已废弃|deprecated|取代|判(?:为)?[：:]?\s*删除/
/** 承认通道，理由非空才生效 */
const ALLOW_RE = /<!--\s*doc-symbol-allow:\s*([^>]*?)\s*-->/

let errors = 0
function report (file, line, code, msg) {
  errors++
  console.log(`${red('[ERROR]')} ${code} ${file}:${line}  ${msg}`)
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

/** 收集某个语料根下的源文件（目录缺失容忍，全缺即「范围读不出」） */
function collectSource (relRoot, isSource) {
  const abs = join(REPO_ROOT, relRoot)
  if (!existsSync(abs)) return []
  const out = []
  const walk = (dir) => {
    for (const ent of readdirSync(dir, { withFileTypes: true })) {
      if (ent.isDirectory()) {
        if (SKIP_DIRS.has(ent.name)) continue
        walk(join(dir, ent.name))
      } else if (isSource(ent.name)) out.push(join(dir, ent.name))
    }
  }
  walk(abs)
  return out
}

const isRust = (n) => n.endsWith('.rs')
const isFrontend = (n) => /\.(ts|vue|js)$/.test(n)

// ── 主流程 ──────────────────────────────────────────────────────────────
console.log('--- doc-symbol-audit: 文档符号指认审计（D-005 / D-009） ---')

const mdFiles = existsSync(REPO_ROOT) ? collectMd(REPO_ROOT).sort() : []
if (mdFiles.length === 0) {
  console.log(`${red('[ERROR]')} D-005 审计范围读不出：0 篇候选 md（${REPO_ROOT}）`)
  process.exit(1)
}

// Rust 语料 → 整词集合（一次构建，O(1) 查询）
const rsFiles = RS_ROOTS.flatMap((r) => collectSource(r, isRust))
if (rsFiles.length === 0) {
  console.log(`${red('[ERROR]')} D-005 审计范围读不出：0 个 .rs 语料根（${RS_ROOTS.join(', ')}）`)
  process.exit(1)
}
const feFiles = FE_ROOTS.flatMap((r) => collectSource(r, isFrontend))

const identRe = /[A-Za-z_][A-Za-z0-9_]*/g
/** D-005 的语料：只有 Rust */
const idents = new Set()
/** D-009 的「逐字存在」：跨栈，引号里的名字也算写过（线上键值就是引号里的字符串） */
const codeNames = new Set()
/** D-009 的「同族」：抹掉字符串内容后再取标识符 ⇒ 只有代码自己写下的名字才开出命名空间 */
const families = new Map()

function indexRust (text) {
  for (const m of text.matchAll(identRe)) idents.add(m[0])
}
function indexCrossStack (raw) {
  for (const m of raw.matchAll(identRe)) codeNames.add(m[0])
  for (const m of codeTextOf(raw).matchAll(identRe)) {
    if (!CONST_SHAPE.test(m[0])) continue
    const fam = m[0].slice(0, m[0].indexOf('_') + 1)
    if (!families.has(fam)) families.set(fam, m[0])
  }
}

for (const f of rsFiles) {
  const text = readFileSync(f, 'utf8')
  indexRust(text)
  indexCrossStack(text)
}
for (const f of feFiles) indexCrossStack(readFileSync(f, 'utf8'))

let scanned = 0
let refs = 0
let constRefs = 0
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
      if (segs.length === 1) {
        // D-009：裸常量名
        if (!CONST_SHAPE.test(inner)) continue // 普通词 / CLI 选项 / 小写配置键
        scanned++
        if (codeNames.has(inner)) { constRefs++; continue }
        const fam = inner.slice(0, inner.indexOf('_') + 1)
        const sibling = families.get(fam)
        // 同族不存在 ⇒ 不是本仓的常量命名空间（环境变量、CI 密钥、外部约定），不判
        if (!sibling) continue
        report(rel, i + 1, 'D-009',
          `指认的常量名在代码中不存在：\`${inner}\`——同族有 \`${sibling}\`，这个名是改名了还是凭空写的？`)
        continue
      }
      if (!segs.every((s) => /^[A-Za-z_][A-Za-z0-9_]*$/.test(s))) continue
      scanned++
      const sym = segs[segs.length - 1]
      if (!idents.has(sym)) {
        report(rel, i + 1, 'D-005', `指认的符号在 Rust 源码中不存在：\`${inner}\`——符号改名了，还是路径写错？`)
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
console.log(green(`\ndoc-symbol-audit 通过：${mdFiles.length} 篇 md、${rsFiles.length} 个 .rs + ${feFiles.length} 个前端语料，${refs} 条符号指认与 ${constRefs} 个常量名全部存在${dim(`（候选 ${scanned} 条）`)}`))
