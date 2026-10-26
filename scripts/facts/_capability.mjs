/**
 * facts/_capability —— 「能力矩阵」的提取实现（`docs/004-ref/006-当前事实表.md` 那一节的输入）
 *
 * ## 它回答什么
 *
 * 「每条 LLM 可见工具 / VDFS 操作 / 前端路由，**用户因此看到什么**」。
 * 三样语义在代码里本来就有，且是**同一份**——它们就长在各自东西旁边：
 *
 * | 面 | 语义出处 |
 * |---|---|
 * | LLM 工具 | `CapabilityMeta.description`（模型与前端看到的原文）+ `category` / `risk`（决定它怎么被呈现、审批） |
 * | VDFS 操作 | `VDFS_*` 常量正上方的 `///`（该操作的一句话定义） |
 * | 前端路由 | 插件 `route()` 里那条 `match` 臂正上方的 `///`（该地址的一句话职责） |
 *
 * 所以这里是**提取**不是复述：改一句描述、重跑，矩阵就跟着变；人不用维护任何表格。
 *
 * ## 诚实优先于完整（与 `facts/current.mjs` 的 `TOOL_NOTES` 反面教材同一条）
 *
 * 抽取不到的字段一律渲染成 `—`，**绝不替它编一句**。运行期拼出来的描述
 * （`local` 的 shell 工具用 `format!` 带上操作系统名）就是抽不到——那一格是空的，
 * 空得对：表格里那句「操作系统 shell」会骗人以为有个固定文案。
 *
 * 这一节的判据形态与 §1/§2 的**覆盖面**判据相反：那一节是「抽不到就标动态」，
 * 这一节是「抽不到就标空」——两者都不许臆造，只是空格的位置不同。
 */

import path from 'node:path'
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import { matchBrace, stripComments, stripTestModules } from '../rust-scan.mjs'
import { routeArmDocs, vdfsOps } from '../route-facts.mjs'
import { UNWIRED_MARKER } from '../route-facts.mjs'

const PLUGINS_DIR_REL = ['crates']
const PLUGIN_CRATE_PREFIX = 'symbio-plugin-'

/** 抽不到时的占位。**空着比编一句强**——见文件头。 */
export const UNKNOWN = '—'

/**
 * VDFS 地址根名（`plugins/vdfs/fs.rs` 的 `VDFS_ADDR_ROOT`）。
 *
 * 输出到文档时替换为 `<vdfs_root>` 占位（见 S-010：根名字面量只归 vdfs 插件）——
 * 「生成器不手写事实」那条纪律的同一条：根名改了，两份投影（RE-006 的能力矩阵与
 * RE-003 的静态骨架）自动跟着改，且口径只有这一处。
 *
 * 读不出来**抛错**而不是回落空串：空串会让 mask 静默变成恒等，占位符从此悄悄消失。
 */
export function vdfsAddrRoot() {
  const here = path.dirname(fileURLToPath(import.meta.url))
  const file = path.resolve(
    here, '..', '..', ...PLUGINS_DIR_REL, PLUGIN_CRATE_PREFIX + 'vdfs', 'src', 'fs.rs',
  )
  const src = fs.readFileSync(file, 'utf8')
  const m = src.match(/pub const VDFS_ADDR_ROOT:\s*&str\s*=\s*"([^"]+)"/)
  if (!m) throw new Error('cannot extract VDFS_ADDR_ROOT from plugins/vdfs/fs.rs')
  return m[1]
}

// ---------- 源文件收集 ----------

/** 某插件目录下参与生产事实的源码，**带注释**（语义就住在注释里）。 */
function rawSources(pluginDir) {
  const out = []
  const walk = (dir) => {
    let entries
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true })
    } catch {
      return
    }
    for (const ent of entries) {
      if (ent.name === 'docs') continue
      const p = path.join(dir, ent.name)
      if (ent.isDirectory()) walk(p)
      else if (
        ent.name.endsWith('.rs') &&
        !ent.name.endsWith('.test.rs') &&
        ent.name !== 'tests.rs'
      ) {
        out.push(p)
      }
    }
  }
  walk(pluginDir)
  return out
    .map((f) => fs.readFileSync(f, 'utf8'))
    // 未接线模块（`#![allow(dead_code)]`）里的东西模型看不到，同 route-facts 的口径排除
    .filter((raw) => !raw.includes(UNWIRED_MARKER))
    // 语义在 `///` 里，故**不去注释**；但内联测试模块里的同名工具会重复计入，去掉它
    .map((raw) => stripTestModules(raw))
}

// ---------- LLM 工具 ----------

/** 收集 `const NAME: &str = "值"`（工具名由常量给出时靠它解析）。 */
function parseStrConsts(text, map) {
  for (const m of text.matchAll(/\bconst\s+([A-Z][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]+)"/g)) {
    map.set(m[1], m[2])
  }
  return map
}

/** 大括号配对取结构体字面量的内容（`matchBrace` 是全仓唯一实现）。 */
function structBody(text, marker) {
  const i = text.indexOf(marker)
  if (i < 0) return null
  const open = text.indexOf('{', i + marker.length - 1)
  if (open < 0) return null
  return text.slice(open, matchBrace(text, open) + 1)
}

const lastVariant = (s) => (s ? s.split('::').pop() : null)

/** `name: "x"` / `name: CONST.to_string()` / `name: NAME.into()` */
function readName(block, consts) {
  let m = block.match(/\bname:\s*"([a-z][a-z0-9_]*)"/)
  if (m) return m[1]
  m = block.match(/\bname:\s*([A-Za-z_][A-Za-z0-9_]*)\s*\.\s*(?:to_string|into)\(\)/)
  if (m) return consts.get(m[1]) ?? null
  return null
}

/**
 * 工具描述 → 表里那一格。
 *
 * 两种形态：
 *  1. `description: "字面量"` —— 直接取（绝大多数工具）；
 *  2. `description: format!("字面量 {动态}", …)` —— 取**字面量的开头一段**。
 *
 * ## 第 2 种为什么要「取开头」而不是「跳过」
 *
 * `agent_run` / `read_skill` 的描述是运行期拼的（拼进可用智能体清单、技能清单）。
 * 跳过它们，那两格就空着，而空着会被读成「这个工具没有描述」——与事实相反。
 * 取开头一段既是真的（模型确实先看到这几字），又不会把「{智能体清单}」那种
 * 只有运行期才知道的内容**编**进表里。截断处标 `…`，不假装取全了。
 */
function readDescription(block) {
  // 转义类用 `\\[\s\S]` 而不是 `\\.`：Rust 的行续写是「反斜杠 + 换行 + 缩进」，
  // `.` 不匹配换行 ⇒ `format!` 里带续写的描述（`agent_run`）整条抽不到，
  // 而症状是「那个工具没有描述」——看起来像没写，其实是提取漏了。
  const L = '((?:[^"\\\\]|\\\\[\\s\\S])*)'
  let m = block.match(new RegExp(`\\bdescription:\\s*(?:\\n\\s*)?"${L}"`))
  if (m) return m[1].replace(/\\"/g, '"')

  m = block.match(new RegExp(`\\bdescription:\\s*format!\\(\\s*"${L}"`))
  if (!m) return null
  const literal = m[1].replace(/\\\s*\n\s*/g, ' ').replace(/\\"/g, '"').replace(/\\n/g, ' ')
  const cut = literal.search(/\{|\s{2,}/)
  const head = (cut < 0 ? literal : literal.slice(0, cut)).trim()
  return (cut < 0 ? head : `${head}…`).replace(/\s{2,}/g, ' ').trim()
}

const enumOf = (block, field, enumName) => {
  const m = block.match(new RegExp(`\\b${field}:\\s*(?:Some\\(\\s*)?[\\w:]*${enumName}::([A-Za-z]+)`))
  return m ? m[1] : null
}

/**
 * 一个插件的全部 LLM 工具语义：`Map<工具短名, {description, category, risk}>`。
 *
 * 三种声明形态都吃，因为它们是**同一份事实在代码里的三种写法**：
 *  1. `CapabilityMeta { … }` 字面量（绝大多数工具）
 *  2. `ToolSpec { name, description, risk }` 表项（vdfs 的七个同形工具）
 *  3. `tool("name", "desc", …)` 调用（vdfs 表装不下的两个：`vdfs_read` / `vdfs_list`）
 *
 * 判据是「**取到什么就记什么**」：某形态里没有的字段留空，不跨形态补——
 * 跨形态补出来的组合可能根本不属于同一条工具。
 */
export function toolFacts(root) {
  const base = path.join(root, ...PLUGINS_DIR_REL)
  const out = new Map()
  let dirs = []
  try {
    dirs = fs
      .readdirSync(base, { withFileTypes: true })
      .filter(
        (e) =>
          e.isDirectory() &&
          e.name.startsWith(PLUGIN_CRATE_PREFIX) &&
          e.name !== PLUGIN_CRATE_PREFIX + 'api',
      )
  } catch {
    return out
  }
  for (const ent of dirs.sort((a, b) => (a.name < b.name ? -1 : 1))) {
    const consts = new Map()
    const sources = rawSources(path.join(base, ent.name))
    for (const t of sources) parseStrConsts(t, consts)

    for (const raw of sources) {
      // 去注释后再取结构体字面量：注释里的示意代码不该被读成事实。
      // （`description` 的文案本身是字符串字面量，不受影响）
      const text = stripComments(raw)
      const put = (name, fields) => {
        if (!name) return
        const prev = out.get(name) ?? { description: null, category: null, risk: null }
        out.set(name, {
          description: prev.description ?? fields.description ?? null,
          category: prev.category ?? fields.category ?? null,
          risk: prev.risk ?? fields.risk ?? null,
        })
      }

      for (const marker of ['CapabilityMeta {', 'ToolSpec {']) {
        let idx = text.indexOf(marker)
        while (idx >= 0) {
          const open = text.indexOf('{', idx)
          const block = matchBrace(text, open) >= 0 ? text.slice(open, matchBrace(text, open) + 1) : ''
          const name = readName(block, consts)
          put(name, {
            description: readDescription(block),
            category: lastVariant(enumOf(block, 'category', 'CapabilityCategory')),
            risk: lastVariant(enumOf(block, 'risk', 'CapabilityRiskLevel')),
          })
          idx = text.indexOf(marker, idx + marker.length)
        }
      }

      // `tool("name", "description", …)`：表装不下的两个 vdfs 工具走这里。
      // 风险档是签名里最后那个实参（`tools/mod.rs::tool`），抽出来——否则同一族
      // 工具在表里一半有风险档、一半空着，而空着会被读成「没风险」。
      for (const m of text.matchAll(/\btool\(\s*"([a-z][a-z0-9_]*)"\s*,\s*(?:\n\s*)?"((?:[^"\\]|\\.)*)"/g)) {
        const call = text.slice(m.index, matchBrace(text, m.index + m[0].length - 1) + 1)
        const risk = /CapabilityRiskLevel::([A-Za-z]+)/.exec(call)?.[1] ?? null
        put(m[1], {
          description: m[2].replace(/\\"/g, '"'),
          risk,
        })
      }
    }
  }
  return out
}

// ---------- VDFS 操作 ----------

/**
 * 每个 VDFS 操作的一句话定义：`Map<操作地址, 定义>`，取自**声明它的那个常量**
 * 正上方的 `///`。
 *
 * 三个跨插件可见的操作（`root` / `watch` / `unwatch`）归 core，其余归 vdfs 插件的
 * 协议文件——两份都收，取值时以先声明者为准（它们是**同一个常量**，不是两份事实）。
 */
export function vdfsOpDocs(root) {
  const out = new Map()
  const files = [
    path.join(root, 'crates', 'symbio-core', 'src', 'plugin', 'route.rs'),
    path.join(root, 'crates', 'symbio-plugin-vdfs', 'src', 'protocol.rs'),
  ]
  for (const file of files) {
    let text
    try {
      text = fs.readFileSync(file, 'utf8')
    } catch {
      continue
    }
    const lines = text.split('\n')
    /** 当前连续注释块；取它的**第一行**（一句话定义），不是最后一行 */
    let block = null
    for (const line of lines) {
      const doc = line.match(/^\s*\/\/\/\s?(.*)$/)
      if (doc) {
        if (doc[1].trim() && (block === null || block.length === 0)) block = [doc[1].trim()]
        else if (doc[1].trim()) block.push(doc[1].trim())
        continue
      }
      const decl = line.match(
        /\b(?:ROUTE_)?VDFS_[A-Z0-9_]+\s*:\s*&(?:'static\s+)?str\s*=\s*"([^"]+)"/,
      )
      if (decl) {
        if (block && block.length && !out.has(decl[1])) {
          // core 里那三条的定义首行以地址自名开头（`vdfs/root — 进入地址空间…`），
          // 表里已经有地址列了，去掉自名免得读成叠句
          const text = block[0].replace(new RegExp(`^${decl[1]}\\s*[—–-]\\s*`), '')
          out.set(decl[1], text)
        }
        block = null
        continue
      }
      // 模块文档 `//!` 与普通 `//` 都不构成定义；空行同样断开连续块
      if (!/^\s*\/\//.test(line)) block = null
    }
  }
  return out
}

// ---------- 路由 ----------

/** 每条静态路由的一句话职责（提取实现见 `route-facts.routeArmDocs`）。 */
export function routeDocs(root) {
  return routeArmDocs(root)
}

/** 三族事实的集中入口：`docs/004-ref/006-当前事实表.md` 能力矩阵那节只认这一个。 */
export function capabilityFacts(root) {
  return {
    tools: toolFacts(root),
    vdfsOps: vdfsOps(root),
    vdfsOpDocs: vdfsOpDocs(root),
    routeDocs: routeDocs(root),
  }
}