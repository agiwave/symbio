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
 * ## 判据的语料来自 git 自己
 *
 * 不维护「哪些路径算 core」的清单：`git diff-tree --name-only` 给真实改动文件，
 * 目录常量只有两条（`crates/symbio-core/` / `crates/symbio-kernel/`），与
 * `core-shape-audit` 的 `core-kernel-files:` 标记同源。新加一个 crate 不需要改本脚本。
 *
 * ## 用法
 *
 * ```bash
 * node scripts/core-change-audit.mjs --range origin/main..HEAD   # CI
 * node scripts/core-change-audit.mjs --last                     # 只判 HEAD
 * node scripts/core-change-audit.mjs                            # 自动取 merge-base(origin/main, HEAD)..HEAD
 * ```
 *
 * 退出码：0 = 通过（或无可判范围）；1 = 有不符合的提交。
 */

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
 * 追溯豁免：**判据生效之前**的提交不追改历史。
 *
 * 一条判据从「今天」开始生效，只能对之后的提交要求；把两个历史提交也判红，唯一效果是
 * 逼人去 `--amend` 重写已经推上去的历史——那不是收紧，是制造一次不可复核的改写。
 *
 * 这里只放**判据诞生之前**的提交；新增条目需要写清为什么（否则它就成了永久后门）。
 */
const BASELINE_SHAS = new Set([
  // 92cd0528：拆 crate（D-191）本身——判据诞生于它之后，它不可能自带 Core-Change。
  '92cd0528',
  // 252e67a6：让拆 crate 后的守卫重新看得见真实仓（门禁 142/144）——同属判据之前。
  '252e67a6',
])

function git(args) {
  const r = spawnSync('git', args, { cwd: repoRoot, encoding: 'utf8', shell: false })
  if (r.status !== 0) return null
  return r.stdout
}

/** 某提交改了哪些文件（不含 rename 的源路径噪声） */
function filesOf(sha) {
  const out = git(['diff-tree', '--no-commit-id', '--name-only', '-r', '--no-renames', sha])
  if (out === null) return null
  return out.split('\n').map((s) => s.trim()).filter(Boolean)
}

function messageOf(sha) {
  const out = git(['log', '-1', '--pretty=%B', sha])
  return out === null ? '' : out
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

function subjectsOf(shas) {
  const out = git(['log', '--no-walk', '--pretty=%h %s', ...shas])
  const map = new Map()
  if (!out) return map
  for (const line of out.split('\n')) {
    const [h, ...rest] = line.split(' ')
    if (h) map.set(h, rest.join(' '))
  }
  return map
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
  const range = resolveRange()
  if (!range) {
    console.log(
      yellow('⊘ core-change-audit：找不到可判的提交范围（无 origin/main 也无 --range）——跳过，不当作通过'),
    )
    return 0
  }

  const listed = git(['rev-list', '--no-merges', '--reverse', range])
  if (listed === null) {
    console.error(red(`✗ 解析提交范围失败：${range}`))
    return 1
  }
  const shas = listed.split('\n').map((s) => s.trim()).filter(Boolean)
  if (shas.length === 0) {
    console.log(green(`✓ core-change-audit：范围内没有提交（${range}）`))
    return 0
  }

  const subjects = subjectsOf(shas)
  const bad = []
  let coreTouching = 0
  let skipped = 0
  for (const sha of shas) {
    if (BASELINE_SHAS.has(sha.slice(0, 8))) {
      skipped += 1
      continue
    }
    const files = filesOf(sha)
    if (files === null) continue
    if (inDirs(files, CORE_DIRS)) coreTouching += 1
    const r = judge(sha, files, messageOf(sha))
    if (r.errors.length) bad.push(r)
  }

  if (bad.length === 0) {
    console.log(
      green(
        `✓ core-change-audit：${shas.length} 个提交（追溯豁免 ${skipped}）里 ${coreTouching} 个触及 core/kernel，全部留了理由`,
      ),
    )
    return 0
  }

  console.log(red(`\ncore-change-audit 未通过：${bad.length} / ${shas.length} 个提交`))
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
