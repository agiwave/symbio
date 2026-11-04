/**
 * 决策号存在性（`D-NNN` 是否被某篇现行文档点名过）。
 *
 * 从退役的 `known-red.mjs` 中抽出：`known-red` 台账随 `LP-006` 一起退役，但
 * 「放宽 / 豁免引用的决策必须真实存在」这条判据仍由 `control-plane-audit` 与
 * `baseline-scope-audit` 共用。只读，不改 docs。
 */

import fs from 'node:fs'
import path from 'node:path'

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
