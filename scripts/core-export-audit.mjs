#!/usr/bin/env node
/**
 * core-export-audit — `symbio_core` **架构出口的有效性**（判定型）
 *
 * ## 它判什么
 *
 * 「symbio_core 的代码全放子目录里，但子目录**不许对外 `pub` 输出**，必须在**根目录
 * 显式输出**；**内核门面**（`kernel.rs`）输出的每一件东西必须被**两个以上模块**使用——
 * 只有一个模块用的必须下沉到那个模块里。内核之外的根出口是一本**过渡期域账本**：
 * 它只许减，不许再长。」
 *
 * 四条规则，一一对应：
 *
 * | 规则 | 判什么 | 为什么必须判 |
 * |---|---|---|
 * | C-001 | 根 `mod.rs` 不得有 `pub mod <域>;` | 域目录一旦 `pub mod`，整个子树就是公共面：`core-surface` 枚举的「公开面 = 根重导出」当场失真，外部还能直接深引绕过根 |
 * | C-002 | core 外的**代码**不得出现 `symbio_core::<域>`（含裸 `use crate::symbio_core::<域>;`） | 深引 = 消费方绕过根出口：符号从根导出移除后深引点照样编译通过，公开面变成两套。裸模块导入此前**没有**任何审计在拦（旧 E-010 的正则要求尾 `::`） |
 * | C-003 | **内核门面**导出的符号，**生产**消费方数必须 ≥2 | D-023 的判据（依赖方数量）此前只有**报告型** `core-surface-audit` 在数、判定权在人手里，于是长期停在 0 消费方 139 / 单消费方 64 上不动。A2 起判定范围 = `kernel.rs` 的公共面（D-185） |
 * | C-004 | 非内核根账本的**条目数**只许降 | C-003 收窄后，其余根出口不再被判「配不配占出口」；不加这一格就会留下一段只长不判的真空。新增共享类型默认插件本地（D-182），要占根出口必须显式改数字，而那一步由 `gate.d/35-baseline.mjs` 按基准版本判 |
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
// @codes C-001 C-002 C-003 C-004

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

const CORE_REL = 'crates/symbio-core/src'
/** 深引扫描范围：与 E-010 的 CODE_ROOTS 同源（壳侧是独立 crate，但同属消费方） */
const CODE_ROOTS = ['crates', 'crates/symbio-cli/src', 'tauri/src', 'tauri/src-tauri/src']

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
      // 内核公共面豁免：**0 条**（2026-10-26 D-190 后）。
      //
      // 最后一条 `ActorSpec` 是**协议接缝**（F3 冻结字段集），曾因 core 外唯一生产
      // 消费方是 `plugins/session` 而豁免；抢占判定随 D-190 搬进
      // `plugins/session/transcript/inbox.rs` 后，`ActorSpec` 有了第二个生产消费方
      // （`turn_exec` 构造 + `inbox` 构造判定者行），C-003 的「≥2 消费方」判据自行
      // 成立 ⇒ 条目按纪律 2 退役，不留空转豁免。
      //
      // `branch_hide_unselected`（节点树接缝）与 `EVENT_TURN_STAGE`（线上事件名）随 C-003
      // 的判定范围收窄到内核公共面而**移出本帐**：它们不是封闭语法，属域账本。
      // 前者仍是单消费方接缝（子树收法 `node_build::subtree_of` 只有一份），C 迁出
      // session 域时一并处置；后者已在 P1c 随 owner 声明迁回 `plugins/loop`（D-197），
      // core 不再出根。口径见 DE-023 D-185。
      //
      // 此前的投影类条目已清零：`turnstate` / `SloLatencyView` / `FallbackRateView` /
      // `mutations_latest_snapshot` 经 `isSinkCandidate` 核验**本来就不是单消费方**
      // （core 自己的投影域在生产使用它们），豁免条目空转，直接删除；`token_ledger`
      // 的唯一生产消费方是 `plugins/session/stats.rs`，按 D-180 连同类型与实现搬进
      // 会话插件（`plugins/session/token_ledger.rs`），根出口与条目同时退役。
    }

/**
 * 棘轮基线：0 消费方 / 单消费方的存量上限（自引用不计）。
 * 只许降——超了说明新符号进了根出口却没跨出 core，或某个消费方被删掉了。
 *
 * `0` 是规则 2 首批整改的结果：140 个「名字没跨出 core」的符号已从根出口收窄
 * （定义留在原域 `pub`，core 内走域内路径），只被测试/无人使用的存量另由
 * `dead-code-audit` R-002 逐项承认。
 *
 * `0/0` 是 **A2 之后**（2026-10-26）的上限：C-003 的判定范围收窄到 `kernel.rs` 门面的
 * 公共面（14 条封闭语法），其中 0 消费方 0 条、单消费方 0 条（D-190 后豁免账本为空）。
 * 此前 13 条单消费方仍是存量，但**移出 C-003**：它们不是封闭语法，属域账本，由 C-004
 * 冻结条数（270）并在 `--verbose` 的 `[L1]` 里逐条列出——C 迁出对应域时一并消失。
 *
 * 下列批次不是新增违规，是口径与下沉的记录：
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
 *   actors 定义域（`pattern_for_principal`，插件层不再出现主体类型名——NDC-001 同一
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
 *
 * - **单消费方投影下沉批**（2026-10-26）：`spend_ledger` 的视图类型与实现搬进唯一消费方
 *   `plugins/session`（DE-023 D-180），根出口与豁免条目同时删除——豁免账本 13 → 12。
 *   违规格计数不变（该符号本就在豁免里，不计入 `one`）。
 *
 * - **B2 投影豁免清零批**（2026-10-26）：`token_ledger` 同法搬进 `plugins/session`
 *   （唯一消费方 `stats.rs`），豁免账本 12 → 11，`one` 不变。另核验四条「投影类」豁免
 *   （`turnstate` / `SloLatencyView` / `FallbackRateView` / `mutations_latest_snapshot`）
 *   本就不是 `isSinkCandidate`（core 自己的投影域在生产使用它们），条目空转 ⇒ 删除；
 *   它们从未计入 `one`，基线不动。`ActorSpec` 随 D-190 有了第二个生产消费方而退役，
 *   `branch_hide_unselected` / `EVENT_TURN_STAGE` 两条接缝移出本帐（口径见 D-185）；
 *   `EVENT_TURN_STAGE` 随后在 P1c 随 owner 声明迁出 core 根出口（D-197）。
 */

const BASELINE = process.env.CORE_EXPORT_BASELINE
  ? (() => {
      const [zero, one] = process.env.CORE_EXPORT_BASELINE.split(':').map((n) => Number(n))
      return { zero, one }
    })()
  : { zero: 0, one: 0 }

/**
 * C-004：**非内核根账本**的条目数上限（只许降）。
 *
 * A2 起内核的公共面 = `kernel.rs` 门面（DE-023 D-185），C-003 只判这一份；
 * 其余根出口是**过渡期域账本**——它们随 C 的阶段迁出。冻结条目数是这段过渡期的
 * 唯一判据：新能力默认插件本地（D-182），不该再往 core 根出口加共享类型；
 * 要加就把数字改大，而那一步会被 `gate.d/35-baseline.mjs` 按基准版本判红。
 *
 * ## 270 → 396 是**口径重定**，不是公共面变宽（2026-10-26，D-191 拆 crate）
 *
 * 本审计只数根上的 `pub use`。拆 crate **之前**，core 与插件同住一个 crate，插件写
 * `crate::symbio_core::<符号>`，那些符号只需要 `pub(crate) use` 就能跨模块可见——
 * 于是它们**从来没进过这张账**。拆完之后插件住独立 crate，同一条依赖只能写成
 * `pub use`，一次性全数入账。
 *
 * 实测对照（同一个脚本、同一份判据，分别跑拆分支 606505eb 与当前树）：
 *   · 拆分支：根 `pub use` 270 条（= 本条基线），另有 `pub(crate) use` **121 个符号**；
 *   · 当前树：`pub use` 396 条，`pub(crate) use` 0 条——那 121 个换了写法，没换依赖。
 * 符号集 diff 也是这个结论：**新增 127 条、消失 0 条**，新增的正是那批 `pub(crate) use`
 * （外加 6 条拆过程中新归位的接缝），没有一个是「新能力往根出口塞共享类型」。
 *
 * 所以把数字提到 396 并**不给公共面放行**——它只是把旧口径漏掉的那半张账补上。
 * 真正该降的部分在别处：C-003 的 `unitOf` 曾把 `crates/` 下三十几个 crate 塌成同一个
 * 消费单位（见 `core-surface.mjs`），修好之后本账里「≥2 个 crate 消费」的有 325 条、
 * 单消费方 71 条、零消费方 0 条——**那 71 条才是下一批下沉对象**。
 *
 * 之前的 270→396 那次上调同理：拆 crate 后 `pub(crate) use` 全部升为 `pub use`
 * （跨 crate 必需），符号集实测**新增 127 / 消失 0**，不是公共面膨胀。
 *
 * 395（2026-10-27，−2）：测试替身外置。`StubLlmAdapter` 与 `trivial_actor_spec` 的
 * 消费方只有测试面，拆 crate 后 `cfg(test)` 对下游 crate 不生效（只在本 crate 编译
 * 自己的测试时存在），于是它们一直以 `pub` 常开。搬进 `symbio-testkit`（插件以
 * dev-dependency 引入）之后 core 的根出口不再为测试面买单——这是**真收窄**，
 * 不是改口径。
 *
 * 397（2026-10-27，+1）：`config_mount_dispatch` 出根。`impl_config_mount_vdfs!` 宏
 * 在**别的 crate 里展开** `$crate::…`，域模块 `plugin` 是 `pub(crate)` ⇒ 展开侧
 * `error[E0603]: module plugin is private`（setting / telegram 两个插件同时红）。
 * 宏引用**数不出来消费方**（消费方是宏调用点，源码里没有那条路径），所以唯一合规
 * 写法就是走根平铺。这是「拆 crate 收窄域可见性」漏掉的一处，不是新增共享类型。
 *
 * 396（2026-10-27，+1）：`VdfsListResponse` 上移。它是 `vdfs/list` 与 `vdfs/root` 的响应
 * 形状，此前定义在 vdfs 插件内部；`agent` 侧拼子会话地址时只能取 `Value` 再手写
 * `v.get("path")`——字段名漂移不会编译失败，只在子会话订阅时静默报错。消费方实测 2 个
 * （vdfs 构造、agent 读），满足 C-003，属 D-196 的「被两个插件共同需要的类型就属于领域」。
 * 这是**该涨的一次**：它换掉的是一处跨 crate 的字面量耦合，不是新开一个出口。
 *
 * 389（2026-10-27，−7）：P1c 首批事件名随 owner 的 `EventDecl` 迁出——`session.config` /
 * `turn.stage` / `authz.granted` / `memory.recalled` / `task.ready`（loop）与
 * `system.triggered` / `system.health`（session）不再需要 core 根出口。这是**真收窄**：
 * 写下与读取这些名字的插件从自己的声明模块取常量，core 不再替它们持有词汇（D-197）。
 *
 * 377→**371**（2026-10-27，D-206）：任务族五格的拼写改由折叠的构造口交出，`EVENT_TASK_*`
 * 五条随内核常量一起不再出根（−5），`TaskVocab` 出一条（+1）；`autonomy` 投影连同
 * `AutonomyView` 随**唯一生产消费方**下沉 `plugins/session`（−2）。登记值此前停在 381 而
 * 实测已是 377——那 4 条是上一批同批退役却没同步的数字（黄提示不是红灯，所以它活了一轮）。
 *
 * **`one` 那一格本批涨过又落回 11**，落回的方式值得记一笔：`autonomy()` 改名 `autonomy_for()`
 * 之后它才**显为**单消费方——此前 `collectCoreDomainConsumers` 按名字匹配，core 里那句
 * `pub mod autonomy;` 与 `pub use projection::autonomy::…` 都被读成「core 内另有生产消费方」。
 * 那是口径的盲区（模块声明不是使用），不是符号的属性。处置按 D-200：唯一消费方在哪个插件，
 * 投影就搬进哪个——**不是**把数字提到 12，也不是给它挂一条豁免。
 */
// core-export-allow C-004: 拆 crate 后 pub(crate) use 全部升为 pub use（跨 crate 必需），270→397 是计数口径补全而非公共面膨胀（符号集实测新增 128 / 消失 0）；其中 +1 是 impl_config_mount_vdfs! 宏必须走根平铺（域模块 pub(crate) 时展开侧 E0603）。此后 397→395 是测试替身外置带来的真收窄；395→396 是 VdfsListResponse 上移（消费方 2，换掉 agent 侧手写的 "path" 字面量，D-196）；396→389 是 7 条 owner-local 事件名迁出根出口（D-197）；389→386 是 `conation` 投影随唯一
// 消费方下沉 plugins/session，`conation` / `ConationView` / `EVENT_CONATION_EXPRESSED`
// 三条出口一起退役；386→381 是 `recall` 投影随唯一消费方下沉 plugins/loop，`recall` /
// `RecallView` / `RecallEntry` 三条出口与 `EVENT_MEMORY_CONSOLIDATED` /
// `EVENT_MEMORY_FORGOTTEN` 两条同批退役（D-201）；登记值此后**停在 381 而实测已是 377**
// （那 4 条是同批退役却没同步数字的黄提示，不是本批新增）；377→371 是 D-206：任务族五条名字
// 不再经 core 出根（−5，折叠在构造期收 `TaskVocab`，+1）、`autonomy` 投影随唯一生产消费方
// 下沉 plugins/session（`autonomy` / `AutonomyView` 两条出口退役，−2）
const DOMAIN_LEDGER_BASELINE = process.env.CORE_LEDGER_BASELINE
  ? Number(process.env.CORE_LEDGER_BASELINE)
  : 371

/**
 * C-004 的**下沉候选**格：域账本里「单消费方」的条目数上限（只许降）。
 *
 * 条目总数只说明账本有多长，不说明其中有多少是「只有一处用、本该下沉」的投影。
 * [DE-024](../docs/003-design/024-能力横向扩充的通路.md "DE-024-能力横向扩充的通路") D-200
 * 把这一格列为词汇棘轮的一格：新能力默认插件本地，这一格涨 = 又多了一个
 * 「只为单点需要而占根出口」的共享符号。
 *
 * `CORE_LEDGER_ONE_BASELINE` 只给回归测试用，`35-baseline` 另判覆盖值不得更松。
 */
const DOMAIN_ONE_BASELINE = process.env.CORE_LEDGER_ONE_BASELINE
  ? Number(process.env.CORE_LEDGER_ONE_BASELINE)
  : 11

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
const { coreDir, domains, symbols, modules, kernelExports } = surface

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

const rootRaw = fs.readFileSync(path.join(coreDir, 'lib.rs'), 'utf8')
const rootLines = rootRaw.split(/\r?\n/)
rootLines.forEach((line, i) => {
  const m = line.match(/^\s*pub\s+mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*[;{]/)
  if (!m) return
  if (waived(rootLines, i)) return
  report(
    'C-001',
    `根 lib.rs 出现 \`pub mod ${m[1]};\` —— 子目录不得对外开模块门；改成 \`mod ${m[1]};\`` +
      `（只在 crate 内可见则 \`pub(crate) mod ${m[1]};\`），` +
      `把需要出现在公共面上的东西在根 \`pub use\` 显式列出`,
    `${CORE_REL}/lib.rs:${i + 1}`,
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

// ==================== C-003 双消费方（内核公共面） ====================

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

// 判定范围 = `kernel.rs` 门面导出的封闭语法（DE-023 A2 / D-185）。没有门面的夹具仓库
// 退回全表面口径——否则 C-003 会在一个空集上恒绿，夹具与「判据失效」长得一模一样。
const judgedRows = kernelExports.size > 0 ? rows.filter((r) => kernelExports.has(r.name)) : rows

const waivedNow = new Set()
for (const [name, reason] of Object.entries(WAIVERS)) {
  if (!String(reason ?? '').trim()) {
    report('C-003', `豁免账本里 \`${name}\` 的理由为空 —— 空理由不算豁免`)
    continue
  }
  const row = judgedRows.find((r) => r.name === name)
  if (!row) {
    report('C-003', `豁免账本里的 \`${name}\` 已不在内核公共面 —— 该条豁免已失效，请删除`)
    continue
  }
  if (row.units.length >= 2) {
    report('C-003', `豁免 \`${name}\` 已失效（现有 ${row.units.length} 个消费方）—— 请从账本删除`)
    continue
  }
  waivedNow.add(name)
}

const selfRef = judgedRows.filter((r) => isSelfReference(r))
// 「没人用」按**含测试**的口径：只有别模块的测试在用的符号，收窄它当场把那些测试编译坏
// （C-002 又不许它们深引）⇒ 它够格占根出口，不进本条。
const zero = judgedRows.filter((r) => r.unitsAll.length === 0 && !waivedNow.has(r.name))
// 「只有测试在用」单列一档：它是**合法的根出口**，但也不是架构元素——既不收窄、不下沉。
const testOnly = judgedRows.filter((r) => r.units.length === 0 && r.unitsAll.length > 0)
// 「该不该下沉」按**不含测试**的口径：测试依赖的是被测对象，不是长期接口。
const one = judgedRows.filter((r) => isSinkCandidate(r, coreConsumers) && !waivedNow.has(r.name))
const many = judgedRows.filter((r) => r.units.length >= 2)
// 内核接缝：core 外只有 1 个生产消费方，但 core 内另有生产模块在用（口径 2）⇒
// 搬走会依赖反转，它本来就该留根出口。既不算 `one`，也不该被读成「≥2 达标」。
const seam = judgedRows.filter(
  (r) => r.units.length === 1 && !isSinkCandidate(r, coreConsumers) && !waivedNow.has(r.name),
)

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

// ==================== C-004 非内核根账本只降不升 ====================
//
// 不在门面里的根出口是过渡期域账本：它们随 C 迁出，期间**只许减**。这一格与 C-003
// 互补——C-003 判内核符号「配不配占出口」，C-004 判域账本「有没有又长回来」。
const ledgerRows = rows.filter((r) => !kernelExports.has(r.name))
const ledgerCount = ledgerRows.length
// 报告用（不判定）：域账本里仍「没跨出 core」/「单消费方」的条目，是 B4 / C 的下沉候选。
const ledgerZero = ledgerRows.filter((r) => r.unitsAll.length === 0)
const ledgerOne = ledgerRows.filter((r) => isSinkCandidate(r, coreConsumers))

if (kernelExports.size > 0 && ledgerCount > DOMAIN_LEDGER_BASELINE) {
  report(
    'C-004',
    `非内核根账本 ${ledgerCount} 条 > 基线 ${DOMAIN_LEDGER_BASELINE} —— 新增共享类型默认插件本地（D-182）；` +
      `确需占根出口的，先证明消费方 ≥2，并同步本脚本的 DOMAIN_LEDGER_BASELINE（数字要随决策走）`,
  )
}

if (kernelExports.size > 0 && ledgerOne.length > DOMAIN_ONE_BASELINE) {
  report(
    'C-004',
    `域账本单消费方 ${ledgerOne.length} 条 > 基线 ${DOMAIN_ONE_BASELINE} —— ` +
      `只有一个消费方的投影不是共享类型，下沉到消费方（DE-024 D-200）；` +
      `确实该占根出口的，先写清第二个消费方或同步 DOMAIN_ONE_BASELINE`,
  )
}

// ==================== 输出 ====================

const ruleLine = (ok, rule, desc) =>
  console.log(`${ok ? green('✓') : red('✗')} ${rule}  ${dim(desc)}`)

if (errors.length === 0) {
  console.log('\n== core-export-audit ==')
  ruleLine(true, 'C-001', '根出口唯一：根 lib.rs 无 `pub mod <域>;`')
  ruleLine(true, 'C-002', `core 外无深引（${domains.size} 个域目录动态取自 core）`)
  ruleLine(
    true,
    'C-003',
    `双消费方棘轮（内核公共面 ${judgedRows.length} 条）：0 消费方 ${zero.length}/${BASELINE.zero}` +
      ` · 单消费方 ${one.length}/${BASELINE.one} · 仅测试在用 ${testOnly.length}` +
      ` · 自引用 ${selfRef.length} · 内核接缝 ${seam.length} · ≥2 ${many.length}`,
  )
  if (kernelExports.size > 0) {
    ruleLine(
      true,
      'C-004',
      `非内核根账本 ${ledgerCount}/${DOMAIN_LEDGER_BASELINE}（只许降）` +
        ` · 其中 0 消费方 ${ledgerZero.length} / 单消费方 ${ledgerOne.length}/${DOMAIN_ONE_BASELINE}（下沉候选，只许降）`,
    )
    if (ledgerCount < DOMAIN_LEDGER_BASELINE) {
      console.log(
        yellow('提示') +
          `：非内核根账本已降到 ${ledgerCount}，请同步本脚本的 DOMAIN_LEDGER_BASELINE = ${ledgerCount}`,
      )
    }
  }
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
    for (const r of seam)
      console.log(`  [ S] ${(r.domain ?? '?').padEnd(12)} ${r.name} -> ${r.units.join(', ')}（core 内另有生产消费方，不可下沉）`)
    for (const r of ledgerOne)
      console.log(`  [L1] ${(r.domain ?? '?').padEnd(12)} ${r.name} -> ${r.units.join(', ')}（域账本）`)
    for (const r of ledgerZero) console.log(`  [L0] ${(r.domain ?? '?').padEnd(12)} ${r.name}（域账本）`)
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
