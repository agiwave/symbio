#!/usr/bin/env node
/**
 * doc-symbol-audit — 文档符号指认审计（D-005 / D-009 / D-011 / D-012 / D-014，全仓级，跨插件）
 *
 * 守的是「现行文档指认的符号确实存在」。这类失效没人看得见：读者按图
 * 索骥找不到定义，而 doc-link-audit 只管链接、facts/current 只从代码生成
 * 结构表，都不校验散文里的 `module::symbol` 与裸常量名。判据在真实数据上校准过。
 *
 * 判据：
 *   **D-005**（`module::symbol` 形态）：提取所有 *.md 中反引号包裹、以 `::` 分段且
 *     **每段为标识符**的路径（≥2 段，剥尾部 `()`），取最后一段查它是否以整词出现在
 *     Rust 源码（symbio/src、cli/src、tauri/src-tauri/src）的 .rs 语料里；查不到 ⇒ ERROR。
 *   **D-009**（裸常量名形态）：反引号里的单段全大写名（形如 `ROUTE_SESSION_CHAT_SEND`，
 *     `^[A-Z][A-Z0-9]*(_[A-Z0-9]+)+$`）必须在 Rust ∪ 前端（tauri/src）∪ 门禁脚本
 *     （scripts）语料里逐字存在——
 *     但**仅当它的同族**（首段 + `_`，如 `ROUTE_`）在代码里另有成员时才判红。
 *   **D-011**（名册锚点列）：[02 §6.1 能力名册](../docs/003-design/008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系")
 *     的「锚点」列——名册 54 行能力若没有任何一行能回答"它在代码里是哪几行"，
 *     "坐标系已归位"与"这条能力真的存在"在日志里长得一模一样。
 *     取值 `文件` / `文件::符号`（反引号形态）或 `—`：**非 `—` 的必须指得到**
 *     （文件在仓内存在；`::符号` 的最后一段查 D-005 的同一份 Rust 语料）。
 *     `—` 只允许出现在真的没落地的行上——故锚定 / 未锚定**计数进日志**，让
 *     "名册在长、锚定率不动"这件事看得见。复用本文件的语料，**不另起一份符号解析**。
 *   **D-012**（CLI 产物指路路径）：现行文档里 `target/<profile>/symbio-cli(.exe)` 这类
 *     会被读者照抄的产物路径，必须等于 `scripts/cli-binary.mjs` 的 `cliArtifactRelPath`
 *     （构建真正落点，见文件末）。带具名目录段（`symbio/target/…`、`cli/target/…`）一律红；
 *     只带 `./` / `../` 的按该 md 的命令执行目录归一后比。任务登记册（`docs/009-task`）
 *     排除——那里引用旧路径是记录本身，不是指路。
 *
 * 为什么 D-009 要「同族」这道闸：全大写 + 下划线不只是常量的形态，也是**环境变量、CI
 * 密钥、线上错误码**的形态（`APPLE_CERTIFICATE`、`GITHUB_TOKEN`）。那些名字的正确出处
 * 不在本仓代码里，硬查就得为每一处写豁免——而豁免喂到判据失效是这类守卫最容易的死法。
 * 同族把范围收成「本仓自己在用的常量命名空间」：改名漂移（文档写 `ROUTE_TRIAGE_DECIDE`、
 * 代码里实际叫 `ROUTE_CLASSIFY_DECIDE`）正好落在闸内，环境变量族自动落在闸外。
 * 族名只由**产品代码的本体**开出（`gate.d/_shared.mjs` 的 `codeTextOf` 抹掉字符串内容与注释）：
 * `env!("CARGO_PKG_VERSION")` 里的名字是外部约定、注释里的名字是叙述，都不算这一族存在。
 * 工具脚本（`scripts`）**只回答「这个名字写过没有」，不开族**——脚本里的临时常量前缀
 * 会把 cargo/CI 的环境变量族（`OUT_DIR`）误判成「本仓在用的命名空间」，那正是闸门要挡在外面的。
 *
 * 为什么两条判据的语料不同：`a::b::c` 是 Rust 语法，D-005 只查 Rust 侧；裸常量名是跨栈
 * 事实（前端 `VDFS_PAGE_SIZE`、后端 `ROUTE_*` 与门禁脚本的 `TEST_ONLY` 一样会被文档指认），
 * D-009 三侧都算存在。`*.test.mjs` 排除在外——测试里为了造反例而杜撰的名字若算「存在」，
 * 每一个假名都会被自己的回归测试洗白，判据就此空转。
 *
 * 为什么 D-005 只查 ≥2 段、只验最后一段：单段反引号词（`PayloadKey`、CLI 选项、配置键）
 *   与普通词无法区分，查了必然误报一片、最后被豁免喂到失效（D-009 例外：全大写 + 下划线
 *   这个形态本身可判别，见上）；中段模块路径因 re-export 天生多解（顶层 `pub use *`、
 *   模块内 use），逐一解析必然误报。整词集合而非子串：`register_option` 不会误匹配
 *   `register_option_field`。
 *
 * 豁免（都对应「文档说的不是现状」）：
 * doc-link-allow D-010: 下一行的 `001.md` 是占位编号示例（真正的 ADR 文件名是去前缀编号）
 *   1. `docs/archive/`、`docs/003-design/` 整体排除——ADR 正文（`docs/003-design/002-架构总览.md`）
 *      与归档都是历史快照，当时的符号指认不随代码改名回改（回改反而篡改历史）；
 *   2. 行内含历史词（已删除 / 已退役 / 已废弃 / deprecated / 取代 / 判为删除）⇒ 跳过，
 *      如迁移表里「已删除」的行、台账里「`X` 判为删除」的行；
 *   3. 承认通道：行内 `<!-- doc-symbol-allow: <理由> -->`，**理由不可为空**
 *      （空理由视为未承认，与 grep-audit / dead-code-audit 同一约定）。代码里查无此名
 *      有两种合法情形，理由要写明是哪一种：**历史**（「批⑤ 已判删」）或**目标态**
 *      （「09 的 S2 才落这个常量」——施工方案指认未来名字是本分，不该被当成漂移）。
 *
 * 失效形态（回归测试逐条钉住）：
 *   - 审计范围读不出来（没有 md 或没有 .rs 语料）却亮绿灯 ⇒ 必须 exit 1；
 *   - 判据写宽（注释 / 普通词 / ADR 正文 / 单段词误报）⇒ 断言仍 exit 0；
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
// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns D 文档正文与链接
// @codes D-005 D-009 D-011 D-012 D-014

import { readFileSync, readdirSync, existsSync } from 'node:fs'
import { join, dirname, resolve, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'
import { codeTextOf } from './gate.d/_shared.mjs'
import { cliArtifactRelPath } from './cli-binary.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
// `--root=` 只为回归测试开的口子：测试在临时目录造一棵最小仓库，把判据用
// 「注入真实违规并断言变红」验证（判定型守卫的教条：只会亮绿灯 = 没有守卫）。
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? resolve(rootArg.slice(7)) : resolve(__dirname, '..')

/** Rust 语料根：任一缺失可容忍，全缺即「范围读不出」。`docs/003-design/verify` 是硬证据程序
 *  的 Rust 源（`rustc` 直接编译的那批文件），它们声明的常量同样是文档指认的对象。 */
const RS_ROOTS = ['symbio/src', 'cli/src', 'tauri/src-tauri/src', 'docs/003-design/verify']
/** D-009 的第二份语料：裸常量名是跨栈事实，前端声明的常量同样被文档指认 */
const FE_ROOTS = ['tauri/src']
/** D-009 的第三份语料：只回答存在性，不开族（原因见文件头）。
 *  `e2e` 与 `scripts` 同侧：md 侧本来就扫 `e2e/README.md`，代码侧却不认 `e2e` 里声明的
 *  常量 ⇒ 文档指向夹具旋钮（`CONTEXT_WINDOW_TOKENS` 这类）恒假红。存在性口径下加它
 *  不会开出新命名空间，同族闸的行为不变。 */
const TOOL_ROOTS = ['scripts', 'e2e']
/** 裸常量名的形态：全大写、至少一段下划线——单段大写词与普通词无法区分，不查 */
const CONST_SHAPE = /^[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+$/
/** walk 时整树跳过的目录。`.symbio` 是运行时产物：每起一个会话就复制一份
 *  `AGENTS.md` / `SKILL.md`，把它们算进「活跃文档」会让命中数无上界。
 *  `.trae` 是 IDE 会话工作区（对话产物 / 规划草稿）：与 `.workbuddy-ai` 同性质，
 *  不进本仓文档语料（与 registry 的 `SCAN_SKIP_DIRS` 同口径）。 */
const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai', '.symbio', '.trae'])
/** 整目录排除：ADR 记的是**当时**的设计，其中被指认的符号可能早已改名 / 删除，
 *  按现行代码判它会永久误报；归档同理。两者都是「文档说的不是现状」。 */
const EXCLUDE_MD_DIRS = ['docs/archive', 'docs/010-adr']
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
    } else if (ent.name.endsWith('.md')) {
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
/** 工具语料排除 `*.test.mjs`：测试为了造反例而杜撰名字（`ROUTE_TRIAGE_DECIDE` 这类），
 *  把测试算进语料等于让每一个假名都被自己的回归测试洗白。 */
const isTool = (n) => /\.mjs$/.test(n) && !/\.test\.mjs$/.test(n)

// ── 主流程 ──────────────────────────────────────────────────────────────
console.log('--- doc-symbol-audit: 文档符号指认审计（D-005 / D-009 / D-011 / D-012 / D-014） ---')

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
const toolFiles = TOOL_ROOTS.flatMap((r) => collectSource(r, isTool))

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
/** 工具脚本只回答「这个名字在本仓写过没有」，**不开命名空间**：脚本里的临时常量前缀
 *  （`OUT_REL`）会把 cargo / CI 的环境变量族（`OUT_DIR`）当成「本仓在用」拉进闸门，
 *  而那正是同族闸要挡在门外的一类。 */
function indexExistence (raw) {
  for (const m of raw.matchAll(identRe)) codeNames.add(m[0])
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
for (const f of toolFiles) indexExistence(readFileSync(f, 'utf8'))

/** D-011 的名册表所在文件（02 §6.1，全仓唯一一份能力名册）。 */
const ROSTER_MD = 'docs/003-design/008-能力注册表与能力坐标系.md'
/** 名册行的判别：第 2 列是「维」闭集（知 / 行 / 言 / 省 / 欲）——只看列数会撞上
 *  02 里其它同样 8 列的表，靠维的闭集把它钉住。 */
const DIMS = new Set(['知', '行', '言', '省', '欲'])

/**
 * D-011：名册「锚点」列——非 `—` 的指认必须指得到。
 *
 * 返回 `{ total, anchored }` 供日志出锚定率。
 *
 * ## 名册不在扫描范围内 ⇒ 本条不适用（不判红）
 *
 * 回归测试造的是**最小仓库**（只有 README.md 与一小片语料），里面没有全仓唯一那份
 * 名册——那是夹具的性质，不是漂移。真仓若真把名册删了，`registry-audit` 的 RG-020
 * 会先在登记侧红（DE-008 是登记条目），不需要这条跨文档判据再补一刀。**判据只在
 * 判据的对象存在时才有意义**，硬判只会让每条夹具用例都为「没有名册」陪跑。
 *
 * 反向用例（回归测试注入）：锚点指向不存在的符号 / 不存在的文件 ⇒ 红。这条是硬要求——
 * 「名册写了锚点」若不可证伪，就等于把 E6（验的是文档自洽，不是文档 == 代码）
 * 原样搬进了坐标系。
 */
function auditRosterAnchors () {
  const abs = join(REPO_ROOT, ROSTER_MD)
  if (!existsSync(abs)) {
    console.log(dim(`（D-011 不适用：${ROSTER_MD} 不在扫描范围内）`))
    return null
  }
  let total = 0
  let anchored = 0
  const lines = readFileSync(abs, 'utf8').split(/\r?\n/)
  lines.forEach((line, i) => {
    if (!line.startsWith('|')) return
    const cells = line.split('|').slice(1, -1).map((c) => c.trim())
    if (cells.length < 8) return
    if (!DIMS.has(cells[1])) return // 不是名册行
    total++
    const name = cells[0].replace(/`/g, '')
    const anchor = cells[7]
    if (anchor === '—' || anchor === '') return // 尚未落地：计数，不判
    const m = anchor.match(/^`([^`]+)`$/)
    if (!m) {
      report(ROSTER_MD, i + 1, 'D-011',
        `「${name}」的锚点列须是 \`文件\` / \`文件::符号\`（反引号形态）或 \`—\`，读到：${anchor}`)
      return
    }
    const inner = m[1].trim().replace(/\(\)$/, '')
    const [file, ...rest] = inner.split('::')
    if (!existsSync(join(REPO_ROOT, file))) {
      report(ROSTER_MD, i + 1, 'D-011',
        `「${name}」的锚点指向的文件不在仓内：\`${file}\`——文件改名了还是从来没落地？`)
      return
    }
    if (rest.length === 0) {
      anchored++
      return
    }
    // `文件::符号`：符号存在性查 D-005 的同一份语料（不另起一份解析）
    const sym = rest[rest.length - 1]
    if (!idents.has(sym)) {
      report(ROSTER_MD, i + 1, 'D-011',
        `「${name}」的锚点符号在 Rust 源码中不存在：\`${inner}\`——符号改名了，还是这一行标错了能力？`)
      return
    }
    anchored++
  })
  if (total === 0) {
    report(ROSTER_MD, 1, 'D-011', '名册一行都没读出——表格结构变了还是维的闭集写错了？')
    return null
  }
  return { total, anchored }
}

// ── D-014：§8 取值 ↔ 投影模块 三方对账 ──────────────────────────────────
/**
 * D-014：`projection` 取值的**两套名字**必须对得上。
 *
 * 同一层存在两套名字：§8 的取值是**事件里能写的视图名**，`projection/*.rs` 是**实现文件**。
 * 二者不是一一对应（`budget` → `cost.rs`、`replay` → `rebuild.rs`），对应关系此前只写在
 * 各自文件头的散文注释里：读得到、机器读不到，所以没人能证明它没漂。
 *
 * 三方对账（任一侧单独改都会红）：
 *   ① §8 的取值集合 ↔ [RE-009] §1 的左列（加取值不填表 ⇒ 红；表里留了已删取值 ⇒ 红）；
 *   ② RE-009 §1 的承接方 ↔ 真实目录（填了不存在的文件 / 同一模块被两个取值占用 ⇒ 红）；
 *   ③ 真实目录 ↔ RE-009 §1∪§2（新增模块没归类 ⇒ 红——这正是「悄悄多一个投影」的形态）。
 *
 * `—` 行合法：`eval`（平凡值）、`skill_compile`（写侧函数）、`snapshot`（会话侧）、
 * `display`（已裁决不做）都是**已裁决的现状**，不是待办。本条判覆盖，不判「必须实现」——
 * 要求每个取值都有文件，等于逼着为一个不需要视图的取值造一个空模块。
 *
 * 反向用例（回归测试注入）：加一个 §8 没写的取值 / 把一个承接方改成不存在的文件 /
 * 往目录里塞一个未归类模块 ⇒ 各自变红。
 */
const PARAMS_MD = 'docs/003-design/001-核心架构.md'
const PROJ_DIR = 'symbio/src/symbio_core/projection'
const MAP_MD = 'docs/004-ref/009-投影映射.md'

/** 反引号 token 提取（表格单元格里可能套着 `**` 粗体） */
const ticks = (s) => (s.match(/`([^`]+)`/g) || []).map((t) => t.slice(1, -1).trim())

function auditProjectionMap () {
  if (!existsSync(join(REPO_ROOT, MAP_MD)) || !existsSync(join(REPO_ROOT, PROJ_DIR)) ||
      !existsSync(join(REPO_ROOT, PARAMS_MD))) {
    console.log(dim('（D-014 不适用：映射表 / 投影目录 / 参数表不在扫描范围内）'))
    return null
  }

  // ── ① §8 侧：`| \`projection\` |` 那一行的「取值域」格 ──
  const paramLines = readFileSync(join(REPO_ROOT, PARAMS_MD), 'utf8').split(/\r?\n/)
  const rowIdx = paramLines.findIndex((l) => /^\|\s*`projection`\s*\|/.test(l))
  if (rowIdx < 0) {
    report(PARAMS_MD, 1, 'D-014', '`projection` 那一行没读出来——参数表结构变了，还是键改名了？')
    return null
  }
  const docValues = ticks(paramLines[rowIdx].split('|')[2] || '')
  if (docValues.length < 2) {
    report(PARAMS_MD, rowIdx + 1, 'D-014', `取值域只读出 ${docValues.length} 个取值——分隔符改了？`)
    return null
  }

  // ── ② 表侧：RE-009 §1（取值 → 承接方）与 §2（无取值的模块）──
  const mapLines = readFileSync(join(REPO_ROOT, MAP_MD), 'utf8').split(/\r?\n/)
  const sec2 = mapLines.findIndex((l) => /^##\s*2\./.test(l))
  const inSec1 = (i) => sec2 < 0 || i < sec2
  const tableValues = [] // { value, target, form, line }
  const tableModules = [] // { file, line }
  mapLines.forEach((line, i) => {
    if (!line.startsWith('|')) return
    const cells = line.split('|').slice(1, -1).map((c) => c.trim())
    if (cells.length < 3) return
    if (inSec1(i)) {
      const v = ticks(cells[0])
      if (v.length !== 1) return // 表头 / 分隔行 / 说明行
      const t = ticks(cells[1])
      tableValues.push({ value: v[0], target: t[0] || cells[1], form: cells[2], line: i + 1 })
      if (t.length === 1) tableModules.push({ file: t[0], line: i + 1 })
    } else {
      const m = ticks(cells[0])
      if (m.length !== 1 || !m[0].endsWith('.rs')) return
      tableModules.push({ file: m[0], line: i + 1 })
      if (cells[2].replace(/[—\-\s]/g, '').length < 6) {
        report(MAP_MD, i + 1, 'D-014', `「${m[0]}」的归类理由太短（至少写清为什么不进 §8）`)
      }
    }
  })
  if (tableValues.length === 0) {
    report(MAP_MD, 1, 'D-014', '映射表一行都没读出——表格结构变了？')
    return null
  }

  // ① 取值集合双向相等
  const docSet = new Set(docValues)
  const tblSet = new Set(tableValues.map((r) => r.value))
  for (const v of docSet) {
    if (!tblSet.has(v)) {
      report(MAP_MD, 1, 'D-014', `§8 有取值 \`${v}\`，映射表没有这一行——加了取值必须同时说清谁承接（写 \`—\` 也算说清）`)
    }
  }
  for (const v of tblSet) {
    if (!docSet.has(v)) {
      report(MAP_MD, tableValues.find((r) => r.value === v).line, 'D-014',
        `映射表有取值 \`${v}\`，§8 的 \`projection\` 行没有——表过期了，还是取值被删了没跟着删？`)
    }
  }
  const dupVals = tableValues.map((r) => r.value).filter((v, i, a) => a.indexOf(v) !== i)
  for (const v of new Set(dupVals)) {
    report(MAP_MD, tableValues.find((r) => r.value === v).line, 'D-014', `取值 \`${v}\` 在映射表里出现多次`)
  }

  // ② 承接方 / 归类模块必须真实存在，且一个模块只能承接一个取值
  const dirFiles = readdirSync(join(REPO_ROOT, PROJ_DIR))
    .filter((f) => f.endsWith('.rs') && !f.endsWith('.test.rs'))
    .map((f) => `projection/${f}`)
  const dirSet = new Set(dirFiles)
  const claimed = new Map()
  for (const r of tableValues) {
    if (!r.target.includes('.rs')) continue // `—` 等非文件形态
    if (!dirSet.has(r.target)) {
      report(MAP_MD, r.line, 'D-014', `取值 \`${r.value}\` 的承接方不在投影目录里：\`${r.target}\`——改名了还是写错了？`)
      continue
    }
    if (claimed.has(r.target)) {
      report(MAP_MD, r.line, 'D-014',
        `\`${r.target}\` 同时被 \`${claimed.get(r.target)}\` 与 \`${r.value}\` 认领——一个模块只承接一个取值，多认领说明两行有一行是猜的`)
    }
    claimed.set(r.target, r.value)
  }
  for (const m of tableModules) {
    if (!dirSet.has(m.file)) {
      report(MAP_MD, m.line, 'D-014', `列出的模块不在投影目录里：\`${m.file}\``)
    }
    claimed.set(m.file, claimed.get(m.file) || '§2')
  }

  // ③ 目录侧：每个实现文件都得有归属
  for (const f of dirFiles) {
    if (!claimed.has(f)) {
      report(MAP_MD, 1, 'D-014',
        `投影目录里的 \`${f}\` 没有归属——它承接某个 §8 取值（写进 §1），还是只是读侧函数（写进 §2）？两处都没它 = 悄悄多了一个投影`)
    }
  }

  return {
    values: docValues.length,
    mapped: tableValues.filter((r) => r.target.includes('.rs')).length,
    modules: dirFiles.length,
  }
}

let scanned = 0
let refs = 0
let constRefs = 0

// ── D-012：指路文档写的 CLI 产物路径必须就是构建出来的那一条 ──────────────
//
// 判据对象是**读者会照抄的路径**。这条指路错过一次真事故（相应批次 实测）：文档写
// `../symbio/target/debug/symbio-cli.exe`，而 `cli` 早已是仓库根 workspace 的成员，
// cargo 把所有成员产物写进**同一个** `<仓库根>/target/`——旧位置躺着几个月前的产物，
// 于是整批实测跑在旧二进制上，两条已修的缺陷被当成还在的重复观测到。
//
// 判据的形状（不看目录树、不做「猜 cwd」的归一化）：
//  1. 路径里 `target/<profile>/symbio-cli` 之前**不许出现具名目录段**——仓库根本身就叫
//     `symbio`，`symbio/target/…` 解析之后看着「像对的」，它恰恰是把读者送进旧二进制的那条；
//     具名段一律红。
//  2. 只带 `./` / `../` 前缀的，按该 md 的**命令执行目录**（[`docBase`]：最近的 crate 目录，
//     没有就是仓库根）归一，必须等于 `scripts/cli-binary.mjs` 的 `cliArtifactRelPath` —
//     「CLI 产物在哪」的唯一来路，这里不另起一份拼接（另起一份就是第二个会漂的真相）。
//
// 语料**排除任务登记册**（`docs/009-task`）：那里的条目记的是「当时撞上了什么」，
// 引用一句旧路径正是那条记录的内容本身，不是给读者的指路。
const REGISTER_MD_PREFIXES = ['docs/007-task']
const CLI_ARTIFACT_RE = /(?:[\w.-]+[\\/])*target[\\/](debug|release)[\\/]symbio-cli(?:\.exe)?/g
const CLI_BUILD_SCRIPT_RE = /build-cli\.mjs/

/** 平台后缀不参与判定：判的是「落在哪个目录、哪个二进制」，不是本机 OS。 */
function withoutExe (p) {
  return p.replace(/\\/g, '/').replace(/\.exe$/i, '')
}

function auditCliArtifactPaths () {
  const files = mdFiles.filter((rel) => !REGISTER_MD_PREFIXES.some((p) => rel.startsWith(p)))
  const read = (rel) => readFileSync(join(REPO_ROOT, rel), 'utf8')
  // 「讲怎么构建 CLI」的文档在场，本条才有对象（夹具仓库里没有 ⇒ 不适用，同 D-011）
  const teachFile = files.find((rel) => CLI_BUILD_SCRIPT_RE.test(read(rel)))
  if (!teachFile) {
    console.log(dim('（D-012 不适用：没有现行文档讲到 build-cli.mjs）'))
    return null
  }

  let artifactRefs = 0
  for (const rel of files) {
    const lines = read(rel).split(/\r?\n/)
    lines.forEach((line, i) => {
      const allow = line.match(ALLOW_RE)
      if (allow && allow[1].trim()) return
      if (HISTORY_RE.test(line)) return
      let m
      CLI_ARTIFACT_RE.lastIndex = 0
      while ((m = CLI_ARTIFACT_RE.exec(line)) !== null) {
        artifactRefs++
        const token = m[0].replace(/\\/g, '/')
        const expected = cliArtifactRelPath(m[1])
        // 前缀里出现**具名目录段**（`symbio/target/…`）＝旧布局的写法，单独说清它错在哪
        const named = token.split('/').slice(0, -3).filter((seg) => seg !== '.' && seg !== '..')
        if (named.length) {
          report(rel, i + 1, 'D-012',
            `指路的 CLI 产物路径带着具名目录段：\`${token}\`——构建落点是仓库根的 \`${expected}\`` +
              `（根 workspace 单 target），多出来的 \`${named.join('/')}\` 指向的是旧位置`)
          continue
        }
        if (withoutExe(join(docBase(rel), token)) !== withoutExe(expected)) {
          report(rel, i + 1, 'D-012',
            `指路的 CLI 产物路径不是构建落点：\`${token}\`（本文件的命令以 \`${docBase(rel) || '.'}\`` +
              ` 为执行目录）应写成 \`${expected}\`——照这条抄就用的是旧二进制`)
        }
      }
    })
  }
  // 有人在教构建，却没有一处写出产物落在哪 ⇒ 指路断了（读者只能猜，猜错就是拿旧产物跑）。
  // 这条不是形式主义：读不出路径与「判据脱节」在日志里长得一模一样，必须分开。
  if (artifactRefs === 0) {
    report(teachFile, 1, 'D-012',
      `讲构建的文档（${teachFile}）一处 CLI 产物路径都没写出——指路断了还是这条判据脱节了？`)
    return null
  }
  return { artifactRefs, teachFile }
}

/**
 * 该 md 里命令的**执行目录**：最近的 crate 目录（有 `Cargo.toml` 的那层），没有就是仓库根。
 *
 * 为什么不是「md 自己所在目录」：`cli/docs/004-usage.md` 里的 `node scripts/build-cli.mjs`
 * 是在 `cli/` 下执行的（文档从 crate 根链接过来），拿 `cli/docs` 当基准会把正确的
 * `./../target/…` 算成 `cli/target/…`——判据自己造的假红。
 */
function docBase (rel) {
  let dir = dirname(rel).replace(/\\/g, '/')
  while (dir && dir !== '.' && dir !== '/') {
    if (existsSync(join(REPO_ROOT, dir, 'Cargo.toml'))) return dir
    dir = dirname(dir) === dir ? '' : dirname(dir)
  }
  return ''
}

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

// ── D-011：能力名册的「锚点」列 ─────────────────────────────────────────
//
// 名册是**表**，不是散文：D-005 的逐行反引号扫描管不到「这一列必须填、填了必须指得到」。
// 故单列一条判据，但它**不解析符号**——符号存在性查的就是上面那份 `idents`
// （D-005 的语料），多一份解析就多一处口径漂移。
const roster = auditRosterAnchors()
// ── D-012：CLI 产物指路路径 == 构建落点 ─────────────────────────────────
const cliPaths = auditCliArtifactPaths()
// ── D-014：§8 取值 ↔ 投影模块 三方对账 ─────────────────────────────────
const projMap = auditProjectionMap()

if (errors > 0) {
  console.log(red(`\ndoc-symbol-audit 未通过：${errors} 处失效指认（扫描 ${scanned} 条候选，${mdFiles.length} 篇 md）`))
  process.exit(1)
}
const rosterLine = roster
  ? dim(` · 名册锚点 ${roster.anchored}/${roster.total} 已锚定`)
  : ''
const cliLine = cliPaths
  ? dim(` · CLI 产物指路 ${cliPaths.artifactRefs} 处与构建落点一致`)
  : ''
const projLine = projMap
  ? dim(` · 投影取值 ${projMap.values} 个（${projMap.mapped} 个有模块），${projMap.modules} 个模块全归类`)
  : ''
console.log(green(`\ndoc-symbol-audit 通过：${mdFiles.length} 篇 md、${rsFiles.length} 个 .rs + ${feFiles.length} 个前端语料，${refs} 条符号指认与 ${constRefs} 个常量名全部存在${dim(`（候选 ${scanned} 条）`)}${rosterLine}${cliLine}${projLine}`))
