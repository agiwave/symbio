#!/usr/bin/env node
/**
 * facts-boundary-audit — 事实轨的**能力面边界**（判定型）
 *
 * ## 它判什么
 *
 * [D-084](../docs/003-design/001-核心架构.md "D-084") §1 把「能力可以取得什么」收成五类（A/B/C/D/E），
 * 其中三条硬不变量里最容易被写回去的是**路径**：能力拿不到 store 类型（编译边界已挡），
 * 但只要它还知道「`session_dir` + `V2_WAL_FILE` 拼出事实源」，它就能再 `open` 一次，
 * 而下一个读到这份代码的人会把那个 `open` 当成第二个允许的入口。
 *
 * | 编号 | 规则 | 它拦的是哪一种回潮 |
 * |---|---|---|
 * | FB-001 | 生产 Rust 源码里 `EventWalStore::open` / `open_readonly` 只许出现在 `plugins/session/store/` | 引擎写点绕过 `SessionStore::facts` 自己开档（第二个 `head`、第二把令牌） |
 * | FB-002 | D-084 §4 的十二个能力文件里，`session_dir` / `V2_WAL_FILE` / `open_readonly` / `EventWalStore::open` 零命中 | 能力面重新长出「拼路径再读」的知识 |
 *
 * ## 为什么判生产文件而不是全仓
 *
 * 测试夹具**必须**能直接造事实源：`wal.test.rs` 测的就是令牌与归档，`inbox.test.rs`
 * 要往盘上落一轮事实。它们不是能力面，也不参与生产调用链。判据若连夹具一起判，
 * 唯一能让门禁变绿的办法是把夹具也套进 `SessionStore`——那既没测到被测对象，
 * 又让「新测试怎么造事实」变成一条不成文的规矩。D-084 的判据因此写明**生产代码**；
 * 排除口径与 `core-shape-audit` 同一套（`*.test.rs` / `tests.rs`）。
 *
 * ## 已知边界
 *
 * 判定基于**去注释后的文本**，不是 AST：
 * - 宏拼出来的调用（`concat!` 之类的类型名）看不到；
 * - 路径别名（`use ... as Wal`）能绕过 FB-001——但类型本身仍是
 *   `pub(in crate::plugins::session)`，绕过者必须就在会话插件内，且 FB-002 仍会命中
 *   它拼路径的那一半；
 * - 它挡的是「顺手写回去」，不挡「刻意开新口」——后者要先改 D-084。
 *
 * ## 用法
 *   node scripts/facts-boundary-audit.mjs              # 判定真实仓库
 *   node scripts/facts-boundary-audit.mjs --root=<dir> # 指向别处的仓库（回归测试用）
 * 退出码：0 = 通过；1 = 违规。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns FB 事实轨边界
// @codes FB-001 FB-002

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'
import { isTestFile, walk } from './core-surface.mjs'
import { blankComments } from './rust-scan.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(scriptDir, '..')

/** D-084 §4 的十二个能力文件（DE-014 迁移表「能力自取磁盘」一行；
 *  6b 成片搬家后驱动器侧的文件住 `plugins/loop/`，`rebuild` 住 core schema）。 */
const CAPABILITY_FILES = [
  'crates/symbio-plugin-loop/src/skill_events.rs',
  'crates/symbio-plugin-loop/src/task_events.rs',
  'crates/symbio-plugin-loop/src/memory_events.rs',
  'crates/symbio-plugin-session/src/worker_progress.rs',
  'crates/symbio-plugin-loop/src/tools/tool_executor/breaker.rs',
  'crates/symbio-plugin-loop/src/tools/tool_executor/elimination.rs',
  'crates/symbio-plugin-session/src/stats.rs',
  'crates/symbio-core/src/projection/rebuild.rs',
  'crates/symbio-plugin-session/src/transcript/inbox.rs',
  'crates/symbio-plugin-session/src/transcript/autonomous.rs',
  'crates/symbio-plugin-session/src/message_events.rs',
  'crates/symbio-plugin-loop/src/derived_facts.rs',
]

const STORE_DIR = 'crates/symbio-plugin-session/src/store/'
const SRC_DIR = 'crates'

/** 冻结的判定面：`EventWalStore` 的构造入口（`open*` 是取写者令牌/开只读档的唯一通道）。 */
const OPEN_TOKENS = ['EventWalStore::open(', 'EventWalStore::open_readonly(']

/** 能力面不得有的存储知识。 */
const CAPABILITY_TOKENS = ['session_dir', 'V2_WAL_FILE', 'open_readonly', 'EventWalStore::open']

const errors = []
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
const production = [...walk(src)]
  .filter((abs) => abs.endsWith('.rs'))
  .filter((abs) => !isTestFile(abs))
  .filter((abs) => !relPosix(abs).startsWith(STORE_DIR))
  .sort()

for (const abs of production) {
  const rel = relPosix(abs)
  const text = readBlanked(rel)
  if (text === null) continue
  scanTokens(
    rel,
    text,
    OPEN_TOKENS,
    'FB-001',
    '事实源只经 SessionStore::facts / snapshot 取得（D-084 §2）',
  )
}

// ── FB-002：十二个能力文件里零存储知识 ─────────────────────────────────
for (const rel of CAPABILITY_FILES) {
  const text = readBlanked(rel)
  if (text === null) {
    fail(`FB-002 ${rel} 不存在——能力文件清单与仓内结构漂移了（改清单要先改 D-084 §4）`)
    continue
  }
  scanTokens(
    rel,
    text,
    CAPABILITY_TOKENS,
    'FB-002',
    '能力面 0 条路径（D-084 §1 不变量 2：值由引擎递，不自己拼事实源）',
  )
}

// ── 输出 ───────────────────────────────────────────────────────────────
if (errors.length > 0) {
  for (const e of errors) console.log(red('[ERROR]'), e)
  console.log(dim(`\n共 ${errors.length} 处违规（判据见 docs/003-design/001-核心架构.md §4）`))
  process.exit(1)
}

console.log(
  green('[OK]   '),
  dim(
    `FB-001 生产开档点 0 处越界（扫 ${production.length} 文件）；` +
      `FB-002 能力文件 ${CAPABILITY_FILES.length} 个零存储知识`,
  ),
)
