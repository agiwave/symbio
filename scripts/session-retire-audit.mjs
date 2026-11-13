#!/usr/bin/env node
// 判据码命名空间（`docs/004-ref/GATE_CODES.md` 由这些行渲染，判据见 gate-codes-audit.mjs）
// @ns SRA 控制面 session 退役线的进度尺（豁免账 / 单槽登记）
// @codes SRA-001 SRA-002 SRA-003 SRA-004 SRA-005
/**
 * session-retire-audit — 控制面 `session` 退役线的进度尺（D-451）
 *
 * ## 它判什么
 *
 * | 码 | 判据 | 范围 |
 * |---|---|---|
 * | SRA-001 | 豁免账 `symbols` 里**每一条**都必须真被 `symbio-plugin-session` 用到（去注释后至少命中一次） | `scripts/exemptions/plugin-session-sdk.json` × `crates/symbio-plugin-session/src/**` |
 * | SRA-002 | `symbols` 条数 ≤ 基线（**只许降**；要涨必须回 sdk / 内核侧解决，不是往账上加） | 同上 |
 * | SRA-003 | 豁免三字段自洽：`expires` 是**将来的**合法复核日、`retire` 写明**可兑现的处置**（删除，或「保留 / 冻结 + 依据决策号」）、`knife` 形如登记号 | 同上 |
 * | SRA-004 | 事实源开面**恰好一家**：生产 `.rs` 里 `submit_fact_source_opener!` 的登记点必须是 1 处 | `crates/` 下全部 `.rs`（去测试） |
 * | SRA-005 | 地址根**至多一家**：生产 `.rs` 里 `AddrRootDecl(…)` 的声明点 `≥ 2` 即红；`0` 绿 + 提示（合法降级） | 同上 |
 *
 * 违规逐条点名**符号 / 字段**。退出码：0 = 通过；1 = 有违规或判据读不出输入。
 *
 * ## 为什么要有这把尺
 *
 * 「session 退役」在仓里被七处决策引用（`D-349` / `D-358` / `D-413` / `D-416` / `D-427` /
 * `D-428` / `D-438` / `D-443` 都写着「随 session 退役同批删除」），但它自己的范围与出口
 * **从来没有登记过**：它挂的刀（一个任务号）早已随「收口即删」消失，只剩豁免 JSON 里的
 * `expires: 2027-06-30` 一条日期。没有出口的「将来会删」等于不会删——2027-06-30 到点时
 * 能做的只有续期。
 *
 * 退役的终态是**一条依赖边消失**：`symbio-plugin-session → symbio-sdk` 不再需要豁免，
 * 于是那一整份 `symbols` 与它的 `reason` 一起被删。这条边今天由 E-009 冻结成一份
 * **符号级白名单**（新增用法即红、删除不判），所以 `symbols` 的条数天然就是一把只降不升
 * 的尺。本脚本把尺**校准**：先在账本上去余量（SRA-001），再钉住条数（SRA-002），
 * 最后保证这张账本身不是一张永远不用兑现的欠条（SRA-003）。
 *
 * ## 为什么需要 SRA-001（实测出来的洞）
 *
 * E-009 只判「用了的必须在账上」，符号在用法消失之后会**静默留在账本里**——
 * 账本比真实依赖面宽出一截。余量本身无害，**余量冒充进度**有害：它让「退役走了
 * 多远」读出一个偏大的分母，也让下一个人以为那些能力还挂在 session 身上。
 *
 * ## 已知边界（写在绿灯里，别把它当证明）
 *
 * - **SRA-001 是文本判据**：`use … as 别名`、`macro_rules!` 拼出的名字、字符串里的同名文本
 *   都不在语义内（本仓当前两类皆 0）。它偏保守：**误报（说没人用）比漏报便宜**，因为
 *   误报只会让人多看一眼账本，漏报会让余量继续积累。
 * - **SRA-001 只扫 `src/`**：插件 crate 的测试与实现同在 `src/`（`*.test.rs`），故「测试里
 *   用过」也算「在用」——E-009 同样把测试判在内，两边口径一致。
 * - **SRA-002 的基线是数据、不是判据**：涨了要改这里，而改这里要在提交说明里写清为什么
 *   这条边必须变宽（与 `arch-shape-audit` 的 `core-shape-allow` 同一条纪律）。
 * - **SRA-003 不判 `knife` 是否存活**：任务按「收口即删」消失是常态，把存活写进判据会让
 *   两阶之间永远红。它在报告里把 `knife` 打出来，供人确认「这条账还有没有主人」。
 * - **本脚本不判「谁该搬去哪」**：那是 D-451 的阶次与各阶出口，不是一把尺能回答的。
 *
 * ## `--breakdown`：账本按「住处」分解（**读数，不是判据**）
 *
 * 普通模式只答「还欠多少条」。这一条边的问题不是条数，而是**这些条数住在哪**：如果它们
 * 全长在边二（装配）/ 边三（存储·转写）要搬的代码里，那么「边一先归零」就不是一条能走的
 * 路（`D-456`）。`--breakdown` 把账本里的符号按 crate 目录结构分成四桶（装配面 / 存储·转写 /
 * 两处都有 / 两处都没有）并逐条列出「两处都没有」的那些——它们才是三条边都没点到的地。
 *
 * 它**不改退出码、不进 GUARDS**：分组口径随目录结构漂，把它做成门禁只会得到一条天天红
 * 的尺。要复核 `D-456` 里的数字，跑这一条命令。
 *
 * ## `--coverage`：**代码**按「谁的活」分解（**读数，不是判据**）
 *
 * `--breakdown` 量的是账本（符号住在哪），`--coverage` 量的是 crate 本身（**行数**住在哪）。
 * 两者回答的问题不同：前者答「这条依赖边还剩几条」，后者答「退役要处理的东西有多少被三条边
 * 点到」。`D-457` 用后者发现：三条边点到的代码不到全 crate 的四分之一，而最大的那块
 * （VDFS 地址面 `<根>/session/<id>/…`）**一条边都没有**——它不是空壳，CLI 与子智能体都在写它。
 *
 * 分桶按**文件的活**，不按目录名猜；判不出的文件落「没有归属」并逐条列出，不静默归并。
 * 已知偏差写在按钮旁边：装配的折叠函数（`exposed_tools` / `dedup_declared_twins`）住在
 * `plugin/turn.rs` 里，按文件口径整份记在「开轮序列」一侧——偏差方向是**边二读数偏小**，
 * 也就是不会把这条边说大。
 *
 * 用法：node scripts/session-retire-audit.mjs [--root=<仓库根>] [--verbose] [--breakdown] [--coverage]
 * 退出码：0 = 通过；1 = 有违规或读不出输入。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow, dim } from './color.mjs'
import { sdkSymbolsIn } from './gate.d/_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..')
const VERBOSE = process.argv.includes('--verbose')
const BREAKDOWN = process.argv.includes('--breakdown')
const COVERAGE = process.argv.includes('--coverage')

const EXEMPTION_REL = 'scripts/exemptions/plugin-session-sdk.json'
const SESSION_SRC_REL = 'crates/symbio-plugin-session/src'

/**
 * 符号条数基线（**只许降**）。
 *
 * 下调只有两种合法来源：① 符号在 session 源码里零命中（账本余量或用法退场）；
 * ② 取字改走 `symbio_plugin_api::` 门面后不再直取 `symbio_sdk::` 本体。每次下调
 * 都对应一条 D-NNN 裁决（D-452…D-455），理由与符号清单归裁决与 `git log`；这里只留当前值。
 *
 * 边界：`D-443` 第 3 条的「控制面专属读口」（`FactSource` 登记面 / `PRINCIPAL_MAIN` /
 * `session_chat` / `rebuild` / `SKILL_TAG` / `commitment_records` /
 * `VdfsChangeSubscriptions` / `DriveEntry` / 词汇结构体）**不上门面**，不因换径退场。
 */
const SYMBOL_BASELINE = 50

const errors = []
const notes = []
const report = (rule, msg) => errors.push({ rule, msg })

// ==================== 读输入 ====================

function readJson(rel) {
  const abs = path.join(ROOT, rel)
  if (!fs.existsSync(abs)) {
    report('SRA-003', `${rel} 不存在——退役进度尺没有输入，宁红不静默`)
    return null
  }
  try {
    return JSON.parse(fs.readFileSync(abs, 'utf8'))
  } catch (e) {
    report('SRA-003', `${rel} 解析失败：${e.message} —— 读不出即判据失效`)
    return null
  }
}

/** 递归收集 `.rs`（跳过 `target` 之类的非源码目录不需要：路径已经钉死在 src）。 */
function collectRs(dir) {
  if (!fs.existsSync(dir)) return null
  const out = []
  const walk = (d) => {
    for (const entry of fs.readdirSync(d, { withFileTypes: true })) {
      const abs = path.join(d, entry.name)
      if (entry.isDirectory()) walk(abs)
      else if (entry.name.endsWith('.rs')) out.push(abs)
    }
  }
  walk(dir)
  return out
}

/**
 * 去注释（`//` 行注释含 `///` / `//!`，以及 C 风格块注释）。
 *
 * 理由与 `port-type-coupling-audit` 同：doc comment 里写「某某已移出」是叙述，不是用法。
 * 不追求字符串里注释符的完美处理（本仓当前 0 处影响本判据）。
 */
function stripComments(text) {
  return text
    .replace(/\/\*[\s\S]*?\*\//g, ' ')
    .split('\n')
    .map((line) => line.replace(/\/\/.*$/, ''))
    .join('\n')
}

// ==================== SRA-001 / SRA-002 ====================

const exemption = readJson(EXEMPTION_REL)
const srcDir = path.join(ROOT, SESSION_SRC_REL)
const files = collectRs(srcDir)
/** 每个文件里出现的 sdk 符号（与 SRA-001 / E-009 同一份取法）；`--breakdown` 用它分组。 */
const perFile = []
if (files === null) {
  report('SRA-001', `${SESSION_SRC_REL} 不存在——没有源码可量，判据没有输入`)
}

if (exemption && files) {
  if (!Array.isArray(exemption.symbols)) {
    report('SRA-002', `${EXEMPTION_REL} 的 \`symbols\` 不是数组——账本形状不对，条数无从判`)
  } else {
    // **按路径算，不按名字算**（D-454）。这段原先按「名字在源码里出现」判，于是 13 条
    // `symbio_kernel` 的词汇（`ChatNode` / `NodeRole` / `NodeStatus` …）被当成 sdk 依赖
    // 冻结在账上：账本凭空多 13 条，同时给这 13 个名字开了「随便从 sdk 取」的口子
    // （E-009 只判「用了的必须在账上」，名字在账上就放行）。取法与 E-009 **同一份**
    // （`scripts/gate.d/_shared.mjs`），两处判据从此量同一件事。
    const used = new Set()
    for (const f of files) {
      const syms = sdkSymbolsIn(stripComments(fs.readFileSync(f, 'utf8')))
      for (const sym of syms) used.add(sym)
      perFile.push({ rel: path.relative(srcDir, f).split(path.sep).join('/'), syms })
    }
    const stale = exemption.symbols.filter((sym) => !used.has(sym))
    for (const sym of stale) {
      report(
        'SRA-001',
        `\`${sym}\` 不经 \`symbio_sdk::\` 路径命中 \`${SESSION_SRC_REL}\`——账本比真实依赖面宽，` +
          `请同批从 ${EXEMPTION_REL} 摘掉（余量冒充进度；名字在源码里出现不算用法，见 D-454）`,
      )
    }

    const n = exemption.symbols.length
    if (n > SYMBOL_BASELINE) {
      report(
        'SRA-002',
        `豁免符号 ${n} 条 > 基线 ${SYMBOL_BASELINE}——这条边只许收窄（D-451）。` +
          '要加一条，先回答「为什么必须由 session 直取 sdk、而不是由 owner 提供或经门面转出」',
      )
    }
    if (n < SYMBOL_BASELINE) {
      notes.push(
        `符号已收到基线下方（${n} < ${SYMBOL_BASELINE}）：请同步本脚本的 \`SYMBOL_BASELINE\`——` +
          '留着旧数字等于把余量白留给下一次',
      )
    }
    if (VERBOSE) {
      console.log(dim(`  账本 ${n} 条（基线 ${SYMBOL_BASELINE}），源码 ${files.length} 个 .rs，余量 ${stale.length} 条`))
    }
  }
}

// ==================== SRA-003 ====================

if (exemption) {
  const { expires, retire, knife } = exemption

  const parsed = typeof expires === 'string' ? Date.parse(`${expires}T00:00:00Z`) : NaN
  if (Number.isNaN(parsed)) {
    report('SRA-003', `\`expires\` 不是可解析的日期（${JSON.stringify(expires)}）——没有到期日的豁免就是永久豁免`)
  } else if (parsed <= Date.now()) {
    report('SRA-003', `\`expires\`（${expires}）已过期——限期豁免过期即红，续期要有新决策`)
  }

  if (typeof retire !== 'string' || !retire.trim()) {
    report('SRA-003', '`retire` 为空——写不出处置（删除或保留）的豁免，既不会删也不会被复核')
  } else if (/删/.test(retire)) {
    // 删除承诺：退场的动作只有删，照旧。
  } else if (/保留|冻结/.test(retire) && /D-\d{3}/.test(retire)) {
    // 保留承诺（`D-460`）：终态是「随身份留下、只许降」时，「将来删除」是不可兑现的承诺
    // ——`D-458` / `D-459` 已判存储与地址面都不能换 owner，session 这条依赖边不会归零。
    // 允许它写「保留」而不是假装会删，但必须同时写出**依据的决策号**：一句没有决策的
    // 「保留」是谁都不用负责的话，与本脚本存在的理由正好相反。
  } else {
    report(
      'SRA-003',
      `\`retire\` 既没写删除动作、也没写「保留 / 冻结 + 依据决策号」（${JSON.stringify(retire)}）——` +
        '豁免必须有可兑现的处置（D-460）',
    )
  }

  if (typeof knife !== 'string' || !/^[TD]-\d{3}$/.test(knife)) {
    report('SRA-003', `\`knife\` 不是登记号（${JSON.stringify(knife)}）——这条账要指得出主人`)
  } else {
    notes.push(`这条账的 \`knife\` 是 ${knife}；任务按「收口即删」会消失，届时请确认主人换成了谁`)
  }

  notes.push(`到期日 ${expires}；${retire}`)
}

// ==================== SRA-004：事实源开面是单槽 ====================

/**
 * 开面（`symbio_sdk::plugin::fact_source`）只认**一家**登记者：
 * `declared_fact_source_opener()` 取 `inventory::iter::<FactSourceOpenerDecl>().next()`，
 * 没有去重、没有按域分派。生产里登记的只有控制面 session 一家。
 *
 * 为什么这把尺要管这件事（`D-458`）：退役线的**边三**是存储，而存储迁移的第一步必然是
 * 「注册第二个开面，让新的那一半先跑起来」。在单槽下这一步是**静的**——两个登记者里谁
 * 生效由链接顺序决定，编译器不报、运行期不报，`open_fact_source()` 照旧返回 `Ok`。等到
 * 你把旧的删掉，才发现生效的一直是它。所以「恰好一家」必须是一条判据：多出来的那家要么
 * 先把重复处置设计出来（按什么分派），要么不要登记。
 *
 * 两个方向都判：**0 家**不是「迁移完成」，是读面已经没有开面了——`open_fact_source()`
 * 恒返回 `Err("没有登记事实源开面")`，CLI 每次发送都硬错（`symbio-cli/src/client.rs`），
 * 而宿主的安全阀出口只是静默退化（`symbio-host/src/drive.rs::record_stall`）。两种处置
 * 并存，正是「停用」不能靠某一个调用点报错来发现的原因。
 */
function collectCrateLines() {
  const cratesDir = path.join(ROOT, 'crates')
  if (!fs.existsSync(cratesDir)) return null
  const lines = []
  const walk = (d) => {
    for (const entry of fs.readdirSync(d, { withFileTypes: true })) {
      const abs = path.join(d, entry.name)
      if (entry.isDirectory()) {
        if (entry.name === 'target' || entry.name === 'node_modules') continue
        walk(abs)
      } else if (entry.name.endsWith('.rs') && !entry.name.endsWith('.test.rs') && entry.name !== 'tests.rs') {
        const text = stripComments(fs.readFileSync(abs, 'utf8'))
        text.split('\n').forEach((line, i) => {
          lines.push({ rel: path.relative(ROOT, abs).split(path.sep).join('/'), line: i + 1, text: line.trim() })
        })
      }
    }
  }
  walk(cratesDir)
  return lines
}

// SRA-004 / SRA-005 共用一份取法：两处判的都是「单槽登记」这同一件事的两面，
// 各写一遍扫描只会让它们漂移（D-454 的教训）。
const crateLines = collectCrateLines()
const openerSites =
  crateLines === null ? null : crateLines.filter((l) => l.text.includes('submit_fact_source_opener!'))

if (openerSites === null) {
  report('SRA-004', '`crates/` 不存在——开面登记点无从读，判据没有输入')
} else if (openerSites.length === 0) {
  report(
    'SRA-004',
    '事实源开面零登记——`declared_fact_source_opener()` 恒 `None`，读面（CLI 投影 / ' +
      '`session/project`）会硬错。这不是「存储边收窄完成」，是没有任何人开面了',
  )
} else if (openerSites.length > 1) {
  const where = openerSites.map((s) => `${s.rel}:${s.line}`).join(' · ')
  report(
    'SRA-004',
    `事实源开面是**单槽**，却登记了 ${openerSites.length} 家（${where}）——` +
      '`declared_fact_source_opener()` 只取迭代到的第一个，其余是**静默死码**（谁生效由链接顺序决定）。' +
      '先把重复处置设计出来（按什么分派），再登记第二个；迁移不许靠链接顺序决定谁生效（D-458）',
  )
} else if (openerSites[0].rel !== 'crates/symbio-plugin-session/src/store/fact_source.rs') {
  notes.push(
    `事实源开面登记点已不在 session（${openerSites[0].rel}:${openerSites[0].line}）——` +
      '存储换 owner 真的动过；请核对 D-451 边三的读数与寄存区补落点是否跟着搬',
  )
}

// ==================== SRA-005：地址根也是单槽 ====================

/**
 * 地址根（`AddrRootDecl`）同样只认**一家**：`declared_addr_root()` 取
 * `inventory::iter::<AddrRootDecl>().next()`（`crates/symbio-sdk/src/vdfs/address.rs`），
 * 没有去重、没有按域分派。生产里声明的只有 vdfs 插件一家。
 *
 * 为什么这把尺要管这件事（`D-459`）：地址面（`<根>/session/<id>/…`，3462 行）是退役线
 * 上唯一一块「换 owner 等于重写」的地。它的第一段 `<根>` 就是这条静态声明；接管地址面的
 * 第一步，必然是「再声明一个根」或「换掉声明的那一家」。在单槽下前者是**静的**——谁生效
 * 由链接顺序决定，编译器不报、运行期不报，`declared_addr_root()` 照旧返回一个名字。
 * 所以只判 `≥ 2`：多出来的那家要么先把处置设计出来，要么不要声明。
 *
 * **0 家不算红**（与 SRA-004 不同）：`declared_addr_root()` 返回 `None` 是既定降级——
 * 构建里没有 vdfs 插件时虚拟资源面本就不存在，容器传给子插件的父地址退化为裸槽名。
 * 那是一个可诊断的形态，不是「读面硬错」。所以 0 家只留一条提示。
 *
 * 注释不算声明：`text` 已在 `collectCrateLines()` 里剥过注释；`pub struct AddrRootDecl(…)`
 * 的定义行按 `pub struct` 排除。
 */
const addrRootSites =
  crateLines === null
    ? null
    : crateLines.filter((l) => /\bAddrRootDecl\s*\(/.test(l.text) && !/\bpub\s+struct\b/.test(l.text))

if (addrRootSites === null) {
  report('SRA-005', '`crates/` 不存在——地址根声明点无从读，判据没有输入')
} else if (addrRootSites.length > 1) {
  const where = addrRootSites.map((s) => `${s.rel}:${s.line}`).join(' · ')
  report(
    'SRA-005',
    `地址根是**单槽**，却声明了 ${addrRootSites.length} 家（${where}）——` +
      '`declared_addr_root()` 只取迭代到的第一个，其余是**静默死码**（谁生效由链接顺序决定）。' +
      '接管地址面前必须先把处置设计出来（按什么分派），不许靠链接顺序决定（D-459）',
  )
} else if (addrRootSites.length === 0) {
  notes.push(
    '地址根零声明——`declared_addr_root()` 恒 `None`，虚拟地址面整体降级为相对地址。' +
      '这在没有 vdfs 插件的构建里是合法的，但若本次改动动了地址面，请确认不是把它删掉了（D-459）',
  )
}

// ==================== --breakdown（读数，不是判据） ====================

/**
 * 账本按「住处」分解：账上每条符号被用在哪一块代码里。
 *
 * **这不是门禁。** 分组口径说错了只会让报告难看，不会放宽任何一条判据（`--breakdown` 的
 * 退出码与普通模式完全一致）。它存在的唯一理由是让「这条边还剩什么」这类判断可以由一条
 * 命令复现，而不是靠人在文档里数一遍——数错的分母比没有分母更糟（`D-454` 就是数错的产物）。
 *
 * 「住处」取自 crate 目录结构，不猜实现角色：
 * - `store`：`store/`（WAL）与 `transcript/`（转写）——D-451 边三 的两块；
 * - `asm`：`plugin.rs` / `plugin/` / `capabilities.rs` / `declarations.rs` / `options/`
 *   ——D-451 边二 说「装配」时点名的那些面（traverse 收集、声明表、选项遍历、回合装配）；
 * - `none`：三条边都没点到的地（命令面、统计读侧、投影）。
 */
const ASM_EXACT = new Set(['plugin.rs', 'capabilities.rs', 'declarations.rs'])
const ASM_PREFIX = ['plugin/', 'options/']
const STORE_PREFIX = ['store/', 'transcript/']

function whereLives(rel) {
  if (STORE_PREFIX.some((p) => rel.startsWith(p))) return 'store'
  if (ASM_EXACT.has(rel) || ASM_PREFIX.some((p) => rel.startsWith(p))) return 'asm'
  return 'other'
}

function printBreakdown(symbols) {
  const homes = new Map(symbols.map((s) => [s, { at: new Set(), files: new Set() }]))
  for (const { rel, syms } of perFile) {
    const where = whereLives(rel)
    for (const s of syms) {
      const h = homes.get(s)
      if (!h) continue
      h.at.add(where)
      h.files.add(rel)
    }
  }
  const groups = [['asm', []], ['store', []], ['both', []], ['none', []]]
  const by = Object.fromEntries(groups)
  for (const [s, h] of homes) {
    if (h.at.has('asm') && h.at.has('store')) by.both.push(s)
    else if (h.at.has('asm')) by.asm.push(s)
    else if (h.at.has('store')) by.store.push(s)
    else by.none.push(s)
  }
  console.log(
    `\n账本按「住处」分解（${symbols.length} 条）——读数，不是判据（D-456）：\n` +
      `  只出现在 装配面（plugin.rs / plugin/ / capabilities.rs / declarations.rs / options/）  ${by.asm.length}\n` +
      `  只出现在 存储·转写（store/ / transcript/）                                          ${by.store.length}\n` +
      `  两处都有                                                                            ${by.both.length}\n` +
      `  两处都没有（命令面 / 统计读侧 / 投影）                                              ${by.none.length}`,
  )
  for (const s of by.none.sort()) {
    console.log(dim(`    ${s.padEnd(30)} ${[...homes.get(s).files].sort().join(' ')}`))
  }
}

// ==================== --coverage（读数，不是判据） ====================

/**
 * 每一块代码归谁——**按「这份代码在退役后由谁接」分**，不按目录名猜。
 *
 * 三条桶对应 `D-451`／`D-457` 的边二 / 边三，外加两个**没有边**的桶：
 * - `address`：VDFS 地址面（`<根>/session/<id>/…`）。它不是空壳——CLI 往
 *   `<根>/session/<id>/inbox` 写、子智能体也经 `/inbox` 发起子会话、CLI 按 `path`
 *   归属订阅变更。**三条边没有一条管它**（`D-457`）。
 * - `other`：判不出来的（命令面 / 统计读侧 / 配置 / 活跃名册 / 工作目录…）。落这里
 *   的文件会被**逐条列出来**，不静默并进别的桶。
 */
const COVERAGE_BUCKETS = [
  {
    id: 'edge2',
    label: '边二 装配（收集与折）',
    match: (rel) => rel === 'capabilities.rs',
  },
  {
    id: 'edge3',
    label: '边三 存储·转写',
    match: (rel) =>
      rel.startsWith('store/') ||
      rel.startsWith('transcript') ||
      rel === 'convergence.rs',
  },
  {
    id: 'address',
    label: '地址面（VDFS）· 无边的归属',
    match: (rel) =>
      rel === 'plugin.rs' ||
      rel === 'plugin/nodes.rs' ||
      rel === 'declarations.rs' ||
      rel.startsWith('plugin/vdfs_'),
  },
  {
    id: 'turn',
    label: '开轮序列（装配的折叠住这里）',
    match: (rel) => rel === 'plugin/turn.rs',
  },
]

/** 测试文件不算「要搬的代码」：它们的去处是随宿主一起删/改，不是搬。 */
function isTestSrc(abs) {
  const name = path.basename(abs)
  return name.endsWith('.test.rs') || name === 'tests.rs'
}

/**
 * 一份源码的**内容行数**（与 `wc -l` 同一口径）。
 *
 * 原先这里写的是 `text.split('\n').length`——那对每个以换行结尾的文件多算 1 行，
 * 56 个文件就是 56 行虚高（`D-458`：读数的分母必须是它声称的那个数，与 `D-454`
 * 同一条纪律）。空文件记 0 行，不记 1 行。
 */
function locOf(text) {
  if (text.length === 0) return 0
  const body = text.endsWith('\n') ? text.slice(0, -1) : text
  return body.split('\n').length
}

function printCoverage() {
  const rows = COVERAGE_BUCKETS.map((b) => ({ ...b, loc: 0, files: [] }))
  const other = { label: '没有归属的其余代码', loc: 0, files: [] }
  let total = 0
  for (const f of files) {
    if (isTestSrc(f)) continue
    const rel = path.relative(srcDir, f).split(path.sep).join('/')
    const loc = locOf(fs.readFileSync(f, 'utf8'))
    total += loc
    const hit = rows.find((b) => b.match(rel))
    const target = hit ?? other
    target.loc += loc
    target.files.push(rel)
  }
  const edges = rows.filter((r) => r.id === 'edge2' || r.id === 'edge3')
  const covered = edges.reduce((n, r) => n + r.loc, 0)
  const pct = (n) => `${((n / total) * 100).toFixed(1)}%`
  console.log(
    `\n退役覆盖面（非测试源码 ${total} 行）——读数，不是判据（D-457）：\n` +
      [...rows, other]
        .map((r) => `  ${r.label.padEnd(34)} ${String(r.loc).padStart(6)} 行  ${pct(r.loc).padStart(6)}  ${r.files.length} 个文件`)
        .join('\n') +
      `\n  ${'三条边合计（边二 + 边三；边一是条数，不占行）'.padEnd(32)} ${String(covered).padStart(6)} 行  ${pct(covered).padStart(6)}` +
      `\n  ${'没有边的（地址面 + 其余）'.padEnd(32)} ${String(total - covered).padStart(6)} 行  ${pct(total - covered).padStart(6)}`,
  )
  console.log(dim('\n  「没有归属」逐文件（判不出就列出来，不静默并进别的桶）：'))
  for (const r of other.files.sort()) console.log(dim(`    ${r}`))
  if (rows.some((r) => r.id === 'turn' && r.files.includes('plugin/turn.rs'))) {
    console.log(dim('  ⚠ 分桶偏差：装配的折叠（exposed_tools / dedup_declared_twins）住在'))
    console.log(dim('    plugin/turn.rs 里，整份记在「开轮序列」⇒ 边二读数偏小，不偏大。'))
  }
}

// ==================== 报告 ====================

if (errors.length) {
  console.error(`\n== session-retire-audit：${errors.length} 处违规 ==`)
  for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
  console.error(
    `\n${yellow('提示')}：这把尺量的是**依赖边的收窄**（${EXEMPTION_REL} 的符号集），` +
      '不是「搬了多少行」。阶次与各阶出口见 DE-056 的 D-451。',
  )
  process.exit(1)
}

const addrRootLine =
  addrRootSites.length === 1
    ? `${green('✓')} SRA-005 地址根单槽：1 家（${addrRootSites[0].rel}:${addrRootSites[0].line}）`
    : `${green('✓')} SRA-005 地址根单槽：零声明（合法降级，见下方提示）`

console.log(
  `${green('✓')} SRA-001 豁免账零余量：${exemption.symbols.length} 条符号在 session 源码里都有真实用法\n` +
    `${green('✓')} SRA-002 只许降：${exemption.symbols.length} 条 ≤ 基线 ${SYMBOL_BASELINE}\n` +
    `${green('✓')} SRA-003 豁免自洽：到期日 ${exemption.expires} · ${exemption.retire}\n` +
    `${green('✓')} SRA-004 事实源开面单槽：1 家（${openerSites[0].rel}:${openerSites[0].line}）\n` +
    addrRootLine,
)
for (const n of notes) console.log(dim(`  · ${n}`))
if (BREAKDOWN) printBreakdown(exemption.symbols)
if (COVERAGE) printCoverage()
process.exit(0)
