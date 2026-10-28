#!/usr/bin/env node
/**
 * core-change-audit — **「改插件顺手动内核」必须在提交里留下理由**（判定型）
 *
 * ## 它判什么
 *
 * | 编号 | 规则 | 它拦的是哪一种回潮 |
 * |---|---|---|
 * | CC-001 | 提交触及 `crates/symbio-core/**` 或 `crates/symbio-kernel/**`，提交信息里没有 `Core-Change:` 段 | 内核 / 契约层被当成「顺手改一下」的地方：没有一行文字解释「为什么非动它不可」，就没有人能复核 |
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
 * 目录常量只有两条（`crates/symbio-core/` / `crates/symbio-kernel/`），与
 * `core-shape-audit` 的 `core-kernel-files:` 标记同源。新加一个 crate 不需要改本脚本。
 *
 * ## 用法
 *
 * ```bash
 * node scripts/core-change-audit.mjs --range origin/main..HEAD   # CI
 * node scripts/core-change-audit.mjs --require-range             # 无 base 可判时判红；CI 环境（CI 变量）自动等效
 * node scripts/core-change-audit.mjs --last                      # 只判 HEAD
 * node scripts/core-change-audit.mjs                             # 自动取 merge-base(origin/main, HEAD)..HEAD
 * ```
 *
 * 退出码：0 = 通过（或本地无可判范围）；1 = 有不符合的提交，或自动化里无可判范围。
 */

import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { red, green, yellow, dim } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const argv = process.argv.slice(2)

/** 内核 / 契约层：改动这两处必须在提交里解释 */
const CORE_DIRS = ['crates/symbio-core/', 'crates/symbio-kernel/']
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
 * 豁免名单自身的形状判据（纯函数，判据 = `core-change-audit.test.mjs`）：
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

function git(args) {
  const r = spawnSync('git', args, {
    cwd: repoRoot,
    encoding: 'utf8',
    shell: false,
    maxBuffer: 64 * 1024 * 1024,
  })
  if (r.status !== 0) return null
  return r.stdout
}

const REC = '\x01'
/**
 * 分隔符写成 git 的 `%xNN` 转义而不是字面控制字符：Node 的 `spawn` **拒绝参数里含
 * NUL 字节**（`ERR_INVALID_ARG_VALUE`），而 `%x00` 由 git 在输出里展开，两边都不脏。
 */
const PRETTY = `%x01%H%x00%B`

/**
 * `--raw` 的一条文件记录：`:100644 100644 3479b6d5 a34de8f1 M\tpath`。
 *
 * 判它**不只看行首的 `:`**——正文里以 `:` 开头的行（列表、`键: 值`）会把 message
 * 拦腰截断，而 `Core-Change:` trailer 住在正文末尾：截断即等于「没写理由」，
 * 一批提交会莫名判红。两个源模式字段是定长 6 位八进制，够把两类行分开。
 */
const RAW_FILE_LINE_RE = /^:[0-7]{6} [0-7]{6} /

/**
 * 解析 `git log --raw --pretty=format:<REC>%H%x00%B` 的输出。
 *
 * 抽成纯函数并被 `judge` 的回归测试直接喂样本：这一层原先没有测试，注释写的是
 * 「喂进来的形状由门禁实跑覆盖」——而批量化改的**恰恰就是这一层**，实测第一版
 * 把正文切到只剩标题，`Core-Change:` trailer 全丢，23 条提交被误判红。**只有
 * 真跑一遍才会露**，而真跑一遍的代价是整轮门禁，所以判据必须能在函数层面被喂。
 *
 * 两条形状事实（都踩过 / 都会再踩）：
 *  - 文件记录由 `RAW_FILE_LINE_RE` 认出，正文与 trailer 都不匹配它；
 *  - 消息必须取到第一条文件记录**之前**的全部——`Core-Change:` 住在正文末尾。
 */
export function parseCommitRecords(out) {
  const commits = []
  for (const rec of String(out).split(REC)) {
    if (!rec.trim()) continue
    const nul = rec.indexOf('\x00')
    if (nul < 0) continue
    const sha = rec.slice(0, nul).trim()
    const lines = rec.slice(nul + 1).split('\n')
    const firstFile = lines.findIndex((l) => RAW_FILE_LINE_RE.test(l))
    const message = (firstFile < 0 ? lines : lines.slice(0, firstFile)).join('\n')
    const files =
      firstFile < 0
        ? []
        : lines
            .slice(firstFile)
            .map((l) => l.slice(l.lastIndexOf('\t') + 1).trim())
            .filter(Boolean)
    commits.push({ sha, message, files })
  }
  return commits
}

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
    `--pretty=format:${PRETTY}`,
    range,
  ])
  if (out === null) return null
  return parseCommitRecords(out)
}

const inDirs = (files, dirs) => files.some((f) => dirs.some((d) => f.startsWith(d)))

/**
 * 判一条提交。
 *
 * 返回 `{ sha, subject, errors: string[] }`；`errors` 空即通过。
 */
export function judge(sha, files, message) {
  const errors = []
  // 只对**源码**判 CC-001。`Cargo.toml` / `README.md` 的改动不改变内核的公开面与语义，
  // 而依赖清单几乎每轮都在变（一次依赖整理就动过 31 份，其中包含 core 的）。把清单算进来，
  // 每一轮机械整理都要挂一条 `Core-Change:`，这条判据会从「留理由」退化成「每次都填表」，
  // 贬值速度比它拦住的东西快。清单膨胀的真实后果是内核面变大，那由 C-002（深引）与
  // C-004（账本条数）从**结果面**判——本条判的是**意图**，两者不该混。
  const sources = files.filter((f) => f.endsWith('.rs'))
  if (!inDirs(sources, CORE_DIRS)) return { sha, errors }

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

/**
 * 是否自动化环境。GitHub Actions 恒置 `CI=true`，`GITHUB_ACTIONS` 是它的专属变量；
 * 值为空 / `false` / `0` 都算不是（有人本地导出 CI=false 来跑脚本）。
 */
function isCiEnv(env = process.env) {
  const v = env.CI ?? env.GITHUB_ACTIONS ?? ''
  return v !== '' && v !== 'false' && v !== '0'
}

/** 「无 base 可判」在哪些场合算判红：显式传参，或本身就是自动化 */
export function requiresRange(args = argv, env = process.env) {
  return args.includes('--require-range') || isCiEnv(env)
}

function resolveRange() {
  const idx = argv.indexOf('--range')
  if (idx >= 0) return argv[idx + 1] || null
  if (argv.includes('--last')) return 'HEAD~1..HEAD'
  for (const base of ['origin/main', 'origin/master', 'main']) {
    const mb = git(['merge-base', base, 'HEAD'])
    if (mb && mb.trim()) return `${mb.trim()}..HEAD`
  }
  return null
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

  const range = resolveRange()
  if (!range) {
    // **无 git 事实可判 ≠ 判据成立**。浅克隆、`actions/checkout` 没拉 base、
    // detached HEAD 都会走到这里；退 0 等于这道闸在那些机器上**静默消失**，而日志里
    // 只有一行黄字。自动化里「没人判」必须等于「判不过」，否则 CI 的绿灯不代表任何东西。
    //
    // 判红条件是 `--require-range` **或** CI 变量：只靠传参就得有人记得拼，而 workflow
    // 是按脚本名跑门禁的，从没拼过它——那条为 CI 加的红线因此一直没被拉起来。
    // 本地（无 CI 变量）仍按「跳过，不当作通过」，不逼开发者去配 base ref。
    const forced = requiresRange()
    const msg =
      '⊘ core-change-audit：找不到可判的提交范围（无 origin/main 也无 --range）——跳过，不当作通过'
    if (forced) {
      console.error(
        red(
          `✗ core-change-audit：${msg.slice(1)}；${argv.includes('--require-range') ? '本次要求有可判范围' : '自动化环境（CI 变量）要求有可判范围'} ⇒ 判红`,
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
    console.log(green(`✓ core-change-audit：范围内没有提交（${range}）`))
    return 0
  }

  const subjects = subjectsOf(commits)
  const bad = []
  let coreTouching = 0
  let skipped = 0
  for (const { sha, message, files } of commits) {
    if (BASELINE_SHAS.has(sha.slice(0, 8))) {
      skipped += 1
      continue
    }
    if (inDirs(files, CORE_DIRS)) coreTouching += 1
    const r = judge(sha, files, message)
    if (r.errors.length) bad.push(r)
  }

  if (bad.length === 0) {
    console.log(
      green(
        `✓ core-change-audit：${commits.length} 个提交（追溯豁免 ${skipped}）里 ${coreTouching} 个触及 core/kernel，全部留了理由`,
      ),
    )
    return 0
  }

  console.log(red(`\ncore-change-audit 未通过：${bad.length} / ${commits.length} 个提交`))
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
