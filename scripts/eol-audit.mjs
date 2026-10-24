#!/usr/bin/env node
/**
 * eol-audit — **行尾是输入的前提**，不是每个解析器各自兜住的东西（判定型）
 *
 * ## 它判什么
 *
 * | 编号 | 规则 | 它拦的是哪一种回潮 |
 * |---|---|---|
 * | EOL-001 | `eol=lf` 声明的文本文件，工作树里带 `\r` | 编辑器 / 工具直写把工作树写成 CRLF，`git diff` 一个字符都不显示 |
 * | EOL-002 | 文本文件没有 `eol` 声明 | 新类型（`*.tsx` / `*.yml` / `LICENSE` …）从声明网里漏掉，行尾从此无人管 |
 * | EOL-003 | 已入库的 blob 带 `\r` | 入库方向的漂移：`eol=lf` 之前提交过 CRLF，diff 从此不可审阅 |
 *
 * ## 为什么需要它（现场）
 *
 * `.gitattributes` 声明了「仓库与工作树都是 LF」，**没有任何判据看过它是否成立**。
 * 2026-10-24 的实测：98 个文本文件的工作树是 CRLF，而 `git diff` 一个字符都不显示——
 * attr 在 diff 时把工作树 CRLF 归一化掉了，整仓「看起来干净」。
 *
 * 受害的是**读取侧**。`route-facts.routeArmDocs` 按 `^\s*\/\/[/!]?\s?(.*)$` 抽「路由臂
 * 上方的 `///`」，JS 的 `.` 不吃 `\r`、`$` 又要求行尾对齐，于是 CRLF 文件里整段注释
 * 一条都抽不到 —— 能力矩阵里 `compose/wording` 的职责静默变成 `—`，`facts` 测试报的是
 * 「表里没写」，真实原因是「读取器读不到」。**守卫红了，报的却是另一件事。**
 *
 * ## 为什么不在每个正则里补 `\r?`
 *
 * 补 `\r?` 是逐个调用点兜底：每个解析器各自持有一份「行尾无关」的口径，而这份口径
 * 没人复核——今天补了三个正则，明天新写的第四个照样踩。行尾属于**输入的前提**，
 * 前提只能有一处判据（仓里 LF），而不是由 N 个解析器各自容忍。与 `SP-013`「锚必须是
 * 版本化的地址」同型：把环境的默认值变成被检查的事实。
 *
 * ## 判据的语料来自 git 自己
 *
 * 不维护扩展名清单：`git ls-files` 给「受版本控制的文件」，`git check-attr text eol`
 * 给「git 认为这条路径该是什么行尾」。手写清单的失效形态是「新类型进来没人加」，
 * 而那正是 EOL-002 要拦的东西——用清单去判清单等于没判。
 *
 * 二进制按**内容**判（前 8KB 里有 `NUL`），与 git 的 auto-detection 同一个启发式：
 * 二进制文件的「行尾」不是一个概念，判它只会逼人写豁免。
 *
 * ## 用法
 *   node scripts/eol-audit.mjs               # 判定真实仓库
 *   node scripts/eol-audit.mjs --root=<dir>  # 指向别处的仓库（回归测试用）
 *   node scripts/eol-audit.mjs --list         # 逐条列出（默认只列前 20 条）
 * 退出码：0 = 通过；1 = 违规（或读不到 git —— 判不了的判据不许亮绿灯）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns EOL 行尾（工作树 / 入库 / 声明三面一致）
// @codes EOL-001 EOL-002 EOL-003

import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { red, green, dim } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')
const SHOW_ALL = process.argv.includes('--list')

/** 例外：`eol=none` 之类显式「不管行尾」的声明不算缺声明（本仓目前没有，留口子是为了
 *  读得懂——`-eol` 是有人**明确写过**的，与「没人写过」不是一回事）。 */
const EOL_OK = new Set(['lf'])

function git(args, input) {
  return spawnSync('git', args, {
    cwd: ROOT,
    encoding: 'buffer',
    input,
    maxBuffer: 64 * 1024 * 1024,
  })
}

/** 受版本控制的文件（`-z`：路径按 NUL 分隔，中文与空格不必转义） */
function trackedFiles() {
  const r = git(['ls-files', '-z'])
  if (r.status !== 0) return null
  return r.stdout.toString('utf8').split('\0').filter(Boolean)
}

/** 路径 → { text, eol }（`git check-attr -z --stdin` 一次问完，逐文件问会 spawn 1544 次） */
function attrsOf(files) {
  const r = git(['check-attr', '-z', '--stdin', 'text', 'eol'], Buffer.from(files.join('\0') + '\0', 'utf8'))
  if (r.status !== 0) return null
  const parts = r.stdout.toString('utf8').split('\0')
  const out = new Map()
  for (let i = 0; i + 2 < parts.length; i += 3) {
    const p = parts[i]
    if (!out.has(p)) out.set(p, {})
    out.get(p)[parts[i + 1]] = parts[i + 2]
  }
  return out
}

/** 内容像不像二进制：前 8KB 有 NUL 即二进制（同 git 的启发式） */
function looksBinary(abs) {
  let fd
  try {
    fd = fs.openSync(abs, 'r')
    const buf = Buffer.alloc(8192)
    const n = fs.readSync(fd, buf, 0, buf.length, 0)
    return buf.subarray(0, n).includes(0)
  } catch {
    return false // 读不到（子模块 / 工作树缺失）不判，见下面的 `missing`
  } finally {
    if (fd !== undefined) fs.closeSync(fd)
  }
}

/** 工作树里是否带 `\r`（只判文本；整文件读，行尾判定要的是全文，不是采样） */
function worktreeHasCr(abs) {
  try {
    return fs.readFileSync(abs).includes(0x0d)
  } catch {
    return false
  }
}

const files = trackedFiles()
if (files === null) {
  console.error(red('eol-audit 无法运行：`git ls-files` 失败（不是 git 仓库，或 git 不可用）'))
  console.error(dim('  判不了的判据不许亮绿灯——工作树行尾只能由 git 回答。'))
  process.exit(1)
}
const attrs = attrsOf(files)
if (attrs === null) {
  console.error(red('eol-audit 无法运行：`git check-attr` 失败'))
  process.exit(1)
}
/** 入库 blob 的行尾（`git ls-files --eol` 的 `i/<eol>` 列，只有它看得见「已经提交进去的 CRLF」） */
function indexEols() {
  const r = git(['ls-files', '--eol', '-z'])
  if (r.status !== 0) return null
  const out = new Map()
  for (const rec of r.stdout.toString('utf8').split('\0')) {
    if (!rec) continue
    // 形如 `i/lf    w/crlf  attr/text eol=lf\t<path>`
    const tab = rec.lastIndexOf('\t')
    if (tab < 0) continue
    const head = rec.slice(0, tab).trim().split(/\s+/)
    const iEol = head.find((t) => t.startsWith('i/'))
    if (iEol) out.set(rec.slice(tab + 1), iEol.slice(2))
  }
  return out
}
const indexEol = indexEols()
if (indexEol === null) {
  console.error(red('eol-audit 无法运行：`git ls-files --eol` 失败'))
  process.exit(1)
}

const undeclared = []
const worktreeCrlf = []
const indexCrlf = []
let textFiles = 0

for (const rel of files) {
  const abs = path.join(ROOT, rel)
  if (looksBinary(abs)) continue
  textFiles += 1
  const a = attrs.get(rel) ?? {}
  // EOL-002：没有行尾声明。放在最前面——它解释另外两条为什么可能「判不到」：
  // 一个从没进过声明网的路径，`eol=lf` 的话对它与对 git 都不成立。
  if (!EOL_OK.has(a.eol)) {
    undeclared.push(rel)
    continue
  }
  // EOL-001：声明了 LF，工作树却带 `\r`
  if (worktreeHasCr(abs)) worktreeCrlf.push(rel)
  // EOL-003：入库 blob 本身带 `\r`（声明之后 git 会自动归一化，所以这条只在
  // 「先提交、后声明」的路径上出现——正是 `.gitattributes` 头注释写的那次事故形状）
  const i = indexEol.get(rel)
  if (i === 'crlf' || i === 'mixed') indexCrlf.push(rel)
}

function report(list, limit = SHOW_ALL ? list.length : 20) {
  for (const rel of list.slice(0, limit)) console.log(`  ✗ ${rel}`)
  if (list.length > limit) console.log(dim(`  … 另有 ${list.length - limit} 条（--list 全列）`))
}

console.log(
  `EOL-001 工作树行尾：${textFiles} 个文本文件（受版本控制 ${files.length} 个）· 带 \\r ${worktreeCrlf.length} 个`,
)
if (worktreeCrlf.length) report(worktreeCrlf)

console.log(`EOL-002 行尾声明：无 \`eol\` 声明 ${undeclared.length} 个`)
if (undeclared.length) report(undeclared)

console.log(`EOL-003 入库 blob：带 \\r ${indexCrlf.length} 个`)
if (indexCrlf.length) report(indexCrlf)

if (worktreeCrlf.length || undeclared.length || indexCrlf.length) {
  console.log('')
  if (undeclared.length) {
    console.log('EOL-002 提示：`eol` 声明是「这条路径的行尾归谁管」的唯一答案。新文件类型不进')
    console.log('      `.gitattributes`，行尾就从「被规定」退回「看谁先写」——而没有任何东西会红。')
    console.log('      处置：在 `.gitattributes` 补一条（或让 `* text=auto eol=lf` 这条默认接住它）。')
  }
  if (worktreeCrlf.length) {
    console.log('EOL-001 提示：工作树的 CRLF 不显示在任何 diff 里（attr 在 diff 时归一化掉了），')
    console.log('      受害的是按行读它的解析器：`^…$` 锚定行尾的正则在 CRLF 上整段抽不到，')
    console.log('      而报错会落在下游（「表里没写」），不在读取器。')
    console.log(dim('      处置（工作区干净时）：`git checkout-index -a -f` —— 按 attr 重写工作树，'))
    console.log(dim('      index 不变、diff 为零；有未提交改动时请逐文件重写，别整仓 checkout。'))
  }
  if (indexCrlf.length) {
    console.log('EOL-003 提示：入库的 CRLF 让 diff 不可审阅（改 65 行显示成 1265 行）。')
    console.log(dim('      处置：`git add --renormalize .` 后提交（blob 归一化，内容不变）。'))
  }
  console.log(red(`\neol-audit 未通过：工作树 ${worktreeCrlf.length} · 未声明 ${undeclared.length} · 入库 ${indexCrlf.length}`))
  process.exit(1)
}

console.log(green(`eol-audit 通过：${textFiles} 个文本文件行尾一致（声明 lf · 工作树 lf · 入库 lf）`))
