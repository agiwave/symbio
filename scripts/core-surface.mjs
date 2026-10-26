/**
 * core-surface — `symbio_core` **公开面**的枚举（共享模块，不直接执行）
 *
 * 被三个审计脚本使用：
 *   - `core-surface-audit.mjs`（**报告型**）：数每个符号的消费方数量（D-023 判据）
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
 * 4. **测试文件不是消费方**（`X.test.rs` 与 `tests.rs`，约定见 `test-layout-audit.mjs`）：
 *    消费方数要回答的是「**谁依赖它**」，而测试依赖的是**被测对象**，不是它要长期持有的
 *    接口。把测试算进来会让「一个模块在生产用、另一个模块的测试顺手写了一次名字」读成
 *    「两个模块依赖」——`TurnRunner` 就是这样长期显为 2 消费方（生产只有 `plugins/session`，
 *    第二个单位来自 `plugins/model/bound_provider.test.rs` 的彩排），而同族的
 *    `TurnOutcome` / `TurnResume` 因为没被那个测试提到，一直显为单消费方。**同一个功能、
 *    四个名字，两种判决**——差别只在测试里恰好写了哪几个名字，那与「该不该住 core」无关。
 *    ⚠️ 本条是 2026-10-09 补的**口径修正**：此前测试算消费方，棘轮基线（单消费方 61）按
 *    那个口径算；口径一改，基线随之一并重定（见 `core-export-audit.mjs` 的 `BASELINE`）。
 * 5. **`crates/` 直属文件（`lib.rs` / `plugins/mod.rs` 这类不在插件子目录里的）
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
 * 返回 `{ names: Map<名字, 域|null>, globs: string[], heads: Map<名字, 首段> }`。
 * `globs` 必须是**数组**——见文件头口径 2。
 *
 * `heads` 是给**门面**用的（口径 6）：`pub use kernel::{A, B}` 里 `kernel` 不是域目录，
 * 这两条 `use` 自己说不出归属，得回到 `kernel.rs` 里看它们从哪个域转出来。
 */
export function parsePubUses(source, domains) {
  const names = new Map()
  const heads = new Map()
  const globs = []
  for (const m of stripComments(source).matchAll(/^[ \t]*pub use\s+([^;]+);/gm)) {
    const body = m[1].trim()
    const segs = body.split('::').map((s) => s.trim())
    const head = segs[0]
    // 域取**路径里最后一个域段**：`super::<域>::`（门面模块的写法）与
    // `symbio_kernel::<域>::`（域实现搬到内核 crate 后的写法）都要认，
    // 只看首段会让后一种全部落进 `(explicit)`。
    let domain = null
    for (const s of segs.slice(0, -1)) {
      const clean = s.replace(/[{}].*$/, '').trim()
      if (domains.has(clean)) domain = clean
    }
    if (domain === null && domains.has(head)) domain = head
    const brace = body.match(/\{([\s\S]*)\}/)
    if (brace) {
      for (const part of brace[1].split(',')) {
        const name = part.trim().split(/\s+as\s+/).pop().trim()
        if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
          names.set(name, domain)
          heads.set(name, head)
        }
      }
      continue
    }
    if (/::\s*\*\s*$/.test(body)) {
      globs.push(domain ?? head)
      continue
    }
    const name = body.split('::').pop().trim()
    if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
      names.set(name, domain)
      heads.set(name, head)
    }
  }
  return { names, globs, heads }
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
  const coreRel = 'crates/symbio-core/src'
  const coreDir = path.join(root, coreRel)
  const rootMod = path.join(coreDir, 'lib.rs')
  if (!fs.existsSync(rootMod)) {
    throw new Error(`找不到 ${path.join(coreRel, 'lib.rs')}（root=${root}）`)
  }

  const domains = new Set(
    fs
      .readdirSync(coreDir, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .map((e) => e.name),
  )

  const rootUses = parsePubUses(fs.readFileSync(rootMod, 'utf8'), domains)
  const symbols = new Map()
  const kernelExports = new Set()

  // 门面单文件模块（`kernel.rs`）的**显式列名**重导出：`pub use kernel::{A, B}` 里
  // `kernel` 不是域目录，这条 `use` 自己说不出 A/B 属于哪个域。回门面里现查——
  // 门面只转出、不拥有声明，`pub use super::decl::{…}` 就是它们的真实归属。
  //
  // 不这么做的后果不是「少算」，而是**算错**：这些名字落进 `(explicit)`，
  // 命名审计读成「没有归属域」（N-005），依赖分析读成「无 owner」。
  // 拆 crate（D-191）把 `pub(crate) use` 升成 `pub use` 后，这类名字一次多了 11 个，
  // 全是门面这条路径本来就该解析、但只对通配实现了的。
  const facadeCache = new Map()
  const facadeUses = (head) => {
    if (facadeCache.has(head)) return facadeCache.get(head)
    const f = path.join(coreDir, `${head}.rs`)
    const u = fs.existsSync(f) ? parsePubUses(fs.readFileSync(f, 'utf8'), domains) : null
    facadeCache.set(head, u)
    return u
  }
  for (const [name, domain] of rootUses.names) {
    if (domain) {
      symbols.set(name, domain)
      continue
    }
    const head = rootUses.heads.get(name)
    const fu = head ? facadeUses(head) : null
    const inherited = fu?.names.get(name) ?? null
    if (inherited) {
      symbols.set(name, inherited)
      if (head === 'kernel') kernelExports.add(name)
      continue
    }
    symbols.set(name, '(explicit)')
  }
  for (const n of declaredIn(rootMod)) symbols.set(n, '(root)')

  // 展开通配：`pub use <域>::*` ⇒ 该域 mod.rs 的重导出 + 直接声明（不含私有子模块里的 pub）。
  // 门面模块也可以写成单文件 `<域>.rs`（`kernel.rs` 就是这种），两种形态都认。
  // 名字的域以**门面自己 `pub use` 的来源域**为准（`d`）——门面不是域，不能把符号
  // 的归属改成门面名，否则命名审计/依赖图会丢掉真实 owner。
  for (const domain of rootUses.globs) {
    const dirMod = path.join(coreDir, domain, 'mod.rs')
    const domMod = fs.existsSync(dirMod) ? dirMod : path.join(coreDir, `${domain}.rs`)
    if (!fs.existsSync(domMod)) continue
    const dom = parsePubUses(fs.readFileSync(domMod, 'utf8'), domains)
    for (const [n, d] of dom.names) {
      if (!symbols.has(n)) symbols.set(n, d ?? domain)
      if (domain === 'kernel') kernelExports.add(n) // 封闭语法的公开面（C-003 的唯一判定范围）
    }
    for (const n of declaredIn(domMod)) symbols.set(n, domain)
  }

  // 子模块：域 `mod.rs` 里 `mod X;` / `pub mod X {` 声明过的名字。只对**能定位到域**
  // 的符号判——`(root)` / `(explicit)` 没有域，也就没有子模块可查。
  //
  // 域实现搬进 `symbio-kernel` 后（D-188），core 的域 `mod.rs` 只剩 `pub use
  // symbio_kernel::<域>::*;`，`pub mod family;` / `pub mod failure_kind { … }` 的
  // 声明落在 kernel 侧。只查 core 会把这两个子命名空间读成函数（N-001 假红），
  // 故两处取并集——与「公开面同口径」是同一条纪律：公开面在哪，判定就在哪。
  const kernelDir = path.join(root, 'crates', 'symbio-kernel', 'src')
  const modCache = new Map()
  const modules = new Set()
  for (const [name, domain] of symbols) {
    if (domain === '(root)' || domain === '(explicit)') continue
    if (!modCache.has(domain)) {
      const found = declaredModules(path.join(coreDir, domain, 'mod.rs'))
      for (const n of declaredModules(path.join(kernelDir, domain, 'mod.rs'))) found.add(n)
      modCache.set(domain, found)
    }
    if (modCache.get(domain).has(name)) modules.add(name)
  }

  return { coreRel, coreDir, domains, symbols, modules, kernelExports }
}

// ==================== 消费方计数（报告型与判定型共用） ====================
//
// 从 `core-surface-audit.mjs` 提上来：`core-export-audit.mjs`（判定型）要判
// 「≥2 个模块消费」，判据必须与报告型**同一份**，否则又是「报告说 A、判定说 B」。
// 口径全文见文件头，此处照搬原实现，不动语义。

/** 跨 crate 消费方：整个 crate 算一个单位（它们是独立 crate，与插件模块不可比） */
export const CROSS_CRATE = [
  { rel: 'crates/symbio-cli/src', label: 'cli' },
  { rel: 'tauri/src-tauri/src', label: 'tauri-shell' },
]

/**
 * 文件 → 消费方单位。**永远返回一个单位，绝不返回 null**（口径 4）。
 *
 * 返回 `null` 会让调用方 `continue` 跳过该文件，于是定义在那里的符号被算成
 * 「0 个消费方」——`PluginErrorCode` / `PluginIdentity` 一族就这样被假报过。
 *
 * ## `crates/` 那一格的单位是**crate 名**，不是「crates 底下算一个」（2026-10-26 D-191）
 *
 * 拆 crate 之前这一格是 `if (norm.startsWith('crates/')) return 'symbio/(crate root)'`
 * ——那时 `crates/` 只装得下一个壳，兜底成单一是对的。D-191 之后每个插件 / provider 都是
 * 独立 crate，同一条兜底会把**三十几个 crate 塌成同一个单位**：C-003 的判据是
 * 「跨 core 的生产消费方 ≥2」，而两个插件现在都被读成 `symbio/(crate root)` ⇒
 * 任何符号的消费方数**结构上永远 ≤1**，判据于是要么恒红要么被基线掩盖。
 * 实测夹具（两个插件消费同一符号）因此被判「单消费方 1 个」，是真误判。
 *
 * 单位取 crate 名而不是 `plugins/<id>`：D-191 把插件目录平铺成 `crates/symbio-plugin-<id>`，
 * 「哪个插件在用」这一层信息由 crate 名承载，再合成 `plugins/<id>` 只会多一层易漂移的映射。
 */
export function unitOf(rel) {
  const norm = rel.split(path.sep).join('/')
  for (const c of CROSS_CRATE) {
    if (norm.startsWith(c.rel)) return c.label
  }
  const m0 = norm.match(/^crates\/([^/]+)\//)
  if (m0) return m0[1]
  const m = norm.match(/^symbio\/src\/plugins\/([^/]+)\//)
  if (m) return `plugins/${m[1]}`
  const m2 = norm.match(/^symbio\/src\/providers\/([^/]+)\//)
  if (m2) return `providers/${m2[1]}`
  if (/^symbio\/src\/plugins\/[^/]+$/.test(norm)) return 'plugins/(registry)'
  return null
}

/**
 * 自引用：插件 id 常量（`PLUGIN_ID_<X>`）与嵌入服务 id（`EMBEDDING_<X>`）被
 * **同名插件 / provider** 使用。这是**正常**的——常量就是它自己的名字，不该算
 * 「下放候选」。
 *
 * 单位形态随 D-191 变过一次：插件消费方的单位从 `plugins/<slug>` 变成 crate 名
 * `symbio-plugin-<slug>`，两种都认（旧形态在回归测试的夹具里仍然在用）。
 */
export function isSelfReference({ name, units }) {
  const m = name.match(/^(?:PLUGIN_ID|EMBEDDING)_([A-Z]+)$/)
  if (!m || units.length !== 1) return false
  const slug = m[1].toLowerCase()
  return (
    units[0] === `plugins/${slug}` ||
    units[0] === `providers/${slug}` ||
    units[0] === `symbio-plugin-${slug}` ||
    units[0] === `symbio-provider-${slug}`
  )
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
 * 符号 → **core 内“另一个模块”** 也在用它（`Map<符号, Set<core 模块>>`）。
 *
 * ## 为什么要单独数这一层（2026-10-11，C-003 的第二口径）
 *
 * `collectConsumers` 把 core 自身整个排除（「公开面是给 core 外用的」）。那条口径回答
 * 的是 **zero 那一半**——「它够不够格占根出口」（core 内自用而对外无人用的符号不该出根）。
 * 它**不**适合回答 **one 那一半**——「该不该下沉回那个唯一消费方」。
 *
 * 后果在豁免账本里看得最清楚：账本上大半条目的理由逐字写着
 * 「core 内另有消费方（transcript / prompt_fidelity），审计只数跨出 core 的那一个」——
 * 即**豁免是在补口径的盲区**，不是在记录符号的属性。本函数把盲区填上：
 * 一个符号若**另有一个 core 模块**在用，那把它下沉到那唯一的外部消费方（一个插件）
 * 就编译不过（core 不得反向依赖插件），它**本来就**是 ≥2 个模块共享的机制。
 *
 * ## 口径（两条，都影响结果）
 *
 * 1. **核心模块 = 域目录的第一段**（`projection/transcript.rs` ⇒ `projection`）；
 *    直接住在 core 根下的文件取**文件名词干**（`authz.rs` ⇒ `authz`）——`authz` 是一个
 *    真实的内部消费方，不是「根」；
 * 2. **根 `mod.rs` 不算消费方**：它是**导出清单**（`pub use <域>::{…}` 会把几乎每个
 *    符号都写成命中），把它算进来等于让每个符号自己给自己凑一个消费方。
 *
 * ## 为什么没有第三条「声明域自身不算」（2026-10-14 删除）
 *
 * 曾有口径 3「声明域自身是自引用，不算消费方」，并由一条回归测试钉住
 * （「声明域自身 / 根导出清单提及，都不算第二个消费模块（仍判红）」）。**那条测试钉的是
 * 错的答案，已一并改掉。**
 *
 * 错在哪：**消费就是依赖，自己依赖自己也是依赖。** `LonelyThing` 声明在 `actors`、
 * 被 `actors/other.rs` 用着，那就是 `actors` 在消费它——凭什么把它沉到 `plugins/alpha`？
 * 把 core 内部的依赖从计数里滤掉，等于**假装 core 不用它**，判据于是给出一个在事实上
 * 不成立的动作建议。
 *
 * 代价当场可见：删掉这一条后 C-003 单消费方 **56 → 0**（`--verbose` 前后清单 diff：
 * **释放 56 条、新增 0 条**——纯释放，没有「换一批人」）。那 56 条每一条 core 自己都在用，
 * 只是被滤掉了。
 *
 * ⚠️ 它已造成过**真实误判**：`LlmAdapter` / `DeltaSink` 的实现方（`ProviderLlmAdapter`、
 * `SilentDeltas`）就住在它们自己的声明域 `adapters` 里，于是被判成「单消费方接缝、该下沉」，
 * 而下沉会让 core 去实现插件的 trait ⇒ **依赖反转**。同类问题另有 `collectCoreBindings`
 * 单独补的「core 内 `impl <Trait> for …`」那一维——两条是同一个病的两副面孔。
 *
 * 与 `collectConsumers` 共享「测试不是消费方」的口径（口径 4），故同样跳过测试文件。
 */
/** 该符号的**声明行**（`pub struct X` / `pub trait X` / `pub fn X` …）——声明不是使用 */
const declRe = (name) =>
  new RegExp(`^[^\\n]*\\bpub(\\([^)]*\\))?\\s+(struct|enum|trait|fn|const|static|type|union)\\s+${name}\\b[^\\n]*$`, 'gm')

/** 该符号所在的 `pub use` **导出清单行**——理由同口径 2：清单会写成命中，但那是清单 */
const reexportRe = (name) => new RegExp(`^[^\\n]*\\bpub\\s+use\\b[^\\n]*\\b${name}\\b[^\\n]*$`, 'gm')

export function collectCoreDomainConsumers(root, symbols, coreRel = 'crates/symbio-core/src') {
  const names = [...symbols.keys()]
  const out = new Map()
  // 内核 crate 也是 core 自己的实现（D-188 C2a）：它内部的跨模块引用同样算
  // 「另一个 core 生产模块在用」，否则符号搬进 kernel 后会凭空丢掉这个消费方，
  // 被误判成「单消费方该下沉」。域名加 `kernel/` 前缀，与 core 的域区分开。
  const scanRoots = [
    { dir: path.join(root, coreRel), prefix: '' },
    { dir: path.join(root, 'crates', 'symbio-kernel', 'src'), prefix: 'kernel/' },
  ]
  for (const { dir: scanDir, prefix } of scanRoots) {
    if (!fs.existsSync(scanDir)) continue
    for (const file of walk(scanDir)) {
      if (!file.endsWith('.rs')) continue
      if (isTestFile(file)) continue
      const rel = path.relative(scanDir, file).split(path.sep).join('/')
      const head = rel.includes('/') ? rel.split('/')[0] : null
      const stem = rel.replace(/\.rs$/, '')
      const domain = prefix + (head ?? (stem === 'mod' ? '(root)' : stem))
      if (domain === '(root)' || domain === `${prefix}(root)`) continue // 口径 2：导出清单不是消费方
      if (domain === 'kernel' || domain === 'kernel/kernel') continue // 内核门面也是导出清单，理由同口径 2
      const src = stripComments(fs.readFileSync(file, 'utf8'))
      for (const name of names) {
        // ⚠️ 排除的是**声明点与导出清单**，不是整个声明域（2026-10-14，见上方口径说明）。
        // 只滤「声明域」那一行是不够的：域的 `mod.rs` 里 `pub struct X;` 本身就是声明，
        // 不滤掉的话每个符号都会被自己的声明算成一个 core 消费方 ⇒ 判据恒绿。
        const body = src
          .replace(declRe(name), '')
          .replace(reexportRe(name), '')
        if (!new RegExp(`\\b${name}\\b`).test(body)) continue
        if (!out.has(name)) out.set(name, new Set())
        out.get(name).add(domain)
      }
    }
  }
  return out
}

/**
 * 「该不该下沉」的唯一判据（判定型与报告型**共用**，防止两处各演化一条）：
 * 跨 core 的**生产**消费方恰 1 个，**且 core 内没有另一个模块**在用，**且**不是自引用。
 * 后两条合起来是一句话：**消费就是依赖，自己依赖自己也是依赖**——core 内部在用它，
 * 它就 ≥2 个单位共享的机制，下沉到那个唯一外部消费方（一个插件）会让 core 反向
 * 依赖插件。见 `collectCoreDomainConsumers` 的口径说明。
 */
export function isSinkCandidate(row, coreConsumers) {
  if (row.units.length !== 1) return false
  if ((coreConsumers.get(row.name)?.size ?? 0) > 0) return false
  return !isSelfReference({ name: row.name, units: row.units })
}

/**
 * 测试文件（命名约定见 `test-layout-audit.mjs`：`X.test.rs` 与 `tests.rs`），
 * 外加**只在测试构建里存在的模块**（`#[cfg(test)] mod test_support;`）。
 *
 * **测试不是架构消费方**——见文件头口径 4。这条判据只有这一处实现，三个审计共用。
 */
export function isTestFile(file) {
  if (file.endsWith('.test.rs') || path.basename(file) === 'tests.rs') return true
  return isCfgTestOnlyModule(file)
}

const PARENT_MODULE_CACHE = new Map()

/**
 * `#[cfg(test)] mod test_support;` 这类模块：文件名不是 `.test.rs`，但它**不参与
 * 生产构建**。旧口径只看文件名，于是把其中的类型读成生产消费方——`EventStore`
 * 因此在 C0 工作清单里（唯一「生产」消费方就是 `plugins/loop/test_support.rs`）。
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
 * 数每个公开符号的消费方单位（`Map<符号, Set<单位>>`）。
 *
 * - core 自身不算消费方（口径：公开面是给 core **外**用的）；
 * - 一个**生产**文件都不能跳过（口径 5）：`crates/` 直属文件也算；
 * - 宏调用点算消费方（见 `collectMacroBodies`）；
 * - **测试文件默认不算**（口径 4）——`opts.includeTests` 为真时才计入。
 *
 * ## 为什么给测试留一个开关，而不是一律排除
 *
 * 两条判定问的是**两个不同的问题**，答案因此不能共用同一个数：
 *
 * | 问题 | 口径 | 为什么 |
 * |---|---|---|
 * | 「它够不够格占根出口？」（0 消费方 → 收窄） | **含测试** | 测试也是 core 外的编译单元。一个只被别模块测试用到的符号，收窄它当场把那些测试编译坏（C-002 又不许它们深引）——所以「测试在用」**足以**让它留在根出口 |
 * | 「它算不算架构元素？」（单消费方 → 下沉） | **不含测试** | 测试依赖的是**被测对象**，不是它要长期持有的接口。让测试凑出第二个单位，等于「谁在测试里顺手写了一次名字」决定一个实现的归属 |
 *
 * 两者混成一个数，就是 `TurnRunner` 长期逃过下沉的机制（见口径 4）。
 */
export function collectConsumers(root, symbols, coreRel = 'crates/symbio-core/src', opts = {}) {
  const includeTests = opts.includeTests === true
  const allNames = [...symbols.keys()]
  const macros = collectMacroBodies(root, coreRel, symbols)
  const consumers = new Map()
  const scanRoots = ['crates', 'crates/symbio-cli/src', 'tauri/src-tauri/src'].map((r) => path.join(root, r))
  for (const scanRoot of scanRoots) {
    for (const file of walk(scanRoot)) {
      if (!file.endsWith('.rs')) continue
      const rel = path.relative(root, file)
      if (rel.split(path.sep).join('/').startsWith(coreRel + '/')) continue // core 自身不算消费方
      if (!includeTests && isTestFile(file)) continue // 测试不是架构消费方（口径 4）
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
