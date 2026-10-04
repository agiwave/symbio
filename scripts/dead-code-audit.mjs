#!/usr/bin/env node
/**
 * 死代码审计（前端 TS/Vue 判定 + Rust 声明级判定 R-001 / 承认标记判定 R-002）
 *
 * 四层判定，尽量零误报：
 *  L1 import 依赖图：从入口（index.html→main.ts、测试、.d.ts、构建配置）可达性。
 *  L2 字符串引用兜底：不可达但文件名仍被源码提及 → 降级「疑似」，不自动判死。
 *  L3 schema 契约：schemas/*.ts 若被 route 调用方以类型名引用则视为存活。
 *  L4 Rust 侧 **R-001（判定型）**：`pub` 声明但全仓（含 cli / tauri / 前端）
 *     一次都没被提及。判据是「这个名字在整仓只出现一次（就是声明那行）」——
 *     `dead_code` lint 对 `pub` 项结构性失明（`symbio_core/mod.rs` 一句
 *     `pub use plugin::*` 就能让整批函数被当成对外 API），故需要这条补网。
 *  L5 Rust 侧 **R-002（判定型）**：`#[allow(dead_code)]` 承认标记必须带
 *     `// dead-code-allow R-002: <非空理由>`，且数量走棘轮只许降——
 *     收窄根导出（`core-export-audit` C-003）之后，只被测试引用的冻结契约名
 *     会被 `dead_code` 逐个点名，留下的每一条都必须有「为什么留 / 什么时候摘」。
 *
 * 承认通道：确需保留但无 Rust 消费方的（消费方在前端 / 闭集成员），在声明行或
 * 紧邻其上一行写 `// dead-code-allow R-001: <理由>`；**理由不可为空**（空理由
 * 视为未承认，与 `grep-audit` / `plugin-entry-audit` 的豁免同一约定）。
 * R-002 的承认写在 `#[allow(dead_code)]` 同行或其上 3 行，同一句话、同一个判空。
 *
 * 用法： node scripts/dead-code-audit.mjs             # 死代码清单（明细只印死代码）
 *        node scripts/dead-code-audit.mjs --verbose   # 附带「未被引用的导出」明细
 * 退出码：1 = 有「确认死代码」（前端文件级）、「未承认的 R-001」或「R-002 违规」；
 *         0 = 通过。三者都在 gate.mjs 的 docs 阶段（判定型）。
 *
 * 已知边界：R-001 只抓「整仓一次都没被提过」这一最低风险形态。`pub` 项被**弱引用**
 * （只在文档 / 注释里被提到，或在运行期被拼名字调用）一律视为存活——宁可漏报。
 * R-002 同理只认**字面属性**：`cfg_attr` 里拼出来的 `allow`、宏内部生成的属性
 * （`define_string_key!` 靠调用点透传臂转发）都按调用点那行算。
 */
import { readdirSync, statSync, readFileSync, existsSync } from 'node:fs'
import { join, dirname, resolve, relative, extname, basename } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
// `--root=` 只为回归测试开的口子：测试在临时目录造一棵最小仓库，好让本脚本的判据
// 能被「注入真实违规并断言变红」地验证——判定型守卫的教条是「一个只会亮绿灯的
// 守卫等于没有守卫」，而它腐烂的方式恰恰是「规则写错了所以永远不命中」。
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? resolve(rootArg.slice(7)) : resolve(__dirname, '..')
const ROOT = join(REPO_ROOT, 'tauri')
const SRC = join(ROOT, 'src')
const EXT = ['.vue', '.ts', '.js', '.tsx', '.mts']
/** 打印「未被引用的导出」明细（默认只给个数，见文件末段说明） */
const VERBOSE = process.argv.includes('--verbose')

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) walk(p, out)
    else if (EXT.includes(extname(p))) out.push(p)
  }
  return out
}

const allFiles = walk(SRC)
const fileSet = new Set(allFiles)
const rel = (p) => relative(ROOT, p).replace(/\\/g, '/')
const isTest = (p) => p.includes('__tests__') || /\.(spec|test)\.[cm]?[jt]sx?$/.test(p)
const lineCount = (p) => readFileSync(p, 'utf8').split(/\r?\n/).length

function resolveSpec(spec, fromFile) {
  let base
  if (spec.startsWith('@/')) base = join(SRC, spec.slice(2))
  else if (spec.startsWith('.')) base = resolve(dirname(fromFile), spec)
  else return null
  return [base, ...EXT.map((e) => base + e), ...EXT.map((e) => join(base, 'index' + e))].find((c) =>
    fileSet.has(c),
  )
}

/** index.html 里的 /src/main.ts 在 Windows 下不能直接 resolve（会跑到盘符根） */
const projPath = (spec) => resolve(ROOT, spec.replace(/^[\\/]+/, ''))

const IMPORT_RE =
  /(?:import|export)\s+[^'"]*?from\s*['"]([^'"]+)['"]|import\s*\(\s*['"]([^'"]+)['"]\s*\)|import\s+['"]([^'"]+)['"]/g

function reachableFrom(entries) {
  const seen = new Set()
  const queue = [...entries]
  while (queue.length) {
    const f = queue.pop()
    if (!f || seen.has(f) || !fileSet.has(f)) continue
    seen.add(f)
    for (const m of readFileSync(f, 'utf8').matchAll(IMPORT_RE)) {
      const r = resolveSpec(m[1] ?? m[2] ?? m[3], f)
      if (r) queue.push(r)
    }
  }
  return seen
}

const entries = []
const seenEntry = new Set()
const addEntry = (p, why) => {
  if (fileSet.has(p) && !seenEntry.has(p)) {
    seenEntry.add(p)
    entries.push(p)
  }
}

if (existsSync(join(ROOT, 'index.html')))
  for (const m of readFileSync(join(ROOT, 'index.html'), 'utf8').matchAll(/\ssrc=["']([^"']+)["']/g))
    addEntry(projPath(m[1]))

for (const f of allFiles)
  if (isTest(f) || f.endsWith('.d.ts') || /(^|[\\/])(vite|vitest)\.config\.[cm]?[jt]s$/.test(f))
    addEntry(f)

const reachable = reachableFrom(entries)
const unreachable = allFiles.filter((f) => !reachable.has(f))

const corpus = allFiles.map((f) => [f, readFileSync(f, 'utf8')])
function stringReferenced(file) {
  const stem = basename(file).replace(/\.[^.]+$/, '')
  if (!stem || stem === 'index') return null
  const re = new RegExp(`${stem.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\.(vue|ts|js)`)
  for (const [f, code] of corpus) {
    if (f === file) continue
    const m = code.match(re)
    if (m) return { by: rel(f), token: m[0] }
  }
  return null
}

const dead = []
const suspicious = []
for (const f of unreachable) {
  const hit = stringReferenced(f)
  ;(hit ? suspicious : dead).push(hit ? [f, hit] : f)
}

console.log(`扫描 ${allFiles.length} 文件 · 入口 ${entries.length} · 可达 ${reachable.size}`)

console.log(`\n【死代码】${dead.length} —— 无 import、无字符串引用：`)
for (const f of dead) console.log(`  ${rel(f)}  (${lineCount(f)} 行)`)

if (suspicious.length) {
  console.log(`\n【疑似】${suspicious.length} —— 不可达但有字符串引用：`)
  for (const [f, h] of suspicious) console.log(`  ${rel(f)}  ← ${h.by} 提到 "${h.token}"`)
}

// ── 未使用的导出（死代码的细粒度形式）──
//
// ## 判据：**「无人用」必须含定义文件自己**
//
// 原先只统计**其他文件**的引用（`g !== f`），于是「定义在本文件、也在本文件里用」
// 的导出全被报了出来 —— 实测 89 条里抽查的每一条都是这一类：
// `UseVdfsOptions` 是同文件里 `useVdfs()` 的形参类型、`messageTypeOf` 在同文件里
// 被调两次、`HEAD_WORKDIR` 在同文件里被读写、`OUTCOME_*` 在同文件里被比较……
// 那不是保守，是**报告在说假话**：它把健康的导出报成可清理的垃圾，读报告的人
// 会顺着去"修"本来没坏的东西（`schema-audit` 的前端那一半踩的是同一个坑，
// 同一条判据已在那边修过一次 —— 两处必须说同一句话）。
//
// 计数含声明行本身，故「只出现一次」= 只在这里声明、无人使用。
//
// ## 已知边界（写清楚，免得把绿灯当证明）
//
// 按**标识符文本**判，不是 AST。因此「经 barrel 再导出后由别处使用」仍可能命中
// （`export * from './x'` 里不出现名字）——这类要靠人看一眼，故仍标**降级提示**：
// 打印个数，`--verbose` 才出明细（否则每次门禁刷几十行噪音，而噪音会教人忽略它）。
console.log(`\n【导出级检查】`)

/**
 * 承认通道：声明行（或紧邻上一行）带 `// dead-code-allow R-001: <理由>` ⇒ 已承认保留。
 * 与 Rust 侧 R-001 是**同一条约定**（理由不可为空）——两个守卫必须说同一句话，
 * 否则「在 Rust 那边承认了、前端这边还报着」会让人再查一遍。
 */
function waiverOf(code, name) {
  const lines = code.split('\n')
  const decl = new RegExp(
    `^[ \\t]*export\\s+(?:const|function|async\\s+function|class|interface|type|enum)\\s+${name.replace(/\$/g, '\\$')}\\b`,
  )
  for (let i = 0; i < lines.length; i += 1) {
    if (!decl.test(lines[i])) continue
    for (const j of [i, i - 1]) {
      if (j < 0) continue
      const m = lines[j].match(/\/\/\s*dead-code-allow\s+R-\d+\s*:\s*(.+?)\s*$/)
      if (m) return m[1]
    }
  }
  return null
}

const unusedList = []
for (const f of reachable) {
  if (isTest(f) || f.endsWith('.d.ts')) continue
  const code = readFileSync(f, 'utf8')
  // ⚠️ 正则**锚定行首**。不锚定时，注释里举例说明的写法也会被当成真导出——
  // `factory.ts` 的文档里就有一句「解构出去单独导出（`export const registerX = …`）」，
  // 于是 `registerX` 被报成死导出，而它**根本不存在**。实测：不锚定 433 个名字、
  // 锚定 432 个，差额恰好就是那一个注释里的假导出。
  const names = [
    ...[...code.matchAll(/^[ \t]*export\s+(?:const|function|async\s+function|class|interface|type|enum)\s+([A-Za-z0-9_$]+)/gm)].map((m) => m[1]),
  ]
  if (!names.length) continue
  const dir = rel(f)
  for (const n of names) {
    const re = new RegExp(`\\b${n.replace(/\$/g, '\\$')}\\b`)
    // 别的文件用过 ⇒ 活；否则再看**定义文件自己**用过没（声明行本身算 1 次）
    if (corpus.some(([g, c]) => g !== f && re.test(c))) continue
    // ⚠️ 计数必须用**全局**正则：非全局的 `match()` 只返回首个匹配，
    // `.length` 恒为 1（无捕获组时），于是「出现过几次」永远算成 1 —— 这条
    // 判据会静默失效、89 条一条都筛不掉，而输出看起来毫无异常。
    const selfOcc = (code.match(new RegExp(re.source, 'g')) ?? []).length
    if (selfOcc > 1) continue
    unusedList.push({ dir, name: n, waiver: waiverOf(code, n) })
  }
}
const unusedExports = unusedList.length
if (VERBOSE) {
  for (const u of unusedList) {
    console.log(`  ${u.dir} :: ${u.name}${u.waiver ? `（已承认保留：${u.waiver}）` : ''}`)
  }
} else if (unusedExports) {
  console.log(`  ${unusedExports} 个导出**全库无人用**（含定义文件自身；降级提示，不判失败；--verbose 看明细）`)
}
if (!unusedExports) console.log('  （无全库无人用的导出）')

// ── Rust 侧 R-001：`pub` 声明但全仓一次都没被提及（**判定型**）──
//
// 为什么需要：前端侧有 import 依赖图这张硬网；**Rust 侧一张都没有**。`dead_code`
// lint 对 `pub` 项结构性失明——`symbio_core/mod.rs` 里一句 `pub use plugin::*`
// 就足以让整批函数被当成"对外 API"，于是攒出过一批零调用的 helper（8 个锁/错误
// 辅助函数，全仓含 cli / tauri 零引用）。
//
// ## 为什么这条能判失败，而 `plugin-entry-audit` 的 `refs=0` 不能
//
// 「定义了但没人用」在**路由**上不可判：路径可以在运行期拼（工具名、子插件名），
// 网关还会把外部 `path` 原样转发给容器 route（见 plugin-entry-audit 头部的两条
// 理由）。但本规则判的不是运行期字符串，而是**一个静态符号名**——`pub fn foo`
// 的 `foo` 有没有在仓库任何地方被写过一次，是纯文本事实，没有动态解析的余地。
//
// ## 承认通道（唯一出口）
//
// 确有必须保留而无 Rust 消费方的（消费方在前端、闭集成员等），在声明行或紧邻其上
// 一行写 `// dead-code-allow R-001: <理由>`。**理由不可为空**——否则「随手加个
// 注释就过」会让这条守卫退化成橡皮图章（与 grep-audit / plugin-entry-audit 同一约定）。
const REPO = REPO_ROOT
const repoRel = (p) => relative(REPO, p).replace(/\\/g, '/')

const rustSources = []
const tsSources = []
const widen = (dir, exts, sink) => {
  let ents
  try {
    ents = readdirSync(dir, { withFileTypes: true })
  } catch {
    return
  }
  for (const e of ents) {
    const p = join(dir, e.name)
    if (e.isDirectory()) {
      if (['target', 'vendor', 'node_modules', 'dist', '.git'].includes(e.name)) continue
      widen(p, exts, sink)
      continue
    }
    if (exts.some((x) => e.name.endsWith(x))) sink.push(p)
  }
}
widen(join(REPO, 'symbio'), ['.rs'], rustSources)
widen(join(REPO, 'cli'), ['.rs'], rustSources)
widen(join(REPO, 'tauri', 'src-tauri'), ['.rs'], rustSources)
// 名字可能被非 Rust 侧提及（路由串常量等），一并纳入「是否被提过」的语料
widen(join(REPO, 'tauri', 'src'), ['.ts', '.vue'], tsSources)

const read = (files) => files.map((f) => [f, readFileSync(f, 'utf8')])
const rustCorpus = read(rustSources)
const nameCorpus = rustCorpus.concat(read(tsSources))

const DECL_RE =
  /^pub(?:\([^)]*\))?\s+(?:async\s+)?(?:unsafe\s+)?(fn|struct|enum|trait|const|static|type)\s+([A-Za-z_][A-Za-z0-9_]*)/gm

/** 承认通道：`// dead-code-allow R-001: <理由>`（理由为空 = 未承认） */
const WAIVER_RE = /\/\/\s*dead-code-allow\s+(R-\d+)\s*:\s*(\S.*)$/

/**
 * 取声明的承认理由：看**声明行本身**与紧邻其上 3 行（覆盖 `#[allow(dead_code)]`
 * 同行注理由、或独立一行注释两种写法）。返回理由字符串；未承认返回 `null`。
 */
function waiverReason(lines, declLine) {
  for (let i = declLine; i >= Math.max(0, declLine - 3); i--) {
    const hit = lines[i].match(WAIVER_RE)
    if (hit) return hit[2].trim()
  }
  return null
}

// 判据：**该名字在全仓只出现过一次**（就是声明那一行）⇒ 没人用过。
//
// 为什么不按"别的文件里有没有"来判：同一文件内的引用同样是真引用——
// `#[serde(default = "default_max_messages")]`、`submit_object_creator!(…, SettingPlugin::build, …)`
// 都只在声明文件里出现，按"别文件"判会把它们全数误报（实测踩坑）。
// 反过来，本规则只抓"从没被任何人提过"这一最低风险形态，与「导出级检查」同一取向。
const tokenCount = new Map()
for (const [, code] of nameCorpus) {
  for (const m of code.matchAll(/[A-Za-z_][A-Za-z0-9_]*/g)) {
    tokenCount.set(m[0], (tokenCount.get(m[0]) ?? 0) + 1)
  }
}

const rustUnused = []
const rustWaived = []
let rustDecls = 0
for (const [file, code] of rustCorpus) {
  if (file.endsWith('.test.rs') || basename(file) === 'tests.rs') continue
  const lines = code.split(/\r?\n/)
  for (const m of code.matchAll(DECL_RE)) {
    const [, kind, name] = m
    rustDecls++
    if ((tokenCount.get(name) ?? 0) > 1) continue
    // 声明所在行（0 基）：`m.index` 落在 `^` 之后，即行首
    const declLine = code.slice(0, m.index).split('\n').length - 1
    const reason = waiverReason(lines, declLine)
    const where = `${repoRel(file)} :: ${kind} ${name}`
    if (reason) rustWaived.push(`${where}  ← ${reason}`)
    else rustUnused.push(where)
  }
}

console.log(`\n【R-001】Rust 声明级检查（判定型）`)
console.log(`  扫描 ${rustCorpus.length} 个 .rs · ${rustDecls} 处 pub 声明`)
if (rustUnused.length) {
  console.log(`  ✗ ${rustUnused.length} 处全仓零引用且未承认：`)
  for (const line of rustUnused) console.log(`    ${line}`)
  console.log(`    ↳ 删掉它；确需保留则写 \`// dead-code-allow R-001: 理由\`（理由必填）`)
} else {
  console.log('  ✓ 无全仓零引用的 pub 声明')
}
if (rustWaived.length) {
  console.log(`  · ${rustWaived.length} 处已承认保留（消费方不在 Rust 侧，见各行理由）：`)
  for (const line of rustWaived) console.log(`    ${line}`)
}

// ── R-002：`dead_code` 承认标记必须带理由，且数量只许降（判定型）──
//
// 起因：`core-export-audit` C-003 把「名字没跨出 core」的符号从根出口收窄之后，
// `dead_code` lint 会把**只被测试引用的冻结契约名**逐个点名——那正是规则 2 的应有
// 之义（无消费者的东西不该占着根出口），但这些契约名要留到接线那天。于是就地承认。
// 承认若既无理由又无数量约束，半年后就是 138 个没人说得清的 `allow`，死码审计等于
// 自废。故：
//   ① 每个 `#[allow(dead_code)]`（含模块级 `#![allow(dead_code)]`）必须在**同一行**
//      或其上 3 行内写 `// dead-code-allow R-002: <非空理由>`——与 R-001 同一判空；
//   ② 数量走棘轮：只许降（接线一批、摘一批），`grep -c` 即可复核，不靠自觉。
//   ③ 理由必须指到**清偿步**（`04 §3.1 批N` 或其 A/B 表行）：接入 + 摘标记 + 步状态
//      改"完成"三者同批发生，账在 `docs/plan/04-工程落地.md §3.1`，本文件只判数量。
//      基线每次随该表的批次下调，不随本文件的意志上调。
const R002_BASELINE = process.env.DEAD_CODE_R002_BASELINE
  ? Number(process.env.DEAD_CODE_R002_BASELINE)
  : 18 // 04 §3.1 批⑪ 子批 A 摘 18（36 → 18）：自主与意图闸门接线——actors/mod.rs 15（AutonomousInitiator / trigger / express_intent / open_long_goal / health_event、ConationPolicy、ConationCandidate 2、GateWarrant、IntentDecision、ApprovedIntent、IntentGate 3，消费方 plugins/session/heartbeat/mod.rs::record_autonomous_trigger 落格 → 闸门 → 动作点验牌）+ event/mod.rs 3（system.triggered / system.health / conation.expressed 名字表）
  // 04 §3.1 批⑩ 子批 B 摘 5（41 → 36）：插话抢占接线——actors/mod.rs 5（Preemption / PreemptionDecider / decide / held_event / control_event，消费方 plugins/session/transcript/inbox.rs 忙窗判定 + 空闲落格 + 插话轮后写恢复；同域新增 resume_event 无标记）
  // 04 §3.1 批⑩ 子批 A 摘 5（46 → 41）：外部执行闸门接线——actors/mod.rs 4（GateDecision / CircuitBreaker / gate / break_event，消费方 plugins/session/tools/tool_executor.rs 判定 + v2_bridge.rs::record_to_wal 落格）+ event/mod.rs 1（EVENT_CONTROL_OPENED = task.controlled，写方同上按名写格）
  // 04 §3.1 批⑨ 摘 10（56 → 46）：任务表接线——projection/readyset.rs 3（ReadyTask / ReadySetView / readyset，读出口 stats 列 + 调度段两处消费）+ event/mod.rs 5（task.* 名字表，写方 plugins/session/v2_tasks.rs 按名写格）+ invariants/mod.rs 2（acyclic_deps / rework_bounded 进 check_all 七条）
  // 04 §3.1 批⑧ 摘 19（75 → 56）：身份 / 承诺 / 声誉全接线——actors/mod.rs 8（ActorSpec 3 起于 TurnInput.actor 入参 + run/run_streaming 平凡值、CommitmentKeeper 5 经 core 域内入口 commitment_events 落四格）+ projection/reputation.rs 7（stats.rs::read 新列 reputation，own/by_principal 全有全无）+ event/mod.rs 4（commitment.* 名字表，写方 v2_bridge::record_to_wal 按名写格）
  // 04 §3.1 批⑦ 摘 13（88 → 75）：记忆三段接线全摘——event/mod.rs 记忆事件名字表 4（写方 plugins/session/v2_memory.rs 按名写格）+ view/mod.rs 视图类型 3（RecallEntry/RecallView/contains_content 生产读视图与去重）+ projection/recall.rs 1（召回进 prepare_turn_inputs）+ projection/consolidate.rs 3（巩固写方过 accept）+ actors/mod.rs 2（RecallTranslator 落 memory.recalled）
  // 04 §3.1 批⑥ 摘 11（99 → 88）：governance/mod.rs 标记**全摘**——矩阵生产构造住 crate::authz（from_names 按能力名闭集校验），写侧 can_reply 进 v2_bridge 收束闸、读侧 can_see 进 session/stats 读闸
  // 04 §3.1 批⑤ 摘 16（115 → 99）：Decider 族 5 + S4 断点锚 2 + 测试面转 #[cfg(test)] 9（其中 3 处本属批⑫，随桩迁入 ⇒ 批⑫ 8 → 5）

/**
 * R-002 的承认理由：复用 R-001 的回看（同行 + 上 3 行、理由非空）。
 * 不校验规则号——`#[allow(dead_code)]` 与它所压制的那条 `pub` 声明常是同一件事
 * （`route.rs` 的路由常量同时挂 R-001 与 R-002），两个通道必须说同一句话。
 */
function r002Reason(lines, i) {
  return waiverReason(lines, i)
}

// ③ **承认必须挂到计划步**：理由里给出 `04 §3.1 批N`（或 `04 §3.1 C 类`），
//    且批号必须真实存在于 `docs/plan/04-工程落地.md §3.1` 的清偿批次表。
//    于是「一处死码 ↔ 一个计划步」双向可查：`grep "04 §3.1 批⑥"` = 该批待摘清单，
//    表里的 `-N` = 该批完成判据；批次号以表为唯一来源，写不存在的批号等于没挂。
const PLAN_04_PATH = join(REPO, 'docs', 'plan', '04-工程落地.md')
// 回归测试用 `--root=` 注入最小仓库，那里没有计划文档——**没有台账就不判这一条**
// （否则 fixture 全红，守卫的测试自己成了噪音）；真仓里文档在，规则才生效。
const PLAN_04 = existsSync(PLAN_04_PATH) ? readFileSync(PLAN_04_PATH, 'utf8') : null
const planStart = PLAN_04 ? PLAN_04.indexOf('### 3.1') : -1
const PLAN_04_SECTION = planStart >= 0 ? PLAN_04.slice(planStart) : PLAN_04
const validBatches = PLAN_04_SECTION ? new Set([...PLAN_04_SECTION.matchAll(/\|\s*([①-⑳])\s*\|/g)].map((m) => `批${m[1]}`)) : new Set()
const hasCClass = PLAN_04_SECTION ? /C\. 结构性承认/.test(PLAN_04_SECTION) : false
const REF_RE = /04 §3\.1 (批[①-⑳]|C 类)/

const r002NoReason = []
const r002BadPlan = []
const r002ByBatch = new Map()
let r002Marks = 0
for (const [file, code] of rustCorpus) {
  const lines = code.split(/\r?\n/)
  for (let i = 0; i < lines.length; i++) {
    // 只认**代码位置**的属性：以 `//` 开头的行是文档/注释里举例的写法
    // （`plugin/route.rs` 的文档段就提了一句 `#[allow(dead_code)]`），
    // 照单全收会凭空多出一条「没有理由的标记」。
    if (lines[i].trimStart().startsWith('//')) continue
    if (!/#!?\[allow\(dead_code\)\]/.test(lines[i].split('//')[0])) continue
    r002Marks++
    const reason = r002Reason(lines, i)
    if (!reason) {
      r002NoReason.push(`${repoRel(file)}:${i + 1}`)
      continue
    }
    if (!PLAN_04) continue // 无台账（fixture 仓库）⇒ 本条不判
    const ref = reason.match(REF_RE)
    if (!ref) {
      r002BadPlan.push(`${repoRel(file)}:${i + 1}  理由未指向 04 §3.1：${reason.slice(0, 60)}`)
      continue
    }
    const key = ref[1]
    const known = key === 'C 类' ? hasCClass : validBatches.has(key)
    if (!known) {
      r002BadPlan.push(`${repoRel(file)}:${i + 1}  ${key} 不在 04 §3.1 的清偿登记里`)
      continue
    }
    r002ByBatch.set(key, (r002ByBatch.get(key) ?? 0) + 1)
  }
}

const r002OverRatchet = r002Marks > R002_BASELINE
console.log(`\n【R-002】死码承认标记检查（判定型）`)
console.log(`  ${r002Marks} 处 \`[allow(dead_code)]\` · 棘轮 ${r002Marks}/${R002_BASELINE}`)
if (r002NoReason.length) {
  console.log(`  ✗ ${r002NoReason.length} 处承认标记没有理由：`)
  for (const loc of r002NoReason) console.log(`    ${loc}`)
  console.log('    ↳ 标记同行或其上 3 行内写 `// dead-code-allow R-002: 理由`（R-001 的理由同样认，理由必填）')
}
if (r002BadPlan.length) {
  console.log(`  ✗ ${r002BadPlan.length} 处承认没挂到计划步（04 §3.1）：`)
  for (const loc of r002BadPlan) console.log(`    ${loc}`)
  console.log('    ↳ 理由尾缀写 `；04 §3.1 批N 接线后摘除`——批号取自 04 §3.1 清偿批次表')
}
if (r002OverRatchet) {
  console.log(`  ✗ 承认标记 ${r002Marks} > 基线 ${R002_BASELINE} —— 只许降：`)
  console.log('    ↳ 接线一批就摘一批；新出现的死码要么删掉、要么接上，不许再加 allow')
}
if (!r002NoReason.length && !r002BadPlan.length && !r002OverRatchet) {
  console.log('  ✓ 标记均带理由并挂到计划步，数量未超基线')
  if (PLAN_04) {
    const tally = [...r002ByBatch].sort((a, b) => a[0].localeCompare(b[0], 'zh'))
    console.log(`  · 各清偿批待摘：${tally.map(([k, n]) => `${k} ${n}`).join(' · ')}`)
  }
}

const lines = dead.reduce((n, f) => n + lineCount(f), 0)
console.log(`\n合计可移除：${dead.length} 文件 / ${lines} 行`)
process.exit(dead.length || rustUnused.length || r002NoReason.length || r002BadPlan.length || r002OverRatchet ? 1 : 0)
