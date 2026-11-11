#!/usr/bin/env node
/**
 * session-retire-audit — 控制面 `session` 退役线的进度尺（D-451）
 *
 * ## 它判什么
 *
 * | 码 | 判据 | 范围 |
 * |---|---|---|
 * | SR-001 | 豁免账 `symbols` 里**每一条**都必须真被 `symbio-plugin-session` 用到（去注释后至少命中一次） | `scripts/exemptions/plugin-session-sdk.json` × `crates/symbio-plugin-session/src/**` |
 * | SR-002 | `symbols` 条数 ≤ 基线（**只许降**；要涨必须回 sdk / 内核侧解决，不是往账上加） | 同上 |
 * | SR-003 | 豁免三字段自洽：`expires` 是**将来的**合法日期、`retire` 非空且写明删除动作、`knife` 形如登记号 | 同上 |
 *
 * 违规逐条点名**符号 / 字段**。退出码：0 = 通过；1 = 有违规或判据读不出输入。
 *
 * ## 为什么要有这把尺
 *
 * 「session 退役」在仓里被七处决策引用（`D-349` / `D-358` / `D-413` / `D-416` / `D-427` /
 * `D-428` / `D-438` / `D-443` 都写着「随 session 退役同批删除」），但它自己的范围与出口
 * **从来没有登记过**：它挂的刀（一个任务号）早已随「收口即删」消失，只剩豁免 JSON 里的
 * `expires: 2027-06-30` 一条日期。没有出口的「将来会删」等于不会删——2027-06-30 到点时
 * 能做的只有续期。
 *
 * 退役的终态是**一条依赖边消失**：`symbio-plugin-session → symbio-sdk` 不再需要豁免，
 * 于是那一整份 `symbols` 与它的 `reason` 一起被删。这条边今天由 E-009 冻结成一份
 * **符号级白名单**（新增用法即红、删除不判），所以 `symbols` 的条数天然就是一把只降不升
 * 的尺。本脚本把尺**校准**：先在账本上去余量（SR-001），再钉住条数（SR-002），
 * 最后保证这张账本身不是一张永远不用兑现的欠条（SR-003）。
 *
 * ## 为什么需要 SR-001（实测出来的洞）
 *
 * 2026-11-11 首次量账：116 条里有 **13 条在 session 源码里零命中**（`CAP_DEFINE_WORK` /
 * `CHAT_STYLE_TAG_COMPRESSION` / `DrainedSupplement` / `EffectiveMatrix` / `PRINCIPAL_REFLEX` /
 * `PRINCIPAL_WORKER` / `actor_can_judge_intent` / `branch_hide_unselected` / `closes_turn` /
 * `is_turn_open` / `llm_state_frame` / `pattern_for_principal` / `unresolved_turns`）。E-009
 * 只判「用了的必须在账上」，于是这些条目在用法消失之后**静默留了下来**——账本比真实依赖面
 * 宽 13 条。余量本身无害，**余量冒充进度**有害：它让「退役走了多远」这个问题读出一个
 * 偏大的分母，也让下一个人以为那 13 个能力还挂在 session 身上。
 *
 * ## 已知边界（写在绿灯里，别把它当证明）
 *
 * - **SR-001 是文本判据**：`use … as 别名`、`macro_rules!` 拼出的名字、字符串里的同名文本
 *   都不在语义内（本仓当前两类皆 0）。它偏保守：**误报（说没人用）比漏报便宜**，因为
 *   误报只会让人多看一眼账本，漏报会让余量继续积累。
 * - **SR-001 只扫 `src/`**：插件 crate 的测试与实现同在 `src/`（`*.test.rs`），故「测试里
 *   用过」也算「在用」——E-009 同样把测试判在内，两边口径一致。
 * - **SR-002 的基线是数据、不是判据**：涨了要改这里，而改这里要在提交说明里写清为什么
 *   这条边必须变宽（与 `arch-shape-audit` 的 `core-shape-allow` 同一条纪律）。
 * - **SR-003 不判 `knife` 是否存活**：任务按「收口即删」消失是常态，把存活写进判据会让
 *   两阶之间永远红。它在报告里把 `knife` 打出来，供人确认「这条账还有没有主人」。
 * - **本脚本不判「谁该搬去哪」**：那是 D-451 的阶次与各阶出口，不是一把尺能回答的。
 *
 * 用法：node scripts/session-retire-audit.mjs [--root=<仓库根>] [--verbose]
 * 退出码：0 = 通过；1 = 有违规或读不出输入。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow, dim } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')
const VERBOSE = process.argv.includes('--verbose')

const EXEMPTION_REL = 'scripts/exemptions/plugin-session-sdk.json'
const SESSION_SRC_REL = 'crates/symbio-plugin-session/src'

/**
 * 符号条数基线（**只许降**）。
 *
 * 2026-11-11：116 → **101**，去掉 15 条零命中余量（SR-001 首次量账的结果）。这次下调
 * 不是「退役推进了 13 步」——它只是把账本校准到真实依赖面。真实进度的分母是 101。
 */
const SYMBOL_BASELINE = 101

const errors = []
const notes = []
const report = (rule, msg) => errors.push({ rule, msg })

// ==================== 读输入 ====================

function readJson(rel) {
  const abs = path.join(ROOT, rel)
  if (!fs.existsSync(abs)) {
    report('SR-003', `${rel} 不存在——退役进度尺没有输入，宁红不静默`)
    return null
  }
  try {
    return JSON.parse(fs.readFileSync(abs, 'utf8'))
  } catch (e) {
    report('SR-003', `${rel} 解析失败：${e.message} —— 读不出即判据失效`)
    return null
  }
}

/** 递归收集 `.rs`（跳过 `target` 之类的非源码目录不需要：路径已经钉死在 src）。 */
function collectRs(dir) {
  if (!fs.existsSync(dir)) return null
  const out = []
  const walk = (d) => {
    for (const entry of fs.readdirSync(d, { withFileTypes: true })) {
      const abs = path.join(d, entry.name)
      if (entry.isDirectory()) walk(abs)
      else if (entry.name.endsWith('.rs')) out.push(abs)
    }
  }
  walk(dir)
  return out
}

/**
 * 去注释（`//` 行注释含 `///` / `//!`，以及 C 风格块注释）。
 *
 * 理由与 `port-type-coupling-audit` 同：doc comment 里写「某某已移出」是叙述，不是用法。
 * 不追求字符串里注释符的完美处理（本仓当前 0 处影响本判据）。
 */
function stripComments(text) {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, ' ')
    .split('\n')
    .map((line) => line.replace(/\/\/.*$/, ''))
    .join('\n')
}

// ==================== SR-001 / SR-002 ====================

const exemption = readJson(EXEMPTION_REL)
const files = collectRs(path.join(ROOT, SESSION_SRC_REL))
if (files === null) {
  report('SR-001', `${SESSION_SRC_REL} 不存在——没有源码可量，判据没有输入`)
}

if (exemption && files) {
  if (!Array.isArray(exemption.symbols)) {
    report('SR-002', `${EXEMPTION_REL} 的 \`symbols\` 不是数组——账本形状不对，条数无从判`)
  } else {
    const corpus = files.map((f) => stripComments(fs.readFileSync(f, 'utf8'))).join('\n')
    const stale = exemption.symbols.filter((sym) => {
      // 词边界命中：避免 `PRINCIPAL_REFLEX` 被 `PRINCIPAL_REFLEXIVE` 之外的更长名命中。
      const re = new RegExp(`\\b${sym.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`)
      return !re.test(corpus)
    })
    for (const sym of stale) {
      report(
        'SR-001',
        `\`${sym}\` 在 \`${SESSION_SRC_REL}\` 里零命中——账本比真实依赖面宽，请同批从 ${EXEMPTION_REL} 摘掉（余量冒充进度）`,
      )
    }
    const n = exemption.symbols.length
    if (n > SYMBOL_BASELINE) {
      report(
        'SR-002',
        `豁免符号 ${n} 条 > 基线 ${SYMBOL_BASELINE}——这条边只许收窄（D-451）。` +
          '要加一条，先回答「为什么必须由 session 直取 sdk、而不是由 owner 提供或经门面转出」',
      )
    }
    if (n < SYMBOL_BASELINE) {
      notes.push(
        `符号已收到基线下方（${n} < ${SYMBOL_BASELINE}）：请同步本脚本的 \`SYMBOL_BASELINE\`——` +
          '留着旧数字等于把余量白留给下一次',
      )
    }
    if (VERBOSE) {
      console.log(dim(`  账本 ${n} 条（基线 ${SYMBOL_BASELINE}），源码 ${files.length} 个 .rs，余量 ${stale.length} 条`))
    }
  }
}

// ==================== SR-003 ====================

if (exemption) {
  const { expires, retire, knife } = exemption

  const parsed = typeof expires === 'string' ? Date.parse(`${expires}T00:00:00Z`) : NaN
  if (Number.isNaN(parsed)) {
    report('SR-003', `\`expires\` 不是可解析的日期（${JSON.stringify(expires)}）——没有到期日的豁免就是永久豁免`)
  } else if (parsed <= Date.now()) {
    report('SR-003', `\`expires\`（${expires}）已过期——限期豁免过期即红，续期要有新决策`)
  }

  if (typeof retire !== 'string' || !retire.trim()) {
    report('SR-003', '`retire` 为空——「将来会删」没有写出删除动作，等于不会删')
  } else if (!/删/.test(retire)) {
    report('SR-003', `\`retire\` 没写明删除动作（${JSON.stringify(retire)}）——豁免退场的唯一动作是删`)
  }

  if (typeof knife !== 'string' || !/^[TD]-\d{3}$/.test(knife)) {
    report('SR-003', `\`knife\` 不是登记号（${JSON.stringify(knife)}）——这条账要指得出主人`)
  } else {
    notes.push(`这条账的 \`knife\` 是 ${knife}；任务按「收口即删」会消失，届时请确认主人换成了谁`)
  }

  notes.push(`到期日 ${expires}；${retire}`)
}

// ==================== 报告 ====================

if (errors.length) {
  console.error(`\n== session-retire-audit：${errors.length} 处违规 ==`)
  for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
  console.error(
    `\n${yellow('提示')}：这把尺量的是**依赖边的收窄**（${EXEMPTION_REL} 的符号集），` +
      '不是「搬了多少行」。阶次与各阶出口见 DE-056 的 D-451。',
  )
  process.exit(1)
}

console.log(
  `${green('✓')} SR-001 豁免账零余量：${exemption.symbols.length} 条符号在 session 源码里都有真实用法\n` +
    `${green('✓')} SR-002 只许降：${exemption.symbols.length} 条 ≤ 基线 ${SYMBOL_BASELINE}\n` +
    `${green('✓')} SR-003 豁免自洽：到期日 ${exemption.expires} · ${exemption.retire}`,
)
for (const n of notes) console.log(dim(`  · ${n}`))
process.exit(0)
