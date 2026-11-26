#!/usr/bin/env node
/**
 * facts-boundary-audit — 事实轨的**能力面边界**（判定型）
 *
 * ## 它判什么
 *
 * [](../../docs/003-design/001-核心架构.md "") §1 把「能力可以取得什么」收成五类（A/B/C/D/E），
 * 其中三条硬不变量里最容易被写回去的是**路径**：能力拿不到 store 类型（编译边界已挡），
 * 但只要它还知道「`session_dir` + `V2_WAL_FILE` 拼出事实源」，它就能再 `open` 一次，
 * 而下一个读到这份代码的人会把那个 `open` 当成第二个允许的入口。
 *
 * | 编号 | 规则 | 它拦的是哪一种回潮 |
 * |---|---|---|
 * | FB-001 | 生产 Rust 源码里 `EventWalStore::open` / `open_readonly` 只许出现在 `crates/symbio-plugin-session/src/store/` | 引擎写点绕过 `SessionStore::facts` 自己开档（第二个 `head`、第二把令牌） |
 * | FB-002 |  §4 的能力文件清单里，`session_dir` / `V2_WAL_FILE` / `open_readonly` / `EventWalStore::open` 零命中 | 能力面重新长出「拼路径再读」的知识 |
 * | FB-003 | `submit_fact_source_opener!` 的登记点**恰好 1 处**（生产源码；0 家与 ≥2 家都红） | 开面是单槽（`inventory::iter().next()`）：第二家是静默死码，零家让读面硬错（） |
 * | FB-004 | `AddrRootDecl(` 的声明点**至多 1 处**（0 家绿 + 提示） | 地址根同为单槽：第二家静默（）；零家是「构建里没有 vdfs 插件」的既定降级 |
 *
 * ## 为什么判生产文件而不是全仓
 *
 * 测试夹具**必须**能直接造事实源：`wal.test.rs` 测的就是令牌与归档，`inbox.test.rs`
 * 要往盘上落一轮事实。它们不是能力面，也不参与生产调用链。判据若连夹具一起判，
 * 唯一能让门禁变绿的办法是把夹具也套进 `SessionStore`——那既没测到被测对象，
 * 又让「新测试怎么造事实」变成一条不成文的规矩。 的判据因此写明**生产代码**；
 * 排除口径与 `arch-shape-audit` 同一套（`*.test.rs` / `tests.rs`）。
 *
 * ## 已知边界
 *
 * 判定基于**去注释后的文本**，不是 AST：
 * - 宏拼出来的调用（`concat!` 之类的类型名）看不到；
 * - 路径别名（`use ... as Wal`）能绕过 FB-001——但类型本身仍是
 *   `pub(crate)`，绕过者必须就在会话插件 crate 内，且 FB-002 仍会命中
 *   它拼路径的那一半；
 * - 它挡的是「顺手写回去」，不挡「刻意开新口」——后者要先改 。
 *
 * ## 用法
 *   node scripts/audits/facts-boundary-audit.mjs              # 判定真实仓库
 *   node scripts/audits/facts-boundary-audit.mjs --root=<dir> # 指向别处的仓库（回归测试用）
 * 退出码：0 = 通过；1 = 违规。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns FB 事实轨边界
// @codes FB-001 FB-002 FB-003 FB-004
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from '../color.mjs'
import { isTestFile, walk } from '../fs-walk.mjs'
import { blankComments } from '../rust-scan.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(scriptDir, '..', '..')

/**  §4 的能力文件（DE-014 迁移表「能力自取磁盘」一行；
 *  6b 成片搬家后驱动器侧的文件住各能力包；`rebuild` 随唯一消费方住 session（）。
 *  `elimination` 随  迁入 owner 插件的 `before` 实现：它只读
 *  控制面批首算好的读数，连取事实的入口都没有，故退出本清单。
 *
 *  **2026-11 清单更新（ 轮次投影退场 /  事实身份键改 `scope`）**：
 *  - `loop/src/memory_events.rs` 的轮末派生与巩固并入 `ring/facts.rs`——环只认
 *    五类记账事实，读侧收放不再由会话插件自取；
 *  - `session/src/worker_progress.rs` 随 `turnstate` 投影退场（轮次状态不再是
 *    可读投影，`chat.assistant.progress` 已无生产写方）；
 *  会话记忆本体已随  从 `session` 迁到 `memory`，地址是 `<根>/memory/SESSION.md`，
 *  落位用上下文递来的 `session_dir`（`<会话数据目录>/MEMORY.md`）。它不进本清单：
 *  本清单是  冻结的既有能力文件集合，不是「所有碰磁盘的插件」名单； 把
 *  「会话数据目录」与「事实源路径」分开——`session_dir` 本身不是 FB-002 要拦的形状，
 *  `session_dir + V2_WAL_FILE + open*` 才是。memory 插件里没有 WAL / open 词，
 *  FB-001 仍扫全仓生产源码守住这条边界。改本清单先要有决策：旧路径是「能力自取磁盘」
 *  的实测面，删条目等于关掉读数。 */
const CAPABILITY_FILES = [
  'crates/symbio-plugin-task/src/task.rs',
  'crates/symbio-plugin-tool/src/breaker.rs',
  'crates/symbio-plugin-session/src/stats.rs',
  'crates/symbio-plugin-session/src/rebuild.rs',
  'crates/symbio-plugin-session/src/transcript/inbox.rs',
  'crates/symbio-plugin-session/src/transcript/autonomous.rs',
  'crates/symbio-plugin-session/src/message_events.rs',
  'crates/symbio-plugin-loop/src/ring/facts.rs',
]

const STORE_DIR = 'crates/symbio-plugin-session/src/store/'
const SRC_DIR = 'crates'

/** 冻结的判定面：`EventWalStore` 的构造入口（`open*` 是取写者令牌/开只读档的唯一通道）。 */
const OPEN_TOKENS = ['EventWalStore::open(', 'EventWalStore::open_readonly(']

/** 能力面不得有的存储知识。 */
const CAPABILITY_TOKENS = ['session_dir', 'V2_WAL_FILE', 'open_readonly', 'EventWalStore::open']

/** 事实源开面**唯一**的合法登记点（）。搬到别处不红，但要提示核对  边三。 */
const OPENER_HOME = 'crates/symbio-plugin-session/src/store/fact_source.rs'

const errors = []
const notes = []
const fail = (m) => errors.push(m)

const relPosix = (abs) => path.relative(ROOT, abs).split(path.sep).join('/')

function readBlanked(rel) {
  const abs = path.join(ROOT, rel)
  if (!fs.existsSync(abs)) return null
  // 保留行号：报告里的 `file:line` 要对得上原文件
  return blankComments(fs.readFileSync(abs, 'utf8'))
}

function scanTokens(rel, lines, tokens, code, why) {
  for (let i = 0; i < lines.length; i++) {
    for (const token of tokens) {
      if (lines[i].includes(token)) {
        fail(`${code} ${rel}:${i + 1} 出现 \`${token}\`——${why}`)
      }
    }
  }
}

// ── FB-001：生产侧开档只在存储层 ───────────────────────────────────────
const src = path.join(ROOT, SRC_DIR)
const allProduction = [...walk(src)]
  .filter((abs) => abs.endsWith('.rs'))
  .filter((abs) => !isTestFile(abs))
  .sort()
const production = allProduction.filter((abs) => !relPosix(abs).startsWith(STORE_DIR))

for (const abs of production) {
  const rel = relPosix(abs)
  const text = readBlanked(rel)
  if (text === null) continue
  scanTokens(
    rel,
    text,
    OPEN_TOKENS,
    'FB-001',
    '事实源只经 SessionStore::facts / snapshot 取得（ §2）',
  )
}

// ── FB-002：能力文件清单里零存储知识 ─────────────────────────────────
for (const rel of CAPABILITY_FILES) {
  const text = readBlanked(rel)
  if (text === null) {
    fail(`FB-002 ${rel} 不存在——能力文件清单与仓内结构漂移了（改清单要先有决策： / ）`)
    continue
  }
  scanTokens(
    rel,
    text,
    CAPABILITY_TOKENS,
    'FB-002',
    '能力面 0 条路径（ §1 不变量 2：值由引擎递，不自己拼事实源）',
  )
}

// ── FB-003 / FB-004：两处单槽登记 ─────────────────────────────────────
// `declared_fact_source_opener()` 与 `declared_addr_root()` 都只取 `inventory::iter().next()`，
// 没有去重、没有按域分派。多登记一家，谁生效由链接顺序决定——编译器不报、运行期不报。
// 判的是去注释后的生产源码（全部生产文件，含存储层；测试夹具不算登记）。
const productionLines = allProduction.map((abs) => ({
  rel: relPosix(abs),
  lines: readBlanked(relPosix(abs)) ?? [],
}))
const at = (sites) => sites.map((s) => `${s.rel}:${s.line}`).join(' · ')
const sitesOf = (pred) =>
  productionLines.flatMap((f) =>
    f.lines.flatMap((text, i) => (pred(text) ? [{ rel: f.rel, line: i + 1 }] : [])),
  )

const openerSites = sitesOf((text) => text.includes('submit_fact_source_opener!'))
if (openerSites.length !== 1) {
  const where = openerSites.length > 0 ? `（${at(openerSites)}）` : ''
  fail(
    `FB-003 ${openerSites.length === 0 ? '事实源开面零登记' : `事实源开面是**单槽**，却登记了 ${openerSites.length} 家`}${where}——` +
      (openerSites.length === 0
        ? '`declared_fact_source_opener()` 恒 `None`，读面会硬错；这不是「存储边收窄完成」，是没有人开面了（）'
        : '只取迭代到的第一个，其余是**静默死码**（谁生效由链接顺序决定）；先把重复处置设计出来，再登记第二个（）'),
  )
} else if (openerSites[0].rel !== OPENER_HOME) {
  notes.push(
    `FB-003 事实源开面登记点已不在 session（${at(openerSites)}）——存储换 owner 真的动过，请核对  边三的读数与寄存区补落点`,
  )
}

// `pub struct AddrRootDecl(…)` 的定义行按 `pub struct` 排除；0 家是既定降级（）。
const addrRootSites = sitesOf((text) => /\bAddrRootDecl\s*\(/.test(text) && !/\bpub\s+struct\b/.test(text))
if (addrRootSites.length > 1) {
  fail(
    `FB-004 地址根是**单槽**，却声明了 ${addrRootSites.length} 家（${at(addrRootSites)}）——` +
      '只取迭代到的第一个，其余是**静默死码**；接管地址面前必须先把处置设计出来（）',
  )
} else if (addrRootSites.length === 0) {
  notes.push(
    'FB-004 地址根零声明——`declared_addr_root()` 恒 `None`，虚拟地址面整体降级为相对地址；' +
      '没有 vdfs 插件的构建里是合法降级，但若本次动了地址面，请确认不是把它删掉了（）',
  )
}

// ── 输出 ───────────────────────────────────────────────────────────────
for (const n of notes) console.log(dim(`  · ${n}`))
if (errors.length > 0) {
  for (const e of errors) console.log(red('[ERROR]'), e)
  console.log(dim(`\n共 ${errors.length} 处违规（判据见 docs/003-design/001-核心架构.md §4）`))
  process.exit(1)
}

console.log(
  green('[OK]   '),
  dim(
    `FB-001 生产开档点 0 处越界（扫 ${production.length} 文件）；` +
      `FB-002 能力文件 ${CAPABILITY_FILES.length} 个零存储知识；` +
      `FB-003 开面登记 ${openerSites.length} 家；FB-004 地址根声明 ${addrRootSites.length} 家`,
  ),
)
