/**
 * fs-walk — 目录遍历与「这个文件算不算测试面」的两条共用判据。
 *
 * ## 为什么单独立一个文件
 *
 * 这两条判据此前住在 `core-surface.mjs` 里——那个库里 90% 的代码是「数公开面消费方」
 * 的分析器，只有 `walk` 与 `isTestFile` 是别人也在用的。D-279 删掉 `symbio-core` 之后，
 * `core-export-audit` / `core-surface-audit` 随之退役，那个库的分析器再没有调用方；
 * 把两个辅助函数留下来单独成文件，比「让一个判据背着 600 行死代码」干净。
 *
 * ## 失效形态（它是判据的**输入前提**，错了下游全错）
 *
 * ① 遍历读不出目录时**静默返回空**——下游会把它读成「0 处违规」（D-280 记的空转绿形态）。
 *    所以这里只负责遍历，**范围存在性由调用方自己判**并给非 0 退出；
 * ② 把 `#[cfg(test)] mod test_support;` 当成生产模块——那一批类型会被数成生产消费方，
 *    于是「只有一个消费方」的下放判据永远不命中。
 */

import fs from 'node:fs'
import path from 'node:path'

/** 深度优先遍历目录下的所有文件。目录读不出 ⇒ 返回空（调用方负责判范围存在性）。 */
export function* walk(dir) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of entries) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) yield* walk(p)
    else yield p
  }
}

/**
 * 测试文件（命名约定见 `test-layout-audit.mjs`：`X.test.rs` 与 `tests.rs`），
 * 外加**只在测试构建里存在的模块**（`#[cfg(test)] mod test_support;`）。
 *
 * **测试不是架构消费方**——这条判据只有这一处实现，多个审计共用。
 */
export function isTestFile(file) {
  if (file.endsWith('.test.rs') || path.basename(file) === 'tests.rs') return true
  return isCfgTestOnlyModule(file)
}

const PARENT_MODULE_CACHE = new Map()

/**
 * `#[cfg(test)] mod test_support;` 这类模块：文件名不是 `.test.rs`，但它**不参与
 * 生产构建**。旧口径只看文件名，于是把其中的类型读成生产消费方。
 *
 * 判据取自声明方：同名 `mod` 声明紧前（允许空行与单行注释）必须是 `#[cfg(test)]`。
 */
function isCfgTestOnlyModule(file) {
  const stem = path.basename(file, '.rs')
  const dir = path.dirname(file)
  let parentText = PARENT_MODULE_CACHE.get(dir)
  if (parentText === undefined) {
    parentText = null
    for (const candidate of [path.join(dir, 'mod.rs'), `${dir}.rs`]) {
      try {
        parentText = fs.readFileSync(candidate, 'utf8')
        break
      } catch {}
    }
    PARENT_MODULE_CACHE.set(dir, parentText)
  }
  if (!parentText) return false
  const lines = parentText.split('\n')
  const decl = new RegExp(`^\\s*(?:pub(?:\\([^)]*\\))?\\s+)?mod\\s+${stem}\\s*;`)
  for (let i = 0; i < lines.length; i++) {
    if (!decl.test(lines[i])) continue
    for (let j = i - 1; j >= 0 && j >= i - 4; j--) {
      const line = lines[j].trim()
      if (!line || line.startsWith('//')) continue
      return line === '#[cfg(test)]'
    }
    return false
  }
  return false
}
