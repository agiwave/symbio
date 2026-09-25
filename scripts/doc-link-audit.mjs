#!/usr/bin/env node
/**
 * doc-link-audit — 文档相对链接审计
 *
 * 用途：**活跃文档体检**——四条机械可判定的规矩，每条都对应一类「没人看着就必然腐烂」的文档病：
 *
 *   D-001 站内相对链接：文档移动 / 归档（`git mv`）最容易留下静默坏链——阅读时才发现，
 *     而它本可以在提交前被机械地查出来。
 *   D-002 过程文档必须归档：靠文档**头部自述**判定（一次性评审 / 体检 / 已落地实施方案）。
 *   D-003 行数预算：活跃文档 **不得超过 `MAX_DOC_LINES`**。
 *   D-004 变更史不得混入活跃文档正文：历史归 `git log` 与 `archive/`。
 *
 * D-003 存在的理由（为什么是「行数」这个粗指标）：文档臃肿不是美学问题，而是**职责失守的
 *   可观测代理**。实测 `docs/design/vdfs.md` 涨到 1045 行时，超出的部分是 §13「范例」——
 *   它自述「非机制组成部分」，内容却与四个模块 README 逐段重复；`DECISIONS.md` 涨到 1782 行时，
 *   超出的部分是变更史与实施过程记录。两处的病灶不同，但都表现为「比它该有的大」。
 *   行数因此是一个**代价极低、不会误报**（不设"疑似"档）的触发器：超了就必须拆分，
 *   而拆分的方向由 docs/README.md 的 owner 表给出。**不给豁免**：>800 行不是需要解释的
 *   特例，而是一个明确信号——要么归给别的 owner，要么拆成两篇。
 *
 * D-004 存在的理由：现行文档写「曾经是什么、后来改成了什么」，代价不是"多几句话"，而是
 *   **每次重构都要回头改历史**——历史既不可能与现状保持一致，改了又等于篡改。判据是
 *   机械的：这类叙述总带着固定的措辞标记（`曾经` / `更正（` / `后记（` / 测试基线 `A → B`）。
 *   豁免走头部注释 `<!-- doc-link-allow D-004: 理由 -->`（理由不可为空）——本条与 D-002 同源，
 *   都需要一个"我确实在引用反例措辞"的出口（`docs/README.md` 定义这条规矩时就在引用它）。
 *
 * 扫描范围（与「文档下沉原则」对齐：单模块文档在该模块目录内）：
 *   docs/                系统级文档（**archive/ 除外**，见下）
 *   symbio/src/          插件模块文档（plugins/<plugin>/README.md + plugins/<plugin>/docs/）
 *   tauri/               前端模块文档（README.md + docs/）
 *   cli/                 命令行模块文档（README.md + docs/）
 *   examples/            示例文档
 *   根目录 *.md          README / CONTRIBUTING / CODE_OF_CONDUCT
 *
 * **整体豁免 `docs/archive/`**：归档记录的是**当时形态**，其中指向的兄弟文档可能
 *   早已被合并 / 删除，改写归档链接等于篡改历史。故该目录整体跳过（跳过文件数会打印）。
 *   活文档（其余全部范围）的失效链接照常报。
 *
 * 用法：
 *   node scripts/doc-link-audit.mjs              # 审计（任一规矩命中即失败）
 *   node scripts/doc-link-audit.mjs --root=<dir> # 换仓库根（回归测试用）
 *
 * 退出码：
 *   0 = 四条规矩全过
 *   1 = 有命中（**默认即失败**，不需要 `--strict`
 *       ——2026-09-20 前失效链接只在 `--strict` 下失败，而门禁从不带该参数 ⇒ 从未真的红过）
 *
 * 豁免：
 *   整体豁免 `docs/archive/`（唯一例外，不做逐条留痕）；
 *   D-002 / D-004 另有头部注释豁免（`<!-- doc-link-allow D-00X: 理由 -->`，理由不可为空）；
 *   D-001 / D-003 **不给豁免**——「目标文件是否存在」与「行数是否超限」都是精确判定，
 *   没有需要解释的中间态。
 *
 * 与仓库约定一致：纯 Node 实现，不依赖 bash / ripgrep，Windows / macOS / Linux 通用。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')
// `--root=<仓库根>` 供回归测试用：在临时目录里铺夹具再跑，才能证明它会红
// （与 `mechanism-audit` / `style-audit` 同一约定）。
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const repoRoot = rootArg ? path.resolve(rootArg.slice(7)) : defaultRoot

// `--strict` 是历史参数（曾经"只有加了它才失败"），现已无额外作用：失效链接默认即失败。
// 保留识别是为了不让旧命令报错，但**不参与判定**——留着参与判定就会有人以为
// "没加 --strict 所以没拦住"是预期行为。

/** 扫描根（相对 repoRoot）；目录递归，文件直接检查 */
const ROOTS = ['docs', 'symbio/src', 'tauri', 'cli', 'examples']

/** 根目录下的散落 Markdown */
const ROOT_FILES = ['README.md', 'CONTRIBUTING.md', 'CODE_OF_CONDUCT.md']

/**
 * 整体豁免的目录（相对 repoRoot，以 `/` 结尾）。
 * docs/archive/ = 历史归档，其链接指向的是**当时**的兄弟文档，改写等于篡改历史。
 */
const EXEMPT_DIRS = ['docs/archive/']

/** 递归时跳过的目录名（构建产物 / 依赖 / 版本库） */
const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'dist', 'build', '.venv'])

// ==================== D-002：过程文档必须归档 ====================
/**
 * 归档是**动作**，能保持住的才是机制。
 *
 * `docs/README.md` 早写明「历史实施记录一律进 `archive/`，现行文档只描述当前行为」，
 * 但它只是一句话，没有任何东西检查——于是 2026-09-23 前的 `docs/design/` 里躺着
 * 8 篇自述「一次性复核报告」「文档类型：评审」「实施前的方案（已全部落地）」的过程文档，
 * 活跃文档 9,662 行里有约 2,200 行是**第 N 轮的评审与整改记录**。
 *
 * 代价不是"文档多"，而是**每次重构都要回头改历史**：实测 `356ba9d`（79 文件）与
 * `c54158b`（81 文件）各改了 13 / 14 个文档文件，其中一部分改的正是这类记录。
 * 它们描述的是"当时怎么想的"，与现状一致既不必要、又不可能——因为现状已经变了。
 *
 * 判据：文档**头部自述**为一次性 / 评审 / 审计 / 已落地实施方案 ⇒ 必须在 `archive/` 下。
 * 用「自述」而不是文件名匹配：文件名（`-review-` / `-health-check-`）是约定，会漂移；
 * 而这批文档**每一篇都在开头写明了自己的类型**，读它们自己写的话比猜文件名准。
 *
 * 扫描范围覆盖**模块文档**，不只 `docs/`——过程文档同样会沉积在模块目录里：
 * `symbio/src/plugins/session/docs/legacy-route-migration.md`（已完成的迁移审计，
 * 每行都是「已删 / 已下线」）就这样在活目录里躺了多轮，每次路由改动都被回改一次。
 * 只扫 `docs/` 时它**永远不会被提示**——这类漏网正是「改一个功能要动十几个文档」的来源之一。
 *
 * 豁免：头部（前 `DOCTYPE_HEAD_LINES` 行）写 `<!-- doc-link-allow D-002: 理由 -->`，
 * 理由不可为空（与 `grep-audit` / `dead-code-audit` 同一条约定）。
 */
const DOCTYPE_HEAD_LINES = 15
// ⚠️ 中文后面**不能**用 `\b`：JS 的 `\b` 是 ASCII 语义，汉字不算 word char，
// 于是 `评审\b` 在「评审（一次性结论…）」里永远不匹配（`审` 与 `（` 之间无 ASCII 边界）。
// 这条不去掉，整条规则会静默失效——输出照样是「应归档 0 篇」，看着像一切正常。
const PROCESS_MARKERS = [
  /文档类型[:：]\s*(?:评审|审计)/,
  /文档类型[:：]\s*设计（实施前的方案）/,
  /状态[:：]\s*一次性(?:复核报告|评估记录|审计记录|结论)?/,
  /状态[:：]\s*\*{0,2}已实施\*{0,2}/,
  /一次性(?:复核报告|评估记录|审计记录)/,
  /本文件是\*{0,2}一次性审计记录\*{0,2}/,
]
const WAIVER_D002_RE = /<!--\s*doc-link-allow\s+D-002\s*:\s*(\S.*?)\s*-->/

/** 头部自述为过程文档则返回命中的正则，否则 null */
function processDocMarker(text) {
  const head = text.split('\n').slice(0, DOCTYPE_HEAD_LINES).join('\n')
  if (WAIVER_D002_RE.test(head)) return null
  return PROCESS_MARKERS.find((re) => re.test(head)) ?? null
}

// ==================== D-003：行数预算 ====================
/**
 * 活跃文档的行数上限。**不给豁免**（`docs/archive/` 整体跳过是唯一的例外）：
 * 超过它不是一个需要解释的特例，而是「内容该归别人了」或「该拆成两篇了」的信号，
 * 而这两个动作都不需要一个理由字段来正当化。
 *
 * 数值取 800 而非更小，是为了不逼人把**必须在一起**的规范切碎——实测本仓库最长的
 * 两篇（`docs/design/vdfs.md` 1045、`session/docs/core-loop.md` 881）都不是"差一点点"，
 * 而是各自含着一整类可归给他处的内容。
 */
const MAX_DOC_LINES = 800

/** 行数口径：末尾无换行也算一行（与编辑器 / `Get-Content` 的计数一致） */
function lineCount(text) {
  return text.replace(/\n$/, '').split('\n').length
}

// ==================== D-004：变更史不得混入活跃文档 ====================
/**
 * 只收**结构标记**与**不会出现在正常论述里的措辞**，不收「不再 / 以前」这类
 * 现行语义也常用的词——守卫一旦开始误报，下场是被整体豁免，比没有还糟。
 * 因此 `docs/README.md` 里「不写『曾经…』」这句规则定义本身会命中，走豁免。
 */
const HISTORY_MARKERS = [
  /^#{2,4}\s*修订/,
  /^#{2,4}\s*更正/,
  /修订（/,
  /更正（/,
  /后记（/,
  /追记（/,
  /曾经/,
  /原(?:先)?写/,
  /已改为/,
  /(?:rustTests|vitestTests)\s*[\d,]+\s*→/,
]
const WAIVER_D004_RE = /<!--\s*doc-link-allow\s+D-004\s*:\s*(\S.*?)\s*-->/

/** 正文里的变更史痕迹（行号 + 命中标记），最多报 `limit` 处 */
function historyHits(text, limit = 5) {
  const hits = []
  const lines = text.split('\n')
  for (let i = 0; i < lines.length && hits.length < limit; i++) {
    const marker = HISTORY_MARKERS.find((re) => re.test(lines[i]))
    if (marker) hits.push({ line: i + 1, text: lines[i].trim().slice(0, 60), marker })
  }
  return hits
}

/** 头部是否声明了 D-004 豁免（理由不可为空） */
function waivedD004(text) {
  const head = text.split('\n').slice(0, DOCTYPE_HEAD_LINES).join('\n')
  return WAIVER_D004_RE.test(head)
}

/** 递归 docs/ 下的 .md（目录不存在 ⇒ 空数组） */
function walkDocs(dir, out = []) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const entry of entries) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue
      walkDocs(path.join(dir, entry.name), out)
    } else if (entry.name.endsWith('.md')) {
      out.push(path.join(dir, entry.name))
    }
  }
  return out
}

// 形如 [文字](目标)；目标里的括号不常见，按非贪婪取到第一个右括号
const LINK = /\[[^\]]*\]\(([^)]+)\)/g

/** 应跳过的目标：外链 / 协议 / 纯锚点 / 空 */
const isExternal = (t) =>
  t === '' ||
  t.startsWith('#') ||
  /^[a-z][a-z0-9+.-]*:/i.test(t) // http: https: mailto: file:

const bad = []
let total = 0
let skippedFiles = 0

const walk = (dir) => {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue
      walk(path.join(dir, entry.name))
    } else if (entry.name.endsWith('.md')) {
      check(path.join(dir, entry.name))
    }
  }
}

function check(file) {
  const rel = path.relative(repoRoot, file).split(path.sep).join('/')
  if (EXEMPT_DIRS.some((d) => rel.startsWith(d))) {
    skippedFiles += 1
    return
  }
  const text = fs.readFileSync(file, 'utf8')
  for (const m of text.matchAll(LINK)) {
    const raw = m[1].trim()
    if (isExternal(raw)) continue
    const target = raw.split('#')[0].trim() // 去锚点
    if (isExternal(target) || target === '') continue
    total += 1
    const resolved = path.resolve(path.dirname(file), target)
    if (!fs.existsSync(resolved)) {
      bad.push({ from: path.relative(repoRoot, file), to: target })
    }
  }
}

for (const rel of ROOTS) {
  const dir = path.join(repoRoot, rel)
  if (!fs.existsSync(dir)) {
    console.error(`找不到扫描根：${dir}`)
    process.exit(1)
  }
  walk(dir)
}
for (const rel of ROOT_FILES) {
  const file = path.join(repoRoot, rel)
  if (fs.existsSync(file)) check(file)
}

// ---- D-002 / D-003 / D-004：活跃文档的三条体检 ----
// 扫描范围 = 活文档根（`docs/` + 模块文档 + 根目录散落 md）；`examples/` 是示例包内容，
// 不是项目文档，跳过（同 D-002 一直以来的口径）。三条规矩**共用同一次读盘**：
// 分三遍读等于让守卫的代价翻三倍，而它们的扫描范围完全一致。
const BODY_ROOTS = ['docs', 'symbio/src', 'tauri', 'cli']
const misplaced = []
const oversized = []
const historical = []
let docsScanned = 0

function scanBody(file) {
  const rel = path.relative(repoRoot, file).split(path.sep).join('/')
  if (EXEMPT_DIRS.some((d) => rel.startsWith(d))) return
  docsScanned += 1
  const text = fs.readFileSync(file, 'utf8')

  const marker = processDocMarker(text)
  if (marker) misplaced.push({ rel, why: marker.source })

  const lines = lineCount(text)
  if (lines > MAX_DOC_LINES) oversized.push({ rel, lines })

  if (!waivedD004(text)) {
    const hits = historyHits(text)
    if (hits.length > 0) historical.push({ rel, hits })
  }
}

for (const root of BODY_ROOTS) {
  for (const file of walkDocs(path.join(repoRoot, root))) scanBody(file)
}
for (const rel of ROOT_FILES) {
  const file = path.join(repoRoot, rel)
  if (fs.existsSync(file)) scanBody(file)
}

const exemptNote = skippedFiles > 0 ? `（豁免 docs/archive/ 下 ${skippedFiles} 个文件）` : ''
console.log(`扫描相对链接 ${total} 条，失效 ${bad.length} 条${exemptNote}`)
for (const { from, to } of bad) {
  console.log(`  ${from}  ->  ${to}`)
}
if (bad.length > 0) {
  console.log('\n提示：活文档的失效链接必须修（多为文档移动 / 改名后未更新入链）。')
}

// 失效链接**默认即失败**（原先要 `--strict` 才失败，而门禁从不带它 ⇒ 这条守卫
// 从未真的红过）。改的理由是这条判定**不是启发式**：目标文件存在或不存在，没有
// "疑似" 的中间地带，因此不存在"误报逼人写豁免"那条顾虑——那正是
// `style-audit` 规则 B 与 `grep-audit` S-002-bonus 保持 WARNING 的理由，此处不适用。
// 豁免只有一处（`docs/archive/`），且是**整体**豁免，不需要逐条留痕。
if (bad.length > 0) {
  console.log('\n（失效链接判定为失败：活文档的站内相对链接要么存在、要么不存在，无中间态。）')
}

console.log(`D-002 过程文档归档：扫描 ${docsScanned} 个活跃文档，应归档 ${misplaced.length} 篇`)
for (const { rel, why } of misplaced) {
  console.log(`  ✗ ${rel}  自述命中 /${why}/`)
}
if (misplaced.length > 0) {
  console.log('\n提示：过程文档（一次性评审 / 体检 / 已落地的实施方案）请 `git mv` 到 docs/archive/。')
  console.log('      它们记录的是「当时怎么想的」，与现状一致既不必要也不可能——而每次重构')
  console.log('      回头改历史，正是「改一个功能要动十几个文档」的一部分来源。')
}

// ---- D-003：行数预算 ----
console.log(`D-003 行数预算：上限 ${MAX_DOC_LINES} 行，超限 ${oversized.length} 篇`)
for (const { rel, lines } of oversized) {
  console.log(`  ✗ ${rel}  ${lines} 行（超 ${lines - MAX_DOC_LINES}）`)
}
if (oversized.length > 0) {
  console.log('\n提示：超限即「内容不属于这里」或「该拆篇」的信号，按 docs/README.md 的 owner 表处置：')
  console.log('      · 模块内部机制 → 该模块 README.md；· 论证与被否决的方案 → DECISIONS.md（只留 ADR 引用）')
  console.log('      · 协议线上形状 → architecture/PROTOCOLS.md；· 列表类事实 → CURRENT.md（自动生成，不手抄）')
  console.log('      · 实例 / 范例 → 由被举例的那个模块的 README 持有')
  console.log('      本条**不给豁免**：拆分方向由 owner 表给出，不需要逐篇解释。')
}

// ---- D-004：变更史不得混入活跃文档 ----
console.log(`D-004 变更史残留：命中 ${historical.length} 篇`)
for (const { rel, hits } of historical) {
  for (const h of hits) console.log(`  ✗ ${rel}:${h.line}  命中 /${h.marker.source}/  ${h.text}`)
}
if (historical.length > 0) {
  console.log('\n提示：现行文档只描述「现在是什么」——把过去写成现在，代价是每次重构都要回头改历史，')
  console.log('      而历史改成与现状一致就不再是历史。改为陈述当前行为（需要论证则指向 ADR，')
  console.log('      需要过程记录则 `git mv` 进 docs/archive/）。确实在引用反例措辞时（如定义该规矩的那篇），')
  console.log('      头部写 `<!-- doc-link-allow D-004: 理由 -->`（理由不可为空）。')
}

process.exit(bad.length > 0 || misplaced.length > 0 || oversized.length > 0 || historical.length > 0 ? 1 : 0)
