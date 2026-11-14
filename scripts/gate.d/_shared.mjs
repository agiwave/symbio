// gate.d 共享库：基线、输出解析、环境探针。任务模块按需引入。
//
// ## 基线棘轮：向上自动、向下才红
//
// 三态判定只有**一处**定义（`ratchetVerdict`，纯函数）：低于基线红、高出但在
// `BASELINE_GRACE` 内黄字提示、超出容差红。`BASELINE` 本身被下调由
// `gate.d/35-baseline.mjs`（元守卫）判红——那是向下方向唯一的放宽口，且必须配
// `// baseline-allow <key>: <理由>` 一行豁免。
//
// 但**应用到门禁时**走 `ratchetOrBump`（带副作用的网关版）：实测 > 基线就**就地把数字
// 写回本文件**（只增、不写注释、本地顺手 `git add`），判绿；实测 < 基线才判红。
// 理由：棘轮的真实价值只在向下方向（测试被删 ⇒ 红，逼人查）。向上是预期内的正常增长，
// 不该拦人、更不该每次手写一段论证——那论证正是被本仓规则禁止的「源码内变更史」。
// 故本文件里 `BASELINE` 对象**不应再堆各批次的变更注释**；要追变更看提交历史。

import fs from 'node:fs'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { stripAnsi, yellow, green, red } from '../color.mjs'

// 本文件自身路径：自动上调 `BASELINE` 时就地重写这里（`acceptBaseline`）。
const SHARED_MJS = fileURLToPath(import.meta.url)
const SHARED_DIR = path.dirname(SHARED_MJS)
const REPO_ROOT = path.resolve(SHARED_DIR, '..', '..')

/**
 * 通过数基线（**只增不减**；跑高了请更新这里并说明理由；**跑低了要说明理由**）
 *
 * VdfsChange 信封 = `{path, data?}`、操作枚举整个退役——信封构造
 *      守卫（`bare` 无载荷 / `with_data` 带载荷 / 无 `change` 键）、门面原样透传、
 *      transcript 四帧生命周期（首帧全量副本 / delta 窄载荷 / removed 状态帧 /
 *      运行态随节点视图）。`transcript_stream` 已随 D-025 退役，908 中相关
 *      用例随之删除，由 VDFS 变更通道的新用例接替。
 * 908：流式链路评审 A–D 批的回归锚点——`event_bus` 满通道语义（不摘除订阅 /
 *      `Closed` 才摘除 / resync 标记形状 / 满通道后标记必达）与**扇出不复制载荷**
 *      （`PluginPayload::Data(Arc<Value>)` 的 `Arc::ptr_eq` 断言），外加
 *      `transcript_stream` 的扇出共享与信封可解回事件（该模块已退役）。逐条见
 *      `docs/archive/streaming-chain-review-2026-09-22.md` §0.1。
 * 901：转写帧日志分级——`FrameLogLevel`（骨架 INFO / 细节 DEBUG）与
 *      `frame_log_of` 的相位判据，折行器新增 `foldable`（骨架帧与首帧不可折）。
 *      新增「骨架帧不可折」「相位分界」「行尾不落空格」用例（3）。
 * 898：控制台日志降噪——新增 `logger` 级别闸门用例（2）与 telegram 配置「缺键不是错误」
 *      用例（2）。闸门：无 subscriber 路径默认 INFO，debug 静默（`--verbose` / `SYMBIO_LOG`
 *      放开）；telegram 结构级 `#[serde(default)]` 使「只有身份键的 PLUGIN.yml」解析为默认值
 *      而非 Err（此前每次启动一条假 WARN）。
 * 894：S24 收口——转写流帧收成一条 `ChatNode`：删除 `NodeOp` / `NodeChange`，
 *      `delta`（增量，与 `content` 互斥）与 `status = removed`（删除状态迁移）落到
 *      `ChatNode` 上，告警下沉为 `TranscriptWriter::warn`；转写核心日志的「纯增量」
 *      折行（`DeltaLogCoalescer`）与请求级会话快照的回退判据一并落地。相应新增/重排了
 *      消息合并、删除帧、压缩终态、转写往返、折行边界（`seq`/图不受影响）、快照回退
 *      判据等用例（含一处 `llm_state_frame` → `llm_message_frame` 回归修复的锁定用例）。
 * 881：v1→v2 迁移 + 节点状态流 S20~S23 + 压缩消息流化 + 中止收口终态化 + 协议
 *      增量提取器逐字节回归 + tool_name 线上名投影 + MCP camelCase/载荷语义网 +
 *      extract_result 判定顺序。逐批明细见对应提交（`git log --grep=<批次/主题>`；
 *      本仓库不维护变更日志，变更历史即提交历史）。
 */
// @gate test
export const BASELINE = {
  // 各测试套件的计数基线（**只增不减**）。
  // 向下（删测试）必须配 `// baseline-allow <key>: <理由>`（见 `35-baseline.mjs` 元守卫，防『悄悄下调基线』）。
  // 向上：门禁实测超过即自动把数字写回本文件，不在此堆各批次变更注释——变更原因看提交历史。
  // baseline-allow rustTests: 只增棘轮（`ratchetVerdict` / `ratchetErosion` 的唯一口径）。
  //   调低 ⇒ 在此写下理由与决策号；调高 ⇒ 门禁自动写回，不堆注释。
  //   当前 2152 = `cargo test --workspace` 的本地求和。逐次增减的归属见
  //   `git log -S'rustTests'` 与各提交的「门禁：」段——**这里不记变更史**：原先 147 行
  //   「A → B」注释里只有最后一条会被 `baselineWaivers` 读到（Map 按键覆盖），其余是死重量。
  rustTests: 2152,
  kernelRustTests: 131,

  // baseline-allow ciRustTestsTotal: CI 侧同一口径（同一条 `cargo test --workspace`、同一批
  //   目标），当前 2242。逐次折算的归属同样归 `git log`——理由同上一条，不在这里记变更史。
  ciRustTestsTotal: 2242,
  cliRustTests: 21,
  tauriRustTests: 4,
  // doc-link-allow D-010: 下一行是退役文件的历史叙述（已把这两个文件删掉），不是入链
  // baseline-allow vitestFiles: （`76324817`，D-063 会话呈现合流）退役 `tauri/src/schemas/conversation_line.ts` 与其 185 行 spec `tauri/src/schemas/__tests__/conversation_line.spec.ts`（整文件删除 ⇒ 测试文件 55→54）。被测对象随「对话/工作分栏退役」消失，判据随之下沉：新形态由 `e2e/cases/t24-panel-split.mjs` 与 `crates/symbio-plugin-loop/src/context/conversation_view.rs` 侧覆盖，不是丢掉覆盖。
  // baseline-allow vitestTests: 上述 spec 的用例 + `tauri/src/stores/__tests__/appearance.spec.ts` 随分栏退役收窄的那部分，合计 856→838（-18）。`appearance` 的其余行为仍由该 spec 的保留用例守着（只删了分栏相关的那一段），非侵蚀。
  // baseline-allow vitestTests: D-111 净删 6 条（894→888）——删的是**已不存在的被测对象**：`registry/messageTypes` 的 `messageCompactionStats` / `messageMemoryStats` / `compactionTriggerLabel` / `formatTokens` 四条前端口径随裁决迁到交付出口（那 6 条逐个断言的正是这四个函数的取值与格式化）。覆盖没有丢，是换了承担者：同一口径现在由 Rust 侧 `crates/symbio-plugin-session/src/stats_reading.test.rs` 7 条判据钉（四行齐全 / 失败路径报「当前水位」不补 0 / 无数字出行即消失 / 压缩产物两行 / 非压缩的 notice 不误读 / token 单位只有一处 / 未知触发词原样透出），前端只留「有行就逐行排版」的渲染判据（`MessageNode.spec.ts` 那三条改吃 `stats` 行，条数不变）。非侵蚀。
  // baseline-allow vitestTests: 1002 → 1003（+1，D-147 / D-149）——新增的是**判据用例**，不是补回覆盖：
  // 折叠顺序反转后「主体优先 / 残留格兜底 / 旧形态不再归一」三组各要一条断言，
  // 由 8 条变 9 条（原「历史裸字符串一路通吃」那条拆成地址归一 + 旧形态不归一，净 +1）。
  // baseline-allow vitestTests: 1003 → 1024（+21，折叠骨架按需重取）——新增的是判据用例：
  // `skeletonLazy.spec.ts` 钉本地释放与按需重取的折叠状态，`MessageNode` / 会话 store 补
  // 重取路径的渲染与竞态断言。不是补回覆盖。
  vitestFiles: 73,
  vitestTests: 1097,
  verifyPrograms: 17,
  verifyNegatives: 4,
  // baseline-allow e2eCases: 读通道禁令（e2e 不得读任何内部存储文件——`messages.json` / `v2-events.wal` / `session.json` 一并禁）下删 3 个用例：t33 熔断（存储层判据，已由 `wal.test.rs` / `active.test.rs` 覆盖）、t40（断言全为 WAL 白盒入格，可见效果已由 t29 记忆召回 / t35 技能编译覆盖）、t43（messages-json-is-rebuildable，判据「真源已换、缓存可弃」已下沉 `store/tests.rs`，且 messages.json 已停写）。45 → 42，非侵蚀：三条的判据都下沉到了 Rust 侧，不是丢掉覆盖。
  // 增 t76（召回可见域）。
  // 增 t99（工具端口三路径：装行执行 / 不装行让位 / 缺行响亮失败并落事实）。
  // baseline-allow e2eCases: 96 → 91（D-332）——`60cedd50`（B2 环侧编排退场，D-318）删 5 条
  // 引擎绑定的 e2e 用例（这些用例的接线点随旧 engine 一起消失，留着只会测不到东西）。后续随
  // 新义务环接入**只会回升**：B 类能力每接回一组，对应的 e2e 用例重新落地。
  // baseline-allow e2eCases: 91 → 90（D-318）——`346b3134` 整例退役 t41 技能快路：
  // 它唯一独有的断言依赖 `session.skill_fast_path`，该快路已随 B2 退场，没有生产事实可断；
  // 能力断言由 t35-skill-compile 承接（退役口径见 `docs/archive/013-e2e实施表.md`，2026-11-04 盘点快照）。
  // 收紧基线，不是放松。
  // baseline-allow e2eCases: 97 → 88（D-395）——9 条整例退役（t89/t91/t93/t94/t98/t100/t101/
  // t102/t103），它们钉的是被撤回的专用验证包；判据对象随实现一起消失，通用扩展面由
  // `t96-glossary-extension.mjs` 承接。非侵蚀。
  e2eCases: 92,
  // 累计「补件 : 迁移」的**欠账本金**（D-372，`capability-migration-audit` 的付账线）。
  // 不是「比值上限」：范围基线一变计数就变，比值当阈值会假红。这里记的是判据落地当天的
  // 存量按脚本每轮重算，不手抄。文档里的手写读数
  // 的**旧尺**读数（同一范围新尺读 7），与今天不可比；口径与拆分见 `capability-migration-audit`
  // 的计数规则与 `D-372`。
  // 增量线只问「从这天起欠账有没有变大」——新增一个补件必须由一次迁移抵账，`debt > 0` 即红。
  // 绝对欠账 6 笔按 D-375 的检查点递减、2026-11-25 归零；台阶表在审计脚本的
  // `DEBT_SCHEDULE`，不在这里复制一份。
  // 两格是 floor 棘轮（`35-baseline` 守）：只许在**欠账为 0** 时随实测上移（等价于把窗口
  // 前移，判据结果不变）；欠账不为 0 时改大是**豁免**，必须先有决策。
  capabilitySupplements: 21,
  capabilityMigrations: 15,
  // 换述读数（D-097）：`recall_eval.rs` 的第四个数字带着一组新断言——指纹锁、
  // 每条换述必须真的换了措辞、依赖度定义域、54 条换述逐条验「承重」（换成无关问句
  // 读数必须精确上抬 1/(n·m)）、以及时间兜底案上读数不动。224 → 235，全是**新增判据**，
  // 没有一条是为了让数字变大而写的：删掉任何一条，对应那半句结论就没有守卫了。
  // 挂生产内核：`invariants.rs` 的 seq 重号在「严格单调且无跳号」语义下
  // 报两条（重号位 + 其后错位），`termination.rs` 加了内核判定的边界自证
  // （恰在上界合法 / 越界一轮判否）。235 → 237。
  // 增 1 条（243→244）：`invariants.rs` 在**真实日志**上注入一处悬空溯源
  // （把一条事件的 `produced_by` 指到片内一个被删掉的号），断言 `I2DanglingProvenance`
  // 必须响。它同时钉住新判据的反向纪律——`enable_i2_dangling: false` 时那条必须消失。
  // 增 3 条（244→247）：停机原因闭集。① 自造 `violated_log` 注入一处越界
  // `reason`（`"timeout"`，用 `authz.denied` 而不是再塞一条 final——后者会把那一轮
  // 收束掉，于是「无收束」那条判据反而消失，注入一条违规弄丢另一条是自假绿）；
  // ② ③ 在**真实数据形状**上喂合法值 / 越界值各一次：合法值必须全绿（否则判据在误杀）、
  // 越界必须红。真实日志的**回放**不跑这条——夹具只留信封、`reason` 在载荷里、且它采于
  // 之前；硬跑只会得到「每条收束格都缺 reason」的假红，那不是判据错了。
  // baseline-allow verifyAsserts: §四.1 裁决删按轮配对判据（`final_unique_per_turn` / `open_unique_per_turn` / `unresolved_turns`）——判据资产与实现同批退场，`invariants.rs` 的 I1DoubleFinal / I3NoClosure 两处反向用例随之删除（256→254），配对改由义务环五事实机制自己记账。非放宽。
  verifyAsserts: 254,
  // 唯一一处**向上**的 panic 面（实测 77）：段归档与它的读侧——
  // `WalStore::archive_before`、归档段并集的缓存（`archive_events` 拿读锁 + 写回写锁）、
  // `range` 拼接时的读锁，都要拿同一个 `RwLock`，沿用本模块既有的「锁当无中毒用」约定
  // （同 `len`/`head` 的 `expect("WalStore 锁中毒")`）。`SessionStore` 事实源
  // 句柄表（`facts` / `snapshot` / `delete_session`）重复同一条约定：表锁只在持锁期间
  // panic 时中毒，本仓不把中毒当可恢复状态。每一处均已按 PN-001 登记，理由同构：
  // 改成毒后恢复是行为变更，需单独 ADR。这不是放宽棘轮——新增的都是既有约定的复制，
  // 不是新引入的健壮性缺口。（把 `len` 的 expect 收进 `cfg(test)` 后实测曾降到 72；
  // 那一轮按 floor 口径保留 73，这里一并落定为实际值 77。）
  // 另加两处同构的复制：`runtime_facts`（读事实源表锁）与 `runtime_cache`
  // （运行态投影缓存表锁）。两处都只是既有「锁当无中毒用」约定的又一个持有者，
  // 不引入新的健壮性缺口；基线随之 77 → 79。
  // 抽内核：`invariants/mod.rs` 的四个检查改成「解析层构造内核输入 + 判定走
  // kernel.rs」后，各加一处 `expect` 把「内核返回的下标/id 必来自本切片」钉成显式
  // 约定（解析层与内核的输入约定若破，这里当场响而不是静默错位）——四条理由同构、
  // 均为穷尽性不变量，已按 PN-001 登记在 mod.rs。基线随之 79 → 83。
  // baseline-allow panicSites: D-387 的报表登记判据改用 Result（不 panic），这里是撤销上一笔的临时上调，不是放松。
  panicSites: 83,
}


/**
 * 棘轮的**回填容差**：实测高出基线、差值在此格数以内只提示，超出即判红。
 *
 * 为什么要有上限：「高于基线」不是无害的——基线不跟着涨，删掉同样多的测试仍然全绿，
 * 棘轮就被削掉了同样的格数。而提示是可以被无限忽略的（日志天天有、门禁天天绿）。
 *
 * **适用边界（2026-10-10 调整）**：测试数 / 断言数这类「只增」棘轮已改为
 * **向上自动写回**（见 `ratchetOrBump`），不再靠本容差逼人回填，故本常量对它们
 * 实际不再触发红。它仍服务于 `35-baseline.mjs` 里的**其他**棘轮（前端覆盖率阈值、
 * 死码存量、core 单消费方符号数），以及 `20-frontend-static` 的覆盖率阈值提示。
 */
export const BASELINE_GRACE = 5

/**
 * 棘轮三态判定的**唯一定义**——所有棘轮（本地 / CI / verify）共用一条判据，
 * 各自决定红不红必然演化成两套口径。
 *
 * | 实测 vs 基线 | 结论 |
 * |---|---|
 * | 低于 | 红：有测试或证据被移除（棘轮存在的唯一理由） |
 * | 高出但在 `BASELINE_GRACE` 内 | 绿 + `warn`：提示回填 |
 * | 高出且超出容差 | 红：没回填的基线等于没有棘轮 |
 * | 等于 | 绿，无 note |
 *
 * `kind` / `unit` 只影响措辞，不影响判定。
 */
export function ratchetVerdict({ actual, baseline, name, kind = '通过数', unit = '测试' }) {
  if (actual < baseline) {
    return { ok: false, note: `${kind} ${actual} < 基线 ${baseline}（有${unit}被删或失败）` }
  }
  if (actual > baseline + BASELINE_GRACE) {
    return {
      ok: false,
      note: `${kind} ${actual} > 基线 ${baseline}：超出回填容差 ${BASELINE_GRACE} ⇒ 更新 BASELINE.${name}`,
    }
  }
  if (actual > baseline) return { ok: true, warn: true, note: `${kind} ${actual}（基线待更新）` }
  return { ok: true }
}

/**
 * BASELINE 的**只增**判据：当前值只要低于任一基准版本里的值，就是削判据。
 *
 * `BASELINE` 是源码常量，而它是「有测试被删就红」这条判据本身——把它改小不需要动
 * 任何测试、任何守卫代码，门禁照样全绿，且**不留痕迹**（与「测试消失」同形）。
 *
 * **基准是「触碰过该文件的最近两个版本」，不是分支根**：远历史里基线本来就低，拿它比
 * 会让每一跑都红，于是豁免被喂到失效；只比最近一版又漏掉「已经提交的那次下调」。
 * 两版足够让**下调发生的当次**变红，无论它还在工作区还是已进 HEAD。
 *
 * ⚠️ 只判 `BASELINE` 一个对象是**覆盖面**问题而非正确性问题——`ratchetErosion` 把它
 * 泛化成 {@link ratchetErosion}（多落点 + 两种方向），本函数保留为 `dir: 'floor'`
 * 的那一格，语义一字未改。
 */
export function baselineErosion(currentText, baseTexts) {
  return ratchetErosion({ dir: 'floor' }, parseBaselineCells, currentText, baseTexts, baselineWaivers(currentText))
}

/**
 * **棘轮元判据**：把「当前值不得比基准版本更松」抽成一处。
 *
 * 两种方向，都是「只许往紧的方向改」：
 * - `floor`（**只增**，下限）：当前值不得**小于**基准值。用于「测试数 / 覆盖率阈值」
 *   这类**越多越严**的判据——调低就是削判据。
 * - `ceiling`（**只减**，上限）：当前值不得**大于**基准值。用于「死码存量 / 未跨出
 *   core 的符号数」这类**越少越严**的判据——调高就是削判据。
 *
 * 两者共用这一处，是为了让「什么算削判据」只定义一次：`35-baseline.mjs` 的
 * `RATCHETS` 清单每加一条落点，不必重新决定一次红不红——那正是 M2 把
 * `ratchetVerdict` 收进单点的同一条理由。
 *
 * `cellsOf(text)` 是**提取函数**，由调用方给（各落点的形状不同：`BASELINE` 是对象
 * 字面量、`CORE_EXPORT_BASELINE` 是 `zero:one` 对、`vitest.config.ts` 是 `thresholds`
 * 嵌套块）。取不到任何格时返回空 Map，**由调用方决定**是红还是跳过——本函数不替它猜。
 *
 * ⚠️ **契约：`cellsOf` 必须返回 `Number`，不能返回字符串。** 两个方向都靠 `<` / `>`
 * 判定，而 JS 的字符串比较是**字典序**：`'9' < '10'` 为 **false**（`'9'` > `'1'`）。
 * 一个返回字符串的提取器会让 `9 → 10` 这类**下调**静默通过——方向恰好是最危险的那一侧。
 */
export function ratchetErosion({ dir = 'floor' } = {}, cellsOf, currentText, baseTexts, waivers) {
  const current = cellsOf(currentText)
  const waivedKeys = waivers ? new Set(waivers.keys()) : new Set()
  const loosened = (now, then) => (dir === 'ceiling' ? now > then : now < then)
  const worst = new Map()
  for (const { ref, text } of baseTexts) {
    for (const [key, then] of cellsOf(text)) {
      const now = current.get(key)
      if (now !== undefined && !loosened(now, then)) continue
      const hit = worst.get(key)
      if (hit && hit.from >= then) continue
      worst.set(key, { key, from: then, to: now ?? null, waived: waivedKeys.has(key), source: ref })
    }
  }
  return [...worst.values()]
}

/**
 * 已登记的基线豁免（键 → 理由）。豁免形态与 `dead-code-allow R-002: 理由` 同一条
 * 约定：**理由非空**，否则不算豁免——空理由的豁免等于没有判据。
 */
/**
 * 自动上调 `BASELINE.<key>` 的数值并重写本文件（**只增不减，不写注释**）。
 *
 * 棘轮的真实价值只在向下方向（实测掉下去 ⇒ 红，逼人查「是不是丢了测试」）。
 * 向上是预期内的正常增长：门禁检测到「实测 > 基线」就**就地写回新数**，不写任何
 * 注释——自动发生的，无需解释。这把原设计里「每次加测试都要手写一段论证再回填」
 * 的仪式整个去掉（那段论证就是被本仓规则禁止的「源码内变更史」）。
 *
 * 仅当 `value > 当前值` 才写，绝不下调（下沉由 `35-baseline.mjs` 的元守卫拦）。
 * 本进程单线程，同步读写不会与同批并发的 `35-baseline` 读产生撕裂。
 *
 * @param stage 仅本地模式传 true：写回后一并 `git add` 本文件，使上调随本次提交走
 *              （与 `autoWork` 对 fmt / 事实文件生成的处理同一条约定）。
 * @returns true 表示确实写回（数值上调）；false 表示无需改或写入失败。
 */
export function acceptBaseline(key, value, { stage = false } = {}) {
  let text
  try {
    text = fs.readFileSync(SHARED_MJS, 'utf8')
  } catch {
    return false
  }
  const re = new RegExp(`^  ${key}: (-?\\d+),$`, 'm')
  const m = re.exec(text)
  if (!m) return false
  const current = Number(m[1])
  if (!(value > current)) return false
  const next = text.replace(re, `  ${key}: ${value},`)
  try {
    fs.writeFileSync(SHARED_MJS, next, 'utf8')
  } catch {
    return false
  }
  if (stage) {
    try {
      spawnSync('git', ['add', '--', SHARED_MJS], { cwd: REPO_ROOT })
    } catch {
      /* 暂存失败不阻断门禁 */
    }
  }
  return true
}

/**
 * 棘轮判定 + 向上自动上调（网关版）。`ratchetVerdict` 保持纯函数（供单测），
 * 本函数在其上叠加「向上即写回」的副作用：
 * - 实测 > 基线：自动上调（只增，无注释），判绿；
 * - 实测 < 基线：判红（向下是唯一要守的方向，对应「测试被删」）；
 * - 相等：判绿。
 *
 * 仅当 `ctx.autoBump` 为真（本地门禁）才真的写回；CI 只亮绿灯、不落盘（CI 无法提交，
 * 且上调让它变绿正是预期）。`ctx.autoBump` 由 `gate.mjs` 设为 `!ctx.ci`。
 */
export function ratchetOrBump({ actual, baseline, name, kind = '通过数', unit = '测试', ctx }) {
  if (actual > baseline) {
    const bump = Boolean(ctx?.autoBump)
    if (bump) acceptBaseline(name, actual, { stage: true })
    return { ok: true, note: bump ? `基线自动上调至 ${actual}（无注释）` : '基线待更新（本地门禁会自动上调）' }
  }
  return ratchetVerdict({ actual, baseline, name, kind, unit })
}

/**
 * 限期豁免是否仍在有效期内。
 *
 * `expires` 是**失效日**：当天仍然有效，次日起失效。写成「复核日」会被读成自动续期，
 * 而「到期未续即红」才是唯一可机读的方向（S2）。日期不是 `YYYY-MM-DD` 即视为无效——
 * 没有到期日的豁免就是永久豁免，宁红不静默。
 */
export function exemptionActive({ expires, today }) {
  const e = String(expires ?? '')
  const t = String(today ?? '')
  if (!/^\d{4}-\d{2}-\d{2}$/.test(e) || !/^\d{4}-\d{2}-\d{2}$/.test(t)) return false
  return e >= t
}

export function baselineWaivers(text) {
  const out = new Map()
  for (const m of text.matchAll(/^ {0,2}\/\/ baseline-allow ([A-Za-z][A-Za-z0-9_]*): (\S[^\n]*)$/gm)) {
    out.set(m[1], m[2].trim())
  }
  return out
}

/**
 * 从 `_shared.mjs` 源码文本里取出 BASELINE 的数值格。
 *
 * 只认对象体内的 `  键: 数字,` 一种形状，且**只扫对象体**——不然注释里那句
 * 「回填 `1339 → 1341`」这类文本会被当成一格，而它描述的恰恰是历史值。
 */
export function parseBaselineCells(text) {
  const norm = text.replace(/\r\n/g, '\n')
  const start = norm.indexOf('export const BASELINE = {')
  const cells = new Map()
  if (start < 0) return cells
  const body = norm.slice(start)
  const end = body.indexOf('\n}')
  const lines = (end < 0 ? body : body.slice(0, end)).split('\n')
  for (const line of lines) {
    const m = /^ {2}([A-Za-z][A-Za-z0-9_]*): (-?\d+),$/.exec(line)
    if (m) cells.set(m[1], Number(m[2]))
  }
  return cells
}

/**
 * 解析脚本里的判据码声明（`@ns` 命名空间 / `@codes` 本脚本判的号）。
 *
 * 声明**只认行注释形态**，写在文件头之后——登记表的唯一真源是这些行，
 * `docs/004-ref/GATE_CODES.md` 由它们生成（`facts/gate-codes.mjs`）。
 */
export function parseGateCodeDecls(text) {
  const ns = new Map()
  const codes = new Map()
  for (const m of text.matchAll(/^ *\/\/ @ns ([A-Z]{1,4}) (\S[^\n]*)$/gm)) {
    ns.set(m[1], m[2].trim())
  }
  for (const m of text.matchAll(/^ *\/\/ @codes ((?:[A-Z]{1,4}-\d{3}\s*)+)$/gm)) {
    for (const code of m[1].trim().split(/\s+/)) codes.set(code, code.slice(0, code.indexOf('-')))
  }
  return { ns, codes }
}

/** 正则字面量可以出现在这些字符之后（其余情况下 `/` 是除号，不是正则的开头） */
const REGEX_PRECEDERS = new Set([
  '', '(', ',', '=', ':', '[', '!', '&', '|', '?', '{', '}', ';', '+', '-', '*', '%', '~', '^', '<', '>',
])

/**
 * 扫一段源码，分出两类区间：
 * - `literals`：字符串字面量的**内容**区间（不含引号）；
 * - `blanks`：应当从「代码标识符」里剔除的区间——字符串内容 **与注释**。
 *
 * 为什么只要字符串：判据码被「用到」的唯一硬形态是它出现在输出语句里；注释里的
 * 「原 E-010 已退役」讲的是历史，要求它进登记表就把叙述当成了事实——而这类叙述
 * 在头注释里成段存在，误报会直接把守卫喂成豁免。
 *
 * 为什么注释要一起剔：注释是**文档的另一处**，它跟着代码一起腐烂。按标识符统计时
 * 留下注释，等于让一个早已删除的常量继续替它那一族「作证」，把合法的改名判成假红。
 */
export function codeSpansOf(src) {
  const literals = []
  const blanks = []
  let i = 0
  let last = ''
  while (i < src.length) {
    const c = src[i]
    const d = src[i + 1]
    if (c === '/' && d === '/') {
      const nl = src.indexOf('\n', i)
      const stop = nl < 0 ? src.length : nl
      blanks.push({ start: i + 2, end: stop })
      i = stop < src.length ? stop + 1 : src.length
      continue
    }
    if (c === '/' && d === '*') {
      const end = src.indexOf('*/', i + 2)
      blanks.push({ start: i + 2, end: end < 0 ? src.length : end })
      i = end < 0 ? src.length : end + 2
      continue
    }
    if (c === '/' && REGEX_PRECEDERS.has(last)) {
      i++
      let inClass = false
      while (i < src.length && src[i] !== '\n') {
        if (src[i] === '\\') {
          i += 2
          continue
        }
        if (src[i] === '[') inClass = true
        else if (src[i] === ']') inClass = false
        else if (src[i] === '/' && !inClass) break
        i++
      }
      last = '/'
      i++
      continue
    }
    if (c === '"' || c === "'" || c === '`') {
      let j = i + 1
      while (j < src.length) {
        if (src[j] === '\\') {
          j += 2
          continue
        }
        if (src[j] === c) break
        if (c !== '`' && src[j] === '\n') break
        j++
      }
      literals.push({ start: i + 1, end: j })
      blanks.push({ start: i + 1, end: j })
      last = c
      i = j + 1
      continue
    }
    if (!/\s/.test(c)) last = c
    i++
  }
  return { literals, blanks }
}

export function stringLiteralsOf(src) {
  return codeSpansOf(src).literals.map(({ start, end }) => src.slice(start, end))
}

/**
 * 取「代码自己写下的标识符」那一份文本：字符串内容与注释整段抹成空格。
 *
 * 为什么需要：`env!("CARGO_PKG_VERSION")` 里的 `CARGO_PKG_VERSION` 是**外部约定**的
 * 名字，不是本仓声明的常量；一条讲历史改名注释里的 `ROUTE_OLD` 也只是叙述。按标识符
 * 统计时留下它们，文档里合法的 `CARGO_TARGET_DIR` 就成了假红，而早已删除的常量还能
 * 替自己那一族「作证」。判据的「这一族存在」只能由代码本体回答。
 */
export function codeTextOf(src) {
  let out = ''
  let at = 0
  for (const { start, end } of codeSpansOf(src).blanks) {
    out += src.slice(at, start) + ' '
    at = end
  }
  return out + src.slice(at)
}

/**
 * `vitest run --coverage` 的**静态基准**墙钟预算（真正用哪个数由 `_machine.mjs::budget` 定）。
 *
 * 它**不是判据**——判定在覆盖率阈值（`tauri/vitest.config.ts` 的 `thresholds`）与
 * 用例通过数上，这里只防「跑飞了还不肯停」。
 *
 * 陷阱：冷全量（基座大改后 backend 全量重编）会把 coverage 推过静态预算，
 * 报出来的是「超时终止」，与真的卡死长得一样——所以预算必须随机器缩放。
 * `docs` 与 `backend` 同批之后，前端这三项与 cargo 争核是**常态**而不是意外，
 * 所以基准按「最坏负载」给，不按「空闲时多快」给：空闲时 40–60s，最坏 ~4m。
 *
 * ⚠️ 这个数**只在首跑**（本机没有实测历史）时直接生效；之后由
 * `budget()` 用上一轮实测覆盖。它是**基准**，不是「本机跑多快」的记录——
 * 把某次实测值写死在这里，就又会变成「只对那台机器正确」。
 */
export const VITEST_TIMEOUT_MS = 300_000

/** cargo 的进度噪音（刷屏且无信息量） */
export const cargoNoiseRe =
  /^(?:\s*$|.*\r$|\s*(Compiling|Checking|Downloading|Downloaded|Updating|Locking|Adding|Removing|Finished|Blocking|Waiting|Fresh|Documenting|Building)\b)/

/** 从输出里抓一个整数（第一个捕获组） */
export function grabInt(output, re) {
  const m = stripAnsi(output).match(re)
  return m ? Number(m[1]) : null
}

/** 把所有匹配的捕获组相加（`cargo test --workspace` 每个目标各打一行 `test result:`） */
export function sumInt(output, re) {
  const text = stripAnsi(output)
  let total = 0
  let found = false
  for (const m of text.matchAll(new RegExp(re.source, re.flags.includes('g') ? re.flags : `${re.flags}g`))) {
    total += Number(m[1])
    found = true
  }
  return found ? total : null
}

/**
 * `cargo test` + **通过数棘轮**（`symbio` 与 `cli` 共用）。
 *
 * 抽成共享函数而不是各写一遍：`symbio` 与 `cli` 是**两个独立 workspace**
 * （仓库根没有 `Cargo.toml`），而「跑测试并比对通过数基线」这件事必须对两者是
 * **同一套判据**——各写一份的结果是两边各自演化，最后变成两条不同的规范
 * （`check-commit-msg.mjs` 把「单条」与「一段范围」抽成同一个 `validate()` 就是为此）。
 *
 * 棘轮语义（与 `BASELINE` 头部的说明一致）：
 * - 低于基线 ⇒ **失败**（有测试被删，或有测试失败而退出码没反映出来）；
 * - 高于基线 ⇒ 通过，但打印提示要求同步基线（那是刻意要人看一眼的地方）；
 * - 解析不到通过数 ⇒ 通过但标注 —— 宁可漏报，也不因为**解析**失败把门禁变红。
 *
 * **CI 分支同样有棘轮**：靠 `ciBaseline`
 * 这一格，口径是 `--workspace` **各行求和**，与下面的本地口径不同。调用方在 CI 模式
 * **必须**传它——不传不降级、直接判红，理由见函数体内注释：省略参数不该让守卫消失。
 *
 * **口径（本地分包时）**：取输出里**第一个** `test result: ok. N passed`——那是 lib
 * 目标的**通过数**（`cargo test -p <pkg>` 先跑 lib，再跑集成测试 / doctest）。所以基线
 * 是「通过数」**不是「用例总数」**：`ignored` 的不计入（实测 `symbio` lib 总 1299、
 * `2 ignored` ⇒ 口径数 1297）。拿「总用例数」去定基线会差掉 ignored 那几个。
 *
 * 自定义任务而非声明式命令：同一次运行既判退出码又解析通过数，
 * 避免为了拿输出再跑一遍测试。
 */
export function cargoTestRatchet(ctx, { label, cwd, args, baseline, baselineName, ciBaseline, ciBaselineName }) {
  return {
    label,
    run: async () => {
      const r = await ctx.run({ label, cmd: 'cargo', args, cwd })
      if (!r.ok) {
        const note = r.timedOut ? '超时终止' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`
        return { ok: false, note, logFile: r.logFile }
      }
      if (ctx.ci) {
        // ⚠️ **没给 `ciBaseline` ⇒ 判红**，而不是退回「只信退出码」。
        //
        // 退回是这条守卫唯一会**静默**失效的方式：调用方少写一个参数，CI 就变回
        // 只抓「测试失败」、抓不住「测试消失」——后者不留任何痕迹（本地 1339 与基线
        // 比，CI 却只是求和打印一行数字）。宁可让漏写的人当场看见红，也不要一个
        // 只亮绿灯的检查项。真要「这个 crate 在 CI 没有可比基线」，显式传 `ciBaseline: 0`，
        // 那是**声明**，不是省略。
        if (ciBaseline === undefined) {
          return {
            ok: false,
            note: 'CI 模式缺 ciBaseline ⇒ 本轮次没有棘轮（调用方少写了一个参数）',
            logFile: r.logFile,
          }
        }
        // CI 跑一次 `--workspace`：输出是**所有测试目标各行求和**，与本地分包「各取
        // 首个 result 行」不是同一个口径（见 `BASELINE.ciRustTestsTotal`）。
        const total = sumInt(r.output, /test result: ok\. (\d+) passed/)
        if (total === null) return { ok: true, note: '未能解析通过数（--workspace 全量；只信退出码）' }
        const v = ratchetOrBump({
          actual: total,
          baseline: ciBaseline,
          name: ciBaselineName ?? baselineName,
          kind: 'CI 全量',
          ctx,
        })
        if (v.ok) {
          console.log(green(`      ${total} passed（CI 全量，基线 ${ciBaseline}）${v.note ? ` · ${v.note}` : ''}`))
        } else {
          console.log(red(`      ↳ ${v.note}`))
        }
        return { ...v, logFile: r.logFile }
      }
      const passed = grabInt(r.output, /test result: ok\. (\d+) passed/)
      if (passed === null) return { ok: true, note: '未能解析通过数' }
      const v = ratchetOrBump({ actual: passed, baseline, name: baselineName, ctx })
      if (v.ok) {
        console.log(green(`      ${passed} passed（基线 ${baseline}）${v.note ? ` · ${v.note}` : ''}`))
      } else {
        console.log(red(`      ↳ ${v.note}`))
      }
      return { ...v, logFile: r.logFile }
    },
  }
}

/** vitest --coverage 表格里「All files」行的行覆盖率（第 4 列，%） */
export function coverageLinesPct(output) {
  const m = stripAnsi(output).match(
    /^All files\s*\|\s*([\d.]+)\s*\|\s*([\d.]+)\s*\|\s*([\d.]+)\s*\|\s*([\d.]+)\s*\|/m,
  )
  return m ? Number(m[4]) : null
}

/** `tauri/vitest.config.ts` 里 `thresholds.lines` 的当前取值 */
export function coverageThreshold(frontendDir) {
  const cfg = path.join(frontendDir, 'vitest.config.ts')
  if (!fs.existsSync(cfg)) return null
  const m = fs.readFileSync(cfg, 'utf8').match(/thresholds:\s*\{[^}]*?\blines:\s*(\d+)/)
  return m ? Number(m[1]) : null
}

/** 沙箱「批量删除守卫」特征（vite 清 dist/ / vitest 清 coverage/ 会触发） */
export const sandboxDeleteRe = /safe-delete|SAFE_DELETE/

/**
 * 沙箱拦截导致的「未判定」：不记失败，记 skipped。
 * 一个**必然红**的门禁比没有门禁更糟 —— 人会学会忽略它。
 */
export function blockedBySandboxDelete(output) {
  return sandboxDeleteRe.test(output)
}

export function maybeSandboxDeleteHint(output) {
  if (!sandboxDeleteRe.test(output)) return
  console.log(
    '      ⚠ 输出含「批量删除被拦」字样：这是沙箱限制，不是构建/测试失败。'.padStart(0),
  )
  console.log('        确认方法：在沙箱外跑同一条命令，或先手工清掉 dist/ 与 coverage/。')
}

// CLI release 二进制的路径与新鲜度判定**不在这里**——统一在
// `scripts/cli-binary.mjs`（门禁与 e2e 共用的唯一真相）。曾经这里只有
// 「文件在不在」两个函数，而「在」不等于「对应当前源码」，于是过期产物被一直用下去。

// ==================== 「自动执行的工作」（不是检查项） ====================

/**
 * `git status --porcelain` → 脏路径集合（含已暂存与未暂存）。
 *
 * 只取路径，不区分状态：本模块关心的是「这个路径的内容在修复前后有没有变」。
 *
 * 用 `-z`（NUL 分隔）而非默认的行分隔：默认输出会把非 ASCII 路径**转义**成
 * `"\346\226\207.md"`（`core.quotepath=true` 是默认值），那样的串既读不了文件
 * （`contentHashes` 拿到 null），也不能直接喂给 `git add`（暂存失败）。
 * `-z` 下路径原样给出、不加引号，这两处一并消失。
 */
function dirtyPaths(repoRoot) {
  const r = spawnSync('git', ['status', '--porcelain', '-z'], { cwd: repoRoot, encoding: 'utf8' })
  const out = new Set()
  if (r.status !== 0) return out
  const tokens = r.stdout.split('\0')
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i]
    if (!t) continue
    // 形如 `XY path`；重命名/复制时**紧随其后还有一个「旧路径」token**，跳过它。
    const xy = t.slice(0, 2)
    out.add(t.slice(3))
    if (xy[0] === 'R' || xy[0] === 'C' || xy[1] === 'R' || xy[1] === 'C') i++
  }
  return out
}

/** 工作区内容的哈希（文件不存在 → null）。用于判定「修复是否真的改写了它」。 */
function contentHashes(repoRoot, paths) {
  const m = new Map()
  for (const p of paths) {
    try {
      m.set(p, createHash('sha256').update(fs.readFileSync(path.join(repoRoot, p))).digest('hex'))
    } catch {
      m.set(p, null)
    }
  }
  return m
}

/**
 * 把一件**确定性的机械工作**交给门禁自己做完，而不是判它「有没有做过」。
 *
 * ## 为什么不判「有没有做过」
 *
 * 格式化与事实文件生成是**函数**，不是判断：`fmt(code) → code'`、`gen(code) → facts`
 * 对同一份输入永远给同一个输出。把它们写成 `--check` 等于让门禁因为
 * **人忘了按一次按钮**而红——它报的不是代码有问题，是流程有问题。而修复动作
 * 完全确定、零风险，没有任何理由等人来按。
 *
 * ## 语义
 *
 * - 命令**跑成功** ⇒ 通过（`ok: true`）。产物被改写不算失败，那是它该做的事。
 * - 命令**本身报错** ⇒ 不通过（真失败：工具坏了 / 输入不可解析）。
 * - 本地：被改写的路径**当场暂存**，使修复与「本次提交」是同一份内容。
 * - CI：CI 不能提交，所以「跑完仍有差异」只能报红——那是唯一能保住不变量的信号。
 *
 * ⚠️ **「CI 报红」靠 `ctx.ci`，而 `ctx.ci` 来自 `--ci`。** 调用本原语的任务
 * （`10-backend` 的两处 fmt、`06-facts-write` 的生成链）必须在 CI 侧被以 `--ci` 调用，
 * 否则会退化成「自动修复 + 暂存」而**静默放过漂移**——一个只亮绿灯的检查项。
 * 回归测试里有一条专门断言 `.github/workflows/ci.yml` 的对应步骤传了 `--ci`。
 *
 * ## 怎么认出「被修复改写的路径」（⚠️ 这里错过一次）
 *
 * 第一版按「修复前干净、修复后变脏」判定，**方向反了**：日常流程是
 * 「改文件 → `git add` → 提交」，所以修复前就已脏（甚至已暂存）才是**常态**，
 * 而那样会漏掉它们 ⇒ 提交里留下**未格式化**的那一版。
 *
 * 正确判据是**内容哈希**：修复前给所有脏路径记哈希，修复后重算，
 * 哈希变了就是被改写过（无论它此前是干净、已暂存、还是已有未提交改动）。
 * 只比较脏路径即可——干净路径修复后若变脏，它自然进入「修复后」这一侧。
 *
 * 这样「修复前就脏、修复没碰」的路径**不会被暂存**：门禁没有立场替人决定
 * 「那些改动该不该进本次提交」（`commit.mjs` 的模型是「先 git add 你要的，
 * 再提交索引」）。
 */
export async function autoWork(ctx, { label, cmd, args = [], cwd }) {
  const before = dirtyPaths(ctx.repoRoot)
  const beforeHash = contentHashes(ctx.repoRoot, before)
  // 判定模式要在本地还原改写（门禁只判不改），所以总是留一份「跑之前已脏」的快照——
  // 干净的路径还原到索引内容即可，不必进快照。
  const snapshot = new Map()
  for (const p of before) {
    try {
      snapshot.set(p, fs.readFileSync(path.join(ctx.repoRoot, p)))
    } catch {
      snapshot.set(p, null)
    }
  }

  const r = await ctx.run({ label, cmd, args, cwd })
  if (!r.ok) {
    return {
      ok: false,
      note: r.timedOut ? '超时终止' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`,
    }
  }

  const after = dirtyPaths(ctx.repoRoot)
  const afterHash = contentHashes(ctx.repoRoot, after)
  const touched = [...after].filter(
    (p) => !before.has(p) || beforeHash.get(p) !== afterHash.get(p),
  )

  if (touched.length === 0) return { ok: true }

  const shown = touched.slice(0, 5).join('、') + (touched.length > 5 ? ` 等 ${touched.length} 个` : '')
  if (!ctx.write || ctx.ci) {
    // 判定模式：把改写还原，红给「跑 gen」。绿 = 树没被改，这是本轮要分开的两件事。
    const back = restoreTouched(ctx, snapshot, touched)
    return {
      ok: false,
      note:
        `已执行但改写了 ${touched.length} 个文件（${shown}）—— 判定模式不代改` +
        (ctx.ci ? '，CI 无法提交' : '') +
        (back ? `，且${back}` : '，已还原') +
        '；请在本地运行 `node scripts/gen.mjs` 修复并暂存',
    }
  }

  const add = spawnSync('git', ['add', '--', ...touched], { cwd: ctx.repoRoot, encoding: 'utf8' })
  if (add.status !== 0) {
    return { ok: false, note: `已修复 ${touched.length} 个文件但暂存失败：${(add.stderr || '').trim()}` }
  }
  console.log(yellow(`      ⚠ 门禁已自动修复并暂存 ${touched.length} 个文件：${shown}`))
  return { ok: true, note: `自动修复 ${touched.length} 个文件（已暂存）` }
}

/**
 * 把一次机械工作改写过的路径还原到「跑之前」的样子。
 *
 * 跑之前已脏的路径按**内容快照**写回（`git restore` 会恢复索引、抹掉人未暂存的改动）；
 * 跑之前干净 / 不存在的交给索引：已跟踪的 `git restore --worktree`，新建的直接删。
 */
function restoreTouched(ctx, snapshot, touched) {
  const toIndex = []
  for (const p of touched) {
    if (snapshot.has(p)) {
      const bytes = snapshot.get(p)
      if (bytes === null) fs.rmSync(path.join(ctx.repoRoot, p), { force: true })
      else fs.writeFileSync(path.join(ctx.repoRoot, p), bytes)
    } else toIndex.push(p)
  }
  if (!toIndex.length) return null
  const ls = spawnSync('git', ['ls-files', '--', ...toIndex], { cwd: ctx.repoRoot, encoding: 'utf8' })
  const tracked = new Set((ls.stdout || '').split('\n').filter(Boolean))
  for (const p of toIndex) if (!tracked.has(p)) fs.rmSync(path.join(ctx.repoRoot, p), { force: true })
  const keep = toIndex.filter((p) => tracked.has(p))
  if (!keep.length) return null
  const r = spawnSync('git', ['restore', '--worktree', '--', ...keep], {
    cwd: ctx.repoRoot,
    encoding: 'utf8',
  })
  return r.status === 0 ? null : `还原失败：${(r.stderr || '').trim()}`
}

/**
 * `autoWork` 的**整链版**：把一串生成器当一个原子工作跑，只在**末态**与跑之前比差异。
 *
 * ## 为什么 `autoWork` 逐格跑不了整条链
 *
 * `autoWork` 每跑完一步就判一次「这一步有没有改写文件」。单步生成器这样是对的
 * （它的产物就是末态）。但「事实文件 → 登记汇总页 → 引用地址 → verify 期望数据」是一条
 * **有中间态的链**：`registry-audit --write` 把汇总页写成 `[[编号]]` 记号，
 * 下一格 `gen-doc-refs` 才展开成地址——逐格判差异时，第一格的产物**本身就是差异**，
 * 于是 `--ci` 侧无论树新树旧都报红。CI 要的是末态判据，不是生成器的自述。
 *
 * ## 语义（与 `autoWork` 同一条约定，只是判据挪到链尾）
 *
 * - 任一步**命令本身报错** ⇒ 不通过（真失败：工具坏了 / 输入不可解析）；CI 侧顺带还原工作区。
 * - 本地：末态有差异 ⇒ 差异路径**当场暂存**，修复与本次提交同一份内容；无差异即绿。
 * - CI：末态有差异 ⇒ **还原工作区**后报红。还原是这条相对 `autoWork` 多出来的一件事：
 *   链跑完树是脏的，而 CI 的红只该说明「生成物没跟上代码」，不该把树留在半成品形态
 *   给后续步骤（`git diff --exit-code`、产物上传）当输入。
 *   还原口径：跑之前已脏的路径按**内容快照**写回（那是人自己的改动，不能用 `git restore`
 *   ——它恢复到索引，会把未暂存改动抹掉）；跑之前干净的路径恢复到索引内容即正确；
 *   跑之前不存在的（生成器新建）删掉自己写出来的东西。
 */
export async function autoChain(ctx, { label, steps }) {
  const before = dirtyPaths(ctx.repoRoot)
  const beforeHash = contentHashes(ctx.repoRoot, before)
  // 快照**总是**留：判定模式要在本地还原（门禁只判不改），而 `--write` 模式在链失败时
  // 也可能需要还原。只拍「跑之前已脏」的路径——干净的路径还原到索引内容即可。
  const snapshot = new Map()
  for (const p of before) {
    try {
      snapshot.set(p, fs.readFileSync(path.join(ctx.repoRoot, p)))
    } catch {
      snapshot.set(p, null)
    }
  }
  const touchedNow = () => {
    const after = dirtyPaths(ctx.repoRoot)
    const afterHash = contentHashes(ctx.repoRoot, after)
    return [...after].filter((p) => !before.has(p) || beforeHash.get(p) !== afterHash.get(p))
  }
  const restore = () => {
    // 写模式（本地 `gen`）不还原：那侧的约定是「生成并暂存」。链某一步真失败时把现状
    // 留给人看，比替他决定该留哪一版更安全。
    if (ctx.write && !ctx.ci) return null
    return restoreTouched(ctx, snapshot, touchedNow())
  }

  for (const s of steps) {
    const r = await ctx.run({
      label: s.label,
      cmd: s.cmd,
      args: s.args ?? [],
      cwd: s.cwd ?? ctx.repoRoot,
    })
    if (!r.ok) {
      const why = r.timedOut ? '超时终止' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`
      const back = restore()
      return {
        ok: false,
        note: `${s.label} 失败（${why}）${back ? `，且${back}` : ctx.ci ? '，工作区已还原' : ''}`,
      }
    }
  }

  const touched = touchedNow()
  if (touched.length === 0) return { ok: true }
  const shown = touched.slice(0, 5).join('、') + (touched.length > 5 ? ` 等 ${touched.length} 个` : '')
  if (!ctx.write || ctx.ci) {
    // 判定模式：门禁**不代改**。先还原（链跑完树是脏的，不能留给后续步骤），再报红。
    const back = restore()
    return {
      ok: false,
      note:
        `${label} 跑完末态有 ${touched.length} 处差异（${shown}）——生成物没跟上代码` +
        (ctx.ci ? '，CI 无法提交' : '') +
        (back ? `，且${back}` : '，工作区已还原') +
        '；请在本地运行 `node scripts/gen.mjs` 生成并暂存',
    }
  }
  const add = spawnSync('git', ['add', '--', ...touched], { cwd: ctx.repoRoot, encoding: 'utf8' })
  if (add.status !== 0) {
    return { ok: false, note: `已生成 ${touched.length} 个文件但暂存失败：${(add.stderr || '').trim()}` }
  }
  console.log(yellow(`      ⚠ 门禁已自动生成并暂存 ${touched.length} 个文件：${shown}`))
  return { ok: true, note: `自动生成 ${touched.length} 个文件（已暂存）` }
}

// ── Rust 源码 → 某个 crate 前缀下的符号集合 ─────────────────────────────
//
// 两处判据在量**同一件事**的两面：E-009（`plugin-entry-audit`：用了的必须在账上）与
// SR-001（`session-retire-audit`：账上的必须真被用）。它们必须用**同一份**取法——
// 一边按「名字出现」、一边按「路径命中」的话，账本可以同时多出虚账、又留下冻结的洞，
// 而两边都绿（D-454 就是这么发现的：13 条 kernel 词汇挂在 sdk 账上）。
//
// 所以取法只在这里写一次。**改这里就是改两处判据**。

/**
 * `A::B::C` 的末段；`as` 别名只认左侧。
 */
function crateLeafOf(pathText) {
  const beforeAs = pathText.split(/\bas\b/)[0]
  const ids = [...beforeAs.matchAll(/[A-Za-z_][A-Za-z0-9_]*/g)]
    .map((m) => m[0])
    .filter((w) => w !== 'self' && w !== 'super' && w !== 'crate')
  return ids.length ? ids[ids.length - 1] : null
}

/** 顶层逗号切分（花括号内的逗号不切）。 */
function crateSplitTop(text) {
  const parts = []
  let depth = 0
  let cur = ''
  for (const ch of text) {
    if (ch === '{') depth++
    else if (ch === '}') depth--
    if (ch === ',' && depth === 0) {
      parts.push(cur)
      cur = ''
      continue
    }
    cur += ch
  }
  if (cur.trim()) parts.push(cur)
  return parts
}

/** 展开一个 `use` 树（前缀之后的部分）里的叶子符号。 */
function crateUseLeaves(tree, out) {
  const open = tree.indexOf('{')
  if (open >= 0) {
    let depth = 0
    let end = -1
    for (let i = open; i < tree.length; i++) {
      if (tree[i] === '{') depth++
      else if (tree[i] === '}') {
        depth--
        if (depth === 0) {
          end = i
          break
        }
      }
    }
    if (end < 0) return
    for (const part of crateSplitTop(tree.slice(open + 1, end))) crateUseLeaves(part, out)
    return
  }
  if (tree.includes('*')) {
    out.add('*')
    return
  }
  const leaf = crateLeafOf(tree)
  if (leaf) out.add(leaf)
}

/**
 * 一段（**已剥注释的**）Rust 文本里经 `crate::` 取用的符号集合。
 *
 * 两种形态分开取：
 * - `use crate::…::{a, b::C};` → 花括号里的**叶子**（含跨行 use 树）；
 * - 表达式 / 类型位置的 `crate::a::b::C` → 路径末段。
 * 后者必须先把 `use` 语句抹掉，否则 `use` 树的花括号会被误当成结构体字面量的字段。
 *
 * ⚠️ 只认路径，不认名字：源码里出现 `ChatNode` 不代表它来自这个 crate——它可能来自
 * `symbio_kernel`。按名字算就是把别人的词汇记在这个 crate 的账上（D-454）。
 */
export function symbolsInCrate(text, crate) {
  const prefix = `${crate}::`
  const out = new Set()
  for (const m of text.matchAll(/\buse\s+([^;]+);/g)) {
    const idx = m[1].indexOf(prefix)
    if (idx < 0) continue
    crateUseLeaves(m[1].slice(idx + prefix.length), out)
  }
  const stripped = text.replace(/\buse\s+[^;]+;/g, (s) => ' '.repeat(s.length))
  for (const m of stripped.matchAll(new RegExp('\\b' + crate + '::', 'g'))) {
    const tail = stripped.slice(m.index + m[0].length, m.index + m[0].length + 240)
    const path = /^\s*([A-Za-z_][A-Za-z0-9_]*(?:::\s*[A-Za-z_][A-Za-z0-9_]*)*)/.exec(tail)
    if (!path) continue
    const leaf = crateLeafOf(methodOwnerPath(path[1], tail.slice(path[0].length)))
    if (leaf) out.add(leaf)
  }
  return out
}

/**
 * `symbio_sdk::clock::ClockNow::now()` 里**符号是类型**、不是方法名。
 *
 * 只在「末段紧跟 `(`」且「末段小写、前一段大写」时把末段摘掉——这是 Rust 的命名约定，
 * 不是猜：`…::round::llm_short_id()`（模块里的自由函数，前一段小写）照旧算 `llm_short_id`，
 * `…::ClockNow::now()`（关联方法）算 `ClockNow`。不摘的话，账上会凭空多出一条 `now`，
 * 而它既不在这条 crate 的账上、也不是被依赖的东西——判据会为一条方法名报红。
 */
function methodOwnerPath(pathText, after) {
  if (!/^\s*\(/.test(after)) return pathText
  const segs = pathText.split('::').map((s) => s.trim())
  if (segs.length < 2) return pathText
  const last = segs[segs.length - 1]
  const prev = segs[segs.length - 2]
  if (!/^[a-z_]/.test(last) || !/^[A-Z]/.test(prev)) return pathText
  return segs.slice(0, -1).join('::')
}

/** `symbolsInCrate(text, 'symbio_sdk')` —— E-009 / SR-001 共用的那一条边。 */
export function sdkSymbolsIn(text) {
  return symbolsInCrate(text, 'symbio_sdk')
}

// ── `@gate` 声明：门禁的「目录即清单」 ─────────────────────────────────────
//
// 每个 `scripts/**` 下的脚本在自己的头注释里写一行：
//
//     // @gate guard     本体静态跑得出（带 `--strict`）+ 同名回归测试
//     // @gate report    只跑本体（退出码恒 0，结论要人工复核）
//     // @gate test      只跑回归测试（本体要运行时输入 / 由别的阶段执行 / 根本没有本体）
//     // @gate lib       只被引用，门禁不跑它
//     // @gate off       不进门禁（git hook / 人工 / 门禁入口自己）
//
// `30-docs` 扫这条声明分组执行，于是**加一条守卫不用改任何名单**——写脚本、写同名
// `.test.mjs`、写下这一行，下一轮门禁就会跑它。
//
// 声明是自述，所以要有对账（`gate-wiring-audit`）：GW-001 每份脚本都得声明、
// GW-002 守卫必须真有回归测试、GW-003 / GW-004 声明的东西**是否真的被跑到**。
// 光有声明没有执行方，就是「看起来接了线」——那正是这份机制唯一的坏法。
export const GATE_BUCKETS = ['guard', 'report', 'test', 'lib', 'off']

const GATE_DECL_RE = /^\/\/\s*@gate\s+([A-Za-z]+)\s*$/
const GATE_SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai', '.symbio'])

/**
 * 阶段文件（`gate.d/` 下非 `_` 前缀）由 `gate.mjs` 按文件名接入，不写 `@gate` 声明：
 * 它们的接线是 `export default { id }`，顺序由 GW-005 判。
 */
export function isStageFile(rel) {
  return rel.startsWith('gate.d/') && !rel.slice('gate.d/'.length).startsWith('_')
}

/** 递归列出 `scripts/` 下的 `.mjs`，返回相对 `scripts/` 的正斜杠路径 */
export function scriptFilesOf(root) {
  const base = path.join(root, 'scripts')
  if (!fs.existsSync(base)) return null
  const out = []
  const walk = (dir, prefix) => {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      if (ent.isDirectory()) {
        if (!GATE_SKIP_DIRS.has(ent.name)) walk(path.join(dir, ent.name), `${prefix}${ent.name}/`)
        continue
      }
      if (ent.name.endsWith('.mjs')) out.push(`${prefix}${ent.name}`)
    }
  }
  walk(base, '')
  return out.sort()
}

/**
 * 读一份源码的 `@gate` 声明。
 * 返回 `{ bucket }` / `{ error: 'missing' | 'unknown' | 'duplicate' }`。
 * 只看前 80 行：声明写在头注释里，掉到正文深处的「声明」读的人看不见，也不该算。
 */
export function gateDeclOf(text) {
  const hits = []
  for (const line of text.split(/\r?\n/).slice(0, 80)) {
    const m = GATE_DECL_RE.exec(line.trim())
    if (m) hits.push(m[1])
  }
  if (hits.length === 0) return { error: 'missing' }
  if (hits.length > 1) return { error: 'duplicate', detail: hits.join(' / ') }
  if (!GATE_BUCKETS.includes(hits[0])) return { error: 'unknown', detail: hits[0] }
  return { bucket: hits[0] }
}

/**
 * 全量扫声明：`{ entries: [{ name, rel, bucket }], problems: [...] }`，两者都按名排序。
 * `entries` 只含**合法声明**的脚本；`problems` 逐条带回原因，由调用方决定红在哪条判据上。
 */
export function gateDeclarations(root) {
  const files = scriptFilesOf(root)
  if (files === null) return null
  const entries = []
  const problems = []
  for (const rel of files) {
    if (rel.endsWith('.test.mjs') || isStageFile(rel)) continue
    const name = rel.replace(/\.mjs$/, '')
    let text
    try {
      text = fs.readFileSync(path.join(root, 'scripts', rel), 'utf8')
    } catch (e) {
      problems.push({ name, rel, error: 'unreadable', detail: (e && e.message) || String(e) })
      continue
    }
    const d = gateDeclOf(text)
    if (d.error) problems.push({ name, rel, ...d })
    else entries.push({ name, rel, bucket: d.bucket })
  }
  const key = (x) => x.name
  return { entries: entries.sort((a, b) => (key(a) < key(b) ? -1 : 1)), problems: problems.sort((a, b) => (key(a) < key(b) ? -1 : 1)) }
}

// ── `@gens` 声明：生成链的「目录即清单」 ───────────────────────────────────
//
// `06-facts-write`（生成）与 `60-facts`（复核）原先各自抄一份生成器清单：哪几份、
// 什么顺序、哪一份只生成不复核。加一份生成器要改两个阶段文件，而**漏改的失效形态是
// 静默**——新生成器永远不会被跑，两个阶段都绿。
//
// 现在由生成器自己声明它在链上的位置与它支持的档位：
//
//     // @gens 10 write check     链上第 10 位；支持 `--write`（生成）与 `--check`（复核）
//     // @gens 20 write           ＜只生成不复核：例如复核由别的阶段负责
//
// 顺序由**声明里的序数**给，不由文件名给：文件名是给人看的，序数是判据的输入
// （`gate-wiring-audit` 的 GW-008 判它唯一）。两个阶段各自遍历一遍声明、按序数排序：
// 声明 `write` 的进生成链，声明 `check` 的进复核阶段——**没有一个阶段知道生成器有哪几份**。
export const GENS_MODES = ['write', 'check']

const GENS_DECL_RE = /^\/\/\s*@gens\s+(\d+)\s+([a-z ]+)$/

/**
 * 读一份源码的 `@gens` 声明（可选）。
 * 返回 `{ order, modes }` / `{ error: 'bad-shape' | 'bad-mode' | 'duplicate' }` / `null`（没声明）。
 * 与 `@gate` 同一个读法：只看前 80 行。
 */
export function gensDeclOf(text) {
  const hits = []
  for (const line of text.split(/\r?\n/).slice(0, 80)) {
    if (line.trim().startsWith('// @gens')) hits.push(line.trim())
  }
  if (hits.length === 0) return null
  if (hits.length > 1) return { error: 'duplicate', detail: hits.join(' / ') }
  const m = GENS_DECL_RE.exec(hits[0])
  if (!m) return { error: 'bad-shape', detail: hits[0] }
  const order = Number(m[1])
  const modes = m[2].trim().split(/\s+/)
  const bad = modes.filter((x) => !GENS_MODES.includes(x))
  if (bad.length) return { error: 'bad-mode', detail: bad.join(' ') }
  return { order, modes: [...new Set(modes)] }
}

/**
 * 全量扫 `@gens`：`{ entries: [{ name, rel, order, modes }], problems: [...] }`，按序数排序。
 * 只有**声明了的**脚本进 `entries`；一份都不声明返回空表（调用方自己判这算不算坏）。
 */
export function gensDeclarations(root) {
  const files = scriptFilesOf(root)
  if (files === null) return null
  const entries = []
  const problems = []
  const seen = new Map()
  for (const rel of files) {
    if (rel.endsWith('.test.mjs') || isStageFile(rel)) continue
    let text
    try {
      text = fs.readFileSync(path.join(root, 'scripts', rel), 'utf8')
    } catch (e) {
      problems.push({ name: rel, rel, error: 'unreadable', detail: (e && e.message) || String(e) })
      continue
    }
    const d = gensDeclOf(text)
    if (d === null) continue
    const name = rel.replace(/\.mjs$/, '')
    if (d.error) {
      problems.push({ name, rel, ...d })
      continue
    }
    const prev = seen.get(d.order)
    if (prev) {
      problems.push({ name, rel, error: 'duplicate-order', detail: `${d.order} 已被 ${prev} 占用` })
      continue
    }
    seen.set(d.order, name)
    entries.push({ name, rel, order: d.order, modes: d.modes })
  }
  entries.sort((a, b) => a.order - b.order)
  return { entries, problems }
}

