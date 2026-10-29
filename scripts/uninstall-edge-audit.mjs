#!/usr/bin/env node
/**
 * uninstall-edge-audit — 可卸载插件的编译期边为零（EX-004 的静态一半）
 *
 * ## 它治的病
 *
 * 「卸载插件 ⇒ 系统完整运行」在运行期已经有用例盯住（停用 / 卸载后工具面与存储
 * 两端都验），但那些用例证明的是**同一个二进制里**的行为：`inventory` 注册项还在、
 * 目录还在，只是运行期被过滤。若另一个 crate 在**编译期**引用了某个可卸载插件的
 * crate（`use symbio_plugin_notes::…`、`symbio::symbio_plugin_notes::…`，或在其
 * `Cargo.toml` 里写一条依赖），那么「卸载」在源码层根本不成立：删掉那个目录，
 * 该处编译不过；留着目录，所谓卸载就只是运行期过滤——**一条静默边**。
 *
 * 本判据只判这一半：可卸载的插件 crate 不被任何别的 crate 的手写代码引用、不被任何
 * 别的 crate 的清单依赖。运行期那一半各有 owner，不在这里按名字再判一遍：
 *
 * | 读数 | owner |
 * |---|---|
 * | 卸载 `loop` / `session` 后存储与起始地址照常、必需插件卸载被拒 | `cargo test -p symbio --lib composite::tests`（`crates/symbio-plugin-composite/src/composite.test.rs`） |
 * | 停用插件后其工具从模型工具面消失 | e2e `t88-plugin-capability-gating` |
 * | 目录残留不参与装配、日志零 WARN | e2e `t80-orphan-plugin-residue` |
 *
 * ## 判谁
 *
 * 目标 = `crates/symbio-plugin-*` 里 `plugin_undisablable` 不为 `true` 的插件。
 * `plugin_manager` / `vdfs` 是界面底座（`ASSEMBLY_UNDISABLABLE_PLUGINS`），没有
 * 「卸载」这条路，对它们的编译期边不构成静默边——判了只会逼人写豁免。
 *
 * ## 豁免只有一处：生成的链接块
 *
 * `crates/symbio/src/{lib.rs,Cargo.toml}` 的 `plugin-links` 标记块由目录投影生成，
 * 它**就是**装配点的链接表（漏一行的后果是注册项被链接器丢掉）。标记块之外的
 * 手写引用全部算边。标记本身缺失也判红：那时生成行会被当成手写边，而报错会指向
 * 一整屏假阳性，真问题（谁写了跨插件引用）反而看不见。
 *
 * ## 失效形态（回归测试钉的是这几条）
 *
 * - 只查 `Cargo.toml` 不查代码 ⇒ 经宿主门面 `pub use` 的符号引用漏网；
 * - 把生成链接块算成手写边 ⇒ 一开豁免就再也看不见新边；
 * - 扫描含注释 / 字符串 ⇒ 文档里提一句插件名就红，判据自伤；
 * - 插件清单一空就通过 ⇒ 判据没有输入却全绿（本仓买到最多次的教训）；
 * - 测试代码放过 ⇒ 卸载后 `cargo test` 编译不过，仍是边。
 *
 * 用法：
 * ```bash
 * node scripts/uninstall-edge-audit.mjs
 * node scripts/uninstall-edge-audit.mjs --root=crates/symbio   # 只扫一棵子树（诊断用）
 * ```
 * 退出码：0 = 通过；1 = 有边，或范围读不出（插件清单为空 / 标记块缺失）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns EX 能力全面拆分（名册处置与迁移边界）
// @codes EX-004

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, dim } from './color.mjs'
import { blankComments } from './rust-scan.mjs'
import { PLUGIN_CRATE_PREFIX, pluginIds, readPluginManifest } from './facts/_plugin-yml.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const argv = process.argv.slice(2)

/** 生成的链接块标记：`lib.rs` 与 `Cargo.toml` 同形（注释前缀不参与判定）。 */
export const MARK_BEGIN = '>>> plugin-links:begin'
export const MARK_END = '<<< plugin-links:end'

/** 装配点：链接表由目录投影生成，是唯一允许提及插件 crate 的手写文件。 */
export const HOST_FACADE = 'crates/symbio/src/lib.rs'
export const HOST_MANIFEST = 'crates/symbio/Cargo.toml'

/** 扫描面：只有会被编译的 Rust 源码与清单算边。 */
const SCAN_ROOTS = ['crates', 'tauri/src-tauri']

/** 不进的目录（`target` 是产物；其余是 Tauri 的非 Rust 生成面）。 */
const SKIP_DIRS = new Set(['target', 'node_modules', '.git', 'gen', 'capabilities', 'icons'])

/**
 * 可卸载插件：`crates/symbio-plugin-*` 里 `plugin_undisablable` 不为 `true` 的那些。
 *
 * 返回的 `crate` 是 Rust 里的标识符形态（`-` → `_`），`manifest` 用连字符形态
 * （Cargo 依赖键）。清单读不出 / 为空都往上抛——那是「范围读空了」，不是「没有插件」。
 */
export function disablablePlugins(root) {
  const ids = pluginIds(root)
  const out = []
  const skipped = []
  for (const id of ids) {
    const manifest = readPluginManifest(root, id)
    const entry = {
      id,
      crate: `${PLUGIN_CRATE_PREFIX}${id}`.replace(/-/g, '_'),
      manifest: `${PLUGIN_CRATE_PREFIX}${id}`,
      dir: `crates/${PLUGIN_CRATE_PREFIX}${id}`,
    }
    if (manifest.plugin_undisablable === 'true') skipped.push(entry)
    else out.push(entry)
  }
  return { targets: out, skipped }
}

/**
 * 把标记块整段置空（含标记行），返回置空后的行与见到的块数。
 *
 * 只用于生成块：块内每一条链接行都是目录投影，不承载人写的依赖。块外的行原样保留。
 */
export function blankMarked(lines, { begin = MARK_BEGIN, end = MARK_END } = {}) {
  const out = []
  let inside = false
  let blocks = 0
  for (const line of lines) {
    if (inside) {
      out.push('')
      if (line.includes(end)) inside = false
      continue
    }
    if (line.includes(begin)) {
      inside = true
      blocks += 1
      out.push('')
      continue
    }
    out.push(line)
  }
  return { lines: out, blocks }
}

/** 一行里的双引号字符串置空：`"symbio_plugin_notes"` 是名字，不是编译期边。 */
export function blankStrings(line) {
  let out = ''
  let inStr = false
  for (let i = 0; i < line.length; i += 1) {
    const ch = line[i]
    if (inStr) {
      if (ch === '\\') {
        out += '  '
        i += 1
        continue
      }
      if (ch === '"') {
        inStr = false
        out += ch
        continue
      }
      out += ' '
      continue
    }
    if (ch === '"') {
      inStr = true
      out += ch
      continue
    }
    out += ch
  }
  return out
}

function walk(dir, out, names) {
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of entries) {
    if (e.isDirectory()) {
      if (SKIP_DIRS.has(e.name)) continue
      walk(path.join(dir, e.name), out, names)
    } else if (names(e.name)) {
      out.push(path.join(dir, e.name))
    }
  }
}

function rel(root, abs) {
  return path.relative(root, abs).split(path.sep).join('/')
}

/** 词边界匹配插件 crate 标识符（`_` 属于单词字符，故不会截到别的 crate 中间）。 */
function crateRe(target) {
  return new RegExp(`\\b${target.crate.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`)
}

/** 清单里的依赖键（连字符形态）：`key = …` / `[dependencies.key]` / `package = "key"`。 */
function manifestRe(target) {
  const key = target.manifest.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  return new RegExp(`(^|[^A-Za-z0-9_-])${key}([^A-Za-z0-9_-]|$)`)
}

/**
 * Rust 源码里的编译期边。
 *
 * 注释与字符串都置空：`// 见 symbio_plugin_notes` 不是边，`"symbio_plugin_notes"`
 * 也不是。测试代码一并扫——`#[cfg(test)]` 引用了可卸载插件的符号，卸载后
 * `cargo test` 就编译不过，那同样是一条静默边。
 */
export function rustEdges({ root, targets }) {
  const files = []
  for (const r of SCAN_ROOTS) walk(path.join(root, ...r.split('/')), files, (n) => n.endsWith('.rs'))
  const prepared = targets.map((t) => ({ ...t, re: crateRe(t) }))
  const hits = []
  const seen = new Set()
  let scanned = 0
  for (const abs of files) {
    const r = rel(root, abs)
    const own = prepared.find((t) => r.startsWith(`${t.dir}/`))
    const rawLines = fs.readFileSync(abs, 'utf8').split(/\r?\n/)
    const { lines } = blankMarked(rawLines)
    const blanked = blankComments(lines.join('\n'))
    scanned += 1
    for (const t of prepared) {
      if (own && own.id === t.id) continue
      const key = `${t.id}\0${r}`
      if (seen.has(key)) continue
      for (let i = 0; i < blanked.length; i += 1) {
        if (t.re.test(blankStrings(blanked[i] ?? ''))) {
          seen.add(key)
          hits.push({ target: t.id, where: `${r}:${i + 1}`, line: (rawLines[i] ?? '').trim().slice(0, 100) })
          break
        }
      }
    }
  }
  return { hits, scanned }
}

/**
 * 清单里的编译期边。
 *
 * `#` 注释与标记块先置空（生成块里的依赖行由目录投影）；自清单跳过——每个插件的
 * `name = "symbio-plugin-x"` 不是依赖。
 */
export function manifestEdges({ root, targets }) {
  const files = []
  for (const r of SCAN_ROOTS) walk(path.join(root, ...r.split('/')), files, (n) => n === 'Cargo.toml')
  const prepared = targets.map((t) => ({ ...t, re: manifestRe(t) }))
  const hits = []
  const seen = new Set()
  let scanned = 0
  for (const abs of files) {
    const r = rel(root, abs)
    const own = prepared.find((t) => `${t.dir}/Cargo.toml` === r)
    const rawLines = fs.readFileSync(abs, 'utf8').split(/\r?\n/)
    const { lines } = blankMarked(rawLines)
    const stripped = lines.map((l) => l.replace(/#.*$/, ''))
    scanned += 1
    for (const t of prepared) {
      if (own && own.id === t.id) continue
      const key = `${t.id}\0${r}`
      if (seen.has(key)) continue
      for (let i = 0; i < stripped.length; i += 1) {
        if (t.re.test(stripped[i])) {
          seen.add(key)
          hits.push({ target: t.id, where: `${r}:${i + 1}`, line: (rawLines[i] ?? '').trim().slice(0, 100) })
          break
        }
      }
    }
  }
  return { hits, scanned }
}

/** 生成的链接块必须在位：缺失时链接表会被当成手写边，报错指向一整屏假阳性。 */
export function markerErrors({ root }) {
  const errors = []
  for (const relPath of [HOST_FACADE, HOST_MANIFEST]) {
    const abs = path.join(root, ...relPath.split('/'))
    if (!fs.existsSync(abs)) {
      errors.push(`EX-004 装配点 ${relPath} 不存在——生成的链接表读不到，判据没有输入`)
      continue
    }
    const { blocks } = blankMarked(fs.readFileSync(abs, 'utf8').split(/\r?\n/))
    if (blocks === 0) {
      errors.push(`EX-004 ${relPath} 缺 \`${MARK_BEGIN}\` 标记块——生成的链接行会被当成手写边；先跑 \`node scripts/gen-facts.mjs\``)
    }
  }
  return errors
}

/** 汇总判定：标记块、插件范围、源码与清单两侧的边。 */
export function judge(root) {
  const errors = []
  const marker = markerErrors({ root })
  errors.push(...marker)
  const { targets, skipped } = disablablePlugins(root)
  if (targets.length === 0) {
    errors.push('EX-004 可卸载插件清单为空——判据没有目标，不当作通过')
    return { errors, targets, skipped, rust: { hits: [], scanned: 0 }, manifest: { hits: [], scanned: 0 } }
  }
  const rust = rustEdges({ root, targets })
  const manifest = manifestEdges({ root, targets })
  for (const h of [...rust.hits, ...manifest.hits]) {
    errors.push(
      `EX-004 可卸载插件 \`${h.target}\` 被 \`${h.where}\` 编译期引用（${h.line}）` +
        ' —— 卸载后该处编译不过，或目录留着只做运行期过滤（静默边）；跨插件能力只经 `plugin-api` 的声明面 / 共享表',
    )
  }
  return { errors, targets, skipped, rust, manifest }
}

function parseRoot() {
  for (const a of argv) {
    if (a.startsWith('--root=')) return path.resolve(a.slice('--root='.length))
  }
  const i = argv.indexOf('--root')
  if (i >= 0 && argv[i + 1]) return path.resolve(argv[i + 1])
  return repoRoot
}

function main() {
  const root = parseRoot()
  let result
  try {
    result = judge(root)
  } catch (e) {
    console.error(red(`✗ uninstall-edge-audit：${String(e?.message ?? e)}`))
    return 1
  }
  if (result.errors.length) {
    console.log(red(`\nuninstall-edge-audit 未通过：${result.errors.length} 处`))
    for (const e of result.errors) console.log(`  ${e}`)
    console.log(
      dim('\n  可卸载的插件只经 `plugin-api` / 声明表被使用；要直接引用它的 crate 符号，就先回答「卸载它时这处怎么办」。'),
    )
    return 1
  }
  console.log(
    green(
      `✓ uninstall-edge-audit：${result.targets.length} 个可卸载插件的编译期边为 0` +
        `（${result.rust.scanned} 份 Rust 源码 / ${result.manifest.scanned} 份清单）`,
    ),
  )
  if (result.skipped.length) {
    console.log(dim(`  ⊘ 不可停用、故不判：${result.skipped.map((t) => t.id).join(' / ')}`))
  }
  return 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(main())
}
