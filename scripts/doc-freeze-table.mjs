#!/usr/bin/env node
/**
 * doc-freeze-table — 决策正文证据锚点的冻结表（D-326）
 *
 * 设计文档的 `### D-NNN` 正文记的是**裁决当时**的实现：文件会随迁移批退场、
 * 符号会改名。按现行代码判这些锚点，只会逼人回改决策正文——那等于篡改裁决。
 * 但完全跳过决策段，又会让「证据全假」重新变成无人看见的盲区。
 *
 * 本模块是那条边界唯一机器可读投影：DE-052 的登记表逐行登记旧锚点与它的处置。
 * `doc-link-audit`（D-017）与 `doc-symbol-audit`（D-005）都读同一张表——
 * **决策段里指不到、又不在表里的锚点仍然是红**；表只做「登记时点冻结」，
 * 不给现行文档开口子。
 *
 * 表格式：`| 文档 | \`旧锚点\` | 处置 | 现落点 |`，第二列是登记锚点。
 * 读不出来 ⇒ 空集合（回归夹具的最小仓库没有这张表，此时表不适用）。
 */

import fs from 'node:fs'
import path from 'node:path'

/** 冻结表所在文档（相对仓库根） */
export const FREEZE_DOC_REL = 'docs/003-design/052-决策正文证据锚点冻结表.md'

/**
 * 读出冻结表：`{ anchors, docAbs }`。
 * `anchors` 是 `file.rs::symbol` / `file.rs:line` 形态的旧锚点集合。
 */
export function loadFreezeTable (root) {
  const docAbs = path.resolve(root, FREEZE_DOC_REL)
  const anchors = new Set()
  if (!fs.existsSync(docAbs)) return { anchors, docAbs }
  const lines = fs.readFileSync(docAbs, 'utf8').split(/\r?\n/)
  let inTable = false
  for (const line of lines) {
    if (/^#{2,3}\s/.test(line)) {
      inTable = /登记表/.test(line)
      continue
    }
    if (!inTable || !line.trim().startsWith('|')) continue
    const cells = line.split('|').map((s) => s.trim())
    if (cells.length < 4) continue
    const m = /^`([^`]+)`$/.exec(cells[2])
    if (m) anchors.add(m[1].trim())
  }
  return { anchors, docAbs }
}

/** 冻结表登记的旧锚点集合（只需要判定的调用方用这个）。 */
export function loadFreezeAnchors (root) {
  return loadFreezeTable(root).anchors
}

/**
 * 逐行标记「该行是否落在 `### D-NNN` 决策段内」。
 * 与 `doc-symbol-audit` 判定决策段的规则同形：`###` 起的 D 条目延续到下一个
 * `#` / `##` 标题为止；`####` 子标题留在决策段内（D-318 的施工表就在子标题下）。
 */
export function decisionLineMask (lines) {
  const mask = new Array(lines.length).fill(false)
  let inDecision = false
  for (let i = 0; i < lines.length; i++) {
    if (/^#{2,6}\s+D-\d{3}[a-z]?\b/.test(lines[i])) inDecision = true
    else if (/^#{1,2}\s/.test(lines[i])) inDecision = false
    mask[i] = inDecision
  }
  return mask
}
