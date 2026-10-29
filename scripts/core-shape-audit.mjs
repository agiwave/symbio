#!/usr/bin/env node
/**
 * core-shape-audit — `symbio_core` 的**公共形状**只许收窄（判定型）
 *
 * ## 它判什么
 *
 * 三格棘轮，全部来自**内核**（DE-023 A2）的五个封闭语法 trait：
 *
 * | 编号 | 格 | 判什么 | 它拦的是哪一种膨胀 |
 * |---|---|---|---|
 * | CS-001 | `traits` | **内核五 trait**（Store / Capability / ModelProvider / VdfsProvider / Registry）的声明数（冻结 5） | **新增接缝**：每多一个内核抽象，就多一处「所有实现方都得知道」的地方。域 trait 不占此格（DE-023 A2 收窄口径） |
 * | CS-001 | `methods` | 上述五 trait 的 `fn` 成员总数（冻结 17） | **契约变宽**：既有接缝上挂新方法，影响面 = 全部实现方 |
 * | CS-001 | `defaults` | 其中**带默认体**的成员数（冻结 2） | **接口上长实现**：行为代码住进架构模块 |
 * | CS-002 | —— | 放宽上面任何一个数字，必须写 `core-shape-allow <格>: ADR-NNN <非空理由>`，且那个 ADR 文件在 `docs/003-design/` **能打开** | **改一个数字**看起来像维护，而它实际是在改架构 |
 * | CS-003 | —— | core 生产代码里 `crate::plugins` 引用数 = 0（**全 core**） | **实现依赖插件**：内核一旦认某个插件的实现坐标，停用/替换那个插件就不再是纯装配动作。依赖方向不随内核边界变化，故不收窄 |
 * | CS-004 | —— | **内核文件集**（`kernel.rs` 的 `core-kernel-files:` 标记）里插件 id 字面量数只许降 | **词汇回流**：在内核写 `"session"` / `"vdfs"` 就是在语法里认一个插件名字——加插件/改 id 因此又变成内核改动（[DE-023](../docs/003-design/023-内核边界与扩展局部性.md "DE-023-内核边界与扩展局部性") D-179） |
 * | CS-005 | `kernelEvents` / `cellsRows` | 内核业务事件常量数、`CELLS` 业务行数只许降 | **事件词汇仍住在语法里**：新事件要能被写闸认，却仍得改内核的两张手写表。声明化（`EventDecl`）之后这两格只许降（[DE-024](../docs/003-design/024-能力横向扩充的通路.md "DE-024-能力横向扩充的通路") D-197 / D-200）；这两格的**目标是零**（D-203 裁决三）——没到零的每一条都是待迁存量，CS-006 逐名点出它该随谁走 |
 * | CS-006 | —— | 逐条内核业务事件名核**消费方分布**：报出「≥2 crate 消费」与「core 读者」两类各几条，第三种（恰好一个消费方且 core 不读）判红 | **数字会骗人**：CS-005 只数条数，一条「只有一个插件读、core 已经不读」的名字躲在数字里谁也看不见，而它恰恰就是当批可迁的形状。这条把判据逐名落到代码；`core 读者` 那一类**不是**归属证据，是待迁存量——名字随那条读者一起迁出（DE-024 D-203） |
 * | CS-008 | —— | core / kernel 的**生产代码**里事件名字面量数 = 0（首段闭集 = `Entity` 的变体；`#[cfg(test)] mod x;` 的夹具文件剔出判据面） | **名字从第二条路回来**：CS-005 / CS-006 的输入都是内核那张表，表归零之后两者都没有输入，而 `match e.kind.as_str() { "chat.assistant.final" => … }` 照样能让「加一条事件」变成改架构模块。这条是 D-203 承诺的翻转，落成独立一格而不是翻转 CS-006（那会让判据在归零后空转） |
 * | CS-009 | `structFields` / `enumVariants` | core + kernel **生产代码**里 `pub struct` 的字段总数、`pub enum` 的变体总数，两格都只许降 | **数据面变宽**：CS-001 只数 trait / `fn` / 默认体，`CapabilityMeta` 的字段、`Entity` 的变体、`FactRole` 的变体都不在它的判定面里——内核可以靠数据面持续变宽而全绿（DE-025 D-212） |
 * | CS-010 | `turnStateFields` | 控制面草稿 `TurnState` 的字段数只许降（冻结 21） | **草稿变宽**：CS-009 只看 core / kernel 的**公共**数据面，而草稿是 loop 插件内部的 `pub(crate)`，一格都看不见；每多一格就多一份只有某条路径知道的状态（DE-022 D-226） |
 *
 * CS-004 的判定范围是 `kernel.rs` 声明的**内核文件集**（A2 之后 core ≠ 内核）：
 * 域文件（authz / chat / plugin / event_bus / projection …）的插件 id 字面量不是内核
 * 词汇，随 C 的阶段迁出，只在汇总行里报数、不设门禁。文件集之外的 `pub trait`
 * 也不占 CS-001（域 trait 不占格）；`crate::plugins` 引用仍按全 core 判（CS-003）。
 *
 * 表里的冻结值是**现值**，owner 是本文件的 `SHAPE_BASELINE`（下面那一行）；放宽一格
 * 必须按 CS-002 在它旁边留一条 `core-shape-allow`，并有一条能打开的 ADR 论证「必需」。
 *
 * ## 为什么需要它（2026-10-12 的实测缺口）
 *
 * 2026-10-12 那一批「`VdfsProvider` 去样板」的处方（把单点分发推广成默认实现）要把
 * `symbio_core::VdfsProvider` 从**一个方法**扩成**十个方法**。按当时全部门禁的口径算一遍：
 *
 * - `core-export-audit` C-001 / C-002：动的是那个符号**内部**，根出口一个字符没改；
 * - C-003：`VdfsProvider` 早就是多消费方导出，它数的是**符号表**（Map 的键），
 *   方法数不在其视野内——符号 +0，形状 ×10；
 * - `dead-code` / `core-naming` / `no-direct-call` / `mechanism-audit`：判据里都没有
 *   「一个 trait 有几个成员」这一项。
 *
 * ⇒ 一次把核心契约面扩到十倍、并在接口上多挂 9 个默认体的改动，**门禁全绿也能提交**。
 * 而这条纪律本仓早就写了：`08 §0 纪律 3`「架构改动必须逐条论证『必需』」，登记条目
 * [SP-007](../docs/005-spec/007.md)——它的判据写的是「论证不出即删（**人工核对**）」。
 * 于是「架构改动」的全部约束，取决于执行的人**记得**去核对；而批次执行时最不缺的就是
 * 合理化：「加 9 个方法是实现层的样板收敛，不算架构改动」。
 *
 * **本脚本把这条纪律从「记得去看」变成「改了就红」。** 它不判某个 trait 该不该宽——那是
 * 论证的事；它只保证：让核心形状变宽，必须显式触碰一处基线数字，并留下一条能被人打开的
 * 决策。红不红不由执行者判断。
 *
 * ## 为什么只有这几格（口径边界，别顺手加）
 *
 * - **不判行数**：`08 §3.3` 写着「加了数据 / 加了调用点 / 改了文档，都不是架构改动」，
 *   而契约加字段是数据变化——行棘轮会对它红，那是误报；一个只会误报的守卫最后一定被人
 *   喂豁免喂到失效（本仓为此写过一整节，见 `core-export-audit.mjs` 的 W1 注）。
 * - **不判 `impl` 数 / 宏**：宏展开的形状静态读不到（`08 §3` 的判据本来也不看这个）。
 * - **枚举变体与 struct 字段（CS-009）当初明确「不顺手扩」，后来单独论证了一次**：
 *   原文写的是「变体**是**形状量，但它与 trait 宽度不是同一个失效面。要加就单独论证
 *   一次，不在本批顺手扩判据面」。DE-025 D-212 就是那次论证——`CapabilityMeta` 的字段、
 *   `Entity` 的变体都是这样长回来的，而 CS-001 一格都看不见。它因此是**第二组格**
 *   （`structFields` / `enumVariants`），不与 trait 三格混在一个基线里：两组量的失效面
 *   不同，混在一起会让「哪一格涨了」在数字里看不出来。
 * - **控制面草稿（CS-010）与内核数据面（CS-009）也必须分两组**：CS-009 只量 core /
 *   kernel 的**公共**数据面，`TurnState` 是 loop 插件的 `pub(crate)` 草稿，公共面一格都
 *   碰不到它。它的失效面与内核同形——每多一格就多一份**只有某条路径知道**的状态——所以
 *   单独立一格（DE-022 D-226），而不是顺手塞进 CS-009 的基线。
 *
 * ## 与既有判据的分工（一个事实只有一个 owner）
 *
 * | 谁 | 判什么 |
 * |---|---|
 * | 本脚本 | core 公共**形状**的量（三格）+ 放宽痕迹必须挂得住一条决策 |
 * | `core-export-audit`（C-001…C-003） | 根出口的**归属**：什么能出现在公共面上、谁在用 |
 * | `gate.d/35-baseline` | 基线数字本身**不得比上一版更松**（比对 git 基准版本）；本脚本的三格登记在它的 `RATCHETS` 清单里，`CORE_SHAPE_BASELINE` 的环境变量覆盖由它兜 |
 * | [SP-007](../docs/005-spec/007.md) | 纪律**条文**本身（owner 在 `08 §3`），本脚本是它的判据执行方 |
 *
 * CS-002 与 `35-baseline` 各判半条：那边只发现「数字变松了」（红不红取决于有没有豁免注释），
 * 这边判「这条豁免**够不够格**」。两边合起来才是完整的「放宽 = 写决策」。
 *
 * ## 已知边界（写清楚，免得把绿灯当成证明）
 *
 * 判定基于**去注释、剔测试模块后的文本**，不是 AST：
 * - `pub trait` 必须写成 `pub trait <名>`（`pub unsafe trait` 读不到）；
 * - 宏生成的 trait / 成员读不到（本仓的 `define_string_key!` 生成的是 impl，不是 trait）；
 * - 成员签名扫描**不跟踪尖括号**（`->` 里那个 `>` 会把括号计数拖成负数）——代价是
 *   「泛型参数里带 `{`」这种罕见形状会被看成带默认体，**只会多数不会少数**，棘轮方向上安全；
 * - 它挡的是「顺手改宽」，不挡「刻意放宽」——后者正是 CS-002 要求留决策的原因。
 *
 * ## 用法
 *   node scripts/core-shape-audit.mjs              # 判定
 *   node scripts/core-shape-audit.mjs --verbose    # 逐 trait 形状 + 逐条内核事件名的消费方
 *   node scripts/core-shape-audit.mjs --churn      # 只出「近 50 次提交的内核改动占比」报告
 *   node scripts/core-shape-audit.mjs --root=<dir> # 指向别处的仓库（测试用）
 * 退出码：0 = 通过，1 = 违规。
 *
 * 环境变量 `CORE_SHAPE_BASELINE=<traits>:<methods>:<defaults>` **只给回归测试用**（真仓门禁
 * 不设）；它不动任何文件就能把上限放到天边，所以 `gate.d/35-baseline.mjs` 另判一次
 * 「覆盖值不得比默认值更松」。`VOCAB_BASELINE=<kernelEvents>:<cellsRows>` 同理；
 * `CORE_DATA_BASELINE=<structFields>:<enumVariants>`、`TURN_STATE_BASELINE=<字段数>` 同理。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns CS core 形状
// @codes CS-001 CS-002 CS-003 CS-004 CS-005 CS-006 CS-008 CS-009 CS-010

import fs from 'node:fs'
import path from 'node:path'
import { execSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'
import { hasDecisionAnchor as anchorExists } from './decision-anchors.mjs'
import { isTestFile, walk } from './core-surface.mjs'
import { blankComments, blankTestModules, matchBrace } from './rust-scan.mjs'

const argv = process.argv.slice(2)
const VERBOSE = argv.includes('--verbose')
const CHURN = argv.includes('--churn')
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

/**
 * `--churn`：最近 N 次非 merge 提交里，`crates/symbio-core/src/` 的改动占比。
 *
 * **报告，不设门禁**（[DE-023](../docs/003-design/023-内核边界与扩展局部性.md "DE-023-内核边界与扩展局部性") A1）：
 * 单次提交的占比没有阈值可定——修内核 bug 的提交本就该 100% 落在内核；判据是**走向**
 * （扩展类提交占比趋 0）。形状由 CS-001…CS-004 判，走向由本报告看，两者不互相冒充。
 *
 * 不落 `CURRENT.md`：那份事实表只收「离线可重生成的代码投影」（`facts/README.md`），
 * 而 git 历史每次提交都变——写进去会让门禁每轮都「修复 1 个文件」，把漂移判据淹掉。
 */
function printChurn(windowSize = 50) {
  let raw
  try {
    raw = execSync(`git log -${windowSize} --no-merges --numstat --format=%x1e`, {
      cwd: ROOT,
      encoding: 'utf8',
      maxBuffer: 32 * 1024 * 1024,
    })
  } catch {
    console.log(yellow('内核改动占比：`git log` 不可读（浅克隆 / 无 git），本节缺数'))
    return
  }
  let commits = 0
  let coreCommits = 0
  let total = 0
  let core = 0
  let counted = false
  for (const line of raw.split('\n')) {
    if (line.trim() === '\x1e') {
      commits += 1
      counted = false
      continue
    }
    const m = line.match(/^(\d+|-)\t(\d+|-)\t(.+)$/)
    if (!m) continue
    const n = (m[1] === '-' ? 0 : Number(m[1])) + (m[2] === '-' ? 0 : Number(m[2]))
    total += n
    if (m[3].startsWith('crates/symbio-core/src/')) {
      core += n
      if (!counted) {
        coreCommits += 1
        counted = true
      }
    }
  }
  if (commits === 0) {
    console.log(yellow('内核改动占比：窗口内没有提交，本节缺数'))
    return
  }
  const pct = (a, b) => (b === 0 ? '0.0' : ((a / b) * 100).toFixed(1))
  console.log(`内核改动占比（最近 ${commits} 次非 merge 提交，报告不设门禁）`)
  console.log(
    `  动过内核的提交：${coreCommits} / ${commits}（${pct(coreCommits, commits)}%）`,
  )
  console.log(`  内核改动行：${core} / ${total}（${pct(core, total)}%）`)
  console.log(dim('  判据是走向：扩展类提交的内核占比应逐步趋 0。'))
}

if (CHURN) {
  printChurn()
  process.exit(0)
}

const CORE_REL = 'crates/symbio-core/src'
/** 内核 crate（D-188 C2a）：文件集标记住 `lib.rs`，与 core 的 `kernel.rs` 合并判定。 */
const KERNEL_REL = 'crates/symbio-kernel/src'
/** 本脚本自己：CS-002 读它源码里的放宽痕迹 */
const SELF_REL = 'scripts/core-shape-audit.mjs'

/**
 * 决策条目是否存在：`D-NNN` 是设计文档内的锚点（`### D-NNN <标题>`），不是独立文件。
 * 扫描口径（扫哪些根、认哪种锚点）住 `decision-anchors.mjs`——三个判「豁免挂没挂决策」
 * 的审计共用一份，抄本不同会让同一条豁免在这边绿、那边红。
 */
const hasDecisionAnchor = (id) => anchorExists(id, ROOT)

/**
 * 形状基线（2026-10-12 冻结，决策与理由见 [D-055](../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
 * 三格都是**上限**，只许降。数字本身不是判据，**改数字才是**：任何让形状变宽的改动都会
 * 撞红，而把它继续推下去只有一条路——改这一行的数字，那又会要求 CS-002 的一条
 * `core-shape-allow <格>: ADR-NNN <理由>`，且那个 ADR 文件必须真实存在。
 * 于是「顺手把架构改宽一点」在机制上等于「写一份决策」，而不是「改一个数字」。
 *
 * ⚠️ 键序就是 `CORE_SHAPE_BASELINE` 的位置口径（`35-baseline` 的 env 覆盖判定按它 zip），
 * 也必须是 `35-baseline` 那条提取函数认的键名；任何一边单改都会红。
 */
// 内核五 trait 是**封闭语法**（DE-023 §1）：它们的形状只许收窄。
// 其余 trait 是域协议（Plugin / LlmAdapter / DeltaSink / …），随 B2/C 迁出内核，
// 不占这条棘轮——在那里加成员不等于「内核接缝变宽」。
const KERNEL_TRAITS = ['Store', 'Capability', 'ModelProvider', 'VdfsProvider', 'Registry']

const SHAPE_BASELINE = process.env.CORE_SHAPE_BASELINE
  ? (() => {
      const [traits, methods, defaults] = process.env.CORE_SHAPE_BASELINE.split(':').map((n) => Number(n))
      return { traits, methods, defaults }
    })()
  : { traits: 5, methods: 17, defaults: 2 }

/**
 * CS-005 词汇棘轮（[DE-024](../docs/003-design/024-能力横向扩充的通路.md "DE-024-能力横向扩充的通路") D-200）。
 *
 * 两格都只许降：
 * - `kernelEvents`：内核 `event/mod.rs` 里**业务事件**常量数（值是点分名、且不是协议五格）；
 * - `cellsRows`：`event/grid.rs` 的 `CELLS` 里业务行数（不含协议五格）。
 *
 * 新事件从 `EventDecl` 进装配表，**不再需要动这两张表**。这两格的**目标是零**（DE-024 D-203
 * 裁决三）：内核只该留协议五格，其余每一条业务名字都该随它的 owner 声明。眼下还没归零，
 * 是因为 core 仍有一条按名字读事实的投影/判据留在原地——那条读者是**待迁存量**，不是
 * 这条名字的归属证据（把「守卫要读」当成「名字该住内核」的裁决已被 D-203 撤回：守卫读的是
 * 格位与角色，词汇是数据，随 owner 交来）。逐条名字的出口见 DE-024 的 D-203 迁移路线。
 *
 * - **P1c 首批迁出**（2026-10-27）：`session.config` / `turn.stage` / `authz.granted` /
 *   `memory.recalled` / `task.ready`（loop）与 `system.triggered` / `system.health`
 *   （session）共 7 条名字随 owner 的 `EventDecl` 走，内核常量与 `CELLS` 行同步退役
 *   ⇒ 40/38 → 33/32。`task.ready` 本来就没有 `CELLS` 行，所以行数只减 6。
 *
 * - **P1c 第二批迁出**（同日）：`conation.expressed` 的读侧投影（`ConationView`）随**唯一
 *   生产消费方**下沉 `plugins/session`，core 不再按这个名字读 ⇒ 内核常量与它的 `CELLS` 行
 *   一起退役 ⇒ 33/32 → 32/31（判据见 DE-024 D-201）。
 *
 * - **P1c 第三批迁出**（同日）：`recall` 投影（连同 `RecallView` / `RecallEntry` 与排序内核
 *   `recall_score.rs`）下沉 `plugins/loop`——它的读侧全在本插件。`memory.consolidated` /
 *   `memory.forgotten` 自此只有 loop 读，两条内核常量与两行 `CELLS` 同批退役
 *   ⇒ 32/31 → 30/29。`memory.encoded` 当时留内核：`plugins/session` 的 `stats` 是它的第二个
 *   生产读方，两个插件同时消费的名字不能被任一插件独占（E-009）——它的出口是**共享词汇表**
 *   （D-203 裁决三路线③），不是继续住在内核。
 *
 * - **D-203 首批迁出**（同日）：core 的终止性判据 `rework_bounded` 改读 `CountsRework`
 *   角色之后，`task.rework_created` 的唯一生产消费方只剩 `plugins/loop`（CS-006 逐名点出）
 *   ⇒ 名字、`EventDecl` 行与 `CELLS` 行同批归 loop ⇒ 30/29 → 29/28。
 *
 * - **D-204 首批迁出**（同日）：跨插件名字的登记口改到各插件自己的 `PLUGIN.yml`
 *   （`shared_events:` → 生成 `symbio-plugin-api/src/events.rs`），「两个插件同时消费 ⇒
 *   只能住内核」这条 E-009 旧推论作废。core 完全不读的两条先走：`memory.encoded` 与
 *   `task.controlled`（原 `control.opened`，随 owner 的拼写约定改名）⇒ 内核常量与两行
 *   `CELLS` 同批退役 ⇒ 29/28 → 27/26（判据见 CS-007）。
 *
 * - **D-205 首批迁出**（同日）：core 的读侧投影不再把业务名字写死在折叠里——
 *   `calibration()` 改 `calibration_for(calibrated)`，名字由调用方在构造期交进来
 *   （owner 交自家常量，第二个消费方交清单生成物）。`memory.calibrated` 因此登记进
 *   loop 的 `shared_events`，内核常量与 `CELLS` 行同批退役 ⇒ 27/26 → 26/25。
 *   这一条拆掉的是「core 读者」那一类 16 条名字的**共同**理由：投影留在 core（两个
 *   插件同时消费），但认识名字的不是投影，是交出名字的那一方。
 *
 * - **D-206 任务族迁出**（同日）：多名字折叠收一张按族的结构体（core 的 `readyset_for(vocab, cap)`
 *   与会话插件的 `autonomy` 折叠都取 `TaskVocab`，字段按算法角色命名——终态与租约同落
 *   `Task × Asserted`，按格子命不出五格）。`task.opened` / `task.progress` / `task.held` /
 *   `task.asserted` / `task.assigned` 五条名字登记进 loop 的 `shared_events`，内核五条常量与
 *   四行 `CELLS` 同批退役 ⇒ 26/25 → 21/21。「读一个折叠就要交出整族」是这张表的形状带来的：
 *   session 交出 `TASK_VOCAB` 时，`task.assigned` 那种只有 loop 写的名字也有了真消费方，
 *   CS-007 因此不必为多名字折叠开一条口径。同一条「消费方分布」判据在这一族给出两个处置：
 *   `readyset` 被 loop 与 session 同时消费 ⇒ 留 core；`autonomy` 只有 session 读 ⇒ 随消费方
 *   搬进会话插件（C-004 的 `one` 格因此回到 11，见那份脚本的记录）。
 *
 * `VOCAB_BASELINE=<kernelEvents>:<cellsRows>` 与 `CORE_SHAPE_BASELINE` 同一条纪律：
 * 只给回归测试用，`35-baseline` 另判覆盖值不得比默认值更松。
 */
const VOCAB_BASELINE = process.env.VOCAB_BASELINE
  ? (() => {
      const [kernelEvents, cellsRows] = process.env.VOCAB_BASELINE.split(':').map((n) => Number(n))
      return { kernelEvents, cellsRows }
    })()
  : { kernelEvents: 0, cellsRows: 0 }

/**
 * CS-009 数据面棘轮（[DE-025](../docs/003-design/025-能力包与扩展面.md "DE-025-能力包与扩展面") D-212）。
 *
 * 两格都只许降，量的是 core + kernel **生产代码**里的公共数据形状：
 *
 * - `structFields`：`pub struct` 的字段总数（含 tuple struct 的位置字段）；
 * - `enumVariants`：`pub enum` 的变体总数。
 *
 * ## 它补的是哪一块盲区
 *
 * CS-001 数 trait 数 / 成员数 / 默认体数，**数据面一格都看不见**：给
 * `CapabilityMeta` 加一个布尔位、给 `Entity` 加一个变体、给 `FactRole` 加一档角色，
 * 三次都是「内核变宽」，而三次都能全绿通过。D-212 把这条翻过来——加一个变体要动
 * 基线数字，而动数字要求一条能打开的决策。
 *
 * ## 为什么不逐 struct / 逐 enum 设上限
 *
 * 逐项上限要求维护一张按名字的表，而那张表本身就是「加一个 struct 要改脚本」的
 * 又一处中心文件——正是本批要拆的形状。总量两格是**同一件事**的等价判据：任何
 * 新增都会抬高总数，而任何删除都会降下来（降下来时脚本提示同步基线，余量不白留）。
 * 逐项清单不进判据，只在 `--verbose` 里出，用来回答「涨的那一个是谁」。
 *
 * ## 口径边界（写清楚，免得把绿灯当证明）
 *
 * - 只认**完全 public**（`pub struct` / `pub enum`）：`pub(crate)` 不在公共面上；
 * - 私有模块里的 `pub` 项仍计入——与 CS-001 同一口径（它数 `pub trait` 也不看模块
 *   可见性），方向上只会多数、不会少数，棘轮安全；
 * - 字段 / 变体按**顶层逗号**切分：泛型参数、函数指针、属性里的逗号都靠括号深度跳过；
 * - 测试文件与 `#[cfg(test)]` 模块剔出判据面（夹具不是长期数据面）。
 *
 * `CORE_DATA_BASELINE=<structFields>:<enumVariants>` 与另外两条同一条纪律：只给回归
 * 测试用，`35-baseline` 另判覆盖值不得比默认值更松。
 */
// core-shape-allow enumVariants: D-209 领域轴必须有一个扩展位——`Domain` 是一次性架构决策，换的是此后 notes / calendar 这类新领域的零 kernel diff；被否候选（`Entity` 改开放字符串会丢 F5 闭集、塞进 `Memory`/`Task` 会让读侧靠载荷猜）见 DE-025
const DATA_BASELINE = process.env.CORE_DATA_BASELINE
  ? (() => {
      const [structFields, enumVariants] = process.env.CORE_DATA_BASELINE.split(':').map((n) => Number(n))
      return { structFields, enumVariants }
    })()
  : { structFields: 565, enumVariants: 245 }

/**
 * CS-010 控制面草稿的字段棘轮（[DE-022](../docs/003-design/022-行表与派发面.md "DE-022-行表与派发面") D-226）。
 *
 * `TurnState` 是 loop 插件在幕内借来借去的草稿：请求轮次、锚点、请求级缓存与产物寄存位。
 * 它不在公共面上（`pub(crate)`），CS-009 一格都看不见；而它是控制面里**唯一一处会自己
 * 长字段**的地方——每多一格，就多一份只有某条路径知道的状态，而事实面本可以回答它。
 *
 * 口径按[字段身份]判，不按「压数字」：**事实的第二份拷贝 ⇒ 退出，读日志**；产物寄存位
 * ⇒ 允许（`D-123` 决策 5 允许「不交回」，不是「不留痕」，最终仍要落格）；请求级缓存 ⇒
 * 允许，但注释要能指出「为什么它不是第二份状态」。三种身份的逐字段处置见 D-226。
 *
 * 基线 = 处置表留下的字段数，也就是目标值（21）。只许降：删掉一格就同步改这里；加一格
 * 必须先在 `docs/003-design/` 立一条决策，再在基线旁写
 * `core-shape-allow turnStateFields: D-NNN <理由>`（CS-002 校验那条决策能打开）。
 *
 * `TURN_STATE_BASELINE=<字段数>` 与另外三条同一条纪律：只给回归测试用，
 * `35-baseline` 另判覆盖值不得比默认值更松。
 */
const TURN_STATE_REL = 'crates/symbio-plugin-loop/src/chat_loop/state.rs'
const TURN_STATE_BASELINE = process.env.TURN_STATE_BASELINE
  ? Number(process.env.TURN_STATE_BASELINE)
  : 21

const CELL_LABEL = {
  traits: '`pub trait` 数（接缝数）',
  methods: '`fn` 成员总数（契约面宽度）',
  defaults: '带默认体的成员数（接口上的实现）',
}

/** 放宽痕迹的读法：与 `35-baseline` 的 `waiverReader('core-shape-allow')` 同一条拼写约定 */
const WAIVER_RE = /^ {0,2}\/\/\s*core-shape-allow\s+([A-Za-z0-9_]+)\s*:\s*(\S[^\n]*)$/gm
const ADR_REF_RE = /(?<!-)(?:D|ADR)-(\d+[a-z]?)/g

const errors = []
const report = (rule, msg) => errors.push({ rule, msg })

// ==================== 读形状 ====================

/**
 * 数一个 trait 体的形状（体已剥掉外层花括号）。
 *
 * 成员 = 深度 0 上的 `fn`；「带默认体」= 该 `fn` 的签名以 `{` 收尾而不是 `;`。命中默认体后
 * 用 `matchBrace` 跳过整个体，否则体内的 `fn`（闭包 / 局部函数）会被当成成员。
 */
function traitShape(body) {
  let fns = 0
  let defaults = 0
  let depth = 0
  for (let i = 0; i < body.length; i++) {
    const c = body[i]
    if (c === '{' || c === '(' || c === '[') {
      depth++
      continue
    }
    if (c === '}' || c === ')' || c === ']') {
      depth--
      continue
    }
    if (depth !== 0 || c !== 'f' || !body.startsWith('fn', i)) continue
    if (!/[^A-Za-z0-9_]/.test(body[i - 1] ?? ' ')) continue
    const m = /^fn\s+([A-Za-z0-9_]+)/.exec(body.slice(i))
    if (!m) continue
    fns++
    let d = 0
    let open = -1
    for (let j = i + 2; j < body.length; j++) {
      const k = body[j]
      if (k === '(' || k === '[') d++
      else if (k === ')' || k === ']') d--
      else if (d === 0 && k === '{') {
        open = j
        break
      } else if (d === 0 && k === ';') break
    }
    if (open >= 0) {
      defaults++
      i = matchBrace(body, open)
    }
  }
  return { fns, defaults }
}

/**
 * 内核文件集：`core-kernel-files:` 标记是**唯一声明处**。
 *
 * 为什么按文件而不是按目录：内核是**语法**的集合，语法声明散在域文件里
 * （`capability/mod.rs` 有 `Capability`/`Registry`，`vdfs/provider.rs` 有 `VdfsProvider`），
 * 按目录划会把整个域的行为也算进内核。文件集只回答「哪几个文件是内核边界」。
 *
 * C2a 之后标记分居两处：`symbio_core/kernel.rs`（仍在 core 的语法）与
 * `crates/symbio-kernel/src/lib.rs`（已搬进内核 crate 的子集）。合并后的文件集才是判据范围。
 *
 * 夹具仓库没有门面 ⇒ 返回 null，退回全 core 口径（夹具只验棘轮本身）。
 */
function readKernelScope(dir, fileName) {
  const file = path.join(dir, fileName)
  if (!fs.existsSync(file)) return null
  const m = fs.readFileSync(file, 'utf8').match(/^[ \t]*\/\/[ \t]*core-kernel-files:[ \t]*(.+)$/m)
  if (!m) return null
  const rels = m[1].trim().split(/\s+/).filter(Boolean)
  const abs = new Set()
  const missing = []
  for (const rel of rels) {
    const p = path.join(dir, rel)
    if (!fs.existsSync(p)) missing.push(rel)
    abs.add(p)
  }
  return { rels, abs, missing }
}

/**
 * core 生产侧每个 `pub trait` 的形状。
 *
 * 测试文件与 `#[cfg(test)]` 模块都不算——口径与 `core-export-audit` C-003 一致：
 * **测试不是架构消费方**，测试里的 trait 形状也不是长期接口。
 * 传入内核文件集时只量内核（`kernelScope` 之外的文件是域行为，随 C 迁出）。
 */
function measureCoreShape(coreDir, kernelFiles = null) {
  const all = []
  for (const file of walk(coreDir)) {
    if (!file.endsWith('.rs') || isTestFile(file)) continue
    if (kernelFiles && !kernelFiles.has(file)) continue
    const txt = blankTestModules(blankComments(fs.readFileSync(file, 'utf8')).join('\n'))
    for (const m of txt.matchAll(/\bpub\s+trait\s+([A-Za-z0-9_]+)/g)) {
      const open = txt.indexOf('{', m.index)
      if (open < 0) continue
      const body = txt.slice(open + 1, matchBrace(txt, open))
      all.push({
        name: m[1],
        file: path.relative(ROOT, file).split(path.sep).join('/'),
        ...traitShape(body),
      })
    }
  }
  const rows = all.filter((r) => KERNEL_TRAITS.includes(r.name))
  rows.sort((a, b) => b.fns - a.fns || a.name.localeCompare(b.name))
  return {
    rows,
    all,
    values: {
      traits: rows.length,
      methods: rows.reduce((s, r) => s + r.fns, 0),
      defaults: rows.reduce((s, r) => s + r.defaults, 0),
    },
  }
}

/** 合并多个扫描根的 trait 形状（C2a 起内核横跨 core 与 kernel 两个 crate）。 */
function mergeShapes(parts) {
  return {
    rows: parts.flatMap((p) => p.rows),
    all: parts.flatMap((p) => p.all),
    values: {
      traits: parts.reduce((n, p) => n + p.values.traits, 0),
      methods: parts.reduce((n, p) => n + p.values.methods, 0),
      defaults: parts.reduce((n, p) => n + p.values.defaults, 0),
    },
  }
}

/**
 * 按**顶层逗号**切分（CS-009 的读法）。
 *
 * 逗号在下面几种位置时不是分隔符，全部靠深度跳过：泛型实参 `BTreeMap<K, V>`、
 * 函数指针 `fn(u8, u8) -> u8`、数组 `[u8; 2]`、字段属性 `#[serde(default, skip)]`。
 * `->` 里的 `>` 不算尖括号收尾（否则 `fn() -> u8` 之后整个深度的符号会翻负）。
 */
function topLevelSegments(body) {
  const out = []
  let depth = 0
  let start = 0
  for (let i = 0; i < body.length; i++) {
    const c = body[i]
    if (c === '<') depth++
    else if (c === '>') {
      if (body[i - 1] !== '-') depth--
    } else if (c === '(' || c === '[' || c === '{') depth++
    else if (c === ')' || c === ']' || c === '}') depth--
    else if (c === ',' && depth === 0) {
      out.push(body.slice(start, i))
      start = i + 1
    }
  }
  out.push(body.slice(start))
  return out.map((s) => s.trim()).filter(Boolean)
}

/**
 * 配对圆括号（tuple struct 的位置字段用）。
 *
 * 为什么不复用 `matchBrace`：那个只数花括号——从 `(` 出发会一路跑到**文件里下一个**
 * `{…}` 的收尾，把整个后续代码都当成字段（实测：`pub struct Pair(u8, u8);` 后面
 * 还有一个带体的 struct 时，字段数会被读成 3）。
 */
function matchParen(txt, openIdx) {
  let depth = 0
  for (let p = openIdx; p < txt.length; p++) {
    const c = txt[p]
    if (c === '(') depth++
    else if (c === ')') {
      depth--
      if (depth === 0) return p
    }
  }
  return txt.length
}

/**
 * core + kernel 生产侧的公共数据形状（CS-009）：逐个 `pub struct` / `pub enum` 量，
 * 汇总成两格。
 *
 * 与 `measureCoreShape` 分开的理由是**判据面不同**：那个只量内核文件集里的 trait，
 * 这个量两个 crate 的全部公共数据面（`pub(crate)` 不算——它不在公共面上）。合并成
 * 一个函数会让两组量的范围混在一处，而它们本来就该各自可读。
 */
function measureDataSurface(dir) {
  const rows = []
  for (const file of walk(dir)) {
    if (!file.endsWith('.rs') || isTestFile(file)) continue
    const txt = blankTestModules(blankComments(fs.readFileSync(file, 'utf8')).join('\n'))
    const rel = path.relative(ROOT, file).split(path.sep).join('/')
    for (const m of txt.matchAll(/\bpub\s+struct\s+([A-Za-z0-9_]+)/g)) {
      const open = txt.indexOf('{', m.index)
      const semi = txt.indexOf(';', m.index)
      const paren = txt.indexOf('(', m.index)
      // tuple struct：`pub struct Foo(u8, u8);`——位置字段同样进公共面。
      if (paren >= 0 && (open < 0 || paren < open) && (semi < 0 || paren < semi)) {
        const close = matchParen(txt, paren)
        rows.push({
          kind: 'struct',
          name: m[1],
          n: topLevelSegments(txt.slice(paren + 1, close)).length,
          file: rel,
        })
        continue
      }
      // `pub struct Foo;`（单元结构体）与声明到别处的写法：零字段，不计入（读不到体）。
      if (open < 0 || (semi >= 0 && semi < open)) continue
      const body = txt.slice(open + 1, matchBrace(txt, open))
      rows.push({ kind: 'struct', name: m[1], n: topLevelSegments(body).length, file: rel })
    }
    for (const m of txt.matchAll(/\bpub\s+enum\s+([A-Za-z0-9_]+)/g)) {
      const open = txt.indexOf('{', m.index)
      if (open < 0) continue
      const body = txt.slice(open + 1, matchBrace(txt, open))
      rows.push({ kind: 'enum', name: m[1], n: topLevelSegments(body).length, file: rel })
    }
  }
  return rows
}

/**
 * 控制面草稿 `TurnState` 的字段数（CS-010 的读法）。
 *
 * 字段切分与 CS-009 同一条（`topLevelSegments`：顶层逗号，泛型 / 函数指针 / 属性里的
 * 逗号靠深度跳过）；入口不同——草稿是 `pub(crate) struct`，不在 `pub struct` 的判定面里。
 * 读不到（文件不在 / struct 改名）返回 `null`：调用方按「判据失去输入」处理，不静默算 0。
 */
function countTurnStateFields(abs) {
  if (!fs.existsSync(abs)) return null
  const txt = blankTestModules(blankComments(fs.readFileSync(abs, 'utf8')).join('\n'))
  const m = /\bpub(?:\(crate\))?\s+struct\s+TurnState\s*\{/.exec(txt)
  if (!m) return null
  const open = txt.indexOf('{', m.index)
  if (open < 0) return null
  return topLevelSegments(txt.slice(open + 1, matchBrace(txt, open))).length
}

const coreDir = path.join(ROOT, CORE_REL)
if (!fs.existsSync(path.join(coreDir, 'lib.rs'))) {
  console.error(red(`✗ 读不出 core 范围：${CORE_REL}/mod.rs 不存在（root=${ROOT}）—— 判据没有输入，宁红不静默`))
  process.exit(1)
}
const kernelDir = path.join(ROOT, KERNEL_REL)
const scanDirs = [coreDir, ...(fs.existsSync(path.join(kernelDir, 'lib.rs')) ? [kernelDir] : [])]
const scopes = [
  readKernelScope(coreDir, 'kernel.rs'),
  scanDirs.includes(kernelDir) ? readKernelScope(kernelDir, 'lib.rs') : null,
].filter(Boolean)
// 没有门面（夹具仓库）⇒ 退回全 core 口径；有门面则只量文件集的并集。
const kernelAbs = scopes.length > 0 ? new Set(scopes.flatMap((s) => [...s.abs])) : null
const kernelMissing = scopes.flatMap((s) => s.missing)
if (kernelMissing.length > 0) {
  report(
    'CS-004',
    `内核文件集指向不存在的文件：${kernelMissing.join(' / ')} —— 门面声明的边界必须可解析，` +
      '否则 CS-001 / CS-004 会在一个缩小了的范围上恒绿（宁红不静默）',
  )
}
const shape = mergeShapes(scanDirs.map((d) => measureCoreShape(d, kernelAbs)))

// 封闭语法里少一个 trait 不是「收窄成功」，是判据失去输入：宁红不静默。
// 夹具仓库（没有宿主侧插件域）只造一两个 trait 验棘轮本身，不要求凑齐五格。
//
// 标记物取 `symbio-core/src/plugin/`：它是**宿主侧插件域**（目录 / 清单 / 遍历端点），
// 只有真仓有。D-191 之前标记写成 symbio/src/plugins/mod.rs（壳里的装配清单所在），拆分那次
// 改动把它替换成了常量 `false`——于是「五格齐全」与 CS-004 的字面量回流**同时静默失效**，
// 门禁全绿而两条判据没跑。回归测试（CS-004 那条）是唯一抓住它的地方。
const hasPlugins = fs.existsSync(path.join(ROOT, 'crates/symbio-core/src/plugin/mod.rs'))
if (hasPlugins) {
  for (const name of KERNEL_TRAITS) {
    if (!shape.rows.some((r) => r.name === name)) {
      report('CS-001', `内核 trait \`${name}\` 读不出来——封闭语法被删/改名，棘轮失去一格输入`)
    }
  }
}

// ==================== CS-001 形状棘轮 ====================

for (const [key, cap] of Object.entries(SHAPE_BASELINE)) {
  if (!Object.hasOwn(shape.values, key)) {
    report('CS-001', `基线里有一格 \`${key}\` 不是形状量（只有 ${Object.keys(shape.values).join(' / ')}）`)
    continue
  }
  if (!Number.isFinite(cap)) {
    report('CS-001', `基线格 \`${key}\` 不是数字（${cap}）—— 判据读不出即失效，宁红不静默`)
    continue
  }
  const now = shape.values[key]
  if (now > cap) {
    report(
      'CS-001',
      `核心形状变宽：${CELL_LABEL[key]} 实测 ${now} > 基线 ${cap} —— symbio_core 的公共形状只许收窄。` +
        `要让这一步成立，先在 \`docs/003-design/\` 立一条 ADR 逐条论证「必需」（08 §0 纪律 3，登记条目 SP-007；` +
        `被这份棘轮钉住的先例见 D-055），再在基线旁写 core-shape-allow ${key}: ADR-NNN <理由>。逐 trait 清单：--verbose`,
    )
  }
}

// ==================== CS-009 数据面棘轮 ====================

const dataRows = scanDirs.flatMap((d) => measureDataSurface(d))
const dataValues = {
  structFields: dataRows.filter((r) => r.kind === 'struct').reduce((s, r) => s + r.n, 0),
  enumVariants: dataRows.filter((r) => r.kind === 'enum').reduce((s, r) => s + r.n, 0),
}
const DATA_CELL_LABEL = {
  structFields: '`pub struct` 字段总数（公共数据面宽度）',
  enumVariants: '`pub enum` 变体总数（封闭取值面的宽度）',
}

for (const [key, cap] of Object.entries(DATA_BASELINE)) {
  if (!Object.hasOwn(dataValues, key)) {
    report('CS-009', `基线里有一格 \`${key}\` 不是数据面量（只有 ${Object.keys(dataValues).join(' / ')}）`)
    continue
  }
  if (!Number.isFinite(cap)) {
    report('CS-009', `基线格 \`${key}\` 不是数字（${cap}）—— 判据读不出即失效，宁红不静默`)
    continue
  }
  const now = dataValues[key]
  if (now > cap) {
    report(
      'CS-009',
      `内核数据面变宽：${DATA_CELL_LABEL[key]} 实测 ${now} > 基线 ${cap} —— ` +
        'core / kernel 的公共数据面只许降。新领域用 `Entity::Domain` 扩展位（DE-025 D-209），' +
        '新行为用 `CapabilityMeta.behaviors` 开放集合，都不该往内核加变体或字段。' +
        `确需放宽：先在 \`docs/003-design/\` 立一条决策逐条论证「必需」，再在基线旁写 ` +
        `core-shape-allow ${key}: D-NNN <理由>。逐项清单：--verbose`,
    )
  }
}

// ==================== CS-010 控制面草稿字段棘轮 ====================

const turnStateAbs = path.join(ROOT, TURN_STATE_REL)
/** 判据有没有输入：文件在就算「该读它」，读到才算数；真仓（有宿主侧插件域）缺文件也红 */
const turnStateApplicable = fs.existsSync(turnStateAbs) || hasPlugins
const turnStateFields = countTurnStateFields(turnStateAbs)
if (!Number.isFinite(TURN_STATE_BASELINE)) {
  report(
    'CS-010',
    `基线 \`turnStateFields\` 不是数字（${TURN_STATE_BASELINE}）—— 判据读不出即失效，宁红不静默`,
  )
} else if (turnStateFields === null) {
  if (turnStateApplicable) {
    report(
      'CS-010',
      `读不出 \`${TURN_STATE_REL}\` 里的 \`TurnState\`——草稿被删 / 改名 / 搬走，` +
        '字段棘轮失去输入（夹具仓库没有宿主侧插件域时不判）',
    )
  }
} else if (turnStateFields > TURN_STATE_BASELINE) {
  report(
    'CS-010',
    `控制面草稿变宽：\`TurnState\` 字段数实测 ${turnStateFields} > 基线 ${TURN_STATE_BASELINE} —— ` +
      '草稿里每一格都该先回答「它是事实的第二份拷贝 / 产物寄存位 / 请求级缓存」' +
      '（逐字段处置与目标值见 DE-022 D-226）。要让新的一格成立，先在 `docs/003-design/` 立一条决策，' +
      '再在基线旁写 core-shape-allow turnStateFields: D-NNN <理由>',
  )
}

// ==================== CS-002 放宽必须挂得住一条决策 ====================

/**
 * 读本脚本**自己**源码里那几行放宽痕迹。
 *
 * 为什么要读自己：棘轮的放宽口就是「把 SHAPE_BASELINE 的数字改大」这一行，而那一行的
 * 合法性（有没有挂得住的决策）只有它自己知道。`35-baseline` 比对的是基准版本的数字，
 * 它只知道「变松了、旁边有没有注释」；注释**够不够格**在这里判——两半合起来才是
 * 「放宽 = 写一份能打开的决策」。
 *
 * 夹具仓库里可能没有本脚本 ⇒ 读不到就跳过（与 `core-surface` 的 `walk` 同一约定：
 * 路径不存在不是违规）。
 */
const waivers = []
const selfPath = path.join(ROOT, SELF_REL)
// 放宽口有三个基线对象（形状三格 + 数据两格 + 草稿一格），键名不重名；豁免注释只写格名，
// 因此校验面取并集——`core-shape-allow enumVariants: D-209 …` 与
// `core-shape-allow methods: D-077 …` 走同一条判定。
const WAIVABLE_KEYS = [
  ...Object.keys(SHAPE_BASELINE),
  ...Object.keys(DATA_BASELINE),
  'turnStateFields',
]
if (fs.existsSync(selfPath)) {
  const selfText = fs.readFileSync(selfPath, 'utf8').replace(/\r\n/g, '\n')
  for (const m of selfText.matchAll(WAIVER_RE)) {
    const [, key, rawReason] = m
    const reason = rawReason.trim()
    waivers.push({ key, reason })
    if (!WAIVABLE_KEYS.includes(key)) {
      report('CS-002', `\`core-shape-allow ${key}\` 放宽的不是基线的格（只有 ${WAIVABLE_KEYS.join(' / ')}）`)
      continue
    }
    const refs = [...reason.matchAll(ADR_REF_RE)].map((r) => r[1])
    if (refs.length === 0) {
      report(
        'CS-002',
        `放宽 \`${key}\` 的理由里没有 ADR 编号（\`${key}: ${reason}\`）—— 动棘轮数字必须挂一条**决策**。` +
          `「批次需要 / 顺手 / 待补 / 见对话」都不算`,
      )
      continue
    }
    for (const id of refs) {
      if (!hasDecisionAnchor(id)) {
        report(
          'CS-002',
          `放宽 \`${key}\` 引用了 \`D-${id}\`，但设计文档里没有任何 \`### D-${id}\` 条目 —— 指向不存在的决策的豁免不算豁免`,
        )
      }
    }
  }
}

// ==================== CS-003 / CS-004 kernel 纯度 ====================

/**
 * 插件的**目录名**就是插件 id 的词表来源：新增插件 = 新 crate，不需要改本脚本。
 * 词表为空（夹具仓库）⇒ CS-004 不适用（CS-003 照判）。
 *
 * D-191 后插件平铺在 `crates/symbio-plugin-<id>`：id 由 crate 名去前缀反推，
 * 不再是「`plugins/` 下的目录名」。旧写法 `crates/plugins` 在真仓永不存在 ⇒
 * 词表恒空 ⇒ CS-004 恒不判（同一类静默失效，见上方 `hasPlugins` 的注）。
 */
const PLUGIN_CRATE_PREFIX = 'symbio-plugin-'
const pluginsDir = path.join(ROOT, 'crates')
const pluginIds = fs.existsSync(pluginsDir)
  ? fs
      .readdirSync(pluginsDir, { withFileTypes: true })
      .filter((e) => e.isDirectory() && e.name.startsWith(PLUGIN_CRATE_PREFIX))
      .map((e) => e.name.slice(PLUGIN_CRATE_PREFIX.length))
      .sort()
  : []

/** 生产侧（core + kernel，剔测试模块 / 注释）的全文；CS-003 / CS-004 的输入。 */
const coreTexts = []
for (const dir of scanDirs) {
  for (const file of walk(dir)) {
    if (!file.endsWith('.rs') || isTestFile(file)) continue
    const text = blankTestModules(blankComments(fs.readFileSync(file, 'utf8')).join('\n'))
    coreTexts.push({ rel: path.relative(ROOT, file).split(path.sep).join('/'), text })
  }
}

/**
 * CS-004 的判定范围：内核文件集（两处门面的 `core-kernel-files:` 标记并集）。
 * CS-003 仍扫**全部生产代码**（core + kernel）——依赖方向（不认插件实现坐标）
 * 不随内核边界变化；域行为里出现插件 id 字面量不是内核纯度问题，随 C 迁出，只做报告。
 */
const purityTexts = kernelAbs
  ? coreTexts.filter((f) => kernelAbs.has(path.join(ROOT, f.rel)))
  : coreTexts


// ── CS-003 `crate::plugins` 引用 0 ──
const pluginRefs = coreTexts.flatMap((f) =>
  [...f.text.matchAll(/crate\s*::\s*plugins\b/g)].map(() => f.rel),
)
if (pluginRefs.length > 0) {
  report(
    'CS-003',
    `core 生产代码引用了插件实现 ${pluginRefs.length} 处（${[...new Set(pluginRefs)].join(' / ')}）——` +
      '内核只该经 trait / 声明与插件交互（DE-023 D-179）；注释里提到不算，但代码路径算',
  )
}

// ── CS-004 插件 id 字面量只许降 ──
/**
 * 逐文件冻结存量（2026-10-26 实测值）。**只许降**：数字变小就同步改这里，
 * 变大 = 内核又认了一个插件名字。整张表的目标是空表。
 *
 * 口径：`"<id>"` / `"<id>:` / `"<id>/` 三种字面量起点，逐个插件目录名匹配；
 * 测试模块与注释已剥离（测试里按名造插件是对的）。
 */
const NOUN_BASELINE = {
  // 2026-10-26（DE-023 C2b）：判定范围随语法整份迁到 `crates/symbio-kernel/src`（标记在
  // `crates/symbio-kernel/src/lib.rs`）。域词汇里只剩两处**随类型一起**搬进内核的存量：
  //  - `chat/principal.rs`：`from_legacy` / `tool_principal` 的工具族前缀表（地址协议
  //    词汇，13 处）。清它要把这两个解析入口改成宿主侧自由函数（与 `actor_can_*`
  //    同一形态），见 DE-023 D-188 的 C2b 步；
  //  - `capability/family.rs`：能力家族 id `"model"` 与插件目录同名（1 处，报的是
  //    家族词汇而非插件依赖）。
  // 只许降，整张表的目标仍是空表。
  'crates/symbio-kernel/src/capability/family.rs': 1,
  'crates/symbio-kernel/src/chat/principal.rs': 13,
}

const nounCounts = new Map()
const purityRels = new Set(purityTexts.map((f) => f.rel))
let domainNounTotal = 0
if (hasPlugins && pluginIds.length > 0) {
  for (const f of purityTexts) {
    let n = 0
    for (const id of pluginIds) {
      const re = new RegExp(`"${id}"|"${id}[:/]`, 'g')
      n += (f.text.match(re) || []).length
    }
    if (n > 0) nounCounts.set(f.rel, n)
  }
  for (const f of coreTexts) {
    if (purityRels.has(f.rel)) continue
    for (const id of pluginIds) {
      const re = new RegExp(`"${id}"|"${id}[:/]`, 'g')
      domainNounTotal += (f.text.match(re) || []).length
    }
  }
  const ratchetDown = []
  for (const [rel, n] of nounCounts) {
    const cap = NOUN_BASELINE[rel] ?? 0
    if (n > cap) {
      report(
        'CS-004',
        `插件 id 字面量回流：${rel} 有 ${n} 处 > 冻结 ${cap} —— ` +
          '词汇（插件 id / 路由 / 装配位）由 owner 声明，内核只 seal（DE-023 D-177 / D-179）',
      )
    } else if (n < cap) {
      ratchetDown.push(`${rel} ${n}<${cap}`)
    }
  }
  for (const rel of Object.keys(NOUN_BASELINE)) {
    if (!nounCounts.has(rel)) ratchetDown.push(`${rel} 0<${NOUN_BASELINE[rel]}`)
  }
  if (ratchetDown.length && errors.length === 0) {
    console.log(
      yellow('提示') +
        `：插件 id 字面量已收到冻结值下方（${ratchetDown.join(' · ')}），` +
        '请同步本脚本的 NOUN_BASELINE——留着旧数字等于把余量白留给下一次',
    )
  }
}


// ── CS-005 业务事件词汇只许降 ──
//
// 两张表是**迁移存量**：新事件经 `EventDecl` 进装配表，不在这里加行。扫描按文件存在
// 与否分三种情况：都在 ⇒ 判；都不在 ⇒ 夹具仓库，不适用；只在一部分 ⇒ 红（判据已失效）。
const VOCAB_SOURCES = {
  kernelEvents: 'crates/symbio-kernel/src/event/mod.rs',
}
// `CELLS`（`event/grid.rs`）已随 D-203 归零批退役。它**不该再出现**：文件在 ⇒ 数它的
// 业务行（基线 0，一行就红）；文件不在 ⇒ `cellsRows = 0`，正是目标值。这样「把表加回来」
// 与「往表里加一行」都会红，判据不必依赖一张已经删掉的文件的形状。
const RETIRED_CELLS = 'crates/symbio-kernel/src/event/grid.rs'
const PROTOCOL_EVENT_VALUES = new Set(['request', 'sent', 'response', 'failed', 'retry'])
const PROTOCOL_EVENT_NAMES = new Set(['REQUEST', 'SENT', 'RESPONSE', 'FAILED', 'RETRY'])

/** 内核 `pub const EVENT_*: &str = "<点分名>";` 里**业务事件**的条数。 */
function countKernelEvents(abs) {
  const text = blankComments(fs.readFileSync(abs, 'utf8')).join('\n')
  return [
    ...text.matchAll(/pub\s+const\s+EVENT_[A-Z0-9_]+\s*:\s*&(?:'static\s+)?str\s*=\s*"([^"]*)"/g),
  ].filter((m) => m[1].includes('.') && !PROTOCOL_EVENT_VALUES.has(m[1])).length
}

/** `CELLS` 块里**业务事件**行的条数（协议五格由内核预置，不算）。 */
function countCellsRows(abs) {
  const text = blankComments(fs.readFileSync(abs, 'utf8')).join('\n')
  const start = text.indexOf('const CELLS')
  if (start < 0) return null
  const end = text.indexOf('];', start)
  if (end < 0) return null
  const block = text.slice(start, end)
  return [...block.matchAll(/\(\s*EVENT_([A-Z0-9_]+)\s*,/g)].filter(
    (m) => !PROTOCOL_EVENT_NAMES.has(m[1]),
  ).length
}

const vocabPaths = Object.fromEntries(
  Object.entries(VOCAB_SOURCES).map(([key, rel]) => [key, path.join(ROOT, rel)]),
)
const cellsAbs = path.join(ROOT, RETIRED_CELLS)
const vocabCounts = {}
let vocabApplicable = false
if (!fs.existsSync(vocabPaths.kernelEvents)) {
  // 夹具仓库没有内核事件表 ⇒ CS-005 不适用（CS-001…004 照判）。
} else {
  vocabApplicable = true
  vocabCounts.kernelEvents = countKernelEvents(vocabPaths.kernelEvents)
  if (fs.existsSync(cellsAbs)) {
    const cells = countCellsRows(cellsAbs)
    if (cells === null) {
      report('CS-005', '`grid.rs` 又出现了，但读不出 `CELLS` 块——形状变了，宁红不静默')
    } else {
      vocabCounts.cellsRows = cells
    }
  } else {
    vocabCounts.cellsRows = 0
  }
  for (const key of Object.keys(VOCAB_BASELINE)) {
    if (vocabCounts[key] > VOCAB_BASELINE[key]) {
      report(
        'CS-005',
        `${key} 实测 ${vocabCounts[key]} > 冻结 ${VOCAB_BASELINE[key]} —— ` +
          '新事件走 `EventDecl` 进装配表，不再往内核这两张手写表加行（DE-024 D-197 / D-200）',
      )
    }
  }
}

// ── CS-006 内核业务事件名的消费方分布（DE-024 D-202 / D-203）──
//
// CS-005 数的是条数，条数本身说不出「这一条此刻在谁手里」。这一格补上那个缺口：
// 逐条名字核**谁在读**，两种分布——
//  ① ≥2 个 crate 的生产消费方：名字被一个以上插件写或读。它**不能**被单一 owner 独占
//     （独占就逼别的插件跨墙引用兄弟插件，E-009），所以出口是一条共享词汇表，
//     不是继续住内核（D-203 裁决三 路线③）；
//  ② core 生产代码读它：这条**不是**归属证据，是**待迁存量**——名字该随那条读者
//     一起迁出（读写同源，D-197 说的是「读者与名字同处」，不是「读者住内核」）。
// 第三种（恰好一个消费方且 core 不读）就是当批可迁的形状：它该随那个 owner 的
// `EventDecl` 走，本条把它判红。①② 两类都不判红，但只有①是有出口的现状，②是欠账——
// 逐条的欠账账目见 DE-024 D-203 的迁移路线，CS-005 的归零就是它还账的方式。
const CONSUMER_CRATE_PREFIXES = ['symbio-plugin-', 'symbio-provider-']
// `symbio-plugin-api` 是宿主↔插件的共享协议 crate，不是能力持有者：它引用一个名字
// 不构成「某个插件读它」的证据。
const NOT_A_CONSUMER_CRATE = new Set(['symbio-plugin-api'])

/** 插件 / provider 各 crate 的生产侧全文（消费方证据的输入；夹具仓库可以为空）。 */
const consumerTexts = (() => {
  const out = []
  if (!fs.existsSync(pluginsDir)) return out
  for (const ent of fs.readdirSync(pluginsDir, { withFileTypes: true })) {
    if (!ent.isDirectory()) continue
    if (!CONSUMER_CRATE_PREFIXES.some((p) => ent.name.startsWith(p))) continue
    if (NOT_A_CONSUMER_CRATE.has(ent.name)) continue
    for (const file of walk(path.join(pluginsDir, ent.name, 'src'))) {
      if (!file.endsWith('.rs') || isTestFile(file)) continue
      out.push({
        crate: ent.name,
        text: blankTestModules(blankComments(fs.readFileSync(file, 'utf8')).join('\n')),
      })
    }
  }
  return out
})()

/**
 * core 的**读者**全文：`pub use` statements 先剥掉——根出口转手一个名字不等于有人
 * 读它（那正是 D-201 要区分开的两件事：出口是账本，读者才是归属证据）。内核那两张
 * 被数的表自己也在生产侧文本里，它们是**名字的登记处**不是读者，一并排除。
 */
const DECLARATION_RELS = new Set(Object.values(VOCAB_SOURCES))
const coreReaderTexts = coreTexts
  .filter((f) => !DECLARATION_RELS.has(f.rel))
  .map((f) => ({
    rel: f.rel,
    text: f.text.replace(/\bpub\s+use\s+[^;]*;/gs, ''),
  }))

/** 内核 `event/mod.rs` 里的业务事件（符号 + 线上名字），协议五格不算。 */
function listKernelBusinessEvents(abs) {
  const text = blankComments(fs.readFileSync(abs, 'utf8')).join('\n')
  return [
    ...text.matchAll(
      /pub\s+const\s+(EVENT_[A-Z0-9_]+)\s*:\s*&(?:'static\s+)?str\s*=\s*"([^"]*)"/g,
    ),
  ]
    .filter((m) => m[2].includes('.') && !PROTOCOL_EVENT_VALUES.has(m[2]))
    .map((m) => ({ sym: m[1], name: m[2] }))
}

let vocabEvidence = null
if (vocabApplicable && consumerTexts.length > 0) {
  const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  vocabEvidence = listKernelBusinessEvents(vocabPaths.kernelEvents).map((ev) => {
    const bySym = new RegExp(`\\b${ev.sym}\\b`)
    const byName = new RegExp(`"${esc(ev.name)}"`)
    const hit = (t) => bySym.test(t) || byName.test(t)
    return {
      ...ev,
      coreReaders: coreReaderTexts.filter((f) => hit(f.text)).map((f) => f.rel),
      consumers: [...new Set(consumerTexts.filter((c) => hit(c.text)).map((c) => c.crate))].sort(),
    }
  })
  for (const ev of vocabEvidence) {
    if (ev.consumers.length >= 2 || ev.coreReaders.length > 0) continue
    report(
      'CS-006',
      ev.consumers.length === 1
        ? `「${ev.name}」的唯一生产消费方是 ${ev.consumers[0]}，core 已不再读它 —— ` +
          '这条名字该随那个 owner 的 `EventDecl` 走（内核常量与 `CELLS` 行同批退役），' +
          '躲在 CS-005 的数字里就等于把待迁存量说成内核词汇（DE-024 D-201 / D-202 / D-203）'
        : `「${ev.name}」在 core 与插件的生产代码里都没有读者 —— ` +
          '内核词汇表里躺着一行没人读的名字：要么删，要么说清它守的是哪条协议',
    )
  }
}

const ruleLine = (ok, rule, desc) =>
  console.log(`${ok ? green('✓') : red('✗')} ${rule}  ${dim(desc)}`)

// ── CS-008 core 生产代码零事件名字面量（DE-024 D-203 的翻转）──
//
// CS-005 判的是「内核**表**里还有几条业务名」，CS-006 判的是「表上那几条该随谁走」。
// 两者都以那张表为输入，于是表归零之后它们都**没有输入**了——一个没有输入却常绿的
// 判据，挡不住下面这条边：
//
// ```ignore
// // 名字没有回到内核表，但回到了 core 的 match 臂里
// match e.kind.as_str() { "chat.assistant.final" => …, _ => {} }
// ```
//
// 这正是 D-203 要拆掉的那条边（「加一条对话面事件」= 改架构模块）。所以归零批必须
// 配一条**反向**判据：core / kernel 的**生产代码**里不出现任何事件名字面量。
//
// ## 判据形状
//
// 只认「首段是 `Entity` 的某个变体（小写）」的点分字符串。理由是这一族可判别：
// `Entity` 是 F5 冻结的**闭集**，所以 `chat.assistant.final` / `user.message` 的形态
// 是语法事实，不是猜——`"utf-8"` / `"application/json"` 这类同形的点分串首段不在闭集里，
// 自动落在闸外（与 D-009 的「同族」闸同一个思路：用闭集收窄，不用豁免兜底）。
//
// ## 什么不算生产代码
//
// `*.test.rs` 与 `#[cfg(test)] mod x { … }` 由 `coreTexts` 剥掉；`#[cfg(test)] mod x;`
// 这种**声明式**测试模块（`core/src/test_fixtures.rs`）不在它的处理范围里，本判据
// 自己解析模块声明把那个文件剔出去——夹具交自己的字面量是合法的，而且正是 D-205 /
// D-206 要的证据形态（「词汇是数据」：判据不为任何插件改名）。
const CFG_TEST_DECL_RE = /#\[cfg\(test\)\]\s*mod\s+([A-Za-z0-9_]+)\s*;/g

/** 被 `#[cfg(test)] mod x;` 声明的模块文件（按 Rust 的两种落点解析）。 */
function cfgTestModuleFiles(dirs) {
  const out = new Set()
  for (const dir of dirs) {
    for (const file of walk(dir)) {
      if (!file.endsWith('.rs') || isTestFile(file)) continue
      const text = fs.readFileSync(file, 'utf8')
      const decls = [...text.matchAll(CFG_TEST_DECL_RE)].map((m) => m[1])
      if (decls.length === 0) continue
      const base = /(?:^|[\\/])(?:mod|lib|main)\.rs$/.test(file)
        ? path.dirname(file)
        : file.replace(/\.rs$/, '')
      for (const name of decls) {
        for (const cand of [path.join(base, `${name}.rs`), path.join(base, name, 'mod.rs')]) {
          if (fs.existsSync(cand)) out.add(path.relative(ROOT, cand).split(path.sep).join('/'))
        }
      }
    }
  }
  return out
}

/** `Entity` 的变体（小写）——事件名的首段闭集；内核文件不在 ⇒ `null`（不适用）。 */
function entityNameSet(abs) {
  if (!abs || !fs.existsSync(abs)) return null
  const text = blankComments(fs.readFileSync(abs, 'utf8')).join('\n')
  const m = text.match(/pub\s+enum\s+Entity\s*\{/)
  if (!m) return null
  const open = text.indexOf('{', m.index)
  const body = text.slice(open + 1, matchBrace(text, open))
  const names = [...body.matchAll(/^\s*([A-Z][A-Za-z0-9_]*)\s*,/gm)].map((x) =>
    x[1].toLowerCase(),
  )
  return names.length ? new Set(names) : null
}

const entityNames = entityNameSet(vocabPaths.kernelEvents)
const testOnlyFiles = cfgTestModuleFiles(scanDirs)
const eventNameHits = []
if (entityNames) {
  const litRe = new RegExp(`"((?:${[...entityNames].join('|')})\\.[a-z][a-z0-9_]*(?:\\.[a-z][a-z0-9_]*)*)"`, 'g')
  for (const f of coreTexts) {
    if (testOnlyFiles.has(f.rel)) continue
    for (const m of f.text.matchAll(litRe)) {
      const line = f.text.slice(0, m.index).split('\n').length
      eventNameHits.push({ rel: f.rel, line, name: m[1] })
    }
  }
  for (const hit of eventNameHits) {
    report(
      'CS-008',
      `\`${hit.rel}:${hit.line}\` 出现了事件名字面量 \`"${hit.name}"\` —— ` +
        'core 的折叠按**构造期词汇面**认名字（`ChatVocab` / `MessageVocab` / `CommitmentVocab` / `TaskVocab`），' +
        '不抄拼写：写死一个字面量等于把「加一条事件要改架构」那条边接回来（DE-024 D-203 / D-205）',
    )
  }
}

const cells = Object.keys(SHAPE_BASELINE)
const shapeLine = cells
  .map((k) => `${k} ${shape.values[k]}/${SHAPE_BASELINE[k]}`)
  .join(' · ')
const dataLine = Object.keys(DATA_BASELINE)
  .map((k) => `${k} ${dataValues[k]}/${DATA_BASELINE[k]}`)
  .join(' · ')

if (errors.length === 0) {
  console.log('\n== core-shape-audit ==')
  ruleLine(true, 'CS-001', `形状棘轮：${shapeLine}`)
  ruleLine(true, 'CS-009', `数据面棘轮：${dataLine}（只许降；新领域走 Entity::Domain 扩展位）`)
  ruleLine(
    true,
    'CS-010',
    turnStateFields === null
      ? '控制面草稿字段棘轮：夹具仓库没有 TurnState，CS-010 不适用'
      : `控制面草稿字段棘轮：turnStateFields ${turnStateFields}/${TURN_STATE_BASELINE}（只许降；` +
          '新增一格要先立决策，逐字段处置见 DE-022 D-226）',
  )
  ruleLine(
    true,
    'CS-002',
    waivers.length
      ? `放宽 ${waivers.length} 格，引用的 ADR 全部能打开：${waivers.map((w) => `${w.key}←${w.reason.match(ADR_REF_RE)?.[0]}`).join('、')}`
      : '没有放宽痕迹（基线就是当前形状）',
  )
  const nounTotal = [...nounCounts.values()].reduce((s, n) => s + n, 0)
  ruleLine(true, 'CS-003', '内核纯度：core 生产代码零 `crate::plugins` 引用')
  ruleLine(
    true,
    'CS-004',
    pluginIds.length
      ? `内核纯度：内核文件集插件 id 字面量 ${nounTotal} 处（冻结 ${Object.values(NOUN_BASELINE).reduce((s, n) => s + n, 0)} 处 / ${Object.keys(NOUN_BASELINE).length} 个文件，只许降）；` +
        `域文件 ${domainNounTotal} 处（C 迁出账，不设门禁）`
      : '内核纯度：夹具仓库没有插件目录，CS-004 不适用',
  )
  if (vocabApplicable) {
    const vocabLine = Object.keys(VOCAB_BASELINE)
      .map((k) => `${k} ${vocabCounts[k]}/${VOCAB_BASELINE[k]}`)
      .join(' · ')
    ruleLine(true, 'CS-005', `事件词汇棘轮：${vocabLine}（只许降，目标是零；逐条分布见 CS-006）`)
    const underVocab = Object.keys(VOCAB_BASELINE).filter(
      (k) => vocabCounts[k] < VOCAB_BASELINE[k],
    )
    if (underVocab.length) {
      console.log(
        yellow('提示') +
          `：词汇已收到冻结值下方（${underVocab.map((k) => `${k} ${vocabCounts[k]}<${VOCAB_BASELINE[k]}`).join(' · ')}），` +
          '请同步本脚本的 VOCAB_BASELINE',
      )
    }
  } else {
    ruleLine(true, 'CS-005', '事件词汇棘轮：夹具仓库没有内核事件表，CS-005 不适用')
  }
  if (vocabEvidence) {
    const byPlugin = vocabEvidence.filter((e) => e.consumers.length >= 2)
    const byCore = vocabEvidence.filter((e) => e.consumers.length < 2 && e.coreReaders.length > 0)
    ruleLine(
      true,
      'CS-006',
      `内核业务事件名的消费方分布：${vocabEvidence.length} 条都不在「单一消费方且 core 不读」这一形态` +
        `（≥2 crate 消费 ${byPlugin.length}：要一条共享词汇表才迁得走 · core 读者 ${byCore.length}：待迁存量，随各自读者一起迁出）——` +
        '新名字进内核词汇表没有通路，只能由 owner 声明',
    )
    if (VERBOSE) {
      for (const ev of vocabEvidence) {
        const readers = ev.coreReaders.map((rel) => rel.replace(/^crates\/symbio-(core|kernel)\/src\//, ''))
        console.log(
          `  ${ev.name.padEnd(24)} | ${(readers.length ? readers.join(' / ') : 'core —').padEnd(46)}| ${ev.consumers.map((c) => c.replace('symbio-', '')).join(' / ') || '—'}`,
        )
      }
    }
  } else if (vocabApplicable) {
    ruleLine(true, 'CS-006', '消费方证据：夹具仓库没有插件 crate，CS-006 不适用')
  } else {
    ruleLine(true, 'CS-006', '消费方证据：没有内核事件表，CS-006 不适用')
  }
  if (entityNames) {
    ruleLine(
      true,
      'CS-008',
      `core 生产代码零事件名字面量（首段闭集 = \`Entity\` 的 ${entityNames.size} 个变体；` +
        `剥掉 ${testOnlyFiles.size} 个 \`#[cfg(test)] mod\` 文件）——` +
        '表归零之后，名字也没有第二条路能回到 core 的 `match` 臂里',
    )
  } else {
    ruleLine(true, 'CS-008', 'core 生产代码零事件名字面量：夹具仓库没有内核事件表，CS-008 不适用')
  }
  const under = cells.filter((k) => shape.values[k] < SHAPE_BASELINE[k])
  if (under.length) {
    console.log(
      yellow('提示') +
        `：已收到基线下方（${under.map((k) => `${k} ${shape.values[k]}<${SHAPE_BASELINE[k]}`).join(' · ')}），` +
        `请同步本脚本的 SHAPE_BASELINE——留着旧数字等于把余量白留给下一次`,
    )
  }
  const dataUnder = Object.keys(DATA_BASELINE).filter((k) => dataValues[k] < DATA_BASELINE[k])
  if (dataUnder.length) {
    console.log(
      yellow('提示') +
        `：数据面已收到基线下方（${dataUnder.map((k) => `${k} ${dataValues[k]}<${DATA_BASELINE[k]}`).join(' · ')}），` +
        `请同步本脚本的 DATA_BASELINE——留着旧数字等于把余量白留给下一次`,
    )
  }
  if (turnStateFields !== null && turnStateFields < TURN_STATE_BASELINE) {
    console.log(
      yellow('提示') +
        `：TurnState 已收到基线下方（turnStateFields ${turnStateFields}<${TURN_STATE_BASELINE}），` +
        '请同步本脚本的 TURN_STATE_BASELINE——留着旧数字等于把余量白留给下一次',
    )
  }
  if (VERBOSE) {
    console.log('  —— pub trait 形状（成员数 / 其中带默认体）')
    for (const r of shape.rows)
      console.log(`  [T] ${r.name.padEnd(26)} ${String(r.fns).padStart(2)}/${String(r.defaults).padEnd(2)} ${r.file}`)
    console.log('  —— 公共数据面（字段数 / 变体数）')
    for (const r of [...dataRows].sort((a, b) => b.n - a.n || a.name.localeCompare(b.name)))
      console.log(`  [${r.kind === 'struct' ? 'S' : 'E'}] ${r.name.padEnd(26)} ${String(r.n).padStart(2)} ${r.file}`)
  }
  process.exit(0)
}

console.error(`\n== core-shape-audit：${errors.length} 处违规 ==`)
for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
console.error(
  `\n${yellow('提示')}：形状量的口径与不判什么见本脚本文件头；放宽口径见 ` +
    `${green('docs/003-design/001-核心架构.md')}；逐 trait 清单跑 node scripts/core-shape-audit.mjs --verbose`,
)
if (VERBOSE) {
  for (const r of shape.rows)
    console.error(`  [T] ${r.name.padEnd(26)} ${String(r.fns).padStart(2)}/${String(r.defaults).padEnd(2)} ${r.file}`)
}
process.exit(1)
