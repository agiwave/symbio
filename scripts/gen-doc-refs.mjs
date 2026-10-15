#!/usr/bin/env node
/**
 * gen-doc-refs — 文档间引用只写编号，链接地址由机制生成与更新（RF-001 / RF-002 / RF-003 / RF-004）
 *
 * ## 它治的病
 *
 * 文档间的站内链接此前是**手写地址**：`[T-008](../009-task/008.md)`。地址里编着两件事——
 * **编号**（身份，稳定）与**相对路径**（位置，随目录调整而变）。手写它意味着每次搬家都要
 * 把全仓的入链改一遍，而不改的那几条会静默失效（`doc-link` D-001 能报出来，但那已经是
 * 事后追账）。本仓的真实教训就在眼前：docs 目录规范化（按 kind 分目录、消灭中间层）动了
 * 一百多篇文档的位置，入链数量是它们的数倍。
 *
 * 解法与 `facts/current` 同一条约定：**能生成的地址不手写**。
 *
 *   - 人只写编号：`[[T-008]]`（可带显示文字 `[[T-008|接管协议那一条]]`、可带锚点
 *     `[[docs/010-adr/ADR-047#决策]]`、三者可组合）；
 *   - 地址由本脚本从**登记**（同一条 `loadEntries` 装载口径）解析出目标文件并写成
 *     `[T-008](../009-task/008.md)`——已展开的形式也由本脚本**每次重算**：
 *     编号不变则地址永远正确，文档搬家后自愈，不需要任何人改入链；
 *   - 带显示文字的引用把**编号写进链接标题**：`[接管协议那一条](../004-plan/011-登记与汇总机制.md "docs/004-plan/PL-011-登记与汇总机制")`
 *     ——标题是 Markdown 标准语法（渲染成悬停提示），所以「给人看的文字」与「喂机制的编号」
 *     各占一个位置，可读性不牺牲，地址照样可重算。
 *   - 解析不出的编号（`[[不存在]]` / 编号拼错）⇒ **RF-001 判红**，不写盘——
 *     引用指向一个不存在的东西，只有在它被写下的那一刻最便宜。
 *   - **新写的手工地址**：地址指向某篇已登记文档、却既没有编号标签也没有编号标题 ⇒
 *     **RF-003 判红**（这正是不受机制的地址：搬走后没人改得动它）。改写方式两种，
 *     都在编号里选一个：`[[编号]]` 或 `[[编号|显示文字]]`。
 *
 * ## 源码注释也归本机制管（RF-004）
 *
 * 源码注释（`.rs` / `.ts` / `.mjs` / … 的注释行）里同样有大量指路链接——实测 319 条，
 * 其中 88 条已因文档搬家而静默失效。**同一处病灶不能两套待遇**：本脚本对源码注释做同样的
 * 展开与重算，差别只有一处——**手工地址不判红，而是自动接管**（RF-004）：
 *
 *   - 源码里的存量地址是一次性历史（300+ 条逐条手改正是「人工维护」本身），而只要登记表里
 *     能**唯一**识别出目标（dir 解析 / 根解析 / 尾缀匹配 / 唯一文件名），改写就是机械的；
 *   - 接管 = 补上编号标题并重算成**相对本文件**的地址，此后文档搬家由 RF-002 自愈；
 *   - 识别不出唯一目标（非登记文件 / 歧义）⇒ 不动，存在性交给 `doc-link-audit` 的 D-010；
 *   - 源码注释里的 `[[记号]]`：能解析的照常展开；**编号形状却解析不出**（前缀存在但整号
 *     未登记，如 `[[T-999]]`）⇒ RF-001 判红；`[[bin]]` / `[[false]]` 这类非编号字面量不动
 *     （源码注释天然充满方括号语法，照文档口径全判红会把守卫喂死）。
 *
 * 扫描口径（注释行、扩展名、跳过目录）导出给 `doc-link-audit` 共用——**审计判的行
 * 必须与改写的行是同一批**，两边各数一遍必然漂移。
 *
 * ## 四个方向的分工
 *
 * | 码 | 判什么 | 怎么处置 |
 * |---|---|---|
 * | **RF-001** | `[[编号]]` 解析不出目标（未登记 / 拼错 / 目标文件不在） | 判红，**不改任何文件**（改一半等于两份半截状态） |
 * | **RF-002** | 已展开的 `[编号](地址)` / `[文字](地址 "编号")` 里，地址 ≠ 该编号当前所在文件 | 本脚本自动改成当前地址（门禁 `60-facts` 的 autoWork 跑它并暂存） |
 * | **RF-003** | 地址指向已登记文档，但链接既不认编号标签也不认编号标题（手工地址，**文档正文**） | 判红并要求写成 `[[编号]]` / `[[编号\|显示文字]]`（这种地址机制搬不动） |
 * | **RF-004** | 手工地址指向已登记文档且**目标可唯一识别**（**源码注释**） | 自动接管：补编号标题 + 重算成相对本文件的地址（改完即受 RF-002 管） |
 *
 * ## 不碰什么
 *
 * 链接**认不出编号**、且**指向的不是已登记文档**的一律不动（外链 / 图片 / 锚点 / 指向代码、
 * 脚本、归档与生成页的普通链接）：本机制只接管「登记条目的引用」，不是通用链接重写器。
 * 源码侧另有一条：**只碰注释行**——代码行里的字符串与字面量是内容，不是入链。
 *
 * 用法：
 *   node scripts/gen-doc-refs.mjs            改写（展开 `[[编号]]` + 重算地址 + 接管源码地址）
 *   node scripts/gen-doc-refs.mjs --check    只比对，漂移则非零退出（CI 用）
 *   node scripts/gen-doc-refs.mjs --root=<仓库根>   （回归测试夹具用）
 * 退出码：0 = 一致 / 已改写；1 = RF-001 / RF-003 有违规（不写盘）；2 = 参数不识别。
 */
// 判据码命名空间（登记表 docs/006-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns RF 文档引用（编号 → 地址由机制生成）
// @codes RF-001 RF-002 RF-003 RF-004

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, dim } from './color.mjs'
import { loadEntries, relFrom } from './registry-lib.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')

const USAGE =
  '用法：node scripts/gen-doc-refs.mjs [--write] [--check] [--root=<仓库根>]\n' +
  '  [[编号]] / [[编号|显示文字]] / [[编号#锚点]] 展开成站内链接；已展开的地址按编号重算；\n' +
  '  源码注释里的手工地址按登记表唯一识别后自动接管（RF-004）；\n' +
  '  编号解析不出即红且不写盘（RF-001）；文档正文里指向已登记文档的手工地址判红（RF-003）。\n' +
  '  退出码：0 = 通过；1 = 违规 / --check 漂移；2 = 参数不识别'

/**
 * 引用记号：`[[编号]]` / `[[编号#锚点]]` / `[[编号|显示文字]]` / `[[编号#锚点|显示文字]]`。
 * 记号的形状是**唯一的**（双对方括号）——不会与正文里的单方括号链接撞车。
 * 第一段是编号（可带 `#锚点`），`|` 之后整段是给人看的显示文字（不参与身份判定）。
 */
export const TOKEN_RE = /\[\[([^\]\s|#]+)(#[^\]\s|]+)?(?:\|([^\]\n]+?))?\]\]/g
/**
 * 已展开的站内链接：`[编号](地址)` / `[显示文字](地址 "编号")`。
 * 地址不允许含空白（本仓文件名不含空格），标题是 Markdown 标准写法（单双引号都认）。
 */
const LINK_RE = /\[(`?)([^\]]+?)\1\]\(\s*([^)\s]+)(?:\s+"([^"]*)")?\s*\)/g

/**
 * 行内代码段的字符范围（`` `…` ``，含多重反引号）。
 *
 * 段内是**字面量**：机制文档自己必须能写 `[[编号]]` 与展开后的样子去讲规则，
 * 否则每写一次例子就被当成一次真引用（要么 RF-001 误红，要么例子被改得看不出原本写法）。
 * 判据用「匹配起点是否落在代码段内」：`` [`T-002`](../009-task/002.md) `` 的链接起点在段外
 * （段只包住标签），仍按真引用重算地址；`` `[T-002](…)` `` 整条在段内，不动。
 */
export function codeRanges(line) {
  const out = []
  for (let i = 0; i < line.length; ) {
    const bt = line.indexOf('`', i)
    if (bt === -1) break
    let n = 0
    while (line[bt + n] === '`') n += 1
    const close = line.indexOf('`'.repeat(n), bt + n)
    if (close === -1) break
    out.push([bt, close + n])
    i = close + n
  }
  return out
}
export const inCode = (ranges, at) => ranges.some(([a, b]) => at >= a && at < b)

/** 按行处理，跳过 ``` / ~~~ 围栏内的内容（示例代码里的链接不是引用）。 */
function mapOutsideFences(text, fn) {
  const lines = text.split('\n')
  let fence = null
  const out = lines.map((line) => {
    const f = line.match(/^\s*(```+|~~~+)/)
    if (f) {
      if (fence === null) fence = f[1][0]
      else if (f[1][0] === fence) fence = null
      return line
    }
    if (fence !== null) return line
    return fn(line)
  })
  return out.join('\n')
}

const isExternal = (t) => /^[a-z][a-z0-9+.-]*:/i.test(t)

/** 目标可被本机制接管吗（站内相对路径；外链 / 纯锚点 / 绝对路径不管） */
const isManagedTarget = (t) => t !== '' && !t.startsWith('#') && !t.startsWith('/') && !isExternal(t)

/** 仓库相对路径 → 该文档当前所在文件（登记的唯一真相） */
const resolveFrom = (fromRel, target) => {
  const dir = path.posix.dirname(fromRel)
  const have = target.split('#')[0]
  return path.posix.normalize(path.posix.join(dir === '.' ? '' : dir, have))
}

// ==================== 源码注释扫描口径（与 doc-link-audit 共用） ====================
/**
 * 源码扫描根（相对仓库根）。**只收仓库跟踪的源码树**：`docs/` 是 Markdown 的地盘，
 * 根目录散件没有注释形态的入链，`.symbio` / `.workbuddy*` / `tmp` 是本地产物。
 */
export const SRC_ROOTS = ['symbio', 'tauri', 'cli', 'scripts', 'e2e', 'examples', '.github']

/** 参与注释扫描的源码扩展名（`.md` 刻意不在：它由文档 pass 与 D-001 管）。 */
export const SRC_EXT = new Set([
  '.rs', '.ts', '.tsx', '.js', '.mjs', '.cjs', '.vue',
  '.yml', '.yaml', '.toml', '.sh', '.ps1', '.html', '.css',
])

/**
 * 递归时跳过的目录：依赖 / 产物 / **机器本地配置**。
 * `.cargo` 必须在列——根 `.gitignore` 用 `.cargo` 前缀通配把它挡在版本库外，
 * 扫描它等于让判定依赖「这台机器上恰好有什么」（不可复现），
 * 改写它更是把内容写进 git 看不见的文件。
 */
export const SRC_SKIP_DIRS = new Set([
  'node_modules', 'target', 'dist', 'build', 'vendor', '.git', '.venv', '.cargo',
])

/** `#` 注释族（yaml / toml / shell）；其余按 `//` `/*` `*` `<!--` 判。 */
const HASH_COMMENT_EXT = new Set(['.yml', '.yaml', '.toml', '.sh', '.ps1'])

/**
 * 该行是否为**注释行**——审计与改写共用的唯一口径（判的行与改的行必须是同一批）。
 * 判据是「去空白后以注释前缀起始」：行首即注释才算，行内尾注释不判
 * （`- run: x # see [y](z)` 这类把注释当内容的边界不值得猜）。
 */
export function isSourceCommentLine(line, ext) {
  const t = line.trim()
  if (HASH_COMMENT_EXT.has(ext)) return t.startsWith('#')
  if (t.startsWith('//') || t.startsWith('/*') || t.startsWith('*')) return true
  if ((ext === '.html' || ext === '.vue') && t.startsWith('<!--')) return true
  return false
}

/** 收集 `root` 下参与扫描的源码文件：`{file, rel, ext}`，`rel` 为仓根相对 posix 路径。 */
export function listSourceFiles(root) {
  const out = []
  const walk = (dir) => {
    let ents
    try {
      ents = fs.readdirSync(dir, { withFileTypes: true })
    } catch {
      return
    }
    for (const e of ents) {
      const abs = path.join(dir, e.name)
      if (e.isDirectory()) {
        if (!SRC_SKIP_DIRS.has(e.name)) walk(abs)
        continue
      }
      const ext = path.extname(e.name).toLowerCase()
      if (!SRC_EXT.has(ext)) continue
      const rel = path.relative(root, abs).split(path.sep).join('/')
      if (rel.startsWith('docs/archive/')) continue // 归档是历史快照（同 md 口径）
      out.push({ file: abs, rel, ext })
    }
  }
  for (const r of SRC_ROOTS) walk(path.join(root, r))
  return out
}

/**
 * 源码注释里的手工地址 → 登记条目（**必须唯一**，否则返回 null 不动）。
 * 四级解析：相对本文件 → 相对仓库根（剥前导 `../`）→ 地址尾缀含仓根相对路径
 * （治「`../` 层数写错」这一最大死链类）→ 文件名在登记表里唯一。
 */
function matchSourceEntry(rel, target, byRel) {
  const have = target.split('#')[0]
  const dirHit = resolveFrom(rel, have)
  if (byRel.has(dirHit)) return byRel.get(dirHit)
  const norm = path.posix.normalize(have)
  if (byRel.has(norm)) return byRel.get(norm)
  const stripped = path.posix.normalize(have.replace(/^(\.\.\/)+/, ''))
  if (byRel.has(stripped)) return byRel.get(stripped)
  const suffixHits = [...byRel.keys()].filter((r) => have.endsWith('/' + r))
  if (suffixHits.length === 1) return byRel.get(suffixHits[0])
  if (suffixHits.length > 1) return null // 多个登记文件共享尾缀 ⇒ 歧义，不猜
  const base = path.posix.basename(have)
  const baseHits = [...byRel.keys()].filter((r) => path.posix.basename(r) === base)
  if (baseHits.length === 1) return byRel.get(baseHits[0])
  return null
}

/**
 * CLI 入口（导出供回归测试直接调用，不 spawn）。
 * @param {string[]} argv 不含 node 与脚本路径的 argv 片段
 * @returns {{code: 0|1|2, lines: string[], changed: string[]}}
 */
export function runGenDocRefs(argv) {
  let root = defaultRoot
  let check = false
  for (const a of argv) {
    if (a === '--check') check = true
    else if (a === '--write') continue // 默认即改写；--write 是显式同义词（门禁 autoWork 用）
    else if (a === '--strict') continue // 门禁对所有 GUARDS 传 --strict；本脚本无 WARNING 档
    else if (a.startsWith('--root=')) root = path.resolve(a.slice(7))
    else return { code: 2, lines: [red(`[ERROR] 参数不识别：${a}`), USAGE], changed: [] }
  }

  const { entries, kinds, errors, missing } = loadEntries(root)
  const lines = ['--- gen-doc-refs: 引用只写编号（RF-001 / RF-002 / RF-003 / RF-004） ---']
  if (missing || entries.length === 0) {
    const msg = errors.length ? errors : ['登记范围读不出：扫不到任何已登记条目']
    return { code: 2, lines: lines.concat(msg.map((e) => red(`[ERROR] ${e}`)), red('[ERROR] 无条目可解析引用——不退化成「什么都没检查」')), changed: [] }
  }
  // 编号 → 条目、文件 → 条目（两张表都从**同一次装载**来：登记的真相只有一处）。
  // 编号有两种合法写法：**全 id**（`T-121-最小闭环`）与**裸编号**（`T-121`，前缀+序号，
  // 名称段可省）——引用规范不变（`[[T-121|...]]` 依旧合法），所以裸编号也要能解析。
  const byId = new Map()
  const byRel = new Map()
  for (const e of entries) {
    if (!e.meta) continue
    if (typeof e.meta.id === 'string' && !byId.has(e.meta.id)) byId.set(e.meta.id, e)
    if (!byRel.has(e.rel)) byRel.set(e.rel, e)
  }
  // 裸编号索引：`<前缀>-<三位序号>` → 条目。冲突（同前缀同序号两条）⇒ 该键不可用，
  // 引用必须写全 id——装 two 条目进同一键会让后写者赢，宁可解析不出（RF-001 报错）
  // 也不静默指错。
  const byShort = new Map()
  for (const e of entries) {
    const m = typeof e.meta?.id === 'string' ? e.meta.id.match(/^(.+)-\d{3}[a-z]?/) : null
    if (m && !byShort.has(m[1])) byShort.set(m[1], e)
    else if (m && byShort.get(m[1]) !== e) byShort.set(m[1], undefined)
  }
  for (const [k, v] of [...byShort]) if (v === undefined) byShort.delete(k)
  /** 引用解析：全 id 优先，裸编号兜底（歧义键已剔除）。 */
  const resolveRef = (id) => byId.get(id) ?? byShort.get(id)
  /** 源码 / 手工地址按登记表识别：四级路径解析 + 文件主干 = 全 id 的一级（旧路径迁移期自愈）。 */
  const resolvePath = (rel, target) => {
    const hit = matchSourceEntry(rel, target, byRel)
    if (hit) return hit
    const base = path.posix.basename(target.split('#')[0]).replace(/\.md$/, '')
    return base ? byId.get(base) : undefined
  }

  const bad = []
  const changed = []
  const pending = []
  let linksChecked = 0
  let tokensExpanded = 0
  let addressesFixed = 0

  for (const e of entries) {
    if (!e.meta) continue // 解析失败的条目由 registry-audit 报，这里不重复
    const before = fs.readFileSync(e.file, 'utf8')
    let after = mapOutsideFences(before, (line) => {
      // ① 记号 → 已展开的链接（人只写编号的那一半）
      const code1 = codeRanges(line)
      let out = line.replace(TOKEN_RE, (m, id, frag, label, off) => {
        if (inCode(code1, off)) return m
        const ref = `${id}${frag ?? ''}`
        const target = resolveRef(id)
        if (!target) {
          bad.push(`${e.rel}：RF-001 引用了未登记的编号 \`[[${ref}${label ? `|${label}` : ''}]]\`（拼错？还是目标还没登记？）`)
          return m
        }
        tokensExpanded += 1
        const addr = `${relFrom(path.posix.dirname(e.rel) === '.' ? '' : path.posix.dirname(e.rel), target.rel)}${frag ?? ''}`
        return label ? `[${label}](${addr} "${id}")` : `[${id}](${addr})`
      })
      // ② 已展开的链接：地址按编号重算（自愈——文档搬家后无需任何人改入链），
      //    并挑出**没有编号的手工地址**（RF-003：这种地址机制搬不动）
      const code2 = codeRanges(out)
      out = out.replace(LINK_RE, (m, _tick, label, target, title, off) => {
        if (inCode(code2, off)) return m
        if (!isManagedTarget(target)) return m
        const key = label.replace(/`/g, '').trim()
        const byLabel = resolveRef(key)
        const byTitle = title ? resolveRef(title) : undefined
        const entry = byTitle ?? byLabel
        if (!entry) {
          // 指向一篇已登记文档，却没有编号（标签与标题都不是编号）⇒ RF-003。
          // 例外：**自指**（目标就是本文）——它随文件一起搬，地址不会失效。
          // 目标识别走两级：路径直达，或文件主干 = 某条目全 id（改名迁移期）。
          const hit = byRel.get(resolveFrom(e.rel, target)) ?? resolvePath(e.rel, target)
          if (hit && hit.meta.id !== e.meta.id) {
            bad.push(
              `${e.rel}：RF-003 手工地址 \`[${key}](${target})\` 指向已登记文档 \`${hit.meta.id}\`——` +
                `地址不会被重算（文档搬家后它静默失效）；改写成被机制接管的两种之一：\`[[${hit.meta.id}]]\` 或 \`[[${hit.meta.id}|${key}]]\``,
            )
          }
          return m
        }
        linksChecked += 1
        const dir = path.posix.dirname(e.rel)
        const want = relFrom(dir === '.' ? '' : dir, entry.rel)
        const have = target.split('#')[0]
        const frag = target.includes('#') ? target.slice(target.indexOf('#')) : ''
        // 身份载体的规范形态：标签是编号 ⇒ 不加标题；标题带编号（显示文字自由）⇒ 保留标题
        const carry = byTitle ? `${want}${frag} "${title}"` : `${want}${frag}`
        // doc-link-allow D-010: 下一行是本条判据讲的路径形态示例（`a.md` 是占位名）
        // 已经解析到同一个文件 ⇒ 不动（避免把 `./a.md` 与 `a.md` 这类等价写法来回改）
        if (path.posix.normalize(path.posix.join(dir, have)) === entry.rel) return m
        addressesFixed += 1
        return `[${_tick}${label}${_tick}](${carry})`
      })
      return out
    })
    if (after !== before) {
      changed.push(e.rel)
      pending.push({ file: e.file, after })
    }
  }

  // ---- 源码注释 pass（RF-004）：同一批注释行里做展开 / 重算 / 接管 ----
  // 与 md pass 共用 `bad` / `changed` / `pending`：写盘决策仍然是「有任何 RF-001 / RF-003
  // 就一个文件都不改」，源码的红同样拦在写盘之前。
  let srcChecked = 0
  let srcTakeovers = 0
  let srcTokens = 0
  const srcTakeoverRels = new Set()
  // 已知编号前缀（`T` / `ADR` / `SP` …）：源码里**形如编号却解析不出** ⇒ 拼错，RF-001 判红；
  // 没有前缀的字面量（`[[bin]]` / `[[false]]`）不判——源码注释天然充满方括号语法。
  // 前缀闭集从 kind 装载来（各 README meta 的 `prefix`），不写死。
  const knownIdPrefixes = new Set(Object.values(kinds).map((k) => k.prefix))
  for (const { file, rel, ext } of listSourceFiles(root)) {
    const before = fs.readFileSync(file, 'utf8')
    const after = before
      .split('\n')
      .map((line, li) => {
        if (!isSourceCommentLine(line, ext)) return line // 只碰注释行：代码里的字符串是内容
        // ① 记号：能解析的展开；编号形状解析不出 ⇒ RF-001；其余字面量不动
        const code1 = codeRanges(line)
        let out = line.replace(TOKEN_RE, (m, id, frag, label, off) => {
          if (inCode(code1, off)) return m
          const target = resolveRef(id)
          if (!target) {
            const prefix = id.includes('-') ? id.slice(0, id.lastIndexOf('-')) : ''
            if (prefix && knownIdPrefixes.has(prefix)) {
              bad.push(
                `${rel}:${li + 1}：RF-001 源码注释引用了未登记的编号 \`[[${id}${frag ?? ''}]]\`（拼错？还是目标还没登记？）`,
              )
            }
            return m
          }
          srcTokens += 1
          const addr = `${relFrom(path.posix.dirname(rel) === '.' ? '' : path.posix.dirname(rel), target.rel)}${frag ?? ''}`
          return label ? `[${label}](${addr} "${id}")` : `[${id}](${addr})`
        })
        // ② 链接：已带编号 ⇒ 重算（RF-002）；不带编号但目标可唯一识别 ⇒ 接管（RF-004）
        const code2 = codeRanges(out)
        out = out.replace(LINK_RE, (m, _tick, label, target, title, off) => {
          if (inCode(code2, off)) return m
          if (!isManagedTarget(target)) return m
          const key = label.replace(/`/g, '').trim()
          const byTitle = title ? resolveRef(title) : undefined
          let entry = byTitle ?? resolveRef(key)
          let takeover = false
          if (!entry) {
            entry = resolvePath(rel, target)
            if (!entry) return m // 非登记目标 / 歧义：不动，存在性交给 D-010
            if (title) return m // 已有非编号标题：不猜人的意图，留给人工
            takeover = true
          }
          srcChecked += 1
          const dir = path.posix.dirname(rel)
          const want = relFrom(dir === '.' ? '' : dir, entry.rel)
          const have = target.split('#')[0]
          const frag = target.includes('#') ? target.slice(target.indexOf('#')) : ''
          if (takeover) {
            srcTakeovers += 1
            srcTakeoverRels.add(rel)
            return `[${_tick}${label}${_tick}](${want}${frag} "${entry.meta.id}")`
          }
          // doc-link-allow D-010: 下一行是本条判据讲的路径形态示例（`a.md` 是占位名）
          // 已经解析到同一个文件 ⇒ 不动（`./a.md` 与 `a.md` 这类等价写法不来回改）
          if (path.posix.normalize(path.posix.join(dir, have)) === entry.rel) return m
          addressesFixed += 1
          const carry = byTitle ? `${want}${frag} "${title}"` : `${want}${frag}`
          return `[${_tick}${label}${_tick}](${carry})`
        })
        return out
      })
      .join('\n')
    if (after !== before) {
      changed.push(rel)
      pending.push({ file, after })
    }
  }

  if (bad.length) {
    for (const b of bad) lines.push(red(`[ERROR] ${b}`))
    lines.push(
      red(
        `gen-doc-refs 未通过：${bad.length} 处引用不受机制接管（未写任何文件——改一半等于两份半截状态）`,
      ),
    )
    return { code: 1, lines, changed: [] }
  }

  // 写盘放在**全部解析之后**：RF-001 / RF-003 命中时一个文件也不改（改一半 = 两份半截状态）
  if (!check) for (const p of pending) fs.writeFileSync(p.file, p.after)

  if (changed.length) {
    if (check) {
      for (const c of changed) {
        if (srcTakeoverRels.has(c)) {
          lines.push(red(`[ERROR] RF-004 ${c} 的源码注释地址待接管 / 重算（重跑 node scripts/gen-doc-refs.mjs）`))
        } else {
          lines.push(red(`[ERROR] RF-002 ${c} 的引用地址与编号不一致（重跑 node scripts/gen-doc-refs.mjs）`))
        }
      }
      lines.push(red(`gen-doc-refs 未通过（--check）：${changed.length} 个文件的引用需要重算`))
      return { code: 1, lines, changed }
    }
    lines.push(
      green(
        `gen-doc-refs 已改写 ${changed.length} 个文件（展开记号 ${tokensExpanded} 处 · 重算地址 ${addressesFixed} 处 · 源码注释：接管 ${srcTakeovers} 处 · 重算 ${srcChecked - srcTakeovers} 处 · 记号 ${srcTokens} 处）`,
      ),
    )
  } else {
    lines.push(
      green(
        `gen-doc-refs 通过：引用地址与编号一致（核对 ${linksChecked} 条受管链接 / ${tokensExpanded} 个记号 · 源码注释 ${srcChecked} 条 / 记号 ${srcTokens} 个）`,
      ),
    )
  }
  for (const c of changed.slice(0, 12)) lines.push(dim(`   · ${c}`))
  return { code: 0, lines, changed }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { code, lines } = runGenDocRefs(process.argv.slice(2))
  for (const l of lines) console.log(l)
  process.exit(code)
}
