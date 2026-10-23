#!/usr/bin/env node
/**
 * agent-ownership-audit — Agent 主机面的归属表 ↔ 装配清单 ↔ 插件目录（AO-001…006）
 *
 * ## 它治的病
 *
 * 「智能体自身的记忆归 `memory`、设置归 `setting`」这类**归属声明**在本仓有三处手写副本
 * （`plugins/agent/README.md`、`host/mod.rs`、`host/plugin.rs` 的模块文档），而它们的
 * 事实来源在别处：清单是 `symbio_core/assembly/mod.rs` 的常量，目录是 `plugins/` 下的文件夹。
 * **没有任何东西会因此变红**——09 报告 D-4/D-9 抓到 README 有 5 处过期描述，其中两处
 * 直接把读者指向不存在的模块（`host/instruction.rs`）与不存在的插件（`work`），
 * 一处把成员资格说反（`vdfs 不在其中`，而清单里就在）。
 *
 * 这类表的失效根因是「手写 + 无人校验」，所以补的是**一条比对脚本**而不是逐次人工核对：
 * 约定 → 机制的标准动作（相应批次 S-7）。
 *
 * ## 判据
 *
 * | 码 | 判什么 | 为什么 |
 * |---|---|---|
 * | AO-001 | 归属表每一行的「归谁」必须是真实插件目录，且「地址」列的首段等于归谁 | 指向已不存在的插件 = 读者照抄一个到不了的地方；两列各说各话 = 同一行内部矛盾，比跨文档漂移更早发生 |
 * | AO-002 | 同一张归属表在各文档里的**所有副本**逐列一致（一个「东西」只有一个归谁与地址） | 副本本身是刻意的（Rust 模块文档要就地可读，各层各放一份），风险只在漂移；「改一处忘改另一处」这个形状只有比对拦得住 |
 * | AO-003 | README 的子树清单判词（`` - `X` 在其中`` / `不在其中`）必须与 `ASSEMBLY_SUB_AGENT_PLUGINS` 一致，且 X 是真实插件目录 | 判词说反（`vdfs`）与判一个不存在的插件（`work`）都在这条里；它是「清单 ↔ 文档」唯一的成员资格陈述 |
 * | AO-004 | `host/mod.rs` 的职责表 ↔ `host/*.rs` 实际模块**双向**覆盖 | 表里列了没有的文件、加了模块不登记表，两边都是「这张表不再描述这一层」；只判单向等于默认后来者只会漏登记 |
 * | AO-005 | 文档写出的 `host/<name>.rs` 与 `../<插件>/README.md` 路径必须存在 | 这两条是本插件**模块路径与兄弟插件地址**的唯一书写形状，改名/删文件后留下的幽灵指路就是 D-4 的第三、五处。`doc-link-audit` 的 D-010 不覆盖它们：那三处失效正发生在它全绿的那一轮门禁里（实测 12:34 那轮 123/124 全绿，README 仍写着 `host/instruction.rs` 与一个已删除的兄弟插件地址） |
 * | AO-006 | 装配清单的每个成员必须是真实插件目录 | 清单写了不存在的名字 ⇒ 容器扫描时静默少挂一个插件，而清单与文档两边都「看起来齐」 |
 *
 * ## 已知边界（写清楚，免得把它的绿灯当成证明）
 *
 * 判定基于**行文本**，不是 AST，也不解析中文散文：
 * - AO-003 只认「`- \`X\` 在其中`」这一书写形状（README 的实际形状），散文里提到
 *   `` `work` `` 不判——那是叙述，不是成员资格声明；
 * - AO-005 只认 `host/` 与 `../<名>/README.md` 两个前缀，别处的相对路径（示例目录
 *   `skill/foo/SKILL.md`、挂载地址 `work/AGENTS.md`）一律不判：**天天误报的守卫最后
 *   只会被豁免喂死**（`grep-audit` S-012 的教训）；
 * - 表头不是那两张的表（如 README 的 `交出什么 | 机制`）不参与判定——本脚本判的是
 *   **归属表与职责表这两个形状**，不是「所有表格的所有单元格」。
 *
 * ## 用法
 *
 *   node scripts/agent-ownership-audit.mjs                 # 审计全仓
 *   node scripts/agent-ownership-audit.mjs --root=<dir>    # 换根目录（回归测试用）
 *
 * 单行豁免：`// agent-ownership-allow AO-00X: 理由`（理由不可为空，与其余守卫同一约定；
 * markdown 里写成 HTML 注释，结尾的 `-->` 不算理由）。
 *
 * 门禁按 `--strict` 调用：本脚本没有 WARNING 级判定，带不带同义。
 *
 * 退出码：0 = 通过；1 = 有失效；2 = 审计范围读不出（清单、目录、host 模块或 agent 文档
 *      任一取不到即红，**不退化成「没发现违规」**）。
 */
// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些号生成，判据见 gate-codes-audit.mjs）
// @ns AO Agent 归属表对账（模块文档 ↔ 装配清单 ↔ 插件目录）
// @codes AO-001 AO-002 AO-003 AO-004 AO-005 AO-006

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, dim } from './color.mjs'
import { cellsOf, isSeparator, plain } from './md-table.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')

/** 本判据的三份来源（唯一地址，变了改这里）。 */
export const AGENT_REL = 'symbio/src/plugins/agent'
export const PLUGINS_REL = 'symbio/src/plugins'
export const ASSEMBLY_REL = 'symbio/src/symbio_core/assembly/mod.rs'
/** 子树默认清单的常量名（全仓插件清单的唯一字面量，见 `assembly/mod.rs` 文档）。 */
export const ASSEMBLY_CONST = 'ASSEMBLY_SUB_AGENT_PLUGINS'

/** 归属表的表头（三列逐字相等才算这张表；换了列名就不是本脚本认的那条事实）。 */
const OWNERSHIP_HEADER = ['东西', '归谁', '地址']
/** 职责表的表头首列（`host/mod.rs` 的「这一层有哪些模块」）。 */
const MODULE_HEADER = '模块'

/** 一行的出处 + 原文。 */
const found = (file, line, text, code, why) => ({
  code,
  file,
  line,
  text: text.trim().slice(0, 72),
  why,
})

/**
 * 参与判定的语料：agent 插件 README + `host/` 各模块的 `//!` 文档注释。
 * 行一律收成 `{ text, line }`（行号保持原文件的，报出来的地址能跳过去）。
 *
 * 取 `//!` 而不是整份源码：模块文档才是「就地可读」的那份归属声明，代码里的字符串
 * 常量不是（把它们当声明判，第一条误报就是 agent id 的命名空间登记表）。
 */
function agentDocs(root) {
  const agentDir = path.join(root, AGENT_REL.split('/').join(path.sep))
  const hostDir = path.join(agentDir, 'host')
  if (!fs.existsSync(agentDir) || !fs.existsSync(hostDir)) return null
  const out = []
  const relOf = (abs) => path.relative(root, abs).split(path.sep).join('/')
  const asLines = (txt) => txt.split(/\r?\n/).map((text, i) => ({ text, line: i + 1 }))
  const readme = path.join(agentDir, 'README.md')
  if (fs.existsSync(readme)) out.push({ rel: relOf(readme), lines: asLines(fs.readFileSync(readme, 'utf8')) })
  for (const f of fs.readdirSync(hostDir).filter((n) => n.endsWith('.rs') && !n.endsWith('.test.rs')).sort()) {
    const abs = path.join(hostDir, f)
    const lines = fs
      .readFileSync(abs, 'utf8')
      .split(/\r?\n/)
      .map((raw, i) => ({ raw, line: i + 1 }))
      .map((r) => ({ m: /^\/\/!\s?(.*)$/.exec(r.raw), line: r.line }))
      .filter((r) => r.m)
      .map((r) => ({ text: r.m[1], line: r.line }))
    out.push({ rel: relOf(abs), lines })
  }
  return out
}

/** 真实插件目录名。 */
function pluginDirs(root) {
  const dir = path.join(root, PLUGINS_REL.split('/').join(path.sep))
  if (!fs.existsSync(dir)) return null
  const names = fs.readdirSync(dir, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name)
  return names.length ? new Set(names) : null
}

/** 装配清单的成员（按声明顺序）。 */
function assemblyNames(root) {
  const abs = path.join(root, ASSEMBLY_REL.split('/').join(path.sep))
  if (!fs.existsSync(abs)) return null
  const m = new RegExp(`pub const ${ASSEMBLY_CONST}\\s*:\\s*&\\[&str\\]\\s*=\\s*&\\[([\\s\\S]*?)\\n\\]`).exec(
    fs.readFileSync(abs, 'utf8')
  )
  if (!m) return null
  const names = [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1])
  return names.length ? names : null
}

/** `host/` 下真实模块名（不含 `mod.rs`、测试聚合 `tests.rs`、`*.test.rs`）。 */
function hostModules(root) {
  const dir = path.join(root, AGENT_REL.split('/').join(path.sep), 'host')
  if (!fs.existsSync(dir)) return null
  return new Set(
    fs
      .readdirSync(dir)
      .filter((n) => n.endsWith('.rs') && !n.endsWith('.test.rs') && n !== 'mod.rs' && n !== 'tests.rs')
      .map((n) => n.replace(/\.rs$/, ''))
  )
}

/** 单元格里第一个反引号片段；没有反引号就取纯文本。 */
function firstBacktick(cell) {
  const m = /`([^`]+)`/.exec(cell)
  return m ? m[1] : plain(cell)
}

/**
 * 取出语料行里所有表：`{ header: [...], rows: [{ text, line, cells }] }`。
 * 表与表之间靠「非表行」断开（与 `md-table.mjs` 同一套形状解析，不另起解析器）。
 */
function tablesOf(lines) {
  const out = []
  let cur = null
  const close = () => {
    if (cur) out.push(cur)
    cur = null
  }
  for (const ln of lines) {
    const cells = cellsOf(ln.text)
    if (!cells) {
      close()
      continue
    }
    if (!cur) {
      cur = { header: cells.map(plain), rows: [] }
      continue
    }
    if (isSeparator(cells)) continue
    cur.rows.push({ text: ln.text, line: ln.line, cells })
  }
  close()
  return out
}

/** 归属表（表头逐列等于 `东西 | 归谁 | 地址`）。 */
function ownershipTables(doc) {
  return tablesOf(doc.lines).filter((t) => t.header.length === OWNERSHIP_HEADER.length && OWNERSHIP_HEADER.every((h, i) => t.header[i] === h))
}

/** AO-001：每行的「归谁」是真实插件目录，且地址首段与归谁同源。 */
function checkOwnershipRows(docs, dirs) {
  const findings = []
  for (const doc of docs) {
    for (const table of ownershipTables(doc)) {
      for (const row of table.rows) {
        const thing = plain(row.cells[0])
        const owner = firstBacktick(row.cells[1])
        const addr = firstBacktick(row.cells[2])
        if (!dirs.has(owner)) {
          findings.push(found(doc.rel, row.line, row.text, 'AO-001', `「归谁」= \`${owner}\` 不是真实插件目录（${thing}）`))
          continue
        }
        const m = /^<根>\/([^/]+)/.exec(addr)
        if (m && m[1] !== owner) {
          findings.push(found(doc.rel, row.line, row.text, 'AO-001', `地址首段 \`${m[1]}\` 与「归谁」\`${owner}\` 不同源（${thing}）`))
        }
      }
    }
  }
  return findings
}

/** AO-002：一个「东西」在全部副本里只有一个归谁与地址。 */
function checkOwnershipCopies(docs) {
  const declared = new Map()
  const findings = []
  for (const doc of docs) {
    for (const table of ownershipTables(doc)) {
      for (const row of table.rows) {
        const thing = plain(row.cells[0])
        const owner = firstBacktick(row.cells[1])
        const addr = firstBacktick(row.cells[2])
        const prev = declared.get(thing)
        if (!prev) {
          declared.set(thing, { owner, addr, where: `${doc.rel}:${row.line}` })
          continue
        }
        if (prev.owner === owner && prev.addr === addr) continue
        findings.push(
          found(doc.rel, row.line, row.text, 'AO-002', `「${thing}」与 ${prev.where} 不一致：${prev.owner} / ${prev.addr} ≠ ${owner} / ${addr}`)
        )
      }
    }
  }
  return findings
}

/** AO-003：子树清单判词 ↔ 装配清单 ↔ 插件目录。 */
function checkMembershipClaims(docs, dirs, assembly) {
  const inList = new Set(assembly)
  const re = /^-\s*`([^`]+)`\s*(不)?在其中/
  const findings = []
  for (const doc of docs) {
    for (const ln of doc.lines) {
      const m = re.exec(ln.text)
      if (!m) continue
      const name = m[1]
      const claimed = !m[2]
      if (!dirs.has(name)) {
        findings.push(found(doc.rel, ln.line, ln.text, 'AO-003', `判词里的 \`${name}\` 不是真实插件目录`))
        continue
      }
      if (inList.has(name) !== claimed) {
        findings.push(
          found(doc.rel, ln.line, ln.text, 'AO-003', `说 \`${name}\` ${claimed ? '在' : '不在'}子树清单，而 ${ASSEMBLY_CONST} ${claimed ? '没有' : '有'}它`)
        )
      }
    }
  }
  return findings
}

/** AO-004：`host/mod.rs` 的职责表 ↔ 实际模块双向覆盖。 */
function checkModuleTable(docs, modules, root) {
  const modRel = `${AGENT_REL}/host/mod.rs`
  const doc = docs.find((d) => d.rel === modRel)
  if (!doc) return [found(modRel, 0, '', 'AO-004', '读不到 host/mod.rs（职责表的来源）')]
  const table = tablesOf(doc.lines).find((t) => t.header[0] === MODULE_HEADER)
  if (!table || !table.rows.length) return [found(modRel, 0, '', 'AO-004', 'host/mod.rs 里没有「模块」开头的职责表（被改名或删掉）')]
  const findings = []
  const listed = new Set()
  for (const row of table.rows) {
    const name = firstBacktick(row.cells[0])
    if (!name) continue
    listed.add(name)
    if (!modules.has(name)) findings.push(found(modRel, row.line, row.text, 'AO-004', `职责表列了 \`${name}\`，而 host/${name}.rs 不存在`))
  }
  const abs = path.join(root, AGENT_REL.split('/').join(path.sep), 'host')
  for (const name of [...modules].sort()) {
    if (!listed.has(name)) findings.push(found(modRel, 0, `${abs}${path.sep}${name}.rs`, 'AO-004', `模块 host/${name}.rs 没进职责表`))
  }
  return findings
}

/** AO-005：文档指认的 `host/<name>.rs` 与 `../<插件>/README.md` 必须存在。 */
function checkPathClaims(docs, dirs, root) {
  const hostRe = /`(host\/[\w.-]+\.rs)`/g
  const siblingRe = /`(\.\.\/([\w-]+)\/README\.md)`/g
  const findings = []
  for (const doc of docs) {
    for (const ln of doc.lines) {
      for (const m of ln.text.matchAll(hostRe)) {
        const abs = path.join(root, AGENT_REL.split('/').join(path.sep), m[1].split('/').join(path.sep))
        if (!fs.existsSync(abs)) findings.push(found(doc.rel, ln.line, ln.text, 'AO-005', `指认的 ${m[1]} 不存在`))
      }
      for (const m of ln.text.matchAll(siblingRe)) {
        if (!dirs.has(m[2])) findings.push(found(doc.rel, ln.line, ln.text, 'AO-005', `指认的兄弟插件 \`${m[2]}\` 没有目录（${m[1]}）`))
        else if (!fs.existsSync(path.join(root, PLUGINS_REL.split('/').join(path.sep), m[2], 'README.md')))
          findings.push(found(doc.rel, ln.line, ln.text, 'AO-005', `${m[1]} 不存在`))
      }
    }
  }
  return findings
}

/** AO-006：装配清单成员必须是真实插件目录。 */
function checkAssemblyMembers(assembly, dirs) {
  return assembly
    .filter((n) => !dirs.has(n))
    .map((n) => found(ASSEMBLY_REL, 0, `"${n}"`, 'AO-006', `清单里的 \`${n}\` 没有插件目录 ⇒ 容器静默少挂`))
}

/** 行级豁免：`agent-ownership-allow AO-00X: 理由`（理由为空视为未豁免）。
 *  markdown 里它写成 HTML 注释，结尾的 `-->` 不是理由的一部分，剥掉再判空。 */
function allowed(lineText, code) {
  if (!lineText) return false
  const m = /agent-ownership-allow\s+([A-Z]{2,4}-\d{3})\s*:\s*(.+)$/.exec(lineText)
  return Boolean(m && m[1] === code && m[2].replace(/-->\s*$/, '').trim().length > 0)
}

console.log('--- agent-ownership-audit: 归属表 ↔ 装配清单 ↔ 插件目录（AO-001…006） ---')

const docs = agentDocs(REPO_ROOT)
const dirs = pluginDirs(REPO_ROOT)
const assembly = assemblyNames(REPO_ROOT)
const modules = hostModules(REPO_ROOT)
if (!docs || !dirs || !assembly || !modules) {
  console.log(
    red('[ERROR] 审计范围读不出：') +
      `docs=${docs ? docs.length : 'null'} dirs=${dirs ? dirs.size : 'null'} assembly=${assembly ? assembly.length : 'null'} modules=${modules ? modules.size : 'null'}`
  )
  console.log(dim('  来源读不出就是这条守卫死了，不许退化成「没发现违规」。'))
  process.exit(2)
}

const all = [
  ...checkOwnershipRows(docs, dirs),
  ...checkOwnershipCopies(docs),
  ...checkMembershipClaims(docs, dirs, assembly),
  ...checkModuleTable(docs, modules, REPO_ROOT),
  ...checkPathClaims(docs, dirs, REPO_ROOT),
  ...checkAssemblyMembers(assembly, dirs),
]
const lineTextOf = (f) => docs.find((d) => d.rel === f.file)?.lines.find((l) => l.line === f.line)?.text ?? ''
const findings = all.filter((f) => !allowed(lineTextOf(f), f.code))

console.log(dim(`  语料：${docs.length} 份 agent 文档 · 清单 ${assembly.length} 项 · 插件目录 ${dirs.size} 个 · host 模块 ${modules.size} 个`))
if (findings.length === 0) {
  console.log(green(`  ✓ 归属表、装配清单与插件目录三者一致（清单 ${assembly.length} 项 / 文档判词全对）`))
  process.exit(0)
}
for (const f of findings) {
  console.log(`${red(`[ERROR] ${f.code}`)} ${f.file}:${f.line} —— ${f.why}`)
  if (f.text) console.log(dim(`    ${f.text}`))
}
console.log(red(`  失效 ${findings.length} 处`))
process.exit(1)
