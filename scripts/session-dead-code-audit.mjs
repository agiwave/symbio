#!/usr/bin/env node
/**
 * session-dead-code-audit — **B 类死码以台账为唯一判据**（判定型）
 *
 * | 编号 | 规则 | 它拦的是哪一种回潮 |
 * |---|---|---|
 * | SD-001 | `cargo check --workspace --all-targets` 报出的每一条 `dead_code` 都必须在 `scripts/exemptions/session-dead-code.json` 里有一行（按 `path` + 符号名）；台账里的每一行也必须在源码里仍然报 `dead_code` | 两头都会漂：源码悄悄长出新的死码（clippy 已按 D-331 让开 `dead_code`），或台账留着已经有消费者的旧行（一行永久豁免变成一张过期地图） |
 *
 * 四条红线（D-331）：
 *
 * 1. **源码有、台账没有** ⇒ 红。新死码不许静默存在。
 * 2. **台账有、源码没有** ⇒ 红。符号已有消费者或已不存在 ⇒ 删行。
 * 3. **某行所属批次已写进 `landed_batches`** ⇒ 红。批次落地即销账。
 * 4. **源码里出现 `#[allow(dead_code)]` / `#![allow(dead_code)]`** ⇒ 红。
 *    台账是唯一的豁免形态；源码内 allow 会把该符号从判据的输入里抹掉。
 *
 * 台账为空且源码无 `dead_code` = **能力全部接入完成**（绿）。台账不得长期保留：
 * 每一行都是「这项能力还没接回来」的欠条，不是保险。
 *
 * ## 为什么用 `cargo check` 而不是 clippy
 *
 * 后端阶段的 clippy 带 `-A dead_code`（D-331）：clippy 判的是其余 lint，`dead_code`
 * 的豁免/销账归本脚本。若本脚本也用 clippy 取诊断，就得再跑一遍 clippy；而
 * `cargo check --all-targets` 的 target 集合与 `clippy --all-targets` 相同，缓存虽
 * 不复用，但语义输入完全一致。
 *
 * ## 为什么按符号而不是按告警分组
 *
 * 同一条符号在 lib 与 lib-test 两个 target 里的告警分组不同（测试构造了的变体在
 * lib 里仍报、在 test 里可能不报）。按 `path::名字` 建集合、跨 target 去重，比对
 * 的是「这个符号现在还有没有生产消费者」，与编译器怎么把字段/变体拼成一句无关。
 *
 * 用法：`node scripts/session-dead-code-audit.mjs`
 * 退出码：0 = 通过；1 = 违规或审计范围读不出（判不了的判据不许亮绿灯）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns SD 会话死码台账（B 类能力待接入）
// @codes SD-001

import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.resolve(scriptDir, '..')
const LEDGER_REL = 'scripts/exemptions/session-dead-code.json'
const LEDGER = path.join(ROOT, LEDGER_REL)
/** 台账行的必填字段：少一列就等于一条不解释自己的豁免。 */
const REQUIRED = ['path', 'symbols', 'capability', 'future_caller', 'batch', 'e2e']
/** 只在这些目录里查「源码内 allow」：台账管的就是这个插件的 B 类死码。 */
const ALLOW_SCAN_DIR = path.join(ROOT, 'crates', 'symbio-plugin-session', 'src')

/** 把 cargo 的 `file_name` 归一成本仓相对路径（Windows 反斜杠 / 绝对前缀都收掉）。 */
function relPath(fileName) {
  const f = String(fileName).replace(/\\/g, '/')
  if (path.isAbsolute(f) || /^[A-Za-z]:\//.test(f)) {
    return path.relative(ROOT, f).replace(/\\/g, '/')
  }
  return f.replace(/^\.\//, '')
}

/** 从一条 `dead_code` 告警正文里取符号名：所有反引号片段。 */
function symbolNames(message) {
  return [...String(message).matchAll(/`([^`]+)`/g)].map((m) => m[1])
}

export function validateLedger(ledger) {
  const errors = []
  if (!ledger || typeof ledger !== 'object') return ['台账读不出（不是 JSON 对象）']
  if (!Array.isArray(ledger.rows)) errors.push('台账缺 `rows` 数组（空台账写 `[]`）')
  if (!Array.isArray(ledger.landed_batches)) errors.push('台账缺 `landed_batches` 数组')
  const seen = new Map()
  for (const [i, row] of (ledger.rows ?? []).entries()) {
    const at = `rows[${i}]${row?.path ? ` (${row.path})` : ''}`
    for (const k of REQUIRED) {
      if (!(k in (row ?? {}))) {
        errors.push(`${at}：缺 \`${k}\``)
        continue
      }
      if (k === 'symbols' || k === 'e2e') {
        const v = row[k]
        if (!Array.isArray(v) || v.length === 0 || v.some((x) => typeof x !== 'string' || !x.trim())) {
          errors.push(`${at}：\`${k}\` 必须是非空字符串数组`)
        }
      } else if (typeof row[k] !== 'string' || !row[k].trim()) {
        errors.push(`${at}：\`${k}\` 必须是非空字符串`)
      }
    }
    if (Array.isArray(row?.symbols) && typeof row?.path === 'string') {
      for (const name of row.symbols) {
        const key = `${row.path}::${name}`
        if (seen.has(key)) errors.push(`${at}：符号 \`${name}\` 已在 rows[${seen.get(key)}] 登记——同一符号只能有一行`)
        else seen.set(key, i)
      }
    }
  }
  for (const b of ledger.landed_batches ?? []) {
    if (typeof b !== 'string' || !b.trim()) errors.push('`landed_batches` 里只能放非空的批次名')
  }
  return errors
}

/** 源码内的 `#[allow(dead_code)]` / `#![allow(dead_code)]`（`cfg_attr(not(test), …)` 不算）。 */
export function allowInSource(dir = ALLOW_SCAN_DIR) {
  const out = []
  const walk = (d) => {
    let entries
    try {
      entries = fs.readdirSync(d, { withFileTypes: true })
    } catch {
      return
    }
    for (const e of entries) {
      const p = path.join(d, e.name)
      if (e.isDirectory()) {
        walk(p)
        continue
      }
      if (!e.isFile() || !e.name.endsWith('.rs')) continue
      const lines = fs.readFileSync(p, 'utf8').split(/\r?\n/)
      lines.forEach((line, i) => {
        if (/^[ \t]*#!?[ \t]*\[[ \t]*allow[ \t]*\([ \t]*dead_code/.test(line)) {
          out.push({ path: relPath(p), line: i + 1 })
        }
      })
    }
  }
  walk(dir)
  return out
}

/** 跑 `cargo check --workspace --all-targets`，把每条 `dead_code` 的 (`path`, 符号) 抽成集合。 */
export function collectDeadPairs(root = ROOT) {
  const r = spawnSync(
    'cargo',
    ['check', '--workspace', '--all-targets', '--message-format=json'],
    { cwd: root, encoding: 'utf8', shell: false, maxBuffer: 256 * 1024 * 1024 },
  )
  if (r.error) return { error: `cargo 无法启动：${r.error.message}` }
  if (r.status !== 0) return { error: `cargo check 退出 ${r.status}：审计范围读不出` }
  const pairs = new Map()
  for (const line of String(r.stdout).split(/\r?\n/)) {
    if (!line.trim()) continue
    let j
    try {
      j = JSON.parse(line)
    } catch {
      continue
    }
    if (j.reason !== 'compiler-message' || !j.message || j.message.code?.code !== 'dead_code') continue
    const primary = (j.message.spans ?? []).find((s) => s.is_primary)
    if (!primary?.file_name) continue
    const file = relPath(primary.file_name)
    for (const name of symbolNames(j.message.message)) pairs.set(`${file}::${name}`, { path: file, name })
  }
  return { pairs }
}

function main() {
  if (!fs.existsSync(LEDGER)) {
    console.error(red(`✗ session-dead-code-audit：台账不存在（${LEDGER_REL}）`))
    return 1
  }
  let ledger
  try {
    ledger = JSON.parse(fs.readFileSync(LEDGER, 'utf8'))
  } catch (e) {
    console.error(red(`✗ session-dead-code-audit：台账不是合法 JSON（${e.message}）`))
    return 1
  }
  const ledgerErrors = validateLedger(ledger)
  if (ledgerErrors.length) {
    for (const e of ledgerErrors) console.error(red(`✗ 台账：${e}`))
    return 1
  }

  const allows = allowInSource()
  if (allows.length) {
    console.error(red(`✗ SD-001 源码里出现 \`allow(dead_code)\`（${allows.length} 处）——台账是唯一豁免形态`))
    for (const a of allows) console.log(`    ${a.path}:${a.line}`)
    return 1
  }

  const { pairs, error } = collectDeadPairs()
  if (error) {
    console.error(red(`✗ session-dead-code-audit：${error}`))
    return 1
  }

  const rows = ledger.rows
  const landed = new Set(ledger.landed_batches)
  const ledgerPairs = new Map()
  for (const row of rows) for (const name of row.symbols) ledgerPairs.set(`${row.path}::${name}`, row)

  const unregistered = [...pairs.keys()].filter((k) => !ledgerPairs.has(k)).map((k) => pairs.get(k))
  const stale = [...ledgerPairs.keys()].filter((k) => !pairs.has(k)).map((k) => ledgerPairs.get(k))
  const landedRows = rows.filter((r) => landed.has(r.batch))

  if (unregistered.length || stale.length || landedRows.length) {
    console.error(red('\nsession-dead-code-audit 未通过（D-331 / SD-001）'))
    if (unregistered.length) {
      console.log(`\n  源码有 dead_code 但台账没有（${unregistered.length}）——登记能力 / 调用方 / 批次 / e2e，或直接删掉死码：`)
      for (const p of unregistered.slice(0, 30)) console.log(`    ${p.path}::${p.name}`)
    }
    if (stale.length) {
      console.log(`\n  台账有但源码已不再报（${stale.length}）——已有消费者或符号已不存在，要求销账：`)
      for (const p of stale.slice(0, 30)) console.log(`    ${p.path}::${p.name}`)
    }
    if (landedRows.length) {
      console.log(`\n  所属批次已落地（${landedRows.length}）——批次接入即从台账删除：`)
      for (const r of landedRows) console.log(`    [${r.batch}] ${r.path} :: ${r.symbols.join(', ')}`)
    }
    console.log(
      dim('\n  台账 = 待接入能力清单，不是永久豁免。接入完成、删掉死码或批次落地，都必须销账。'),
    )
    return 1
  }

  if (rows.length === 0) {
    console.log(green('✓ session-dead-code-audit：台账为空且源码无 dead_code——B 类能力全部接入完成（D-331）'))
    return 0
  }
  console.log(
    green(
      `✓ session-dead-code-audit：${rows.length} 行台账 / ${pairs.size} 个符号在册，均有生产消费者缺口且未过期` +
        `（landed_batches ${landed.size}）`,
    ),
  )
  return 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(main())
}
