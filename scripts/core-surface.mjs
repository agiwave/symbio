/**
 * core-surface — `symbio_core` **公开面**的枚举（共享模块，不直接执行）
 *
 * 被三个审计脚本使用：
 *   - `core-surface-audit.mjs`（**报告型**）：数每个符号的消费方数量（ADR-023 判据）
 *   - `core-naming-audit.mjs`（**判定型**）：检查每个符号的前缀是否落在所属域登记的前缀里
 *   - `core-export-audit.mjs`（**判定型**）：判「根显式出口」与「≥2 个模块消费」
 *
 * 两者必须对「公开面是什么」给出**同一份**答案。各写一份解析必然各自演化，最后变成
 * 两套口径——这与本项目 `schema-audit` 曾出现的「报告型与判定型判据不一致」是同一类
 * 失效（报告说 A、判定说 B，读报告的人据此改坏健康的东西）。故解析收在这一处。
 *
 * ## 公开面的口径（四条，都影响结果，别凭直觉）
 *
 * 1. **公开面 = 根 `mod.rs` 的重导出**，不是「域目录下所有 `pub`」：私有子模块里的
 *    `pub` 从 crate 外够不着（`TOOL_NAME_WIRE_SEPARATOR` 就是这种），把域内所有
 *    `pub` 都算进来会凭空多出一批「零消费方」。
 * 2. **通配重导出必须逐个展开**：根 `mod.rs` 有 `pub use plugin::*` / `pub use keys::*`
 *    / `pub use logger::*` **三条**通配，用 `map.set('*', …)` 会让后一条覆盖前一条——
 *    于是 `PLUGIN_*`、`PathKey`、`PluginStopReason` 全部凭空消失（实测少算 85 个符号）。
 *    故 `parsePubUses` 返回 `{ names, globs }`，`globs` 是**数组**。
 * 3. **宏生成的公开符号扫不到 `pub` 关键字**：`define_string_key!(PathKey, PATH, "path")`
 *    展开出的类型与常量都不带 `pub`，`^pub\s+(struct|…)` 完全看不见它们——而它们恰是
 *    **消费方最多**的一批（`PATH` 有 17 个消费方）。故按宏的形参位置取。
 * 4. **`symbio/src/` 直属文件（`lib.rs` / `plugins/mod.rs` 这类不在插件子目录里的）
 *    也是消费方**：第一版把它们跳过，于是「只在注册表里被用到」的符号被算成 0——
 *    `PluginErrorCode` / `PluginIdentity` 一族全部假报。**「数不到」与「真的没人用」
 *    是两件事**，归属函数因此**永不返回 null**（见 `core-surface-audit.mjs` 的 `unitOf`）。
 */

import fs from 'node:fs'
import path from 'node:path'

/** 递归遍历目录（目录不存在 ⇒ 空迭代，不抛错：夹具仓库里可能没有某个子树） */
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

/** 去掉块注释与行注释（文档注释里提到一个符号不代表依赖它） */
export function stripComments(src) {
  return src.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')
}

/**
 * 从 `pub use …;` 语句里抽名字。
 *
 * 返回 `{ names: Map<名字, 域|null>, globs: string[] }`。`globs` 必须是**数组**——
 * 见文件头口径 2。
 */
export function parsePubUses(source, domains) {
  const names = new Map()
  const globs = []
  for (const m of stripComments(source).matchAll(/^[ \t]*pub use\s+([^;]+);/gm)) {
    const body = m[1].trim()
    const head = body.split('::')[0].trim()
    const domain = domains.has(head) ? head : null
    const brace = body.match(/\{([\s\S]*)\}/)
    if (brace) {
      for (const part of brace[1].split(',')) {
        const name = part.trim().split(/\s+as\s+/).pop().trim()
        if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) names.set(name, domain)
      }
      continue
    }
    if (/::\s*\*\s*$/.test(body)) {
      globs.push(domain ?? head)
      continue
    }
    const name = body.split('::').pop().trim()
    if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) names.set(name, domain)
  }
  return { names, globs }
}

/**
 * 直接声明在某个 `.rs` 文件里的 `pub` 符号集合。
 *
 * 含宏生成的键（`define_string_key!` 的形参位）——见文件头口径 3。
 */
export function declaredIn(file) {
  const out = new Set()
  if (!fs.existsSync(file)) return out
  const src = stripComments(fs.readFileSync(file, 'utf8'))
  for (const m of src.matchAll(
    /^pub\s+(?:struct|enum|trait|union|type|const|static|fn)\s+([A-Za-z_][A-Za-z0-9_]*)/gm,
  )) {
    out.add(m[1])
  }
  // `define_string_key!(PathKey, PATH, "path")` ⇒ 键类型 + 键常量。
  // 换宏就得跟着改这里——这是本模块唯一的宏形态假设。
  for (const m of src.matchAll(
    /^[ \t]*define_string_key!\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*,\s*([A-Za-z_][A-Za-z0-9_]*)/gm,
  )) {
    out.add(m[1])
    out.add(m[2])
  }
  return out
}

/**
 * 直接声明在某个 `.rs` 文件里的**子模块**名（`mod X;` / `pub mod X {`）。
 *
 * 为什么要单独识别模块：`pub use capability::failure_kind;` 是**模块**重导出，
 * 而模块名是小写 snake_case——`kindOf` 会把它当成**函数**。`failure_kind` 就是这样
 * 被误报成「函数缺域前缀」的（实为子命名空间，见 `core-naming-audit.mjs` 的口径）。
 * 「名字形态」推断对**符号**成立，对**命名空间**不成立——命名空间不是符号。
 */
export function declaredModules(file) {
  const out = new Set()
  if (!fs.existsSync(file)) return out
  const src = stripComments(fs.readFileSync(file, 'utf8'))
  for (const m of src.matchAll(
    /^[ \t]*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*[;{]/gm,
  )) {
    out.add(m[1])
  }
  return out
}

/**
 * 按 Rust 命名约定推断符号种类。
 *
 * 为什么用形态而不是解析 `pub struct` / `pub const`：重导出链（`plugin/mod.rs` 的
 * `pub use ids::{…}`）里的名字在**声明处**才带关键字，追定义要一路解析子模块；而
 * Rust 的命名约定是强约束（类型 PascalCase、常量 SCREAMING_SNAKE、函数 snake_case，
 * 违反会被 `non_camel_case_types` / `non_snake_case` 警告），形态推断在这里足够可靠，
 * 且**不依赖解析深度**。
 *
 * 判据：全大写（可含数字与下划线）⇒ 常量；首字母大写 ⇒ 类型；其余 ⇒ 函数。
 */
export function kindOf(name) {
  if (/^[A-Z][A-Z0-9_]*$/.test(name)) return 'const'
  if (/^[A-Z]/.test(name)) return 'type'
  return 'fn'
}

/**
 * 枚举 `symbio_core` 的公开面。
 *
 * 返回 `{ coreRel, coreDir, domains, symbols, modules }`：
 *   - `domains`：域目录名（`Set`）
 *   - `symbols`：`Map<符号名, 域>`；`'(root)'` = 直接声明在根 `mod.rs`，
 *     `'(explicit)'` = 根显式列名重导出且未标出域
 *   - `modules`：`symbols` 里其实是**子模块**（命名空间）的那些名字。判定型守卫
 *     必须跳过它们——命名空间不是符号，前缀规则对它不适用（见 `declaredModules`）
 */
export function collectCoreSurface(root) {
  const coreRel = 'symbio/src/symbio_core'
  const coreDir = path.join(root, coreRel)
  const rootMod = path.join(coreDir, 'mod.rs')
  if (!fs.existsSync(rootMod)) {
    throw new Error(`找不到 ${path.join(coreRel, 'mod.rs')}（root=${root}）`)
  }

  const domains = new Set(
    fs
      .readdirSync(coreDir, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .map((e) => e.name),
  )

  const rootUses = parsePubUses(fs.readFileSync(rootMod, 'utf8'), domains)
  const symbols = new Map()
  for (const [name, domain] of rootUses.names) symbols.set(name, domain ?? '(explicit)')
  for (const n of declaredIn(rootMod)) symbols.set(n, '(root)')

  // 展开通配：`pub use <域>::*` ⇒ 该域 mod.rs 的重导出 + 直接声明（不含私有子模块里的 pub）
  for (const domain of rootUses.globs) {
    const domMod = path.join(coreDir, domain, 'mod.rs')
    if (!fs.existsSync(domMod)) continue
    const dom = parsePubUses(fs.readFileSync(domMod, 'utf8'), domains)
    for (const [n] of dom.names) if (!symbols.has(n)) symbols.set(n, domain)
    for (const n of declaredIn(domMod)) symbols.set(n, domain)
  }

  // 子模块：域 `mod.rs` 里 `mod X;` / `pub mod X {` 声明过的名字。只对**能定位到域**
  // 的符号判——`(root)` / `(explicit)` 没有域，也就没有子模块可查。
  const modCache = new Map()
  const modules = new Set()
  for (const [name, domain] of symbols) {
    if (domain === '(root)' || domain === '(explicit)') continue
    if (!modCache.has(domain)) {
      modCache.set(domain, declaredModules(path.join(coreDir, domain, 'mod.rs')))
    }
    if (modCache.get(domain).has(name)) modules.add(name)
  }

  return { coreRel, coreDir, domains, symbols, modules }
}

// ==================== 消费方计数（报告型与判定型共用） ====================
//
// 从 `core-surface-audit.mjs` 提上来：`core-export-audit.mjs`（判定型）要判
// 「≥2 个模块消费」，判据必须与报告型**同一份**，否则又是「报告说 A、判定说 B」。
// 口径全文见文件头，此处照搬原实现，不动语义。

/** 跨 crate 消费方：整个 crate 算一个单位（它们是独立 crate，与插件模块不可比） */
export const CROSS_CRATE = [
  { rel: 'cli/src', label: 'cli' },
  { rel: 'tauri/src-tauri/src', label: 'tauri-shell' },
]

/**
 * 文件 → 消费方单位。**永远返回一个单位，绝不返回 null**（口径 4）。
 *
 * 返回 `null` 会让调用方 `continue` 跳过该文件，于是定义在那里的符号被算成
 * 「0 个消费方」——`PluginErrorCode` / `PluginIdentity` 一族就这样被假报过。
 */
export function unitOf(rel) {
  const norm = rel.split(path.sep).join('/')
  const m = norm.match(/^symbio\/src\/plugins\/([^/]+)\//)
  if (m) return `plugins/${m[1]}`
  const m2 = norm.match(/^symbio\/src\/providers\/([^/]+)\//)
  if (m2) return `providers/${m2[1]}`
  if (/^symbio\/src\/plugins\/[^/]+$/.test(norm)) return 'plugins/(registry)'
  if (norm.startsWith('symbio/src/')) return 'symbio/(crate root)'
  for (const c of CROSS_CRATE) {
    if (norm.startsWith(c.rel)) return c.label
  }
  return null
}

/**
 * 自引用：插件 id 常量（`PLUGIN_ID_<X>`）与嵌入服务 id（`EMBEDDING_<X>`）被
 * **同名插件 / provider** 使用。这是**正常**的——常量就是它自己的名字，不该算
 * 「下放候选」。
 */
export function isSelfReference({ name, units }) {
  const m = name.match(/^(?:PLUGIN_ID|EMBEDDING)_([A-Z]+)$/)
  if (!m || units.length !== 1) return false
  const slug = m[1].toLowerCase()
  return units[0] === `plugins/${slug}` || units[0] === `providers/${slug}`
}

/**
 * core 内 `macro_rules!` 宏体引用到的公开符号（`宏名 → Set<符号>`）。
 *
 * **为什么必须认这层**：宏体在 core 内书写，却在 core **外**的调用点展开——
 * `plugin_error!` 之类的名字只落在 core 的宏定义文件里，按文本找消费方永远是 0。
 * 不认这层，这些符号会被判成「0 消费方 → 收窄根导出」，然后宏体的根路径
 * `$crate::symbio_core::<符号>` 在插件调用点当场 E0603（`mod` 是私有的）。
 * 判定型审计与报告型**共用本口径**，见文件头。
 */
export function collectMacroBodies(root, coreRel, symbols) {
  const names = [...symbols.keys()]
  const macros = new Map()
  for (const file of walk(path.join(root, coreRel))) {
    if (!file.endsWith('.rs')) continue
    const src = stripComments(fs.readFileSync(file, 'utf8'))
    for (const m of src.matchAll(/macro_rules!\s*([A-Za-z_]\w*)\s*\{/g)) {
      let depth = 1
      let j = m.index + m[0].length
      for (; j < src.length && depth > 0; j++) {
        if (src[j] === '{') depth++
        else if (src[j] === '}') depth--
      }
      const body = src.slice(m.index + m[0].length, j - 1)
      const hit = names.filter((n) => new RegExp(`\\b${n}\\b`).test(body))
      if (!hit.length) continue
      if (!macros.has(m[1])) macros.set(m[1], new Set())
      for (const n of hit) macros.get(m[1]).add(n)
    }
  }
  return macros
}

/**
 * 数每个公开符号的消费方单位（`Map<符号, Set<单位>>`）。
 *
 * - core 自身不算消费方（口径：公开面是给 core **外**用的）；
 * - 一个文件都不能跳过（口径 4）：`symbio/src/` 直属文件也算；
 * - 宏调用点算消费方（见 `collectMacroBodies`）。
 */
export function collectConsumers(root, symbols, coreRel = 'symbio/src/symbio_core') {
  const allNames = [...symbols.keys()]
  const macros = collectMacroBodies(root, coreRel, symbols)
  const consumers = new Map()
  const scanRoots = ['symbio/src', 'cli/src', 'tauri/src-tauri/src'].map((r) => path.join(root, r))
  for (const scanRoot of scanRoots) {
    for (const file of walk(scanRoot)) {
      if (!file.endsWith('.rs')) continue
      const rel = path.relative(root, file)
      if (rel.split(path.sep).join('/').startsWith(coreRel + '/')) continue // core 自身不算消费方
      const unit = unitOf(rel)
      if (!unit) continue
      const src = stripComments(fs.readFileSync(file, 'utf8'))
      const add = (name) => {
        if (!consumers.has(name)) consumers.set(name, new Set())
        consumers.get(name).add(unit)
      }
      for (const name of allNames) {
        if (!new RegExp(`\\b${name}\\b`).test(src)) continue
        add(name)
      }
      // 宏调用点：`plugin_error!(…)` 在这里展开 ⇒ 这里是真消费方（名字不落地）
      for (const m of src.matchAll(/\b([A-Za-z_]\w*)\s*!/g)) {
        const hit = macros.get(m[1])
        if (!hit) continue
        for (const name of hit) add(name)
      }
    }
  }
  return consumers
}
