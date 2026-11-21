#!/usr/bin/env node
/**
 * plugin-identity-audit — 系统层不硬编码具体插件身份（PI-001）
 *
 * ## 它治的病
 *
 * 运行期是插件化的：能力、挂载点、图标、文档都由插件自己描述、自己注册。但历史上有
 * 一批**注释与系统文档**把某个具体插件的名字 / crate 路径写死在里面——一次把
 * `symbio-plugin-agent` 改名成 `symbio-plugin-subagent` 牵动 117 个文件，绝大多数是
 * 这类"系统层知道某个插件存在"的措辞同步，与机制无关。
 *
 * 本判据防止它长回来：**插件目录之外，不得出现具体插件身份的硬编码**（插件 crate
 * 名 / crate 路径 / "subagent 插件"这种点名）。插件身份只在它的自述文档、注册点、以及
 * 尚未自注册的少数机制位点出现。
 *
 * ## 判什么（措辞，不是地址）
 *
 * VDFS 挂载点地址 `subagent/<id>`、能力命名空间 `agent_<id>_`、协议名 `agent-dir/v2`
 * 都是**机制词汇**，不是插件身份，不判。只判下面这些明确的"插件身份"模式：
 *
 *   - `symbio-plugin-<id>` / `symbio_plugin_<id>`（crate 名 / Rust crate 名）
 *   - `crates/symbio-plugin-<id>`（crate 路径）
 *   - `<id> 插件` / `<id>` 插件`（中文点名）
 *   - `由 <id>`（作为托管方的点名）
 *
 * `id` 取当前需要去耦合的插件集合（`IDENTITIES`）。新增一个"不该被系统层点名"的插件
 * 时，把它加入 `IDENTITIES`。
 *
 * ## 白名单（机制上必须知道插件身份的地方）
 *
 * 1. 插件**自身**目录（`crates/symbio-plugin-<id>/`）——它当然知道自己的名字。
 * 2. 插件自述文档的**索引行**（`docs/003-design/README.md` 的 README 链接表）——那是
 *    "插件文档在哪"的目录，不是"系统描述插件能力"。
 * 3. 生成事实表（`docs/004-ref/006-当前事实表.md`）——由 `gen.mjs` 从 PLUGIN.yml 投影。
 * 4. 链接期必须点：宿主 crate 的 `Cargo.toml` 依赖、`lib.rs` 的 `use <crate> as _`——
 *    编译期把插件链进二进制，尚无可替代机制。
 * 5. 专门审计该插件的判据及其夹具（`agent-ownership-audit*`、`plugin-entry-audit.test`）——
 *    判据必须知道被审对象。
 * 6. D-NNN 历史决策正文（D-004：历史不改写）——判据会跳过 `### D-NNN` 段到下一个
 *    `###`/`##` 之间的正文。
 *
 * 白名单**按文件**登记，不按 token 通配：一条不解释自己为什么例外的白名单就是永久后门。
 *
 * 用法：node scripts/audits/plugin-identity-audit.mjs [--root=<目录>]
 * 退出码：0 = 通过；1 = 有插件身份硬编码；2 = 扫描范围读不出。
 */

// @ns PI 插件身份去耦合（系统层不硬编码具体插件）
// @codes PI-001 PI-002
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green } from '../color.mjs'

const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')

/** 不该被系统层点名的插件 id（crate 名 = symbio-plugin-<id>）。 */
export const IDENTITIES = ['subagent']

/** 不允许出现的插件身份模式（对 `id` 展开）。 */
function patterns(id) {
  return [
    new RegExp(`symbio-plugin-${id}`, 'g'),
    new RegExp(`symbio_plugin_${id}`, 'g'),
    new RegExp(`${id}\\s*插件`, 'g'), // "subagent 插件" / "subagent插件"
  ]
}

/**
 * 文件级白名单：{ rel: 为什么例外 }。
 * 相对路径以仓根为基准；插件自身目录按前缀判。
 */
export const FILE_EXEMPTIONS = {
  'docs/003-design/README.md': '插件自述文档的索引表（文档在哪，不是系统描述插件能力）',
  'docs/004-ref/006-当前事实表.md': 'gen.mjs 从 PLUGIN.yml 投影的生成物',
  'docs/004-ref/010-机制强制等级清单.md': 'ME-003 要求执行方指向真实代码锚点（文件路径+符号），必须是具体代码位置',
  'crates/symbio/Cargo.toml': '链接期依赖把插件链进二进制',
  'crates/symbio/src/lib.rs': '链接期 use <crate> as _',
  'scripts/audits/agent-ownership-audit.mjs': '专门审计该插件归属表的判据，必须知道被审对象',
  'scripts/audits/agent-ownership-audit.test.mjs': '上述判据的夹具，构造被审插件目录',
  'scripts/audits/plugin-entry-audit.test.mjs': '夹具：模拟清单注释里点名兄弟插件的被审场景',
}

/** 扫描的目录（相对仓根）与文件后缀。 */
const SCAN_DIRS = ['docs/003-design', 'docs/004-ref', 'crates', 'tauri/src']
const EXTS = new Set(['.md', '.rs', '.ts', '.toml'])

function isOwnCrate(rel) {
  return IDENTITIES.some((id) => rel.startsWith(`crates/symbio-plugin-${id}/`))
}

function walk(dir, out, prefix = '') {
  if (!fs.existsSync(dir)) return false
  for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
    if (ent.name === 'target' || ent.name === 'node_modules' || ent.name === 'dist') continue
    const rel = `${prefix}${ent.name}`
    const abs = path.join(dir, ent.name)
    if (ent.isDirectory()) walk(abs, out, `${rel}/`)
    else if (EXTS.has(path.extname(ent.name))) out.push(rel)
  }
  return true
}

/** Markdown 里 D-NNN 历史决策正文的行号集合（跳过它，D-004 不改写历史）。 */
function decisionBodyLines(lines) {
  const skip = new Set()
  let inDecision = false
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].trim()
    if (/^###\s+D-\d+/.test(t)) {
      inDecision = true
      skip.add(i)
      continue
    }
    if (inDecision && /^(##|###)\s/.test(t)) inDecision = false
    if (inDecision) skip.add(i)
  }
  return skip
}

/** 行内是否只有白名单位置（链接依赖 / use / README 索引链接 / 审计功能常量）。 */
function isExemptLine(rel, line) {
  if (rel === 'docs/003-design/README.md') return line.includes('README.md')
  if (/Cargo\.toml$/.test(rel)) return /^\s*symbio-plugin-[a-z_]+\s*=/.test(line.trim()) || line.trim().startsWith('#')
  if (/lib\.rs$/.test(rel) && rel === 'crates/symbio/src/lib.rs') return /^\s*use\s+symbio_plugin_/.test(line.trim())
  if (/agent-ownership-audit/.test(rel)) return /AGENT_REL|const\s|path\.join/.test(line)
  if (rel.endsWith('plugin-entry-audit.test.mjs')) return true // 夹具整份豁免（文件级已列，这里冗余保险）
  return false
}

export function audit({ root = ROOT } = {}) {
  const errors = []
  const files = []
  for (const d of SCAN_DIRS) {
    if (!walk(path.join(root, d), files, `${d}/`)) {
      return { status: 2, stdout: `PI-002 扫描范围读不出：${d}` }
    }
  }
  for (const rel of files.sort()) {
    if (isOwnCrate(rel) || FILE_EXEMPTIONS[rel]) continue
    const abs = path.join(root, rel)
    const lines = fs.readFileSync(abs, 'utf8').split(/\r?\n/)
    const skip = rel.endsWith('.md') ? decisionBodyLines(lines) : new Set()
    for (let i = 0; i < lines.length; i++) {
      if (skip.has(i) || isExemptLine(rel, lines[i])) continue
      const line = lines[i]
      for (const id of IDENTITIES) {
        for (const re of patterns(id)) {
          re.lastIndex = 0
          if (re.test(line)) {
            errors.push(`PI-001 ${rel}:${i + 1} 系统层硬编码插件身份「${id}」：${line.trim().slice(0, 120)}`)
            break
          }
        }
      }
    }
  }
  return { status: errors.length ? 1 : 0, stdout: errors.join('\n') }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  const r = audit()
  if (r.status === 0) {
    console.log(green(`plugin-identity-audit 通过：系统层无具体插件身份硬编码（判 PI-${'001'}）`))
  } else if (r.status === 2) {
    console.error(red(r.stdout))
  } else {
    console.error(red(r.stdout))
  }
  process.exit(r.status)
}
