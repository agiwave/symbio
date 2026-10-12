#!/usr/bin/env node
/**
 * gen-doc-refs — 文档间引用只写编号，链接地址由机制生成与更新（RF-001 / RF-002 / RF-003）
 *
 * ## 它治的病
 *
 * 文档间的站内链接此前是**手写地址**：`[T-008](../task/T-008.md)`。地址里编着两件事——
 * **编号**（身份，稳定）与**相对路径**（位置，随目录调整而变）。手写它意味着每次搬家都要
 * 把全仓的入链改一遍，而不改的那几条会静默失效（`doc-link` D-001 能报出来，但那已经是
 * 事后追账）。本仓的真实教训就在眼前：docs 目录规范化（按 kind 分目录、消灭中间层）动了
 * 一百多篇文档的位置，入链数量是它们的数倍。
 *
 * 解法与 `gen-current-facts` 同一条约定：**能生成的地址不手写**。
 *
 *   - 人只写编号：`[[T-008]]`（可带显示文字 `[[T-008|接管协议那一条]]`、可带锚点
 *     `[[docs/adr/ADR-047#决策]]`、三者可组合）；
 *   - 地址由本脚本从**登记**（同一条 `loadEntries` 装载口径）解析出目标文件并写成
 *     `[T-008](../task/T-008.md)`——已展开的形式也由本脚本**每次重算**：
 *     编号不变则地址永远正确，文档搬家后自愈，不需要任何人改入链；
 *   - 带显示文字的引用把**编号写进链接标题**：`[接管协议那一条](../plan/14-登记与汇总机制.md "docs/plan/14-登记与汇总机制")`
 *     ——标题是 Markdown 标准语法（渲染成悬停提示），所以「给人看的文字」与「喂机制的编号」
 *     各占一个位置，可读性不牺牲，地址照样可重算。
 *   - 解析不出的编号（`[[不存在]]` / 编号拼错）⇒ **RF-001 判红**，不写盘——
 *     引用指向一个不存在的东西，只有在它被写下的那一刻最便宜。
 *   - **新写的手工地址**：地址指向某篇已登记文档、却既没有编号标签也没有编号标题 ⇒
 *     **RF-003 判红**（这正是不受机制的地址：搬走后没人改得动它）。改写方式两种，
 *     都在编号里选一个：`[[编号]]` 或 `[[编号|显示文字]]`。
 *
 * ## 三个方向的分工
 *
 * | 码 | 判什么 | 怎么处置 |
 * |---|---|---|
 * | **RF-001** | `[[编号]]` 解析不出目标（未登记 / 拼错 / 目标文件不在） | 判红，**不改任何文件**（改一半等于两份半截状态） |
 * | **RF-002** | 已展开的 `[编号](地址)` / `[文字](地址 "编号")` 里，地址 ≠ 该编号当前所在文件 | 本脚本自动改成当前地址（门禁 `60-facts` 的 autoWork 跑它并暂存） |
 * | **RF-003** | 地址指向已登记文档，但链接既不认编号标签也不认编号标题（手工地址） | 判红并要求写成 `[[编号]]` / `[[编号\|显示文字]]`（这种地址机制搬不动） |
 *
 * ## 不碰什么
 *
 * 链接**认不出编号**、且**指向的不是已登记文档**的一律不动（外链 / 图片 / 锚点 / 指向代码、
 * 脚本、归档与生成页的普通链接）：本机制只接管「登记条目的引用」，不是通用链接重写器。
 *
 * 用法：
 *   node scripts/gen-doc-refs.mjs            改写（展开 `[[编号]]` + 重算地址）
 *   node scripts/gen-doc-refs.mjs --check    只比对，漂移则非零退出（CI 用）
 *   node scripts/gen-doc-refs.mjs --root=<仓库根>   （回归测试夹具用）
 * 退出码：0 = 一致 / 已改写；1 = RF-001 / RF-003 有违规（不写盘）；2 = 参数不识别。
 */
// 判据码命名空间（登记表 docs/ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns RF 文档引用（编号 → 地址由机制生成）
// @codes RF-001 RF-002 RF-003

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
  '  编号解析不出即红且不写盘（RF-001）；指向已登记文档的手工地址判红（RF-003）。\n' +
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
 * 判据用「匹配起点是否落在代码段内」：`` [`T-002`](../task/T-002.md) `` 的链接起点在段外
 * （段只包住标签），仍按真引用重算地址；`` `[T-002](…)` `` 整条在段内，不动。
 */
function codeRanges(line) {
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
const inCode = (ranges, at) => ranges.some(([a, b]) => at >= a && at < b)

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

  const { entries, errors, missing } = loadEntries(root)
  const lines = ['--- gen-doc-refs: 引用只写编号（RF-001 / RF-002 / RF-003） ---']
  if (missing || entries.length === 0) {
    const msg = errors.length ? errors : ['登记范围读不出：扫不到任何已登记条目']
    return { code: 2, lines: lines.concat(msg.map((e) => red(`[ERROR] ${e}`)), red('[ERROR] 无条目可解析引用——不退化成「什么都没检查」')), changed: [] }
  }
  // 编号 → 条目、文件 → 条目（两张表都从**同一次装载**来：登记的真相只有一处）
  const byId = new Map()
  const byRel = new Map()
  for (const e of entries) {
    if (!e.meta) continue
    if (typeof e.meta.id === 'string' && !byId.has(e.meta.id)) byId.set(e.meta.id, e)
    if (!byRel.has(e.rel)) byRel.set(e.rel, e)
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
        const target = byId.get(id)
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
        const byLabel = byId.get(key)
        const byTitle = title ? byId.get(title) : undefined
        const entry = byTitle ?? byLabel
        if (!entry) {
          // 指向一篇已登记文档，却没有编号（标签与标题都不是编号）⇒ RF-003。
          // 例外：**自指**（目标就是本文）——它随文件一起搬，地址不会失效。
          const hit = byRel.get(resolveFrom(e.rel, target))
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
      for (const c of changed) lines.push(red(`[ERROR] RF-002 ${c} 的引用地址与编号不一致（重跑 node scripts/gen-doc-refs.mjs）`))
      lines.push(red(`gen-doc-refs 未通过（--check）：${changed.length} 个文件的引用需要重算`))
      return { code: 1, lines, changed }
    }
    lines.push(
      green(
        `gen-doc-refs 已改写 ${changed.length} 个文件（展开记号 ${tokensExpanded} 处 · 重算地址 ${addressesFixed} 处）`,
      ),
    )
  } else {
    lines.push(
      green(
        `gen-doc-refs 通过：引用地址与编号一致（核对 ${linksChecked} 条受管链接 / ${tokensExpanded} 个记号）`,
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
