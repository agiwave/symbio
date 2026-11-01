#!/usr/bin/env node
/**
 * dead-path-audit — **写出来的 crate 名必须存在**（判定型，全仓级）
 *
 * ## 它判什么
 *
 * | 编号 | 规则 | 它拦的是哪一种回潮 |
 * |---|---|---|
 * | DR-001 | 代码与现行文档（`crates/` · `scripts/` · `e2e/` · `tauri/` · `docs/004-ref` · `docs/005-spec` · `docs/006-guide`）里出现**仓内没有的 crate 名**（`symbio_<名字>::` 或 `crates/symbio-<名字>/`） | 改完架构留下的死指针：读者按图索骥找不到东西，而所有判据都绿 |
 * | DR-002 | `docs/003-design/` · `docs/007-task/` 里出现不存在的 crate 名，且同一行**没有历史标记**（曾 / 原先 / 当时 / 迁移 / 删 / 退役 / D-279 …） | 把「过去」写成「现在」：决策记录允许写历史，但不许让历史冒充现状 |
 * | DR-003 | 判据自己的范围读不出（没有 `crates/`、或一个目标文件都没扫到） | 判据的对象消失后扫到 0 条 ⇒ 0 处违规 ⇒ 绿（D-280） |
 *
 * ## 为什么需要它（现场）
 *
 * D-279 把 `symbio_core` 整棵删掉、收成两层之后，留下 **186 处**指向它的路径：其中
 * `crates/**` 139 处（含 30 处指向已删 crate 的**文件路径**形态）、`docs/006-guide`
 * 里有 `use <被删的 crate>::{…}` 这样**可直接照抄**的代码样例。没有任何判据看得见它们—— 
 * `doc-symbol-audit` 只验 `module::symbol` 的最后一段在 Rust 语料里存在（`PLUGIN_PAYLOAD_KEY`
 * 确实存在，只是不在那个 crate 里），`doc-link-audit` 只管相对链接，`arch-shape-audit`
 * 只管架构 crate 的形状。于是「一个不存在的 crate」可以被引用任意多次而全绿。
 *
 * 样例最贵的两处：`docs/006-guide/005-PLUGIN_DEVELOPMENT.md` 让插件作者写
 * 照抄编译不过；而 `crates/symbio-sdk/src/lib.rs` 的头注释还在 
 * 描述「core 保留控制面运行时」，那是这个 crate 自己的门面说明。
 *
 * ## 为什么不点名 `symbio_core`
 *
 * 点名它只修这一次。判据的问法是**「这个名字还能不能打开」**：从源码里抽出所有
 * `symbio_<名字>::` 与 `crates/symbio-<名字>/`，到 `crates/` 下找同名目录。这样下一层
 * 拆分、下一个被删的 crate、以及任何拼错的名字（把 kernel 拼成 kernal）都落在同一条判据里， 
 * 不需要维护一张「已删名单」——那种名单只会越来越长，而漏一项就等于没判。
 *
 * ## 为什么设计文档要有「历史标记」
 *
 * `docs/003-design/` 是**决策记录**：`D-279` 正文必须能写「当时的路径是那个已删的 crate」， 
 * 否则历史被篡改成「一直就是 sdk」。这与 `doc-symbol-audit` 整体排除 `003-design` 是同一
 * 条理由。但允许写历史 ≠ 允许把历史写成现状，所以判据退一格：那两处目录里的每一条死名
 * 都要**自带「这是过去」的标记**。标记是廉价的（一个「曾」字），而它的缺席正是
 * 「读者会把这段当成现状」的那一瞬。
 *
 * ## 用法
 *   node scripts/dead-path-audit.mjs               # 判定真实仓库
 *   node scripts/dead-path-audit.mjs --root=<dir>  # 指向别处的仓库（回归测试用）
 *   node scripts/dead-path-audit.mjs --list        # 逐条列出（默认每条规则前 20 条）
 * 退出码：0 = 通过；1 = 违规；2 = 范围读不出（判不了的判据不许亮绿灯）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns DR 死路径（写出来的 crate 名必须存在）
// @codes DR-001 DR-002 DR-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')
const SHOW_ALL = process.argv.includes('--list')

/** 判 DR-001 的目录：代码与**现行**文档（读者会照抄的地方）。 */
const STRICT_DIRS = ['crates', 'scripts', 'e2e', 'tauri']
/** 判 DR-002 的目录：决策记录与任务账（允许写历史，但必须写明是历史）。 */
const HISTORY_DIRS = ['docs/003-design', 'docs/007-task']
/** 归 DR-001 的文档目录：规范 / 参考 / 指南都描述**现在**。 */
const STRICT_DOC_DIRS = ['docs/004-ref', 'docs/005-spec', 'docs/006-guide']
/** 归档是快照，回改等于篡改历史。 */
const SKIP = ['docs/archive/']

const EXTS = ['.rs', '.md', '.mjs', '.js', '.toml', '.yml', '.yaml', '.ts', '.tsx', '.json']

/**
 * 承认通道：行内 `dead-path-allow: <理由>`（理由不可为空）。
 *
 * 与 `doc-symbol-allow` / `special-case-allow` 同一约定。它存在是因为**判据自己的
 * 文档**必须能写出被禁的名字（「为什么删 `symbio_core`」这句话里就有那个名字），
 * 而把它逼成绕口令只会让判据的说明先烂掉。豁免必须逐行、带理由，且 `--list` 之外
 * 也能 grep 到——它是一条**被看见**的例外，不是一张清单。
 */
const ALLOW_RE = /dead-path-allow:\s*[^\s\]]/

/**
 * 文件级「本文件是决策记录」标记：`dead-path-history: <理由>`。
 *
 * 逐行标记对**决策正文**是错的粒度——`D-253` 那几节每一段都在引用当时的路径，
 * 给十七行各插一句「（当时）」会让判据的说明比它守的东西更难读。文件级标记与逐行
 * 标记在同一条纪律下：**带理由、可 grep、被看见**；它不豁免 DR-001（代码与现行文档
 * 没有这个口子），只把 DR-002 的「历史」判断从行级放宽到文件级。
 */
const FILE_HISTORY_RE = /dead-path-history:\s*\S/
/** `symbio_<名字>::`（Rust 路径形态）与 `` `symbio-<名字>` ``（crate 名形态，反引号内整词）。 */
const PATH_RE = /\bsymbio_([a-z][a-z0-9_]*)::/g
/** `crates/symbio-<名字>/`（文件路径形态，文档与脚本里最常见）。 */
const DIR_RE = /crates\/symbio-([a-z0-9-]+)\//g

/**
 * 历史标记：这条行在说「过去」。
 * 收得宽是有意的——判据要拦的是**把过去写成现在**，不是逼人换一种说法。标记词只要
 * 在行内出现一次就算说明白；真正会被拦下的是「那个已删的 crate 才是权威定义」这种
 * 平静的现状陈述。
 */
const HISTORY_RE = /(曾|原先|原来|当时|此前|之前|迁移|搬迁|搬家|删掉|删除|已删|已并入|消失|不再|退役|转发桩|历史|旧路径|D-279)/

/**
 * 不判的路径：归档（历史快照）与 `*.test.mjs`（回归测试的**夹具**）。
 *
 * 夹具必须能杜撰名字——一个不存在的 `crates/symbio-plugin-<x>/`、一个不存在的 crate 下的模块路径 是
 * 造违规用的道具，不是对仓库的断言。这与 `doc-symbol-audit` 排除 `*.test.mjs` 同一条
 * 理由：测试里为造反例而写的名字若算「存在」，每个假名都会被自己的回归测试洗白。
 */
function shouldSkip(rel) {
  if (SKIP.some((s) => rel.startsWith(s))) return true
  return rel.endsWith('.test.mjs')
}

/** 收集目标文件：目录 → 扩展名过滤。 */
function collect(dir) {
  const out = []
  const abs = path.join(ROOT, dir)
  if (!fs.existsSync(abs)) return out
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      if (e.name === 'target' || e.name === 'node_modules' || e.name === '.git') continue
      const p = path.join(d, e.name)
      if (e.isDirectory()) {
        walk(p)
        continue
      }
      if (!EXTS.some((x) => e.name.endsWith(x))) continue
      const rel = path.relative(ROOT, p).split(path.sep).join('/')
      if (shouldSkip(rel)) continue
      out.push(rel)
    }
  }
  walk(abs)
  return out
}

/** 仓内存在的 crate 名（`crates/symbio-<名字>` 的目录名，去 `symbio-` 前缀）。 */
function existingCrates() {
  const dir = path.join(ROOT, 'crates')
  if (!fs.existsSync(dir)) return null
  const names = fs
    .readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isDirectory() && e.name.startsWith('symbio-'))
    .map((e) => e.name.slice('symbio-'.length))
  // 目录名里 `-` 与 `_` 混用是既成事实（`symbio-provider-vdfs_service`），比对时两边
  // 都抹掉分隔符——判的是「这个名字打不打得开」，不是「拼写是否一致」。
  return names.length ? new Set(names.map((n) => n.replace(/[-_]/g, ''))) : null
}

const crates = existingCrates()
if (crates === null) {
  console.log(`${red('[ERROR]')} dead-path-audit：范围读不出——\`crates/\` 不存在或一个 \`symbio-*\` 都没有`)
  console.log(dim('  DR-003：判不了的名字存在性不许亮绿灯。'))
  process.exit(2)
}

const strictFiles = [...STRICT_DIRS, ...STRICT_DOC_DIRS].flatMap(collect)
const historyFiles = HISTORY_DIRS.flatMap(collect)

if (strictFiles.length === 0 && historyFiles.length === 0) {
  console.log(`${red('[ERROR]')} dead-path-audit：范围读不出——一个目标文件都没扫到`)
  console.log(dim('  DR-003：扫到 0 个文件时的「0 处违规」不是结论，是判据失效（D-280）。'))
  process.exit(2)
}

/** 一条死名（同一行同一条名字只记一次）。 */
function scan(files, { allowHistory }) {
  const hits = []
  let lines = 0
  for (const rel of files) {
    let text
    try {
      text = fs.readFileSync(path.join(ROOT, rel), 'utf8')
    } catch {
      continue
    }
    const fileHistory = FILE_HISTORY_RE.test(text)
    const arr = text.split(/\r?\n/)
    lines += arr.length
    for (const [i, line] of arr.entries()) {
      const names = new Set()
      for (const m of line.matchAll(PATH_RE)) names.add(m[1])
      for (const m of line.matchAll(DIR_RE)) names.add(m[1].replace(/-/g, '_'))
      if (names.size === 0) continue
      const bad = [...names].filter((n) => !crates.has(n.replace(/[-_]/g, '')))
      if (bad.length === 0) continue
      if (ALLOW_RE.test(line)) continue
      if (allowHistory && (fileHistory || HISTORY_RE.test(line))) continue
      hits.push({ rel, line: i + 1, names: bad, text: line.trim() })
    }
  }
  return { hits, lines }
}

const strict = scan(strictFiles, { allowHistory: false })
const history = scan(historyFiles, { allowHistory: true })

function report(hits) {
  const limit = SHOW_ALL ? hits.length : 20
  for (const h of hits.slice(0, limit)) {
    console.log(`  ✗ ${h.rel}:${h.line}  ${h.names.map((n) => `symbio_${n}`).join(', ')}`)
    console.log(dim(`      ${h.text.slice(0, 140)}`))
  }
  if (hits.length > limit) console.log(dim(`  … 另有 ${hits.length - limit} 条（--list 全列）`))
}

console.log(
  `DR-001 现行面：${strictFiles.length} 个文件 / ${strict.lines} 行 · 死名 ${strict.hits.length} 处`,
)
if (strict.hits.length) report(strict.hits)

console.log(
  `DR-002 决策记录：${historyFiles.length} 个文件 / ${history.lines} 行 · 死名（无历史标记） ${history.hits.length} 处`,
)
if (history.hits.length) report(history.hits)

if (strict.hits.length || history.hits.length) {
  console.log('')
  if (strict.hits.length) {
    console.log('DR-001 提示：这个名字在 `crates/` 下打不开——按图索骥的读者会停在半路，')
    console.log('      而所有判据都绿（`doc-symbol-audit` 只验最后一段符号在不在 Rust 语料里，')
    console.log('      它回答不了「在哪个 crate」）。处置：改成今天的 crate，或删掉这句。')
  }
  if (history.hits.length) {
    console.log('DR-002 提示：决策记录允许写历史，但要在同一行说明这是历史（曾 / 原先 / 当时 /')
    console.log('      迁移 / 删 / D-279…）。写不出来的，多半是它其实在讲现状——那就该改路径。')
  }
  console.log(
    red(`\ndead-path-audit 未通过：现行面 ${strict.hits.length} · 决策记录 ${history.hits.length}`),
  )
  process.exit(1)
}

console.log(
  green(
    `dead-path-audit 通过：${strictFiles.length + historyFiles.length} 个文件里写出的 crate 名` +
      `（${crates.size} 个）全部存在`,
  ),
)
