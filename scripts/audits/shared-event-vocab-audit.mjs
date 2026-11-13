#!/usr/bin/env node
/**
 * arch-shape-audit 的兄弟判据 —— 跨插件**事件名**的登记口是清单，不是内核表
 *
 * ## 它判什么
 *
 * `shared_events` 这条正道（DE-024 D-204）开通之后，「两个插件同时消费一条名字 ⇒ 只能住
 * 内核」（E-009 的旧推论）不再有说服力：名字有 owner，格位有声明，共享面是清单的投影。
 * 于是CS-005 只数条数的那两格多了一个同伴——本判据逐条核**清单上的每一行**：
 *
 * | 编号 | 判什么 | 它拦的是哪一种腐烂 |
 * |---|---|---|
 * | CS-007 | 清单上每条名字：① 除 owner 外**至少一个**插件在生产代码里引用它；② 内核 `event/mod.rs` / `CELLS` 上不许再有它 | 清单变成第二张内核表：登记一条没人用的名字、或者登记完忘了把内核那一行删掉，两处都留着就是两个 owner |
 * | CS-011 | `FactRole` 的四档**每档都有生产居住者**（插件生产源码里的 `FactRole::<档>`） | 判据读角色、角色来自 owner 的 `EventDecl.roles`；owner 删掉那一行不会报错，只让那道守卫从「拒绝」变成「看不见」（DE-024 D-203） |
 *
 * 名字**形状**（`<域>.<事实>`）、owner 有没有为它提交带格位的 `EventDecl`、以及「一条名字
 * 两个 owner」由生成器 `parseEventList` / `collectSharedEvents` 抛错拦住（读不懂的清单连
 * 生成物都产不出，本判据以退出码 2 转述它），本判据不再判一遍。
 *
 * 口径与生成器（`scripts/facts/events-rs.mjs`）**同一个读法**：本判据 import 它的
 * `collectSharedEvents` / `pluginDeclarationFaces`，不另写一遍正则。两处各扫一遍的话，
 * 分叉只会表现为「生成物有这条、判据说没有」。
 *
 * 另有一类**只报数不判红**：内核表上仍是 ≥2 插件消费的名字。它们是该走的存量，不是
 * 该拦的违规——拦了就是把「还没迁完」写成「不许迁」，与 D-203 的裁决三正相反。
 *
 * 用法：node scripts/audits/shared-event-vocab-audit.mjs [--root=<目录>]
 * 退出码：1 = 有违规；2 = 范围读不出（缺内核表 / 缺插件清单）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns CS core 形状
// @codes CS-007 CS-011
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { pluginDeclarationFaces, collectSharedEvents } from '../facts/events-rs.mjs'
import { red, green } from '../color.mjs'

const argv = process.argv.slice(2)
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')

const CONST_OF = (name) => `EVENT_${name.replace(/\./g, '_').toUpperCase()}`

const errors = []
const notes = []

/**
 * 内核表上的业务名字（`event/mod.rs` 的常量值 + 若还在的 `CELLS` 行名）。
 *
 * `event/mod.rs` 必须在（协议五格住那儿，缺了就是范围读不出）；`grid.rs` 已随
 * DE-024 D-203 归零批退役，**缺席是目标态**：它不在 ⇒ 没有 `CELLS` 行可数；它若
 * 回来 ⇒ 照数，下面那条「还挂在 `CELLS` 上」的判据会逐名点红。判据不靠一张已删
 * 文件的形状活着。
 */
function kernelNames(root) {
  const dir = path.join(root, 'crates', 'symbio-kernel', 'src', 'event')
  const read = (name) => {
    const abs = path.join(dir, name)
    if (!fs.existsSync(abs)) throw new Error(`范围读不出：缺少 ${path.relative(root, abs)}`)
    return fs
      .readFileSync(abs, 'utf8')
      .split('\n')
      .filter((l) => !l.trim().startsWith('//'))
      .join('\n')
  }
  const consts = new Map()
  const values = new Set()
  for (const m of read('mod.rs').matchAll(/pub const (EVENT_[A-Z0-9_]+)\s*:[^=]*=\s*"([^"]+)"/g)) {
    consts.set(m[1], m[2])
    if (m[2].includes('.')) values.add(m[2])
  }
  if (fs.existsSync(path.join(dir, 'grid.rs'))) {
    for (const m of read('grid.rs').matchAll(/EventDecl::(?:with_roles|new)\s*\(\s*(?:"([^"]+)"|([A-Z][A-Z0-9_]*))/g)) {
      const value = m[1] ?? consts.get(m[2])
      if (value?.includes('.')) values.add(value)
    }
  }
  return values
}

/** 引用形态有两种：字符串字面量 `"a.b"` 与常量标识符 `EVENT_A_B`。 */
function refRe(name) {
  const esc = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  return new RegExp(`"${esc}"|\\b${CONST_OF(name)}\\b`)
}

/**
 * CS-011 的扫描面：插件 crate 的**生产**源码（`src/**.rs`，剔掉 `*.test.rs`）。
 *
 * 为什么不扫内核与 `plugin-api`：内核只定义 `FactRole`，门面只转出协议五格的宿主声明，
 * 两者都不回答「谁在守着这一档判据」；居住者必须是被判据读的那张表上的 owner。
 */
function productionPluginSources(root) {
  const dir = path.join(root, 'crates')
  if (!fs.existsSync(dir)) throw new Error('范围读不出：缺少 crates/')
  const out = []
  for (const name of fs.readdirSync(dir)) {
    if (!name.startsWith('symbio-plugin-')) continue
    const src = path.join(dir, name, 'src')
    if (!fs.existsSync(src)) continue
    const walk = (d) => {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const abs = path.join(d, e.name)
        if (e.isDirectory()) walk(abs)
        else if (e.name.endsWith('.rs') && !e.name.endsWith('.test.rs')) out.push(abs)
      }
    }
    walk(src)
  }
  if (out.length === 0) throw new Error('范围读不出：没有任何插件生产源码')
  return out
}

/** 四档角色各被哪些生产文件持有（`DE-024` D-203 的居住账）。 */
function roleResidents(root) {
  const roles = ['FinalizesScope', 'StopsScope', 'DeclaresBudget', 'CountsRework']
  const residents = new Map(roles.map((r) => [r, []]))
  for (const abs of productionPluginSources(root)) {
    const text = fs.readFileSync(abs, 'utf8')
    for (const m of text.matchAll(/FactRole::([A-Za-z]+)/g)) {
      const bucket = residents.get(m[1])
      if (bucket) bucket.push(path.relative(root, abs))
    }
  }
  return residents
}

let faces
let kernel
let rows
try {
  faces = pluginDeclarationFaces(ROOT)
  kernel = kernelNames(ROOT)
  rows = collectSharedEvents(ROOT, faces)
} catch (e) {
  console.log(`${red('[ERROR]')} shared-event-vocab-audit：${e.message}`)
  process.exit(2)
}

for (const row of rows) {
  const where = `crates/symbio-plugin-${row.owner}/PLUGIN.yml`
  const fail = (msg) => errors.push(`${where}：${msg}`)
  const consumers = [...faces.entries()]
    .filter(([other, f]) => other !== row.owner && f.texts.some((t) => refRe(row.name).test(t)))
    .map(([other]) => other)
    .sort()
  if (consumers.length === 0) {
    fail(
      `${row.name} 除 owner 之外没有任何插件在生产代码里引用它——单消费方的名字随 owner 走，` +
        '不进共享表（预置常量正是上一轮清理掉的腐烂）',
    )
  }
  if (kernel.has(row.name)) {
    fail(
      `${row.name} 还挂在内核 \`event/mod.rs\` / \`CELLS\` 上——跨插件名字的唯一登记口是清单，` +
        '内核表再留一行就是让 CS-005 的数字替它遮掩（DE-024 D-204）',
    )
  }
  row.consumers = consumers
}

for (const name of kernel) {
  const consumers = [...faces.entries()]
    .filter(([, f]) => f.texts.some((t) => refRe(name).test(t)))
    .map(([id]) => id)
    .sort()
  if (consumers.length >= 2) {
    notes.push(
      `内核表上的「${name}」被 ${consumers.join(' / ')} 同时消费，却还没走 \`shared_events\` 这条正道` +
        '（随各自读方迁出时同批退役，DE-024 D-203 / D-204）',
    )
  }
}

// ---- CS-011：四档守卫角色每档都要有生产居住者（DE-024 D-203）----
// 判据空转的经典形态：角色表少一行不报错，只让那道守卫从「拒绝」变成「看不见」。
// 装配期判不了（只链接部分插件的用例会假红，见 `assemble_declarations` 的说明），
// 所以判在整仓这一面：生产声明里缺一档就红。
let roleGaps = []
let roleLine = ''
try {
  const residents = roleResidents(ROOT)
  for (const [role, where] of residents) {
    if (where.length === 0) {
      roleGaps.push(
        `${role} 没有任何生产声明持有它——判据读角色，角色缺席时那道守卫不报错、只是不再判`,
      )
    }
    roleLine += `${roleLine ? ' · ' : ''}${role}←${where.length}`
  }
} catch (e) {
  console.log(`${red('[ERROR]')} shared-event-vocab-audit：${e.message}`)
  process.exit(2)
}

if (errors.length || roleGaps.length) {
  for (const e of errors) console.log(red(`✗ CS-007 ${e}`))
  for (const e of roleGaps) console.log(red(`✗ CS-011 ${e}`))
  console.log(`\n事件名共享表：${errors.length} 条违规 · 角色居住：${roleGaps.length} 档无人`)
  process.exit(1)
}
const summary = rows
  .map((r) => `${r.name}←${r.owner}→${(r.consumers ?? []).join('/')}`)
  .join('、')
console.log(
  green(
    `✓ CS-007 事件名共享表：${rows.length} 条由清单登记（${summary || '暂无'}），` +
      '每条都有 owner 的 `EventDecl` 与外部消费方，内核表上都不再有它',
  ),
)
console.log(green(`✓ CS-011 守卫角色：四档都有生产居住者（${roleLine}）`))
for (const n of notes) console.log(`  · ${n}`)
