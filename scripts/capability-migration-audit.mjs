#!/usr/bin/env node
/**
 * capability-migration-audit — 迁移批次只搬运（EX-003 / EX-007）
 *
 * ## 它治的病
 *
 * 拆分期最贵的走样不是「搬得慢」，而是**红绿不再说明任何事**：一次提交里既搬走
 * 一条能力、又顺手在 `loop` 加一行 `Decl`，端到端用例照旧全绿，于是没人知道那条
 * 用例证明的是「搬运可行」还是「新机制在兜底」。D-247 因此把提交分成两类，本判据
 * 判的就是**这两类各自有没有越界**。
 *
 * | 类别 | 怎么认 | 判什么 |
 * |---|---|---|
 * | **迁移批次** | 提交正文里一行 `Migration: <批次>` | `core` / `kernel` / `plugin-api` 的非生成 `.rs` diff = 0；`loop` / `session` 没有新增**带能力名**的源码行（绑定切换可以是通用胶水，为某条能力新加的行不行）；且至少删掉一行旧实现 |
 * | **架构补件** | 在架构面新增 `pub` 项，且没有 `Migration:` | 引用一条**能打开**的 `### D-NNN`；新增的非注释代码不含插件 id 字符串、不含工具名 |
 *
 * 两类之外（只改注释、只删架构面、只动文档）不判——判据不假装能判它没定义的东西。
 *
 * ## 为什么必须显式声明，而不是自动猜
 *
 * 「这次提交是迁移还是补件」是一次提交的**意图**，不是某个文件的属性——同一个
 * 删除行，在迁移批次里是「搬走」，在补件里是「换掉一个不通用的小口子」。这与
 * `core-change-audit` 的 `Core-Change:` 同一种形状：提交者声明类别，判据判声明
 * 是否成立。没有声明时宁可漏判，也不猜——猜错的判据会逼人写假声明。
 *
 * ## 两半的分工
 *
 * - **EX-003（T2）**：`旧实现已删（同提交）` 这一半在本脚本——迁移批次在 `loop` /
 *   `session` 的删除行必须非零，且新增行不得带能力名（为某条能力新加一行，意味着
 *   两份实现可以同时生效；绑定切换用的通用胶水不算）。`两份实现同时生效` 的**装配期**
 *   另一半归 `seal` 的 `Gap::Duplicate{Name,Event,Fact,View,Actor,Port}`
 *   （`crates/symbio-kernel/src/decl/ring.rs`）：同名声明在装载期红，
 *   不在这里按名字再判一遍（SC-006 的教训：同一形状两个 owner）。
 * - **EX-007（D-247）**：夹带、定制、反向拆分三条失败形态——架构面零改动是「反向拆分」
 *   与「夹带」的机械面；补件的决策号与通用性判「定制」。语义是否真的通用（不是把
 *   一条能力改名成 trait）由 review 判，脚本只保证「写着能力名 / 工具名」无法通过。
 *
 * ## 失效形态（回归测试钉的是这几条）
 *
 * - 把「有 `Migration:`」当成「不用判」⇒ 只要写上 trailer 就能夹带，判据变装饰；
 * - 只判新增、不判删除 ⇒ 插件落地而旧实现留着（两份同时生效）照样绿；
 * - 注释里的工具名 / 插件 id 被算成代码 ⇒ 补件几乎每条都红，豁免写满之后没人再看；
 * - 决策号只做正则不查存在 ⇒ 拼一个不存在的 `D-999` 就能进架构。
 *
 * 用法：
 * ```bash
 * node scripts/capability-migration-audit.mjs --range origin/main..HEAD
 * node scripts/capability-migration-audit.mjs --last
 * node scripts/capability-migration-audit.mjs --require-range
 * node scripts/capability-migration-audit.mjs            # 自动取 merge-base(main, HEAD)..HEAD
 * ```
 * 退出码：0 = 通过（或本地无可判范围）；1 = 有违规，或自动化里无可判范围，或夹具读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns EX 能力全面拆分（名册处置与迁移边界）
// @codes EX-003 EX-007

import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { green, red, yellow, dim } from './color.mjs'
import { blankComments } from './rust-scan.mjs'
import { toolFacts } from './facts/_capability.mjs'
import { pluginIds } from './facts/_plugin-yml.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const argv = process.argv.slice(2)

/** 机制面：迁移批次完全不该碰的三棵树（`plugin-api` 也在内——补件才改它）。 */
export const ARCH_TREES = [
  'crates/symbio-core',
  'crates/symbio-kernel',
  'crates/symbio-plugin-api',
]

/** 控制面：迁移批次只许在这里删旧实现。 */
export const CONTROL_TREES = ['crates/symbio-plugin-loop', 'crates/symbio-plugin-session']

/** 迁移批次的声明：提交正文里一行 `Migration: <批次>`。 */
export const MIGRATION_RE = /^\s*Migration[:：]\s*(\S.*)$/m

/** `Core-Change:` 段是补件引用的决策号所在处（与 `core-change-audit` 同一格式）。 */
const CORE_CHANGE_RE = /^\s*Core-Change[:：]\s*(.+)$/m

/** 生成物：清单 / 装配的镜像，随能力包增删；它不是机制（D-247）。 */
const GENERATED_RE = /\.gen\.rs$/

/**
 * 生成物清单：`scripts/facts/` 各描述符声明的输出。
 *
 * 生成物不都叫 `*.gen.rs`——`plugin-api` 的 `view_names.rs` / `events.rs` /
 * `routes.rs` 是插件清单的投影，随能力包增删而变。按名字猜会把「加一条能力顺带
 * 重生成登记表」判成机制增量；只认后缀则这三份漏网，补件几乎条条假红。清单直接
 * 读描述符的 `OUT_REL`：那是生成器的唯一注册处（`scripts/gen-facts.mjs` 目录即接线），
 * 不在这里抄第二份路径。
 */
export function generatedPaths(root) {
  const out = new Set()
  const dir = path.join(root, 'scripts', 'facts')
  let entries = []
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    if (!e.isFile() || !e.name.endsWith('.mjs') || e.name.startsWith('_') || e.name.endsWith('.test.mjs')) continue
    const m = fs.readFileSync(path.join(dir, e.name), 'utf8').match(/const\s+OUT_REL\s*=\s*['"]([^'"]+)['"]/)
    if (m) out.add(m[1])
  }
  return out
}

/** 本仓的生成物清单（模块加载时读一次；描述符是唯一注册处）。 */
const GENERATED = generatedPaths(repoRoot)

/** 测试文件：迁移批次可能改测试而机制不变（断言跟着搬）。 */
const TEST_RE = /\.test\.rs$/

/**
 * 判据诞生之前的提交不追溯。此前 868 个提交按旧纪律写，追溯的唯一效果是逼人
 * 重写历史——那不是收紧，是制造一次不可复核的改写。名单只有一条，且必须是
 * 「判据落地前的最后一个提交」：判据生效的提交是它的子提交。
 */
export const BEFORE_GUARD = {
  sha: '811b2e7d',
  why: 'EX-002 落地时的 HEAD：迁移纪律（D-247）从此后的提交开始判，之前的不追溯（不重写历史）',
}

/** 补件的正向证据：给 `plugin-api` 加前置判断端口的那次提交（D-219 / D-220）。 */
export const FIXTURE = {
  sha: '709ee583',
  why: '前置判断端口落地（D-219 / D-220）：补件带一条能打开的决策，新增面不含插件 id 与工具名',
}

/**
 * 补件次数的告警阈值（D-269 §5.3）。
 *
 * 它不是棘轮：计数随「判据起点..HEAD」的范围变化，拿它当基线会在换基线时假红。
 * 但它必须刺眼——**补件次数是设计缺口的读数**。阈值取 2：一次补件可以是真的位置缺失，
 * 连着两次就是「每一批都在补架构」，那时该做的是 §5.1 的站位表复核，不是继续加面。
 */
export const SUPPLEMENT_ALARM = 2

/** 架构面「新增一个公开项」的形状——机制增量的机械面。 */
const PUB_ITEM_RE =
  /^\s*pub(?:\([^)]*\))?\s+(?:trait|struct|enum|fn|type|const|static|mod|use|async\s+fn|unsafe\s+fn|extern\s+fn)\b/

/** 非测试、非生成的 Rust 源码：迁移期唯一该变少的文件类。 */
export function isCapabilitySource(p) {
  return p.endsWith('.rs') && !TEST_RE.test(p) && !isGenerated(p)
}

/** 生成物：后缀一类的 `*.gen.rs`，加上描述符声明的输出（`view_names.rs` 等）。 */
export function isGenerated(p, generated = GENERATED) {
  return GENERATED_RE.test(p) || generated.has(p)
}

function inTrees(p, trees) {
  return trees.some((t) => p.startsWith(`${t}/`))
}

/**
 * 解析 `git show --unified=0` 的补丁：按文件给出新增 / 删除行。
 *
 * 文件边界只认 `diff --git` 之后的第一对 `---` / `+++`：一行内容恰好以 `--- ` 开头
 * （删掉一行 `-- …` 的 diff 行就是 `--- …`）不会被当成文件头；新增文件的
 * `--- /dev/null` 与删除文件的 `+++ /dev/null` 各取另一侧作路径，否则整份新文件的
 * 增行会挂到上一个文件上——报错点名错文件，比不报更贵。
 */
export function parsePatch(text) {
  const files = []
  let cur = null
  let oldPath = null
  let header = false
  let inHunk = false
  for (const line of String(text).split('\n')) {
    if (line.startsWith('diff --git ')) {
      cur = null
      oldPath = null
      header = true
      inHunk = false
      continue
    }
    if (header && line.startsWith('--- ')) {
      const p = line.slice(4).trim()
      oldPath = p === '/dev/null' ? null : p.replace(/^a\//, '')
      continue
    }
    if (header && line.startsWith('+++ ')) {
      const p = line.slice(4).trim()
      const path = p === '/dev/null' ? oldPath : p.replace(/^b\//, '')
      cur = path ? { path, added: [], removed: [] } : null
      if (cur) files.push(cur)
      header = false
      oldPath = null
      continue
    }
    if (line.startsWith('@@')) {
      inHunk = true
      continue
    }
    if (!inHunk || !cur) continue
    if (line.startsWith('+')) cur.added.push(line.slice(1))
    else if (line.startsWith('-')) cur.removed.push(line.slice(1))
  }
  return files
}

/**
 * 补件在架构面新增的公开项（前几项，供报错点名）。
 *
 * 只看**新增的非注释行**：注释里提一句 `pub fn` 不是机制。
 */
export function mechanismAdditions(patch) {
  const out = []
  for (const file of patch) {
    if (!inTrees(file.path, ARCH_TREES) || !isCapabilitySource(file.path)) continue
    for (const line of file.added) {
      if (PUB_ITEM_RE.test(line)) out.push({ path: file.path, line: line.trim() })
    }
  }
  return out
}

/**
 * 新增行里的能力名：插件 id 的字符串字面量、或任一工具名。
 *
 * 判的是**代码里的名字**，注释不算（`// notes 曾经在这里分支` 是叙述，不是通道）。
 * 迁移与补件两侧共用这一把尺：迁移侧判「有没有为某条能力新加一行」，补件侧判
 * 「这条通用口子有没有为某条能力定制」。
 */
export function capabilityNameHits(file, ids = [], toolNames = []) {
  const out = []
  for (const raw of file.added) {
    if (!raw.trim()) continue
    const code = blankComments(raw)[0] ?? ''
    for (const id of ids) {
      if (code.includes(`"${id}"`)) out.push({ path: file.path, line: raw.trim(), what: `插件 id \`"${id}"\`` })
    }
    for (const name of toolNames) {
      const re = new RegExp(`\\b${name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`)
      if (re.test(code)) out.push({ path: file.path, line: raw.trim(), what: `工具名 \`${name}\`` })
    }
  }
  return out
}

function describeHit(h) {
  return `${h.path}: ${h.what} —— ${h.line.slice(0, 80)}`
}

/** 提交类别：有 `Migration:` 一律是迁移批次；否则新增了公开项才是补件；其余不判。 */
export function classifyCommit({ message, patch }) {
  if (MIGRATION_RE.test(String(message ?? ''))) return 'migration'
  if (mechanismAdditions(patch).length > 0) return 'supplement'
  return null
}

/**
 * 迁移批次的判定（EX-003 的「同提交删除」+ EX-007 的两条 containment）。
 */
export function migrationErrors({ sha, patch, pluginIds: ids = [], toolNames = [] }) {
  const errors = []

  const archTouched = patch.filter(
    (f) => inTrees(f.path, ARCH_TREES) && f.path.endsWith('.rs') && !isGenerated(f.path) && (f.added.length || f.removed.length),
  )
  if (archTouched.length) {
    errors.push(
      `EX-007 迁移提交改了机制面（${archTouched.map((f) => `\`${f.path}\``).join(' / ')}）` +
        ' —— 迁移批次 core / kernel / plugin-api diff 必须为 0；确有机制增量就拆成带 `### D-NNN` 的独立补件提交（D-247）',
    )
  }

  const control = patch.filter((f) => inTrees(f.path, CONTROL_TREES) && isCapabilitySource(f.path))
  const named = control.flatMap((f) => capabilityNameHits(f, ids, toolNames))
  if (named.length) {
    errors.push(
      `EX-007 迁移提交在控制面新增了 ${named.length} 行带能力名的源码（如 ${describeHit(named[0])}）` +
        ' —— 绑定切换可以是通用胶水，为某条能力新加行 / 分支 / 常量不行：那会让两份实现同时生效',
    )
  }

  const deletions = control.reduce((n, f) => n + f.removed.length, 0)
  if (deletions === 0) {
    errors.push(
      'EX-003 迁移提交没有删除 `loop` / `session` 的非测试源码——插件落地与旧实现删除必须同提交（D-245），否则两份实现同时生效',
    )
  }

  return { sha, errors }
}

/**
 * 补件的判定（EX-007 的决策号与通用性）。
 *
 * 决策号取 `Core-Change:` 段；没有这一段（只动 `plugin-api` 的补件不要求它）则取
 * 整条信息里任意一个 `D-NNN`。**必须能打开**：拼一个不存在的号等于没写。
 */
export function supplementErrors({ sha, message, patch, decisions, pluginIds: ids = [], toolNames = [] }) {
  const errors = []
  const text = String(message ?? '')
  const trailer = text.match(CORE_CHANGE_RE)
  const scope = trailer ? trailer[1] : text
  const cited = [...scope.matchAll(/\bD-\d{3,}\b/g)].map((m) => m[0])
  if (cited.length === 0) {
    errors.push(
      'EX-007 补件没有决策号 —— 架构增量必须挂一条 `Core-Change: D-NNN`，说清哪条能力因缺它而迁不动、它服务哪些能力',
    )
  } else if (!cited.some((d) => decisions.has(d))) {
    errors.push(
      `EX-007 补件引用的决策号在 docs/003-design/ 里打不开（${[...new Set(cited)].join(' / ')}）—— 写错的号等于没写`,
    )
  }

  const offenders = []
  for (const file of patch) {
    if (!inTrees(file.path, ARCH_TREES) || !isCapabilitySource(file.path)) continue
    offenders.push(...capabilityNameHits(file, ids, toolNames))
  }
  if (offenders.length) {
    errors.push(
      `EX-007 补件不是通用的（${offenders.length} 处带能力名 / 工具名，如 ${describeHit(offenders[0])}）` +
        ' —— 补件至少服务一条真实能力，但不是为某一条能力定制的通道（D-247）',
    )
  }

  return { sha, errors }
}

/**
 * 豁免名单自身的形状：必须是 7–40 位提交号 + 非空理由。一条不解释自己的豁免
 * 就是永久后门。
 */
export function validateBeforeGuard(entry) {
  const errors = []
  if (!entry?.sha || !/^[0-9a-f]{7,40}$/.test(entry.sha)) {
    errors.push(`${entry?.sha ?? '(无 sha)'}：判据起点必须有 7–40 位提交号`)
  }
  if (typeof entry?.why !== 'string' || !entry.why.trim()) {
    errors.push(`${entry?.sha ?? '(无 sha)'}：判据起点没有理由——不解释起点的豁免就是永久后门`)
  }
  return errors
}

/** `docs/003-design/` 里存在的决策号（与 core-change-audit 同一口径）。 */
export function knownDecisions(root) {
  const out = new Set()
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
  walk(path.join(root, 'docs', '003-design'))
  return out
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
const RAW_FILE_LINE_RE = /^:[0-7]{6} [0-7]{6} /

/** 与 extension-locality-audit 同一形状：`git log --raw` 的记录解析。 */
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

function patchOf(sha) {
  const out = git([
    'show',
    '--format=',
    '--unified=0',
    '--no-renames',
    sha,
    '--',
    ...ARCH_TREES,
    ...CONTROL_TREES,
  ])
  return out === null ? null : parsePatch(out)
}

function touchesAuditedTree(files) {
  return files.some((p) => (inTrees(p, ARCH_TREES) || inTrees(p, CONTROL_TREES)) && p.endsWith('.rs'))
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

function reportFailures(bad) {
  console.log(red(`\ncapability-migration-audit 未通过：${bad.length} 个提交`))
  for (const r of bad) {
    console.log(`\n  ${r.sha.slice(0, 8)} ${r.subject || ''}`)
    for (const e of r.errors) console.log(`    ${e}`)
  }
  console.log(
    dim(
      '\n  两类提交各有一条退路：迁移批次只删不增；机制增量拆成独立补件，并挂一条能打开的 `### D-NNN`。',
    ),
  )
}

function judgeCommit({ sha, message, patch, decisions, ids, tools }) {
  const kind = classifyCommit({ message, patch })
  if (kind === 'migration') {
    return {
      sha,
      subject: subjectOf(message),
      kind,
      errors: migrationErrors({ sha, patch, pluginIds: ids, toolNames: tools }).errors,
    }
  }
  if (kind === 'supplement') {
    return {
      sha,
      subject: subjectOf(message),
      kind,
      errors: supplementErrors({ sha, message, patch, decisions, pluginIds: ids, toolNames: tools }).errors,
    }
  }
  return { sha, subject: subjectOf(message), kind, errors: [] }
}

function main() {
  const guardErrors = validateBeforeGuard(BEFORE_GUARD)
  if (guardErrors.length) {
    for (const e of guardErrors) console.error(red(`✗ 判据起点：${e}`))
    return 1
  }

  const decisions = knownDecisions(repoRoot)
  if (decisions.size === 0) {
    console.error(red('✗ capability-migration-audit：docs/003-design/ 里一个 `### D-NNN` 都读不到——补件的资格线没有输入'))
    return 1
  }
  const ids = pluginIds(repoRoot)
  const tools = [...toolFacts(repoRoot).keys()]

  // ── 正向证据：补件夹具（与范围无关）──
  if (git(['rev-parse', '--verify', `${FIXTURE.sha}^{commit}`]) === null) {
    console.error(red(`✗ capability-migration-audit：夹具提交 ${FIXTURE.sha} 打不开（浅克隆？先 \`git fetch --unshallow\`）`))
    return 1
  }
  const fixtureMsg = git(['log', '-1', '--format=%B', FIXTURE.sha])
  const fixturePatch = patchOf(FIXTURE.sha)
  if (fixtureMsg === null || fixturePatch === null) {
    console.error(red(`✗ capability-migration-audit：夹具提交 ${FIXTURE.sha} 的 diff 读不出`))
    return 1
  }
  const fixture = judgeCommit({
    sha: FIXTURE.sha,
    message: fixtureMsg,
    patch: fixturePatch,
    decisions,
    ids,
    tools,
  })
  if (fixture.kind !== 'supplement') {
    console.error(
      red(`✗ capability-migration-audit：夹具提交 ${FIXTURE.sha} 没有被认成补件——夹具指错了对象（${FIXTURE.why}）`),
    )
    return 1
  }
  if (fixture.errors.length) {
    reportFailures([fixture])
    return 1
  }

  // ── 向前判据：判据起点之后、提交范围之内的提交 ──
  const range = resolveRange()
  if (!range) {
    const msg = '⊘ capability-migration-audit：找不到可判的提交范围（无 origin/main 也无 --range）——跳过，不当作通过'
    if (requiresRange()) {
      console.error(
        red(
          `✗ capability-migration-audit：${msg.slice(1)}；${argv.includes('--require-range') ? '本次要求有可判范围' : '自动化环境（CI 变量）要求有可判范围'} ⇒ 判红`,
        ),
      )
      return 1
    }
    console.log(yellow(msg))
    return 0
  }

  const log = git(['log', '--no-merges', '--reverse', '--raw', '--no-renames', `--pretty=format:${PRETTY}`, range])
  if (log === null) {
    console.error(red(`✗ capability-migration-audit：解析提交范围失败：${range}`))
    return 1
  }
  const forwardOut = git(['rev-list', '--no-merges', `${BEFORE_GUARD.sha}..HEAD`])
  if (forwardOut === null) {
    console.error(red(`✗ capability-migration-audit：读不出判据起点之后的提交（${BEFORE_GUARD.sha}..HEAD）`))
    return 1
  }
  const forward = new Set(forwardOut.split('\n').filter(Boolean))
  const commits = parseRawCommits(log).filter((c) => forward.has(c.sha))

  const bad = []
  let migrations = 0
  let supplements = 0
  for (const c of commits) {
    if (!touchesAuditedTree(c.files)) continue
    const patch = patchOf(c.sha)
    if (patch === null) {
      bad.push({ sha: c.sha, subject: subjectOf(c.message), errors: ['读不出本提交的补丁——判据没有输入，宁红不静默'] })
      continue
    }
    const r = judgeCommit({ sha: c.sha, message: c.message, patch, decisions, ids, tools })
    if (r.kind === 'migration') migrations += 1
    if (r.kind === 'supplement') supplements += 1
    if (r.errors.length) bad.push(r)
  }
  if (bad.length) {
    reportFailures(bad)
    return 1
  }

  console.log(
    green(
      `✓ capability-migration-audit：夹具 ${FIXTURE.sha}（补件）` +
        `+ 判据起点 ${BEFORE_GUARD.sha} 之后 ${migrations} 个迁移批次 / ${supplements} 个补件，全部只搬运`,
    ),
  )
  if (migrations === 0) {
    console.log(
      dim('  ⊘ 还没有迁移批次（S6 起生效）：本轮的读数只是「补件夹具成立」，不证明任何一批拆得动'),
    )
  }
  // 补件次数是**设计缺口的读数**（D-269 §5.3）：不设棘轮（范围一变计数就变，基线会假红），
  // 但它必须刺眼——补件与迁移批次同数，说明每一批都在补架构，而不是在搬能力。
  if (supplements >= SUPPLEMENT_ALARM) {
    console.log(
      yellow(
        `  △ 本段补件 ${supplements} 次 / 迁移批次 ${migrations} 个：补件次数是**设计缺口**的读数` +
          '（D-269 §5.3）。每一条先过 §5.1 的两条准入（不提能力名 ∧ ≥2 条能力要用），' +
          '过不了的记进 §5.2 缺口账、结论写「留控制面」，不改架构。',
      ),
    )
  }
  return 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(main())
}
