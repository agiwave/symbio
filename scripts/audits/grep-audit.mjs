#!/usr/bin/env node
/**
 * grep-audit — 静态审计检查脚本
 *
 * ⚠️ 文件头曾写着「对应 PLAN §S-4」，而那份 PLAN **已不在仓库里**（它随归档层
 * 一起退役，历史在 `git log`）。规则编号由此**继承**自
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
 *   - S-012:        代码注释不得写只有实施排期含义的话（待办标记 / 批次号）
 *   - S-013:        `WalkBuilder` 构造点必须钉遍历序（默认序是文件系统返回序 ⇒
 *                   分页窗口与截断点随平台变；NTFS 名字序 / ext4 哈希序）
 *   - S-014:        ChatNode 存量 `role` 格的直写只许降（棘轮；身份写 `principal`，
 *                   读走 `node_role()` / `legacy_role()`，字段已退出 wire）；
 *                   判据面不含 provider 请求构造（`wire_messages` 的瞬时线格式）
 *   - S-015:        收束闸只有一个 owner（`closure_gate`），且每次调用当场处理 `Err`
 *                   （布尔壳 `closure_granted` 零出现）——否则「拒绝」又变成「没写」，
 *                   日志里与「没跑完」同形
 *   - S-016:        停机原因只有一个形状（`terminated.reason` / 停机格的 `why` 不许回来）
 *   - S-017:        恢复路径不许再有旁路（`resume.rs` 与 `resume_anchor` 零出现）——
 *                   唤醒 / 续写只能从事实进环
 *
 * 用法：
 *   node scripts/audits/grep-audit.mjs            # 审计 crates/（全部插件与 core）
 *   node scripts/audits/grep-audit.mjs --strict   # 严格模式：warning 也算失败
 *   SCOPE=<dir> node scripts/audits/grep-audit.mjs
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
// @codes S-001 S-002 S-003 S-006 S-007 S-008 S-009 S-010 S-012 S-013 S-014 S-015 S-016 S-017
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green } from '../color.mjs'
import { walkFiles } from '../fs-walk.mjs'
/** 源码树里的 `.rs`（S-012…S-015 各自的根都由调用方给）。 */
const walk = (dir) => walkFiles(dir, { exts: ['.rs'], tolerant: true })


const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
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
  // 插件与 core 都在 `crates/` 下：S-002 的扫描面是**整个 workspace 源树**。
  return path.resolve(cwd, 'crates')
}

/**
 * `cwd` 是不是一棵仓库树（回归测试用 `spawn(cwd=<临时树>)` 注入夹具）。
 *
 * S-010 起的每条规则都要在「当前工作树」与「仓库根」之间选一个扫描基准：选错方向不会
 * 报错，只会**去错的地方找**——夹具造的违规一条也搜不到，用例全红而真仓全绿。
 *
 * 标记必须与 `resolveScope` 同一口径：cwd 下有 `crates/` 才算仓库树。
 * **六条规则各写一遍判断**会改一处漏五处，故抽成这一格。
 */
function looksLikeRepoTree(dir) {
  return isDir(path.resolve(dir, 'crates'))
}

const scopeAbs = resolveScope()
/** 显示用路径：相对 cwd，并统一为正斜杠（跨平台一致） */
const disp = (p) => path.relative(cwd, p).split(path.sep).join('/') || '.'

// ── 共享源码语料：一次遍历、按路径缓存行 ─────────────────────────────────
/** 六条规则共用的扫描基准与「是不是真实仓库」判定（原先各写一份）。 */
const BASE = looksLikeRepoTree(cwd) ? cwd : repoRoot
const IS_REPO = path.resolve(BASE) === path.resolve(repoRoot)

const lineCache = new Map()
/**
 * 读一份 .rs 的行数组并缓存；读不到 ⇒ null（与原先各规则 `catch → continue` 同义）。
 * 扫描根里 `crates` 与 `crates/symbio-cli/src` 重叠，重叠文件过去每条规则各读两遍。
 * 缓存后只读一次，但**文件表里的重复项照旧保留**——S-014 的棘轮读数依赖它，
 * 顺手去重会悄悄改掉读数。
 */
const linesOfPath = (file) => {
  if (lineCache.has(file)) return lineCache.get(file)
  let lines
  try {
    lines = fs.readFileSync(file, 'utf8').split(/\r?\n/)
  } catch {
    lines = null
  }
  lineCache.set(file, lines)
  return lines
}
/** 按根展开文件表：保留 roots 的重叠项与原有顺序，不排序、不去重。 */
const filesUnder = (roots) => roots.flatMap((r) => walk(path.join(BASE, r)))

// ── 收集 .rs 源文件 ────────────────────────────────────────────────────
// 生产路径才判：测试文件里 `Mutex::new` + 下一行 `.await` 是常态（锁根本没跨 await 持有），
// 而 S-002 的 5 行窗口分不出「建锁」与「持锁」——crate 化后扫描面扩到整个 `crates/`，
// 这条窗口噪声必须先挡住，否则守卫会红在 false positive 上、然后被人豁免到失效。
const isTestFile = (f) => f.endsWith('.test.rs') || path.basename(f) === 'tests.rs'
const files = walk(scopeAbs).filter((f) => !isTestFile(f))
const linesOf = new Map()
for (const f of files) {
  const lines = linesOfPath(f)
  if (lines) linesOf.set(f, lines)
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
// 词表只有一套：`symbio_sdk::vdfs::VDFS_STATUS_*`（`docs/003-design/009-VDFS规范.md`
// §3.2）。裸字面量的危险不是拼错（那会立刻看见），而是**改名时不会编译失败**——
// `status` 是跨进程边界的字符串，改了常量而漏掉字面量，前端只会静默认不出状态。
//
// 该词表**曾经**把「以错误结束」从 `error` 改名为 `failed`（理由见
// `VDFS_STATUS_FAILED` 的文档：与消息层 `NodeStatus::Failed` 同词），而
// `symbio_sdk::schemas::options.rs`（**该文件已于 2026-09-23 随会话选项 schema 化
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
// 词表只有一套：`symbio_sdk::event_bus::KIND_*`（文档见
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
// 根名是 vdfs 插件自己的挂载规则（`crates/symbio-plugin-vdfs/src/fs.rs::VDFS_ADDR_ROOT`，
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
//   · `crates/symbio-plugin-vdfs/src/**` —— 根名的所有者，字面量只允许在这里；
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
// 与 resolveScope 同策略：cwd 是仓库树就用 cwd（回归测试注入临时树），否则退回仓库根。
// 「是不是仓库树」由 `looksLikeRepoTree` 统一判——从前这里各写一份 `isDir(cwd/'symbio')`，
// D-191 之后根下再无 symbio/，标记恒假 ⇒ 夹具永远落到 `repoRoot`，S-010 的用例测的是
// **真仓**而不是自己造的树（实测造在 tauri/src/schemas/x.ts 的违规被去真仓里找 ⇒ 全红）。
const S010_ROOT = looksLikeRepoTree(cwd) ? cwd : repoRoot

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
  if (rel.startsWith('crates/symbio-plugin-vdfs/')) return false // 所有者（含 README：根名在它的文档里本来就该出现）
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
        `（crates/symbio-plugin-vdfs/src/fs.rs），消费方用运行期数据（后端 VDFS_PARENT_ADDR / 前端 vdfsRoot 锚点）`,
    )
    s010++
  })
}
if (s010 === 0) ok('S-010 通过：根名字面量只在 vdfs 插件内')
console.log()

// ── 判据运行时：一次遍历、多规则 ─────────────────────────────────────────
//
// S-012…S-017 过去各写一份「BASE 推断 + IS_REPO + roots + walk + readFileSync +
// 注释跳过 + 双行豁免 + 汇总」：同一份 .rs 被读六遍，roots 里 `crates` 与
// `crates/symbio-cli/src` 还彼此重叠，重叠部分每条规则再读一遍。这里收成
// **一份语料缓存 + 一张规则表**：规则只声明「扫哪些根、哪一行算命中、汇总怎么说」。
//
// 输出是判据的对外契约：标题、逐条 err、末尾 ok、空行的顺序与文案逐字不动；
// 重叠 roots 造成的重复项也照旧保留（S-014 的棘轮读数依赖它）。
const RS_ROOTS = ['crates', 'crates/symbio-cli/src', 'tauri/src-tauri/src', 'docs/003-design/verify']

/** 注释行（`//` / `/*` / 块注释续行）——S-014 / S-015 / S-016 / S-017 的同一种跳过。 */
const isCommentStart = (l) => /^\s*(?:\/\/|\/\*|\*)/.test(l)
/** 留痕豁免的统一字形：理由必须含至少一个字母 / 数字 / 汉字，空理由不算豁免。 */
const waiverRe = (code) =>
  new RegExp(`\\/\\/\\s*grep-audit-allow ${code}:[^\\n]*[A-Za-z0-9\\u4e00-\\u9fff]`)
/** 命中行或紧邻上一行写了理由即放行（S-010 / S-012…S-016 同一口径）。 */
const waived = (re) => (l, i, lines) => re.test(l) || (i > 0 && re.test(lines[i - 1]))

/**
 * 行级判据执行壳。规则字段：
 *   `title` 小节标题；`roots` 扫描根（重叠项保留、顺序不变），或 `files()` 直接给文件表；
 *   `begin(ctx)` 开跑前置；`skipFile(file)` 整份不判；
 *   `beginFile(file, lines)` 每份文件的附加上下文；`skip(l, i, …)` 行级跳过；
 *   `hit(l, i, file, lines, per, ctx)` 命中处理；`done(ctx, list)` 汇总输出。
 */
function runLineRule({ title, roots, files, begin, skipFile, beginFile, skip, hit, done }) {
  console.log(title)
  const list = files ? files() : filesUnder(roots)
  const ctx = {}
  begin?.(ctx)
  for (const file of list) {
    const lines = linesOfPath(file)
    if (!lines) continue
    if (skipFile?.(file)) continue
    const per = beginFile ? beginFile(file, lines) : null
    lines.forEach((l, i) => {
      if (skip && skip(l, i, lines, per, ctx)) return
      hit(l, i, file, lines, per, ctx)
    })
  }
  done(ctx, list)
  console.log()
}

const W012 = waived(waiverRe('S-012'))
const W013 = waived(waiverRe('S-013'))
const W014 = waived(waiverRe('S-014'))
const W016 = waived(waiverRe('S-016'))

// ── S-012: 代码注释不得写只有实施排期含义的话 ──────────────────────────────
//
// 注释的边界（`CONTRIBUTING.md` §4）：写「读这个文件需要知道的不变量 / 陷阱 / 为什么」，
// **不写过程**。过程叙述会变成假的：「本批未修」的下一批修了，注释就指着一个不存在的
// 时间点；批次号在重排后无人能解。这类句子没有 owner，也就没人回头改它。
//
// 判据只取两种无歧义形态：注释行以 `TODO` / `FIXME` / `XXX` / `HACK` 起头；或出现
// `批 N` / `批次 N`。**已知边界（刻意不判）**：`本批` / `曾经` 在注释里常是运行期语义
// 或陷阱说明，实测列进词表会在 `symbio/src` 命中 50+ 处误报——一条天天误报的守卫
// 最后只会被人用豁免喂死。
const S012_TODO_RE = /^\s*(?:\/\/[/!]*)\s*(?:TODO|FIXME|XXX|HACK)\b/
const S012_BATCH_RE = /批(?:次)?\s*\d/

runLineRule({
  title: '--- S-012: 代码注释里的实施排期指认 ---',
  roots: RS_ROOTS,
  skip: (l) => !/^\s*\/\//.test(l),
  hit: (l, i, file, lines, per, ctx) => {
    if (W012(l, i, lines)) return
    const why = S012_TODO_RE.test(l)
      ? '注释以 TODO / FIXME 起头 ⇒ 待办登记进计划文档，不写进代码注释'
      : S012_BATCH_RE.test(l)
        ? '批次号属施工计划 ⇒ 注释只写不变量与陷阱，不写施工过程'
        : null
    if (!why) return
    err(`${disp(file)}:${i + 1}  ${why}`)
    ctx.n = (ctx.n ?? 0) + 1
  },
  done: (ctx) => {
    if (!ctx.n) ok('S-012 通过：注释里没有待办标记与批次号')
  },
})

// ── S-013: `WalkBuilder` 构造点必须钉遍历序 ────────────────────────────────
//
// `ignore::WalkBuilder` 不配排序时按**文件系统返回的目录项序**遍历：NTFS 恰是名字序、
// ext4 是哈希序。下游三处判定都切在这个序上（`content_search` 的分页窗口、
// `MAX_RESULTS` 截断点、`codebase_search` 降级路径的 `limit`）。
// 判据：`WalkBuilder::new(` 之后**同一函数体内**（扫到下一个 `fn` 起头为止）必须出现
// `.sort_by(` / `.sort_by_file_name(`。**已知边界（刻意）**：跨函数传 builder 不判；
// 同函数两个 builder 共用一次 sort 也判绿——三处既有代码都是「构造 + 链式配置同作用域」。
const S013_WALK_RE = /WalkBuilder::new\s*\(/
const S013_SORT_RE = /\.sort_by(?:_file_name)?\s*\(/
const S013_NEXT_FN_RE = /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s/

runLineRule({
  title: '--- S-013: WalkBuilder 构造点的遍历序 ---',
  roots: RS_ROOTS,
  hit: (l, i, file, lines, per, ctx) => {
    if (!S013_WALK_RE.test(l)) return
    ctx.sites = (ctx.sites ?? 0) + 1 // 豁免过的构造点也算一处（汇总读这个数）
    if (W013(l, i, lines)) return
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
    ctx.bad = (ctx.bad ?? 0) + 1
  },
  done: (ctx) => {
    if (!ctx.bad)
      ok(
        !ctx.sites
          ? 'S-013 通过：scope 内没有 WalkBuilder 构造点'
          : `S-013 通过：${ctx.sites} 处 WalkBuilder 构造点都钉了遍历序（或已写豁免理由）`,
      )
  },
})

// ── S-014: ChatNode `role`（存量格）的直写只许降 ──────────────────────────
//
// 身份的权威是 `principal.kind`：读侧经 `node_role()` 折叠，写侧每处 `role` 直写都是
// 待清零的存量债务。字段不能私有化——`ChatNode { .., ..Default::default() }` 的函数式
// 更新语法要求每个字段可见（E0451），会把数百个构造点一起炸掉。所以约束是**棘轮**：
// 存量总数钉在基线上，只许降；降了要同步下调基线（`35-baseline` 守这个数不被放松）。
//
// **不含** `chat_node.rs`（字段定义本体）；**不含** provider 请求构造
// `symbio-plugin-model/src/port.rs::wire_messages` —— 那些 `ChatNode` 不落库，`role` 是
// 线协议值（D-305 裁决一），逐行豁免会让棘轮靠豁免活着，故按函数体整段排除。
const S014_WRITE_RE =
  /role:\s*Some\((?:cm::)?NodeRole::|\.role\s*=\s*Some\(|\.role\.get_or_insert\(/

/** provider 请求构造的函数体行区间（0 基 `[start, end]`）；只认 `fn wire_messages(` 这个名字。 */
function providerWireSpan(lines) {
  const start = lines.findIndex((l) => /^fn\s+wire_messages\s*\(/.test(l))
  if (start < 0) return null
  let depth = 0
  let opened = false
  for (let i = start; i < lines.length; i++) {
    for (const ch of lines[i]) {
      if (ch === '{') {
        depth++
        opened = true
      } else if (ch === '}') {
        depth--
        if (opened && depth === 0) return [start, i]
      }
    }
  }
  return null
}
// 基线三元形状与 `dead-code-audit` 的 R-002 同构——`35-baseline` 的提取函数按这个形状读格。
// 151 → 146 → 143 → 0（D-147 三档）：构造点全部改收主体，剩下的只有读边界与两条
// 存量格读法用例。清零之后这条棘轮数的就是「新增直写」：任何一处新写都即红。
const LEGACY_ROLE_WRITE_BASELINE = process.env.LEGACY_ROLE_WRITE_BASELINE
  ? Number(process.env.LEGACY_ROLE_WRITE_BASELINE)
  : 0

runLineRule({
  title: '--- S-014: ChatNode 存量 role 格直写棘轮 ---',
  roots: RS_ROOTS,
  skipFile: (file) => path.basename(file) === 'chat_node.rs',
  beginFile: (file, lines) =>
    file.split(path.sep).join('/').endsWith('symbio-plugin-model/src/port.rs')
      ? providerWireSpan(lines)
      : null,
  hit: (l, i, file, lines, wireSpan, ctx) => {
    if (wireSpan && i >= wireSpan[0] && i <= wireSpan[1]) return
    if (isCommentStart(l)) return
    if (!S014_WRITE_RE.test(l)) return
    if (W014(l, i, lines)) return
    ctx.n = (ctx.n ?? 0) + 1
    if (!ctx.list) ctx.list = []
    if (ctx.list.length < 8) ctx.list.push(`${disp(file)}:${i + 1}`)
  },
  done: (ctx) => {
    const n = ctx.n ?? 0
    const list = ctx.list ?? []
    if (n > LEGACY_ROLE_WRITE_BASELINE) {
      err(
        `S-014 违例：存量 role 格直写 ${n} 处 > 基线 ${LEGACY_ROLE_WRITE_BASELINE}——` +
          `身份写 ` +
          '`principal`，不写 role（读走 node_role()/legacy_role()）。首批命中：' +
          list.join('、'),
      )
    } else if (IS_REPO && n < LEGACY_ROLE_WRITE_BASELINE) {
      err(
        `S-014 基线该收紧：存量 role 格直写 ${n} 处 < 基线 ${LEGACY_ROLE_WRITE_BASELINE}——` +
          `把 LEGACY_ROLE_WRITE_BASELINE 下调到 ${n}（棘轮只许留痕地下调）`,
      )
    } else {
      ok(
        IS_REPO
          ? `S-014 通过：role 格直写存量 ${n} 处，与基线持平（只许降）`
          : 'S-014 跳过：当前树不是真实仓库（临时 fixture），棘轮不比',
      )
    }
  },
})

// ── S-015: 收束闸只有一个 owner，且每次调用都当场处理 Err ──────────────────
//
// `closure_gate(actor, kind) -> Result<(), String>` 把「为什么拒」带出来交给 `deny(…)`
// 落 `authz.denied`。否定分支若把闸压成布尔或把结果存起来另判，拒绝就又成了
// 「什么都没写」：日志里「被拒」与「根本没跑完」同形，且**不会有任何东西变红**。
// 判据是**零出现**：① 布尔壳 `closure_granted` 零出现；② 闸的调用行必须当场
// `if let Err(`——拆行写法由「上一行同看」挡住，不许 `.is_ok()` 取布尔。
// 测试文件豁免：那里断言的是谓词本身，不是拒绝路径。
const isWaived015 = (l) => {
  const marker = 'grep-audit-allow S-015:'
  const at = l.indexOf(marker)
  if (at < 0) return false
  const rest = l.slice(at + marker.length)
  return [...rest].some((c) => /[0-9A-Za-z]/.test(c) || c.codePointAt(0) > 127)
}
const W015 = (l, i, lines) => isWaived015(l) || (i > 0 && isWaived015(lines[i - 1]))

runLineRule({
  title: '--- S-015: 收束闸单一 owner 且调用当场处理 Err ---',
  roots: ['crates'],
  skipFile: (file) => path.basename(file).endsWith('.test.rs'),
  hit: (l, i, file, lines, per, ctx) => {
    if (isCommentStart(l)) return
    if (W015(l, i, lines)) return
    if (/\bclosure_granted\b/.test(l)) {
      ctx.n = (ctx.n ?? 0) + 1
      if (!ctx.list) ctx.list = []
      ctx.list.push(`${disp(file)}:${i + 1}（布尔壳 \`closure_granted\`）`)
      return
    }
    if (!/\bclosure_gate\s*\(/.test(l) || /\bfn\s+closure_gate\b/.test(l)) return
    ctx.calls = (ctx.calls ?? 0) + 1
    if (l.includes('if let Err(')) return
    if (i > 0 && lines[i - 1].includes('if let Err(')) return
    ctx.n = (ctx.n ?? 0) + 1
    if (!ctx.list) ctx.list = []
    ctx.list.push(`${disp(file)}:${i + 1}（闸的调用没当场处理 \`Err\`）`)
  },
  done: (ctx) => {
    const n = ctx.n ?? 0
    const calls = ctx.calls ?? 0
    const list = ctx.list ?? []
    if (n > 0) {
      err(
        `S-015 违例：收束闸被压回布尔形态（${n} 处）——` +
          '布尔壳 `closure_granted` 已退役；闸的每一次调用都要当场 `if let Err(why) = closure_gate(..)` ' +
          '并调 `deny(..)` 落 `authz.denied`。' +
          `命中：${list.join('、')}`,
      )
    } else {
      ok(
        IS_REPO
          ? `S-015 通过：布尔壳零处、${calls} 处闸调用都当场处理 \`Err\`（全部走 closure_gate + deny）`
          : 'S-015 跳过：当前树不是真实仓库（临时 fixture）',
      )
    }
  },
})

// ── S-016: 停机原因只有一个形状 ───────────────────────────────────────────
// 收束格 / 否定事实的「为什么停」只能写在顶层 `reason`（闭集）。两个已退役形状不许
// 复活：`terminated.reason`（同一件事的两份账）与停机格载荷里的 `why`（自由文本）。
// 扫描面 2026-11-13 重指为 `crates/symbio-plugin-loop/src` 下全部 `.rs`：原写死的三个
// 路径随旧引擎退役都已不存在，`readFileSync` 落进 `catch → continue`，判据恒绿——
// 正是 D-280 的「扫描面为空 ⇒ 读不到即全绿」。故另加「真仓里一个文件都没扫到即判红」。
// 正则要求 `"reason"` / `reason` 带冒号或引号，否则 `with_terminated(with_reason(..)`
// 这种合法组合会被误命中。
const RETIRED_RE = /terminated[^\n]*(?:"reason"\s*:|\breason\s*:)|"why"\]\s*=|\bwhy"\s*:/

runLineRule({
  title: '--- S-016: 停机原因单形状（terminated.reason / why 不许回来）---',
  roots: ['crates/symbio-plugin-loop/src'],
  hit: (l, i, file, lines, per, ctx) => {
    if (isCommentStart(l)) return
    if (!RETIRED_RE.test(l)) return
    if (W016(l, i, lines)) return
    ctx.n = (ctx.n ?? 0) + 1
    if (!ctx.list) ctx.list = []
    ctx.list.push(`${disp(file)}:${i + 1}`)
  },
  done: (ctx, list) => {
    const n = ctx.n ?? 0
    const hits = ctx.list ?? []
    if (IS_REPO && list.length === 0) {
      err(
        'S-016 扫描面为空：`crates/symbio-plugin-loop/src` 下一份 `.rs` 都没扫到 ⇒ ' +
          '判据读不到任何东西（`D-280`）；路径写错或 crate 改名都要在这里变红，而不是静默变绿。',
      )
    } else if (n > 0) {
      err(
        `S-016 违例：停机原因出现已退役的形状（${n} 处）——` +
          '`terminated.reason` 与停机格的 `why` 都已并入顶层 `reason`（闭集）。' +
          `命中：${hits.join('、')}`,
      )
    } else {
      ok(
        IS_REPO
          ? `S-016 通过：\`plugin-loop/src\` 下 ${list.length} 份 .rs 里零处已退役的停机原因形状`
          : 'S-016 跳过：当前树不是真实仓库（临时 fixture）',
      )
    }
  },
})

// ── S-017: 恢复路径不许再有旁路 ───────────────────────────────────────────
// 恢复 / 审批 / 问答三条唤醒路径曾走 `resume.rs` 的私有旁路：自己写协议事实、自己跑
// 执行体，绕开行表。旁路退役后唯一入口是事实——用户动作落成 `request(port=tool)`。
// 判据是**零出现**：① `resume.rs` / `resume.test.rs` 不许存在；② 生产代码里
// `resume_anchor` 零出现。第三样 `resumed` 已移出判据面：它如今是召回阶段格的载荷键。
const RETIRED_FILES = [
  'crates/symbio-plugin-loop/src/resume.rs',
  'crates/symbio-plugin-loop/src/resume.test.rs',
]

runLineRule({
  title: '--- S-017: 恢复路径不许再有旁路 ---',
  roots: ['crates'],
  skipFile: (file) => path.basename(file).endsWith('.test.rs'),
  begin: (ctx) => {
    for (const rel of RETIRED_FILES) {
      if (!fs.existsSync(path.join(BASE, rel))) continue
      ctx.n = (ctx.n ?? 0) + 1
      if (!ctx.list) ctx.list = []
      ctx.list.push(rel)
    }
  },
  hit: (l, i, file, lines, per, ctx) => {
    if (isCommentStart(l)) return
    if (!/\bresume_anchor\b/.test(l)) return
    ctx.n = (ctx.n ?? 0) + 1
    if (!ctx.list) ctx.list = []
    ctx.list.push(`${disp(file)}:${i + 1}`)
  },
  done: (ctx) => {
    const n = ctx.n ?? 0
    const list = ctx.list ?? []
    if (n > 0) {
      err(
        `S-017 违例：恢复路径的私有旁路回来了（${n} 处）——` +
          '唤醒 / 续写只能从事实进环：用户动作落成 `request(port=tool)`，' +
          '之后与批次走同一条授权 / 派发链（`resume.rs` 与 `resume_anchor` 均已退役）。' +
          `命中：${list.join('、')}`,
      )
    } else {
      ok(
        IS_REPO
          ? 'S-017 通过：恢复路径零旁路（`resume.rs` 已退役、`resume_anchor` 零出现）'
          : 'S-017 跳过：当前树不是真实仓库（临时 fixture）',
      )
    }
  },
})

// ── 汇总 ───────────────────────────────────────────────────────────────
console.log('=== 汇总 ===')
console.log(`Errors:   ${errors}`)
console.log(`Warnings: ${warnings}`)

if (errors > 0) process.exit(1)
if (STRICT && warnings > 0) process.exit(2)
process.exit(0)
