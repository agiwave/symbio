/**
 * 事实 · `crates/symbio-plugin-api/src/view_names.rs` —— 跨插件**可读视图名**的共享侧投影
 *
 * ## 它补的是哪一格
 *
 * 视图（`ViewDecl`）一直是 owner 插件自己声明的纯折叠，但**没有跨插件读口**：别的插件
 * 想读一条所有者声明的视图，只能：① 深引 owner 的 crate（跨插件依赖，禁止）；② 在
 * core 里再写一份同样的折叠（同一事实两个 owner，必然漂移）。两者都让「加能力」变成
 * 「改架构」。
 *
 * 本表是 owner 清单里 `shared_views:` 行的投影：它只回答「哪几条视图**公开给别的插件
 * 按名字读**」。读口（`symbio_plugin_api::views`）按名字查本表，再经冻结声明表求值——
 * 名字、版本、成本与算法仍只有 owner 一个来源。
 *
 * ## 为什么不是把所有视图都放出来
 *
 * 默认私有。一条视图对外开放 = 一条要长期兼容的口径（`ver` 一变，读方拿到的是另一
 * 个数）。没人读的视图挂在共享表上只会让「谁能改它」变模糊，与 `routes-rs` /
 * `shared_events` 同一条纪律（CS-007 判红的是「声明了却没被读」，见 DE-025 D-221）。
 *
 * 由 `scripts/gen-facts.mjs` 遍历 `scripts/facts/` 时调用，不要直接跑。
 */

import fs from 'node:fs'
import path from 'node:path'

import { PLUGIN_CRATE_PREFIX, pluginIds, readPluginManifest } from './_plugin-yml.mjs'

const OUT_REL = 'crates/symbio-plugin-api/src/view_names.rs'

const HEADER = `//! 跨插件可读视图名 —— **由 \`scripts/facts/views-rs.mjs\` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 \`PLUGIN.yml\` 的 \`shared_views:\` 行。名字、版本、
//! 扫描成本与求值函数仍住在 owner 自己的 \`ViewDecl\` 里；本表只回答「哪几条视图公开
//! 给别的插件按名字读」。读口见 \`symbio_plugin_api::views\`（DE-025 D-221）。
//!
//! 手改本文件会被 \`gen-facts --check\` 判红。`

const NAME_RE = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$/
const CONST_RE = /const\s+([A-Z][A-Z0-9_]*)\s*:\s*&[^=]*=\s*"([^"]+)"/g
const VIEW_DECL_RE = /\bViewDecl\s*\{/g
const NAME_FIELD_RE = /name:\s*(?:"([a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)+)"|([A-Z][A-Z0-9_]*))/

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

/** owner 的视图声明面：本 crate 生产代码里出现过的 `ViewDecl { name: … }`。 */
function ownerViewNames(texts) {
  const consts = new Map()
  for (const t of texts) {
    for (const m of t.matchAll(CONST_RE)) consts.set(m[1], m[2])
  }
  const names = new Set()
  for (const t of texts) {
    for (const m of t.matchAll(VIEW_DECL_RE)) {
      const window = t.slice(m.index, m.index + 400)
      const field = window.match(NAME_FIELD_RE)
      if (!field) continue
      const name = field[1] ?? consts.get(field[2])
      if (name) names.add(name)
    }
  }
  return names
}

/** `shared_views: a.b, c.d` → `["a.b", "c.d"]`；缺键 ⇒ `[]`。 */
function parseViewList(raw, where) {
  if (!raw) return []
  const names = raw
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
  const seen = new Set()
  for (const name of names) {
    if (!NAME_RE.test(name)) {
      throw new Error(
        `${where}：shared_views 里的 ${JSON.stringify(name)} 不是合法视图名` +
          '（`<域>.<视图>`，小写字母 / 数字 / `_`，至少一段点分）',
      )
    }
    if (seen.has(name)) throw new Error(`${where}：shared_views 里的 ${JSON.stringify(name)} 出现了两次`)
    seen.add(name)
  }
  return names
}

/**
 * 收集跨插件视图：名字 + owner。逐条核对 owner 确实提交了 `ViewDecl`——
 * 清单登记了一个不存在的视图，生成一条幽灵名字比不生成更坏：读方会以为
 * 「这个名字已经有人负责」，而冻结表在运行期才说没有。
 */
export function collectSharedViews(root) {
  const byName = new Map()
  for (const id of pluginIds(root)) {
    const where = `crates/symbio-plugin-${id}/PLUGIN.yml`
    const manifest = readPluginManifest(root, id)
    const declared = ownerViewNames(
      productionTexts(path.join(root, 'crates', `${PLUGIN_CRATE_PREFIX}${id}`, 'src')),
    )
    for (const name of parseViewList(manifest.shared_views, where)) {
      if (!declared.has(name)) {
        throw new Error(
          `${where}：shared_views 声明了 ${JSON.stringify(name)}，但 ${id} 的生产代码里没有` +
            '提交对应的 `ViewDecl { name: … }`——没有实现的视图读不出值',
        )
      }
      const prior = byName.get(name)
      if (prior) {
        throw new Error(`${where}：${name} 已由 ${prior.owner} 声明——一条视图名只能有一个 owner`)
      }
      byName.set(name, { name, owner: id })
    }
  }
  return [...byName.values()].sort((a, b) => a.name.localeCompare(b.name))
}

/** 纯函数：声明清单 → 文件内容。导出给回归测试（测试不触发写盘）。 */
export function render(rows) {
  const L = [HEADER, '']
  L.push('/// 一条公开给别的插件按名字读的视图：名字 + owner。')
  L.push('///')
  L.push('/// `ver` / `cost` / 求值函数不在这里：它们随 owner 的 `ViewDecl` 走，读口在')
  L.push('/// 冻结声明表里查——本表多抄一份就会与 owner 漂移。')
  L.push('#[derive(Debug, Clone, Copy, PartialEq, Eq)]')
  L.push('pub struct SharedViewDecl {')
  L.push('    pub name: &\'static str,')
  L.push('    pub owner: &\'static str,')
  L.push('}')
  L.push('')
  if (rows.length === 0) {
    L.push('/// 当前没有跨插件可读的视图。owner 在自家 `PLUGIN.yml` 的 `shared_views:` 里登记一行。')
    L.push('pub const SHARED_VIEWS: &[SharedViewDecl] = &[];')
    return `${L.join('\n').replace(/\n+$/, '')}\n`
  }
  L.push('/// 跨插件可读视图（按名字排序）。')
  L.push('#[rustfmt::skip]')
  L.push('pub const SHARED_VIEWS: &[SharedViewDecl] = &[')
  for (const r of rows) {
    L.push(`    SharedViewDecl { name: "${r.name}", owner: "${r.owner}" },`)
  }
  L.push('];')
  return `${L.join('\n').replace(/\n+$/, '')}\n`
}

export default {
  id: 'views-rs',
  title: '跨插件可读视图名（各插件 `PLUGIN.yml` 的 `shared_views` 投影）',
  out: OUT_REL,
  stamp: false,
  render: ({ root }) => render(collectSharedViews(root)),
}
