#!/usr/bin/env node
/**
 * 封闭集声明 —— 「同一事实不许写两遍」判据读的**同一份**声明
 *
 * ## 它是什么
 *
 * `scripts/facts.test.mjs` 的跨文档判据要回答一个问题：**哪些成员集合**
 * （错误码 / 配置键 / 判据码 / 工具 / 操作 / 路由）只许在它们的 owner 里成表，
 * 在别处成表即「同一事实写了两遍」。这名单以前**散落在测试文件里**（三个集合
 * 写死在 `CLOSED_SETS` 常量中），于是：
 *
 *   - 新增一类 owner 投影（错误码表 / 配置表 / 判据码表）要改测试，
 *     而测试是判据的**住所**——改判据与改被判的东西混在一处；
 *   - 「哪些 owner 已被覆盖、哪些还是欠账」这个问题，代码里读不出来，
 *     只能靠注释里的一句「见某份清单」。承诺进了散文，就没有执行方。
 *
 * 本模块把「成员集合 + owner」收敛成**一份**声明：每个集合写明成员从哪来
 * （代码 / Rust 枚举 / 脚本声明 / owner 自己的清单表），owner **不写死路径**，
 * 从各文档 front-matter 的 `lists: [<id>=owner]` 声明里读（与
 * `facts/lists-index.mjs` 投影的 `RE-007-清单归属总账` 同源）。判据只读本模块。
 *
 * ## 为什么 owner 从声明读，不写死
 *
 * 写死路径的失效形态是**静默漏判**：RE-006 被改名或搬目录那一刻，
 * 常量里的路径開始指向一个不存在的文件 ⇒ 集合空 ⇒ 判据对全仓视而不见，
 * 而它照样全绿。从声明读则相反：声明动了而标记没动，RG-029 先红；
 * 声明与标记都动了，这里跟着动；两头都不在，这里报「owner 不见了」。
 *
 * ## 成员从哪来（四种 origin，各有理由）
 *
 * | origin | 集合 | 真源 | 为什么不读 owner 文档 |
 * |---|---|---|---|
 * | `code` | 工具 / VDFS 操作 / 路由 | `_capability.mjs` + 扫插件目录 | owner 文档本身就是代码投影，读它是循环论证 |
 * | `rust` | 错误码 | `PluginErrorCode::as_str` 的匹配臂 | 同上；且这样能顺带判「Rust 加了码、表没收」 |
 * | `scripts` | 判据码 | 各脚本的 `@ns` / `@codes` 声明（`gate-codes-audit.collect`） | 同上（GATE_CODES 由同一份声明生成） |
 * | `owner-list` | 配置键 | owner 清单表自己被标记的那一块 | 配置键散在 N 个 `config.rs` 里，**今天没有**单一代码侧清单；见下面的已知边界 |
 *
 * ## 已知边界（写在这里，不靠判据假装没看见）
 *
 * - **配置键集合的成员就是 owner 表自己**。因此这条判据守的是「第二份抄的」，
 *   不是「第一份对不对」——第一份的正确性归各自的 owner（RE-001 自己写明
 *   「默认值以代码为准」，session 侧还有 `config_definition_covers_every_session_config_field`
 *   守着字段全表）。要升级成代码侧提取，得先有一份跨插件的键清单，那是另一件事。
 * - **owner 表里写「见下」的键不在集合内**（RE-001 的 gateway / telegram 两行）。
 *   「见下」不是清单声明，而它们的详细键表在 owner 文档内部——跨文档判据
 *   跨不进去，也不该跨进去（同一文档内的重复归文件内判据）。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { loadEntries } from '../registry-lib.mjs'
import { capabilityFacts } from './_capability.mjs'
import { pluginSources, extractRouteArms, vdfsOps } from '../route-facts.mjs'
import { collect as collectGateCodes } from '../gate-codes-audit.mjs'

/** 正文里的清单标记：`<!-- list:<id> owner -->`（RG-029 守它与 front-matter 两面对账）。 */
const LIST_MARK = (id) => new RegExp(`<!--\\s*list:\\s*${id}\\s+owner\\s*-->`)

/**
 * 取「被标记的那一块」：从标记起，到下一个 `##`/`###` 标题（`## ` 的前缀不匹配 `### `——
 * 用 `indexOf('\n## ')` 会把 `### ` 小节一起吃进来，第一次试就多收了 200 行）。
 */
export function markedBlock(text, listId) {
  const m = text.match(LIST_MARK(listId))
  if (!m) return null
  const rest = text.slice(m.index + m[0].length)
  const j = rest.search(/^#{2,3}[^#]/m)
  return j === -1 ? rest : rest.slice(0, j)
}

/**
 * 「哪份清单归谁」——读 front-matter 的 `lists: [<id>=owner]` 声明，
 * 与 `RE-007-清单归属总账` 同一份输入。
 *
 * 重复 owner 在这里**直接抛错**，不挑第一个：两份 owner 意味着「两份事实」，
 * 判据 silently 认其中一个，等于替人做了它不该做的裁决。分工有重叠由 RG-027 判红，
 * 这里报的是「判据自己读到的声明是坏的」。
 */
function listOwners(root) {
  const { entries, projections } = loadEntries(root)
  const owner = new Map()
  for (const e of [...entries, ...projections]) {
    const raw = e?.meta?.lists
    if (!Array.isArray(raw)) continue
    for (const item of raw) {
      const [id, role] = String(item).split('=').map((s) => s.trim())
      if (!id || role !== 'owner') continue
      if (owner.has(id) && owner.get(id) !== e.rel) {
        throw new Error(`清单 \`${id}\` 有两个 owner：${owner.get(id)} 与 ${e.rel}——判据拒绝裁决，分工重叠归 RG-027`)
      }
      owner.set(id, e.rel)
    }
  }
  return owner
}

/**
 * 错误码：`PluginErrorCode::as_str` 的匹配臂（线上字面量，即 `code` 字段的值）。
 *
 * `UNKNOWN` **刻意不收**：它是对端版本超前 / 帧未携带 code 时的兜底，
 * 不是生产侧会发出的分类，RE-002 的表也不收它。收了它会让「owner 没收全部成员」
 * 那条判据永久差一个。
 */
function rustErrorCodes(root) {
  const rel = 'symbio/src/symbio_core/plugin/error.rs'
  const txt = fs.readFileSync(path.join(root, rel), 'utf8')
  // 分组 1 是变体名、分组 2 才是线上字面量——取错组的失效形态很安静：集合里装满
  // `InternalError` 这类**哪儿都不出现**的名字，扫描一份都命中不了，判据恒真。
  // 「owner 必须逐字收留每个成员」那条就是抓这个的（首跑当场抓到）。
  const arms = [...txt.matchAll(/PluginErrorCode::(\w+)\s*=>\s*"([A-Z][A-Z0-9_]*)"/g)]
  const codes = arms.filter(([, , lit]) => lit !== 'UNKNOWN').map(([, , lit]) => lit)
  if (codes.length < 10) {
    throw new Error(`${rel} 里只认出 ${codes.length} 个错误码——判据自己失效了（as_str 的匹配臂形状变了？）`)
  }
  return new Set(codes)
}

/** 判据码：各脚本自己的 `@ns` / `@codes` 声明（GATE_CODES 由同一份声明渲染）。 */
function scriptGateCodes(root) {
  const codes = new Set()
  for (const s of collectGateCodes(root)) for (const code of s.codes.keys()) codes.add(code)
  if (codes.size < 10) {
    throw new Error(`@codes 声明里只认出 ${codes.size} 个判据码——判据自己失效了`)
  }
  return codes
}

/**
 * 配置键：owner 清单表里「主要键」一列的反引号键。
 *
 * 两条收窄，都是被噪声教出来的：
 *   - 只取**第一个「；」之前的段**——`；` 之后是解释（「字段全表见哪个文件」、
 *     「哪个单测守着」），里面的反引号是文件名与测试名，不是键；
 *   - 只认 `snake_case` / `点路径` 形状——`true`、`native` 这类**值**不是键。
 */
function ownerListConfigKeys(text, listId) {
  const block = markedBlock(text, listId)
  if (!block) return new Set()
  const keys = new Set()
  for (const line of block.split(/\r?\n/)) {
    if (!line.startsWith('|')) continue
    const cells = line.split('|').slice(1, -1)
    if (cells.length < 3) continue
    const keyCol = cells[2].split('；')[0]
    for (const m of keyCol.matchAll(/`([^`]+)`/g)) {
      if (/^[a-z][a-z0-9_]*(\.[a-z0-9_]+)?$/.test(m[1])) keys.add(m[1])
    }
  }
  return keys
}

/**
 * 集合声明。`name` 是判据日志里的人话；`listId` 是 front-matter 里的清单 id。
 *
 * 新增一行的纪律：**成员要么从代码 / 声明提，要么从 owner 的被标记清单提**——
 * 不许在测试文件里手抄一份名单（手抄的那一份本身就是「同一事实写两遍」，
 * 而且它必然与真源漂移，然后判据开始恒真）。
 */
const DECLARATIONS = [
  { name: 'LLM 可见工具', listId: 'tools', origin: 'code' },
  { name: 'VDFS 操作', listId: 'vdfs-ops', origin: 'code' },
  { name: '前端 / 宿主路由', listId: 'routes', origin: 'code' },
  { name: '错误码', listId: 'error-codes', origin: 'rust' },
  { name: '配置项', listId: 'plugin-configs', origin: 'owner-list' },
  { name: '判据码', listId: 'gate-codes', origin: 'scripts' },
]

/**
 * 职责边界表里**没有成员集合可判**的 owner 投影，逐行写明理由。
 *
 * 写这一栏是为了让「覆盖每一个 owner 投影」成为一句**可查的话**：每个投影要么
 * 在上面有一份集合声明，要么在这里有一条不带成员集合的理由。不写的话，
 * 「都覆盖了吗」只能靠人读一遍职责边界表——而那正是判据要省掉的那件事。
 *
 * - **PROMPT_SHAPE**：投影物是**运行时形状**（消息序列 / system 小节标签 / 信封），
 *   判据是 PS-001 逐字段比，本来就不按名单判；里面唯一像名单的东西是工具面，
 *   而它与「LLM 可见工具」是同一份真源（代码），再立一份成员表就是复述。
 * - **DE-009 的操作族**：DE-009 §5 自己写明 `VDFS_OPS` 是**唯一操作清单**，
 *   即它的操作族与「VDFS 操作」集合同源同成员——不新增第二份成员表。
 */
const NO_MEMBER_SET = [
  {
    owner: 'docs/004-ref/PROMPT_SHAPE.md',
    why:
      '投影物是运行时形状而非名单；判据是 PS-001 逐字段比（口径见 D-057）。' +
      '其中唯一像名单的工具面与「LLM 可见工具」同源（代码），再立一份成员表即为复述',
  },
  {
    owner: 'docs/003-design/009-VDFS规范.md',
    why:
      '§5 自己声明 `plugins/vdfs/protocol.rs` 的 `VDFS_OPS` 是唯一操作清单——' +
      '操作族与「VDFS 操作」集合同源同成员，不新增第二份',
  },
]

/**
 * 物化全部集合 ⇒ `name → { members, listId, ownerRel, origin }`。
 *
 * 任一集合的 owner 不见了（声明被删 / 文档被搬）当场抛错：判据看不见自己的 owner
 * 时唯一正确的行为是停下来，不是继续扫一个空集合然后全绿。
 */
export function closedSets(root) {
  const owners = listOwners(root)
  const cap = capabilityFacts(root)
  const routes = new Set()
  const base = path.join(root, 'symbio', 'src', 'plugins')
  // 插件目录**扫出来**而不是写死一份名单：写死的名单会与新增插件分叉，而分叉的
  // 方向是「静默漏判」——新插件的路由不进集合，于是它在别处被复述时判据看不见。
  for (const p of fs.readdirSync(base, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name)) {
    for (const src of pluginSources(path.join(base, p))) {
      for (const a of extractRouteArms(src, p)) routes.add(a)
    }
  }

  const out = new Map()
  for (const d of DECLARATIONS) {
    const ownerRel = owners.get(d.listId)
    if (!ownerRel) {
      throw new Error(`集合「${d.name}」的 owner 不见了：没有任何文档声明 \`lists: [${d.listId}=owner]\`——判据拒绝空转`)
    }
    let members
    if (d.origin === 'code') {
      members = d.listId === 'tools' ? new Set([...cap.tools.keys()]) : d.listId === 'vdfs-ops' ? new Set(cap.vdfsOps) : routes
    } else if (d.origin === 'rust') {
      members = rustErrorCodes(root)
    } else if (d.origin === 'scripts') {
      members = scriptGateCodes(root)
    } else {
      const ownerText = fs.readFileSync(path.join(root, ownerRel), 'utf8')
      members = ownerListConfigKeys(ownerText, d.listId)
    }
    out.set(d.name, { members, listId: d.listId, ownerRel, origin: d.origin })
  }
  return out
}

export { NO_MEMBER_SET, DECLARATIONS }
