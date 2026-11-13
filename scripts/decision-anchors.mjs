/**
 * `decision-anchors` —— 「一条 `D-NNN` 决策到底存不存在」的**唯一**读法。
 *
 * ## 为什么单独一个文件
 *
 * 棘轮类判据（CS-002 / PP-005 / 特殊实例审计的豁免）都要求：**放宽的理由必须挂一条
 * 能打开的决策**。这条要求的可执行含义只有一个：设计文档（或模块 `README.md`）的
 * 「关键决策」节里有一个 `### D-NNN` 标题。裁决的正文就住在那个标题下面——不另立
 * 目录、不另立 kind（[DE-017 §6](../docs/003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")）。
 *
 * 它此前在三个脚本里各抄了一遍（扫描根、跳过目录、锚点正则三处都要一致），而抄本
 * 一旦不同，同一条豁免就会在这边绿、那边红。
 *
 * ## 口径（改这里 = 改判据，三处一起改）
 *
 * - 扫描根：`docs/003-design/` + `crates/`（模块就地文档，含 `crates/symbio-cli/docs/`）
 *   + `tauri/docs/`；调用方给了 `docsRoot` 时**只用它**（回归夹具把文档面指到临时目录）。
 * - 跳过 `node_modules` / `target` / `archive`：那些是历史或产物，不是现行登记。
 * - 锚点形态：`/^#{2,6}\s+(D-\d{3}a?)/`——`###` 到 `######` 都算，编号带可选字母尾巴。
 * - 返回的 id **去掉 `D-` 前缀**（`'077'` / `'209a'`），与各审计里 `D-(\d{3}a?)` 捕获组
 *   的取值同形；两边形状不一致会让「存在的决策」被判成不存在。
 *
 * 结果按 `docsRoot` 缓存一次（同一进程里逐条豁免各扫一遍全仓会拖慢门禁）。
 */
import fs from 'node:fs'
import path from 'node:path'

const SKIP_DIRS = new Set(['node_modules', 'target', 'archive'])
const ANCHOR_RE = /^#{2,6}\s+(D-\d{3}a?)\b/

const cache = new Map()

/** 一次全量扫描；只在缓存未命中时调用。 */
function scan(roots) {
  const found = new Set()
  const walk = (dir) => {
    let list
    try {
      list = fs.readdirSync(dir, { withFileTypes: true })
    } catch {
      return
    }
    for (const ent of list) {
      const abs = path.join(dir, ent.name)
      if (ent.isDirectory()) {
        if (SKIP_DIRS.has(ent.name)) continue
        walk(abs)
        continue
      }
      if (!ent.name.endsWith('.md')) continue
      let text = ''
      try {
        text = fs.readFileSync(abs, 'utf8')
      } catch {
        continue
      }
      for (const line of text.split('\n')) {
        const m = ANCHOR_RE.exec(line)
        if (m) found.add(m[1].slice(2))
      }
    }
  }
  for (const r of roots) walk(r)
  return found
}

/**
 * @param {string} repoRoot 仓库根（决定默认扫描的四个目录）
 * @param {string|null} [docsRoot] 覆盖扫描面：只扫这一棵（回归夹具用）
 * @returns {Set<string>} 存在决策 id 的集合（不含 `D-` 前缀）
 */
export function decisionAnchorIds(repoRoot, docsRoot = null) {
  const key = `${repoRoot}::${docsRoot ?? ''}`
  let ids = cache.get(key)
  if (!ids) {
    const roots = docsRoot
      ? [docsRoot]
      : [
          path.join(repoRoot, 'docs', '003-design'),
          path.join(repoRoot, 'crates'),
          path.join(repoRoot, 'tauri', 'docs'),
        ]
    ids = scan(roots)
    cache.set(key, ids)
  }
  return ids
}

/** `D-NNN` 是否存在（`id` 用不带 `D-` 的数字段，如 `'221'`）。 */
export function hasDecisionAnchor(id, repoRoot, docsRoot = null) {
  return decisionAnchorIds(repoRoot, docsRoot).has(id)
}
