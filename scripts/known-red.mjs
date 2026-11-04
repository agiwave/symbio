/**
 * known-red 台账的读写与对账（`LP-006` / `CP-009` 共用）
 *
 * ## 它不是豁免
 *
 * 台账命中的红**仍然**让调用方 `exit 1`：`known-red` 只回答「这一处红是不是已经被承认、
 * 钉在哪一刀上」，不回答「能不能放过」。把真实红洗成绿是 `baseline-allow` 的事，
 * 而那条通道在 `baseline-scope-audit` 里只对机制面指纹开放，且不许碰任何计数。
 *
 * ## 为什么按 `code + file + kind + token` 归并，而不是按行号
 *
 * 行号在重构里天天变，按行号登记会让台账每次 `cargo fmt` 都过期；而红的位置真正稳定
 * 的身份是「哪个文件里的哪一类词元」。同一身份出现几次由 `count` 对账：
 * - 实测 > 登记 ⇒ 新增，红（**不许**用它悄悄扩容）；
 * - 实测 < 登记 ⇒ 已清除未销账，红（**必须**同批把这一行删掉或调小）。
 *
 * 于是台账只增不减这件事在这里不成立：它和门禁的棘轮一样只许往下走。
 */

import fs from 'node:fs'
import path from 'node:path'

export const KNOWN_RED_REL = 'scripts/known-red.json'

export function findingKey(code, file, kind, token) {
  return `${code}|${file}|${kind}|${token}`
}

/** `null` = 读不出（调用方应退 2，不许当「没有台账」放行）。 */
export function loadKnownRed(root, rel = KNOWN_RED_REL) {
  const file = path.join(root, rel)
  if (!fs.existsSync(file)) return null
  try {
    const data = JSON.parse(fs.readFileSync(file, 'utf8'))
    return {
      entries: Array.isArray(data.entries) ? data.entries : [],
      exemptions: Array.isArray(data.exemptions) ? data.exemptions : [],
      note: data.note ?? '',
    }
  } catch {
    return null
  }
}

/** `D-NNN` 是否真的存在：当前文档里点名过它（只读，不改 docs）。 */
export function adrExists(root, adr) {
  const dir = path.join(root, 'docs')
  if (!fs.existsSync(dir)) return false
  const seen = new Set()
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name)
      if (e.isDirectory()) {
        walk(p)
        continue
      }
      if (!e.name.endsWith('.md')) continue
      const text = fs.readFileSync(p, 'utf8')
      if (text.includes(adr)) seen.add(p)
    }
  }
  walk(dir)
  return seen.size > 0
}

/** 已知的迁移单元名。台账的「刀」列必须指向其中**真实存在**的那一步；拼错（U5 / B3a /
 * `B2a` …）即台账形状错——刀次写错不会让红消失，只会让清除任务指向不存在的单元。 */
const KNOWN_UNITS = new Set(['U2', 'U3', 'U4', 'B3'])

/** 从 `刀 <单元名>：<说明>` 里取单元名；形状不对返回 null。 */
function knifeUnit(knife) {
  const m = /^刀\s+([A-Za-z0-9]+)\s*[：:]/.exec(String(knife).trim())
  return m ? m[1] : null
}

/**
 * 对账。`findings` 是 `Map<key, {file, kind, token, count, lines}>`。
 *
 * @returns {{
 *   known: object[], unregistered: object[], stale: object[], malformed: object[],
 *   exemption: object|null, exemptionProblem: string|null
 * }}
 */
export function reconcile({ code, findings, loaded, requireExemption = false, root }) {
  const entries = (loaded?.entries ?? []).filter((e) => e.code === code)
  // `findings` 的键是 `file|kind|token`（不含 code）；台账也按同一把尺子量。
  const entryKey = (e) => `${e.file}|${e.kind}|${e.token}`
  const byKey = new Map(entries.map((e) => [entryKey(e), e]))
  const known = []
  const unregistered = []
  const stale = []
  const malformed = []

  for (const [key, f] of findings) {
    const entry = byKey.get(key)
    if (!entry) {
      unregistered.push({ ...f, reason: '未登记' })
      continue
    }
    const entryCount = Number(entry.count ?? 0)
    if (f.count > entryCount) {
      unregistered.push({ ...f, reason: `新增（台账 ${entryCount} → 实测 ${f.count}）` })
    } else if (f.count < entryCount) {
      stale.push({ ...f, entry, reason: `台账多于实测（${entryCount} → ${f.count}），已清除未销账` })
    } else {
      known.push({ ...f, entry })
    }
    if (!entry.problem || !entry.knife || !entry.clearWhen) {
      malformed.push({ ...entry, reason: '缺少 problem / knife / clearWhen 三者之一' })
    } else {
      const unit = knifeUnit(entry.knife)
      if (!unit) {
        malformed.push({ ...entry, reason: 'knife 未以「刀 <单元名>：」开头' })
      } else if (!KNOWN_UNITS.has(unit)) {
        malformed.push({ ...entry, reason: `knife 单元名 "${unit}" 不是已知单元（U2 / U3 / U4 / B3）` })
      }
    }
  }

  for (const entry of entries) {
    const key = entryKey(entry)
    if (!findings.has(key)) stale.push({ ...entry, reason: '台账指向已不存在的红（已清除未销账）' })
  }

  let exemption = null
  let exemptionProblem = null
  if (requireExemption) {
    exemption = (loaded?.exemptions ?? []).find((e) => e.code === code) ?? null
    if (!exemption) {
      exemptionProblem = `${code} 的规则内例外没有登记：豁免必须带决策号（不许匿名豁免）`
    } else if (!exemption.adr || !/^D-\d{3}$/.test(exemption.adr)) {
      exemptionProblem = `${code} 的豁免决策号缺失或形状不对（应为 \`D-NNN\`）：${JSON.stringify(exemption.adr)}`
    } else if (!adrExists(root, exemption.adr)) {
      exemptionProblem = `${code} 的豁免引用了不存在的决策 ${exemption.adr}——文档里找不到它，等于匿名豁免`
    } else if (!exemption.why || !exemption.clearWhen) {
      exemptionProblem = `${code} 的豁免缺少 why / clearWhen——冻结保留也要写清它凭什么留、由哪一刀清`
    }
  }

  return { known, unregistered, stale, malformed, exemption, exemptionProblem }
}
