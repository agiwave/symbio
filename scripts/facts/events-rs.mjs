/**
 * 事实 · `crates/symbio-plugin-api/src/events.rs` —— 跨插件事件名常量的**共享侧**投影
 *
 * ## 它替掉了什么
 *
 * 「一条事件名被两个插件同时消费」在过去只有两个去处：内核的 `event/mod.rs` 常量 +
 * `CELLS` 行，或者让两个插件各写一遍字面量。前者把内核当成语义字典（加能力 = 改架构），
 * 后者是同一个事实的两个 owner（改一次要动两处）。现在它是各插件 `PLUGIN.yml` 里
 * `shared_events:` 行的投影——**登记随 owner 的清单走，共享文件零 diff**（DE-024 D-204）。
 *
 * ## 事实源是清单，格位仍是声明
 *
 * `shared_events` 只回答「这个 owner 导出哪几条名字给别的插件命名用」。一条名字**落在
 * 哪个格子、扮演什么角色**仍由 owner 的 `EventDecl`（Rust 声明）回答，写闸读的是装配表；
 * 生成器逐条核对 owner 确实提交了这条 `EventDecl`，声明了一条没有格位的名字直接抛错——
 * 那比不生成更坏，因为它会让调用方以为名字已经登记好了。
 *
 * ## 为什么不整表生成
 *
 * 只有**跨插件**消费的名字才需要共享。单消费方的名字随它自己的 owner 声明走（DE-024
 * D-197 / D-201），core 的判据读格子与角色、不读名字（D-203）。给没人共享的名字预置
 * 一条共享常量，就是给下一轮留腐烂——与 `routes-rs` 同一条纪律，由 CS-007 判红。
 *
 * 由 `scripts/gen-facts.mjs` 遍历 `scripts/facts/` 时调用，不要直接跑。
 */

import fs from 'node:fs'
import path from 'node:path'

import { PLUGIN_CRATE_PREFIX, pluginIds, readPluginManifest } from './_plugin-yml.mjs'

const OUT_REL = 'crates/symbio-plugin-api/src/events.rs'

const HEADER = `//! 跨插件事件名常量 —— **由 \`scripts/facts/events-rs.mjs\` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 \`PLUGIN.yml\` 的 \`shared_events:\` 行（**owner** 导出给
//! 别的插件命名的那几条事件名）与 owner 自己的 \`EventDecl\` 表（格位）。加一条跨插件
//! 事件 = 在 owner 自己的清单里登记一行，并在 owner 的 \`EventDecl\` 里给它格位；内核与
//! core 的文件零 diff。手改本文件会被 \`gen-facts --check\` 判红。
//!
//! # 口径（展开见 \`docs/003-design/024-能力横向扩充的通路.md\`）
//!
//! - **本表只收跨插件消费的名字**。单消费方的名字由那个插件自己声明，不进这里。
//! - **格位取自 owner 的声明**：写闸问的是「这个名字落在哪一格」（DE-024 D-197），所以这里
//!   连 \`EventDecl\` 一起投影一份 \`SHARED_EVENTS\`——链接了本 crate 的每个进程都认识它，
//!   不必把 owner 插件也链进同一个二进制（那正是 E-009 禁掉的那条边）。
//! - **内核不认识业务事件名**（DE-024 D-203 / D-204）：内核的两张表只留封闭语法——Entity×Verb
//!   网格与协议五格。名字要跨插件就登记进清单，不要回内核加行（CS-005 / CS-007）。
//! - **角色不在这里**：\`FactRole\` 是 owner 交给守卫的陈述，仍住 \`EventDecl\` 由装配表读；
//!   本表只投影「名字 + 格位」两维。
//!
//! # 命名
//!
//! 常量名去掉 \`EVENT_\` 前缀后必须**逐字等于**值的大写形式（\`.\` → \`_\`）：
//! \`memory.encoded\` → \`EVENT_MEMORY_ENCODED\`。由 CS-007 检查。`

const NAME_RE = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$/
const DECL_RE =
  /EventDecl::(?:with_roles|new)\s*\(\s*(?:"([a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)+)"|([A-Z][A-Z0-9_]*))/g
/**
 * 声明处的格位：`(Entity::Memory, Verb::Opened)`（跨行也认）。
 * 路径限定写法（`(symbio_core::Entity::Memory, …)`——不 `use` 直接全量写）同认：
 * 拼写上两种都对，只认一种会让生成器对同一形状的声明给出两种答案。
 */
const CELL_RE = /\(\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*Entity::([A-Za-z0-9_]+)\s*,\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*Verb::([A-Z][A-Za-z0-9_]*)\s*\)/

/** `shared_events: a.b, c.d` → `["a.b", "c.d"]`；缺键 ⇒ `[]`。 */
function parseEventList(raw, where) {
  if (!raw) return []
  const names = raw
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
  const seen = new Set()
  for (const name of names) {
    if (!NAME_RE.test(name)) {
      throw new Error(
        `${where}：shared_events 里的 ${JSON.stringify(name)} 不是合法事件名` +
          '（`<域>.<事实>`，小写字母 / 数字 / `_`，至少一段点分）',
      )
    }
    if (seen.has(name)) throw new Error(`${where}：shared_events 里的 ${JSON.stringify(name)} 出现了两次`)
    seen.add(name)
  }
  return names
}

function productionTexts(srcDir) {
  const out = []
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      if (e.name.startsWith('.') || e.name === 'target') continue
      const p = path.join(dir, e.name)
      if (e.isDirectory()) walk(p)
      else if (p.endsWith('.rs') && !/test/i.test(e.name)) out.push(fs.readFileSync(p, 'utf8'))
    }
  }
  walk(srcDir)
  return out
}

/**
 * owner 的声明面：`EventDecl` 名字 → 格位。
 *
 * 标识符形态（`EventDecl::new(EVENT_TASK_READY, …)`）先按本 crate 的 `const` 定义解析回
 * 值，解析不出时按拼写约定反推（`EVENT_TASK_READY` → `task.ready`）——与 CS-007 同一个
 * 口径，两处推不出同一个值就会一边说「没登记」一边说「登记了」，那正是清单机制最怕的分叉。
 *
 * 认不出格位（多行写法越过 400 字符窗口、或宏生成）时返回 `cell: null`：调用方直接抛错，
 * 而不是生成一条「有名字没格子」的行——那会让写闸在运行期把它当网格之外拒掉。
 */
function ownerDeclarations(texts) {
  const localConsts = new Map()
  for (const t of texts) {
    for (const m of t.matchAll(/const\s+([A-Z][A-Z0-9_]*)\s*:\s*&[^=]*=\s*"([^"]+)"/g)) {
      localConsts.set(m[1], m[2])
    }
  }
  const cells = new Map()
  for (const t of texts) {
    for (const m of t.matchAll(DECL_RE)) {
      const value = m[1] ?? localConsts.get(m[2]) ?? (m[2] ? m[2].replace(/^EVENT_/, '').toLowerCase().replace(/_/g, '.') : undefined)
      if (!value || cells.has(value)) continue
      const cell = t.slice(m.index, m.index + 400).match(CELL_RE)
      cells.set(value, cell ? { entity: cell[1], verb: cell[2] } : null)
    }
  }
  return cells
}

/**
 * 每个插件的**生产源码面**：源码文本 + 它提交过的 `EventDecl`（名字 → 格位）。
 *
 * 判定方（CS-007）与生成方共用这一个读法：两边各写一遍正则，分叉不会报错，只会
 * 一边说「没登记」一边说「登记了」——那正是清单机制最怕的形态。
 *
 * @returns {Map<string, { texts: string[], cells: Map<string, {entity: string, verb: string} | null> }>}
 */
export function pluginDeclarationFaces(root) {
  const out = new Map()
  for (const id of pluginIds(root)) {
    const texts = productionTexts(path.join(root, 'crates', `${PLUGIN_CRATE_PREFIX}${id}`, 'src'))
    out.set(id, { texts, cells: ownerDeclarations(texts) })
  }
  return out
}

/** 收集所有声明（名字 + owner + 格位），逐条核对 owner 确实提交了带格位的 `EventDecl`。 */
export function collectSharedEvents(root, faces = pluginDeclarationFaces(root)) {
  const byName = new Map()
  for (const dir of pluginIds(root)) {
    const where = `crates/symbio-plugin-${dir}/PLUGIN.yml`
    const manifest = readPluginManifest(root, dir)
    const declared = faces.get(dir).cells
    for (const name of parseEventList(manifest.shared_events, where)) {
      if (!declared.has(name)) {
        throw new Error(
          `${where}：shared_events 声明了 ${JSON.stringify(name)}，但 ${dir} 的生产代码里没有为它` +
            '提交 `EventDecl`——没有格位的名字落不了格，生成一条幽灵常量比不生成更坏',
        )
      }
      const cell = declared.get(name)
      if (!cell) {
        throw new Error(
          `${where}：${dir} 为 ${JSON.stringify(name)} 提交了 \`EventDecl\`，但生成器读不到它的` +
            '格位（`(Entity::…, Verb::…)`）——格位是写闸的答题处，猜一条不如抛错',
        )
      }
      const prior = byName.get(name)
      if (prior) {
        throw new Error(`${where}：${name} 已由 ${prior.owner} 声明——一条事件名只能有一个 owner`)
      }
      byName.set(name, { name, owner: dir, ...cell })
    }
  }
  return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name))
}

const constOf = (name) => `EVENT_${name.replace(/\./g, '_').toUpperCase()}`

/** 纯函数：声明清单 → 文件内容。导出给回归测试（测试不触发写盘）。 */
export function render(rows) {
  const L = [HEADER, '', 'use symbio_core::{Entity, EventDecl, Verb};', '']
  let current = null
  for (const r of rows) {
    const domain = r.name.split('.')[0]
    if (domain !== current) {
      current = domain
      L.push(`// ============ ${domain} ============`)
      L.push('')
    }
    L.push(
      `/// ${r.name} — owner \`symbio-plugin-${r.owner}\`，格位 \`(${r.entity}, ${r.verb})\``,
    )
    L.push(`pub const ${constOf(r.name)}: &str = "${r.name}";`)
    L.push('')
  }
  if (rows.length === 0) {
    L.push('// 当前没有跨插件共享的事件名。owner 在自家 `PLUGIN.yml` 的 `shared_events:` 里登记一行。')
    return `${L.join('\n').replace(/\n+$/, '')}\n`
  }
  L.push('/// 跨插件共享的事件行（名字 + 格位）——写闸的**共享臂**。')
  L.push('///')
  L.push('/// 每一行的格位来自 owner 自己的 `EventDecl`；角色不在这里（那是 owner 交给守卫的')
  L.push('/// 陈述，经装配表读）。链接了本 crate 的每个进程都认识这些名字，不必把 owner 插件')
  L.push('/// 链进同一个二进制——那正是 E-009 禁掉的边（DE-024 D-204）。')
  L.push('#[rustfmt::skip]')
  L.push('pub const SHARED_EVENTS: &[EventDecl] = &[')
  for (const r of rows) {
    L.push(
      `    EventDecl::new(${constOf(r.name)}, (Entity::${r.entity}, Verb::${r.verb})), // ${r.name} ← ${r.owner}`,
    )
  }
  L.push('];')
  return `${L.join('\n').replace(/\n+$/, '')}\n`
}

export default {
  id: 'events-rs',
  title: '跨插件事件名常量（各插件 `PLUGIN.yml` 的 `shared_events` 投影）',
  out: OUT_REL,
  stamp: false,
  render: ({ root }) => render(collectSharedEvents(root)),
}
