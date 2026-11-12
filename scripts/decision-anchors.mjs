/**
 * `decision-anchors` —— 「一条 `D-NNN` 决策到底存不存在」的**唯一**读法。
 *
 * ## 为什么单独一个文件
 *
 * 棘轮类判据（CS-002 / PP-005 / 特殊情形审计的豁免）都要求：**放宽的理由必须挂一条
 * 能打开的决策**。这句话今天有两个可执行含义，合起来才是「决策存在」的完整读法：
 *
 *   1. 历史裁决住在设计文档的「关键决策」节，形态是 `### D-NNN` 锚点；
 *   2. 判定决策 kind 建立之后，裁决正文改由 `docs/008-judgement/` 的**单文档**持有，
 *      身份是 front-matter 的 `id: D-NNN-<主干>`，宿主设计文档只留编号与指路。
 *
 * 只认第一种，新 kind 的裁决在三个消费方眼里都是「指向不存在的决策」——而把
 * `### D-NNN` 补回宿主设计文档会在两处写同一个事实，正是「一个事实两个 owner」。
 * 所以这里读的是**owner**（单文档的 `id`），不是第二处锚点。
 *
 * 它此前在三个脚本里各抄了一遍（扫描根、跳过目录、锚点正则三处都要一致），而抄本
 * 一旦不同，同一条豁免就会在这边绿、那边红。
 *
 * ## 口径（改这里 = 改判据，三处一起改）
 *
 * - 扫描根：`docs/003-design/` + `docs/008-judgement/` + `crates/`（模块就地文档）+
 *   `tauri/docs/`，再加一份**可能不存在**的 CLI 文档面（`cli` 尚未落在这个布局里，
 *   目录不在就静默跳过——与 `doc-link-audit` 的 ROOTS 同一口径）；
 *   调用方给了 `docsRoot` 时**只用它**（回归夹具把文档面指到临时目录）。
 * - 跳过 `node_modules` / `target` / `archive`：那些是历史或产物，不是现行登记。
 * - 锚点形态：`/^#{2,6}\s+(D-\d{3}a?)\b/`——`###` 到 `######` 都算，编号带可选字母尾巴。
 * - 单文档形态：仅当 front-matter 里 `kind: judgement` 时，取 `id: D-(\d{3}a?)(…主干)`；
 *   不认其它 kind 的 `id`，也不认正文里的散写编号。`projection: true` 的 kind 汇总页
 *   （`D-000-…`）是投影的地址，不是裁决，排除——否则「随便挂什么都算挂」又回来了。
 *   此处不看 `status`：与 `### D-NNN` 锚点同口径，草稿够不够格当放宽依据是裁决问题，
 *   不是扫描问题。
 * - 返回的 id **去掉 `D-` 前缀**（`'077'` / `'209a'`），与各审计里 `D-(\d{3}a?)` 捕获组
 *   的取值同形；两边形状不一致会让「存在的决策」被判成不存在。
 *
 * 结果按 `docsRoot` 缓存一次（同一进程里逐条豁免各扫一遍全仓会拖慢门禁）。
 */
import fs from 'node:fs'
import path from 'node:path'

const SKIP_DIRS = new Set(['node_modules', 'target', 'archive'])
const ANCHOR_RE = /^#{2,6}\s+(D-\d{3}a?)\b/
/** 判定决策单文档：kind 与 id 都只在文件开头的 front-matter 里认。 */
const JUDGEMENT_KIND_RE = /^kind:\s*judgement\b/m
const JUDGEMENT_ID_RE = /^id:\s*['"]?D-(\d{3}a?)(?:-|\s|$)/m
/** kind 的汇总页也是 `kind: judgement`，但 `D-000` 是投影的地址、不是一条裁决。 */
const PROJECTION_RE = /^projection:\s*true\b/m

/** 文件开头的 front-matter 内文；没有或未闭合 ⇒ null。 */
function frontMatter(text) {
  if (!text.startsWith('---')) return null
  const end = text.indexOf('\n---', 3)
  return end === -1 ? null : text.slice(3, end)
}

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
      // 判定决策 kind：owner 是这份单文档 front-matter 的 `id`，没有第二处 `###` 锚点。
      const fm = frontMatter(text)
      if (fm && JUDGEMENT_KIND_RE.test(fm) && !PROJECTION_RE.test(fm)) {
        const m = JUDGEMENT_ID_RE.exec(fm)
        if (m) found.add(m[1])
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
          path.join(repoRoot, 'docs', '008-judgement'),
          path.join(repoRoot, 'crates'),
          path.join(repoRoot, 'tauri', 'docs'),
          path.join(repoRoot, 'cli', 'docs'),
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
