#!/usr/bin/env node
/**
 * doc-count-audit — 文档里「少数几个」的那些数必须等于代码（DC-001…003）
 *
 * ## 它治的病
 *
 * `plan/13` 批 D3 / D4 的出口写着「由 M5 + **新增计数守卫**守」「数字由脚本从代码
 * 提取，不再手填」。在本脚本之前，那个守卫**不存在**——于是这两批按现写法执行会
 * 退化成一次性人工对账，正是 `plan/13` 纪律 2 明令不算闭环的形态（纪律 4：出口不许
 * 依赖一个尚不存在的守卫）。
 *
 * 另一半由 `doc-link-audit` 的 D-004 扩展承担：数「从几个变成几个」（`五条 → 七条`）
 * 本身就是变更史，归 `git log`。**本脚本只判「现在是什么」**，不判箭头形态。
 *
 * ## 判据：一个**封闭**的名字集，一个**形状**
 *
 * 只判三个名字，且每个都有唯一的代码来源：
 *
 * | 码 | 名字 | 来源 | 判什么 |
 * |---|---|---|---|
 * | DC-001 | `check_all` 条数 | `symbio_core/invariants/mod.rs` 的函数体 | 活跃文档陈述它有 N 条时，N 必须相等 |
 * | DC-002 | 机制表顶层键数 | `docs/004-plan/verify/facts/mod.rs` 的 `MECHANISMS`（M1 的生成物） | 同上 |
 * | DC-003 | §8 登记的键数（含子键） | 同上文件的 `PARAM_KEYS` | 同上 |
 * | DC-004 | 验证程序数 | `docs/004-plan/verify/*.rs` 的文件数（门禁 55 阶段逐个编译运行的那批） | 同上 |
 * | DC-005 | 事件网格的行 × 列 | 01 §6 的表 vs `symbio_core/event/mod.rs` 的 `Entity` / `Verb` 变体 | 行数 = 实体数、列数 = 动词数 |
 *
 * **DC-004 治的是哪一类漂移**：文档与门禁**读同一批文件**、却各数各的。
 * 一个新程序落进 `verify/` 之后，门禁当天就会跑它，而文档里「N 个验证程序」那句话
 * 没有任何东西跟着变——于是"证据已经变多、声称没变"这件事在日志里与全绿长得一模一样。
 * 数它的唯一来源是文件系统，因为那里就是**实际会被编译**的那一份。
 *
 * **为什么只判这几个**：判据必须可穷举。本仓的文档里有几百个数字（行数、版本、
 * 档位毫秒、事件格），给它们全都配来源就是「写第二份真相」——那正是
 * `docs/README.md` 纪律 3 要防的。而这几个是**机制面的计数**：它们出现在冻结锚点
 * 的描述里（`README §0.1.2` 的「机制表 10 个顶层键」），一旦对不上，「架构面不得
 * 妥协」这句话本身就在骗人。
 *
 * **形状**：`名字` 附近（同一行、限定窗口内）出现「数 + 量词」即判。窗口小是为了
 * 不误伤散文（「check_all 是唯一读出口」这种句子没有数），而**紧邻**才是真正的闸：
 * 数与名字之间只许空白与「的 / 为 / 是」这类连接词，否则「读侧 `check_all` 会把同一条」
 * 里的「一条」会被当成 check_all 的条数（那是 WAL 的计数）。
 *
 * **扫两类文件**：`docs/` 下的活跃文档，加 `symbio/src` 下的 `*.rs` 源码注释。
 * 注释承载的是同一批数字，却比文档更容易漂——改代码的人手边没有「这条注释里写了几条」
 * 这件事，而读注释的人和读代码的人都以为自己看的是真相。判据、窗口、紧邻限定两侧共用一份，
 * 不另立规则（[T-214](../docs/009-task/214-文档口径对账批二.md "T-214-文档口径对账批二") 的 08-5 口径扩展）。
 *
 * **带箭头的一律不判**：数「从几个变成几个」是变更史，归 `doc-link-audit` 的 D-004。
 *
 * 用法：node scripts/doc-count-audit.mjs [--root=<仓库根>] [--strict]
 * 退出码：0 = 通过；1 = 有对不上；2 = 审计范围读不出（来源读不出来即 exit 2，
 *      **不退化成「什么都没检查」**——那正是这类守卫最安静的死法）。
 */
// 判据码命名空间（登记表 docs/006-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns DC 文档计数对账（文档与源码注释里的少数几个数 vs 代码）
// @codes DC-001 DC-002 DC-003 DC-004 DC-005

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? path.resolve(rootArg.slice(7)) : path.resolve(scriptDir, '..')

const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai', '.symbio', 'archive'])
const INVARIANTS_REL = path.join('symbio', 'src', 'symbio_core', 'invariants', 'mod.rs')
const FACTS_REL = path.join('docs', '004-plan', 'verify', 'facts', 'mod.rs')

/** 中文数字 → 数值（本仓文档里的计数一律用汉字，量词是 条 / 项 / 个 / 处 / 格）。 */
const CN_DIGITS = { 零: 0, 一: 1, 二: 2, 两: 2, 三: 3, 四: 4, 五: 5, 六: 6, 七: 7, 八: 8, 九: 9 }
function cnToNum(s) {
  if (/^\d+$/.test(s)) return Number(s)
  if (!/^[零一二两三四五六七八九十]+$/.test(s)) return null
  // 十 / 十二 / 二十 这类
  if (s === '十') return 10
  const m = s.match(/^([一二两三四五六七八九])?十([一二三四五六七八九])?$/)
  if (m) return (m[1] ? CN_DIGITS[m[1]] : 1) * 10 + (m[2] ? CN_DIGITS[m[2]] : 0)
  let n = 0
  for (const ch of s) {
    if (!(ch in CN_DIGITS)) return null
    n = n * 10 + CN_DIGITS[ch]
  }
  return n
}

/** `check_all` 的函数体（大括号配对）里 `all.extend(` 的次数 = 断言条数。
 *  注意第一条是 `let mut all = seq_monotonic(...)`（没有 `extend`），所以**条数 =
 *  extend 次数 + 1**。写成这样是因为它照代码形状数，不认「哪几条算独立断言」。 */
function checkAllCount(root) {
  const abs = path.join(root, INVARIANTS_REL)
  if (!fs.existsSync(abs)) return null
  const txt = fs.readFileSync(abs, 'utf8')
  const i = txt.indexOf('pub fn check_all(')
  if (i < 0) return null
  const open = txt.indexOf('{', i)
  const body = txt.slice(open, txt.indexOf('\n}', open))
  const extends_ = (body.match(/all\.extend\(/g) || []).length
  if (!/\ball\b/.test(body)) return null
  return extends_ + 1
}

/** `facts/mod.rs`（M1 的生成物）里 `MECHANISMS` / `PARAM_KEYS` 的条目数。
 *  **不重新解析 01 §8**：那份表已经由 `gen-verify-facts.mjs` 解析过一次，读它的
 *  生成物才是「提取实现只有一份」（纪律 3）。 */
function factListCounts(root) {
  const abs = path.join(root, FACTS_REL)
  if (!fs.existsSync(abs)) return null
  const txt = fs.readFileSync(abs, 'utf8')
  const countOf = (name) => {
    // 声明形如 `pub const MECHANISMS: &[&str] = &[` ——`=` 与 `[` 之间还有一个 `&`，
// 少写它这条判据就会「读不出规则源」而 exit 2（比误报更安全，但守卫就此失效）。
const m = txt.match(new RegExp(`(?:const|pub const)\\s+${name}\\b[^=]*=\\s*&?\\s*\\[([\\s\\S]*?)\\]`))
    if (!m) return null
    return (m[1].match(/"/g) || []).length / 2
  }
  const mechanisms = countOf('MECHANISMS')
  const paramKeys = countOf('PARAM_KEYS')
  if (mechanisms == null || paramKeys == null) return null
  return { mechanisms, paramKeys }
}

/** `docs/004-plan/verify/` 下的验证程序数——**门禁 55 阶段会逐个编译运行的那一批**。
 *  口径取顶层 `.rs` 文件：`facts/` 是被它们 `mod facts;` 进来的共享数据，
 *  `recursion` 子目录不参与（它们不是独立程序）。 */
function verifyProgramCount(root) {
  const dir = path.join(root, 'docs', '004-plan', 'verify')
  if (!fs.existsSync(dir)) return null
  const n = fs.readdirSync(dir).filter((f) => f.endsWith('.rs') && fs.statSync(path.join(dir, f)).isFile()).length
  return n > 0 ? n : null
}

/** 事件语法网格的两侧：代码侧（枚举变体数）与文档侧（§6 表的行 × 列 × 已填格子）。
 *
 *  **为什么要判它**：网格是「新增事件 = 加一格」这句承诺的载体，而格子数此前没有任何
 *  东西读它——格子从 4×5 长到 11×5 的过程里，这句话一次也没有被检验过。判据不是
 *  「格子不许长」（加格子是本设计预期的演进），而是**加行必须在同一处声明**：
 *  代码枚举多了变体而 §6 表没这行（或反之），就是「两处各记一份账」。
 *
 *  格子数同时是**可观测指标**：门禁每次通过都把 `实体 × 动词 = 格（已填）` 打出来，
 *  「网格在膨胀」这件事因此可被趋势读出，而不是靠谁去数表。 */
function eventGridParity(root) {
  const enumsRel = path.join(root, 'symbio', 'src', 'symbio_core', 'event', 'mod.rs')
  const planRel = path.join(root, 'docs', '004-plan', '001-核心架构.md')
  if (!fs.existsSync(enumsRel) || !fs.existsSync(planRel)) return null
  const src = fs.readFileSync(enumsRel, 'utf8')
  const variants = (name) => {
    const i = src.indexOf(`pub enum ${name} {`)
    if (i < 0) return null
    const body = src.slice(i, src.indexOf('\n}', i))
    return body.split(/\r?\n/).filter((l) => /^ {4}[A-Z]\w*,$/.test(l)).length
  }
  const entities = variants('Entity')
  const verbs = variants('Verb')
  if (!entities || !verbs) return null

  const plan = fs.readFileSync(planRel, 'utf8').split(/\r?\n/)
  const head = plan.findIndex((l) => l.startsWith('| 实体 \\ 动词'))
  if (head < 0) return null
  const cols = plan[head].split('|').filter((c) => c.trim().length > 0).length - 1
  let rows = 0
  let filled = 0
  for (let i = head + 2; i < plan.length && plan[i].startsWith('|'); i++) {
    rows++
    const cells = plan[i].split('|').slice(1, -1)
    filled += cells.slice(1).filter((c) => c.trim() !== '—' && c.trim() !== '').length
  }
  if (!rows) return null
  return { entities, verbs, rows, cols, filled }
}

/** 递归收集活跃文档（`docs/` + 仓根 `*.md`；跳过 archive 与垃圾目录） */
function activeDocs(root) {
  const out = []
  const walk = (dir) => {
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

/**
 * 源码注释侧（DC-006）：`symbio/src` 下的 `*.rs`。
 *
 * 为什么代码注释也在判据里：**它承载的是同一批数字**，却比文档更容易漂——改代码的人
 * 手边没有「这条注释里写了几条」这件事。`invariants/mod.rs` 的模块头当年就写着
 * 「八条」而函数体早已是别的条数，读注释的人按它理解机制，读函数体的人按代码理解，
 * 两边都以为自己看的是真相。
 *
 * 口径与文档侧**同一条规则**（同一个 `RULES`、同一个窗口与标点限定），只是文件集不同：
 * 判据形状为「名字 + 紧邻的 数 + 量词」，注释里那句「至多一条开轮」这类散文因为
 * 数之前跨过标点而被放掉，误报面与文档侧同宽。
 */
function activeComments(root) {
  const out = []
  const walk = (dir) => {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      if (ent.isDirectory()) {
        if (!SKIP_DIRS.has(ent.name)) walk(path.join(dir, ent.name))
        continue
      }
      if (ent.name.endsWith('.rs')) out.push(path.join(dir, ent.name))
    }
  }
  walk(path.join(root, 'symbio', 'src'))
  return out.sort()
}

/**
 * 在一行里找「名字**紧邻**的 数+量词」。窗口取名字前后各 6 个字符，但窗口只是搜索范围，
 * **紧邻**才是判据：
 *
 * - 数在名字**之后**：`check_all` 八条 / `check_all`（八条）——数与名字之间只许有空白与
 *   「的 / 为 / 是 / =」这类连接词；本仓真例 `（\`check_all\` 八条，空 = 全绿）` 里
 *   数**后面**紧跟逗号，那是正常的，判据只看数**之前**那一段。
 * - 数在名字**之前**：`机制表 10 个顶层键`——量词必须紧贴名字，中间不许有别的词。
 *
 * 少了「紧邻」这一层，注释里的散文会混进来：`读侧 \`check_all\` 会把同一条…` 的
 * 「一条」说的是 WAL 的计数，与 `check_all` 有几条无关。这类误报一旦进判据，
 * 天天误报的守卫最后只会被豁免喂死——这是 `grep-audit` S-012 明确拒绝词表判据时给出的教训。
 * 反过来 `顶层键、三条不变量` 里顿号在数**之前**，说明跨过了并列项，抓到的是别的计数，同样放掉。
 *
 * **带 `→` 的一律跳过**：数「从几个变成几个」是变更史，归 D-004。
 */
const RULES = [
  { code: 'DC-001', name: '`check_all`', re: /`check_all`/ },
  { code: 'DC-002', name: '顶层键', re: /顶层键/ },
  { code: 'DC-003', name: '登记的键（含子键）', re: /登记的键（含子键）/ },
  { code: 'DC-004', name: '验证程序', re: /验证程序/ },
]
const WIN = 6
const ARROW_WIN = 12
const COUNT = /([零一二两三四五六七八九十]+|\d+)\s*[（(]?\s*(?:条|项|个|处|格)/
/** 名字与数**之间**不得跨过的标点 */
const PUNCT = /[、，；。：|]/
/** 数在名字之后时，两者之间允许的连接符（空白 + 「的 / 为 / 是 / =」+ 左括号） */
const GLUE_AFTER = /^[\s（(的为是=:：]*$/
/** 箭头常紧跟在这个数后面（叙述一次迁移），那属于 D-004 的形状。窗口取 ARROW_WIN：
 *  13 号自己的 D0 行写着「`check_all` 的 `五条 → 七条`」——数距名字只有 4 字而
 *  箭头在 6 字外，只按 WIN 判就会把它当成「说 check_all 有 5 条」。 */
const hasArrow = (s) => s.includes('→') || s.includes('=>')

/** 数在名字**之后**：取窗口里第一个数，且它与名字之间只有连接符，否则返回 null */
function countAfter(win) {
  const m = win.match(COUNT)
  if (!m) return null
  if (!GLUE_AFTER.test(win.slice(0, m.index))) return null
  return cnToNum(m[1])
}

/** 数在名字**之前**：量词必须紧贴名字（匹配段落在窗口末尾），且数之前不跨过标点 */
function countBefore(win) {
  const m = win.match(COUNT)
  if (!m) return null
  if (win.slice(m.index + m[0].length).trim() !== '') return null
  if (PUNCT.test(win.slice(0, m.index))) return null
  return cnToNum(m[1])
}

function findings(docs, rule, expected) {
  const out = []
  for (const f of docs) {
    const lines = fs.readFileSync(f, 'utf8').split(/\r?\n/)
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i]
      const m0 = line.match(rule.re)
      if (!m0) continue
      const at = m0.index
      const end = at + m0[0].length
      if (hasArrow(line.slice(Math.max(0, at - ARROW_WIN), end + ARROW_WIN))) continue
      // 数在名字**之前**：`机制表 10 个顶层键`；**之后**：`顶层键（10 项）`
      const n = countBefore(line.slice(Math.max(0, at - WIN), at)) ?? countAfter(line.slice(end, end + WIN))
      if (n == null) continue
      if (n !== expected) out.push({ file: f, line: i + 1, said: n, expected, text: line.trim().slice(0, 70) })
    }
  }
  return out
}

console.log('--- doc-count-audit: 文档计数 vs 代码（DC-001…005） ---')

const checkAll = checkAllCount(REPO_ROOT)
const facts = factListCounts(REPO_ROOT)
const verifyPrograms = verifyProgramCount(REPO_ROOT)
const grid = eventGridParity(REPO_ROOT)
if (checkAll == null || facts == null || verifyPrograms == null) {
  console.log(
    red(
      '[ERROR] DC-001 审计范围读不出：' +
        `check_all=${checkAll} facts=${facts ? 'ok' : 'null'} verify=${verifyPrograms}` +
        ' ⇒ 来源读不出来，此时退化成「什么都没检查」是最坏的一种失败',
    ),
  )
  process.exit(2)
}
// 网格两侧的来源在**夹具根**里本就不存在（它们要真实的 core 源码与计划文档）。
// 因此这里不 exit 2，而是**出声跳过**——静默跳过才是这类守卫最安静的死法。
if (grid == null) {
  console.log(`[WARN] DC-005 跳过：读不出事件网格两侧的来源（${path.join('symbio','src','symbio_core','event','mod.rs')} 或 01 §6 表）`)
}
const docs = activeDocs(REPO_ROOT)
const comments = activeComments(REPO_ROOT)
if (docs.length === 0) {
  console.log(red('[ERROR] DC-001 审计范围读不出：`docs/` 下没有活跃文档'))
  process.exit(2)
}

const expected = {
  'DC-001': checkAll,
  'DC-002': facts.mechanisms,
  'DC-003': facts.paramKeys,
  'DC-004': verifyPrograms,
}
let errors = 0
for (const rule of RULES) {
  // 文档与源码注释共用同一条判据：数字的来源只有一个（代码），说它有几条的地方有两类。
  for (const h of findings([...docs, ...comments], rule, expected[rule.code])) {
    errors++
    console.log(
      red(
        `[ERROR] ${rule.code} ${path.relative(REPO_ROOT, h.file)}:${h.line} 说 ${rule.name} 有 ${h.said}，` +
          `而代码里是 ${h.expected} ⇒ 数字手填漂移`,
      ),
    )
    console.log(`       ${h.text}`)
  }
}

if (errors) {
  console.log(`doc-count-audit 未通过：${errors} 处计数对不上（扫 ${docs.length + comments.length} 处声称（文档 ${docs.length} · 源码注释 ${comments.length}）/ ${RULES.length} 个计数）`)
  process.exit(1)
}

// ── DC-005：网格的行 × 列必须与代码枚举逐项相等（加行不许只加在一处）──
if (grid) {
  const gridErrors = []
  if (grid.rows !== grid.entities) {
    gridErrors.push(`01 §6 的实体行数 ${grid.rows} ≠ 代码 Entity 变体数 ${grid.entities}`)
  }
  if (grid.cols !== grid.verbs) {
    gridErrors.push(`01 §6 的动词列数 ${grid.cols} ≠ 代码 Verb 变体数 ${grid.verbs}`)
  }
  if (gridErrors.length) {
    for (const e of gridErrors) console.log(red(`[ERROR] DC-005 ${e} ⇒ 网格两处各记一份账`))
    console.log(`doc-count-audit 未通过：DC-005 ${gridErrors.length} 处网格账不平`)
    process.exit(1)
  }
}

console.log(
  green(
    `doc-count-audit 通过：扫 ${docs.length} 份活跃文档 + ${comments.length} 份源码注释，` +
      `check_all ${checkAll} 条 / 顶层键 ${facts.mechanisms} 个 / 含子键 ${facts.paramKeys} 个 / 验证程序 ${verifyPrograms} 个`,
  ),
)
if (grid) {
  console.log(
    `  事件网格：实体 ${grid.entities} × 动词 ${grid.verbs} = ${grid.entities * grid.verbs} 格，已填 ${grid.filled} 格` +
      `（空格 ${grid.entities * grid.verbs - grid.filled}）——新增事件 = 加一格，加行必须同时出现在 §6 与枚举里`,
  )
}