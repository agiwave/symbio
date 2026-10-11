#!/usr/bin/env node
/**
 * registry-lib — 登记机制共享库：单文档解析、schema 校验、汇总渲染
 *
 * ## 这套顶层机制是什么
 *
 * 项目里的汇总文档（待办清单 / 规范清单 / 批次台账…）此前都是**手写表格**：
 * 状态流转靠人改格子、依赖倒置没人判、「表里的编号指到不存在的东西」无人核对——
 * 正是 `plan/13` 纪律 2（机制优先）与纪律 4（出口可核对）要治的形态。
 * 顶层解法只有三段：
 *
 *   1. **条目 = 数据**：每条任务 / 规范是一个带 front-matter 的单文档
 *      （`registry/<kind>/<id>.md`），编号、状态、依赖、证据全在机读字段里；
 *   2. **汇总 = 投影**：任何「列表 / 看板 / 总账」都由本库渲染，
 *      `registry/` 下各级 `INDEX.md` 由门禁自动重生成（`gate.d/60-facts.mjs` 的 `autoWork`），
 *      不存在第二处手抄；
 *   3. **校验 = 守卫**：`scripts/registry-audit.mjs` 把字段 / 状态机 / 依赖 /
 *      优先级 / 取代链 / 判据指认全部机械判定（RG-001…RG-015），违规即红，范围读不出 exit 2。
 *
 * ## 受限 YAML 子集（front-matter）
 *
 * 零外部依赖（Node 无内置 YAML），所以**只认以下形状**，其余一律报 RG-001（宁红不漏）：
 *
 *   - 首行 `---` 起始、`---` 闭合；键形如 `key: value`（取**第一个**冒号之后整段为值）；
 *   - 整行注释以 `#` 开头；**行内 `#` 属于值的一部分**（不剥离，避免吞掉正当内容）；
 *   - 值四种形态：裸标量、`"..."` / `'...'`（去外层引号，不支持转义）、
 *     `[]` / `[a, b]` 行内列表、键后换行的 `  - item` 块列表；
 *   - 未知字段 / 重复字段 / 不合形状的行 ⇒ RG-001（未知字段按 kind 判，
 *     错域字段如 task 里写 `scope` 会被指名）。
 *
 * ## 判据码的归属
 *
 * 全部 RG 码声明在**本文件**（消息字面量住这里，`gate-codes-audit` GC-003 要求
 * 声明的码在本文件里出现）。CLI 出口只有两个：`registry-audit`（守卫）与
 * `registry-board`（查询），它们只 import，不重复判据。
 */
// 判据码命名空间（登记表 docs/reference/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns RG 登记条目校验（registry 单文档的字段 / 状态 / 依赖 / 判据指认）
// @codes RG-001 RG-002 RG-003 RG-004 RG-005 RG-006 RG-007 RG-008 RG-009 RG-010 RG-011 RG-012 RG-013 RG-014 RG-015 RG-016

import fs from 'node:fs'
import path from 'node:path'

// ==================== schema 配置 ====================

/** 公共必填字段：所有 kind 一致。 */
export const COMMON_REQUIRED = ['kind', 'id', 'title', 'status', 'created']
/** 公共可选字段：`deps` 允许跨 kind（全 registry 编号唯一才敢跨）。 */
export const COMMON_OPTIONAL = ['deps']

/**
 * 每个 kind 一份配置——**新增一个 kind = 在这里加一项 + 一个渲染器 + 回归测试**，
 * 不新开脚本、不新开命名空间。字段事实的文档侧真相是 `registry/spec/SP-001.md`
 * （其字段表由 RG-011 与本配置对账，两边不等即红）。
 */
export const KINDS = {
  task: {
    label: '任务',
    required: ['track'],
    optional: ['where', 'exit', 'signal', 'evidence', 'reason', 'prio'],
    sections: ['内容', '处理过程'],
    refFields: ['exit'],
    refSections: [],
    statusRules: {
      todo: { require: [], forbid: ['evidence', 'reason'] },
      doing: { require: [], forbid: ['evidence', 'reason'] },
      done: { require: ['evidence'], forbid: ['signal', 'reason'] },
      wont: { require: ['reason'], forbid: ['evidence'] },
    },
    closedStatus: 'done',
    requiresDepsDone: ['doing', 'done'],
  },
  spec: {
    label: '规范',
    required: ['scope'],
    optional: ['reason', 'supersedes', 'clears'],
    sections: ['规范', '判据'],
    refFields: [],
    refSections: ['判据'],
    statusRules: {
      draft: { require: [], forbid: ['reason'] },
      active: { require: [], forbid: ['reason'] },
      retired: { require: ['reason'], forbid: [] },
    },
    closedStatus: 'active',
    requiresDepsDone: ['active'],
  },
}

/** SP-001 字段表必须**恰好**覆盖的键集（= 公共 + 各 kind 的并集）。 */
export const ALL_KEYS = [
  ...COMMON_REQUIRED,
  ...COMMON_OPTIONAL,
  ...new Set([...KINDS.task.required, ...KINDS.task.optional, ...KINDS.spec.required, ...KINDS.spec.optional]),
].sort()

// 编号必须能当文件名：字母开头，其后为字母数字 / . _ - 与 Unicode 字母数字。
// 历史批次号里有 `S-end①` 这类圈号——编号是**身份**不是代号，迁移登记时不得重排。
export const ID_RE = /^[A-Za-z][A-Za-z0-9._\-\p{L}\p{N}\p{M}]+$/u
export const DATE_RE = /^\d{4}-\d{2}-\d{2}$/

/**
 * task 的排序提示词表。**prio 是提示（判断），deps 是义务（事实）**：
 * 只影响同一拓扑层内的排序（先拓扑、后 prio、再编号），不参与任何冲突裁决。
 * 缺席 = 最低档——不想定级不必硬填，判据不因此判红。
 */
export const PRIO = { now: 0, next: 1, later: 2 }

/** RG-010 的消息以常量形式住本文件（GC-003：声明的码必须在声明文件里出现）。 */
export const SCOPE_ERROR =
  'RG-010 登记范围读不出：registry/ 下没有任何条目——退化成「什么都没校验」是最坏的失败'

// ==================== front-matter 解析 ====================

function unquote(s) {
  if (s.length >= 2 && (s[0] === '"' || s[0] === "'") && s[s.length - 1] === s[0]) return s.slice(1, -1)
  return s
}

function parseScalar(raw) {
  const v = raw.trim()
  if (v === '[]') return []
  if (v.startsWith('[') && v.endsWith(']')) {
    const inner = v.slice(1, -1).trim()
    return inner === '' ? [] : inner.split(',').map((s) => unquote(s.trim()))
  }
  return unquote(v)
}

/**
 * 解析一个条目文件。
 *
 * @param text 文件全文
 * @param name 报错用的仓根相对路径
 * @returns {{meta: object|null, body: string, errors: string[]}}
 *          解析失败时 `meta` 为 null（调用方跳过语义校验，避免级联噪音）。
 */
export function parseEntryText(text, name) {
  const errors = []
  if (text.charCodeAt(0) === 0xfeff) text = text.slice(1)
  const lines = text.split(/\r?\n/)
  if (lines[0]?.trim() !== '---') {
    return { meta: null, body: '', errors: [`${name}：RG-001 首行必须是 \`---\`（front-matter 起始分隔符）`] }
  }
  let close = -1
  for (let i = 1; i < lines.length; i++) {
    if (lines[i].trim() === '---') {
      close = i
      break
    }
  }
  if (close < 0) {
    return { meta: null, body: '', errors: [`${name}：RG-001 front-matter 没有用 \`---\` 闭合`] }
  }
  const meta = {}
  let i = 1
  while (i < close) {
    const raw = lines[i]
    if (!raw.trim() || raw.trim().startsWith('#')) {
      i++
      continue
    }
    const m = raw.match(/^([A-Za-z][A-Za-z0-9_-]*):(.*)$/)
    if (!m) {
      errors.push(`${name}：RG-001 第 ${i + 1} 行不是 \`键: 值\` 形状：${raw.trim()}`)
      i++
      continue
    }
    const key = m[1]
    const rest = m[2]
    if (key in meta) {
      errors.push(`${name}：RG-001 字段 \`${key}\` 重复`)
      i++
      continue
    }
    if (rest.trim() === '') {
      // 块列表（`key:` 换行 + `  - item`）或空值
      const items = []
      let j = i + 1
      while (j < close && /^\s+-\s+\S/.test(lines[j])) {
        items.push(lines[j].replace(/^\s+-\s+/, '').trim())
        j++
      }
      if (items.length) {
        meta[key] = items
        i = j
      } else {
        meta[key] = null
        i++
      }
    } else {
      meta[key] = parseScalar(rest)
      i++
    }
  }
  return { meta, body: lines.slice(close + 1).join('\n'), errors }
}

// ==================== 装载 ====================

/**
 * 扫 `registry/`：每个**已登记**子目录按其 kind 装载 `*.md`（`INDEX.md` 是生成物，跳过）。
 *
 * @param registryDir `<root>/registry`
 * @param root 仓根（算相对路径用）
 * @returns {{entries: Array, errors: string[], notes: string[], missing: boolean}}
 */
export function loadEntries(registryDir, root) {
  const errors = []
  const notes = []
  const entries = []
  if (!fs.existsSync(registryDir)) return { entries, errors, notes, missing: true }
  for (const ent of fs.readdirSync(registryDir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
    if (!ent.isDirectory()) continue // 仓根 INDEX.md 是生成物
    const kind = ent.name
    const cfg = KINDS[kind]
    if (!cfg) {
      errors.push(
        `RG-013 registry/${kind}/：未登记的 kind 目录——命名空间必须闭合，先在 registry-lib 的 KINDS 里声明`,
      )
      continue
    }
    const dir = path.join(registryDir, kind)
    const files = fs
      .readdirSync(dir)
      .filter((n) => n.endsWith('.md') && n !== 'INDEX.md')
      .sort()
    if (files.length === 0) notes.push(`registry/${kind}/：暂无条目（${cfg.label}）`)
    for (const name of files) {
      const file = path.join(dir, name)
      const text = fs.readFileSync(file, 'utf8')
      const rel = path.relative(root, file).split(path.sep).join('/')
      const { meta, body, errors: perr } = parseEntryText(text, rel)
      entries.push({ file, rel, kind, name, meta, body, errors: perr })
    }
  }
  return { entries, errors, notes, missing: false }
}

// ==================== 校验 ====================

export function isEmpty(v) {
  return v == null || (typeof v === 'string' && !v.trim()) || (Array.isArray(v) && v.length === 0)
}

/** 取 `## <name>` 小节正文（到下一个二级标题为止）；不存在返回 ''。 */
export function sectionOf(body, name) {
  const m = body.match(new RegExp(`^##\\s+${name}\\s*$`, 'm'))
  if (!m) return ''
  const after = body.slice(m.index + m[0].length)
  const next = after.search(/^##\s/m)
  return next < 0 ? after : after.slice(0, next)
}

/** 正文 / 字段里形如 `docs/x.md`、`scripts/y.mjs`、`e2e/z.mjs` 的反引号指认。 */
export function refsIn(text) {
  return [...text.matchAll(/`((?:docs|scripts|e2e)\/[^`\s]+\.(?:mjs|md|rs))`/g)].map((m) => m[1])
}

function validDate(s) {
  if (!DATE_RE.test(s)) return false
  const d = new Date(`${s}T00:00:00Z`)
  return !Number.isNaN(d.getTime()) && d.toISOString().slice(0, 10) === s
}

/**
 * 同层比较器：先 `prio`（now < next < later < 缺席），再编号字典序。
 * 拓扑约束永远优先于它——`deps` 是义务（事实），`prio` 是提示（判断）。
 */
function cmpPrio(byIdEntry) {
  const rank = (id) => {
    const p = byIdEntry.get(id)?.meta?.prio
    return typeof p === 'string' && p in PRIO ? PRIO[p] : 9
  }
  return (a, b) => rank(a) - rank(b) || (a < b ? -1 : a > b ? 1 : 0)
}

/** 在已有依赖图上找一条环（确定性：按 id 字典序起步，deps 排序）。 */function findCycle(ids, depsOf) {
  const state = new Map()
  const stack = []
  let cycle = null
  const visit = (id) => {
    if (cycle) return
    const s = state.get(id) ?? 0
    if (s === 2) return
    if (s === 1) {
      cycle = [...stack.slice(stack.indexOf(id)), id]
      return
    }
    state.set(id, 1)
    stack.push(id)
    for (const d of depsOf(id)) visit(d)
    stack.pop()
    state.set(id, 2)
  }
  for (const id of [...ids].sort()) visit(id)
  return cycle
}

/**
 * 全量校验。返回**纯文本**一行一条的违规（颜色由 CLI 上），
 * 空数组 = 零违规。`root` 用于 RG-012 的存在性判定。
 */
export function auditEntries(entries, root) {
  const out = []
  const byId = new Map()

  for (const e of entries) {
    out.push(...e.errors)
    if (!e.meta) continue
    const cfg = KINDS[e.kind]
    const meta = e.meta
    const allowed = new Set([...COMMON_REQUIRED, ...COMMON_OPTIONAL, ...cfg.required, ...cfg.optional])

    // RG-001 未知字段（按 kind 判，错域字段被指名）
    for (const k of Object.keys(meta)) {
      if (!allowed.has(k)) out.push(`${e.rel}：RG-001 未知字段 \`${k}\`（${e.kind} 允许：${[...allowed].sort().join('、')}）`)
    }
    // RG-002 必填非空 + kind 与目录一致
    for (const k of [...COMMON_REQUIRED, ...cfg.required]) {
      if (isEmpty(meta[k])) out.push(`${e.rel}：RG-002 必填字段 \`${k}\` 缺失或为空`)
    }
    if (!isEmpty(meta.kind) && meta.kind !== e.kind) {
      out.push(`${e.rel}：RG-002 kind=${meta.kind} 与所在目录不一致（目录声明它是 ${e.kind}）`)
    }
    // RG-003 编号形状 / 文件名对账 / 全 registry 唯一
    if (!isEmpty(meta.id)) {
      if (typeof meta.id !== 'string' || !ID_RE.test(meta.id)) {
        out.push(`${e.rel}：RG-003 id 不合形状（字母开头，其后为字母数字 / . _ - 与 Unicode 字母数字；必须可作文件名）：${JSON.stringify(meta.id)}`)
      } else {
        if (e.name !== `${meta.id}.md`) {
          out.push(`${e.rel}：RG-003 文件名必须等于编号（应为 \`${meta.id}.md\`）——文件名即编号的对账`)
        }
        if (byId.has(meta.id)) {
          out.push(`${e.rel}：RG-003 编号 \`${meta.id}\` 重复（已登记于 ${byId.get(meta.id).rel}）`)
        } else {
          byId.set(meta.id, e)
        }
      }
    }
    // RG-004 状态词
    if (!isEmpty(meta.status) && !(meta.status in cfg.statusRules)) {
      out.push(`${e.rel}：RG-004 status=${JSON.stringify(meta.status)} 不在 ${e.kind} 的状态词（${Object.keys(cfg.statusRules).join(' / ')}）`)
    }
    // RG-005 日期
    if (!isEmpty(meta.created) && (typeof meta.created !== 'string' || !validDate(meta.created))) {
      out.push(`${e.rel}：RG-005 created 不是有效日期（YYYY-MM-DD）：${JSON.stringify(meta.created)}`)
    }
    // RG-014 排序提示词表（prio 是提示不是义务：义务在 deps，词表封闭只是为了让排序确定）
    if (!isEmpty(meta.prio) && !(typeof meta.prio === 'string' && meta.prio in PRIO)) {
      out.push(`${e.rel}：RG-014 prio 不在词表（${Object.keys(PRIO).join(' / ')}）：${JSON.stringify(meta.prio)}`)
    }
    // RG-006 deps 形状 / 自指 / 重复（存在性与成环在跨文件段判）
    if ('deps' in meta && !isEmpty(meta.deps) && !Array.isArray(meta.deps)) {
      out.push(`${e.rel}：RG-006 deps 必须是 \`[]\` / \`[A, B]\` / 块列表，读到：${JSON.stringify(meta.deps)}`)
    }
    if (Array.isArray(meta.deps)) {
      const seen = new Set()
      for (const d of meta.deps) {
        if (d === meta.id) out.push(`${e.rel}：RG-006 deps 含自身（自己不能是自己的前置）`)
        if (seen.has(d)) out.push(`${e.rel}：RG-006 deps 重复登记 \`${d}\``)
        seen.add(d)
      }
    }
    // RG-007 状态 × 字段矩阵
    const rule = typeof meta.status === 'string' ? cfg.statusRules[meta.status] : null
    if (rule) {
      for (const k of rule.require) {
        if (isEmpty(meta[k])) out.push(`${e.rel}：RG-007 status=${meta.status} 必须填 \`${k}\``)
      }
      for (const k of rule.forbid) {
        if (!isEmpty(meta[k])) out.push(`${e.rel}：RG-007 status=${meta.status} 不得带 \`${k}\`（已填非空值）`)
      }
    }
    // RG-009 正文小节
    for (const sec of cfg.sections) {
      if (!new RegExp(`^##\\s+${sec}\\s*$`, 'm').test(e.body)) {
        out.push(`${e.rel}：RG-009 正文缺 \`## ${sec}\` 小节`)
      }
    }
    // RG-012 判据指认的文件必须存在（落点 `where` 不判——落点可以尚不存在）
    for (const f of cfg.refFields) {
      if (typeof meta[f] !== 'string') continue
      for (const ref of refsIn(meta[f])) {
        if (!fs.existsSync(path.join(root, ref))) {
          out.push(`${e.rel}：RG-012 \`${f}\` 指认的 \`${ref}\` 不存在（出口判据不得依赖尚不存在的东西）`)
        }
      }
    }
    for (const sec of cfg.refSections) {
      for (const ref of refsIn(sectionOf(e.body, sec))) {
        if (!fs.existsSync(path.join(root, ref))) {
          out.push(`${e.rel}：RG-012 判据小节指认的 \`${ref}\` 不存在`)
        }
      }
    }
  }

  // ---- 跨条目：依赖存在性（RG-006）、依赖状态（RG-008）、成环（RG-006） ----
  for (const e of entries) {
    if (!e.meta || !Array.isArray(e.meta.deps)) continue
    const cfg = KINDS[e.kind]
    for (const d of e.meta.deps) {
      const dep = byId.get(d)
      if (!dep) {
        out.push(`${e.rel}：RG-006 deps 指到不存在的编号 \`${d}\``)
        continue
      }
      if (cfg.requiresDepsDone.includes(e.meta.status) && dep.meta) {
        const depClosed = KINDS[dep.kind].closedStatus === dep.meta.status
        if (!depClosed) {
          out.push(
            `${e.rel}：RG-008 status=${e.meta.status} 要求前置 \`${d}\` 已闭环，而它是 ${dep.meta.status}` +
              `（${KINDS[dep.kind].label}的闭环状态是 ${KINDS[dep.kind].closedStatus}）`,
          )
        }
      }
    }
  }
  {
    const ids = [...byId.keys()]
    const depsOf = (id) => (Array.isArray(byId.get(id).meta.deps) ? byId.get(id).meta.deps.filter((d) => byId.has(d)) : [])
    const cycle = findCycle(ids, depsOf)
    if (cycle) out.push(`RG-006 依赖成环：${cycle.join(' -> ')}（环上的编号永远等不到前置闭环）`)
  }

  // ---- RG-015：spec 的 supersedes（显式取代链——冲突靠数据消解，不靠优先级裁决） ----
  for (const e of entries) {
    if (!e.meta || e.kind !== 'spec' || !('supersedes' in e.meta)) continue
    const v = e.meta.supersedes
    if (isEmpty(v)) continue
    if (!Array.isArray(v)) {
      out.push(`${e.rel}：RG-015 supersedes 必须是 \`[编号]\` 列表，读到：${JSON.stringify(v)}`)
      continue
    }
    const seen = new Set()
    for (const t of v) {
      if (t === e.meta.id) {
        out.push(`${e.rel}：RG-015 supersedes 含自身（自己不取代自己）`)
        continue
      }
      if (seen.has(t)) out.push(`${e.rel}：RG-015 supersedes 重复指认 \`${t}\``)
      seen.add(t)
      const target = byId.get(t)
      if (!target) {
        out.push(`${e.rel}：RG-015 supersedes 指到不存在的编号 \`${t}\``)
        continue
      }
      if (target.kind !== 'spec') {
        out.push(`${e.rel}：RG-015 supersedes 指向 ${target.kind}（\`${t}\`）——取代关系只在规范之间`)
        continue
      }
      if (e.meta.status === 'active' && target.meta && target.meta.status === 'active') {
        out.push(
          `${e.rel}：RG-015 status=active 的规范不得取代另一条 active 规范（\`${t}\`）` +
            '——两条现行规范同框即矛盾未消解，先把旧条 retired',        )
      }
    }
  }
  {
    const specIds = [...byId.values()].filter((x) => x.kind === 'spec').map((x) => x.meta.id)
    const supOf = (id) => {
      const m = byId.get(id).meta
      return Array.isArray(m.supersedes) ? m.supersedes.filter((t) => t !== id && byId.has(t)) : []
    }
    const cycle = findCycle(specIds, supOf)
    if (cycle) out.push(`RG-015 supersedes 成环：${cycle.join(' -> ')}（取代链必须有尽头）`)
  }

  // ---- RG-016：spec 的 clears（豁免的清除任务指认） ----
  // clears 是**指认**不是前置（与 deps 的语义分界）：豁免可以合法地挂在**开放**的
  // 清除任务上——那正是「现在不修 + 倒计时」的语义，所以这里**不**做 RG-008 式的闭环要求。
  for (const e of entries) {
    if (!e.meta || e.kind !== 'spec' || !('clears' in e.meta)) continue
    const v = e.meta.clears
    if (isEmpty(v)) continue
    if (!Array.isArray(v)) {
      out.push(`${e.rel}：RG-016 clears 必须是 \`[编号]\` 列表，读到：${JSON.stringify(v)}`)
      continue
    }
    const seen = new Set()
    for (const t of v) {
      if (seen.has(t)) out.push(`${e.rel}：RG-016 clears 重复指认 \`${t}\``)
      seen.add(t)
      const target = byId.get(t)
      if (!target) {
        out.push(`${e.rel}：RG-016 clears 指到不存在的编号 \`${t}\`（清除任务必须先登记）`)
        continue
      }
      if (target.kind !== 'task') {
        out.push(`${e.rel}：RG-016 clears 指向 ${target.kind}（\`${t}\`）——清除任务是任务，不是规范`)
      }
    }
  }

  // ---- RG-011：SP-001 的字段表必须恰好等于代码侧 schema ----
  const sp = entries.find((e) => e.kind === 'spec' && e.meta && e.meta.id === 'SP-001')
  if (!sp) {
    out.push('RG-011 registry/spec/SP-001.md 不存在——schema 的文档侧真相没有登记')
  } else {
    const m = sp.body.match(/^##\s+字段\s*$/m)
    if (!m) {
      out.push('RG-011 registry/spec/SP-001.md 缺 \`## 字段\` 小节——字段表读不出')
    } else {
      const section = sectionOf(sp.body, '字段')
      const keys = [...section.matchAll(/^\|\s*`([A-Za-z][A-Za-z0-9_-]*)`\s*\|/gm)].map((x) => x[1])
      if (keys.length === 0) {
        out.push('RG-011 registry/spec/SP-001.md 字段表读不出（\`## 字段\` 小节里没有 \`| \`键\` |\` 形状的行）')
      } else {
        const got = new Set(keys)
        const want = new Set(ALL_KEYS)
        const missing = ALL_KEYS.filter((k) => !got.has(k))
        const extra = keys.filter((k) => !want.has(k))
        if (missing.length || extra.length) {
          out.push(
            'RG-011 registry/spec/SP-001.md 字段表与代码 schema 不一致' +
              (missing.length ? `；表里缺：${missing.join('、')}` : '') +
              (extra.length ? `；表里多出：${extra.join('、')}` : '') +
              '（改 schema 必须同批更新 SP-001，反之亦然）',
          )
        }
      }
    }
  }

  return out
}

// ==================== 排序与渲染 ====================

/**
 * 拓扑序（前置在前），同层按 id 字典序——**确定性**，是汇总页可再生的前提。
 * 有环时退回字典序（环已被 RG-006 判红，渲染不再抢报）。
 */
export function topoOrder(entries) {
  const withMeta = entries.filter((e) => e.meta && !isEmpty(e.meta.id))
  const ids = withMeta.map((e) => e.meta.id)
  const idSet = new Set(ids)
  const indeg = new Map(ids.map((id) => [id, 0]))
  const dependents = new Map(ids.map((id) => [id, []]))
  for (const e of withMeta) {
    for (const d of Array.isArray(e.meta.deps) ? e.meta.deps : []) {
      if (!idSet.has(d)) continue
      indeg.set(e.meta.id, indeg.get(e.meta.id) + 1)
      dependents.get(d).push(e.meta.id)
    }
  }
  const byIdEntry = new Map(withMeta.map((e) => [e.meta.id, e]))
  const ready = ids.filter((id) => indeg.get(id) === 0).sort(cmpPrio(byIdEntry))
  const order = []
  while (ready.length) {
    const id = ready.shift()
    order.push(id)
    for (const dep of dependents.get(id).slice().sort()) {
      indeg.set(dep, indeg.get(dep) - 1)
      if (indeg.get(dep) === 0) {
        ready.push(dep)
        ready.sort(cmpPrio(byIdEntry))
      }
    }
  }
  if (order.length < ids.length) {
    const seen = new Set(order)
    order.push(...ids.filter((id) => !seen.has(id)).sort())
  }
  return order.map((id) => byIdEntry.get(id)).filter(Boolean)
}

const cell = (s) => String(s ?? '')
  .replace(/\|/g, '\\|')
  .replace(/\s*\n\s*/g, ' ')
  .trim() || '—'

const depsCell = (e) => (Array.isArray(e.meta.deps) && e.meta.deps.length ? e.meta.deps.join('、') : '—')

const HEADER_TASK =
  '> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 从 `registry/task/*.md` 生成，请勿手改。**\n' +
  '> 状态的唯一登记处是各任务文件本身，本页是投影。字段规范见 [SP-001](../spec/SP-001.md)，' +
  '原则与流程见 [14 · 登记与汇总机制](../../docs/plan/14-登记与汇总机制.md)。\n'

const HEADER_SPEC =
  '> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 从 `registry/spec/*.md` 生成，请勿手改。**\n' +
  '> 状态的唯一登记处是各规范文件本身，本页是投影。字段规范见 [SP-001](./SP-001.md)，' +
  '原则与流程见 [14 · 登记与汇总机制](../../docs/plan/14-登记与汇总机制.md)。\n'

function table(header, rows) {
  if (rows.length === 0) return '（无）'
  return [`| ${header.join(' | ')} |`, `|${header.map(() => '---').join('|')}|`, ...rows.map((r) => `| ${r.join(' | ')} |`)].join('\n')
}

function renderTaskIndex(allEntries) {
  const mine = topoOrder(allEntries.filter((e) => e.kind === 'task'))
  const count = (s) => mine.filter((e) => e.meta.status === s).length
  const lines = [
    '# 任务登记索引（自动生成）',
    '',
    HEADER_TASK,
    `**计数**：☐ todo ${count('todo')} · 🟡 doing ${count('doing')} · ☑ done ${count('done')} · ❌ wont ${count('wont')}（共 ${mine.length}）`,
    '',
    '## 待处理（按依赖顺序）',
    table(
      ['编号', '任务', '优先', '轨', '依赖'],
      mine.filter((e) => e.meta.status === 'todo').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.prio), cell(e.meta.track), depsCell(e)]),
    ),
    '',
    '## 进行中',
    table(
      ['编号', '任务', '优先', '轨', '依赖'],
      mine.filter((e) => e.meta.status === 'doing').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.prio), cell(e.meta.track), depsCell(e)]),
    ),
    '',
    '## 已完成',
    table(
      ['编号', '任务', '轨', '证据'],
      mine.filter((e) => e.meta.status === 'done').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.track), cell(e.meta.evidence)]),
    ),
    '',
    '## 不做',
    table(
      ['编号', '任务', '轨', '理由'],
      mine.filter((e) => e.meta.status === 'wont').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.track), cell(e.meta.reason)]),
    ),
    '',
  ]
  return lines.join('\n')
}

/** spec 的判据列：`## 判据` 里第一个 `scripts/*.mjs` 指认；没有则是人工判据。 */
function guardCell(e) {
  const ref = refsIn(sectionOf(e.body, '判据')).find((r) => r.startsWith('scripts/'))
  return ref ? `\`${ref}\`` : '人工'
}

/** supersedes 列：显式取代链（冲突消解留下的数据痕迹）。 */
const supCell = (e) => (Array.isArray(e.meta.supersedes) && e.meta.supersedes.length ? e.meta.supersedes.join('、') : '—')

/** clears 列：豁免类规范的清除任务指认（存在性由 RG-016 保证）。 */
const clearsCell = (e) => (Array.isArray(e.meta.clears) && e.meta.clears.length ? e.meta.clears.join('、') : '—')

function renderSpecIndex(allEntries) {
  const mine = allEntries.filter((e) => e.kind === 'spec').slice().sort((a, b) => (a.meta.id < b.meta.id ? -1 : 1))
  const count = (s) => mine.filter((e) => e.meta.status === s).length
  const lines = [
    '# 规范登记索引（自动生成）',
    '',
    HEADER_SPEC,
    `**计数**：active ${count('active')} · draft ${count('draft')} · retired ${count('retired')}（共 ${mine.length}）`,
    '',
    '## 现行规范（active）',
    table(
      ['编号', '规范', '适用范围', '判据', '清除', '取代'],
      mine.filter((e) => e.meta.status === 'active').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.scope), guardCell(e), clearsCell(e), supCell(e)]),
    ),
    '',
    '## 草案（draft）',
    table(
      ['编号', '规范', '适用范围', '取代'],
      mine.filter((e) => e.meta.status === 'draft').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.scope), supCell(e)]),
    ),
    '',
    '## 废止（retired）',
    table(
      ['编号', '规范', '理由', '取代'],
      mine.filter((e) => e.meta.status === 'retired').map((e) => [`\`${e.meta.id}\``, cell(e.meta.title), cell(e.meta.reason), supCell(e)]),
    ),
    '',
  ]
  return lines.join('\n')
}

const RENDERERS = { task: renderTaskIndex, spec: renderSpecIndex }

/** 某 kind 的汇总页全文（确定性：同输入同输出，无时间戳）。 */
export function renderKindIndex(kind, allEntries) {
  return RENDERERS[kind](allEntries)
}

/** 仓根总账页：各 kind 的条目数、状态分布、汇总页链接。 */
export function renderRootIndex(allEntries) {
  const rows = Object.entries(KINDS).map(([kind, cfg]) => {
    const mine = allEntries.filter((e) => e.kind === kind)
    const dist = Object.keys(cfg.statusRules)
      .map((s) => `${s} ${mine.filter((e) => e.meta && e.meta.status === s).length}`)
      .join(' · ')
    return [`[${kind} · ${cfg.label}](./${kind}/INDEX.md)`, String(mine.length), dist, `[${kind}/INDEX.md](./${kind}/INDEX.md)`]
  })
  return [
    '# 登记总账（自动生成）',
    '',
    '> ⚠️ **本文件由 `node scripts/registry-audit.mjs --write` 生成，请勿手改。**',
    '> 条目各是一个单文档（`registry/<kind>/<id>.md`）；机制见 [14 · 登记与汇总机制](../docs/plan/14-登记与汇总机制.md)，',
    '> 字段规范见 [SP-001](./spec/SP-001.md)。',
    '',
    table(['类别', '条目数', '状态分布', '汇总页'], rows),
    '',
  ].join('\n')
}
