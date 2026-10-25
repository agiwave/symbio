#!/usr/bin/env node
/**
 * core-export-audit — `symbio_core` **架构出口的有效性**（判定型）
 *
 * ## 它判什么
 *
 * 「symbio_core 的代码全放子目录里，但子目录**不许对外 `pub` 输出**，必须在**根目录
 * 显式输出**；根输出的每一件东西必须被**两个以上模块**使用——没人用的不许存在，
 * 只有一个模块用的必须下沉到那个模块里（只有一个消费方的不算架构元素）。」
 *
 * 三条规则，一一对应：
 *
 * | 规则 | 判什么 | 为什么必须判 |
 * |---|---|---|
 * | C-001 | 根 `mod.rs` 不得有 `pub mod <域>;` | 域目录一旦 `pub mod`，整个子树就是公共面：`core-surface` 枚举的「公开面 = 根重导出」当场失真，外部还能直接深引绕过根 |
 * | C-002 | core 外的**代码**不得出现 `symbio_core::<域>`（含裸 `use crate::symbio_core::<域>;`） | 深引 = 消费方绕过根出口：符号从根导出移除后深引点照样编译通过，公开面变成两套。裸模块导入此前**没有**任何审计在拦（旧 E-010 的正则要求尾 `::`） |
 * | C-003 | 根导出符号的**生产**消费方数必须 ≥2 | D-023 的判据（依赖方数量）此前只有**报告型** `core-surface-audit` 在数、判定权在人手里，于是长期停在 0 消费方 139 / 单消费方 64 上不动 |
 *
 * ## 「消费方」的口径（2026-10-09 修正，C-003 的判据输入）
 *
 * **测试文件不算架构消费方。** 本条要回答的是「谁依赖它」，而测试依赖的是**被测对象**，
 * 不是它要长期持有的接口。旧口径把测试算进来 ⇒「一个模块在生产用、另一个模块的测试
 * 顺手写了一次名字」读成「两个模块依赖」⇒ 真·单消费方符号逃过下沉。实测：
 * `TurnRunner` / `TurnInput`（生产只有 `plugins/session`，第二个单位来自
 * `plugins/model/bound_provider.test.rs` 的彩排）长期显为 ≥2，而同族的 `TurnOutcome` /
 * `TurnResume`（没被那个测试提到）一直显为单消费方——**同一个功能、四个名字、两种判决**。
 * 这族符号已于 2026-10-09 整体下沉 `plugins/session`（见 `symbio_core/mod.rs` 的
 * `actors` 导出注与 `plugins/session/turn_runner.rs` 文件头）——本条口径修正的动机即由此暴露。
 *
 * 但**「0 消费方」那一半仍按含测试计**：只被别模块测试用到的符号，收窄它当场把那些测试
 * 编译坏（C-002 又不许它们深引）⇒「测试在用」**足以**让它留在根出口。故本条实际分三档：
 * `0 消费方`（含测试，必须收窄）/ `单消费方`（不含测试，下沉或豁免）/ `仅测试在用`
 * （合法根出口，不动）。两个口径的实现同一处：`core-surface.mjs` 的 `collectConsumers`
 * （`opts.includeTests`）。
 *
 * ## 与既有审计的分工（别在两处写两套判据）
 *
 * - **`core-surface-audit.mjs`**（报告型）：同一份公开面（`collectCoreSurface`）与
 *   同一份消费方计数（`collectConsumers`）。报告负责**列出候选**并提示 README §4 四问；
 *   本脚本负责**判定**：豁免账本（必须写非空理由）+ 棘轮基线（只许降）。
 * - **原 E-010**（`plugin-entry-audit.mjs` 的「不得深引 `symbio_core::<域>::`」）：**已退役，
 *   由 C-002 取代**——深引是**内核出口**的事、不是插件门面的事，两处各判一遍必然分叉出两套
 *   域清单与豁免通道。C-002 是它的全量版：域目录动态取自 core（含 `schemas`，不再豁免），
 *   并额外拦裸 `use crate::symbio_core::<域>;`（原正则要求尾 `::`，漏了这种形态）。
 * - **`dead-code-audit.mjs`**：判「全仓零引用的 `pub` 声明」（core 自身也算引用）。
 *   C-003 判的是「名字没跨出 core 的根导出」——**core 内部在用、对外零消费**的符号归这里：
 *   它不该占着根出口（收窄 `pub(crate)` 或下沉）。互补不重叠。
 * - **`core-shape-audit.mjs`**（CS-001 / CS-002）：判 core 的**公共形状**（`pub trait` 数、
 *   `fn` 成员总数、带默认体的成员数）。与本脚本判的是**两个不同的量**，谁也不能代谁：
 *   本脚本数的是**符号表**（哪些名字能出现在根出口、各有几个消费方），一个既有导出符号
 *   在接口上多挂 9 个方法，符号数一点不变 ⇒ C-003 看不见。两边各自的棘轮、各自的豁免，
 *   不共用口径（这条缺口是 2026-10-12 实测出来的，见那份脚本文件头）。
 *
 * ## 存量怎么办：棘轮基线
 *
 * 口径各归其位后全仓 0 消费方 0 个、单消费方 58 个（自然 64 − 账本 6）、仅测试在用 7 个。
 * 一次性下沉 200+ 符号等于把 core 翻一遍，
 * 不可独立回退；故按本仓既有棘轮口径（`_shared.mjs` 的 `BASELINE.rustTests`、
 * `test-layout-audit` 基线）：**计数只许降，超了就红**。降到基线以下时脚本会提示同步
 * `BASELINE`。
 *
 * ## 豁免（对应 `symbio_core/README.md` §4 四问）
 *
 * 单消费方不等于该下沉——宿主接缝（`ExecTranscriptWriter`）、跨 crate 契约、trait 形参
 * 类型都只有一个消费方而必须留 core。把符号加进 `WAIVERS` 并写**非空理由**即豁免；
 * 空理由、以及**失效豁免**（符号已不在公开面 / 已 ≥2 消费方）都判红——豁免要随代码一起退役。
 *
 * C-001 / C-002 的行内豁免：同一行（或其上 3 行内）的注释写 `core-export-allow: <理由>`。
 *
 * ## 用法
 *   node scripts/core-export-audit.mjs            # 判定
 *   node scripts/core-export-audit.mjs --verbose  # 附存量清单（0 / 1 消费方逐条）
 *   node scripts/core-export-audit.mjs --root=<dir>  # 指向别处的仓库（测试用）
 * 退出码：0 = 通过，1 = 违规（源头可改）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns C core 出口
// @codes C-001 C-002 C-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'
import {
  collectCoreSurface,
  collectConsumers,
  collectCoreDomainConsumers,
  isSelfReference,
  isSinkCandidate,
} from './core-surface.mjs'
import { blankComments } from './rust-scan.mjs'

const argv = process.argv.slice(2)
const VERBOSE = argv.includes('--verbose')
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const CORE_REL = 'symbio/src/symbio_core'
/** 深引扫描范围：与 E-010 的 CODE_ROOTS 同源（壳侧是独立 crate，但同属消费方） */
const CODE_ROOTS = ['symbio/src', 'cli/src', 'tauri/src', 'tauri/src-tauri/src']

/**
 * C-003 豁免账本：符号 → **非空**理由（README §4 四问）。
 * 空字符串不算豁免；符号离开公开面或消费方变 ≥2 ⇒ 本条失效（判红，要求同步删除）。
 *
 * `CORE_EXPORT_WAIVERS`（JSON 对象）/ `CORE_EXPORT_BASELINE`（`zero:one`）是回归测试的
 * 注入通道——真仓门禁不设这两个变量（设了就等于把棘轮调到零，见 `.test.mjs`）。
 */
const WAIVERS = process.env.CORE_EXPORT_WAIVERS
  ? JSON.parse(process.env.CORE_EXPORT_WAIVERS)
  : {
      // 2026-10-11（W1）：账本从 18 条清到 **6 条**。清掉的那 12 条，理由逐字写着
      // 「core 内另有消费方（transcript / prompt_fidelity / authz …），审计只数跨出
      // core 的那一个」——那是**口径的盲区**，不是符号的属性。C-003 的「该不该下沉」
      // 现在按 core-surface.mjs 的 isSinkCandidate 判（跨 core 的生产消费方恰 1 个
      // **且 core 内没有另一个模块在生产用它**）；盲区填上后那 12 条**结构上**已不再
      // 显为单消费方 ⇒ 豁免自行失效并被删除（纪律 2：豁免是在补口径的洞，补洞该补
      // 口径，不该把洞登记成永久账目）。
      //
      // 剩下 5 条是**真·单消费方接缝**：core 内确实没有第二个模块（生产）在用，但既
      // 下沉不了（core 不得反向依赖插件 / 类型行为无处安放），又必须占根出口（C-002
      // 禁插件深引 core 域目录）。
      ActorSpec:
        'F3 冻结锚点（plan/01 §4 主体规格五字段：principal/pattern/capabilities/budget_ms/scope）：core 外唯一生产消费方是 plugins/session（turn_exec 构造 + turn_runner 读 principal），但**不可下沉**——F3 冻的是它的字段集，搬出 core 即架构变更（03 §1）；且 C-002 禁止插件深引 core 域目录 ⇒ 插件只能经根出口拿它。属 README §4 四问的「接缝」而非「单消费方该下沉」',
      turnstate:
        '投影函数：与 transcript / slo_report / fallback_rate / checkpoint 同族同形——**投影只该在 core 有一份实现**，插件经根函数名调用（C-002 禁深引域目录）。core 外唯一生产消费方 = plugins/session/worker_progress（S2 的 in_flight 判据）。core 内没有第二个生产模块用它（同域内只有测试彩排），故显为单消费方',
      SloLatencyView:
        'P2 跨会话聚合的视图类型：merge 是类型行为（提成根函数即无主函数），C-002 禁插件深引 ⇒ 类型必须占根。core 内没有第二个生产消费模块（跨会话聚合实现在插件侧），故显为单消费方',
      FallbackRateView:
        '同上：与 SloLatencyView 成对（时延 + 兜底一起合并），core 外唯一生产消费方同为 plugins/session 的实例级聚合列，core 内没有第二个生产消费模块',
      mutations_latest_snapshot:
        '快照定位规则（S2.7-b2）：core 内一份实现（transcript 换基 / prompt_fidelity 跳过被取代段），插件经根名调用（C-002 禁深引 projection 域）。core 外唯一生产消费方 = plugins/session（turn_runner 取窗口 / turn_exec 判幂等并取轮次水位 / rebuild 换基）。core 内没有第二个生产消费模块（store 侧只有测试），故显为单消费方。不可下沉——五处读侧各写一份必然漂，而漂的后果是静默的',
      token_ledger:
        '投影函数：与 cost_ledger / slo_report / fallback_rate 同族同形——**投影只该在 core 有一份实现**，插件经根函数名调用（C-002 禁深引 projection 域目录）。core 外唯一生产消费方 = plugins/session/stats.rs 的用量列（`response{port=model}` 载荷的 `usage`）。core 内没有第二个生产模块用它（同域内只有测试），故显为单消费方。不可下沉：下沉等于让「用量」的口径住进会话插件，而它读的是事件载荷的通用形状（任何读侧都该按同一本账报数）',
      // 2026-10-21（段一）：事件 kind 名是**事件网格的命名空间**，README §3
      // event 域写明「常量 = 事件名字表（名字是数据，单点定义）」。`turn.stage` 与
      // `turn.supplemented` 同表；写读两侧今天都在 plugins/loop，但 kind 名不是 loop
      // 私有行为——它是 WAL / 回放 / 任何读侧的线上数据键，C-002 禁插件深引事件域
      // ⇒ 只能经根出口。把一条名字搬进唯一消费方会让同一命名空间有两个 owner，
      // 与「名字单点定义」直接冲突（与 `SOURCE_PRINCIPAL` 同一条判据：协议键不可下沉）。
      EVENT_TURN_STAGE:
        '事件 kind 名表住 core 的 event 域（S01 turn/* 全部，名字是数据、单点定义）；当前写读都在 plugins/loop，但它是 WAL / 回放 / 跨插件读侧的线上数据键，C-002 禁插件深引事件域 ⇒ 必须占根出口。下沉到唯一消费方会让同一命名空间分两个 owner。属 README §4 四问的「接缝」，不是「单消费方该下沉」',
    }

/**
 * 棘轮基线：0 消费方 / 单消费方的存量上限（自引用不计）。
 * 只许降——超了说明新符号进了根出口却没跨出 core，或某个消费方被删掉了。
 *
 * `0` 是规则 2 首批整改的结果：140 个「名字没跨出 core」的符号已从根出口收窄
 * （定义留在原域 `pub`，core 内走域内路径），只被测试/无人使用的存量另由
 * `dead-code-audit` R-002 逐项承认。
 *
 * `58` 是当前存量（自然单消费方 64，其中 6 条登记在 `WAIVERS`），两条来源都不是新增违规：
 * - **口径修正**（2026-10-09）：**测试文件不再算架构消费方**（口径全文见
 *   `core-surface.mjs` 的 `collectConsumers`）——此前「一个模块在生产用、另一个模块的
 *   测试顺手写了一次名字」被读成「两个模块依赖」，遮住了一批真·单消费方符号。
 *   `zero` 不受影响（仍按**含测试**计——只被测试用到的符号收窄会编译坏那些测试）。
 * - **W1 的第二口径**（2026-10-11）：「该不该下沉」不再只看跨 core 的消费方，而是按
 *   `isSinkCandidate` 判——**core 内另有生产模块在用**的符号本来就下沉不了（core 不得
 *   反向依赖插件），把它留在根出口不是「豁免」而是口径本该如此。此前的豁免账本大半
 *   条目正是在补这个盲区（见上方 `WAIVERS` 注），现在账本 18 → 6，一格计数随之下沉。
 *   ⚠️ 这**抵消了** 2026-10-09 那次口径修正引入的一格虚高（当时 one 61→77 是测量口径
 *   的一次真实变化；这次把 two 条口径各归其位后回落到 58，不再需要
 *   `core-export-allow one` 那条棘轮格下修豁免）。
 * - ** 合并批**（2026-10-14）降一格 58 → 57：`Pattern::Reasoner` 的派生下沉回
 *   actors 定义域（`Pattern::for_principal`，插件层不再出现主体类型名——NDC-001 同一
 *   纪律），`Reasoner` 退出单消费方清单；`SOURCE_PRINCIPAL` 作为线上协议键登记豁免
 *   （跨进程契约，不可下沉，见账本）。
 *
 * - **同域实现批**（2026-10-14）降一格 57 → 56：`collectCoreBindings` 补上「core 内
 *   `impl <Trait> for …`」这一类耦合（原先被口径 3 的「声明域自身不算」全部滤掉）。
 *   `LlmAdapter` / `DeltaSink` 的实现方就在它们自己的声明域 `adapters` 里
 *   （`ProviderLlmAdapter`、`SilentDeltas`…），于是被判成「单消费方接缝、该下沉」——
 *   而下沉的前提「B 只被 A 依赖」在这里**不成立**，那会是真·依赖反转。这两条不是被清偿的
 *   存量，是**判错**；降这一格是因为判据修好后它们本就不该出现在清单里。
 *
 * - **同域使用也算消费方批**（2026-10-14）降一格 56 → 16：删掉 `collectCoreDomainConsumers`
 *   的口径 3「声明域自身不算」。**消费就是依赖，自己依赖自己也是依赖**——`LonelyThing`
 *   声明在 `actors`、被 `actors/other.rs` 用着，那就是 `actors` 在消费它，凭什么沉到
 *   插件？滤掉 core 内部的依赖等于**假装 core 不用它**，判据于是给出一个在事实上不成立
 *   的动作建议（真实误判见下）。
 *   连带删掉 `collectCoreBindings`（core 内 `impl <Trait> for …` 那一层）：口径 3 一去，
 *   同域的任何非声明引用都会把该域算成消费方，实测关掉它 C-003 逐条不变 ⇒ **冗余**。
 *   留着它就是多一处会静默腐烂、且今天恒真的判断层。
 *   ⚠️ 删口径 3 时有个必须一起钉住的副作用：域的 `mod.rs` 里 `pub struct X;` 与
 *   `pub use …X…` 是**声明 / 导出清单**，不是使用。改判据为「排除声明行与 `pub use` 行」
 *   才不至于让每个符号都被自己的声明算成消费方（不滤的话实测会得到 0/0，判据恒绿）。
 *
 *   存量影响：`--verbose` 前后清单 diff = **释放 40 条、新增 0 条**，56 → 16。
 *   ⚠️ 这不是一次「清偿 40 条」，是**判据修好后它们本就不算违规**——剩下 16 条
 *   （`symbio_core/README.md` §4 的宿主接缝 / 跨 crate 契约一类）是真的还没处置的存量。
 *
 * 存量的处置判据仍是 README §4 四问（下沉 / 成文契约 / 登记豁免），处置一个就把这里
 * 降一格。**注意**：本格只数「既没跨 core 第二个模块用、又不是自引用、core 内也没有**
 * 使用**它」的单消费方；一个符号在 `--verbose` 的 `[ 1]` 里出现即代表它仍待处置。
 */

const BASELINE = process.env.CORE_EXPORT_BASELINE
  ? (() => {
      const [zero, one] = process.env.CORE_EXPORT_BASELINE.split(':').map((n) => Number(n))
      return { zero, one }
    })()
  : { zero: 0, one: 13 }



const errors = []
const report = (rule, msg, loc = '') => errors.push({ rule, msg, loc })

/** 行内豁免：本行或其上 3 行内的注释写 `core-export-allow: <非空理由>`（看**原始行**，注释在 blank 后就没了） */
function waived(rawLines, i) {
  for (let j = Math.max(0, i - 3); j <= i; j++) {
    const m = rawLines[j].match(/core-export-allow:\s*(.*)$/)
    if (m) return m[1].trim().length > 0
  }
  return false
}

// ==================== 读仓库 ====================

let surface
try {
  surface = collectCoreSurface(ROOT)
} catch (e) {
  console.error(red(`✗ ${e.message}`))
  process.exit(1)
}
const { coreDir, domains, symbols, modules } = surface

function* walk(dir) {
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

const relOf = (abs) => path.relative(ROOT, abs).split(path.sep).join('/')

// ==================== C-001 根出口唯一 ====================

const rootRaw = fs.readFileSync(path.join(coreDir, 'mod.rs'), 'utf8')
const rootLines = rootRaw.split(/\r?\n/)
rootLines.forEach((line, i) => {
  const m = line.match(/^\s*pub\s+mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*[;{]/)
  if (!m) return
  if (waived(rootLines, i)) return
  report(
    'C-001',
    `根 mod.rs 出现 \`pub mod ${m[1]};\` —— 子目录不得对外开模块门；改成 \`mod ${m[1]};\`，` +
      `把需要出现在公共面上的东西在根 \`pub use\` 显式列出`,
    `${CORE_REL}/mod.rs:${i + 1}`,
  )
})

// ==================== C-002 core 外无深引 ====================

const deepRefRe = /\bsymbio_core\s*::\s*([a-z_][a-z0-9_]*)\b/
for (const r of CODE_ROOTS) {
  for (const file of walk(path.join(ROOT, r))) {
    if (!file.endsWith('.rs')) continue
    const abs = path.resolve(file)
    if (abs.startsWith(path.resolve(coreDir) + path.sep)) continue // core 内部深引是模块内部的事
    const raw = fs.readFileSync(file, 'utf8')
    const rawLines = raw.split(/\r?\n/)
    const codeLines = blankComments(raw) // 与原始行一一对应（豁免注释看原行）
    codeLines.forEach((line, i) => {
      const m = line.match(deepRefRe)
      if (!m || !domains.has(m[1])) return
      if (waived(rawLines, i)) return
      report(
        'C-002',
        `\`symbio_core::${m[1]}::…\` 深引 core 子目录（或裸 \`use …::symbio_core::${m[1]};\`）` +
          ` —— 根平铺是唯一公共面，写 \`symbio_core::<符号>\``,
        `${relOf(file)}:${i + 1}`,
      )
    })
  }
}

// ==================== C-003 双消费方 ====================

// 两份计数，问两个不同的问题（口径全文见 `core-surface.mjs` 的 `collectConsumers`）：
// - `units`（**不含测试**）：判「算不算架构元素」⇒ 单消费方要下沉；
// - `unitsAll`（含测试）：判「够不够格占根出口」⇒ 只有测试在用的，收窄会把测试编译坏。
const consumers = collectConsumers(ROOT, symbols, CORE_REL)
const consumersAll = collectConsumers(ROOT, symbols, CORE_REL, { includeTests: true })
// 「该不该下沉」的第二个输入：core 内**另一个生产模块**也在用它（口径见 core-surface.mjs
// 的 collectCoreDomainConsumers）。一个符号若有第二个 core 生产消费模块，下沉到那唯一的
// 外部消费方（插件）当场编译不过 ⇒ 它本来就该留根出口，不是「单消费方接缝」。
const coreConsumers = collectCoreDomainConsumers(ROOT, symbols, CORE_REL)
const byName = (a, b) => a.domain.localeCompare(b.domain) || a.name.localeCompare(b.name)
const rows = [...symbols.keys()]
  .map((name) => ({
    name,
    domain: symbols.get(name),
    units: [...(consumers.get(name) ?? [])].sort(),
    unitsAll: [...(consumersAll.get(name) ?? [])].sort(),
    isModule: modules.has(name),
  }))
  .sort(byName)

const waivedNow = new Set()
for (const [name, reason] of Object.entries(WAIVERS)) {
  if (!String(reason ?? '').trim()) {
    report('C-003', `豁免账本里 \`${name}\` 的理由为空 —— 空理由不算豁免`)
    continue
  }
  const row = rows.find((r) => r.name === name)
  if (!row) {
    report('C-003', `豁免账本里的 \`${name}\` 已不在公开面 —— 该条豁免已失效，请删除`)
    continue
  }
  if (row.units.length >= 2) {
    report('C-003', `豁免 \`${name}\` 已失效（现有 ${row.units.length} 个消费方）—— 请从账本删除`)
    continue
  }
  waivedNow.add(name)
}

const selfRef = rows.filter((r) => isSelfReference(r))
// 「没人用」按**含测试**的口径：只有别模块的测试在用的符号，收窄它当场把那些测试编译坏
// （C-002 又不许它们深引）⇒ 它够格占根出口，不进本条。
const zero = rows.filter((r) => r.unitsAll.length === 0 && !waivedNow.has(r.name))
// 「只有测试在用」单列一档：它是**合法的根出口**，但也不是架构元素——既不收窄、不下沉。
const testOnly = rows.filter((r) => r.units.length === 0 && r.unitsAll.length > 0)
// 「该不该下沉」按**不含测试**的口径：测试依赖的是被测对象，不是长期接口。
const one = rows.filter((r) => isSinkCandidate(r, coreConsumers) && !waivedNow.has(r.name))
const many = rows.filter((r) => r.units.length >= 2)

if (zero.length > BASELINE.zero) {
  report(
    'C-003',
    `0 消费方 ${zero.length} 个 > 基线 ${BASELINE.zero} —— 名字没跨出 core 的符号不许占根出口` +
      `（收窄为 \`pub(crate)\` 或删除）。逐条：--verbose`,
  )
}
if (one.length > BASELINE.one) {
  report(
    'C-003',
    `单消费方 ${one.length} 个 > 基线 ${BASELINE.one} —— 只有一个模块消费的不算架构元素，` +
      `下沉到该模块；确属 README §4 四问的接缝，写进 WAIVERS 并给理由。逐条：--verbose`,
  )
}

// ==================== 输出 ====================

const ruleLine = (ok, rule, desc) =>
  console.log(`${ok ? green('✓') : red('✗')} ${rule}  ${dim(desc)}`)

if (errors.length === 0) {
  console.log('\n== core-export-audit ==')
  ruleLine(true, 'C-001', '根出口唯一：根 mod.rs 无 `pub mod <域>;`')
  ruleLine(true, 'C-002', `core 外无深引（${domains.size} 个域目录动态取自 core）`)
  ruleLine(
    true,
    'C-003',
    `双消费方棘轮：0 消费方 ${zero.length}/${BASELINE.zero} · 单消费方 ${one.length}/${BASELINE.one}` +
      ` · 仅测试在用 ${testOnly.length} · 自引用 ${selfRef.length} · ≥2 ${many.length}`,
  )
  if (zero.length < BASELINE.zero || one.length < BASELINE.one) {
    console.log(
      yellow('提示') +
        `：已降到基线下方（${zero.length}/${one.length}），请同步本脚本的 ` +
        `BASELINE = { zero: ${zero.length}, one: ${one.length} }`,
    )
  }
  if (VERBOSE) {
    for (const r of zero) console.log(`  [ 0] ${(r.domain ?? '?').padEnd(12)} ${r.name}`)
    for (const r of testOnly)
      console.log(`  [ T] ${(r.domain ?? '?').padEnd(12)} ${r.name} -> 仅测试在用：${r.unitsAll.join(', ')}`)
    for (const r of one) console.log(`  [ 1] ${(r.domain ?? '?').padEnd(12)} ${r.name} -> ${r.units.join(', ')}`)
  }
  process.exit(0)
}

console.error(`\n== core-export-audit：${errors.length} 处违规 ==`)
for (const e of errors) {
  console.error(`  ${red(e.rule)}  ${e.msg}`)
  if (e.loc) console.error(dim(`        ${e.loc}`))
}
console.error(
  `\n${yellow('提示')}：C-003 存量走棘轮（只许降），处置判据见 ` +
    `${green('symbio_core/README.md §4 四问')}；逐条清单跑 ` +
    `node scripts/core-export-audit.mjs --verbose`,
)
if (VERBOSE) {
  for (const r of zero) console.error(`  [ 0] ${(r.domain ?? '?').padEnd(12)} ${r.name}`)
  for (const r of testOnly)
    console.error(`  [ T] ${(r.domain ?? '?').padEnd(12)} ${r.name} -> 仅测试在用：${r.unitsAll.join(', ')}`)
  for (const r of one) console.error(`  [ 1] ${(r.domain ?? '?').padEnd(12)} ${r.name} -> ${r.units.join(', ')}`)
}
process.exit(1)
