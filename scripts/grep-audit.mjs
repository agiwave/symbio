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
 *   doc-link-allow D-010: 下行 `docs/CHANGELOG.md` 是已废除规则的规则源（史实指认）
 *   - S-007:        （已废除）CHANGELOG 维护检查 —— 仓库不再维护 `docs/CHANGELOG.md`，
 *                   变更历史以 `git log` 为准（比手抄的一份文件更准确）。
 *                   **编号保留空缺，不补号**（见上文）。
 *   - S-008:        VdfsNode.status 用裸字面量赋值（词表只有 `VDFS_STATUS_*`）
 *   - S-009:        事件总线 kind 用裸字面量（词表只有 `KIND_*`）
 *   - S-010:        vdfs 挂载根名字面量不得出现在 vdfs 插件之外（仓级）
 *   - S-011:        补充注入点的位置（两处行序断言：`turn_runner.rs` 的结果/注入次序、
 *                   `turn_exec.rs` 的抽干/汇报次序；位置错了不会有任何测试变红）
 *   - S-012:        代码注释不得写只有实施排期含义的话（待办标记 / 无出处的批次号）
 *   - S-013:        `WalkBuilder` 构造点必须钉遍历序（默认序是文件系统返回序 ⇒
 *                   分页窗口与截断点随平台变；NTFS 名字序 / ext4 哈希序）
 *   - S-014:        ChatNode 存量 `role` 格的直写只许降（棘轮；身份写 `principal`，
 *                   读走 `node_role()` / `legacy_role()`，字段已退出 wire）
 *   - S-015:        收束闸只有一个 owner（`closure_gate`），且每次调用当场处理 `Err`
 *                   （布尔壳 `closure_granted` 零出现）——否则「拒绝」又变成「没写」，
 *                   日志里与「没跑完」同形
 *   - S-016:        停机原因只有一个形状（`terminated.reason` / 停机格的 `why` 不许回来）
 *   - S-017:        恢复路径不许再有旁路（`resume.rs` 与 `resume_anchor` 零出现）——
 *                   唤醒 / 续写只能从事实进环
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

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns S 源码 grep 形态
// @codes S-001 S-002 S-003 S-006 S-007 S-008 S-009 S-010 S-011 S-012 S-013 S-014 S-015 S-016 S-017

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
// doc-link-allow D-010: 下行是被废除规则的原文（该文件已随规则一起删除）
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
// 词表只有一套：`symbio_core::vdfs::VDFS_STATUS_*`（`docs/003-design/009-VDFS规范.md`
// §3.2）。裸字面量的危险不是拼错（那会立刻看见），而是**改名时不会编译失败**——
// `status` 是跨进程边界的字符串，改了常量而漏掉字面量，前端只会静默认不出状态。
//
// 该词表**曾经**把「以错误结束」从 `error` 改名为 `failed`（理由见
// `VDFS_STATUS_FAILED` 的文档：与消息层 `NodeStatus::Failed` 同词），而
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
// `docs/003-design/005-协议规范.md` §事件总线频道）。与 S-008 同源：`kind` 是
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
// 范围：仓内全部 .rs / .ts / .vue / .md（`scripts/` 工具自身除外），
//       **不含**构建产物与运行期数据目录（`.symbio` 等，见 `S010_SKIP_DIRS`）——
//       审计对象是源码树；运行期数据不在版本控制里，模型往里写什么与规则无关。
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
// 跳过的是**非源码树**：构建产物 + 依赖 + 运行期数据目录。
// `.symbio` 是运行期 homedir（插件树落在磁盘上的那份，见 `DEFAULT_HOMEDIR`），
// 已被 `.gitignore` 忽略——**它不在版本控制里，也就不是本规则的对象**。
// 漏掉它会让规则去审计「agent 自己写下的记忆」：实测一次会话把 `.vdfs` 字面量
// 写进 `.symbio/session/<id>/MEMORY.md`（模型在正文里引用挂载路径），
// 门禁于是因为**运行期数据**判红——源码一个字节没改。
// `tmp` 同理：`.gitignore` 的临时区（报告草稿 / 迁移备份的落点），不在版本控制里，
// 与登记机制的 `SCAN_SKIP_DIRS` 同一口径——分析稿里引用挂载路径不算源码树的违规。
const S010_SKIP_DIRS = new Set([
  'node_modules',
  'target',
  'dist',
  '.git',
  '.workbuddy',
  '.workbuddy-ai',
  '.symbio',
  '.venv',
  'tmp',
])
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

// ── S-011: 补充注入点必须在「工具结果已接进请求」之后、汇报判定之前 ──────
//
// 抽干点在 v2 搬了家（5b-2：改到运行器的批次边界，见 D-074），约束一条没少，
// 而且**违反时依然没有测试会变红**——所以判据换成搬家后的**两处行序断言**：
//   ① `turn_runner.rs`：2f 的 `inject` 必须排在 2e 的 `tool_exchange_messages` **之后**。
//      反过来写 = 用户那句补充插进 `assistant(tool_calls)` 与它的 `tool` 结果之间——
//      部分协议直接 400（三种错法里唯一会响的那种），而且时间序颠倒（工具结果先发生，
//      用户随后才插话）。
//   ② `turn_exec.rs`：抽干（`supplement_into_request`）必须排在汇报判定（`report`）**之前**。
//      这条钉的是「先抽干、后判定」——`supplemented_at` 靠抽干结果才拿得到，顺序反过来
//      等于用户已经开口了、插件还在按「没人说话」报进度（静默时钟没归零）。
// 两个锚点**任一找不到即 ERROR**：找不到不等于"无需检查"——那说明注入点被删了
// （那就该连本规则与文档一起改）或被改名了（那本规则必须同步）。**让守卫悄悄退化成
// 空转，比没有守卫更糟**（它会以绿灯的形式提供虚假保证）。
// 这也是为什么"锚点缺失"**不接受豁免**：豁免的前提是"我看过并确认没问题"，
// 而锚点缺失时根本无从判断。
//
// 豁免：`// grep-audit-allow S-011: 理由`（写在**后一个锚点**那一行，理由不可为空）——
// 只在"位置确实不对但已确认可接受"时用。
// 文档侧的定义见 `symbio/src/plugins/classify/README.md` §5 S0 的 C-D2b。
console.log('--- S-011: 补充注入点位置检查 ---')

const WAIVER_S011_RE = /\/\/\s*grep-audit-allow S-011:[^\n]*[A-Za-z0-9\u4e00-\u9fff]/

const S011_PAIRS = [
  {
    file: 'plugins/session/turn_runner.rs',
    before: {
      re: /exchange\.extend\(\s*tool_exchange_messages\(/,
      name: '2e 工具结果接进请求',
    },
    after: {
      re: /exchange\.extend\(\s*inject\(/,
      name: '2f 轮内补充接进请求',
    },
    why: '补充必须排在工具结果**之后**（插进 `assistant(tool_calls)` 与它的 `tool` 结果之间是协议级 400，且时间序颠倒）',
  },
  {
    file: 'plugins/session/turn_exec.rs',
    before: {
      re: /supplement_into_request\(/,
      ex: /\bfn\s+supplement_into_request\(/,
      name: '`supplement_into_request` 抽干点',
    },
    after: { re: /\breport\(/, name: '`report` 汇报判定' },
    why: '必须先抽干再判定：`supplemented_at`（抽到补充 ⇒ 静默时钟归零）只有抽干之后才知道，反过来等于用户已开口、插件仍按"没人说话"报进度',
  },
]

for (const pair of S011_PAIRS) {
  const entry = [...linesOf.entries()].find(([f]) =>
    f.split(path.sep).join('/').endsWith(pair.file),
  )
  if (!entry) {
    ok(`S-011 跳过：scope 内没有 ${pair.file}`)
    continue
  }
  const [s011File, s011Lines] = entry
  // 只认代码行：这两个文件里各有一大段解释"为什么放在这里"的注释，注释里提到锚点
  // 不等于调用在这里。**注释掉注入调用**会被判成"锚点缺失"（响亮），
  // 这正是想要的——注释掉一个守卫点，不该得到绿灯。
  const firstLine = (re, exclude) => {
    for (let i = 0; i < s011Lines.length; i++) {
      const l = s011Lines[i]
      if (l.trimStart().startsWith('//')) continue
      if (!re.test(l)) continue
      if (exclude && exclude.test(l)) continue
      return i + 1
    }
    return null
  }

  const beforeAt = firstLine(pair.before.re, pair.before.ex)
  const afterAt = firstLine(pair.after.re, pair.after.ex)
  const rel = disp(s011File)

  const missing = []
  if (beforeAt == null) missing.push(pair.before.name)
  if (afterAt == null) missing.push(pair.after.name)

  if (missing.length > 0) {
    err(
      `${rel}  S-011 锚点缺失：${missing.join(' / ')}——注入点被删除或改名了？` +
        `守卫不得悄悄退化成空转：同步更新本规则与 PL-007-对话面插件拆分实施方案 §5 S0 的 C-D2b`,
    )
  } else if (!(beforeAt < afterAt)) {
    if (WAIVER_S011_RE.test(s011Lines[afterAt - 1])) {
      ok(`${rel}  S-011 通过（逐行豁免：位置已确认可接受）`)
    } else {
      err(
        `${rel}  S-011 位置错（${pair.before.name}@${beforeAt} → ${pair.after.name}@${afterAt}）：` +
          pair.why,
      )
    }
  } else {
    ok(
      `S-011 通过：${rel} ${pair.before.name}(${beforeAt}) 在 ${pair.after.name}(${afterAt}) 之前`,
    )
  }
}
console.log()

// ── S-012: 代码注释不得写只有实施排期含义的话 ──────────────────────────
//
// 注释的边界（`CONTRIBUTING.md` §4）：写「读这个文件需要知道的不变量 / 陷阱 / 为什么」，
// **不写过程**。过程叙述的危害不是啰嗦，而是**它会变成假的**：「本批未修」的下一批
// 修了，那句注释就躺在代码里指着一个已不存在的时间点；「批 2 的能力」在批次重排后
// 无人能解。这类句子没有 owner，也就没人回头改它——而读者只能选择信或不信。
//
// 判据只取**两种无歧义形态**（同 D-004「只收结构标记」的取舍）：
//   ① 待办标记：注释行以 `TODO` / `FIXME` / `XXX` / `HACK` 起头——待办有自己的
// 登记处（`docs/007-task/` 下登记条目：引入新待办就登记成条目，
//      **不是代码注释**），写在注释里就等于「谁也不会再看见」的那一类；
//   ② 无出处的批次号：`批 N` / `批次 N` 且**同一行没有 `plan/NN`**。批次号属实施
//      计划，单独一个数字不知道是哪份计划的哪一批；带出处就是合法指认
//      （仓内既有的 `[plan/11 批 2 ③](…)` 那种写法全部通过）。
//
// **已知边界（刻意不判）**：`本批` / `诚实缺口` / `曾经` 在注释里都可能是**运行期语义**
// 或**陷阱说明**——「本批未执行的工具调用」说的是这一批工具调用（`tool_executor` 的
// 收口不变量），as-of 投影里「这条目标曾经声明过吗」是查询语义。实测把它们列进词表：
// `本批|待接|尚未接` 在 `symbio/src` 命中 50 处、绝大多数是运行期 prose，`曾经` 命中
// 59 处含真误报——**一条天天误报的守卫最后只会被人用豁免喂死**（本文件反复写的那条）。
// 范围：`.rs` 的注释行（`symbio/src`、`cli/src`、`tauri/src-tauri/src`、`docs/003-design/verify`），
// 测试文件同样参与检查（与 S-002 一致）。
// 豁免：`// grep-audit-allow S-012: 理由`（写在命中行或其紧邻上一行，理由不可为空，同 S-010）。
console.log('--- S-012: 代码注释里的实施排期指认 ---')

const S012_ROOTS = ['symbio/src', 'cli/src', 'tauri/src-tauri/src', 'docs/003-design/verify']
/** 与 resolveScope 同策略：cwd 是仓库树就用 cwd（回归测试注入临时树），否则退回仓库根 */
const S012_BASE = isDir(path.resolve(cwd, 'symbio')) ? cwd : repoRoot
const S012_TODO_RE = /^\s*(?:\/\/[/!]*)\s*(?:TODO|FIXME|XXX|HACK)\b/
const S012_BATCH_RE = /批(?:次)?\s*\d/
const S012_ANCHOR_RE = /plan\/\d+/
const WAIVER_S012_RE = /\/\/\s*grep-audit-allow S-012:[^\n]*[A-Za-z0-9\u4e00-\u9fff]/

{
  const s012Files = S012_ROOTS.flatMap((r) => walk(path.join(S012_BASE, r)))
  let s012 = 0
  for (const file of s012Files) {
    let lines
    try {
      lines = fs.readFileSync(file, 'utf8').split(/\r?\n/)
    } catch {
      continue
    }
    lines.forEach((l, i) => {
      if (!/^\s*\/\//.test(l)) return
      if (WAIVER_S012_RE.test(l) || (i > 0 && WAIVER_S012_RE.test(lines[i - 1]))) return
      const why = S012_TODO_RE.test(l)
        ? '注释以 TODO / FIXME 起头 ⇒ 待办登记进计划文档，不写进代码注释'
        : S012_BATCH_RE.test(l) && !S012_ANCHOR_RE.test(l)
          ? '批次号没有出处 ⇒ 写成 `plan/NN 批 M`（单独的「批 N」指不出是哪份计划的第几批）'
          : null
      if (!why) return
      err(`${disp(file)}:${i + 1}  ${why}`)
      s012++
    })
  }
  if (s012 === 0) ok('S-012 通过：注释里没有待办标记与无出处的批次号')
}
console.log()

// ── S-013: `WalkBuilder` 构造点必须钉遍历序 ────────────────────────────
//
// `ignore::WalkBuilder` 不配排序时按**文件系统返回的目录项序**遍历：NTFS 上恰是名字序、
// ext4 上是哈希序。于是同一份代码、同一个查询在两个平台给出行序不同的结果，而下游三处
// 判定都切在这个序上——`content_search` 的 `offset` / `head_limit` 分页窗口、
// `MAX_RESULTS` 截断点、`codebase_search` 降级路径的 `limit` 截断。
// 病灶实例（①）：两条分页单测 Windows 绿、Linux 红，红的是症状，缺陷在实现侧
// （用户看到的是「换台机器同一查询少几行 / 换一批行」）。这类「判据依赖平台」的病
// 在本仓已有两次实例（是 shell 工具名，这次是路径序），所以钉在门内而不是靠人记得。
//
// 判据：`WalkBuilder::new(` 之后，**同一函数体内**（往后扫到下一个 `fn` 起头为止）
// 必须出现一次 `.sort_by(` / `.sort_by_file_name(`。
// **已知边界（刻意）**：只认「构造点之后、同一函数体内」——把 builder 传出去在别处配排序
// 不判，同函数里两个 builder 共用一次 `sort_by` 也判绿。取这个粒度是因为三处既有代码
// 都是「构造 + 链式配置同作用域」的形状，而一条天天误报的守卫最后只会被人用豁免喂死
// （本文件反复写的那条）。跨函数传 builder 真出现时，人会看见这条注释。
// 范围：`.rs` 源码根（同 S-012：`symbio/src`、`cli/src`、`tauri/src-tauri/src`、
// `docs/003-design/verify`），测试文件同样参与。
// 豁免：`// grep-audit-allow S-013: 理由`（写在构造行或其紧邻上一行，理由不可为空，同 S-010）。
console.log('--- S-013: WalkBuilder 构造点的遍历序 ---')

const S013_ROOTS = ['symbio/src', 'cli/src', 'tauri/src-tauri/src', 'docs/003-design/verify']
const S013_BASE = isDir(path.resolve(cwd, 'symbio')) ? cwd : repoRoot
const S013_WALK_RE = /WalkBuilder::new\s*\(/
const S013_SORT_RE = /\.sort_by(?:_file_name)?\s*\(/
/** 下一个函数起头：扫到它就把上一个 builder 的作用域关掉 */
const S013_NEXT_FN_RE = /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s/
const WAIVER_S013_RE = /\/\/\s*grep-audit-allow S-013:[^\n]*[A-Za-z0-9\u4e00-\u9fff]/

{
  const s013Files = S013_ROOTS.flatMap((r) => walk(path.join(S013_BASE, r)))
  let s013 = 0
  let s013Sites = 0
  for (const file of s013Files) {
    let lines
    try {
      lines = fs.readFileSync(file, 'utf8').split(/\r?\n/)
    } catch {
      continue
    }
    lines.forEach((l, i) => {
      if (!S013_WALK_RE.test(l)) return
      s013Sites++
      if (WAIVER_S013_RE.test(l) || (i > 0 && WAIVER_S013_RE.test(lines[i - 1]))) return
      let pinned = false
      for (let j = i; j < lines.length; j++) {
        if (j > i && S013_NEXT_FN_RE.test(lines[j])) break
        if (S013_SORT_RE.test(lines[j])) {
          pinned = true
          break
        }
      }
      if (pinned) return
      err(
        `${disp(file)}:${i + 1}  S-013 未钉遍历序：默认遍历序是文件系统返回序` +
          `（NTFS 名字序 / ext4 哈希序），下游的分页窗口与截断点都切在它上面 ⇒ ` +
          '补 `.sort_by_file_name(|a, b| a.cmp(b))`，或在构造行写 `grep-audit-allow S-013: 理由`' +
          '（理由要能指认序在哪里被钉住）',
      )
      s013++
    })
  }
  if (s013 === 0)
    ok(
      s013Sites === 0
        ? 'S-013 通过：scope 内没有 WalkBuilder 构造点'
        : `S-013 通过：${s013Sites} 处 WalkBuilder 构造点都钉了遍历序（或已写豁免理由）`,
    )
}
console.log()

// ── S-014: ChatNode `role`（存量格）的直写只许降 ───────────────────────
//
// d-iii / D-147：身份的权威是 `principal.kind`——读侧一律经 `node_role()` 折叠
// （与前端 `messageRoleOf` 同一判据：**主体在先、残留格兜底**），写侧的每一处 `role`
// 直写都是待清零的存量债务。它今天还剩的唯一合法用途是「构造期声明族、地址由落库点
// `attributed` 补」，所以棘轮数的不是「错」，而是**退役还没走完的格数**。
//
// 为什么不是字段私有：`ChatNode { .., ..Default::default() }` 的**函数式更新语法
// 要求每个字段可见**（E0451）——私有化会把全仓数百个不碰 `role` 的构造点一起炸掉。
// 机制的爆炸半径大于收益，所以约束从「编译期」退到「棘轮」：存量总数钉在基线上，
// **只许降不许升**；降了要同步下调基线（`35-baseline` 守这个数不被放松）。
//
// 命中形状只剩两种（过渡入口 `with_role` / `set_legacy_role` 已随 D-147 档 3 删除，
// 全仓零命中；它们若复活，字段直写这一形态也拦不住，故由「零出现」的 grep 守）：
//   · `role: Some(NodeRole::…)` / `role: Some(cm::NodeRole::…)`（字段初始化）
//   · `.role = Some(…)` / `.role.get_or_insert(…)`（直写）
//   · `with_role(…)` / `set_legacy_role(…)`（命名过渡入口）
// 范围：`.rs` 源码根（同 S-012），测试文件同样参与；**不含** `chat_node.rs`
// （字段定义、折叠与入口的本体在那里）。注释行不判（doc 示例会复述被禁形状）。
// 豁免：`// grep-audit-allow S-014: 理由`（本行或紧邻上一行，理由不可为空，同 S-010）。
// 单测跑在临时 fixture 树里：那里没有真实仓库这么多存量债务，棘轮一比就报「基线该收紧」，
// 把每条无关用例一起拖红。`LEGACY_ROLE_WRITE_BASELINE` 把基线搬到 fixture 上；扫描根
// 由下面那条「cwd 下有 `symbio/`」的判定搬运。真实仓库两者都不动 ⇒ 照旧扫全仓。
console.log('--- S-014: ChatNode 存量 role 格直写棘轮 ---')

const S014_ROOTS = ['symbio/src', 'cli/src', 'tauri/src-tauri/src', 'docs/003-design/verify']
const S014_BASE = isDir(path.resolve(cwd, 'symbio')) ? cwd : repoRoot
// **只在真实仓库树上跑棘轮**：回归测试注入的临时树（`grep-audit.test.mjs` 的
// `audit()` / `s010Audit()` 写在 `os.tmpdir()`）即使造出 `symbio/src/...` 的几层目录，
// 存量也恒 0——与基线数字的差是「树不对」，不是「该收紧」。据此报错误把每条断言退出码
// 0 的 fixture 用例判红，守卫的测试自己成了噪音。判据用 `repoRoot`（脚本自身所在仓库）：
// 临时树在 tmpdir、真实仓库不是；从真实仓库或其子目录跑都相等。
const S014_IS_REPO = path.resolve(S014_BASE) === path.resolve(repoRoot)
const S014_WRITE_RE =
  /role:\s*Some\((?:cm::)?NodeRole::|\.role\s*=\s*Some\(|\.role\.get_or_insert\(/
const WAIVER_S014_RE = /\/\/\s*grep-audit-allow S-014:[^\n]*[A-Za-z0-9\u4e00-\u9fff]/
// 基线三元形状与 `dead-code-audit` 的 R-002 同构——`35-baseline` 的提取函数按这个形状读格。
// 151 → 146（d）：`resume.rs` 及其用例整份退役，5 处存量 role 直写随之消失。
// 146 → 143（D-147 档 1）：三处**真双写**清零——`tool_executor::build_user_prompt_message` /
// `tool_dispatch` 的工具结果 / `subagent::relabel_for_parent` 原本同处既写 `principal`
// 又写 `role`。折序改成「主体在先、残留格兜底」之后（D-147），第二份写不再有任何读者：
// 它既不影响折叠，也不是 `attributed` 的输入，却给了「顺带写反就静默改写失败」的机会。
// 143 → 0（D-147 档 2 / 档 3）：构造点全部改收主体，测试夹具按「模仿哪个面」归位；
// 剩下的只有**读边界**（协议形状 / 客户端补丁 / 帧合并 / CLI 快照）与两条**存量格读法**
// 用例，它们各自带豁免注明理由。清零之后这条棘轮数的就是「新增直写」：任何一处新写
// 都即红，不必再靠单调下调来追债。
const LEGACY_ROLE_WRITE_BASELINE = process.env.LEGACY_ROLE_WRITE_BASELINE
  ? Number(process.env.LEGACY_ROLE_WRITE_BASELINE)
  : 0
{
  let s014 = 0
  const s014List = []
  for (const file of S014_ROOTS.flatMap((r) => walk(path.join(S014_BASE, r)))) {
    if (path.basename(file) === 'chat_node.rs') continue
    let lines
    try {
      lines = fs.readFileSync(file, 'utf8').split(/\r?\n/)
    } catch {
      continue
    }
    lines.forEach((l, i) => {
      if (/^\s*(?:\/\/|\/\*|\*)/.test(l)) return
      if (!S014_WRITE_RE.test(l)) return
      if (WAIVER_S014_RE.test(l) || (i > 0 && WAIVER_S014_RE.test(lines[i - 1]))) return
      s014++
      if (s014List.length < 8) s014List.push(`${disp(file)}:${i + 1}`)
    })
  }
  if (s014 > LEGACY_ROLE_WRITE_BASELINE) {
    err(
      `S-014 违例：存量 role 格直写 ${s014} 处 > 基线 ${LEGACY_ROLE_WRITE_BASELINE}——` +
        `身份写 ` +
        '`principal`，不写 role（读走 node_role()/legacy_role()）。首批命中：' +
        s014List.join('、'),
    )
  } else if (S014_IS_REPO && s014 < LEGACY_ROLE_WRITE_BASELINE) {
    err(
      `S-014 基线该收紧：存量 role 格直写 ${s014} 处 < 基线 ${LEGACY_ROLE_WRITE_BASELINE}——` +
        `把 LEGACY_ROLE_WRITE_BASELINE 下调到 ${s014}（棘轮只许留痕地下调）`,
    )
  } else {
    ok(
      S014_IS_REPO
        ? `S-014 通过：role 格直写存量 ${s014} 处，与基线持平（只许降）`
        : `S-014 跳过：当前树不是真实仓库（临时 fixture），棘轮不比`,
    )
  }
}
console.log()

// ── S-015: 收束闸只有一个 owner，且每次调用都当场处理 Err ────────────────
// 写侧闸今天只有一个函数：`closure_gate(actor, kind) -> Result<(), String>`。
// 它把「为什么拒」带出来，交给 `deny(…)` 落一条 `authz.denied`。
//
// 否定分支若把闸压成布尔或把结果存起来另判，拒绝就又成了「什么都没写」：
// 日志里「被拒」与「根本没跑完」同形，事后答不出谁/何时/因何被拒，回放重现不出
// 这次拒绝（相应批次）。
// 这条不是风格问题——它违反 I1（单通道），而违反的形态**不会有任何东西变红**：
// 该轮照样留在未收束态，`unresolved_turns` 照样报，看起来和"正常被拒"一样。
//
// 判据是**零出现**（不是棘轮）：没有"存量"可言——存量已经全部迁完。两条形状，
// 各自都还能真的变红：
// ① 布尔壳 `closure_granted` 零出现。它是 时期 `closure_gate(..).is_ok`
// 的一行壳，随收束闸收敛到单一 owner（d 删掉它唯一的生产调用点后
//     一并删除）而退役。复活它就是复活「拒绝不落事实」的那个形态。
//   ② 闸的调用行必须**当场**处理 `Err`：不许 `.is_ok()` / `.is_err()` 取布尔，
//     也不许把结果存到别处再判（那种写法拒绝照样不落事实）。
//
// 为什么形状②不写成「不许 `.is_ok()`」就够：拆行的写法
// `let r = closure_gate(..)` + 下一行 `if r.is_ok()` 骗得过行级正则。
// 「调用行自己带 `if let Err(`」这条判据把那种写法一并挡住（拆行时按上一行同看）。
console.log('--- S-015: 收束闸单一 owner 且调用当场处理 Err ---')
{
  const S015_ROOTS = ['symbio/src/plugins']
  const S015_BASE = isDir(path.resolve(cwd, 'symbio')) ? cwd : repoRoot
  const S015_IS_REPO = path.resolve(S015_BASE) === path.resolve(repoRoot)
  const isComment = (l) => {
    const t = l.trimStart()
    return t.startsWith('//') || t.startsWith('/*') || t.startsWith('*')
  }
  const isWaived = (l) => {
    const marker = 'grep-audit-allow S-015:'
    const at = l.indexOf(marker)
    if (at < 0) return false
    // 理由必须含**至少一个字母 / 数字 / 汉字**（与 S-010 / S-014 同口径：空理由不算豁免）
    const rest = l.slice(at + marker.length)
    return [...rest].some((c) => /[0-9A-Za-z]/.test(c) || c.codePointAt(0) > 127)
  }
  /** 形状①：退役的布尔壳。 */
  const isRetiredShell = (l) => /\bclosure_granted\b/.test(l)
  /** 形状②：闸的调用行——定义行不算，调用行必须自己带 `if let Err(`。 */
  const isGateCall = (l) => /\bclosure_gate\s*\(/.test(l) && !/\bfn\s+closure_gate\b/.test(l)
  const handlesErr = (l) => l.includes('if let Err(')
  let s015 = 0
  const s015List = []
  for (const file of S015_ROOTS.flatMap((r) => walk(path.join(S015_BASE, r)))) {
    // 测试文件豁免：那里断言的是谓词本身，不是拒绝路径。
    // 拒绝路径只在生产代码里。
    if (path.basename(file).endsWith('.test.rs')) continue
    let text
    try {
      text = fs.readFileSync(file, 'utf8')
    } catch {
      continue
    }
    const lines = text.split(String.fromCharCode(10))
    lines.forEach((l, i) => {
      if (isComment(l)) return
      if (isWaived(l) || (i > 0 && isWaived(lines[i - 1]))) return
      if (isRetiredShell(l)) {
        s015++
        s015List.push(`${disp(file)}:${i + 1}（布尔壳 \`closure_granted\`）`)
        return
      }
      if (!isGateCall(l)) return
      // 拆行写法：`if let Err(why) =` 上一行、调用在下一行 ⇒ 上一行同看。
      if (handlesErr(l)) return
      if (i > 0 && handlesErr(lines[i - 1])) return
      s015++
      s015List.push(`${disp(file)}:${i + 1}（闸的调用没当场处理 \`Err\`）`)
    })
  }
  if (s015 > 0) {
    err(
      `S-015 违例：收束闸被压回布尔形态（${s015} 处）——` +
        '布尔壳 `closure_granted` 已退役；闸的每一次调用都要当场 `if let Err(why) = closure_gate(..)` ' +
        '并调 `deny(..)` 落 `authz.denied`。' +
        `命中：${s015List.join('、')}`,
    )
  } else {
    ok(
      S015_IS_REPO
        ? 'S-015 通过：布尔壳零处、七处闸调用都当场处理 `Err`（全部走 closure_gate + deny）'
        : 'S-015 跳过：当前树不是真实仓库（临时 fixture）',
    )
  }
}
console.log()

// ── S-016: 停机原因只有一个形状─────────────────────────────
// 收束格 / 否定事实的「为什么停」只能写在顶层 `reason`（闭集）。两个已退役的
// 形状不许复活：
//   · `terminated.reason` —— 与顶层 `reason` 是同一件事的两份账，必然有一处先漂；
//   · 停机格载荷里的 `why` —— 自由文本，与 `reason` 语义重叠，读侧要靠猜。
// （`why` 在别的格子上仍然合法：commitment.breach / memory.consolidated 等没有
//   竞争的 `reason` 字段，那条不在本规则的范围内。）
//
// 判据是**零出现**：这两个形状已经全部迁完，没有存量可言。放任一条回来的代价
// 不是风格问题——读侧从此要判两个键，而多出来的那个键**不会让任何东西变红**。
//
// 范围收在**三个构造点文件**（`with_terminated` / `append_fallback` / `deny`）：
// 停机格载荷只从这三处出生，扫整个目录会把 `memory_events` 那个格子的 `why`
// （没有竞争的 `reason` 字段，不在本规则范围内）一起卷进来。
// 正则要求 `"reason"` / `reason` **带冒号或引号**：否则 `with_terminated(with_reason(..)`
// 这种合法组合会被 `terminated…reason` 误命中——一条误报就会让人学会忽略这条规则。
console.log('--- S-016: 停机原因单形状（terminated.reason / why 不许回来）---')
{
  const S016_FILES = [
    'symbio/src/plugins/loop/turn_runner.rs',
    'symbio/src/plugins/loop/turn_runner/facts.rs',
    'symbio/src/plugins/loop/turn_runner/settle.rs',
  ]
  const S016_BASE = isDir(path.resolve(cwd, 'symbio')) ? cwd : repoRoot
  const S016_IS_REPO = path.resolve(S016_BASE) === path.resolve(repoRoot)
  // 两种写法都要认：`json!({ "why": .. })`（键后冒号）与 `payload["why"] = ..`
  // （下标赋值）。只认前者会漏掉生产里真正在用的那种。
  const RETIRED_RE = /terminated[^\n]*(?:"reason"\s*:|\breason\s*:)|"why"\]\s*=|\bwhy"\s*:/
  const WAIVER_S016_RE = /\/\/\s*grep-audit-allow S-016:[^\n]*[A-Za-z0-9\u4e00-\u9fff]/
  let s016 = 0
  const s016List = []
  for (const rel of S016_FILES) {
    const file = path.join(S016_BASE, rel)
    let lines
    try {
      lines = fs.readFileSync(file, 'utf8').split(/\r?\n/)
    } catch {
      continue
    }
    lines.forEach((l, i) => {
      if (/^\s*(?:\/\/|\/\*|\*)/.test(l)) return
      if (!RETIRED_RE.test(l)) return
      if (WAIVER_S016_RE.test(l) || (i > 0 && WAIVER_S016_RE.test(lines[i - 1]))) return
      s016++
      s016List.push(`${disp(file)}:${i + 1}`)
    })
  }
  if (s016 > 0) {
    err(
      `S-016 违例：停机原因出现已退役的形状（${s016} 处）——` +
        '`terminated.reason` 与停机格的 `why` 都已并入顶层 `reason`（闭集）。' +
        `命中：${s016List.join('、')}`,
    )
  } else {
    ok(
      S016_IS_REPO
        ? 'S-016 通过：`turn_runner/` 下零处已退役的停机原因形状'
        : 'S-016 跳过：当前树不是真实仓库（临时 fixture）',
    )
  }
}
console.log()

// ── S-017: 恢复路径不许再有旁路（d）─────────────────────────
// 恢复 / 审批 / 问答三条唤醒路径曾经走 `resume.rs` 的**私有旁路**：它自己写协议
// 事实、自己跑执行体，绕开行表。旁路退役之后，这条路径的唯一入口是**事实**——
// 用户动作落成 `request(port=tool)`，之后与批次走同一条授权 / 派发链。
//
// 判据是**零出现**（不是棘轮）：旁路已经全部迁完，没有存量可言。放任它回来的
// 代价不是风格问题——恢复路径会**绕过行表**写事实，于是「这一轮欠了什么」
// 与「这一轮做了什么」对不上账，而没有任何东西会变红。
//
// 扫三样：
//   · `resume.rs` / `resume.test.rs` 两个文件不许存在；
//   · 生产代码里 `resume_anchor`（旁路的选债口）零出现；
//   · `turn_exec.rs` / `turn_exec/` 里 `resumed` 分支零出现（该词在
//     `loop_rows.rs` 里是视图字段名 `turn_head.resumed`，不在本规则范围内）。
console.log('--- S-017: 恢复路径不许再有旁路 ---')
{
  const S017_BASE = isDir(path.resolve(cwd, 'symbio')) ? cwd : repoRoot
  const S017_IS_REPO = path.resolve(S017_BASE) === path.resolve(repoRoot)
  const RETIRED_FILES = [
    'symbio/src/plugins/loop/resume.rs',
    'symbio/src/plugins/loop/resume.test.rs',
  ]
  // `resumed` 只在执行体那两处判——`loop_rows.rs` 里的 `turn_head.resumed` 是
  // 视图字段名，不是旁路分支。
  const RESUMED_FILES = [
    'symbio/src/plugins/loop/turn_exec.rs',
    'symbio/src/plugins/loop/turn_exec',
  ]
  let s017 = 0
  const s017List = []
  for (const rel of RETIRED_FILES) {
    if (fs.existsSync(path.join(S017_BASE, rel))) {
      s017++
      s017List.push(rel)
    }
  }
  const scan = (rel, re) => {
    const file = path.join(S017_BASE, rel)
    let lines
    try {
      lines = fs.readFileSync(file, 'utf8').split(/\r?\n/)
    } catch {
      return
    }
    lines.forEach((l, i) => {
      if (/^\s*(?:\/\/|\/\*|\*)/.test(l)) return
      if (!re.test(l)) return
      s017++
      s017List.push(`${disp(file)}:${i + 1}`)
    })
  }
  // `resume_anchor`：整个生产树（插件 + core 的视图层）都不许再有这个名字。
  for (const file of ['symbio/src/plugins', 'symbio/src/symbio_core'].flatMap((r) =>
    walk(path.join(S017_BASE, r)),
  )) {
    if (path.basename(file).endsWith('.test.rs')) continue
    scan(path.relative(S017_BASE, file), /\bresume_anchor\b/)
  }
  // `resumed` 分支：只在执行体那两处判。
  for (const rel of RESUMED_FILES) {
    const abs = path.join(S017_BASE, rel)
    if (!fs.existsSync(abs)) continue
    const files = fs.statSync(abs).isDirectory() ? walk(abs) : [abs]
    for (const file of files) {
      if (path.basename(file).endsWith('.test.rs')) continue
      scan(path.relative(S017_BASE, file), /\bresumed\b/)
    }
  }
  if (s017 > 0) {
    err(
      `S-017 违例：恢复路径的私有旁路回来了（${s017} 处）——` +
        '唤醒 / 续写只能从事实进环：用户动作落成 `request(port=tool)`，' +
        '之后与批次走同一条授权 / 派发链（`resume.rs` 与 `resume_anchor` 均已退役）。' +
        `命中：${s017List.join('、')}`,
    )
  } else {
    ok(
      S017_IS_REPO
        ? 'S-017 通过：恢复路径零旁路（`resume.rs` 已退役、`resume_anchor` 与 `resumed` 分支零出现）'
        : 'S-017 跳过：当前树不是真实仓库（临时 fixture）',
    )
  }
}
console.log()

// ── 汇总 ───────────────────────────────────────────────────────────────
console.log('=== 汇总 ===')
console.log(`Errors:   ${errors}`)
console.log(`Warnings: ${warnings}`)

if (errors > 0) process.exit(1)
if (STRICT && warnings > 0) process.exit(2)
process.exit(0)
