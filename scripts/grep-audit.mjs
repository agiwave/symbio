#!/usr/bin/env node
/**
 * grep-audit — 静态审计检查脚本
 *
 * ⚠️ 文件头曾写着「对应 PLAN §S-4」，而那份 PLAN **已不在仓库里**
 * （`docs/archive/` 下的三份计划文档里都没有 `S-*` 编号）。规则编号由此**继承**自
 * 一份找不回来的清单，所以 S-001 / S-003..S-006 是**跳号，不是被删的规则**——
 * 别去"补号"，也别以为那里漏了检查。
 *
 * 用途：拦截常见异步/同步错误模式，防止 v27-v28 修复过的 bug 复发
 *   - S-002:        std::sync::Mutex 在 async 上下文中持锁跨 await
 *   - S-002-bonus:  业务路径 `let _ = ...await` 吞错
 *   - S-007:        （已废除）CHANGELOG 维护检查 —— 仓库不再维护 `docs/CHANGELOG.md`，
 *                   变更历史以 `git log` 为准（比手抄的一份文件更准确）。
 *                   **编号保留空缺，不补号**（见上文）。
 *   - S-008:        VdfsNode.status 用裸字面量赋值（词表只有 `VDFS_STATUS_*`）
 *   - S-009:        事件总线 kind 用裸字面量（词表只有 `KIND_*`）
 *   - S-010:        vdfs 挂载根名字面量不得出现在 vdfs 插件之外（仓级）
 *
 * 用法：
 *   node scripts/grep-audit.mjs            # 审计 symbio/src/plugins（全部插件）
 *   node scripts/grep-audit.mjs --strict   # 严格模式：warning 也算失败
 *   SCOPE=<dir> node scripts/grep-audit.mjs
 *
 * 退出码：
 *   0 = 全部通过
 *   1 = 发现 ERROR（必须修复）
 *   2 = 仅 WARNING（建议修复，--strict 才会失败）
 *
 * 由 scripts/grep_audit.sh 迁移而来：纯 Node 实现，**不依赖 bash / ripgrep**，
 * Windows / macOS / Linux 通用（仓库约定：脚本一律平台无关）。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const cwd = process.cwd()

const STRICT = process.argv.includes('--strict')

let errors = 0
let warnings = 0

const err = (m) => {
  console.log(`${red('[ERROR]')} ${m}`)
  errors++
}
const warn = (m) => {
  console.log(`${yellow('[WARN] ')} ${m}`)
  warnings++
}
const ok = (m) => console.log(`${green('[OK]   ')} ${m}`)

// ── SCOPE 推断 ─────────────────────────────────────────────────────────
// 环境变量 > cwd 相对 > 仓库根相对 > 默认值
const isDir = (p) => {
  try {
    return fs.statSync(p).isDirectory()
  } catch {
    return false
  }
}

function resolveScope() {
  if (process.env.SCOPE) return path.resolve(cwd, process.env.SCOPE)
  for (const p of [
    path.resolve(cwd, 'src/plugins'),
    path.resolve(cwd, 'symbio/src/plugins'),
    path.resolve(repoRoot, 'symbio/src/plugins'),
  ]) {
    if (isDir(p)) return p
  }
  return path.resolve(cwd, 'src/plugins')
}

const scopeAbs = resolveScope()
/** 显示用路径：相对 cwd，并统一为正斜杠（跨平台一致） */
const disp = (p) => path.relative(cwd, p).split(path.sep).join('/') || '.'

// ── 收集 .rs 源文件 ────────────────────────────────────────────────────
function walk(dir) {
  const out = []
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) out.push(...walk(p))
    else if (e.isFile() && e.name.endsWith('.rs')) out.push(p)
  }
  return out
}

const files = walk(scopeAbs)
const linesOf = new Map()
for (const f of files) {
  try {
    linesOf.set(f, fs.readFileSync(f, 'utf8').split(/\r?\n/))
  } catch {
    /* 读不到就跳过 */
  }
}

console.log('=== grep-audit.mjs ===')
console.log(`Scope: ${disp(scopeAbs)}`)
console.log(`Strict: ${STRICT}`)
console.log()

// ── S-002: std::sync::Mutex + .lock() 跨 await ─────────────────────────
// 启发式：某行出现 .lock()，且同文件后续 5 行内出现 .await，
//         且该文件引用了 std::sync::Mutex → ERROR
console.log('--- S-002: std::sync::Mutex 跨 await 检查 ---')

if (files.length === 0) {
  warn(`scope 不存在或无 .rs 文件（跳过 S-002 检查）：${disp(scopeAbs)}`)
} else {
  let asyncCount = 0
  for (const lines of linesOf.values()) {
    for (const l of lines) if (/^\s*(pub\s+)?async\s+fn\s+\w+/.test(l)) asyncCount++
  }

  if (asyncCount === 0) {
    ok('未发现 async fn（跳过 S-002 检查）')
  } else {
    let bad = 0
    for (const [file, lines] of linesOf) {
      if (!/use std::sync::Mutex|std::sync::Mutex</.test(lines.join('\n'))) continue

      const lockLines = []
      const awaitLines = []
      lines.forEach((l, i) => {
        const n = i + 1
        if (/\.lock\(\)/.test(l)) lockLines.push(n)
        if (/\.await\b/.test(l)) awaitLines.push(n)
      })
      if (lockLines.length === 0 || awaitLines.length === 0) continue

      for (const ln of lockLines) {
        // 仅允许带理由的行级人工豁免；测试代码同样参与检查。
        if (/\/\/\s*grep-audit-allow S-002:\s*\S/.test(lines[ln - 1])) continue
        if (awaitLines.some((a) => a > ln && a <= ln + 5)) {
          err(
            `${disp(file)}:${ln}  std::sync::Mutex 持锁后 5 行内出现 .await` +
              `（潜在跨 await 持锁，请改用 tokio::sync::Mutex）`,
          )
          bad++
        }
      }
    }
    if (bad === 0) ok('S-002 通过：未发现 std::sync::Mutex 跨 await 持锁')
  }
}
console.log()

// ── S-002-bonus: 业务路径 let _ = ...await ─────────────────────────────
// 命中行若位于 #[test] / #[tokio::test] / fn test_ / mod tests 之后 200 行内，
// 视为测试代码并跳过。
//
// ⚠️ 本规则**只能靠正则近似**，故必然有假阳性，两类都真实存在：
//   · 被 await 的 future **本就不返回 Result**（如 `fire_hook` 返回 `HookOutput`，
//     它自己内部已把路由失败吞成默认值）——`let _ =` 丢的不是错误；
//   · 「关闭 / 清理 / kill / flush」这类**收尾动作**，失败时本就没有后续动作可做
//     （对端已消失、文件可能本就不存在、子进程可能已退出）。
// 正则分不清「吞了错误」与「本就无错误可吞」。
//
// 因此与 S-002 / S-008 / S-009 / S-010 一致，留一条**必须写理由**的逐行豁免：
// `// grep-audit-allow S-002-bonus: 理由`。
// 为什么必须有这条通道：本规则输出的是「请人工 review」——**review 完没地方写结论，
// 就等于每次跑门禁都要从头再 review 一遍**。永不消失的告警不会让人更谨慎，只会教人
// 忽略整个审计。留痕豁免把「已 review 且判定为刻意」这件事固化成一次性的。
console.log('--- S-002-bonus: 业务路径 let _ = ...await 检查 ---')

const SUSPECT_RE = /let _ = .*\.await/
const TEST_MARKER_RE = /#\[(tokio::)?test\]|fn test_|mod tests/
// 理由必须含**至少一个字母 / 数字 / 汉字**（与 S-010 同口径：空理由不算豁免）
const WAIVER_S002B_RE = /\/\/\s*grep-audit-allow S-002-bonus:[^\n]*[A-Za-z0-9\u4e00-\u9fff]/
// 注释行（含 `///` 文档注释）不算命中：本规则判的是**代码形状**，而注释里写
// 「这里为什么可以 `let _ = ...await`」恰恰是在解释它——把解释也判成违规，等于
// 惩罚留痕。本文件这次自查就踩到了：三处新增的说明性注释各报了一条假阳性。
const isCommentLine = (l) => l.trimStart().startsWith('//')

const suspects = []
for (const [file, lines] of linesOf) {
  const markers = []
  const hits = []
  lines.forEach((l, i) => {
    const n = i + 1
    if (TEST_MARKER_RE.test(l)) markers.push(n)
    if (SUSPECT_RE.test(l) && !isCommentLine(l) && !l.includes('.tx.send')) hits.push(n)
  })
  for (const h of hits) {
    if (WAIVER_S002B_RE.test(lines[h - 1])) continue
    // 同一行可能既是 marker 又是命中，此时 markers 含 h 自身，需排除 m === h
    const inTest = markers.some((m) => m < h && h - m <= 200)
    if (!inTest) suspects.push(`${disp(file)}:${h}:${lines[h - 1].trim()}`)
  }
}

if (suspects.length > 0) {
  for (const s of suspects) {
    warn(`${s}  业务路径疑似吞错，请人工 review（应改为 if let Err(e) = ... + plugin_warn）`)
  }
} else {
  ok('S-002-bonus 通过：业务路径无新增 let _ = ...await')
}
console.log()

// ── S-007: CHANGELOG 维护检查 —— **已废除** ────────────────────────────
//
// 原规则：要求 `docs/CHANGELOG.md` 存在且有 `## vNN` / `## YYYY-MM-DD` 标题。
// 废除理由：git 本身**就是**变更历史，且比手抄的一份文件更准确——不会漏、
// 不会与代码漂移、也不必维护第二份同样的信息。那份文件涨到 4384 行之后，
// 作为「当前状态」的参考资料几乎只剩噪音（检索成本高、上下文开销大），
// 而它承载的历史在 `git log` 里一条不少。
//
// 替代：提交信息。本仓库的提交消息本就有严格格式（`<type>(<scope>): 标题`
// + 编号分节 + 「门禁：」段，见 `scripts/check-commit-msg.mjs`），
// 它比一条 CHANGELOG 条目更结构化，且**与代码同一次提交**，不可能漂移。
// 编号保留空缺（`S-007` 不再补号），见本文件头部说明。

// ── S-008: VdfsNode.status 不得用裸字面量 ──────────────────────────────
//
// 词表只有一套：`symbio_core::vdfs_provider::VDFS_STATUS_*`（`docs/design/vdfs.md`
// §3.2）。裸字面量的危险不是拼错（那会立刻看见），而是**改名时不会编译失败**——
// `status` 是跨进程边界的字符串，改了常量而漏掉字面量，前端只会静默认不出状态。
//
// 该词表**曾经**把「以错误结束」从 `error` 改名为 `failed`（理由见
// `VDFS_STATUS_FAILED` 的文档：与消息层 `MessageStatus::Failed` 同词），而
// `symbio_core::schemas::options.rs`（**该文件已于 2026-09-23 随会话选项 schema 化
// 删除**）里留了一枚 `OPTION_STATUS_ERROR = "error"` 的化石、`mcp` / `skill` /
// `model` 三个插件各自手写 `"unknown"` / `"active"` / `"disabled"`——共 6 处。
// 三处都**没有任何测试会因此变红**，故立此规则。
//
// 判据（只认两处无歧义的位置，宁可漏报）：
//   · `.status = "<字面量>"`，含 `if` / `match` 分支里写字面量的形态；
//   · `with_status("<字面量>")`。
// **不**认结构体字面量的 `status: "…"`——响应信封（`SimpleResponse`）的
// `status: "success"` 是另一套词表，按位置判会误报，故留作已知边界。
console.log('--- S-008: VdfsNode.status 字面量检查 ---')

const STATUS_ASSIGN_RE = /\.status\s*=\s*([^;]{0,240});/g
const STATUS_SETTER_RE = /with_status\(\s*"/
const WAIVER_S008_RE = /\/\/\s*grep-audit-allow S-008:\s*\S/

let s008 = 0
for (const [file, lines] of linesOf) {
  const text = lines.join('\n')
  for (const m of text.matchAll(STATUS_ASSIGN_RE)) {
    const rhs = m[1].trim()
    // 只在「值位置直接是字面量」时判：`= "x"` 或 `= if/match … { "x" … }`
    const bad = rhs.startsWith('"') || (/^(if|match)\b/.test(rhs) && rhs.includes('"'))
    if (!bad) continue
    const ln = text.slice(0, m.index).split('\n').length
    if (WAIVER_S008_RE.test(lines[ln - 1])) continue
    err(`${disp(file)}:${ln}  status 用裸字面量赋值；改引 VDFS_STATUS_* 常量（改名才不会静默失效）`)
    s008++
  }
  lines.forEach((l, i) => {
    if (!STATUS_SETTER_RE.test(l) || WAIVER_S008_RE.test(l)) return
    err(`${disp(file)}:${i + 1}  with_status 收到裸字面量；改引 VDFS_STATUS_* 常量`)
    s008++
  })
}
if (s008 === 0) ok('S-008 通过：status 一律取自 VDFS_STATUS_* 常量')
console.log()

// ── S-009: 事件总线 kind 不得用裸字面量 ────────────────────────────────
//
// 词表只有一套：`symbio_core::event_bus::KIND_*`（文档见
// `docs/architecture/PROTOCOLS.md` §事件总线频道）。与 S-008 同源：`kind` 是
// **跨进程**字符串，改名不会编译失败，只会让消费方的「按 kind 分派」静默失效。
//
// 为什么单独立一条：真实事故就是「发布点写裸字面量 → 常量声明出来却无人引用」，
// 被 `dead-code-audit` 的 R-001 当成死代码报了出来（`KIND_SESSION` / `KIND_SYSTEM`
// 的家史）。而 R-001 只在常量**全仓零引用**时才响——只要别处引用过一次，写裸字面量
// 的发布点就再也无人发现。故按位置立此规则。
//
// 判据（只认 `publish` / `try_publish` 首参直接是字面量的形态，宁可漏报）：
//   · `EventBus::publish("<字面量>"` / `EventBus::try_publish("<字面量>"`
//   · `<receiver>.publish("<字面量>"` / `.try_publish("<字面量>"`
console.log('--- S-009: 事件总线 kind 字面量检查 ---')

const KIND_LITERAL_RE = /(?:try_)?publish\(\s*"/
const WAIVER_S009_RE = /\/\/\s*grep-audit-allow S-009:\s*\S/

let s009 = 0
for (const [file, lines] of linesOf) {
  lines.forEach((l, i) => {
    if (!KIND_LITERAL_RE.test(l) || WAIVER_S009_RE.test(l)) return
    err(
      `${disp(file)}:${i + 1}  publish 首参是裸 kind 字面量；` +
        `改引 KIND_* 常量（kind 是跨进程字符串，改名不会编译失败）`,
    )
    s009++
  })
}
if (s009 === 0) ok('S-009 通过：事件 kind 一律取自 KIND_* 常量')
console.log()

// ── S-010: vdfs 挂载根名不得出现在 vdfs 插件之外 ───────────────────────
//
// 根名是 vdfs 插件自己的挂载规则（`plugins/vdfs/fs.rs::VDFS_ADDR_ROOT`，
// 全仓唯一的字面量）。历史上它曾以 `.vdfs` 的形态蔓延到全系统——前端路径代数、
// 提示词地址、十几份文档——导致「改个挂载名」变成全仓手术。现机制已收口：
// 后端插件经 `AddrRootDecl` 静态声明、父地址经上下文传递（`VDFS_PARENT_ADDR`）、
// 前端启动期经 `vdfs/root` 拿根地址当运行期数据（`schemas/vdfsRoot`）。
// 本规则把收口**钉死**：插件之外再出现根名字面量即违规。
//
// 判据（按 token 匹配，宁可误报到注释也不放过地址字面量）：
//   · `.vdfs` 裸名 / `.vdfs/…`（地址） / 带版本后缀的变体（`.vdfsv2`、`.vdfs2`）。
//     **连字符后缀不判**（`.vdfs-card` 这类 CSS 类名是另一码事），
//     其它字母数字后缀（`.vdfsx`）同样不判——按形状判根名，不猜意图。
// 范围：仓内全部 .rs / .ts / .vue / .md（`scripts/` 工具自身除外）。
// 豁免（写在规则里，逐条留痕）：
//   · `symbio/src/plugins/vdfs/**` —— 根名的所有者，字面量只允许在这里；
//   · `docs/archive/**` —— 历史记录不改写，
//     改写等于伪造当时的代码状态；
//   · 本文件（审计脚本自己要描述这条规则）。
// 逐行豁免：`grep-audit-allow S-010: 理由`（本行或紧邻上一行，理由不可为空）。
console.log('--- S-010: vdfs 挂载根名归属检查 ---')

const S010_TOKEN_RE = /(?<![A-Za-z0-9_.])\.vdfs(?:v?\d+)?(?![A-Za-z0-9_-])/g
// 理由必须含**至少一个字母 / 数字 / 汉字**：markdown 里 `<!-- ... :   -->` 的
// 注释终止符 `-->` 不能充当理由（空理由视为未豁免的约定不能被它绕过）。
const WAIVER_S010_RE = /grep-audit-allow S-010:[^\n]*[A-Za-z0-9\u4e00-\u9fff]/
const S010_EXTS = new Set(['.rs', '.ts', '.vue', '.md'])
const S010_SKIP_DIRS = new Set(['node_modules', 'target', 'dist', '.git', '.workbuddy', '.workbuddy-ai', '.venv'])
// 与 resolveScope 同策略：cwd 是仓库树就用 cwd（回归测试注入临时树），否则退回仓库根
const S010_ROOT = isDir(path.resolve(cwd, 'symbio')) ? cwd : repoRoot

function walkS010(dir) {
  const out = []
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) {
      if (S010_SKIP_DIRS.has(e.name)) continue
      out.push(...walkS010(p))
    } else if (e.isFile() && S010_EXTS.has(path.extname(e.name))) {
      out.push(p)
    }
  }
  return out
}

const s010Self = fileURLToPath(import.meta.url)
const s010Files = walkS010(S010_ROOT).filter((f) => {
  if (path.resolve(f) === path.resolve(s010Self)) return false
  const rel = path.relative(S010_ROOT, f).split(path.sep).join('/')
  if (rel.startsWith('symbio/src/plugins/vdfs/')) return false // 所有者
  if (rel.startsWith('docs/archive/')) return false // 历史
  if (rel.startsWith('cli/target/') || rel.startsWith('tauri/target/')) return false // 构建产物
  return true
})

let s010 = 0
for (const file of s010Files) {
  let lines
  try {
    lines = fs.readFileSync(file, 'utf8').split(/\r?\n/)
  } catch {
    continue
  }
  lines.forEach((l, i) => {
    if (!S010_TOKEN_RE.test(l)) {
      S010_TOKEN_RE.lastIndex = 0
      return
    }
    S010_TOKEN_RE.lastIndex = 0
    if (WAIVER_S010_RE.test(l) || (i > 0 && WAIVER_S010_RE.test(lines[i - 1]))) return
    err(
      `${disp(file)}:${i + 1}  出现 vdfs 挂载根名字面量；根名只归 vdfs 插件` +
        `（plugins/vdfs/fs.rs），消费方用运行期数据（后端 VDFS_PARENT_ADDR / 前端 vdfsRoot 锚点）`,
    )
    s010++
  })
}
if (s010 === 0) ok('S-010 通过：根名字面量只在 vdfs 插件内')
console.log()

// ── 汇总 ───────────────────────────────────────────────────────────────
console.log('=== 汇总 ===')
console.log(`Errors:   ${errors}`)
console.log(`Warnings: ${warnings}`)

if (errors > 0) process.exit(1)
if (STRICT && warnings > 0) process.exit(2)
process.exit(0)
