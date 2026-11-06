#!/usr/bin/env node
/**
 * turn-root-audit — turn 词根冻结（TR-001…TR-004）
 *
 * ## 它治的病
 *
 * `turn` 是**废弃概念**：端口独立后 model / tool 各自是独立债务，轮次
 * 不该再是机制状态，展示层也不留投影。但「逐步拆除」需要一条**只许降**的棘轮——
 * 否则每批拆一点、每批又顺手加一点，最终停在半拆的形态里，而没有任何判据说话。
 *
 * 本脚本不判「该不该有 turn」（那是架构裁决的事），只判**存量有没有变宽**。
 *
 * ## 五个面（口径见 T0 冻结门禁的判据面）
 *
 * | 面 | 扫什么 | 范围 |
 * |---|---|---|
 * | `F1` | 标识符：`Turn…` / `turn_…` / `TURN_…` / 裸 `turn` 字段 | `crates/**`（`.rs`） |
 * | `F2` | `turn.<x>` 字面量、`Entity::Turn`、`FinalizesTurn` / `StopsTurn`、`turn_head` 等视图名、`*-turn` 行名 | `crates/**`（`.rs` / `.yml`） |
 * | `F3` | Rust DTO 字段 `turn:` / `turn?` | `crates/symbio/**`（`.rs`） |
 * | `F4` | 前端 / 展示层 `turn` | `tauri/src/**`（`.ts` / `.vue`） |
 * | `F5` | 存量数据迁移通道（`chat_node.ts` 旧词折叠） | 单文件 |
 *
 * 每面分**生产 / 测试**两组计数：生产面严格冻结（新增即红），测试面只许降。
 * 注释正文不扫（注释由 D-016 管），`docs/**` 不扫（文档是「为什么」的 owner）。
 *
 * ## 目标：0
 *
 * 基线只记**存量**，目标另写 `target: 0`——轮次概念退场后，每个面的正确读数就是 0。
 * 计数下降是拆题面的**预期形态**，不是「对齐数字」：本脚本承认退场，不为存量找说法。
 * 到 0 之后不再靠棘轮推动，而由 `TR-003` 守「空扫不退化」（范围读不出仍然红）。
 *
 * ## 四条判红
 *
 * - `TR-001` **新增**：任一面（生产 / 测试）计数高于基线。
 * - `TR-002` **只升不降**：基线高于实测——收窄了却没在**同批**把基线降下来。
 *   棘轮必须跟着收紧走；「降了但基线不降」等于给下一批留出悄悄变宽的空间。
 *   基线声明的 `target` 不是 **0** 也判红：目标是写死的，改大等于给退场设终点。
 * - `TR-003` **空扫**：某个面扫到 0 个文件，或实测 0 而基线不为 0。
 *   这一族的老病是「读不出 ⇒ 没什么可判 ⇒ 通过」。
 * - `TR-004` **陈旧 allow**：`allow` / `pending` 条目所指的文件或写法不存在；
 *   或 `allow` 出现在 `F5` 之外（迁移通道是唯一的显式例外）。
 *
 * ## 用法
 *
 *   node scripts/turn-root-audit.mjs [--root=<仓库根>] [--print-baseline] [--write-baseline]
 *
 * 退出码：0 = 通过；1 = 有红格或读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns TR turn 词根冻结（废弃概念的存量棘轮）
// @codes TR-001 TR-002 TR-003 TR-004

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')
const BASELINE_REL = 'scripts/turn-root-baseline.json'
// 目标写死 0：轮次不是「要不要保留」的产品问题，是已经退场的概念。
// 因此不存在「允许残留多少」的谈判空间——基线只记还有多少没拆完。
const TARGET = 0

const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'dist', '.next', 'verify-target'])

const FACE_DEFS = [
  {
    id: 'F1',
    label: '标识符（类型 / 函数 / 常量 / 字段）',
    roots: ['crates'],
    exts: ['.rs'],
    // `Turn…` 覆盖类型与枚举格；`turn_` / `TURN_` 覆盖函数与常量；裸 `turn` 覆盖字段与
    // 局部名（排除紧邻 `.` 的写法——那是 F2 的事实字面量，避免同一处被数两遍）。
    pattern: /\bTurn[A-Za-z0-9_]*\b|\bturn_[a-z0-9_]+\b|\bTURN_[A-Z0-9_]+\b|\bturn\b(?!\s*\.)/g,
  },
  {
    id: 'F2',
    label: '事实 / 视图 / 行名',
    roots: ['crates'],
    exts: ['.rs', '.yml'],
    pattern:
      /\bturn\.(?:stage|dispatch|dispatched|supplemented|branch|branch_selected|recalled|closed|head|stages|slice|embryo)\b|\bEntity::Turn\b|\b(?:FinalizesTurn|StopsTurn)\b|\b(?:turn_head|turn_closed|turn_slice|turn_stages|turn_recalled|final_unique_per_turn)\b|\b(?:close|prepare|continue)-turn\b/g,
  },
  {
    id: 'F3',
    label: 'DTO 字段（Rust 命令层）',
    roots: ['crates/symbio'],
    exts: ['.rs'],
    pattern: /\bturn\s*[:?]/g,
  },
  {
    id: 'F4',
    label: '前端 / 展示层',
    roots: ['tauri/src'],
    exts: ['.ts', '.vue'],
    pattern: /\bturn\b/g,
  },
  {
    id: 'F5',
    label: '存量数据迁移通道',
    files: ['tauri/src/schemas/chat_node.ts'],
    exts: ['.ts'],
    pattern: /turn:\s*NODE_KIND_SKELETON/g,
  },
]

/** 测试面：判据随被测物走，测试文件只许降（拆 turn 时用例会跟着删）。 */
function isTestFile(rel) {
  return (
    /\.(?:test|spec)\.(?:rs|ts|tsx|mts|mjs)$/.test(rel) ||
    rel.includes('/__tests__/') ||
    rel.includes('/tests/') ||
    /test_fixtures\.rs$/.test(rel) ||
    /\/stubs\.rs$/.test(rel)
  )
}

function walk(absDir, relDir, out) {
  let entries
  try {
    entries = fs.readdirSync(absDir, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of entries) {
    if (e.name.startsWith('.')) continue
    if (e.isDirectory()) {
      if (SKIP_DIRS.has(e.name)) continue
      walk(path.join(absDir, e.name), `${relDir}/${e.name}`, out)
    } else if (e.isFile()) {
      out.push(`${relDir}/${e.name}`)
    }
  }
}

/** 去掉注释正文：注释里叙述历史是允许的（D-016），它不是机制面。 */
function stripComments(text, ext) {
  const norm = text.replace(/\r\n/g, '\n')
  if (ext === '.rs' || ext === '.ts' || ext === '.vue' || ext === '.yml') {
    const block = ext === '.yml' ? norm : norm.replace(/\/\*[\s\S]*?\*\//g, '')
    return block
      .split('\n')
      .map((line) => {
        const t = line.trimStart()
        // 行尾的 allow 标记不是词根（`… , // turn-migration-allow: …`），否则每加
        // 一个标记都会把计数抬高，冻结就变成「标记越多越红」。只去掉标记本身，
        // **代码部分照常计数**——否则迁移通道那一行会被标记自己抹掉（F5 变 0）。
        line = line.replace(/\/\/\s*turn-migration-allow.*$/, '')
        // 行尾注释同样不是词根：生成器给共享事件行留的 `// turn.branch ← loop` 这类
        // 投影注解若计入，冻结的就不是代码而是注释正文（与「注释正文不扫」自相矛盾）。
        if (line.trim().startsWith('//')) line = ''
        else line = line.replace(/\s+\/\/(?!\/).*$/, '')
        if (line.trim() === '') return ''
        const isComment =
          t.startsWith('//') ||
          (ext === '.yml' && t.startsWith('#')) ||
          ((ext === '.ts' || ext === '.vue') && (t.startsWith('*') || t.startsWith('/*')))
        return isComment ? '' : line
      })
      .join('\n')
  }
  return norm
}

function readText(rel) {
  const abs = path.join(ROOT, rel)
  if (!fs.existsSync(abs)) return null
  return stripComments(fs.readFileSync(abs, 'utf8'), path.extname(rel))
}

/** 面内所有文件（F5 是显式单文件）。 */
function filesOf(face) {
  if (face.files) return face.files
  const out = []
  for (const root of face.roots) walk(path.join(ROOT, root), root, out)
  return out.filter((rel) => face.exts.includes(path.extname(rel)))
}

function measure(face) {
  const files = filesOf(face)
  const acc = { prod: 0, test: 0, files: files.length, scanned: 0, hits: [] }
  for (const rel of files) {
    const text = readText(rel)
    if (text === null) continue
    acc.scanned += 1
    const n = (text.match(face.pattern) || []).length
    if (n === 0) continue
    if (isTestFile(rel)) acc.test += n
    else acc.prod += n
    acc.hits.push({ rel, n })
  }
  return acc
}

function loadBaseline() {
  const abs = path.join(ROOT, BASELINE_REL)
  if (!fs.existsSync(abs)) return null
  try {
    return JSON.parse(fs.readFileSync(abs, 'utf8'))
  } catch {
    return null
  }
}

/** allow / pending 条目必须指得到（TR-004）。 */
function checkEntries(baseline, measured) {
  const problems = []
  for (const entry of baseline.allow || []) {
    if (entry.face !== 'F5') {
      problems.push(`allow 只允许落在 F5（迁移通道），这条落在 ${entry.face || '未写面'}：${entry.file}`)
      continue
    }
    const text = readText(entry.file)
    if (text === null) {
      problems.push(`allow 指向的文件不存在：${entry.file}`)
      continue
    }
    if (!new RegExp(entry.pattern).test(text)) {
      problems.push(`allow 的写法已不存在：${entry.file} / ${entry.pattern}`)
    }
  }
  for (const entry of baseline.pending || []) {
    const text = readText(entry.file)
    if (text === null) {
      problems.push(`pending 指向的文件不存在：${entry.file}`)
      continue
    }
    if (!new RegExp(entry.pattern).test(text)) {
      problems.push(`pending 的写法已不存在（可转正或删条目）：${entry.file} / ${entry.pattern}`)
    }
  }
  const f5Allow = (baseline.allow || []).filter((e) => e.face === 'F5')
  if (f5Allow.length > 1) problems.push(`F5 迁移通道只允许一条 allow，实际 ${f5Allow.length} 条`)
  // 迁移通道在、却没登记 ⇒ 例外成了隐式豁免（展示层没有豁免行）。
  if (measured.F5.prod > 0 && f5Allow.length === 0) {
    problems.push('F5 有存量折叠项，但 allow 清单里没有登记（迁移通道必须是显式例外）')
  }
  return problems
}

function main() {
  const printBaseline = process.argv.includes('--print-baseline')
  const writeBaseline = process.argv.includes('--write-baseline')

  const measured = {}
  for (const face of FACE_DEFS) measured[face.id] = measure(face)

  if (printBaseline || writeBaseline) {
    for (const face of FACE_DEFS) {
      const m = measured[face.id]
      console.log(`${face.id}: prod=${m.prod} test=${m.test} files=${m.scanned}`)
      for (const h of m.hits.sort((a, b) => b.n - a.n).slice(0, 8)) console.log(`  ${h.n}  ${h.rel}`)
    }
    if (writeBaseline) {
      const baseline = loadBaseline() || {}
      baseline.faces = {}
      for (const face of FACE_DEFS) {
        baseline.faces[face.id] = { prod: measured[face.id].prod, test: measured[face.id].test }
      }
      baseline.allow = baseline.allow || []
      baseline.pending = baseline.pending || []
      baseline.target = TARGET
      const abs = path.join(ROOT, BASELINE_REL)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, `${JSON.stringify(baseline, null, 2)}\n`)
      console.log(`已写入 ${BASELINE_REL}`)
    }
    return
  }

  const baseline = loadBaseline()
  if (baseline === null || !baseline.faces) {
    console.error(red(`turn-root-audit 未通过：读不出基线 ${BASELINE_REL}（缺文件或不是合法 JSON）`))
    console.error(red('  空扫 / 读不出不许当通过——先修基线再跑门禁。'))
    process.exit(1)
  }

  console.log('--- turn-root-audit: turn 词根冻结（TR-001…004） ---')

  const problems = []
  if (baseline.target !== TARGET) {
    problems.push({
      code: 'TR-002',
      msg: `基线 target 必须是 ${TARGET}（轮次已退场，每个面的正确读数就是 0），实际 ${JSON.stringify(baseline.target)}`,
    })
  }
  for (const face of FACE_DEFS) {
    const m = measured[face.id]
    const base = baseline.faces[face.id] || { prod: 0, test: 0 }

    if (m.scanned === 0) {
      problems.push({ code: 'TR-003', msg: `${face.id} 空扫：0 个文件（范围读不出 ⇒ 判据空转）` })
      continue
    }
    if (m.prod === 0 && base.prod !== 0) {
      problems.push({ code: 'TR-003', msg: `${face.id} 实测生产面 0，而基线 ${base.prod}（读法失配？）` })
    }
    for (const face2 of ['prod', 'test']) {
      if (m[face2] > base[face2]) {
        problems.push({
          code: 'TR-001',
          msg: `${face.id} ${face2 === 'prod' ? '生产' : '测试'}面新增：${base[face2]} → ${m[face2]}（+${m[face2] - base[face2]}）`,
        })
      } else if (m[face2] < base[face2]) {
        problems.push({
          code: 'TR-002',
          msg: `${face.id} ${face2 === 'prod' ? '生产' : '测试'}面收窄 ${base[face2]} → ${m[face2]}：基线未同批下调（棘轮只许降，降要跟着改基线）`,
        })
      }
    }
    const ok = m.prod === base.prod && m.test === base.test
    console.log(
      ok
        ? green(`  ✓ ${face.id} ${face.label}：生产 ${m.prod} / 测试 ${m.test}（${m.scanned} 文件）`)
        : red(`  ✗ ${face.id} ${face.label}：生产 ${m.prod} / 测试 ${m.test}（基线 ${base.prod} / ${base.test}）`),
    )
  }

  const total = FACE_DEFS.reduce(
    (s, f) => s + measured[f.id].prod + measured[f.id].test,
    0,
  )
  console.log(`  目标 ${TARGET}：实测仍余 ${total} 处待退场`)

  for (const p of checkEntries(baseline, measured)) problems.push({ code: 'TR-004', msg: p })

  if (problems.length > 0) {
    for (const p of problems) console.log(red(`  ✗ ${p.code} ${p.msg}`))
    const codes = [...new Set(problems.map((p) => p.code))].join(' / ')
    console.log(red(`turn-root-audit 未通过：${problems.length} 条（${codes}）。turn 是废弃概念，拆除只能往下走。`))
    process.exit(1)
  }

  const pending = (baseline.pending || []).length
  console.log(
    green(
      `turn-root-audit 通过：5 面与基线一致（turn 词根 ${total} 处；待裁决 ${pending} 条挂 pending）`,
    ),
  )
  if (pending > 0) {
    console.log(yellow(`  ↳ pending 不是豁免：Entity::Turn 裁决后随批删除`))
  }
}

main()
