#!/usr/bin/env node
/**
 * doc-link-audit — 文档相对链接审计
 *
 * 用途：**活跃文档体检**——七条机械可判定的规矩，每条都对应一类「没人看着就必然腐烂」的文档病：
 *
 *   D-001 站内相对链接：文档移动 / 归档（`git mv`）最容易留下静默坏链——阅读时才发现，
 *     而它本可以在提交前被机械地查出来。
 *   D-002 过程文档必须归档：靠文档**头部自述**判定（一次性评审 / 体检 / 已落地实施方案）。
 *   D-003 行数预算：活跃文档 **不得超过 `MAX_DOC_LINES`**。
 *   D-004 变更史不得混入活跃文档正文：历史归 `git log` 与 `archive/`。
 *   D-006 反引号里的文件路径：正文用 `` `path/to/x.md` `` 指路时，目标必须存在。
 *   D-007 站内锚点：链接的 `#fragment` 必须等于目标文件某个标题的 slug（含纯锚点 `#x`）。
 *   D-008 裸风险编号：正文里的 `R\d+` 必须是**定义**（`| Rn |` 表行的首列）或
 *     **限定引用**（编号紧邻「风险」二字），其余一律报红。
 *
 * D-006 存在的理由（它是 D-001 的**盲区补丁**）：D-001 只认 Markdown 链接语法
 *   `[文字](目标)`，而本仓正文里指路**更常**写成行内反引号（"详见 `docs/design/vdfs.md`"）。
 *   这种写法 D-001 一个字都看不到——实测 `tauri/docs/FRONTEND.md` 首段引用的
 *   `docs/design/frontend-ui-ux-prd.md` 与 `-design.md` **两个文件均不存在**，
 *   而 D-001 报「失效 0 条」已经很久了。**守卫报 0 不等于没有坏链，只等于它看不见。**
 *
 * D-006 的两条防误报设计（缺任一条都会让规则被豁免喂到失效）：
 *   ① **双根解析**：`docs/DECISIONS.md` 这类路径在模块 README 里是**相对仓库根**
 *      写的，在 `docs/` 内部又是相对当前文件——先试文件目录、再试仓库根，任一命中即通过。
 *      实测把失败数从 34 降到 9。
 *   ② **只判定「不含 `./` `../` 且至少含一个 `/`」的路径**：无斜杠的（`` `README.md` ``）
 *      无法判定相对谁；显式相对路径（`../x.md`）另行处理。实测 193 次出现里只有
 *      79 次进入判定——剩余 114 次是说明性文字（`` `x.md` `` / `` `CHANGELOG.md` ``），
 *      判定它们会让守卫满屏误报。
 *
 * **给豁免**（与 D-001 / D-003 的「不给豁免」不同）：本规则是**启发式**——
 *   缩写式引用（`` `session/docs/core-loop.md` `` 指 `symbio/src/plugins/session/docs/core-loop.md`）
 *   与「示意性路径」（`` `.vite/license.md` ``）在文本上与真断链无法区分。
 *   硬判会让作者被迫改写正确的简写，故提供豁免
 *   `<!-- doc-link-allow D-006: 理由 -->`（理由不可为空，同 D-002 / D-004）。
 *
 *   **豁免分档**（从头部注释升级而来，理由是实测暴露的两个缺口）：
 *   · **行内**：写在**同一行**（或该行**前一行**）的豁免注释，只豁免**本行**的反引号路径。
 *     规则文档（`session/docs/README.md` 用 `` `docs/x.md` `` 举例说明路径怎么写）
 *     与缺陷记录（`frontend-ui-ux-plan.md` 引用两个已不存在的文件名，以记录"已修"）
 *     都属此类——它们的"坏路径"是**内容**，不是**入链**。
 *   · **全文**：写在头部（前 `DOCTYPE_HEAD_LINES` 行）的豁免注释，豁免**整篇**。
 *     实测全文豁免会被滥用（一篇讲文档规矩的文章给整篇挂豁免，等于规则对它失效），
 *     故只在确需时用；行内豁免覆盖绝大多数真实场景。
 *
 * D-008 存在的理由（**编号引用**这一类的首条规则）：2026-10-08 复核时发现
 *   `docs/plan/04` §3.2 与 `docs/decisions/session.md` 的 ADR-047 都写着「R1」，
 *   而本仓的 `R1` 有**两个**含义——`04` 风险登记表的 R1（从未写过可运行系统代码）
 *   与从 `feat` 分支并入的 `docs/plan/06` §10.2 的 R1（「主会话不持有工具」的架构定案）。
 *   后者那篇文档**不在本仓**：引用悬空，却撞进了前者的编号空间。
 *
 *   关键在于**解析型规则抓不到它**——`R1` 能解析到风险表那一条，只是解析到了**错误的
 *   含义**。这正是本文件反复写下的判词：**守卫报 0 不等于没有坏链，只等于它看不见。**
 *   机械可判的只有**形态**：这个编号是定义、是限定引用、还是裸的。撞号是裸编号的产物，
 *   故本条判形态、不判解析。
 *
 *   **不给豁免**：`R\d+` 在本仓只表示风险编号，处置只有两种且都不需要解释——
 *   要么它是 `| Rn |` 表行的首列（定义），要么紧邻「风险」二字（限定引用）。
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
 *   0 = 六条规矩全过
 *   1 = 有命中（**默认即失败**，不需要 `--strict`
 *       ——2026-09-20 前失效链接只在 `--strict` 下失败，而门禁从不带该参数 ⇒ 从未真的红过）
 *
 * 豁免：
 *   整体豁免 `docs/archive/`（唯一例外，不做逐条留痕）；
 *   D-002 / D-004 另有头部注释豁免（`<!-- doc-link-allow D-00X: 理由 -->`，理由不可为空）；
 *   D-006 的豁免**分两档**——行内（本行或前一行，只豁免该行）/ 全文（须为文件首行非空内容）；
 *   D-001 / D-003 **不给豁免**——「目标文件是否存在」与「行数是否超限」都是精确判定，
 *   没有需要解释的中间态。（D-006 与 D-001 的差别就在这：前者判的是反引号文本，
 *   与「缩写」「示意」在字面上不可分，故必须留豁免出口。）
 *
 * 与仓库约定一致：纯 Node 实现，不依赖 bash / ripgrep，Windows / macOS / Linux 通用。
 */

// 判据码命名空间（登记表 docs/reference/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns D 文档正文与链接
// @codes D-001 D-002 D-003 D-004 D-006 D-007 D-008

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
  // D-004 的第 8 条（plan/13 批 D0）：**计数的箭头形态**。
  //
  // 「`check_all` 五条 → 七条」与「曾经 / 已改为」是同一件事——它在数「从几个变成
  // 几个」，而变更历史归 `git log`。此前 D-004 只认那三个**词**，于是这个**形状**
  // 完全漏掉：活跃文档里四处 `N → M` 的计数迁移长期无人拦（`04` 两处、`11` 一处、
  // `12` 一处）。
  //
  // 判据取**形状**且要求箭头两侧都带**量词**：`条`/`项`/`个`/`处`/`格`。这个限定是
  // 必要的——本仓大量用 `→` 画流转（`turn → opened`），而 04 §3.1 的状态表合法地
  // 记着测试数增减（`56 → 46`）。不带量词的裸数字箭头一律不判，免得那条状态表
  // 被整片误伤。
  /(?:[一二三四五六七八九十]+|\d+)\s*(?:条|项|个|处|格)\s*→\s*(?:[一二三四五六七八九十]+|\d+)/,
  // 同类的时序回指标记，与「曾经」同类：它把「现在的样子」指向「最初的样子」。
  /初版/,
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

// ==================== D-006：反引号里的文件路径 ====================
/**
 * 行内反引号包着的路径（`` `docs/x.md` `` / `` `../a/b.md` ``）。
 *
 * 字符集限 ASCII 路径字符：本仓不存在中文文件名，放宽只会误吃正文里的
 * 其它反引号片段（如 `` `a.b/c.md` `` 这类伪路径）。
 */
const BACKTICK_PATH = /`([A-Za-z0-9_][A-Za-z0-9_./-]*\.md)`/g
const WAIVER_D006_RE = /<!--\s*doc-link-allow\s+D-006\s*:\s*(\S.*?)\s*-->/

/**
 * 该反引号路径是否**可判定**——不可判定的直接跳过（防误报的第一道闸）。
 *
 * - 不含 `/`：相对谁无法确定（`` `README.md` `` / `` `CHANGELOG.md` `` 常是泛指）；
 * - 以 `/` 开头：是绝对文件系统路径或路由，不是仓库内相对路径；
 * - 含通配星号 / `...`：是模式或省略写法。
 */
function isJudgeableBacktickPath(t) {
  if (!t.includes('/')) return false
  if (t.startsWith('/')) return false
  if (t.includes('*') || t.includes('...')) return false
  return true
}

/**
 * 双根解析（防误报的第二道闸）：先当**相对当前文件**，再当**相对仓库根**，
 * 任一命中即视为存在。
 *
 * 为什么必须双根：本仓两种写法都合法且在用——
 *   · `docs/` 内部互相引用走相对（`./vdfs.md`）
 *   · 模块 README 引用系统文档走仓库根（`docs/DECISIONS.md`）
 * 只认一种会把另一种全部误报（实测：单根时 34 条失败，双根后 9 条）。
 */
function backtickPathExists(fromFile, t) {
  const relToFile = path.resolve(path.dirname(fromFile), t)
  if (fs.existsSync(relToFile)) return true
  const relToRoot = path.resolve(repoRoot, t)
  return fs.existsSync(relToRoot)
}

/**
 * 该行是否被**行内豁免**覆盖：本行或**前一行**带 `<!-- doc-link-allow D-006: 理由 -->`。
 *
 * 取两行是为了让注释既可以写在被豁免内容之前（Markdown 里更常见，因为注释
 * 混在正文行内会打断排版），也可以写在同行（`<!-- ... -->` 与反引号共存）。
 */
function waivedD006Line(lines, i) {
  if (WAIVER_D006_RE.test(lines[i])) return true
  return i > 0 && WAIVER_D006_RE.test(lines[i - 1])
}

/**
 * **全文豁免只在它是文件第一行非空内容时生效**（实测教训）。
 *
 * 起初「头部 15 行内出现注释 ⇒ 豁免整篇」，结果行内豁免**永远不可达**：
 * 一段开头就写行内豁免的文档，整篇都被放行——测试 `D-006 行内豁免` 因此蒙混过关，
 * 真实的 `session/docs/README.md`（第 43 行举例）也会被整篇豁免，规则对它失效。
 * 判据改为「注释之前没有别的内容」：这样「整篇豁免」是文件的**首行声明**，
 * 而出现在正文中间的同类注释自然退化为行内作用域。
 */
function waivedD006Whole(text) {
  for (const line of text.split('\n').slice(0, DOCTYPE_HEAD_LINES)) {
    if (line.trim() === '') continue
    return WAIVER_D006_RE.test(line)
  }
  return false
}

/** 正文里**判不出存在**的反引号路径（行号 + 路径），最多报 `limit` 处 */
function backtickHits(file, text, limit = 8) {
  const hits = []
  const lines = text.split('\n')
  for (let i = 0; i < lines.length && hits.length < limit; i++) {
    if (waivedD006Line(lines, i)) continue
    for (const m of lines[i].matchAll(BACKTICK_PATH)) {
      const t = m[1]
      if (!isJudgeableBacktickPath(t)) continue
      backtickChecked += 1
      if (!backtickPathExists(file, t)) hits.push({ line: i + 1, target: t })
    }
  }
  return hits
}

/** 递归 docs/ 下的 .md（目录不存在 ⇒ 空数组） */
// ==================== D-008：裸风险编号必须是定义或限定引用 ====================

/**
 * D-008 单篇判定：返回**裸** `R\d+` 的命中行。
 *
 * 三种形态，只有第三种是缺陷：
 *  · **定义** —— `| Rn | ...` 表行的**首列**（编号落在首列区间内）。编号只能从这里产生；
 *  · **限定引用** —— 编号紧邻「风险」二字（其间只允许空白），如「参见风险 R3」；
 *  · **裸** —— 两者皆非。撞号唯一的机械信号。
 *
 * 为什么判形态而不判解析：撞号的两个 `R1` **都能解析**（`04` 风险表里就有一条 R1），
 * 解析会把「错误的含义」判成通过——**守卫报 0 不等于没有坏链，只等于它看不见**。
 */
function riskIdHits(text) {
  const hits = []
  const lines = text.split('\n')
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const def = line.match(/^\s*\|\s*R\d+\s*\|/) // 定义行的首列区间
    const re = /\bR\d+\b/g
    let m
    while ((m = re.exec(line))) {
      if (def && m.index < def[0].length) continue // 首列 = 定义
      if (/风险\s*$/.test(line.slice(0, m.index))) continue // 紧邻「风险」= 限定引用
      hits.push({ line: i + 1, text: line.trim() })
      break // 一行报一次：判定与处置都按行
    }
  }
  return hits
}

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

// ==================== D-007：站内锚点必须指向存在的标题 ====================
/**
 * 为什么需要它：D-001 在判定前 `raw.split('#')[0]` 把 fragment **整段丢掉**——于是
 * 「文件在、锚点指向的标题不在」这类失效**全仓无人守**。它与 D-006 是同一类**盲区补丁**
 * （D-006 的判词同样适用于此：**守卫报 0 不等于没有坏链，只等于它看不见**）：本次把
 * `DECISIONS.md` 拆成索引 + 5 册时新增了 90 余条跨册锚点，锚点面一下子翻了数倍，
 * 而没有任何东西会红。
 *
 * 判据：站内链接（含纯锚点 `#x`）的 fragment 必须等于**目标文件某个标题的 GitHub slug**。
 *   标题 → slug：小写 → 去掉非（字母 / 数字 / 空白 / `-` / `_`）→ 每个空白转一个 `-`。
 *   于是 `**` 加粗、反引号、`——`、`（出）`、`+` 一律被丢弃，所以
 *   ``## ADR-020: 执行期与传输层**分离**——`EventSink`（出）+ `AbortSignal`（入）…``
 *   的 slug 里 `eventsink出` 是**连着的**（`+ ` 那个空格才产生连字符）。
 *   这条算法在真实数据上校准过：全仓 90 余条 ADR 索引锚点与 4 处外部 ADR 锚点逐字通过。
 *
 * 不判定（各自有更合适的归属，报了只是噪音）：
 *   · 非 `.md` 目标（`x.rs#L10`、`x.html#id`）——行号 / DOM id 不是标题 slug；
 *   · 目标文件不存在——D-001 已经报了；
 *   · `docs/archive/` 下的**源文件**——同 D-001 / D-002 的整体豁免（归档是历史快照）。
 *
 * 重复标题：GitHub 给第 2、3 个同名标题追加 `-1` / `-2`，故接受 `slug` 与 `slug-N`（N < 出现次数）。
 */
/** 标题 → GitHub 锚点 slug（保留字母/数字/空白/`-`/`_`，其余丢弃，空白逐个转 `-`） */
function ghSlug(heading) {
  return heading
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, '')
    .replace(/\s/g, '-')
}

/** 取一篇 md 的全部 ATX 标题（跳过 ``` 围栏内的内容，避免把代码注释里的 `#` 当标题） */
function mdHeadings(text) {
  const out = []
  let fence = null
  for (const line of text.split('\n')) {
    const f = line.match(/^\s*(```+|~~~+)/)
    if (f) {
      if (fence === null) fence = f[1][0]
      else if (f[1][0] === fence) fence = null
      continue
    }
    if (fence !== null) continue
    const h = line.match(/^#{1,6}\s+(.+?)\s*#*\s*$/)
    if (h) out.push(h[1])
  }
  return out
}

/** 文件路径 → 可用锚点集合（含重复标题的 `-N` 变体）；读盘结果缓存 */
const slugCache = new Map()
function slugSet(absPath) {
  if (slugCache.has(absPath)) return slugCache.get(absPath)
  const set = new Set()
  try {
    const counts = new Map()
    for (const h of mdHeadings(fs.readFileSync(absPath, 'utf8'))) {
      const base = ghSlug(h)
      const n = counts.get(base) ?? 0
      counts.set(base, n + 1)
      set.add(n === 0 ? base : `${base}-${n}`)
    }
  } catch {
    /* 读不出 ⇒ 交给 D-001 报「文件不存在」 */
  }
  slugCache.set(absPath, set)
  return set
}

const bad = []
const badAnchors = []
let total = 0
let anchorsChecked = 0
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

/** D-007 单条锚点判定：fragment 必须是目标文件某标题的 slug */
function checkAnchor(fromRel, absFile, rawFrag, toLabel) {
  if (!rawFrag) return
  let frag = rawFrag
  try {
    frag = decodeURIComponent(rawFrag)
  } catch {
    /* 非法转义 ⇒ 按原样比，让判定去报它 */
  }
  anchorsChecked += 1
  if (slugSet(absFile).has(frag.toLowerCase())) return
  badAnchors.push({ from: fromRel, to: toLabel, frag: rawFrag })
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
    if (raw.startsWith('#')) {
      // 纯锚点：查本文件的标题（这类链接 D-001 完全看不见）
      checkAnchor(rel, file, raw.slice(1), rel)
      continue
    }
    if (isExternal(raw)) continue
    const hash = raw.indexOf('#')
    const target = (hash >= 0 ? raw.slice(0, hash) : raw).trim()
    if (isExternal(target) || target === '') continue
    total += 1
    const resolved = path.resolve(path.dirname(file), target)
    if (!fs.existsSync(resolved)) {
      bad.push({ from: rel, to: target }) // D-001 报它；锚点不再重复报
      continue
    }
    // 只判 .md 目标：`x.rs#L10` / `x.html#id` 的 fragment 不是标题 slug
    if (hash >= 0 && target.endsWith('.md')) {
      checkAnchor(rel, resolved, raw.slice(hash + 1), target)
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
const backtickBad = []
const bareRisk = []
let docsScanned = 0
let backtickChecked = 0

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

  if (!waivedD006Whole(text)) {
    const dead = backtickHits(file, text)
    if (dead.length > 0) backtickBad.push({ rel, dead })
  }

  const bare = riskIdHits(text)
  if (bare.length > 0) bareRisk.push({ rel, hits: bare })
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

// ---- D-006：反引号里的文件路径 ----
console.log(`D-006 反引号路径：判定 ${backtickChecked} 条，失效 ${backtickBad.length} 篇`)
for (const { rel, dead } of backtickBad) {
  for (const d of dead) console.log(`  ✗ ${rel}:${d.line}  \`${d.target}\`  不存在`)
}
if (backtickBad.length > 0) {
  console.log('\n提示：行内反引号写出的文件路径 D-001 看不见（它只认 `[文字](目标)`）。两种处置：')
  console.log('      · 真断链 → 改正路径（多为文件移动 / 改名后未更新入链）；')
  console.log('      · 缩写 / 示意性路径（如 `session/docs/core-loop.md` 指插件内同名文件）→')
  console.log('        改为完整路径，或头部写 `<!-- doc-link-allow D-006: 理由 -->`（理由不可为空）。')
}

// ---- D-007：站内锚点 ----
console.log(`D-007 站内锚点：判定 ${anchorsChecked} 条，失效 ${badAnchors.length} 条`)
for (const { from, to, frag } of badAnchors) {
  console.log(`  ✗ ${from}  ->  ${to}#${frag}  目标文件里没有这个标题`)
}
if (badAnchors.length > 0) {
  console.log('\n提示：D-001 判定前会把 `#…` 丢掉，所以「文件在、标题不在」一直是盲区。')
  console.log('      多为文档拆篇 / 改标题后未更新入链。标题 → 锚点的算法：小写 → 去掉非')
  console.log('      （字母 / 数字 / 空白 / `-` / `_`）→ 每个空白转一个 `-`（加粗、反引号、')
  console.log('      `——`、`（出）` 之类都被丢掉）。本条**不给豁免**：标题存在与否是精确判定。')
}

// ---- D-008：裸风险编号 ----
console.log(`D-008 裸风险编号：扫描 ${docsScanned} 篇，命中 ${bareRisk.length} 篇`)
for (const { rel, hits } of bareRisk) {
  for (const h of hits) console.log(`  ✗ ${rel}:${h.line}  ${h.text}`)
}
if (bareRisk.length > 0) {
  console.log('\n提示：`R<n>` 在本仓只表示风险编号，而裸编号会撞进别人的编号空间——实测')
  console.log('      `04` 风险表的 R1 与并入文档的 R1 各指一事，且**都能被解析到**，所以')
  console.log('      解析型守卫看不见它（**守卫报 0 不等于没有坏链，只等于它看不见**）。')
  console.log('      两种处置都不需要解释：')
  console.log('      · 定义 → 写成 `| Rn | ...` 表行的首列；')
  console.log('      · 引用 → 紧邻「风险」二字（如「参见风险 R3」）。')
  console.log('      本条**不给豁免**：判定的是编号的形态，不是语义。')
}

process.exit(
  bad.length > 0 ||
    misplaced.length > 0 ||
    oversized.length > 0 ||
    historical.length > 0 ||
    backtickBad.length > 0 ||
    badAnchors.length > 0 ||
    bareRisk.length > 0
    ? 1
    : 0
)
