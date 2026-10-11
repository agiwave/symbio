#!/usr/bin/env node
/**
 * list-write-fact-audit — 列表写入口必须伴随事实
 *
 * [ADR-054](docs/decisions/session.md) 的过渡期强制面。那条 ADR 定的是**目标态**：
 * 消息列表的变更收成闭集原语（诞生 / 断言 / 关闭 / 取代），写侧顺序反转成
 * 「先落事实，列表是折叠出来的」，最终由**类型**保证「改了列表却没落事实」表达不出来。
 * 类型改造要动全部调用点，一次性做完会把 S2.7-b3 的前置与主体揉成一批——所以在类型
 * 到位之前，本守卫顶上。
 *
 * ## 治的病
 *
 * 10 个列表写入口里有 6 个没有对应事实。这个比例不是疏忽，是**机制问题**：没有任何
 * 东西要求「改了列表就必须落事实」，于是每加一条改列表的路径，默认形态就是「忘了」。
 * 症状在 S2.7-b3 切主读路径时爆发——重建只看事实，事实里没有的变更**等于没发生过**，
 * 被截断的消息复活、编辑过的内容回退。
 *
 * ## 判定口径
 *
 * 扫描非测试 Rust 源码里的**入口点调用**（`replace_messages` / `append_and_publish` /
 * `append_messages`，不含函数定义行），每一处都必须有一条**就地登记**：
 *
 *     // list-write-fact <函数名>: <事实符号 | by-design | pending> — <理由>
 *
 *   - **事实符号**（如 `v2_messages::removed`）：必须在调用行 ±40 行内出现——
 *     这不是「证明语义正确」，是「证明写的人确实在这一处落了格」；
 *   - `by-design`：此处**按设计**不落事实（例如「事实由运行器在别处落」）。不进
 *     待补计数，但理由必须非空——「按设计」是这个仓最容易变成谎话的词；
 *   - `pending`：**已知缺口**，进 `PENDING_MAX` 棘轮（只降不升）。
 *
 * 理由非空是硬要求：登记的全部价值就是「这一处为什么这样」，空着就等于没有登记。
 *
 * ## 为什么登记**就地**而不是集中一张表
 *
 * 同 `panic-audit` 的取舍：登记要跟着代码走。集中表的失效形态是**漂移**——改了函数、
 * 搬了调用点，表还在原处指着一个不存在的地方；而就地的登记，代码搬了它跟着搬，
 * 函数改名了它会以 LWF-001（调用点找不到对应登记）变红，逼人重看一眼。
 * 这条纪律本身也是 ADR-054 的理由 3：**靠纪律守的东西迟早会漏**，所以让守卫来判。
 *
 * 用法：
 *   node scripts/list-write-fact-audit.mjs              # 审计 symbio/src
 *   node scripts/list-write-fact-audit.mjs ROOT=<dir>   # 审计指定目录（回归测试用）
 *
 * 退出码：0 = 通过；1 = 发现违规。
 * 平台无关（纯 Node，不依赖 bash / ripgrep），带回归测试（list-write-fact-audit.test.mjs）。
 */

// 判据码命名空间（登记表 docs/reference/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns LWF 列表写入口事实
// @codes LWF-001 LWF-002 LWF-003 LWF-004

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

/** 列表写入口点：把一份消息列表写进存储的函数名。 */
const ENTRY_POINTS = ['replace_messages', 'append_and_publish', 'append_messages']

/** 入口点调用（不是函数定义）。 */
const CALL_RE = new RegExp(`\\b(?:${ENTRY_POINTS.join('|')})\\s*\\(`)
const FN_DEF_RE = new RegExp(`\\bfn\\s+(?:${ENTRY_POINTS.join('|')})\\b`)
/** Rust 函数头（取缩进与名字，用来定位调用点所在的函数）。 */
const FN_HEAD_RE = /^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)/

/** 就地登记：`// list-write-fact <fn>: <kind> — <理由>`（理由不可为空——空着由 LWF-002 判，
 *  所以正则**允许**空理由：不认它会让「写了登记但没写理由」退化成「没写登记」）。 */
const DIRECTIVE_RE = /\/\/\s*list-write-fact\s+([A-Za-z_][A-Za-z0-9_]*)\s*:\s*([^—]*?)\s*—\s*(.*)$/

/** 登记与调用点的最大距离（行）。取宽不取窄：窄了会误报一片，而一条天天误报的守卫
 *  最后只会被人用豁免喂死（同 `panic-audit` 的口径）。 */
const NEAR = 40

/** 待补缺口的上限（棘轮，只降不升）。当前 4 处见各处的 `pending` 登记。 */
const PENDING_MAX = 4

/** 递归收集 `.rs`（测试文件排除：测试是排演驱动方，不受写入口纪律约束）。 */
function walk(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walk(p, out)
    else if (e.name.endsWith('.rs') && !e.name.endsWith('.test.rs')) out.push(p)
  }
  return out
}

/** 一个文件里的入口点调用（含所在函数名）与就地登记。 */
function parseFile(text) {
  const lines = text.split('\n')
  const calls = []
  const directives = []
  lines.forEach((line, i) => {
    const d = DIRECTIVE_RE.exec(line)
    if (d) directives.push({ line: i + 1, fn: d[1], kind: d[2].trim(), reason: d[3].trim() })
    const code = line.split('//')[0]
    if (!CALL_RE.test(code) || FN_DEF_RE.test(code)) return
    const indent = line.length - line.trimStart().length
    let fn = ''
    for (let j = i; j >= 0; j--) {
      const m = FN_HEAD_RE.exec(lines[j])
      if (m && m[1].length <= indent) {
        fn = m[2]
        break
      }
    }
    calls.push({ line: i + 1, fn, text: lines[i].trim() })
  })
  return { calls, directives }
}

function audit(root) {
  const violations = []
  let pending = 0

  for (const file of walk(root)) {
    const rel = path.relative(root, file).split(path.sep).join('/')
    const text = fs.readFileSync(file, 'utf8')
    const lines = text.split('\n')
    const { calls, directives } = parseFile(text)
    const used = new Set()

    for (const call of calls) {
      let best = null
      for (const d of directives) {
        if (used.has(d) || d.fn !== call.fn) continue
        if (Math.abs(d.line - call.line) > NEAR) continue
        if (!best || Math.abs(d.line - call.line) < Math.abs(best.line - call.line)) best = d
      }
      if (!best) {
        violations.push(
          `LWF-001 ${rel}:${call.line}  ${call.fn} 里的列表写入口没有就地登记：${call.text}\n` +
            `        补一行：// list-write-fact ${call.fn}: <事实符号 | by-design | pending> — <理由>`,
        )
        continue
      }
      used.add(best)

      if (!best.reason) {
        violations.push(`LWF-002 ${rel}:${best.line}  登记的理由为空——空着就等于没有登记`)
        continue
      }
      if (best.kind === 'by-design') continue
      if (best.kind === 'pending') {
        pending++
        continue
      }
      if (best.kind.startsWith('fact=')) {
        violations.push(
          `LWF-002 ${rel}:${best.line}  登记的 kind 写法过时：直接写事实符号，不要 \`fact=\` 前缀`,
        )
        continue
      }
      const lo = Math.max(0, call.line - 1 - NEAR)
      const hi = Math.min(lines.length, call.line + NEAR)
      // ⚠️ 窗口里要**剔掉登记行本身**：登记里写着那个事实符号，拿它当「在现场」的
      // 证据就是同义反复——这一条曾被自己的回归测试抓出来（声明 `removed`、代码里
      // 只有 `edited`，却照样绿）。
      const window = lines.slice(lo, hi).filter((l) => !DIRECTIVE_RE.test(l)).join('\n')
      if (!window.includes(best.kind)) {
        violations.push(
          `LWF-002 ${rel}:${call.line}  登记声明的事实「${best.kind}」不在现场（±${NEAR} 行内找不到）`,
        )
      }
    }

    for (const d of directives) {
      if (!used.has(d)) {
        violations.push(
          `LWF-004 ${rel}:${d.line}  登记失效：${d.fn} 里 ±${NEAR} 行内找不到对应的列表写入口` +
            '（函数改名 / 调用点搬走 / 写入口被删都会落到这里）',
        )
      }
    }
  }

  if (pending > PENDING_MAX) {
    violations.push(
      `LWF-003 待补的列表写入口 ${pending} 处 > 基线 ${PENDING_MAX}——新增一处「改了列表却没落事实」` +
        '必须先解释为什么它是**新的一种**（ADR-054：闭集原语，新场景不该需要新原语）',
    )
  }

  return { violations, pending }
}

const rootArg = process.argv.find((a) => a.startsWith('ROOT='))
const root = rootArg ? path.resolve(rootArg.slice(5)) : path.join(repoRoot, 'symbio', 'src')
if (!fs.existsSync(root)) {
  console.error(red(`✗ ROOT 不存在：${root}`))
  process.exit(1)
}
const { violations, pending } = audit(root)
if (violations.length > 0) {
  console.error(red(`✗ 列表写入口事实扫描：${violations.length} 处违规`))
  for (const v of violations) console.error('  ' + v)
  process.exit(1)
}
console.log(
  green(
    `✓ 列表写入口事实扫描：每一处都有就地登记（待补 ${pending}/${PENDING_MAX}，ADR-054）`,
  ),
)
