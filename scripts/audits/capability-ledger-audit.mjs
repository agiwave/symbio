#!/usr/bin/env node
/**
 * capability-ledger-audit — 名册与处置账逐行对账（EX-006）
 *
 * ## 它治的病
 *
 * `DE-008 §6.1` 的 54 行名册是拆分的**完整输入**，`DE-026 §2.3` 的处置账是逐行回答。
 * 两份表之间的每一处漂移都长得像一句正常的话：
 *
 * - 名册加了一行，账里没跟上 ⇒ 那条能力**不存在于拆分计划**，而账本看起来「54/54 齐」；
 * - 账里为一行写了「留」，理由却是「难拆」「今天没有端口」⇒ 留守从**残留类**退化成
 *   一句借口，D-246 的被否项在文本上无法与合法留守区分；
 * - 账里的「拆」指不出 crate ⇒ 「拆」只是一句承诺，S6 / S9 无从分配；
 * - 「拆」写了 crate 却没写交付段 ⇒ 目标存在但没有人认领，S11 收口时才发现；
 * - 合计表与逐行账各数各的 ⇒ 一份是全表、另一份是样本，两个数都「有来源」。
 *
 * 这类漂移没有任何一个既有守卫看得见：`registry-audit` 判登记条目、`doc-symbol-audit`
 * 判反引号里的符号、`doc-section-audit` 判小节地址——都不读**表与表之间**的对账。
 *
 * ## 判据
 *
 * | 码 | 判什么 |
 * |---|---|
 * | `EX-006` | 名册每一行恰好一条处置；账本编号 1..N 唯一；处置在闭集内；「拆」指得出 crate 或同包行、并写明交付段（S6 / S7 / S8 / S9），且目标 crate 在 `crates/` 下真实存在；「留」写明残留类且不是借口、不挂交付段；「不做」写明边界并挂决策号、不挂交付段；「待裁决」写 S10；合计表与逐行账双向一致 |
 *
 * **「不做」是单列一类，不是「拆」的同义词**：能力不存在（或按裁决删除而不换 crate）时，
 * 记 `不做（边界）` 并挂裁决号。把它记成 `拆` 会让完成率虚高——比漏记更危险：漏记少一条，
 * 虚记让后来者以为那件事已经落地。判据对「记为拆、目标 crate 不存在」的行直接判红。
 *
 * 判据读的是**两份文档**，不是代码：账本是计划，名册是输入。两者都对不上时先修账——
 * 数字不能改，改名册或改处置（`DE-008` §6.1 的口径）。
 *
 * ## 失效形态（回归测试钉的是这几条）
 *
 * - 名册表 / 账本表读不出 ⇒ 必须 exit 1（读不出却「没有违规」是最坏的一种绿）；
 * - 按名字对账时把括号说明（`（≈4 槽）`）算作名字的一部分 ⇒ 第一行永远「漏行」，
 *   守卫会被一条假红喂到关掉；
 * - 合计表只数总数不对类别 ⇒ 把一条「拆」写成「留」仍能凑出 54。
 *
 * 用法：node scripts/audits/capability-ledger-audit.mjs [--root=<仓库根>]
 * 退出码：0 = 通过；1 = 有违规（含判据输入读不出）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns EX 能力全面拆分（名册处置与迁移边界）
// @codes EX-006
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red } from '../color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..', '..')

const ROSTER_REL = 'docs/003-design/008-能力注册表与能力坐标系.md'
const LEDGER_REL = 'docs/003-design/026-能力拆分与影响类.md'
const DEPLOY_REL = 'crates/symbio-deploy/src/lib.rs'
const TASKS_REL = 'docs/007-task'

/** 实现状态的三档闭集：设计在不在 / 实现还在不在，是两个字段。 */
const ROSTER_STATUSES = ['done', 'planned', 'planned-unspecified']
const STATUS_COLUMN = '实现状态'

/** 处置闭集：`拆` 的三种限定、`留` 的五个残留类、`撤回`、外加「待裁决」。 */
const DISPOSITIONS = [
  '拆',
  '拆（折叠）',
  '拆（未建）',
  '拆（受限）',
  '留 环',
  '留 写口',
  '留 守门',
  '留 过程体',
  '留 基底',
  '撤回（设计保留）',
  '不做（边界）',
  '待裁决',
]

/** 交付段闭集：拆的三类只能落在这四段；待裁决落裁决段；留守不挂交付段。 */
const SPLIT_STAGES = ['S6', 'S7', 'S8', 'S9']
const PENDING_STAGE = 'S10'

/** D-246 明确否掉的留守理由：出现即红，不论后面接了什么。 */
const EXCUSES = /难拆|拆不动|今天没有端口|没有端口可|实现困难|先放着/

/**
 * D-294 的移除计划：`骨架（T-NNN）：期限 YYYY-MM-DD；触发 <一句话>；承接 T-NNN`。
 *
 * 四处都必填：骨架由哪个任务开出、期限、触发条件、承接任务。分隔符是中文分号与
 * 全角括号——写错一格就是「无计划」，不做宽松兜底（否则三样齐全会变成「能认出
 * 一部分就算」）。
 */
const PLAN_RE = /骨架（(T-\d{3})）：期限 (\d{4}-\d{2}-\d{2})；触发 ([^；|\n]+)；承接 (T-\d{3})/

/** 触发条件里的空话：出现即红。它们不可判定，等于没有触发条件。 */
const PLAN_VACUOUS = /待定|以后|将来|看情况|再说|未定|适时|择机/

/** 触发条件必须带一个可观测谓词；只有名词短语不算「何时发生」。 */
const PLAN_PREDICATE = /新增|出现|落地|进入|完成|达到|超过|至少|≥|<=|<|>/

/** 已收口的状态：承接任务若是这两档，等于没人接。 */
const CLOSED_TASK_STATUS = ['done', 'wont']

/** 编号条目的短号：`T-<三位>-名字` → `T-<三位>`。 */
function shortRef(id) {
  return id.match(/T-\d{3}/)?.[0] ?? null
}

/** front-matter 的键值对（只取首块，够读 `created` / `deps` / `status`）。 */
function frontMatter(text) {
  if (!text) return null
  const m = text.match(/^---\n([\s\S]*?)\n---/)
  if (!m) return null
  const fm = {}
  for (const line of m[1].split('\n')) {
    const i = line.indexOf(':')
    if (i > 0) fm[line.slice(0, i).trim()] = line.slice(i + 1).trim()
  }
  return fm
}

/** `deps: [T-<三位>-…, …]` → 每项的 `T-<三位>` 前缀。 */
function depsOf(fm) {
  if (!fm || !fm.deps) return []
  const inner = fm.deps.replace(/^\[/, '').replace(/\]$/, '')
  return [...inner.matchAll(/T-\d{3}/g)].map((m) => m[0])
}

/**
 * 任务表：只收编号条目（`projection: true` 的汇总页不算）。
 * `docs/007-task` 读不出时返回 null —— 有骨架行却没任务表 = 判据没有一半输入。
 */
function readTasks() {
  const dir = path.join(ROOT, TASKS_REL)
  if (!fs.existsSync(dir)) return null
  const tasks = []
  for (const name of fs.readdirSync(dir)) {
    if (!name.endsWith('.md')) continue
    const rel = `${TASKS_REL}/${name}`
    const fm = frontMatter(readRel(rel))
    if (!fm || fm.projection === 'true') continue
    const short = shortRef(fm.id ?? '')
    if (!short) continue
    tasks.push({ rel, id: fm.id, short, status: fm.status ?? '', deps: depsOf(fm) })
  }
  return tasks
}

/** `start` 沿 `deps` 闭包能否走到 `target`（含 start 自身）。 */
function reaches(start, target, tasks) {
  const seen = new Set()
  const stack = [start]
  while (stack.length) {
    const cur = stack.pop()
    if (cur === target) return true
    if (seen.has(cur)) continue
    seen.add(cur)
    const task = tasks.find((t) => t.short === cur)
    if (!task) continue
    for (const dep of task.deps) stack.push(dep)
  }
  return false
}

function findTask(ref, tasks) {
  return tasks.find((t) => t.short === ref) ?? null
}

function readRel(rel) {
  const file = path.join(ROOT, rel)
  if (!fs.existsSync(file)) return null
  return fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n')
}

function cells(line) {
  return line
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((c) => c.trim())
}

/** 表头含 `needle` 的那一行（用于判列数与列名）。 */
function headerAfter(text, needle) {
  if (text === null) return null
  const lines = text.split('\n')
  const i = lines.findIndex((l) => l.trim().startsWith('|') && l.includes(needle))
  if (i < 0) return null
  return { line: i + 1, cells: cells(lines[i].trim()) }
}

/** 表头含 `needle` 的那张表之后的数据行（跳过分隔行；遇非表格行即止）。 */
function tableAfter(text, needle) {
  if (text === null) return null
  const lines = text.split('\n')
  const start = lines.findIndex((l) => l.trim().startsWith('|') && l.includes(needle))
  if (start < 0) return null
  const rows = []
  for (let i = start + 1; i < lines.length; i++) {
    const line = lines[i].trim()
    if (!line.startsWith('|')) break
    if (/^\|[\s:|-]+\|$/.test(line)) continue
    rows.push({ line: i + 1, cells: cells(line) })
  }
  return rows
}

/** 名字对账口径：去掉括号说明与空白，`工作记忆（≈4 槽）` 与 `工作记忆` 是同一行。 */
function keyOf(name) {
  return name.replace(/（[^）]*）/g, '').replace(/\s+/g, '')
}

function categoryOf(disposition) {
  if (disposition.startsWith('拆')) return '拆'
  if (disposition.startsWith('留')) return '留守'
  if (disposition.startsWith('撤回')) return '撤回'
  if (disposition.startsWith('不做')) return '不做'
  return '待裁决'
}

const errors = []
function report(rel, msg) {
  errors.push({ rel, msg })
}

const ledgerText = readRel(LEDGER_REL)
const rosterText = readRel(ROSTER_REL)
const rosterRows = tableAfter(rosterText, '| 能力 | 维 | 轴 |')
const rosterHeader = headerAfter(rosterText, '| 能力 | 维 | 轴 |')
const ledgerRows = tableAfter(ledgerText, '| # | 能力 | 今天 |')
const totalRows = tableAfter(ledgerText, '| 处置 | 条数 | 组成 |')

let roster = []
let ledger = []
/** 表头是不是「七列 + 锚点 + 实现状态」的形状。形状不对只报一次，不逐行重复。 */
let statusColumnOk = false

if (!rosterRows || rosterRows.length === 0) {
  report(ROSTER_REL, '名册表读不出（`DE-008 §6.1` 的表格头没了）—— 判据没有输入，宁红不静默')
} else {
  statusColumnOk =
    rosterHeader !== null &&
    rosterHeader.cells.length === 9 &&
    rosterHeader.cells[7] === '锚点' &&
    rosterHeader.cells[8] === STATUS_COLUMN
  if (!statusColumnOk) {
    report(
      ROSTER_REL,
      `名册表头必须是「能力 / 维 / 轴 / 天梯 / 等级 / 类型 / 落点 / 锚点 / ${STATUS_COLUMN}」九列` +
        `（实际 ${rosterHeader ? rosterHeader.cells.join(' | ') : '（读不出）'}）—— 没有状态列，` +
        '「设计在、实现没了」只能靠猜',
    )
  }
  roster = rosterRows.map((r) => ({
    line: r.line,
    name: r.cells[0],
    anchor: r.cells[7] ?? '',
    status: (r.cells[8] ?? '').trim(),
  }))
  const seen = new Map()
  for (const r of roster) {
    const key = keyOf(r.name)
    if (!key) {
      report(`${ROSTER_REL}:${r.line}`, '名册行没有能力名')
      continue
    }
    if (seen.has(key)) {
      report(`${ROSTER_REL}:${r.line}`, `名册里「${r.name}」与第 ${seen.get(key)} 行规范化后同名——按名字对账会有一行永远对不上`)
    } else {
      seen.set(key, r.line)
    }
    if (!statusColumnOk) continue
    if (!ROSTER_STATUSES.includes(r.status)) {
      report(
        `${ROSTER_REL}:${r.line}`,
        `「${r.name}」的实现状态 \`${r.status || '（空）'}\` 不在三档（${ROSTER_STATUSES.join(' / ')}）—— ` +
          '状态不是自由文本，新取值先改判据',
      )
      continue
    }
    if (r.status === 'done') {
      const anchor = r.anchor.replace(/`/g, '').trim()
      if (!anchor || anchor === '—') {
        report(
          `${ROSTER_REL}:${r.line}`,
          `「${r.name}」标 done，但锚点是 \`${r.anchor || '（空）'}\` —— 指不出具体 crate / 工具 / 视图，` +
            '「已实现」与「设计在」在日志里会长得一模一样',
        )
        continue
      }
      const file = anchor.split('::')[0].trim()
      if (!fs.existsSync(path.join(ROOT, file))) {
        report(
          `${ROSTER_REL}:${r.line}`,
          `「${r.name}」标 done，但锚点文件 \`${file}\` 不在仓内 —— 实现已经不在，状态却还说 done`,
        )
      }
    }
  }
}

if (!ledgerRows || ledgerRows.length === 0) {
  report(LEDGER_REL, '处置账表读不出（`DE-026 §2.3` 的表格头没了）—— 「54/54」没有可核对的输入')
} else {
  ledger = ledgerRows.map((r) => ({
    line: r.line,
    num: Number(r.cells[0]),
    name: r.cells[1],
    where: r.cells[2],
    disposition: r.cells[3],
    reason: r.cells[4] ?? '',
    stage: (r.cells[5] ?? '').trim(),
  }))
}

if (!totalRows || totalRows.length === 0) {
  report(LEDGER_REL, '账本缺合计表（`DE-026 §2.3.1`）—— 汇总与逐行账必须同源，没有汇总就没有「能拆未拆」的反向读数')
}

// ── 逐行账自洽 ───────────────────────────────────────────────────────

const byNum = new Map()
for (const row of ledger) {
  const at = `${LEDGER_REL}:${row.line}`
  if (!Number.isInteger(row.num) || row.num < 1) {
    report(at, `账本编号 \`${row.cells?.[0] ?? ''}\` 不是正整数`)
  } else if (byNum.has(row.num)) {
    report(at, `账本编号 #${row.num} 重复（先出现在第 ${byNum.get(row.num).line} 行）—— 合计表按编号点名，重复号会同时指两行`)
  } else {
    byNum.set(row.num, row)
  }
  if (!row.name) report(at, '账本行没有能力名')
  if (!row.where) report(at, `#${row.num} 没写「今天」在哪 —— 迁移任务无从分配`)
  if (!DISPOSITIONS.includes(row.disposition)) {
    report(
      at,
      `#${row.num}「${row.name}」的处置 \`${row.disposition}\` 不在闭集（${DISPOSITIONS.join(' / ')}）—— ` +
        '新处置必须先改 D-246，不是在账里多写一个词',
    )
    continue
  }
  if (categoryOf(row.disposition) === '拆') {
    if (!/symbio-plugin-[a-z0-9-]+/.test(row.reason) && !/与 #\d+ 同包/.test(row.reason)) {
      report(
        at,
        `#${row.num}「${row.name}」记为拆，但落点指不出 crate（\`symbio-plugin-…\`）也没有「与 #N 同包」—— ` +
          '「拆」必须能直接分配给 S6 / S7 / S9',
      )
    }
    if (!SPLIT_STAGES.includes(row.stage)) {
      report(
        at,
        `#${row.num}「${row.name}」记为拆，但交付段 \`${row.stage || '（空）'}\` 不在闭集（${SPLIT_STAGES.join(' / ')}）—— ` +
          '拆分目标没有任务认领，S11 收口时才发现',
      )
    }
  } else if (categoryOf(row.disposition) === '留守') {
    if (EXCUSES.test(row.reason)) {
      report(
        at,
        `#${row.num}「${row.name}」的留守理由用了 D-246 的否项（\`${row.reason.match(EXCUSES)[0]}\`）—— ` +
          '「难拆」「今天没有端口」不是留守理由：先补端口，或给出残留类',
      )
    } else if (row.reason.trim().length < 4) {
      report(at, `#${row.num}「${row.name}」留守但没写理由 —— 残留类要说得出「写不进 I-1…I-6」的具体原因`)
    }
    if (row.stage && row.stage !== '—') {
      report(
        at,
        `#${row.num}「${row.name}」记为留守，却挂了交付段 \`${row.stage}\` —— 留守不建任务；` +
          '若它其实要拆，改处置，不要两边都写',
      )
    }
  } else if (categoryOf(row.disposition) === '撤回') {
    if (!row.reason.includes('设计保留 / 实现撤回 / 待重新实现')) {
      report(
        at,
        `#${row.num}「${row.name}」记为撤回，但没写「设计保留 / 实现撤回 / 待重新实现」—— ` +
          '撤回是实现账：设计行仍留在名册，别把它记成删除',
      )
    }
    const scene = row.reason.match(/场景：(.+)$/m)
    if (!scene || scene[1].trim().length < 4) {
      report(
        at,
        `#${row.num}「${row.name}」记为撤回，但没给一句真实使用场景 —— ` +
          '写不出场景的能力应标 planned-unspecified，不是「撤回」',
      )
    }
    if (row.stage && row.stage !== '—') {
      report(
        at,
        `#${row.num}「${row.name}」记为撤回，却挂了交付段 \`${row.stage}\` —— ` +
          '撤回不建段；重新实现要另开任务',
      )
    }
  } else if (categoryOf(row.disposition) === '不做') {
    if (!/D-\d{3}/.test(row.reason)) {
      report(
        at,
        `#${row.num}「${row.name}」记为不做（边界），但理由没挂决策号（\`D-NNN\`）—— ` +
          '「决定不做」必须是裁决，不是一次顺手记；没有决策号，这一栏迟早被当成「已拆」',
      )
    }
    if (row.reason.trim().length < 4) {
      report(at, `#${row.num}「${row.name}」记为不做，但没写边界在哪 —— 要说清「能力不存在 / 已删」与重启条件`)
    }
    if (row.stage && row.stage !== '—') {
      report(
        at,
        `#${row.num}「${row.name}」记为不做，却挂了交付段 \`${row.stage}\` —— 不做不建段；` +
          '若它其实要拆，改处置，不要两边都写',
      )
    }
  } else {
    if (row.stage !== PENDING_STAGE) {
      report(
        at,
        `#${row.num}「${row.name}」记为待裁决，但归宿 \`${row.stage || '（空）'}\` 不是 ${PENDING_STAGE} —— 悬空的待裁决等于没人管`,
      )
    }
    if (row.reason.trim().length < 4) {
      report(at, `#${row.num}「${row.name}」记为待裁决，但没写为什么悬着 —— 下一步无从裁决`)
    }
  }
}

// ── 拆的落点必须真实存在 ─────────────────────────────────────────────
//
// 「记为拆」等于「这个能力已经有新家」。目标 crate 不存在时把它记成拆，完成率就会虚高：
// 漏记少一条，虚记让后来者以为那件事已经落地。`拆（未建）` 是显式的「先记计划」，
// 不在此列；其余三种拆法都要求 `crates/<crate>/Cargo.toml` 真实存在。
const UNBUILT_DISPOSITION = '拆（未建）'
function cratesOf(row, seen = new Set()) {
  const same = row.reason.match(/与 #(\d+) 同包/)
  if (same) {
    const other = byNum.get(Number(same[1]))
    if (other && !seen.has(other.num)) {
      seen.add(row.num)
      return cratesOf(other, seen)
    }
  }
  return [...row.reason.matchAll(/symbio-plugin-[A-Za-z0-9_-]+/g)].map((m) => m[0])
}
for (const row of ledger) {
  if (categoryOf(row.disposition) !== '拆' || row.disposition === UNBUILT_DISPOSITION) continue
  const crates = cratesOf(row)
  if (!crates.length) continue // 落点指不出的情况已在上面的逐行检查里报过
  if (crates.some((c) => fs.existsSync(path.join(ROOT, 'crates', c, 'Cargo.toml')))) continue
  report(
    `${LEDGER_REL}:${row.line}`,
    `#${row.num}「${row.name}」记为 ${row.disposition}，但目标 crate ${crates.map((c) => `\`${c}\``).join(' / ')} 都不存在 —— ` +
      '把「决定不做」记成「已拆」会让完成率虚高；改记 `不做（边界）` 并挂决策号，' +
      '或真的把它建出来',
  )
}

// ── 实现状态 ↔ 默认装配清单 ─────────────────────────────────────────
//
// 「设计保留、实现撤回」与「还在默认装配里」不能同时成立：状态说没实现，装配清单
// 却把它装给每个新会话。判据从处置账取该行的专用包（`与 #N 同包` 沿链找），再对
// `ASSEMBLY_SUB_AGENT_PLUGINS` 的插件 id。**与 done 行同包共享是例外**——同一个 crate
// 里既有已实现的能力又有撤回的能力时，装的是前者，不是后者。
const deployText = readRel(DEPLOY_REL)
function assemblyIds(text) {
  if (!text) return null
  const m = text.match(/ASSEMBLY_SUB_AGENT_PLUGINS[\s\S]*?=\s*&\[([\s\S]*?)\];/)
  if (!m) return null
  return [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1])
}
const assembly = assemblyIds(deployText)
const ledgerByKey = new Map(ledger.map((row) => [keyOf(row.name), row]))
// 设计与实现是两本账：名册说 done、处置账说撤回，必有一本在说假话。
for (const r of roster) {
  if (r.status !== 'done') continue
  const row = ledgerByKey.get(keyOf(r.name))
  if (row && categoryOf(row.disposition) === '撤回') {
    report(
      `${ROSTER_REL}:${r.line}`,
      `「${r.name}」标 done，账本处置却是「实现撤回」—— 设计与实现两本账互相矛盾`,
    )
  }
}
const doneCrates = new Set()
for (const r of roster) {
  if (r.status !== 'done') continue
  // 锚点里的 crate 与处置账里写出的 crate 都算「这一行用的包」：有些已实现的行
  // 锚在 `session` 上，而它的另一半实现住在能力包里（例如 #15 的另一半在 heartbeat）。
  for (const m of r.anchor.matchAll(/crates\/(symbio-plugin-[A-Za-z0-9_-]+)\//g)) doneCrates.add(m[1])
  const row = ledgerByKey.get(keyOf(r.name))
  if (row) for (const crate of cratesOf(row)) doneCrates.add(crate)
}
if (statusColumnOk) {
  if (!assembly) {
    report(
      DEPLOY_REL,
      '默认装配清单读不出（找不到 `ASSEMBLY_SUB_AGENT_PLUGINS`）—— planned 行没有可核对的装配面，' +
        '「实现撤回」与「仍在默认装配」之间的冲突看不见',
    )
  } else {
    for (const r of roster) {
      if (r.status !== 'planned' && r.status !== 'planned-unspecified') continue
      const row = ledgerByKey.get(keyOf(r.name))
      if (!row) continue // 漏行已由名册 ↔ 账本那一段报过
      for (const crate of cratesOf(row)) {
        const id = crate.replace(/^symbio-plugin-/, '')
        if (!assembly.includes(id)) continue
        if (doneCrates.has(crate)) continue
        report(
          `${ROSTER_REL}:${r.line}`,
          `「${r.name}」标 ${r.status}，但专用包 \`${crate}\` 仍在默认装配清单` +
            `（\`ASSEMBLY_SUB_AGENT_PLUGINS\` 里的 \`${id}\`）—— 状态说实现撤回，装配却说每个新会话都装它`,
        )
      }
    }
  }
}

// ── D-294 移除计划 ───────────────────────────────────────────────────

const plannedRows = ledger.filter((row) => row.reason.includes('骨架'))
if (plannedRows.length) {
  const fm = frontMatter(ledgerText)
  const created = fm?.created ?? ''
  const tasks = readTasks()
  if (!created) {
    report(LEDGER_REL, 'D-294 的骨架计划要对比期限，但账本 front-matter 读不出 `created` —— 期限无从判「还来不来得及」')
  }
  if (tasks === null) {
    report(LEDGER_REL, 'D-294 的骨架计划要接任务，但 `docs/007-task` 读不出 —— 计划没有承接面')
  }
  for (const row of plannedRows) {
    const at = `${LEDGER_REL}:${row.line}`
    if (categoryOf(row.disposition) !== '拆') {
      report(
        at,
        `#${row.num}「${row.name}」写了骨架计划，处置却是 \`${row.disposition}\` —— ` +
          '骨架是「先留一层、到期拆掉」的拆法，不是留守',
      )
    }
    const m = PLAN_RE.exec(row.reason)
    if (!m) {
      report(
        at,
        `#${row.num}「${row.name}」写了骨架但没有可判计划 —— 格式必须是 ` +
          '`骨架（T-NNN）：期限 YYYY-MM-DD；触发 <一句话>；承接 T-NNN`，四处缺一即「无计划」',
      )
      continue
    }
    const [, owner, deadline, trigger, successor] = m
    if (Number.isNaN(Date.parse(deadline))) {
      report(at, `#${row.num}「${row.name}」的期限 \`${deadline}\` 不是合法日期`)
    } else if (created && deadline <= created) {
      report(
        at,
        `#${row.num}「${row.name}」的期限 ${deadline} 不晚于账本登记日 ${created} —— ` +
          '期限已经或当场到期，等于没有期限',
      )
    }
    const t = trigger.trim()
    if (t.length < 4) {
      report(at, `#${row.num}「${row.name}」的骨架没有触发条件 —— 没有「何时必须拆」就没有移除计划`)
    } else if (PLAN_VACUOUS.test(t)) {
      report(
        at,
        `#${row.num}「${row.name}」的触发条件用了不可判定的词（\`${t.match(PLAN_VACUOUS)[0]}\`）—— ` +
          '「以后」「看情况」不是触发条件',
      )
    } else if (!PLAN_PREDICATE.test(t)) {
      report(
        at,
        `#${row.num}「${row.name}」的触发条件没有可观测谓词 —— 要写成「出现/新增/进入/达到…」的事实形状`,
      )
    }
    if (successor === owner) {
      report(at, `#${row.num}「${row.name}」的承接任务就是骨架宿主 ${owner} —— 自己接自己等于没人接`)
    }
    if (tasks !== null) {
      const ownerTask = findTask(owner, tasks)
      const successorTask = findTask(successor, tasks)
      if (!ownerTask) {
        report(at, `#${row.num}「${row.name}」的骨架宿主 ${owner} 不在 \`docs/007-task\` 里`)
      } else if (CLOSED_TASK_STATUS.includes(ownerTask.status)) {
        report(at, `#${row.num}「${row.name}」的骨架宿主 ${owner} 已是 \`${ownerTask.status}\` —— 收口的任务不再承接计划`)
      }
      if (!successorTask) {
        report(at, `#${row.num}「${row.name}」的承接任务 ${successor} 不在 \`docs/007-task\` 里 —— 计划没有接棒的人`)
      } else if (CLOSED_TASK_STATUS.includes(successorTask.status)) {
        report(
          at,
          `#${row.num}「${row.name}」的承接任务 ${successor} 已是 \`${successorTask.status}\` —— ` +
            '「以后删」的承诺落在一条已收口的任务上',
        )
      } else if (ownerTask && !reaches(successor, owner, tasks)) {
        report(
          at,
          `#${row.num}「${row.name}」的承接任务 ${successor} 的 \`deps\` 闭包到不了骨架宿主 ${owner} —— ` +
            '承接任务必须声明它接的是这份骨架',
        )
      }
    }
  }
}

if (ledger.length) {
  const nums = [...byNum.keys()].sort((a, b) => a - b)
  for (let n = 1; n <= ledger.length; n++) {
    if (!byNum.has(n)) report(LEDGER_REL, `账本编号缺 #${n}（1..${ledger.length} 必须连续，合计表按号点名）`)
  }
  if (nums.length && nums[nums.length - 1] > ledger.length) {
    report(LEDGER_REL, `账本编号最大 ${nums[nums.length - 1]} 大于行数 ${ledger.length} —— 编号与行数对不上`)
  }
}

// ── 名册 ↔ 账本 ──────────────────────────────────────────────────────

if (roster.length && ledger.length) {
  const rosterKeys = new Set(roster.map((r) => keyOf(r.name)))
  const counts = new Map()
  for (const row of ledger) counts.set(keyOf(row.name), (counts.get(keyOf(row.name)) ?? 0) + 1)
  for (const r of roster) {
    const k = keyOf(r.name)
    const n = counts.get(k) ?? 0
    if (n === 0) {
      report(LEDGER_REL, `名册「${r.name}」（${ROSTER_REL}:${r.line}）没有处置行 —— 漏一行，账本就不是「54/54」`)
    } else if (n > 1) {
      report(LEDGER_REL, `名册「${r.name}」有 ${n} 条处置行 —— 一行能力只能有一条处置`)
    }
  }
  for (const row of ledger) {
    if (!rosterKeys.has(keyOf(row.name))) {
      report(`${LEDGER_REL}:${row.line}`, `账本里的「${row.name}」不在 \`DE-008\` §6.1 名册里 —— 凭空多一行，或者名字拼错`)
    }
  }
}

// ── 合计表 ↔ 逐行账 ─────────────────────────────────────────────────

let splitCount = 0
let residueCount = 0
let withdrawCount = 0
let boundaryCount = 0
let pendingCount = 0
for (const row of ledger) {
  const c = categoryOf(row.disposition)
  if (c === '拆') splitCount++
  else if (c === '留守') residueCount++
  else if (c === '撤回') withdrawCount++
  else if (c === '不做') boundaryCount++
  else pendingCount++
}

if (totalRows && totalRows.length) {
  const claimed = new Map() // 编号 → 合计表给的类别
  const sums = { 拆: 0, 留守: 0, 撤回: 0, 不做: 0, 待裁决: 0 }
  for (const r of totalRows) {
    const at = `${LEDGER_REL}:${r.line}`
    const label = r.cells[0] ?? ''
    const count = Number(r.cells[1])
    const list = r.cells[2] ?? ''
    const cat = label.startsWith('拆')
      ? '拆'
      : label === '留守'
        ? '留守'
        : label.startsWith('撤回')
          ? '撤回'
          : label.startsWith('不做')
            ? '不做'
            : label === '待裁决'
              ? '待裁决'
              : null
    if (!cat) {
      report(at, `合计表有一行「${label}」不属于 拆 / 留守 / 撤回 / 不做 / 待裁决 —— 汇总类别必须是这五类`)
      continue
    }
    if (!Number.isInteger(count) || count < 0) {
      report(at, `合计表「${label}」的条数 \`${r.cells[1]}\` 不是非负整数`)
      continue
    }
    const ids = [...list.matchAll(/\d+/g)].map((m) => Number(m[0]))
    if (ids.length !== count) {
      report(at, `合计表「${label}」写 ${count} 条，组成列点名 ${ids.length} 个编号 —— 条数与名单不同源`)
    }
    sums[cat] += count
    for (const id of ids) {
      if (claimed.has(id)) {
        report(at, `编号 #${id} 在合计表里出现两次（${claimed.get(id)} / ${cat}）—— 一行能力只能归一类`)
        continue
      }
      claimed.set(id, cat)
      const row = byNum.get(id)
      if (!row) {
        report(at, `合计表点名 #${id}，逐行账里没有这个编号`)
      } else if (categoryOf(row.disposition) !== cat) {
        report(
          at,
          `合计表把 #${id}「${row.name}」归为 ${cat}，逐行账的处置是 \`${row.disposition}\` —— ` +
            '汇总与逐行账必须指向同一个处置',
        )
      }
    }
  }
  for (const n of byNum.keys()) {
    if (!claimed.has(n)) report(LEDGER_REL, `#${n} 没有进合计表 —— 合计不是全表的和，是一个样本`)
  }
  const pairs = [
    ['拆', splitCount],
    ['留守', residueCount],
    ['撤回', withdrawCount],
    ['不做', boundaryCount],
    ['待裁决', pendingCount],
  ]
  for (const [cat, actual] of pairs) {
    if (sums[cat] !== actual) {
      report(LEDGER_REL, `合计表的 ${cat} 小计 ${sums[cat]} ≠ 逐行账的 ${actual} —— 改一行处置必须同批改合计`)
    }
  }
}

// ── 输出 ────────────────────────────────────────────────────────────

const statusSummary = ROSTER_STATUSES.map(
  (s) => `${s} ${roster.filter((r) => r.status === s).length}`,
).join(' / ')

if (errors.length) {
  for (const e of errors) console.log(red(`[ERROR] EX-006 ${e.rel}: ${e.msg}`))
  console.log(
    red(
      `capability-ledger-audit 未通过：${errors.length} 处 —— 名册 ${roster.length} 行 / 账本 ${ledger.length} 行` +
        `（拆 ${splitCount} · 留守 ${residueCount} · 撤回 ${withdrawCount} · 不做 ${boundaryCount} · 待裁决 ${pendingCount}）`,
    ),
  )
  process.exit(1)
}

console.log(
  green(
    `capability-ledger-audit 通过：名册 ${roster.length} 行 = 账本 ${ledger.length} 行` +
      `（拆 ${splitCount} · 留守 ${residueCount} · 撤回 ${withdrawCount} · 不做 ${boundaryCount} · 待裁决 ${pendingCount}）；` +
      `实现状态 ${statusSummary}；` +
      '每行恰好一条处置，合计表与逐行账一致',
  ),
)
