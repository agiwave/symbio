#!/usr/bin/env node
/**
 * extension-locality-audit — 新增能力只改自己目录（EX-002）
 *
 * ## 它治的病
 *
 * 「加一个能力要不要动架构」只有一个可判的答案：**四棵树的目录指纹不变**——
 * `crates/symbio-core` / `crates/symbio-kernel` / `crates/symbio-plugin-loop` /
 * `crates/symbio-plugin-session` 是能力**不该**碰的架构面。判据读指纹，不读
 * 「有没有新 crate」：新 crate 只是触发条件；一个提交完全可以新增能力包却顺手
 * 在 `loop` 里加一行按能力名分支，那正是 T1 要抓的形态。
 *
 * 既有守卫各自只看见一半：`SC-006` 判控制面里的工具名字面量，`CP-006` 判机制面
 * 指纹与端口闭集，`core-change-audit` 要求动 core/kernel 的提交留理由——没有一条
 * 判「这次新增能力有没有动控制面」。本判据补的是**提交的意图面**：同一次提交里
 * 既加能力包又改四棵树，只能说明机制补件夹带了（D-247 的「夹带」）。
 *
 * ## 两半
 *
 * | 半 | 判什么 |
 * |---|---|
 * | 正向证据 | 夹具提交 `77ff41fa`（`symbio-plugin-notes`，EX-001）：新增能力包的四棵树指纹不变。它不在判据范围内也要判——T1 的唯一实证 |
 * | 向前判据 | 范围内**新增 `crates/symbio-plugin-<名字>/PLUGIN.yml`** 的提交，逐个比较父提交与本提交的四棵树指纹 |
 *
 * ## 三处例外，各自有理由
 *
 * - **生成物**：已存在的 `*.gen.rs` 可以随能力包的清单变化（`rows.gen.rs` /
 *   `tool_namespaces.gen.rs` / `trigger_kinds.gen.rs`）。它是 `PLUGIN.yml` 的镜像，
 *   作者只改自己目录；生成器保证内容与清单一致（`GEN-001`）。**新出现**的
 *   `*.gen.rs` 不算例外——那是新增一条生成面，属机制补件。
 * - **迁移批次**（正文声明 `Migration:`，D-247）：**迁出**一条既有能力的提交必然在新增
 *   `PLUGIN.yml` 的同时删除 `loop` / `session` 里的旧实现——指纹不变反而说明旧实现没删
 *   （EX-003 要拦的「两份实现同时生效」）。这一类的形状归 `capability-migration-audit`
 *   （EX-003）判：架构树零非生成 diff、无能力名的新增行、至少删掉一行旧实现。本判据只对
 *   **没有** `Migration:` 声明的新增能力包守指纹——不声明的提交按「全新能力」论，照旧一格不许动。
 * - **判据之前的提交**：`BEFORE_GUARD` 里的提交不判（crate 拆分本身一次性触过四棵树）。
 *   名单逐条带理由；判据之后的提交只有上面两条退路。
 *
 * ## 失效形态（回归测试钉的是这几条）
 *
 * - 读不出指纹时最省事的实现是「没什么可判 ⇒ 通过」——故任何一棵树读不出都判红；
 * - 把 `.gen.rs` 一律豁免 ⇒ 手写一个新文件冒充生成物，故新出现的 `.gen.rs` 不豁免；
 * - 只判「有没有新 crate」⇒ 新增能力包夹带 `loop` 分支照样绿，故判定对象是四棵树。
 *
 * 用法：
 * ```bash
 * node scripts/extension-locality-audit.mjs --range origin/main..HEAD
 * node scripts/extension-locality-audit.mjs --last
 * node scripts/extension-locality-audit.mjs --require-range
 * node scripts/extension-locality-audit.mjs            # 自动取 merge-base(main, HEAD)..HEAD
 * ```
 * 退出码：0 = 通过（或本地无可判范围）；1 = 有违规，或自动化里无可判范围，或夹具读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns EX 能力全面拆分（名册处置与迁移边界）
// @codes EX-002

import { createHash } from 'node:crypto'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { green, red, yellow, dim } from './color.mjs'
import { MIGRATION_RE } from './capability-migration-audit.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const argv = process.argv.slice(2)

/** 能力不该碰的四棵树。 */
export const TREES = [
  'crates/symbio-core',
  'crates/symbio-kernel',
  'crates/symbio-plugin-loop',
  'crates/symbio-plugin-session',
]

/** 一个「新能力包」的判据：新增一份插件清单。 */
export const PACKAGE_MANIFEST_RE = /^crates\/symbio-plugin-[^/]+\/PLUGIN\.yml$/

/** 生成物：内容是清单的镜像，已存在的随清单改；新出现的不算。 */
const GENERATED_RE = /\.gen\.rs$/

/** 正向证据：扩展夹具的落地提交。范围里没有它也要判。 */
export const FIXTURE = {
  sha: '77ff41fa',
  why: 'symbio-plugin-notes 落地（扩展夹具）：新增能力包的四棵树指纹不变',
}

/** 判据诞生前的一次性搬家：不追溯已推送的历史。每条必须有理由。 */
export const BEFORE_GUARD = [
  {
    sha: '92cd0528',
    why: 'crate 拆分（D-191）：一次性把 crates/* 搬成插件一 crate，触及四棵树的是搬家本身，不是新增能力',
  },
]

/**
 * 豁免名单自身的形状判据：条目必须有 7–40 位提交号与非空理由。
 * 一条不解释自己的豁免就是永久后门。
 */
export function validateBeforeGuard(entries) {
  const errors = []
  for (const e of entries) {
    if (!e?.sha || !/^[0-9a-f]{7,40}$/.test(e.sha)) {
      errors.push(`${e?.sha ?? '(无 sha)'}：豁免条目必须有 7–40 位的提交号`)
      continue
    }
    if (typeof e.why !== 'string' || !e.why.trim()) {
      errors.push(`${e.sha}：豁免没有理由正文——一条不解释自己的豁免就是永久后门`)
    }
  }
  return errors
}

/**
 * 迁移批次的自声明（D-247）：正文一行 `Migration: <批次>`。这类提交里四棵树的变化是
 * 迁移的**删除侧**——新包接管旧实现时，`loop` / `session` 的旧代码同批退场，指纹必然变；
 * 删得对不对归 `capability-migration-audit`（EX-003）判。本判据不重复判，只守
 * **没有**这条声明的新增能力包。
 */
export function isMigrationBatch(message) {
  return MIGRATION_RE.test(String(message ?? ''))
}

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
const PRETTY = `%x01%H%x00%B`
/**
 * 文件记录的正则：两个定长十六进制模式 + 两个对象号 + 状态 + 制表符 + 路径。
 * 不按行首的 `:` 判——正文里以 `:` 开头的行会把提交信息拦腰截断。
 */
const RAW_FILE_LINE_RE = /^:[0-7]{6} [0-7]{6} /
const RAW_FILE_RE = /^:[0-7]{6} [0-7]{6} [0-9a-f]+ [0-9a-f]+ ([A-Z][0-9]*)\t(.*)$/

/**
 * 解析 `git log --raw --pretty=format:<REC>%H%x00%B`。
 * 返回每个提交的 `{ sha, message, files: [{ status, path }] }`。
 */
export function parseRawCommits(out) {
  const commits = []
  for (const rec of String(out).split(REC)) {
    if (!rec.trim()) continue
    const nul = rec.indexOf('\x00')
    if (nul < 0) continue
    const sha = rec.slice(0, nul).trim()
    const lines = rec.slice(nul + 1).split('\n')
    const firstFile = lines.findIndex((l) => RAW_FILE_LINE_RE.test(l))
    const message = (firstFile < 0 ? lines : lines.slice(0, firstFile)).join('\n')
    const files = []
    if (firstFile >= 0) {
      for (const line of lines.slice(firstFile)) {
        const m = line.match(RAW_FILE_RE)
        if (m && m[2].trim()) files.push({ status: m[1], path: m[2].trim() })
      }
    }
    commits.push({ sha, message, files })
  }
  return commits
}

/** 本提交新增的能力包清单（`A` 状态 + 插件目录下的 `PLUGIN.yml`）。 */
export function addedPackages(commit) {
  return (commit.files ?? [])
    .filter((f) => f.status === 'A' && PACKAGE_MANIFEST_RE.test(f.path))
    .map((f) => f.path)
}

/**
 * 四棵树的目录指纹。
 *
 * `entries` 是 `git ls-tree -r` 的结果（`{ path, oid }`）；`parentPaths` 是父提交的
 * 全量路径集合，用来只豁免**已存在**的生成物。指纹按路径排序后拼接，与输入顺序无关。
 */
export function fingerprints(entries, parentPaths = new Set()) {
  const buckets = new Map(TREES.map((t) => [t, []]))
  for (const { path: p, oid } of entries) {
    if (!p || !oid) continue
    if (GENERATED_RE.test(p) && parentPaths.has(p)) continue
    const tree = TREES.find((t) => p.startsWith(`${t}/`))
    if (!tree) continue
    buckets.get(tree).push([p, oid])
  }
  const out = {}
  for (const tree of TREES) {
    const body = buckets
      .get(tree)
      .sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0))
      .map(([p, oid]) => `${p}\x00${oid}\n`)
      .join('')
    out[tree] = createHash('sha256').update(body).digest('hex').slice(0, 16)
  }
  return out
}

/**
 * 纯判定：新增了能力包的提交，四棵树的指纹必须逐一相等。
 *
 * 任何一棵树的指纹读不出也算红——判据没有输入时，「没有违规」是最坏的一种绿。
 */
export function judgeNewCapability({ sha, added, before, after }) {
  const errors = []
  if (!added || added.length === 0) return { sha, errors }
  for (const tree of TREES) {
    const b = before?.[tree]
    const a = after?.[tree]
    if (typeof b !== 'string' || typeof a !== 'string') {
      errors.push(
        `EX-002 读不出 \`${tree}\` 的目录指纹 —— 判据没有输入，宁红不静默（历史不完整时先 \`git fetch --unshallow\`）`,
      )
      continue
    }
    if (b !== a) {
      errors.push(
        `EX-002 新增能力包（${added.map((p) => `\`${p}\``).join(' / ')}）同时改了 \`${tree}\` 的目录指纹（${b} → ${a}）` +
          ' —— 新能力只能改自己目录；机制补件必须拆成独立提交并挂 `### D-NNN`（D-247），补件落地后再提交能力包',
      )
    }
  }
  return { sha, errors }
}

function treeEntries(rev) {
  const out = git(['ls-tree', '-r', rev, '--', ...TREES])
  if (out === null) return null
  const entries = []
  for (const line of out.split('\n')) {
    const tab = line.indexOf('\t')
    if (tab < 0) continue
    const meta = line.slice(0, tab).split(' ')
    const p = line.slice(tab + 1).trim()
    if (!p || meta.length < 3) continue
    entries.push({ path: p, oid: meta[2] })
  }
  return entries
}

function judgeCommit(sha, added, subject) {
  const before = treeEntries(`${sha}^`)
  const after = treeEntries(sha)
  if (before === null || after === null) {
    return {
      sha,
      subject,
      errors: [`EX-002 读不出 ${sha}（或其父提交）的四棵目录树 —— 判据没有输入，宁红不静默`],
    }
  }
  const parentPaths = new Set(before.map((e) => e.path))
  return {
    ...judgeNewCapability({
      sha,
      added,
      before: fingerprints(before, parentPaths),
      after: fingerprints(after, parentPaths),
    }),
    subject,
  }
}

function subjectOf(message) {
  return String(message ?? '').split('\n')[0] ?? ''
}

function isCiEnv(env = process.env) {
  const v = env.CI ?? env.GITHUB_ACTIONS ?? ''
  return v !== '' && v !== 'false' && v !== '0'
}

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

function reportFailures(bad, subjects) {
  console.log(red(`\nextension-locality-audit 未通过：${bad.length} 个提交`))
  for (const r of bad) {
    console.log(`\n  ${r.sha.slice(0, 8)} ${r.subject || subjects.get(r.sha.slice(0, 7)) || ''}`)
    for (const e of r.errors) console.log(`    ${e}`)
  }
  console.log(
    dim(
      '\n  判据是「新增能力包的四棵树指纹不变」。红了通常意味着两件事之一：' +
        '\n  ① 机制补件应当独立成一次提交（带 `### D-NNN`），再让能力包进来；' +
        '\n  ② 新出现的 `*.gen.rs` 是一条新生成面，也属机制补件。',
    ),
  )
}

function main() {
  const baselineErrors = validateBeforeGuard(BEFORE_GUARD)
  if (baselineErrors.length) {
    for (const e of baselineErrors) console.error(red(`✗ 豁免名单：${e}`))
    return 1
  }

  // ── 正向证据：夹具提交（与范围无关）──
  if (git(['rev-parse', '--verify', `${FIXTURE.sha}^{commit}`]) === null) {
    console.error(
      red(
        `✗ extension-locality-audit：夹具提交 ${FIXTURE.sha} 打不开（浅克隆？先 \`git fetch --unshallow\`）` +
          ' —— T1 的正向证据读不到，判据等于没跑',
      ),
    )
    return 1
  }
  const fixtureLog = git(['log', '-1', '--raw', '--no-renames', `--pretty=format:${PRETTY}`, FIXTURE.sha])
  const [fixtureCommit] = parseRawCommits(fixtureLog ?? '')
  if (!fixtureCommit) {
    console.error(red(`✗ extension-locality-audit：夹具提交 ${FIXTURE.sha} 的改动读不出`))
    return 1
  }
  const fixtureAdded = addedPackages(fixtureCommit)
  if (fixtureAdded.length === 0) {
    console.error(
      red(
        `✗ extension-locality-audit：夹具提交 ${FIXTURE.sha} 没有新增能力包 —— 夹具指错了对象（${FIXTURE.why}）`,
      ),
    )
    return 1
  }
  const fixtureResult = judgeCommit(fixtureCommit.sha, fixtureAdded, FIXTURE.why)
  if (fixtureResult.errors.length) {
    reportFailures([fixtureResult], new Map())
    return 1
  }
  const fixtureFull = fixtureCommit.sha

  // ── 向前判据：范围内的新增能力包提交 ──
  const range = resolveRange()
  if (!range) {
    const msg =
      '⊘ extension-locality-audit：找不到可判的提交范围（无 origin/main 也无 --range）——跳过，不当作通过'
    if (requiresRange()) {
      console.error(
        red(
          `✗ extension-locality-audit：${msg.slice(1)}；${argv.includes('--require-range') ? '本次要求有可判范围' : '自动化环境（CI 变量）要求有可判范围'} ⇒ 判红`,
        ),
      )
      return 1
    }
    console.log(yellow(msg))
    return 0
  }

  const log = git([
    'log',
    '--no-merges',
    '--reverse',
    '--raw',
    '--no-renames',
    `--pretty=format:${PRETTY}`,
    range,
  ])
  if (log === null) {
    console.error(red(`✗ extension-locality-audit：解析提交范围失败：${range}`))
    return 1
  }
  const commits = parseRawCommits(log)
  const subjects = new Map(commits.map((c) => [c.sha.slice(0, 7), subjectOf(c.message)]))
  const exempt = new Set(BEFORE_GUARD.map((e) => e.sha))
  const candidates = commits.filter(
    (c) =>
      c.sha !== fixtureFull &&
      !exempt.has(c.sha.slice(0, 8)) &&
      addedPackages(c).length > 0 &&
      !isMigrationBatch(c.message),
  )

  const bad = []
  for (const c of candidates) {
    const r = judgeCommit(c.sha, addedPackages(c), subjectOf(c.message))
    if (r.errors.length) bad.push(r)
  }
  if (bad.length) {
    reportFailures(bad, subjects)
    return 1
  }

  console.log(
    green(
      `✓ extension-locality-audit：夹具 ${FIXTURE.sha}（${fixtureAdded.join(' / ')}）` +
        `+ 范围内 ${candidates.length} 个新增能力包提交，四棵树指纹不变`,
    ),
  )
  return 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(main())
}
