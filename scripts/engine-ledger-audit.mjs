#!/usr/bin/env node
/**
 * engine-ledger-audit — 引擎账本的**可数读数**（EL-001…003）
 *
 * ## 它治的病
 *
 * `DE-044` 给引擎收口立了一条判据：「每一处自持都要回答『事实面答不了吗』」。
 * 但这条判据**只有散文**——`CP-00n` 判形状唯一、`PTS-00n` 判端口不渗引擎、
 * `EX-00n` 判能力名清零，没有一条判**必要性**。于是一个形状唯一但账很厚的引擎
 * （一次调用写 4N 条协议事实、过六站、读十余遍全量日志）是合法的绿。
 *
 * 「为什么留」判不了（那是账本表里人手填的一栏），但**账有多厚**可数。本脚本把
 * 三样冻成只许降的读数——它们不是判据本身，是判据的**代理**：数字涨了，就说明
 * 有人又加了一处自持，得回去回答那句话。
 *
 * ## 三格
 *
 * | 码 | 读数 | 取法 |
 * |---|---|---|
 * | `EL-001` | 引擎自持的行数 | `crates/symbio-plugin-loop/src/**`（生产 `.rs`）里 `Decl {` 的出现次数（行数据已由应用交来，环侧目标 0） |
 * | `EL-002` | 分发方的出参通道数 | `ring/engine.rs` 的 `CollectingWriter` 结构体里 `Mutex<` 字段数 |
 * | `EL-003` | 驱动 / 回合路径的全量读点数 | `ring/engine.rs`、`crates/symbio/src/drive.rs`、`session/plugin/turn.rs` 三处的 `range(…Seq::new(0))` |
 *
 * **账本只有这三格**：`TurnState` 那一格随对象下线一并消失，不再有第四个读数
 * （`CS-010` 同时退役，见 `GATE_CODES.md` 的退役列）。
 *
 * ## 为什么是「只许降」而不是「等于」
 *
 * 等于会把**收窄**也判红——删掉一行、合并一条通道、拿掉一处全量读都是好事，却要
 * 先改基线才能提交，于是改基线成了常规动作，棘轮就失效了。方向只留一个：**变宽即红**，
 * 变窄时打一行提示（可以顺手把基线降下来），不拦提交。
 *
 * ## 失效形态
 *
 * 这一族的老病是「读不出 ⇒ 没什么可判 ⇒ 通过」。三条读法都**先断言读得到**：
 * 文件不在、结构体找不到、一处都匹配不到，都直接 exit 1——绿只说明检查过，
 * 不说明没问题。
 *
 * 用法：node scripts/engine-ledger-audit.mjs [--root=<仓库根>] [--print-baseline]
 * 退出码：0 = 通过（或只有「可以降基线」的提示）；1 = 有读数变宽或读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns EL 引擎收口（DE-044 的账本读数）
// @codes EL-001 EL-002 EL-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')

/**
 * 基线：**只许降**。变宽要改这里，而改这里必须在提交正文里回答「这一处自持事实面答不了吗」
 * ——判据是人，脚本只负责让「悄悄变宽」这件事变得可见。
 *
 * `rows = 0`：行数据由应用经 `DriveRow` 交来（`ring/table.rs` 的 `DebtRow` 只是数据形状），
 * 环不再自持行表；任何一行 `Decl {` 回到 loop 就是引擎又把行收回来了。
 * `channels = 1`：`CollectingWriter` 只有一条跨 await 累积的 `collected: Mutex<…>`；
 * 同步返回的端口结果不是通道。
 * `fullScans = 1`：驱动 / 回合路径只剩 `session/plugin/turn.rs` 的终答投影读一次全量
 * （按 `scope` 找终答事实，事实面没有「按实体取一条」以外的入口）；`ring/engine.rs`
 * 只读当前站的 `FactView`，`crates/symbio/src/drive.rs` 按 `head - 1` 续读，都不从 0 扫。
 */
const BASELINE = { rows: 0, channels: 1, fullScans: 1 }

const LOOP_SRC = 'crates/symbio-plugin-loop/src'
const DISPATCH = 'crates/symbio-plugin-loop/src/ring/engine.rs'
const SCAN_FILES = [
  'crates/symbio-plugin-loop/src/ring/engine.rs',
  'crates/symbio/src/drive.rs',
  'crates/symbio-plugin-session/src/plugin/turn.rs',
]

const read = (rel) => {
  const abs = path.join(ROOT, rel)
  if (!fs.existsSync(abs)) return null
  return fs.readFileSync(abs, 'utf8').replace(/\r\n/g, '\n')
}

/** loop 生产源码里 `Decl {` 的出现次数（行数据应由应用交来）。 */
function countRows() {
  const abs = path.join(ROOT, LOOP_SRC)
  if (!fs.existsSync(abs)) return null
  const files = []
  walk(abs, files, (n) => n.endsWith('.rs') && !n.endsWith('.test.rs'))
  let total = 0
  for (const rel of files) total += (read(rel).match(/^\s*Decl \{/gm) || []).length
  return total
}

function walk(absDir, out, keep) {
  let entries
  try {
    entries = fs.readdirSync(absDir, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of entries) {
    if (e.name.startsWith('.')) continue
    const abs = path.join(absDir, e.name)
    if (e.isDirectory()) {
      if (e.name === 'target' || e.name === 'node_modules') continue
      walk(abs, out, keep)
    } else if (e.isFile() && keep(path.relative(ROOT, abs).replace(/\r/g, ''))) {
      out.push(path.relative(ROOT, abs))
    }
  }
}

/** `CollectingWriter` 结构体里 `Mutex<` 字段数。 */
function countChannels(text) {
  if (text === null) return null
  const m = /\bstruct CollectingWriter \{(.*?)\n\}/s.exec(text)
  if (!m) return null
  const hits = m[1].match(/^\s+\w+: Mutex</gm)
  return hits ? hits.length : 0
}

/** 三处文件里 `range(…Seq::new(0))` 的出现次数。 */
function countFullScans() {
  let total = 0
  const perFile = []
  for (const rel of SCAN_FILES) {
    const text = read(rel)
    if (text === null) return null
    const hits = text.match(/\.range\((?:symbio_sdk::)?Seq::new\(0\)\)/g)
    const n = hits ? hits.length : 0
    perFile.push({ rel, n })
    total += n
  }
  return { total, perFile }
}

function main() {
  const printBaseline = process.argv.includes('--print-baseline')

  const rows = countRows()
  const channels = countChannels(read(DISPATCH))
  const scans = countFullScans()

  if (printBaseline) {
    if (rows === null || channels === null || scans === null) {
      console.error(red('读数读不出——先修文件再打基线'))
      process.exit(1)
    }
    console.log(
      `rows: ${rows}, channels: ${channels}, fullScans: ${scans.total}`,
    )
    for (const f of scans.perFile) console.log(`  ${f.n}  ${f.rel}`)
    return
  }

  // 读不出 ⇒ 红（不是「没什么可判」）。
  const unreadable = []
  if (rows === null) unreadable.push(LOOP_SRC)
  if (channels === null) unreadable.push(`${DISPATCH}（CollectingWriter 结构体）`)
  if (scans === null) unreadable.push(SCAN_FILES.join(' / '))
  if (unreadable.length > 0) {
    console.error(red('EL 读数读不出（绿只说明检查过，不说明没问题）：'))
    for (const u of unreadable) console.error(red(`  ✗ ${u}`))
    process.exit(1)
  }

  const cells = [
    { code: 'EL-001', label: '引擎自持的行数', got: rows, base: BASELINE.rows, hint: 'DE-044 §3：行数据由应用交来——这一行答得出「事实面答不了吗」？' },
    { code: 'EL-002', label: '分发方的出参通道数', got: channels, base: BASELINE.channels, hint: 'DE-044 E-3：通道要跨 await 累积才需要 Mutex；同步返回值不是通道。' },
    { code: 'EL-003', label: '驱动 / 回合路径的全量读点数', got: scans.total, base: BASELINE.fullScans, hint: 'DE-044 E-2：同一站内的 N 笔只读一次；跨站是链式的，不许共用一份幕首切片。' },
  ]

  console.log('--- engine-ledger-audit: 引擎账本的可数读数（EL-001…003）---')

  let widened = 0
  let narrowed = 0
  for (const c of cells) {
    if (c.got > c.base) {
      widened += 1
      console.log(red(`  ✗ ${c.code} ${c.label}：${c.base} → ${c.got}（变宽 ${c.got - c.base}）`))
      console.log(`      ${c.hint}`)
    } else if (c.got < c.base) {
      narrowed += 1
      console.log(yellow(`  ↓ ${c.code} ${c.label}：${c.base} → ${c.got}（收窄了，可以顺手把基线降下来）`))
    } else {
      console.log(green(`  ✓ ${c.code} ${c.label}：${c.got}`))
    }
  }

  if (widened > 0) {
    console.log(red(`engine-ledger-audit 未通过：${widened} 格变宽。变宽不是错，但要在提交正文里回答「事实面答不了吗」。`))
    process.exit(1)
  }

  console.log(
    narrowed > 0
      ? `engine-ledger-audit 通过：3 格无变宽（${narrowed} 格可降基线）`
      : 'engine-ledger-audit 通过：3 格与基线一致',
  )
}

main()
