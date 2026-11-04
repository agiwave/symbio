/**
 * 逐提交补记表（D-333）：历史提交缺 `Core-Change:` 的**唯一读取与形状校验实现**。
 *
 * `arch-change-audit`（CC-001）与 `capability-migration-audit`（EX-007）共用本模块：
 * 一张表、一个形状判据，两处各自只问「本判据的规则名有没有出现在行里」。
 *
 * 表是机器可读的（`scripts/exemptions/core-change-backfill.json`），因为「这个提交当年
 * 到底改了什么」必须可逐条复核——一张口头的豁免名单三个月后就没人记得它为什么在那儿。
 *
 * ## 「在表内 ⇒ 按表核验」到底核验什么
 *
 * 提交不能改写，所以缺失的 `Core-Change:` 只能由本表补记。形状判据（本模块）保证：
 *
 * - 每条都有 7–40 位提交号、非空 `change`（实际改了什么）、`decision`（那笔改动自己的
 *   裁决号）、`rules`（补记的是哪条判据，且只允许 CC-001 / EX-007）；
 * - `decision` 与表的 `authorized_by` 都能在现行文档里打开——写错的号等于没写；
 * - `authorized_by` 是**表级**的追加授权：新增行只能由一条新决策追加，那条决策必须改
 *   `authorized_by`，且每行的 `authorized_by` 必须与表级同值。判据保证两侧都能打开、
 *   同值；「这次追加是不是真有一条新裁决」超出脚本能判的范围，由 review 判。
 *
 * ## 为什么不让它变成「整条判据退役」
 *
 * 表只覆盖**逐提交**的补记，范围之外（不在表内）的提交仍按原判据判红：缺 `Core-Change:`
 * 就是红，没有「无署名 allow」这种形态。
 */

import fs from 'node:fs'
import path from 'node:path'
import { adrExists } from './adr-index.mjs'

export const BACKFILL_REL = 'scripts/exemptions/core-change-backfill.json'
export const BACKFILL_JUDGEMENTS = ['CC-001', 'EX-007']

const SHA_RE = /^[0-9a-f]{7,40}$/
const ADR_RE = /\bD-\d+\b/

/** `D-NNN` 是否能在现行文档里打开（含模块 README 里的决策）。 */
function opensDecision(root, cited) {
  return adrExists(root, cited)
}

/**
 * 补记表的形状判据（纯函数，判据 = `core-change-backfill.test.mjs`）。
 * 返回错误清单；空数组即成立。
 */
export function validateBackfill(table, root) {
  const errors = []
  if (!table || typeof table !== 'object') return ['补记表读不出（不是 JSON 对象）']
  // 表的 `rule` 就是建立它的那条决策。不把号写成字符串常量：脚本里出现的 `D-\d+`
  // 字面量会被 `gate-codes-audit`（GC-004）当成判据码输出，而决策号不是判据码。
  const rule = String(table.rule ?? '').match(ADR_RE)?.[0] ?? null
  if (!rule) {
    errors.push('补记表缺 `rule`——它是建立本表的那条决策号')
  } else if (!opensDecision(root, rule)) {
    errors.push(`补记表的 rule ${rule} 在现行文档里打不开`)
  }
  const tableAuth = String(table.authorized_by ?? '').match(ADR_RE)?.[0] ?? null
  if (!tableAuth) {
    errors.push('补记表缺表级 `authorized_by`（新增行只能由一条新决策追加）')
  } else if (!opensDecision(root, tableAuth)) {
    errors.push(`表级 authorized_by ${tableAuth} 在现行文档里打不开`)
  }
  if (!Array.isArray(table.entries)) {
    errors.push('补记表缺 `entries` 数组')
    return errors
  }
  const seen = new Set()
  for (const [i, e] of table.entries.entries()) {
    const at = `entries[${i}]${e?.sha ? ` (${e.sha})` : ''}`
    if (!SHA_RE.test(String(e?.sha ?? ''))) {
      errors.push(`${at}：必须有 7–40 位提交号`)
      continue
    }
    if (seen.has(e.sha)) errors.push(`${at}：提交号重复——一个提交只能有一条补记`)
    seen.add(e.sha)
    if (typeof e.change !== 'string' || !e.change.trim()) {
      errors.push(`${at}：缺 \`change\`——不写「实际改了什么」的补记等于一张永久后门`)
    }
    const cited = String(e.decision ?? '').match(ADR_RE)?.[0] ?? null
    if (!cited) {
      errors.push(`${at}：缺 \`decision\`（这笔改动自己的裁决号）`)
    } else if (!opensDecision(root, cited)) {
      errors.push(`${at}：decision ${cited} 在现行文档里打不开（写错的号等于没写）`)
    }
    if (!Array.isArray(e.rules) || e.rules.length === 0) {
      errors.push(`${at}：\`rules\` 必须是非空数组（补记的是哪条判据）`)
    } else {
      for (const r of e.rules) {
        if (!BACKFILL_JUDGEMENTS.includes(r)) {
          errors.push(`${at}：\`rules\` 只允许 ${BACKFILL_JUDGEMENTS.join(' / ')}，读到了 \`${r}\``)
        }
      }
    }
    const rowAuth = String(e.authorized_by ?? '').match(ADR_RE)?.[0] ?? null
    if (!rowAuth) {
      errors.push(`${at}：缺 \`authorized_by\`（这一行由哪条决策批准补记）`)
    } else if (tableAuth && rowAuth !== tableAuth) {
      errors.push(`${at}：\`authorized_by\`=${rowAuth} 与表级 ${tableAuth} 不一致——追加必须同一条决策`)
    } else if (!opensDecision(root, rowAuth)) {
      errors.push(`${at}：authorized_by ${rowAuth} 在现行文档里打不开`)
    }
  }
  return errors
}

/** 读表并校验；出错时 `errors` 非空（调用方据此判红，**不得当成空表**）。 */
export function loadBackfill(root) {
  const file = path.join(root, BACKFILL_REL)
  if (!fs.existsSync(file)) return { entries: [], errors: [`补记表不存在（${BACKFILL_REL}）`] }
  let table
  try {
    table = JSON.parse(fs.readFileSync(file, 'utf8'))
  } catch (e) {
    return { entries: [], errors: [`补记表不是合法 JSON（${e.message}）`] }
  }
  const errors = validateBackfill(table, root)
  if (errors.length) return { entries: table.entries ?? [], errors }
  return { entries: table.entries, errors: [] }
}

/** 本判据名（CC-001 / EX-007）在表内的提交号（短 8 位，与 `git log` 输出同形）。 */
export function backfillShas(entries, judgement) {
  return new Set(
    entries.filter((e) => e.rules.includes(judgement)).map((e) => e.sha.slice(0, 8)),
  )
}
