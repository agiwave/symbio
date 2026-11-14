/**
 * fs-walk — 目录遍历与「这个文件算不算测试面」的两条共用判据。
 *
 * ## 为什么单独立一个文件
 *
 * 这两条判据此前住在 `core-surface.mjs` 里——那个库里 90% 的代码是「数公开面消费方」
 * 的分析器，只有 `walk` 与 `isTestFile` 是别人也在用的。D-279 删掉旧 core 之后，
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
// @gate lib

import fs from 'node:fs'
import os from 'node:os'
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

/**
 * 带过滤的遍历：**各审计此前各抄一遍的那个循环**（20 处）。
 *
 * 口径（每条都对应一份存量实现的语义，不是新发明）：
 *
 * | 选项 | 含义 |
 * |---|---|
 * | `exts` | 后缀数组（`['.rs']`）；命中任一即收 |
 * | `keep(abs, name)` | 自定义谓词，**优先于** `exts`（给了它就不再单独判后缀） |
 * | `skipDirs` | 目录名集合：整棵跳过 |
 * | `skipName` | 文件名正则：命中即收不进来 |
 * | `skipHidden` | 跳过 `.` 开头的文件与目录 |
 * | `rel` | 给出基准 ⇒ 返回相对它的 posix 路径，而不是绝对路径 |
 * | `tolerant` | 目录读不出时返回已收集的部分而不是抛错 |
 *
 * `tolerant` **默认关**，与 [`walk`] 相反：判据的扫描根读不出时，「零份文件」会让
 * 审计安安静静地全绿，所以默认把「根不存在」当输入缺席、由调用方给非 0 退出；
 * 只有「目录可能合法地不存在」（夹具 / 换根的回归）才显式打开它。
 *
 * 遍历顺序与逐层 `readdirSync` 一致，**不排序**：审计输出的行序不依赖文件顺序，
 * 而排序会让逐份比对存量输出时出现假的差异。
 */
export function walkFiles(dir, opts = {}) {
  const {
    exts = null,
    keep = null,
    skipDirs = null,
    skipName = null,
    skipHidden = false,
    rel = null,
    tolerant = false,
  } = opts
  const out = []
  const relOf = (base, abs) => path.relative(base, abs).split(path.sep).join('/')
  const visit = (abs) => {
    let entries
    try {
      entries = fs.readdirSync(abs, { withFileTypes: true })
    } catch (err) {
      if (tolerant) return
      throw err
    }
    for (const e of entries) {
      if (skipHidden && e.name.startsWith('.')) continue
      const child = path.join(abs, e.name)
      if (e.isDirectory()) {
        if (skipDirs && skipDirs.has(e.name)) continue
        visit(child)
        continue
      }
      if (!e.isFile()) continue
      if (skipName && skipName.test(e.name)) continue
      if (keep ? keep(child, e.name) : !exts || exts.some((x) => e.name.endsWith(x))) {
        out.push(rel ? relOf(rel, child) : child)
      }
    }
  }
  visit(dir)
  return out
}

/**
 * 逐个删掉临时夹具目录——**不用** `fs.rmSync({ recursive: true })`：沙箱会拦截批量删除
 * （见 `test-layout-audit.test.mjs`）。20 份审计回归测试原先各抄一份，现统一归这里。
 */
export function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

/**
 * 把「相对路径 → 内容」写进一棵树。值为 `null` 表示只建目录，不写文件。
 * 审计回归测试里这段循环原先各抄一份，现归这里（`withTempTree` 也走它）。
 */
export function writeTree(root, files) {
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    if (content === null) fs.mkdirSync(abs, { recursive: true })
    else fs.writeFileSync(abs, content)
  }
}

/**
 * 临时仓夹具运行时：**造树 → 跑回调 → 无条件清理**。
 *
 * 审计回归测试此前各写一份 `mkdtempSync` + 逐文件 `mkdirSync/writeFileSync` +
 * `try/finally` 清理（有的还在每个用例里各写一遍）。回调返回值原样透出、异常原样抛，
 * `finally` 只负责把树删干净；清理统一走 `rmTree`（`fs.rmSync` 的批量删除会被沙箱拦）。
 *
 * @param {Record<string, string|null>} files 相对路径 → 内容（`null` = 只建目录）
 * @param {(root: string) => unknown} fn 在临时树里跑的回调
 * @param {{ prefix?: string, dirs?: string[] }} [opts] `prefix` 给 mkdtemp 命名；
 *   `dirs` 是额外要建的空目录（夹具里「只有目录没有文件」的那类形状）
 */
export function withTempTree(files, fn, { prefix = 'audit-', dirs = [] } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), prefix))
  try {
    for (const d of dirs) fs.mkdirSync(path.join(root, d), { recursive: true })
    writeTree(root, files)
    return fn(root)
  } finally {
    rmTree(root)
  }
}
