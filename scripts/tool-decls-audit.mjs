#!/usr/bin/env node
/**
 * tool-decls-audit — 工具自声明行为位（decls）的声明面基线对账（TD-001 / TD-002）
 *
 * ## 它治什么
 *
 * 工具用 `CapabilityMeta.behaviors` 里的行为名**自声明**自己归哪几族闸门管：
 * `assign_work`（淘汰闸）/ `delegation_promise`（承诺记账）/
 * `task_table`（任务表写侧资格闸）。判定读的是**名字**，不认工具名——
 * 于是「该声明的工具漏了声明」与「这工具压根不在管辖面」在代码上长得一样，
 * 三道闸对它**静默不命中**（[] 行为位自声明 / [] ③ 声明面 fail-closed）。
 *
 * 这条判据守的是**声明面本身的封闭性**，不判哪一位声明得对不对（那语义判据
 * 机械化不了）：LLM 可见工具的**名单**是一份从代码提取的事实
 * （`facts/_capability.mjs::toolFacts`，即 `docs/004-ref/006-当前事实表.md` §2.1 的
 * 生成源），声明面基线是另一份**冻结快照**。两份必须逐名相等——工具进了注册表
 * 却没有基线行 ⇒ 声明面从来没有为它过账（**漏声明**）；基线有行而工具不在册
 * ⇒ 账上留着一个已经不存在的工具（**悬空行**）。任一方向都是红。
 *
 * ## 判据
 *
 * | 编号 | 判什么 | 为什么 |
 * |---|---|---|
 * | TD-001 | 工具在册（facts 工具清单）而基线无行 | 新工具进来时声明面没有被过账 |
 * | TD-002 | 基线有行而工具不在册 | 账上的工具已经在代码里找不到了 |
 *
 * ## 反例怎么变红
 *
 *   ① 从 `DEFAULT_BASELINE` 删掉一行（如 `todo_write`）再跑 ⇒ TD-001 红；
 *   ② 往 `DEFAULT_BASELINE` 加一行不存在的工具（如 `ghost_tool`）⇒ TD-002 红；
 *   ③ 工具清单一个都读不出（提取器坏了）⇒ exit 2——**不许「读空即全绿」**：
 *      空集合不是「全部通过」，是判据失效。
 *
 * ## 口径
 *
 * 工具名**只有一份提取实现**（`facts/_capability.mjs::toolFacts`，被 §2.1 与
 * `plugin-special-case-audit` 共读）。本脚本不另写提取器——第二份提取必然与真源
 * 漂移，然后判据开始恒真。
 *
 * 用法：node scripts/tool-decls-audit.mjs [--root=<仓库根>] [--strict]
 *      （`TOOL_DECLS_BASELINE` 环境变量可整体替换基线，回归测试用）
 * 退出码：0 = 两份逐一相等；1 = 有失效；2 = 工具清单读不出。
 */
// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns TD 工具自声明行为位（声明面基线对账）
// @codes TD-001 TD-002

import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'
import { toolFacts } from './facts/_capability.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')

/**
 * 声明面基线：LLM 可见工具的**冻结名单**，一行一个短名。
 *
 * 成员由 `facts/_capability.mjs::toolFacts` 生成（与 `docs/004-ref/006-当前事实表.md`
 * §2.1 同一份提取），在此冻结成账。加一个 LLM 可见工具 = 在此加一行；
 * 删一个 = 在此删一行——两个方向都必须显式过账（TD-001 / TD-002 各守一头）。
 */
const DEFAULT_BASELINE = [
  'agent_run',
  'ask_user',
  'codebase_search',
  'content_search',
  'heartbeat',
  'http_request',
  'notes_add',
  'read_skill',
  'todo_write',
  'vdfs_delete',
  'vdfs_edit',
  'vdfs_list',
  'vdfs_mkdir',
  'vdfs_read',
  'vdfs_search',
  'vdfs_stat',
  'vdfs_tree',
  'vdfs_write',
  'web_fetch',
  'web_search',
]

/** 基线可由环境变量替换（回归测试用）：守卫逻辑与具体工具名解耦。 */
const envBaseline = process.env.TOOL_DECLS_BASELINE
const BASELINE = envBaseline === undefined ? DEFAULT_BASELINE : JSON.parse(envBaseline)

console.log('--- tool-decls-audit: 工具自声明行为位声明面对账（TD-001 / TD-002） ---')

let tools
try {
  tools = toolFacts(ROOT)
} catch (e) {
  console.error(red(`✗ 范围读不出：${e.message}`))
  process.exit(2)
}
if (tools.size === 0) {
  // 与 plugin-special-case-audit 同一条纪律：读不出工具清单 ⇒ 判据一条都不命中却照样绿，
  // 故空集合直接 exit 2，绝不当成「全部通过」。
  console.error(
    red('✗ 失去判据：`facts/_capability.mjs::toolFacts` 一个工具名都没读出来（提取器坏了？）'),
  )
  process.exit(2)
}

let errors = 0
const hits = new Map()
function report(code, where, message) {
  console.log(`${red('[ERROR]')} ${code} ${where}  ${message}`)
  errors++
  hits.set(code, (hits.get(code) ?? 0) + 1)
}

const registered = new Set(tools.keys())
const baseline = new Set(BASELINE)

// ── 双向对账 ─────────────────────────────────────────────────────────────
for (const name of [...registered].sort()) {
  if (!baseline.has(name)) {
    report(
      'TD-001',
      name,
      '工具在册而基线无行 ⇒ 声明面没有为它过账（在 `DEFAULT_BASELINE` 补一行，并核对它的行为位声明）',
    )
  }
}
for (const name of [...baseline].sort()) {
  if (!registered.has(name)) {
    report('TD-002', name, '基线有行而工具不在册 ⇒ 账上留着一个已经不存在的工具（删掉这一行）')
  }
}

// ── 报告 ─────────────────────────────────────────────────────────────────
console.log('')
const codeNames = {
  'TD-001': '工具在册而基线无行（声明面漏账）',
  'TD-002': '基线有行而工具不在册（悬空行）',
}
for (const [code, name] of Object.entries(codeNames)) {
  const n = hits.get(code) ?? 0
  console.log(`  ${n === 0 ? green('✓') : red('✗')} ${code}  ${name}${n ? `  (${n})` : ''}`)
}
console.log(dim(`  （在册 ${registered.size} 个 · 基线 ${baseline.size} 行）`))
console.log('')

if (errors > 0) {
  console.log(red(`✗ tool-decls-audit 未通过：${errors} 处声明面对账失效`))
  process.exit(1)
}
console.log(green(`✓ tool-decls-audit 通过：声明面 ${registered.size} 行与基线逐一相等`))
