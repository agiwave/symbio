/**
 * 逐提交补记表（）：历史提交缺 `Core-Change:` 的**唯一读取与形状校验实现**。
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
 * - 每条都有 7–40 位提交号、非空 `change`（实际改了什么）、非空 `why`（今天为什么
 *   补记这条）、`rules`（补记的是哪条判据，且只允许 CC-001 / EX-007）；
 * - `rule` 是建立本表的判据码，`why` 是本表存在的当前理由；两者都不是历史案例编号；
 * - 表是既成事实的登记，不是授权：新增行要有当前理由，范围外的提交照旧判红。
 *
 * ## 为什么不让它变成「整条判据退役」
 *
 * 表只覆盖**逐提交**的补记，范围之外（不在表内）的提交仍按原判据判红：缺 `Core-Change:`
 * 就是红，没有「无署名 allow」这种形态。
 *
 * ## 使用边界：只登记既成事实，不承担授权
 *
 * `why` 只是「今天为什么补记这一行」的当前理由，**不是**被补记改动本身的授权——
 * 那笔改动的授权始终由功能判据与提交信息承担。因此：
 *
 * - **本地未推送的提交不得进本表**。同批后继提交用 `Core-Change-For:` 显式补签
 *   （），不改写历史；混装（功能 + 格式化 / 重生成）先拆提交再提交。
 *   补记不是消红工具。
 * - 允许补记的只有三种：①判据生效前的历史提交；②已推送、改写会伤及他人历史；
 *   ③改动是纯格式 / 纯生成物且已无法与功能批分离（`change` 必须写明「混装带入」
 *   与「当时为何没拆」，不得写成正常授权）。
 * - 补记随提交存亡：所在提交被重写或回滚时，行必须一并删除；它不是永久豁免。
 */
// @gate test

import fs from 'node:fs'
import path from 'node:path'

export const BACKFILL_REL = 'scripts/exemptions/core-change-backfill.json'
export const BACKFILL_JUDGEMENTS = ['CC-001', 'EX-007']

const SHA_RE = /^[0-9a-f]{7,40}$/

/**
 * 补记表的形状判据（纯函数，判据 = `core-change-backfill.test.mjs`）。
 * 返回错误清单；空数组即成立。
 */
export function validateBackfill(table, root) {
  const errors = []
  if (!table || typeof table !== 'object') return ['补记表读不出（不是 JSON 对象）']
  if (!BACKFILL_JUDGEMENTS.includes(String(table.rule ?? ''))) {
    errors.push(`补记表的 rule 必须是 ${BACKFILL_JUDGEMENTS.join(' / ')}`)
  }
  if (typeof table.why !== 'string' || table.why.trim().length < 10) {
    errors.push('补记表缺当前理由 why——表存在的理由不是历史案例编号')
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
    if (typeof e.why !== 'string' || e.why.trim().length < 10) {
      errors.push(`${at}：缺当前理由 why——不写「今天为什么补记」的条目等于一张永久后门`)
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
