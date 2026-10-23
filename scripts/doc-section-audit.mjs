#!/usr/bin/env node
/**
 * doc-section-audit — 小节的**地址**必须可寻址、引用必须指得到（DS-001…002）
 *
 * ## 它治的病
 *
 * 本仓用 `§N` / `§N.M` 当小节的**地址**：设计文档之间互相指「见 DE-001 §13.4」，
 * 同一篇里指「见 §2.4」。这个寻址体系此前**没有任何东西校验**——`doc-link-audit`
 * 判的是 Markdown 链接（`[文字](地址)`），`doc-symbol-audit` 判的是反引号里的
 * Rust 符号，两者都不看 `§`。于是发生了两件事，且都在门禁全绿的情况下发生：
 *
 * 1. **编号重复**：`016-工程落地与验证.md` 的一级小节依次是
 *    `1,2,3,4,5,7,1,2,4,2,4,5` —— 三篇文档被并进一个文件，各自从 1 开始编号。
 *    读者看到「§2」时无法知道指哪一个，**地址本身失去了指认能力**。
 * 2. **引用悬空**：`DE-001` 的小节是 `0…12,14`，没有 §13；而全仓有 6 处正文
 *    在指 `DE-001 §13.2 / §13.3 / §13.4 / §13.5`。它们指的内容早已被删或搬走。
 *
 * 这两件事的共同点是**读者看得见、守卫看不见**：一句「详见 DE-001 §13.4」读起来
 * 完全正常，点进去却什么都没有——比一句坏链接更坏，因为它连「点」都没有。
 *
 * ## 判据
 *
 * | 码 | 判什么 | 为什么 |
 * |---|---|---|
 * | DS-001 | 同一篇内一级小节编号**唯一**且**严格递增** | 重复 ⇒ 地址不唯一；回退 ⇒ 两篇拼在一处 |
 * | DS-002 | `§N` / `§N.M` / `§N.M.K` 引用**必须解析得到** | 悬空引用 = 指认失效 |
 *
 * ### 为什么**不判「连续」**（这是本判据最重要的一条取舍）
 *
 * 最初的设想是「编号必须 0,1,2,… 连续无缺号」，落地时否掉了。理由是**缺号是删除的
 * 留痕，重编号会让地址漂移**：
 *
 * - 缺号本身无害——`DE-001` 缺 §13，没有任何引用指 §14 之外的东西，读者读
 *   `12 → 14` 只是知道「这里曾有一节被拿掉了」。
 * - 若为了「连续」把 §14 改成 §13，那么 6 处指向**已删除的** §13.2 / §13.4 的引用
 *   会**突然解析成功**，而且指到「边界：本架构明确不做」这一节——**从悬空变成指错**。
 *   悬空能被 DS-002 判红，指错不能。**要求连续等于把可判的缺陷换成不可判的缺陷。**
 * - 地址一旦发布就该稳定：`§13` 被别的文档引用过，它就该永远指同一件事，或者不存在。
 *   重编号等于让同一个地址先后指两件事，这正是版本控制里最贵的一种破坏。
 *
 * 所以 DS-001 只判**唯一**与**递增**：这两条错了，地址体系就已经崩了；缺号不判。
 * `DE-001` 缺 §13 这件事**仍然被判红**——不是被 DS-001，而是被 DS-002（6 处引用
 * 指不到它）。判据落在「引用指不到」而不是「编号有缺」上，红的理由才是真理由。
 *
 * ### 什么算一个「可寻址的小节」
 *
 * 标题的编号**必须与父标题的地址相容**才登记为地址：
 *
 * - `## 2. 横向`（一级、单号）⇒ 地址 `2`；
 * - `### 2.1 为什么` ⇒ 前缀 `2` 已存在 ⇒ 地址 `2.1`；
 * - `#### 3.1.1 外部效标` ⇒ 前缀 `3.1` 已存在 ⇒ 地址 `3.1.1`。
 *   多号标题**不看标题级别**：本仓既写 `## 2` + `### 2.1` 的嵌套，也写
 *   `### 3.2` + `### 3.2.1` 的同级平铺（编号分层、层级扁平）。判据只问
 *   「前缀地址是否已经出现」，两种写法都成立；若按「父标题必须是上一级」判，
 *   平铺写法里的 `3.2.1` 会被判成非地址，于是指它的悬空引用**静默漏判**；
 * - `### 1. 自相似性`（二级以下、单号）⇒ **不是地址**。它在 `## 核心设计哲学`
 *   这样的小节里从 1 数起，指 `§1` 指的是这篇文档的 §1，不是它；
 * - `### 17 场景 × 单环形状` ⇒ 前缀 `17` 不存在 ⇒ **不是地址**；
 * - `### 机制 1：…` / `### Q1 …` / `### D-059 …` 开头没有编号 ⇒ 不是地址。
 *
 * 少了「前缀必须存在」这一层，`### 1. 自相似性` 会被登记成 `§1`，于是正文里指
 * 「§1」的悬空引用会被它**冒充解析成功**——守卫变绿，缺陷还在。这是本判据最危险
 * 的失效形态，回归测试专门钉它。
 *
 * ### 引用的归属：靠**紧邻的文档锚点**，不靠猜
 *
 * 一句 `见 [DE-010](010-VDFS前端页面规范.md) §13.4` 里的 `§13.4` 属于 DE-010。
 * 归属规则是**同一行内、引用位置之前最近的文档锚点**：
 *
 * - Markdown 链接：取 href 指向的 `.md`（相对本文件解析，必须真实存在）；
 * - 裸编号：`DE-001` / `RE-006` 这类，按各文档 front-matter 的 `id` 前缀反查。
 *
 * 没有前置锚点 ⇒ 属于本文件。**目标文档若一个小节编号都没有**（场景 / 规格 /
 * 任务类文档不用 `§N` 寻址）⇒ 跳过，不判——判它只会制造一片误报，而天天误报的
 * 守卫最后只会被豁免喂死。
 *
 * 行内代码（`` `§7.2` ``）里的 `§` 不算引用：**引用一处地址**与**引用一句话**
 * 是两件事，后者是数据（任务文档、ADR 正文大量这么写）。
 *
 * 用法：node scripts/doc-section-audit.mjs [--root=<仓库根>] [--strict]
 * 退出码：0 = 通过；1 = 有违规；2 = 审计范围读不出（不退化成「什么都没检查」）
 */
// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns DS 文档小节寻址（一级编号唯一递增 + §N 引用可解析）
// @codes DS-001 DS-002

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? path.resolve(rootArg.slice(7)) : path.resolve(scriptDir, '..')

const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai', '.symbio', 'archive'])

/** 活跃文档：`docs/` 下的 `*.md`（跳过 archive 与垃圾目录） */
function activeDocs(root) {
  const out = []
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      if (ent.isDirectory()) {
        if (!SKIP_DIRS.has(ent.name)) walk(path.join(dir, ent.name))
        continue
      }
      if (ent.name.endsWith('.md')) out.push(path.join(dir, ent.name))
    }
  }
  walk(path.join(root, 'docs'))
  return out.sort()
}

/** front-matter 的 `id` 短号（`DE-001`）→ 文件，用于解析裸编号锚点 */
function idIndex(docs) {
  const map = new Map()
  for (const f of docs) {
    const txt = fs.readFileSync(f, 'utf8')
    const fm = txt.match(/^---\r?\n([\s\S]*?)\r?\n---/)
    if (!fm) continue
    const idm = fm[1].match(/^id:\s*(\S+)/m)
    if (!idm) continue
    const short = idm[1].match(/^([A-Z]+-\d{3})/)
    if (short && !map.has(short[1])) map.set(short[1], f)
  }
  return map
}

/** 标题里开头的编号路径：`§1` / `0.` / `2.1` / `2.1.1`；没有则 null */
function headingNumber(text) {
  const m = text.trim().match(/^§?\s*(\d+(?:\.\d+)*)[.．]?(?:\s|$)/)
  return m ? m[1] : null
}

/**
 * 解析一篇文档的地址集合与一级编号序列。
 *
 * `addr` 的相容性规则见文件头「什么算一个『可寻址的小节』」：
 * 单号标题必须是 `##`（一级）；多号标题要求**前缀地址已经存在**（不看级别，
 * 因为本仓把 `### 3.2` 与 `### 3.2.1` 平铺写成同级）。
 *
 * 返回值：`{ sections:Set<string>, topLevel:number[] }`
 */
function parseDoc(file) {
  const lines = fs.readFileSync(file, 'utf8').split(/\r?\n/)
  const sections = new Set()
  const topLevel = []
  let fence = false
  for (const line of lines) {
    if (/^\s*```/.test(line)) {
      fence = !fence
      continue
    }
    if (fence) continue
    const h = line.match(/^(#{2,6})\s+(.*)$/)
    if (!h) continue
    const level = h[1].length
    const num = headingNumber(h[2])
    if (!num) continue
    let addr = null
    if (!num.includes('.')) {
      // 单号只有一级标题才是地址：二级以下的「1. 自相似性」是节内自数
      if (level === 2) addr = num
    } else if (sections.has(num.slice(0, num.lastIndexOf('.')))) {
      addr = num
    }
    if (!addr) continue
    sections.add(addr)
    if (level === 2 && !addr.includes('.')) topLevel.push(Number(addr))
  }
  return { sections, topLevel }
}

/** 一行里的文档锚点（链接 href + 裸编号），按出现位置排序 */
function anchorsOf(line, file, byId) {
  const out = []
  for (const m of line.matchAll(/\[[^\]]*\]\(\s*([^)\s]+?\.md)(?:#[^)\s]*)?(?:\s+"[^"]*")?\s*\)/g)) {
    let href
    try {
      href = decodeURIComponent(m[1])
    } catch {
      continue
    }
    const abs = path.resolve(path.dirname(file), href)
    if (fs.existsSync(abs)) out.push({ at: m.index, file: abs })
  }
  for (const m of line.matchAll(/\b([A-Z]{2}-\d{3})\b/g)) {
    const t = byId.get(m[1])
    if (t) out.push({ at: m.index, file: t })
  }
  return out.sort((a, b) => a.at - b.at)
}

/** 去掉行内代码：`` `§7.2` `` 是引用**一句话**，不是引用一处地址 */
const stripInlineCode = (s) => s.replace(/`[^`]*`/g, (m) => ' '.repeat(m.length))

console.log('--- doc-section-audit: 小节寻址（DS-001…002） ---')

const docs = activeDocs(REPO_ROOT)
if (docs.length === 0) {
  console.log(red('[ERROR] DS-001 审计范围读不出：`docs/` 下没有活跃文档 ⇒ 不退化成「什么都没检查」'))
  process.exit(2)
}
const byId = idIndex(docs)
const parsed = new Map(docs.map((f) => [f, parseDoc(f)]))
const addressable = docs.filter((f) => parsed.get(f).sections.size > 0)
if (addressable.length === 0) {
  console.log(red('[ERROR] DS-001 审计范围读不出：没有任何文档登记到小节地址 ⇒ 提取器可能坏了'))
  process.exit(2)
}

let errors = 0

// ── DS-001：一级小节编号唯一且严格递增 ─────────────────────────────────
for (const f of addressable) {
  const { topLevel } = parsed.get(f)
  const rel = path.relative(REPO_ROOT, f)
  const seen = new Set()
  for (let i = 0; i < topLevel.length; i++) {
    const n = topLevel[i]
    if (seen.has(n)) {
      errors++
      console.log(red(`[ERROR] DS-001 ${rel}:§${n} 编号重复 ⇒ 地址不唯一，「§${n}」指不出是哪一节`))
    }
    seen.add(n)
    if (i > 0 && n <= topLevel[i - 1]) {
      errors++
      console.log(
        red(
          `[ERROR] DS-001 ${rel}:§${topLevel[i - 1]} → §${n} 编号回退 ⇒ 疑似两篇文档并进一个文件` +
            `（编号各自从 1 起）`,
        ),
      )
    }
  }
}

// ── DS-002：`§N` / `§N.M` 引用必须解析得到 ────────────────────────────
const REF = /§\s?(\d+(?:\.\d+){0,2})/g
let refTotal = 0
let refJudged = 0
let refSkipped = 0
for (const f of docs) {
  const lines = fs.readFileSync(f, 'utf8').split(/\r?\n/)
  const rel = path.relative(REPO_ROOT, f)
  let fence = false
  for (let i = 0; i < lines.length; i++) {
    if (/^\s*```/.test(lines[i])) {
      fence = !fence
      continue
    }
    if (fence || !lines[i].includes('§')) continue
    const line = stripInlineCode(lines[i])
    const anchors = anchorsOf(line, f, byId)
    for (const m of line.matchAll(REF)) {
      refTotal++
      const num = m[1]
      let target = f
      for (const a of anchors) if (a.at < m.index) target = a.file
      const t = parsed.get(target)
      // 目标文档不用 `§N` 寻址 ⇒ 不判（场景 / 规格 / 任务文档）
      if (!t || t.sections.size === 0) {
        refSkipped++
        continue
      }
      refJudged++
      if (t.sections.has(num)) continue
      errors++
      const where = target === f ? '本文件' : path.basename(target)
      console.log(
        red(`[ERROR] DS-002 ${rel}:${i + 1} §${num} 指不到 ⇒ ${where}没有这一节（引用悬空）`),
      )
      console.log(`       ${lines[i].trim().slice(0, 100)}`)
    }
  }
}

if (errors) {
  console.log(
    `doc-section-audit 未通过：${errors} 处（DS-001 编号 / DS-002 引用）；` +
      `扫 ${docs.length} 份文档 / ${refTotal} 处 § 引用（可判 ${refJudged} · 跳过 ${refSkipped}）`,
  )
  process.exit(1)
}

console.log(
  green(
    `doc-section-audit 通过：${addressable.length} 份文档有编号小节，编号唯一且递增；` +
      `${refJudged} 处 § 引用全部解析得到（另 ${refSkipped} 处指向不用 §N 寻址的文档，不判）`,
  ),
)
