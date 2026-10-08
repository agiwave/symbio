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
 * | DC-002 | 机制表顶层键数 | `docs/plan/verify/facts/mod.rs` 的 `MECHANISMS`（M1 的生成物） | 同上 |
 * | DC-003 | §8 登记的键数（含子键） | 同上文件的 `PARAM_KEYS` | 同上 |
 *
 * **为什么只判这三个**：判据必须可穷举。本仓的文档里有几百个数字（行数、版本、
 * 档位毫秒、事件格），给它们全都配来源就是「写第二份真相」——那正是
 * `docs/README.md` 纪律 3 要防的。而这三个是**机制面的计数**：它们出现在冻结锚点
 * 的描述里（`README §0.1.2` 的「机制表 10 个顶层键」），一旦对不上，「架构面不得
 * 妥协」这句话本身就在骗人。
 *
 * **形状**：`名字` 附近（同一行、限定窗口内）出现「数 + 量词」即判。窗口小是为了
 * 不误伤散文（「check_all 是唯一读出口」这种句子没有数）。**带箭头的一律不判**
 * ——那是 D-004 的形状，不是本脚本的。
 *
 * 用法：node scripts/doc-count-audit.mjs [--root=<仓库根>] [--strict]
 * 退出码：0 = 通过；1 = 有对不上；2 = 审计范围读不出（来源读不出来即 exit 2，
 *      **不退化成「什么都没检查」**——那正是这类守卫最安静的死法）。
 */
// 判据码命名空间（登记表 docs/reference/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns DC 文档计数对账（文档里的少数几个数 vs 代码）
// @codes DC-001 DC-002 DC-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? path.resolve(rootArg.slice(7)) : path.resolve(scriptDir, '..')

const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai', '.symbio', 'archive'])
const INVARIANTS_REL = path.join('symbio', 'src', 'symbio_core', 'invariants', 'mod.rs')
const FACTS_REL = path.join('docs', 'plan', 'verify', 'facts', 'mod.rs')

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
 * 在一行里找「名字紧邻的 数+量词」。窗口取名字**前后各 6 个字符**，且
 * 只查「数**之前**」那一段，数**之后**的标点是正常的——本仓真例
 * `（\`check_all\` 八条，空 = 全绿）` 里数后面紧跟一个逗号。早先的写法把整个窗口
 * 都判一遍，于是这一行被整条放掉：**改错数字反而不红**（实测注入「七条」判据没反应）。
 * 反过来 `顶层键、三条不变量` 里顿号在数**之前**，说明跨过了并列项，抓到的是别的
 * 计数，必须放掉。
 *
 * 这两条限定是这条判据能不能用的前提：不加就会对**本仓正确的正文**误报
 * （实测会报 `每 turn 至多一条开轮` 与 `三条不变量` 两处），而天天误报的守卫最后
 * 只会被豁免喂死——这是 `grep-audit` S-012 明确拒绝词表判据时给出的教训。
 *
 * **带 `→` 的一律跳过**：数「从几个变成几个」是变更史，归 D-004。
 */
const RULES = [
  { code: 'DC-001', name: '`check_all`', re: /`check_all`/ },
  { code: 'DC-002', name: '顶层键', re: /顶层键/ },
  { code: 'DC-003', name: '登记的键（含子键）', re: /登记的键（含子键）/ },
]
const WIN = 6
const ARROW_WIN = 12
const COUNT = /([零一二两三四五六七八九十]+|\d+)\s*[（(]?\s*(?:条|项|个|处|格)/
/** 名字与数**之间**不得跨过的标点 */
const PUNCT = /[、，；。：|]/
/** 箭头常紧跟在这个数后面（叙述一次迁移），那属于 D-004 的形状。窗口取 ARROW_WIN：
 *  13 号自己的 D0 行写着「`check_all` 的 `五条 → 七条`」——数距名字只有 4 字而
 *  箭头在 6 字外，只按 WIN 判就会把它当成「说 check_all 有 5 条」。 */
const hasArrow = (s) => s.includes('→') || s.includes('=>')

/** 取窗口里那个数；数之前跨过标点就返回 null（宁漏不误报） */
function countIn(win) {
  const m = win.match(COUNT)
  if (!m) return null
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
      const n = countIn(line.slice(Math.max(0, at - WIN), at)) ?? countIn(line.slice(end, end + WIN))
      if (n == null) continue
      if (n !== expected) out.push({ file: f, line: i + 1, said: n, expected, text: line.trim().slice(0, 70) })
    }
  }
  return out
}

console.log('--- doc-count-audit: 文档计数 vs 代码（DC-001…003） ---')

const checkAll = checkAllCount(REPO_ROOT)
const facts = factListCounts(REPO_ROOT)
if (checkAll == null || facts == null) {
  console.log(
    red(
      '[ERROR] DC-001 审计范围读不出：' +
        `check_all=${checkAll} facts=${facts ? 'ok' : 'null'} ⇒ 来源读不出来，` +
        '此时退化成「什么都没检查」是最坏的一种失败',
    ),
  )
  process.exit(2)
}
const docs = activeDocs(REPO_ROOT)
if (docs.length === 0) {
  console.log(red('[ERROR] DC-001 审计范围读不出：`docs/` 下没有活跃文档'))
  process.exit(2)
}

const expected = { 'DC-001': checkAll, 'DC-002': facts.mechanisms, 'DC-003': facts.paramKeys }
let errors = 0
for (const rule of RULES) {
  const hits = findings(docs, rule, expected[rule.code])
  for (const h of hits) {
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
  console.log(`doc-count-audit 未通过：${errors} 处计数对不上（扫 ${docs.length} 份活跃文档 / 3 个计数）`)
  process.exit(1)
}
console.log(
  green(
    `doc-count-audit 通过：扫 ${docs.length} 份活跃文档，` +
      `check_all ${checkAll} 条 / 顶层键 ${facts.mechanisms} 个 / 含子键 ${facts.paramKeys} 个，无手填漂移`,
  ),
)