#!/usr/bin/env node
/**
 * baseline-scope-audit — 棘轮基线面的**变更审**（`36-baseline-scope`）
 *
 * ## 它治什么
 *
 * `loop-purity-audit` 的 LP-001…005 与 `control-plane-audit` 的 CP-006 都是「只许降」的
 * 棘轮，但**棘轮自己是被谁调的**此前没有判据：`--write-baseline` 一条命令就能把
 * 实测值写回基线，于是「新增一处世界词根」与「修好一处」在 diff 里长得一模一样——
 * 只要顺手重写基线，门禁全绿。十次洗红里九次不是有人决定作恶，而是**同批把基线
 * 对齐到了现状**。
 *
 * 本脚本把「基线变了」本身变成一次需要署名的变更：
 *
 * | 码 | 判什么 | 处理 |
 * |---|---|---|
 * | `BS-001` | 基线**新增**键（一个新的文件 / 子键 / 名单项） | 红，**没有** allow 通道——注册新债不许靠改基线 |
 * | `BS-002` | 任一项计数**上调** | 红，**没有** allow 通道——这是把真实红洗成绿的唯一路径 |
 * | `BS-003` | 基线**删除**键（债已归零）而没有 `baseline-allow` | 红；删除是清债，必须留下决策号 |
 * | `BS-004` | `control-plane.json` 的 `mechanism.file` / `mechanism.hash` 变了而没有 allow | 红；机制面演进必须署名 |
 * | `BS-005` | allow 记录形状不对 / 决策号不存在 / 指向一个没发生的变更 | 红；不许匿名或空头豁免 |
 * | `BS-006` | `scripts/baselines/*.json`（控制面以外）**新增一格**而没有 `(none)→<格>` allow | 红；账本里长出新的一格就是注册新债，必须署名 |
 * | `BS-007` | 同一份账本里**计数上调**或**指纹改写**而没有 allow | 红；把基线对齐到现状 = 洗红，必须署名 |
 *
 * **为什么 `BS-001` / `BS-002` 不给 allow，而 `BS-006` / `BS-007` 给**：两份被逐格冻结的基线
 * （`loop-purity` / `control-plane`）的**每一格都挂着自己的判据**，长出一格或抬高一个数，正确
 * 答案只有一个——把那处红修掉；给它们留 allow 就是把「洗红」做成一条合法命令。其余账本
 * （`plugin-api-loc` / `arch-dead-code` / `sink-rows` / `tool-names` / `port-surface` /
 * `tool-surface`）的语义是**登记存量**：它们的判据（`FA-001` / `AD-001` / `SR-001` …）只对
 * 「生产 vs 基线」发话，同批把基线与生产一起改是**它们自己文档里写明的合法动作**（新增一处
 * 必须同批登记 `why` / `adr`）。所以这里判的不是「不许改」，而是「改了就必须署名」——
 * 否则一次 `--print-baseline` 重写就能把「新增一处债」和「修好一处」在 diff 里抹成同一形态。
 *
 * 账本面按**目录**发现（`scripts/baselines/*.json`，控制面除外），不维护第二份清单：
 * 新增一份基线文件自动进入本审计，不需要谁记得登记（`D-491`）。
 *
 * ## `baseline-allow` 的形状
 *
 * 写在**被改的那份基线文件**的顶层数组里（`loop-purity-baseline.json` 或
 * `baselines/control-plane.json`），一行一条：
 *
 * ```json
 * "baseline-allow": [
 *   "sha256:old→sha256:new 驱动循环体随 D-324 迁移，逻辑逐字未动 D-324",
 *   "lp003WorldRoots/entry.rs→(none) 该文件的世界词根已归零 D-324"
 * ]
 * ```
 *
 * 解析口径：`<old>→<new> <理由> D-NNN`（`D-NNN` 必须真的存在于 `docs/` 里）。
 * **它只对「删除 / 机制面变化」开放**；计数上调与新增键永远不接受 allow——那两件事
 * 的正确答案不是「登记一条豁免」，而是「把这处红修掉」。
 *
 * **allow 只对「相对合并基尚未落地」的变更有效**。`to` 一旦成为当前基线的值，它就
 * 不再是豁免而是历史记录：本轮没有任何 diff 可命中，继续留着就是**永久豁免**——
 * 没人敢删、也没人知道它还在不在起作用。这种记录判红，文案直说「已落地 allow，请删除」
 * （`allowLanded`）。清理动作本身就是下一批的一部分；空头（`from` 与 `to` 相同 /
 * `to` 不等于任何当前基线值）照旧判红。
 *
 * ## 与「known-red」的区别
 *
 * `known-red` 是**还没修的红**（门禁仍然 exit 1）；`baseline-allow` 是**在途变更的
 * 署名**（机制演进 / 清债），它让那条变更从「洗红」变成「署名」，并随那一批一起落地、
 * 落地后必须删除。两者都不许静默：前者逐条列出清除刀，后者必须带决策号。
 *
 * ## 用法
 *
 *   node scripts/audits/baseline-scope-audit.mjs              # 与 HEAD 比（本地在途改动）
 *   node scripts/audits/baseline-scope-audit.mjs --base=<rev> # CI 必须传合并基
 *   node scripts/audits/baseline-scope-audit.mjs --root=<dir> # 回归测试用
 *
 * 退出码：0 = 通过；1 = 有违规；2 = 范围读不出（不是 git 仓 / 读不到基线 / JSON 坏了）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns BS 棘轮基线面的变更审
// @codes BS-001 BS-002 BS-003 BS-004 BS-005 BS-006 BS-007
// @gate test

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import { red, green, dim } from '../color.mjs'
import { hasDecisionAnchor } from '../decision-anchors.mjs'

const argv = process.argv.slice(2)
const rootArg = argv.find((a) => a.startsWith('--root='))
const baseArg = argv.find((a) => a.startsWith('--base='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
const BASE = baseArg ? baseArg.slice('--base='.length) : process.env.SYMBIO_BASE_REV || 'HEAD'

const LP_REL = 'scripts/loop-purity-baseline.json'
const CP_REL = 'scripts/baselines/control-plane.json'

/** 元数据键不参与键集对账。 */
const META = new Set(['note', 'baseline-allow'])

let errors = 0
function report(code, where, message) {
  console.log(`${red('[ERROR]')} ${code} ${where}  ${message}`)
  errors++
}

function readAtBase(rel) {
  const r = spawnSync('git', ['-C', ROOT, 'show', `${BASE}:${rel}`], { encoding: 'utf8' })
  if (r.status !== 0) return null
  return r.stdout
}

function readNow(rel) {
  const file = path.join(ROOT, rel)
  return fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null
}

function parse(rel, text) {
  if (text === null) return null
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

/**
 * `baseline-allow` 解析。返回 `{ records, malformed }`；`records` 里带 `from` / `to` /
 * `reason` / `adr` / `used`。
 */
function allowRecords(obj) {
  const records = []
  const malformed = []
  const list = Array.isArray(obj?.['baseline-allow']) ? obj['baseline-allow'] : []
  for (const item of list) {
    const m = /^(.+?)→(.+?)\s+(.+?)\s+(D-\d{3})$/.exec(String(item).trim())
    if (!m) {
      malformed.push(String(item))
      continue
    }
    if (!hasDecisionAnchor(m[4].slice(2), ROOT)) {
      malformed.push(`${item}（决策 ${m[4]} 不存在）`)
      continue
    }
    records.push({ from: m[1], to: m[2], reason: m[3], adr: m[4], raw: String(item), used: false })
  }
  return { records, malformed }
}

function findAllow(records, from, to) {
  const hit = records.find((r) => r.from === from && r.to === to)
  if (hit) hit.used = true
  return hit
}

/**
 * 一条**没被本轮命中**的 allow 是不是「已落地」——是就得删，不是「可以留着」。
 *
 * `baseline-allow` 只对**相对合并基尚未落地**的变更有效：它是一次署名的在途变更，
 * 不是永久豁免。落地之后（`to` 已成为当前基线值）它就不再对应任何 diff——若继续
 * 放行，它会在文件里住一辈子，没人敢删、也没人知道它还在不在起作用，与「空头豁免」
 * 是同一个病，只是走到了另一面。故本判据判红，文案直说「请删除」。
 *
 * 判据只有两条，都不接受含糊：
 *  - `to` 等于当前基线里的某个值（机制面 file / hash、各棘轮格的计数或名单项）；
 *  - 或 `to` 是 `(none)` 且 `from` 指的键**在当前基线里确实不存在**（清债已落地）。
 * 其余情况仍按「指向一个没发生的变更」判红（`from` 与 `to` 相同、`to` 谁也不认识）。
 */
function allowLanded(r, values) {
  if (r.from === r.to) return false
  if (values.has(r.to)) return true
  if (r.to !== '(none)') return false
  return fromPathAbsentNow(r.from)
}

/** `objName/key` 形状的 allow 端点：该键在当前两份基线里都不存在 ⇒ 删除已落地。 */
function fromPathAbsentNow(pathKey) {
  const slash = pathKey.indexOf('/')
  if (slash < 0) return false
  const name = pathKey.slice(0, slash)
  const key = pathKey.slice(slash + 1)
  let seen = false
  for (const obj of [nowLp, nowCp]) {
    const v = obj?.[name]
    if (Array.isArray(v)) return !v.includes(key)
    if (v && typeof v === 'object') return !Object.prototype.hasOwnProperty.call(v, key)
    if (v !== undefined) seen = true
  }
  return !seen
}

/** 当前两份基线里的全部「值」——`allowLanded` 用它判 `to` 是否已落地。 */
function currentBaselineValues() {
  const values = new Set()
  for (const v of [nowCp?.mechanism?.file, nowCp?.mechanism?.hash]) {
    if (v !== undefined) values.add(String(v))
  }
  for (const obj of [nowLp, nowCp]) {
    for (const [k, v] of Object.entries(obj ?? {})) {
      if (META.has(k)) continue
      if (typeof v === 'string' || typeof v === 'number') values.add(String(v))
      else if (Array.isArray(v)) for (const x of v) values.add(String(x))
      else if (v && typeof v === 'object') for (const x of Object.values(v)) values.add(String(x))
    }
  }
  return values
}

// ───────────────────────────── 读入 ─────────────────────────────

const baseLp = parse(LP_REL, readAtBase(LP_REL))
const nowLp = parse(LP_REL, readNow(LP_REL))
const baseCp = parse(CP_REL, readAtBase(CP_REL))
const nowCp = parse(CP_REL, readNow(CP_REL))

for (const [rel, base, now] of [
  [LP_REL, baseLp, nowLp],
  [CP_REL, baseCp, nowCp],
]) {
  if (base === null || now === null) {
    console.error(red(`范围读不出：${rel}（base=${BASE} / 工作树）——基线面审计不许在看不见变更时通过`))
    process.exit(2)
  }
}

const lpAllow = allowRecords(nowLp)
const cpAllow = allowRecords(nowCp)
const key = (a, b) => `${a}/${b}`

// ───────────────────────────── 键集与计数 ─────────────────────────────

function compareMap(rel, objName, base, now, allow) {
  const baseKeys = new Set(Object.keys(base ?? {}))
  const nowKeys = new Set(Object.keys(now ?? {}))
  for (const k of nowKeys) {
    if (!baseKeys.has(k)) {
      report('BS-001', rel, `${objName}/${k} 是新增基线键 —— 注册新债不许靠改基线（先修掉那处红）`)
    }
  }
  for (const k of baseKeys) {
    if (!nowKeys.has(k)) {
      const hit = findAllow(allow.records, key(objName, k), '(none)')
      if (!hit) {
        report('BS-003', rel, `${objName}/${k} 从基线删除 —— 清债必须留下 \`baseline-allow: ${objName}/${k}→(none) <理由> D-NNN\``)
      }
      continue
    }
    const before = Number(base[k])
    const after = Number(now[k])
    if (Number.isFinite(before) && Number.isFinite(after) && after > before) {
      report('BS-002', rel, `${objName}/${k} 的计数 ${before} → ${after} —— 计数只许降，不许同批对齐现状`)
    }
  }
}

function compareArray(rel, objName, base, now, allow) {
  const baseSet = new Set(base ?? [])
  const nowSet = new Set(now ?? [])
  for (const item of nowSet) {
    if (!baseSet.has(item)) report('BS-001', rel, `${objName} 新增项 ${item} —— 注册新债不许靠改基线`)
  }
  for (const item of baseSet) {
    if (nowSet.has(item)) continue
    const hit = findAllow(allow.records, key(objName, item), '(none)')
    if (!hit) {
      report('BS-003', rel, `${objName} 删除项 ${item} —— 清债必须留下 \`baseline-allow: ${objName}/${item}→(none) <理由> D-NNN\``)
    }
  }
}

function compareNumber(rel, objName, base, now) {
  const before = Number(base)
  const after = Number(now)
  if (Number.isFinite(before) && Number.isFinite(after) && after > before) {
    report('BS-002', rel, `${objName} 的计数 ${before} → ${after} —— 计数只许降，不许同批对齐现状`)
  }
}

function compareTop(rel, base, now) {
  const baseKeys = new Set(Object.keys(base ?? {}).filter((k) => !META.has(k)))
  const nowKeys = new Set(Object.keys(now ?? {}).filter((k) => !META.has(k)))
  for (const k of nowKeys) {
    if (!baseKeys.has(k)) report('BS-001', rel, `${k} 是新顶层键 —— 棘轮面不许长出新的一格`)
  }
  for (const k of baseKeys) {
    if (!nowKeys.has(k)) report('BS-003', rel, `${k} 从基线删除 —— 删掉一整格必须留下 allow 记录`)
  }
}

// ───────────────────────────── LP 基线 ─────────────────────────────

compareTop(LP_REL, baseLp, nowLp)
compareMap(LP_REL, 'lp001PayloadImports', baseLp.lp001PayloadImports, nowLp.lp001PayloadImports, lpAllow)
compareArray(LP_REL, 'lp002ForbiddenDeps', baseLp.lp002ForbiddenDeps, nowLp.lp002ForbiddenDeps, lpAllow)
compareMap(LP_REL, 'lp003WorldRoots', baseLp.lp003WorldRoots, nowLp.lp003WorldRoots, lpAllow)
for (const k of ['lp004WorldNamedContracts', 'lp005WorldPayloadSig']) {
  if (baseLp[k] !== undefined && nowLp[k] !== undefined) compareNumber(LP_REL, k, baseLp[k], nowLp[k])
}

// ───────────────────────────── 控制面基线 ─────────────────────────────

compareTop(CP_REL, baseCp, nowCp)
const baseMech = baseCp.mechanism ?? {}
const nowMech = nowCp.mechanism ?? {}
for (const field of ['file', 'hash']) {
  const before = baseMech[field]
  const after = nowMech[field]
  if (before === after) continue
  const hit = findAllow(cpAllow.records, String(before ?? '(none)'), String(after ?? '(none)'))
  if (!hit) {
    report(
      'BS-004',
      CP_REL,
      `mechanism.${field} 从 ${before} 变为 ${after} —— 机制面演进必须留下 ` +
        `\`baseline-allow: ${before}→${after} <理由> D-NNN\``,
    )
  }
}
if (baseCp.extension?.productionFiles !== undefined && nowCp.extension?.productionFiles !== undefined) {
  compareNumber(CP_REL, 'extension.productionFiles', baseCp.extension.productionFiles, nowCp.extension.productionFiles)
}

// ────────────────────────── 数据账本基线（BS-006 / BS-007） ──────────────────────────

/**
 * 数据账本基线：`scripts/baselines/*.json` 里除控制面以外的那几份。
 *
 * **按目录发现，不手写清单**：新加一份基线文件自动进入本审计。控制面被排除是因为它走
 * 的是逐格判据（`BS-004`），两份形状不同、语义也不同。
 */
function ledgerBaselines(root) {
  const dir = path.join(root, 'scripts', 'baselines')
  if (!fs.existsSync(dir)) return []
  return fs
    .readdirSync(dir)
    .filter((n) => n.endsWith('.json'))
    .sort()
    .map((n) => `scripts/baselines/${n}`)
    .filter((rel) => rel !== CP_REL)
}

/** 指纹样（`sha256:…` 或长十六进制）：唯一一种「值变了就算改写」的字符串。 */
const FINGERPRINT_RE = /^(?:sha256:)?[0-9a-f]{16,}$/i

/**
 * 数组项的稳定身份：把项里出现的语义键**全部**拼起来（`symbol|path`），没有才退到下标。
 *
 * 不用「取第一个命中」：同名符号可以住在两个文件里，只取 `symbol` 会让两条不同的账目
 * 在拍平后撞成同一格，新加的那条就带着旧条目的额度进来了。
 */
function identityOf(item, i) {
  if (item && typeof item === 'object' && !Array.isArray(item)) {
    const parts = ['name', 'symbol', 'path', 'key', 'id']
      .map((k) => item[k])
      .filter((v) => typeof v === 'string' && v.trim())
      .map((v) => v.trim())
    if (parts.length) return parts.join('|')
  }
  return String(i)
}

/**
 * 把任意形状的账本拍成「格 → 值」。
 *
 * 只收**数字**（计数 / 行数 / 字节）与**指纹样字符串**：其余字符串是身份（`name` / `path`）
 * 或散文（`why` / `reason` / `adr`），改写它们不是改存量，逐字比只会让 allowance 被琐事喂死。
 * 元数据键（`note` / `baseline-allow`）不参与对账。数组项的身份取 `name` / `symbol` / `path` /
 * `key` / `id`，没有才退回下标。
 */
function flattenCells(node, prefix = '', out = new Map()) {
  if (Array.isArray(node)) {
    node.forEach((item, i) => {
      const cell = `${prefix}@${identityOf(item, i)}`
      const before = out.size
      flattenCells(item, cell, out)
      // 条目本身就是一格，但只在它**没有可数的格**时补上：`arch-dead-code` 的每一格只有
      // 名字与署名，只看数字会让「新增一条零调用符号」隐形成功耗掉额度；而 `plugin-api-loc`
      // 的文件条目已经有 `lines`，再记一条存在性格只会让一次新增要写两条 allow。
      if (out.size === before && item && typeof item === 'object' && !Array.isArray(item)) {
        out.set(cell, 1)
      }
    })
    return out
  }
  if (node && typeof node === 'object') {
    for (const [k, v] of Object.entries(node)) {
      if (prefix === '' && META.has(k)) continue
      flattenCells(v, prefix ? `${prefix}.${k}` : k, out)
    }
    return out
  }
  if (typeof node === 'number' || (typeof node === 'string' && FINGERPRINT_RE.test(node))) {
    out.set(prefix, node)
  }
  return out
}

function compareLedger(rel, base, now, allow) {
  const before = flattenCells(base)
  const after = flattenCells(now)
  for (const [cell, value] of after) {
    if (!before.has(cell)) {
      if (!findAllow(allow.records, '(none)', cell)) {
        report(
          'BS-006',
          rel,
          `新增格 \`${cell}\` —— 账本里长出新的一格就是登记新债；` +
            `同批写 \`baseline-allow: (none)→${cell} <理由> D-NNN\` 或把那处红修掉`,
        )
      }
      continue
    }
    const was = before.get(cell)
    if (typeof value === 'number' && typeof was === 'number') {
      if (value > was && !findAllow(allow.records, `${cell}=${was}`, `${cell}=${value}`)) {
        report(
          'BS-007',
          rel,
          `\`${cell}\` 的计数 ${was} → ${value} —— 把基线对齐到现状就是洗红；` +
            `同批写 \`baseline-allow: ${cell}=${was}→${cell}=${value} <理由> D-NNN\``,
        )
      }
      continue
    }
    if (value !== was && !findAllow(allow.records, String(was), String(value))) {
      report(
        'BS-007',
        rel,
        `\`${cell}\` 的指纹被改写（${String(was).slice(0, 20)}… → ${String(value).slice(0, 20)}…）—— ` +
          `机制面演进必须署名：\`baseline-allow: ${was}→${value} <理由> D-NNN\``,
      )
    }
  }
  for (const cell of before.keys()) {
    if (after.has(cell)) continue
    if (!findAllow(allow.records, cell, '(none)')) {
      report(
        'BS-003',
        rel,
        `格 \`${cell}\` 从账本删除 —— 清债必须留下 \`baseline-allow: ${cell}→(none) <理由> D-NNN\``,
      )
    }
  }
}

for (const rel of ledgerBaselines(ROOT)) {
  const baseText = readAtBase(rel)
  if (baseText === null) {
    // 基线文件本身是本分支新加的：没有前态就判不了「变了什么」，但也不能当「通过」说。
    console.log(dim(`  · ${rel} 在 ${BASE} 里没有前态 —— 本文件跳过（不是通过）`))
    continue
  }
  const baseObj = parse(rel, baseText)
  const nowText = readNow(rel)
  if (baseObj === null) {
    report('BS-003', rel, `前态不是合法 JSON —— 无法对账；先修复 ${BASE} 里的这份文件`)
    continue
  }
  if (nowText === null) {
    report('BS-003', rel, `整份基线被删除 —— 删掉一格（哪怕是一整个文件）必须留下 allow 记录`)
    continue
  }
  const nowObj = parse(rel, nowText)
  if (nowObj === null) {
    report('BS-003', rel, `工作树里的基线不是合法 JSON —— 读不出的账本不上线`)
    continue
  }
  const allow = allowRecords(nowObj)
  for (const m of allow.malformed) {
    report('BS-005', rel, `\`baseline-allow\` 记录不合法：${m} —— 必须写「<old>→<new> <理由> D-NNN」`)
  }
  compareLedger(rel, baseObj, nowObj, allow)
  for (const r of allow.records) {
    if (r.used) continue
    report(
      'BS-005',
      rel,
      `\`baseline-allow\` 没有命中本轮任何变更：${r.raw} —— 要么变更没发生（空头豁免），要么已落地请删除`,
    )
  }
}

// ─────────────────────── allow 记录本身的形状 ───────────────────────

const landedValues = currentBaselineValues()
for (const [rel, parsed] of [
  [LP_REL, lpAllow],
  [CP_REL, cpAllow],
]) {
  for (const m of parsed.malformed) {
    report('BS-005', rel, `\`baseline-allow\` 记录不合法：${m} —— 必须写「<old>→<new> <理由> D-NNN」`)
  }
  for (const r of parsed.records) {
    if (r.used) continue
    if (allowLanded(r, landedValues)) {
      report(
        'BS-005',
        rel,
        `已落地 allow，请删除：${r.raw} —— 它相对合并基已无 diff 可命中，留着就是永久豁免`,
      )
      continue
    }
    report('BS-005', rel, `\`baseline-allow\` 指向一个没有发生的变更：${r.raw}`)
  }
}

// ───────────────────────────── 汇总 ─────────────────────────────

if (errors) {
  console.log(red(`baseline-scope-audit 红：${errors} 项（base=${BASE}）`))
  process.exit(1)
}
console.log(
  green(
    `baseline-scope-audit 通过：棘轮基线相对 ${BASE} 无未署名的格增删 / 计数上调 / 指纹改写，` +
      `无未署名的机制面变化（allow ${lpAllow.records.length + cpAllow.records.length} 条；` +
      `账本面 ${ledgerBaselines(ROOT).length} 份）`,
  ),
)
if (lpAllow.records.length + cpAllow.records.length === 0) {
  console.log(dim('  （没有 baseline-allow 记录：本次没有需要署名的变宽 / 删格）'))
}
process.exit(0)
