#!/usr/bin/env node
/**
 * arch-change-audit — **「改插件顺手动内核」必须在提交里留下理由**（判定型）
 *
 * ## 它判什么
 *
 * | 编号 | 规则 | 它拦的是哪一种回潮 |
 * |---|---|---|
 * | CC-001 | 提交**新增了** `crates/symbio-sdk/**` / `crates/symbio-kernel/**` / `crates/symbio-contract/**` 的生产 `.rs` 代码行，提交信息里没有 `Core-Change:` 段 | 内核 / 契约层被当成「顺手改一下」的地方：没有一行文字解释「为什么非动它不可」，就没有人能复核。纯测试断言（`.test.rs` / `tests/`）、纯注释、**纯收窄**（只删不增）都不改公开面与语义，不在判定面内 |
 * | CC-002 | 同一提交**既动插件又动 core/kernel**，而 `Core-Change:` 的值不是 `D-NNN` | 横向扩充能力的提交悄悄扩大了内核面——正是 DE-023 §2 第 1 条（扩展局部性）要拦的形态 |
 *
 * ## 为什么用提交信息，而不是再写一个静态审计
 *
 * 「这次改动是不是在拿插件功能换内核改动」**不在任何单个文件里**——它是一次提交的
 * 意图。静态扫描只能看到「core 里有插件 id 字面量增加了」（CS-004 已在判），看不到
 * 「为了让一个新插件跑起来，我在 core 里加了个分支」。前者是结果，后者是原因。
 *
 * 代价是这条判据**依赖人写字**：`Core-Change: D-195` 不是自证的。所以它只做两件事——
 * 强制留下理由、强制在「扩能力」这一档给出决策号。**理由是否成立由 review 判**，
 * 不由脚本判；脚本只保证「没有理由」无法通过。
 *
 * ## 豁免名单也要有理由（`validateBaseline`）
 *
 * 「不追改已推送的历史」是这条判据唯一诚实的退路，但退路本身会被滥用，所以名单的
 * 形状由脚本判：每条必须有提交号与 `why`；**晚于判据诞生**的那些还必须带 `decision`
 * （这条内核改动自己的裁决号），且那个 `D-NNN` 必须能在 `docs/003-design/` 里打开。
 * 判据是：豁免顶替的就是那条缺失的 `Core-Change:`，内容不许比它少。
 *
 * ## 判据的语料来自 git 自己
 *
 * 不维护「哪些路径算 core」的清单：一次 `git log --no-merges --raw` 给出真实改动文件，
 * 目录常量只有三条（`crates/symbio-sdk/` / `crates/symbio-kernel/` /
 * `crates/symbio-contract/`），与 `arch-shape-audit` 的 `core-kernel-files:` 标记同源。
 * D-486 五层规划把契约层立为冻结 ABI 层——它的改动同样需要 `Core-Change:` 段。
 *
 * ## 判据退役随架构裁决走（流程纪律，不是本脚本的机械判定）
 *
 * 本条只判**新增**的 core 行——裁决把被测物删掉（纯收窄）时，指向它的判据不会在这
 * 触发任何机械判定。所以「删了被测物 ⇒ 同批检查指向它的判据是否该退役」是流程纪律：
 * 架构裁决里顺手核对，退役写成判据自己的 `// @state retired: D-NNN <理由>`（对账归
 * `gate-wiring-audit` 的 GW-009），不在这里另开通道。
 *
 * ## 用法
 *
 * ```bash
 * node scripts/audits/arch-change-audit.mjs --range origin/main..HEAD   # CI
 * node scripts/audits/arch-change-audit.mjs --require-range             # 无 base 可判时判红；CI 环境（CI 变量）自动等效
 * node scripts/audits/arch-change-audit.mjs --last                      # 只判 HEAD
 * node scripts/audits/arch-change-audit.mjs                             # 自动取 merge-base(origin/main, HEAD)..HEAD
 * ```
 *
 * 退出码：0 = 通过（或本地无可判范围）；1 = 有不符合的提交，或自动化里无可判范围。
 */
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, yellow, dim } from '../color.mjs'
import { backfillShas, loadBackfill } from '../core-change-backfill.mjs'
import {
  gitOut, GIT_LOG_PRETTY, requiresRange, resolveRange, parseGitLogRecords,
} from '../gate.d/_shared.mjs'
// 回归测试直接喂样本的两个纯函数仍从本脚本取，实现已归共享库。
export { requiresRange, parseGitLogRecords as parseCommitRecords }

const git = (args) => gitOut(args, { cwd: repoRoot })

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const argv = process.argv.slice(2)

/** 内核 / 契约层：改动这两处必须在提交里解释 */
const ARCH_DIRS = ['crates/symbio-sdk/', 'crates/symbio-kernel/', 'crates/symbio-contract/']
/** 「扩能力」的证据：动到任何插件 / provider / 测试设施 */
const EXT_DIRS = ['crates/symbio-plugin-', 'crates/symbio-provider', 'crates/symbio-testkit/']

/** `Core-Change: D-195` / `Core-Change: D-195 理由…` / `Core-Change: 纯注释整理` */
const TRAILER_RE = /^\s*Core-Change[:：]\s*(.+)$/m

/**
 * 追溯豁免：**不追改已经推出去的历史**。
 *
 * 把一条判据判红的唯一效果，是逼人去 `--amend` / rebase 重写历史——那不是收紧，是制造
 * 一次不可复核的改写。两种情形因此都必须允许豁免：
 *
 * - `beforeGuard: true` —— 判据诞生之前的提交，它不可能自带 `Core-Change:`。
 * - `beforeGuard: false` —— 判据之后**但已经推到远端**的提交。这一类的代价是把「留理由」
 *   的责任从提交信息搬到了这里，所以它必须补上豁免本该写的那句话：`decision` 指向
 *   这条内核改动自己的裁决（`D-NNN`，且必须能在设计文档的「关键决策」里打开，见
 *   `knownDecisions`），`why` 说清为什么不能改历史。缺任何一列 ⇒ 本脚本判红。
 *
 * 为什么不干脆做成「开关」：一个没有内容的名单就是永久后门——三个月后没人记得它为什么
 * 在那儿。汇总行会数出豁免条数（`追溯豁免 N`），条数变化在门禁输出里看得见。
 */
const BASELINE = [
  {
    sha: '92cd0528',
    beforeGuard: true,
    why: '拆 crate（D-191）本身——判据诞生于它之后，它不可能自带 Core-Change。',
  },
  {
    sha: '252e67a6',
    beforeGuard: true,
    why: '让拆 crate 后的守卫重新看得见真实仓（门禁 142/144）——同属判据之前。',
  },
  {
    sha: '4e6cd88e',
    beforeGuard: false,
    decision: 'D-336（跨插件模型往返：共享载荷住 SDK 词汇层）',
    why:
      '该提交是用户手工收尾的死码删除批（`D-336` 尾款）：`crates/symbio-sdk/src/turn/facts.rs` 只是删掉' +
      '`DrainedSupplement`（最后一个引用随同步抽干口一起消失），属 narrowing，没有扩能力；' +
      '提交信息只写了批次号与绿点，缺独立 `Core-Change:` 行。该提交已由用户核对收下，' +
      '按「不追改历史」处理；删除本身的边界就是 D-336 的迁移尾款。',
  },
  {
    sha: '6b16285e',
    beforeGuard: false,
    // 决策号写成**散文的一部分**，不是整串等值：`gate-codes-audit` 把整串 `'D-272'`
    // 当成「本脚本发射了一个判据码」，而这里指的是设计文档里的那条裁决（GC 的 D 前缀判法）。
    decision: 'D-272（core 根出口只出自己定义的东西，023 §D-272）',
    why:
      'D-272（core 根出口 324 → 34）的提交信息里写的是「裁决三件」与「决策：023 §D-272」，' +
      '没有 `Core-Change:` 那一行的字面形态——理由是有的（根出口收窄本身就是内核改动），' +
      '只是没落在这个 key 上。提交已在 origin/v2 上：追改它的唯一方式是 rewrite 已推送的历史。',
  },
  {
    sha: '5189d064',
    beforeGuard: false,
    decision: 'D-307（授权事实挂 scope 实体）',
    why:
      'D-307 的提交信息把内核改动理由写在正文第 1 条（「Core-Change：」中文冒号 + 编号前缀），' +
      '不是判据要求的独立 trailer 行，字面形态不匹配。理由本身完整（Entity 新增 Scope 变体、' +
      'CS-009 放宽行挂在 D-307 旁）；该提交已经用户核实并要求不再改动，故按「不追改历史」处理，' +
      '修正字面形态留待日后重写提交信息时一并处理。',
  },
  {
    sha: '33f3e52a',
    beforeGuard: false,
    decision: 'D-313（P2 切割计划：轮编排退出环，生产入口改为按债驱动）',
    why:
      'P2-3a 把模型往返的「解析 + 适配器构造 + 流式 / 回退 / 终止判定」搬进端口执行体，' +
      '为此在 SDK / kernel 补了通用往返与事实写口；提交信息只写了批次号与绿点，' +
      '没有独立 `Core-Change:` 行。该提交已经是 P2 批次绿点（用户已收），' +
      '追改它的唯一方式是重写已核对的历史，故按「不追改历史」处理。',
  },
  {
    sha: '3eaefce8',
    beforeGuard: false,
    decision: 'D-313（P2 切割计划：轮编排退出环，生产入口改为按债驱动）',
    why:
      'P2-3a2 步骤一：流式增量改走 provider 自有事实行，节点身份与投影口径随之内核 / SDK 同调。' +
      '提交信息缺独立 `Core-Change:` 行；理由即 D-313 的「环不解析端口载荷、展示按行身份投影」。' +
      '提交已是绿点，按「不追改历史」处理。',
  },
  {
    sha: '3aae6e62',
    beforeGuard: false,
    decision: 'D-313（P2 切割计划：轮编排退出环，生产入口改为按债驱动）',
    why:
      'P2-3b2a 立起 `port=session` 的声明与执行体（D-315）：端口合同与通用执行上下文住在 ' +
      'plugin-api / SDK，属于内核 / 契约层改动；提交信息缺独立 `Core-Change:` 行。' +
      '提交已是绿点，按「不追改历史」处理。',
  },
  {
    sha: '1069f346',
    beforeGuard: false,
    decision: 'D-313（P2 切割计划：轮编排退出环，生产入口改为按债驱动）',
    why:
      'P2-3b2-1 把配置与 facts 读点切到 `session-config` / `session-facts` 同源投影，' +
      '涉及 SDK 的读面类型；提交信息缺独立 `Core-Change:` 行。理由即 D-313 / D-315 的' +
      '「会话状态归世界端口」。提交已是绿点，按「不追改历史」处理。',
  },
]

/** `docs/003-design/**` 里存在过的决策号；豁免的 `decision` 必须能在其中打开。 */
function knownDecisions() {
  const out = new Set()
  const dirs = [path.join(repoRoot, 'docs', '003-design')]
  const walk = (dir) => {
    let entries
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true })
    } catch {
      return
    }
    for (const e of entries) {
      const p = path.join(dir, e.name)
      if (e.isDirectory()) walk(p)
      else if (e.isFile() && e.name.endsWith('.md')) {
        for (const m of fs.readFileSync(p, 'utf8').matchAll(/^### (D-\d+)/gm)) out.add(m[1])
      }
    }
  }
  for (const d of dirs) walk(d)
  return out
}

/**
 * 豁免名单自身的形状判据（纯函数，判据 = `arch-change-audit.test.mjs`）：
 * 空理由、缺 `sha`、判据之后的豁免没有可打开的决策号，都判红。
 */
export function validateBaseline(entries, decisions) {
  const errors = []
  for (const e of entries) {
    const at = e?.sha ?? '(无 sha)'
    if (!e?.sha || !/^[0-9a-f]{7,40}$/.test(e.sha)) {
      errors.push(`${at}：豁免条目必须有 7–40 位的提交号`)
      continue
    }
    if (typeof e.why !== 'string' || !e.why.trim()) {
      errors.push(`${e.sha}：豁免没有理由正文——一条不解释自己的豁免就是永久后门`)
    }
    if (e.beforeGuard === false) {
      // 从一句话里抠出决策号（不要求整串等值）：裸的 `'D-124''` 会被 `gate-codes-audit`
      // 当成脚本发射了一个判据码，而这里的 D 号是设计文档的决策指路。
      const cited = String(e.decision ?? '').match(/\bD-\d+\b/)?.[0] ?? null
      if (!cited) {
        errors.push(
          `${e.sha}：晚于判据的豁免必须补上这条内核改动自己的裁决（decision 里写一个 D-NNN）——` +
            '豁免顶替的就是那条 `Core-Change:`，内容不许比它少',
        )
      } else if (decisions && !decisions.has(cited)) {
        errors.push(`${e.sha}：decision ${cited} 在 docs/003-design/ 里打不开（写错的决策号等于没写）`)
      }
    }
  }
  return errors
}

const BASELINE_SHAS = new Set(BASELINE.map((e) => e.sha))


/**
 * 一次 `git log` 拿到范围内**每个提交的 sha、完整信息与改动文件清单**。
 *
 * 原先这里是「每提交 spawn 一次 `diff-tree` + 触及 core 的再 spawn 一次 `log`」：
 * 816 个提交 ≈ 1700 次进程创建，实测 33s——而它是 docs 阶段最慢的一项，同阶段
 * 其余 38 个守卫本体都在 0–3s。`--no-merges` + `--raw` 一次遍历就给出同样的
 * 三元组（实测本机 53ms）⇒ 三次 spawn 替代一千七百次。
 *
 * 与旧实现逐条对拍过：816 条提交的**文件清单完全一致**（`--no-merges` 已排除合并
 * 提交，正是旧实现里那些被 `diff-tree` 判成空清单而 `continue` 掉的记录）。
 */
function commitsWithFiles(range) {
  const out = git([
    'log',
    '--no-merges',
    '--reverse',
    '--raw',
    '--no-renames',
    `--pretty=format:${GIT_LOG_PRETTY}`,
    range,
  ])
  if (out === null) return null
  return parseGitLogRecords(out)
}

/**
 * 一段 `git show --unified=0` 的 diff 属于哪一种改动。
 *
 * | 返回值 | 含义 | 要不要 `Core-Change:` |
 * |---|---|---|
 * | `'none'` | 一行代码都没动（只改注释 / 空行） | 不要 |
 * | `'narrowing'` | 只删代码、没新增任何代码行 | 不要 |
 * | `'semantic'` | 新增了代码行（含改一行：删一行 + 加一行） | 要 |
 *
 * **收窄不需要理由**是这个仓库里反复立过的一条：所有棘轮（`CS-001` / `CS-009` /
 * `PP-004` / `HS-002` / `HS-006`）都只许降，降不需要批准——批准只为「放宽」而存在。
 * 本条此前只认「只改注释」，于是**删掉一个零消费方的常量**（内核面变小、风险变小）
 * 会被要求补一条 `Core-Change:`，而设计文档里根本没有对应决策可挂：判据把人逼到
 * 要么自造决策号、要么改写已推送的历史，两条都比它想拦的东西更糟。
 *
 * 读不到 diff（`patch === null`）一律当 `'semantic'`：错的绿比红的代价大。
 */
export function patchVerdict(patch) {
  if (patch === null) return 'semantic'
  let added = 0
  let removed = 0
  for (const line of String(patch).split('\n')) {
    if (!/^[+-]/.test(line)) continue
    if (line.startsWith('+++') || line.startsWith('---')) continue
    const code = line.slice(1).replace(/\/\/.*$/, '').trim()
    if (code.length === 0) continue
    if (line[0] === '+') added += 1
    else removed += 1
  }
  if (added > 0) return 'semantic'
  return removed > 0 ? 'narrowing' : 'none'
}

const inDirs = (files, dirs) => files.some((f) => dirs.some((d) => f.startsWith(d)))

/**
 * 只判**生产**源码。测试文件（`*.test.rs` 与 `tests/` 下的用例）断言的只是既有契约，
 * 不改内核的公开面与语义——把测试维护算成「改内核」会让一次夹具改名也要求架构理由，
 * 这条判据随即退化成填表（与 CC-001 只管源码、不管 Cargo.toml 是同一条理由）。
 */
export function isProductionSource(f) {
  return f.endsWith('.rs') && !/\.test\.rs$/.test(f) && !/(^|\/)tests\//.test(f)
}

/**
 * 判一条提交。
 *
 * 返回 `{ sha, subject, errors: string[] }`；`errors` 空即通过。
 */
export function judge(sha, files, message) {
  const errors = []
  // 只对**生产源码**判 CC-001。测试文件、`Cargo.toml` / `README.md` 的改动不改变内核的公开面与语义，
  // 而依赖清单几乎每轮都在变（一次依赖整理就动过 31 份，其中包含 core 的）。把清单算进来，
  // 每一轮机械整理都要挂一条 `Core-Change:`，这条判据会从「留理由」退化成「每次都填表」，
  // 贬值速度比它拦住的东西快。清单膨胀的真实后果是内核面变大，那由 C-002（深引）与
  // C-004（账本条数）从**结果面**判——本条判的是**意图**，两者不该混。
  const sources = files.filter(isProductionSource)
  if (!inDirs(sources, ARCH_DIRS)) return { sha, errors }

  const m = message.match(TRAILER_RE)
  const touchesExt = inDirs(files, EXT_DIRS)

  if (!m) {
    errors.push(
      'CC-001 触及 core/kernel 但没有 `Core-Change:` 段 —— 需写清「为什么非动内核不可」' +
        '（扩能力请挂决策号，如 `Core-Change: D-195`）',
    )
  } else if (touchesExt && !/D-\d{3,}/.test(m[1])) {
    errors.push(
      `CC-002 同一提交既动插件又动 core/kernel，但 \`Core-Change: ${m[1].trim()}\` 里没有决策号` +
        ' —— 横向扩充能力不该顺手扩大内核面；确需扩，挂 `D-NNN` 并说明为什么共享无处可放',
    )
  }
  return { sha, errors }
}

function subjectsOf(commits) {
  const map = new Map()
  for (const c of commits) {
    const subject = c.message.split('\n')[0] ?? ''
    map.set(c.sha.slice(0, 7), subject)
  }
  return map
}


function main() {
  // 豁免名单自己先过一遍形状判据：**判据的红线不能靠它豁免的那几行代码来解释**。
  // 这一格放在范围解析之前，因为它与「有没有 base 可判」无关——浅克隆的 CI 也要能
  // 看见「名单里有一条没写理由的豁免」。
  const baselineErrors = validateBaseline(BASELINE, knownDecisions())
  if (baselineErrors.length) {
    for (const e of baselineErrors) console.error(red(`✗ 追溯豁免名单：${e}`))
    return 1
  }

  // 逐提交补记表（D-333）：提交不能改写，缺 `Core-Change:` 的历史只能由表补记。
  // 表自身的形状先过判据——「在表内 ⇒ 按表核验」，核验的就是这张表（不是跳过）。
  const backfill = loadBackfill(repoRoot)
  if (backfill.errors.length) {
    for (const e of backfill.errors) console.error(red(`✗ 补记表：${e}`))
    return 1
  }
  const BACKFILL_SHAS = backfillShas(backfill.entries, 'CC-001')

  const range = resolveRange(argv, git)
  if (!range) {
    // **无 git 事实可判 ≠ 判据成立**。浅克隆、`actions/checkout` 没拉 base、
    // detached HEAD 都会走到这里；退 0 等于这道闸在那些机器上**静默消失**，而日志里
    // 只有一行黄字。自动化里「没人判」必须等于「判不过」，否则 CI 的绿灯不代表任何东西。
    //
    // 判红条件是 `--require-range` **或** CI 变量：只靠传参就得有人记得拼，而 workflow
    // 是按脚本名跑门禁的，从没拼过它——那条为 CI 加的红线因此一直没被拉起来。
    // 本地（无 CI 变量）仍按「跳过，不当作通过」，不逼开发者去配 base ref。
    const forced = requiresRange(argv)
    const msg =
      '⊘ arch-change-audit：找不到可判的提交范围（无 origin/main 也无 --range）——跳过，不当作通过'
    if (forced) {
      console.error(
        red(
          `✗ arch-change-audit：${msg.slice(1)}；${argv.includes('--require-range') ? '本次要求有可判范围' : '自动化环境（CI 变量）要求有可判范围'} ⇒ 判红`,
        ),
      )
      return 1
    }
    console.log(yellow(msg))
    return 0
  }

  const commits = commitsWithFiles(range)
  if (commits === null) {
    console.error(red(`✗ 解析提交范围失败：${range}`))
    return 1
  }
  if (commits.length === 0) {
    console.log(green(`✓ arch-change-audit：范围内没有提交（${range}）`))
    return 0
  }

  const subjects = subjectsOf(commits)
  const bad = []
  let coreTouching = 0
  let skipped = 0
  let backfilled = 0
  let narrowed = 0
  for (const { sha, message, files: rawFiles } of commits) {
    if (BASELINE_SHAS.has(sha.slice(0, 8))) {
      skipped += 1
      continue
    }
    // 在补记表内：按表核验通过（表的形状已在范围解析前判过），这条提交不再要求
    // 提交信息里的 `Core-Change:`——它缺失的署名由表补上。
    if (BACKFILL_SHAS.has(sha.slice(0, 8))) {
      skipped += 1
      backfilled += 1
      continue
    }
    let files = rawFiles
    // 内核文件的改动性质要按**真实 diff** 判，不能按「文件被打开过」判：只加注释
    // （`arch-shape-allow` 登记）、只删代码（零消费方常量退场）都不改变内核语义，
    // 两者都不该要求架构理由。判法见 `patchVerdict`。
    if (inDirs(files, ARCH_DIRS)) {
      const core = files.filter((f) => ARCH_DIRS.some((d) => f.startsWith(d)))
      const patch = git(['show', '--format=', '--unified=0', sha, '--', ...core])
      const verdict = patchVerdict(patch)
      if (verdict !== 'semantic') {
        files = files.filter((f) => !core.includes(f))
        if (verdict === 'narrowing') narrowed += 1
      }
    }
    if (inDirs(files.filter(isProductionSource), ARCH_DIRS)) coreTouching += 1
    const r = judge(sha, files, message)
    if (r.errors.length) bad.push(r)
  }

  if (bad.length === 0) {
    console.log(
      green(
        `✓ arch-change-audit：${commits.length} 个提交（追溯豁免 ${skipped}，其中补记表 ${backfilled}）` +
          `里 ${coreTouching} 个触及 core/kernel，全部留了理由（另有 ${narrowed} 个只是收窄 / 改注释，无需理由）`,
      ),
    )
    return 0
  }

  console.log(red(`\narch-change-audit 未通过：${bad.length} / ${commits.length} 个提交`))
  for (const r of bad) {
    console.log(`\n  ${r.sha.slice(0, 8)} ${subjects.get(r.sha.slice(0, 7)) || ''}`)
    for (const e of r.errors) console.log(`    ${e}`)
  }
  console.log(
    dim(
      '\n  豁免的唯一形态是理由：`Core-Change: D-NNN <为什么这条共享只能住 core>`。' +
        '\n  没有决策号可挂，通常说明该共享类型其实该住生产方插件（D-180）。',
    ),
  )
  return 1
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(main())
}
