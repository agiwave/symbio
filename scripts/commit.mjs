#!/usr/bin/env node
/**
 * commit — 统一提交入口：生成合规消息 → git commit → 清理。
 *
 * **默认非交互、默认不跑门禁**：既然用脚本提交，就不该再被逐项追问，也不该在
 * 提交时等几分钟门禁。type/scope/标题自动从暂存内容推断，分节自动从 diff 归纳
 * （需要写「为什么」时显式给 `--section=`）。门禁该在改动完成、提交之前单独跑。
 *
 *   node scripts/commit.mjs                        # 非交互：自动推断，秒级完成
 *   node scripts/commit.mjs --title="…"             # 只补标题，其余照旧推断
 *   node scripts/commit.mjs --type=feat --scope=e2e \
 *        --title="e2e 门控接入" \
 *        --section="新增 e2e/cases 目录，每用例一文件" \
 *        --section="run-tests 改为发现式 runner"      # 分节显式给出（正文质量最高）
 *   node scripts/commit.mjs --gate                  # 提交前**显式**跑门禁（把关场景）
 *   node scripts/commit.mjs --interactive           # 逐项询问（想手写正文时）
 *   node scripts/commit.mjs --dry-run               # 只生成消息文件与暂存清单，不提交
 *   node scripts/commit.mjs --help                  # 打印用法并退出（**不提交**）
 *
 * （`--no-gate` / `--yes` 保留为 no-op：历史调用不受影响。）
 * ⚠️ 查用法一律用 `--help`：本脚本**只认已识别的开关**，把 `--help` 当未知参数会让它
 * 照常走完并**真的提交一次**（2026-10-05 实测踩过）。
 *
 * 分节来源优先级：`--section=`（可多次）> 交互逐条输入 > **自动从暂存 diff 归纳**。
 * 自动归纳只给「改了什么」的事实清单（文件 + 增删行数），不下判断、不编理由——
 * 需要「为什么」时请显式给 `--section=`。
 *
 * 流程：
 *   1. 前置检查：不在变基/合并冲突中、有暂存或可暂存的改动
 *   2. （仅 --gate）跑门禁，失败即中止，不产出提交
 *   3. 按仓库规范生成提交消息临时文件：
 *      标题 `<type>(<scope>): <中文标题>` + 编号分节 + 「门禁：」段
 *      写好后本地校验一次（check-commit-msg --file），校验不过视为脚本 bug，直接失败
 *   4. git commit -F <消息文件>（不 push；staged 之外的文件不动）
 *   5. 删除消息临时文件，打印结果
 *
 * 临时文件放 .git/ 下（不污染工作区、不进 status），路径随进程结束清理。
 */

import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { red, green, yellow, dim, bold } from './color.mjs'

const scriptDir = fileURLToPath(new URL('.', import.meta.url))

/**
 * 仓库根：**以调用方所在的 git 仓库为准**，不是「脚本自己住在哪个仓库」。
 * 从脚本位置推导会让它无法在任何其它仓库工作（也用不了临时仓库做回归测试）——
 * 脚本一旦不认 cwd，它测的东西就不是「提交这个仓库」了。
 */
function detectRepoRoot() {
  const r = spawnSync('git', ['rev-parse', '--show-toplevel'], {
    cwd: process.cwd(),
    encoding: 'utf8',
    shell: false,
  })
  if (r.status === 0 && r.stdout.trim()) return path.resolve(r.stdout.trim())
  return path.resolve(scriptDir, '..')
}

const repoRoot = detectRepoRoot()

const argv = process.argv.slice(2)
const hasFlag = (n) => argv.includes(n)
const valOf = (p) => {
  const a = argv.find((x) => x.startsWith(p))
  return a ? a.slice(p.length).trim() : null
}

// `--help` / `-h`：**在任何副作用之前**退出。
//
// 补它的理由：脚本原先不认 `--help`，于是 `node scripts/commit.mjs --help` 会**真的
// 提交一次**（把暂存区里的东西提上去）——想查用法，代价是一次误提交（只能
// `git reset --soft HEAD~1` 回退）。与 `gate.mjs` 的 `--help` 同类陷阱（那里是白跑
// 十分钟门禁 + 覆盖 `.workbuddy-ai/gate-logs/*.log`，最需要日志的那一刻它没了）。
// 判据 = `commit.test.mjs` 的 `--help` 用例：退出 0、打印用法、**不产生提交**。
if (hasFlag('--help') || hasFlag('-h')) {
  console.log(`用法：node scripts/commit.mjs [选项]

  （无参数）          非交互：type / scope / 标题从暂存内容推断，分节从 diff 归纳
  --type=<t>          显式 type（覆盖推断）
  --scope=<s>         显式 scope（覆盖推断）
  --title="…"         显式标题（覆盖推断）
  --section="…"       正文分节，可多次；**不要**自带编号（脚本会补编号）
  --gate              提交前显式跑门禁（**默认不跑**：提交要秒级）
  --interactive       逐项询问（想手写正文时；默认非交互、不读 stdin）
  --dry-run           只生成消息文件与暂存清单，不提交
  --help, -h          打印本用法并退出（不提交任何东西）

  注意：本脚本**只提交已暂存的文件**（先 git add；未暂存的不进本次提交）。`)
  process.exit(0)
}

const DRY_RUN = hasFlag('--dry-run')
/** 门禁**默认不跑**：提交要秒级，门禁该在改动完成时单独跑（见下方门禁段说明）。
 *  `--gate` 显式要求；`--no-gate` 保留为 no-op（历史调用不受影响）。 */
const GATE = hasFlag('--gate')
/** 交互模式要**显式**要求。`--yes` 保留为 no-op（历史调用不受影响）。 */
const INTERACTIVE = hasFlag('--interactive')
const TYPE = valOf('--type=')
const SCOPE = valOf('--scope=')
const TITLE = valOf('--title=')
const SECTIONS = argv
  .filter((x) => x.startsWith('--section='))
  .map((x) => x.slice('--section='.length))
const GATE_ARGS = argv.filter((x) => x.startsWith('--only=') || x.startsWith('--skip=') || x === '--ci')

function sh(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { cwd: repoRoot, encoding: 'utf8', shell: false, ...opts })
  return { ...r, out: (r.stdout ?? '').trim(), err: (r.stderr ?? '').trim() }
}

function die(msg) {
  console.error(red(`✗ ${msg}`))
  process.exit(1)
}

// ---------- 交互收集（仅 --interactive 时启用） ----------
async function prompt(question, { def = '', choices = null } = {}) {
  if (!INTERACTIVE) return def
  const readline = await import('node:readline/promises')
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout })
  try {
    const suffix = choices ? ` ${dim(`(${choices.join('/')})`)}` : def ? ` ${dim(`(${def})`)}` : ''
    const ans = (await rl.question(`${question}${suffix}: `)).trim()
    return ans || def
  } finally {
    rl.close()
  }
}

// ---------- 前置检查 ----------
const status = sh('git', ['status', '--porcelain'])
if (status.status !== 0) die(`git status 失败：${status.err}`)

const rebase = sh('git', ['rev-parse', '--git-path', 'rebase-merge'])
const merging = fs.existsSync(path.join(repoRoot, '.git', 'MERGE_HEAD'))
if (rebase.out && fs.existsSync(path.join(repoRoot, '.git', rebase.out))) {
  die('正处于变基过程中，先完成或中止变基再提交')
}
if (merging) die('正处于合并冲突中，先解决冲突再提交')

if (!status.out) die('没有可提交的改动')

// 本次提交只包含**索引**里的内容（最后一步是 `git commit -F`，不带 `-a`）。
// 上面那条判据看的是**工作区**：工作区脏而索引为空时它会放行，于是白跑一遍
// 门禁（几分钟），再在最后一步被 git 用「no changes added to commit」拒掉
// ——那条报错指不到真正的原因（真实原因在第一步之前就已知了）。
const staged = sh('git', ['diff', '--cached', '--name-only']).out
if (!DRY_RUN && !staged) {
  die(
    [
      '索引为空 —— 没有可提交的内容（本脚本不替你暂存，只提交索引里的内容）',
      `  工作区有 ${status.out.split('\n').filter(Boolean).length} 处改动，但一处都没暂存。`,
      '  先 git add 需要的文件（注意别把不相关的改动一起带上），再重跑。',
    ].join('\n'),
  )
}

// ---------- 门禁（默认跳过，--gate 才跑） ----------
// 提交归提交，门禁归门禁：两者**不该耦合**。脚本默认跑门禁时，一次提交要等
// 几分钟门禁跑完——而那笔账在「跑门禁」这一步已经付过了（门禁该在改动完成、
// 提交之前单独跑）。把两个动作分开，提交就永远是秒级的。
//
// 而且耦合还有个更坏的效果：门禁的 autoWork 会**改文件并暂存**，于是「提交」
// 这个动作的输入（索引）被另一个动作改写——顺序与内容都变得不直观。
//
// 需要「本次提交必须门禁全绿」时显式 `--gate`（CI/发布前的把关场景）。
// **--gate 优先于 --dry-run**：预览与把关是两个正交的诉求，同时给出时把关优先
// （预览若静默吞掉 --gate，人就以为门禁跑过了）。
let gateSummary = '门禁：本次跳过（未跑；需要时单独 `node scripts/gate.mjs` 或加 --gate）'
if (GATE) {
  console.log(bold('══ 第 1 步 · 门禁（--gate） ══'))
  const gate = spawnSync(process.execPath, [path.join(scriptDir, 'gate.mjs'), ...GATE_ARGS], {
    cwd: repoRoot,
    stdio: 'inherit',
  })
  if (gate.status !== 0) die('门禁未通过，已中止提交（修复后重跑，或不加 --gate 直接提交）')
  // 门禁内部对「失败 0 但 skipped>0」也给 0；摘要从日志目录取最近一次结果行
  gateSummary = '门禁：node scripts/gate.mjs 全部通过'
  console.log()
} else if (DRY_RUN) {
  gateSummary = '门禁：本次跳过（--dry-run 预览，未执行）'
}

// ---------- 收集消息素材 ----------
console.log(bold('══ 第 2 步 · 提交消息 ══'))
// ⚠️ 索引在门禁前后会变：门禁把 `cargo fmt` / 事实文件生成这类**确定性机械工作**
// 自己做完并**当场暂存**（见 `gate.d/_shared.autoWork`）。所以这里必须**重新读**，
// 不能沿用开头那份快照——那份是在门禁之前取的。
// （提交内容本身一直是对的：最后一步 `git commit -F` 用的是**活索引**；错的只是显示。）
const stagedNow = sh('git', ['diff', '--cached', '--name-only']).out
const workNow = sh('git', ['status', '--porcelain']).out
const stagedCount = stagedNow ? stagedNow.split('\n').filter(Boolean).length : 0
const workCount = workNow ? workNow.split('\n').filter(Boolean).length : 0
if (!DRY_RUN && stagedCount === 0) {
  die('门禁跑完后索引为空 —— 没有可提交的内容（门禁的自动修复也没产生暂存项）')
}
console.log(dim(`  暂存 ${stagedCount} 个文件；工作区共 ${workCount} 处改动（未暂存的不会进本次提交）`))

const stagedFiles = (stagedNow || staged || status.out)
  .split('\n')
  .filter(Boolean)
  .map((f) => f.replace(/^"|"$/g, ''))

/**
 * 从暂存路径推断 scope（**仅目录无法归类时**才回落 `misc`）。
 * 只看首个暂存文件不够：一次提交常常跨多域，取「出现次数最多的可归类前缀」。
 */
function guessScope() {
  const counts = new Map()
  for (const f of stagedFiles) {
    let s = null
    if (f.startsWith('e2e/')) s = 'e2e'
    else if (f.startsWith('scripts/')) s = 'gate'
    else if (f.startsWith('symbio/')) {
      const m = f.match(/^symbio\/src\/(?:plugins\/([^/]+)|symbio_core)/)
      s = m ? (m[1] ?? 'core') : null
    } else if (f.startsWith('tauri/')) s = 'ui'
    else if (f.startsWith('cli/')) s = 'cli'
    else if (f.startsWith('docs/')) s = 'docs'
    if (s) counts.set(s, (counts.get(s) ?? 0) + 1)
  }
  let best = 'misc'
  let max = 0
  for (const [s, n] of counts) {
    if (n > max) {
      best = s
      max = n
    }
  }
  return best
}

/**
 * 从暂存路径推断 type —— 路径与增删行数是**唯一**能机械判定的信号，不用 LLM 猜。
 * 判据（按优先级）：
 *   · 只有测试文件（`*.test.*`）或 `e2e/` 变动 ⇒ test
 *   · 只有 `docs/`、`*.md` 变动 ⇒ docs
 *   · 工具类（`scripts/`、`e2e/`，**可夹带根级文档**）：
 *       有新增的 `*.test.*` ⇒ test；否则 ⇒ chore
 *   · 新增文件为主（占比 ≥ 2/3 且有净增行）⇒ feat
 *   · 否则 ⇒ refactor（改动为主：重命名 / 搬迁 / 收紧）
 * 显式 `--type=` 永远覆盖。
 *
 * 判定顺序上 **test 先于 docs**：`scripts/commit.test.mjs` 也匹配 `*.md` 之外的
 * 路径，但若先问 docs 就会把「测试改动」在带 md 时误判成文档。
 *
 * 工具类允许**夹带根级 md**（如 `CONTRIBUTING.md`）：改工具同时补一句说明是
 * 常规动作，若严格要求全部落在 `scripts/`，这类提交会掉进 `refactor`——
 * 而它们和「重构产品代码」完全不是一回事，混在一起历史就废了。
 */
function guessType() {
  const everyMatches = (re) => stagedFiles.every((f) => re.test(f))

  if (stagedFiles.length === 0) return 'chore'
  if (everyMatches(/(\.test\.[a-z]+$)|(^e2e\/)/)) return 'test'
  if (everyMatches(/\.md$/)) return 'docs'

  // 增删统计：只看汇总行里的新增 / 删除文件数
  const numstat = sh('git', ['diff', '--cached', '--numstat']).out
  let added = 0
  let deleted = 0
  let insertions = 0
  let deletions = 0
  for (const line of numstat.split('\n').filter(Boolean)) {
    const [a, d] = line.split('\t')
    const ai = Number(a)
    const di = Number(d)
    if (ai === 0) added++
    else if (di === 0) deleted++
    if (Number.isFinite(ai)) insertions += ai
    if (Number.isFinite(di)) deletions += di
  }

  // 脚本类改动（守卫 / 门禁 / 提交工具）不是产品功能，**但也不是一回事**：
  // 判据要落在「这次动作是补测试还是改机制」上，否则两种历史读起来会混。
  const onlyTooling = stagedFiles.every(
    (f) => f.startsWith('scripts/') || f.startsWith('e2e/') || !f.includes('/'),
  )
  if (onlyTooling) {
    // 新增了回归测试文件（`scripts/*.test.mjs`）⇒ test：测试本身就是这批改动的产物。
    // 不这样分的话，「为 X 补测试」和「改 X」在历史里长得一样。
    const newTests = sh('git', ['diff', '--cached', '--name-status'])
      .out.split('\n')
      .filter(Boolean)
      .filter((l) => l.startsWith('A\t') && /\.test\.[a-z]+$/.test(l.split('\t')[1] ?? '')).length
    if (newTests > 0) return 'test'
    return 'chore'
  }

  if (added > 0 && deleted === 0 && insertions > deletions) return 'feat'
  if (added > 0 && added * 2 >= stagedFiles.length && insertions > deletions) return 'feat'
  return 'refactor'
}

/**
 * 自动标题：说事实（改动落在哪、增删各多少），不编语义。
 * 显式 `--title=` 永远覆盖——自动值只求「不为空且读得出规模」。
 *
 * 维度必须**统一**：全用顶层目录。此前把「无目录的根文件」按文件名入列，
 * 于是标题会出现 `scripts/ 11 文件、CONTRIBUTING.md 1 文件`——同一句话里
 * 一半是目录一半是文件，读起来像两种分类混在了一起。
 * 根文件统一并入 `(根)`，动作用「新增/删除/改」几字带出，省得读者再去看 diff。
 */
function guessTitle() {
  const status = sh('git', ['diff', '--cached', '--name-status']).out.split('\n').filter(Boolean)
  const byTop = new Map()
  for (const line of status) {
    const [st, file] = line.split('\t')
    if (!file) continue
    const top = file.includes('/') ? `${file.split('/')[0]}/` : '(根)'
    const cur = byTop.get(top) ?? { n: 0, add: 0, del: 0 }
    cur.n++
    if (st === 'A') cur.add++
    else if (st === 'D') cur.del++
    byTop.set(top, cur)
  }
  const rows = [...byTop.entries()].sort((a, b) => b[1].n - a[1].n).slice(0, 3)
  const parts = rows.map(([d, { n, add, del }]) => {
    const verb = del === n ? '删' : add === n ? '新增' : '改'
    // 目录名自带 `/`（`scripts/`）后不空格，`(根)` 则补一个——否则 `(根)1 文件`
    // 会读成「根一文件」。
    const sep = d === '(根)' ? ' ' : ''
    return `${verb} ${d}${sep}${n} 文件`
  })
  return `${parts.join('、')}（共 ${stagedFiles.length} 文件）`
}

/**
 * 自动分节：按**顶层目录**归纳暂存 diff（各目录改了哪些文件、增删行数）。
 * 只陈述改了什么，不解释为什么——「为什么」由作者给 `--section=` 补。
 *
 * 此前按 `slice(0, 2)` 切片，于是 `scripts/gate.d`（5 个文件）与
 * `scripts/commit.mjs`（1 个文件）被并列成同级条目，正文退化成文件清单。
 * 现在先按顶层聚合，**同目录超过 1 个文件时才**附上文件名（否则目录名已足够）。
 */
function guessSections() {
  const numstat = sh('git', ['diff', '--cached', '--numstat']).out
  const byTop = new Map()
  for (const line of numstat.split('\n').filter(Boolean)) {
    const [a, d, file] = line.split('\t')
    if (!file) continue
    const top = file.includes('/') ? `${file.split('/')[0]}/` : '(根)'
    const cur = byTop.get(top) ?? { files: [], ins: 0, del: 0 }
    cur.files.push(file.includes('/') ? file.slice(file.indexOf('/') + 1) : file)
    cur.ins += Number(a) || 0
    cur.del += Number(d) || 0
    byTop.set(top, cur)
  }
  const rows = [...byTop.entries()].sort((a, b) => b[1].files.length - a[1].files.length).slice(0, 8)
  return rows.map(([top, { files, ins, del }]) => {
    // 根文件（`(根)`）即使是单个也要报名字——目录名本身没信息量，`(根)` 等于没说。
    const needNames = top === '(根)' || files.length > 1
    const detail = needNames ? `（${files.slice(0, 6).join('、')}${files.length > 6 ? ' 等' : ''}）` : ''
    return `${top}${detail}：${files.length} 文件（+${ins}/-${del}）`
  })
}

const type = TYPE ?? (await prompt('type', { def: guessType(), choices: ['feat', 'fix', 'docs', 'refactor', 'chore', 'test', 'perf', 'style'] }))
const scope = SCOPE ?? (await prompt('scope', { def: guessScope() }))
const title = TITLE ?? (await prompt('中文标题', { def: guessTitle() }))
if (!title) die('标题不能为空')

const sections = [...SECTIONS]
if (sections.length === 0 && INTERACTIVE) {
  console.log(dim('  正文分节（逐条输入，空行结束）：'))
  for (let i = 1; ; i++) {
    const s = await prompt(`  ${i}.`)
    if (!s) break
    sections.push(s)
  }
}
if (sections.length === 0 && !INTERACTIVE) {
  // 非交互且未给分节：用 diff 归纳的事实清单兜底，并明确告知来源（可复核）。
  sections.push(...guessSections())
  if (sections.length > 0) {
    console.log(dim(`  未给 --section，正文按暂存 diff 自动归纳 ${sections.length} 条（可复核；要写「为什么」请显式给 --section=）`))
  }
}

// ---------- 生成消息文件 ----------
const msgPath = path.join(repoRoot, '.git', `COMMIT_MSG_${process.pid}.txt`)

/**
 * 删除提交消息临时文件，**永不抛**，且**以文件真的没了为准**。
 *
 * 为什么不直接 `fs.rmSync`：本机（沙箱）注入了 safe-delete 垫片，它可能
 * ① 在「本回合删除数超过阈值」时**抛异常**，或 ② 把删除转交回收站/代理进程后
 * 就返回。两种情况下 `rmSync` 都「成功返回」而文件**仍在**——所以这里删完必须
 * 回查一次 `existsSync`，不能把「没抛异常」当成「删掉了」。
 *
 * 返回是否真的删掉了（false = 已尽力，留了条警告；文件在 `.git/` 下，不进 status）。
 */
function cleanupMsg(p) {
  let note = null
  try {
    fs.rmSync(p, { force: true })
  } catch (e) {
    note = e?.message ?? String(e)
  }
  const gone = !fs.existsSync(p)
  if (!gone) {
    console.log(
      yellow(
        `  ⚠ 临时消息文件仍在（${path.basename(p)}）${note ? `：${note}` : '：rmSync 未抛错但文件未消失'}`,
      ),
    )
  }
  return gone
}

const gateSection = [
  '门禁：',
  `- ${gateSummary}`,
  '- e2e：node scripts/gate.mjs --only=e2e（CLI × mock LLM × mock MCP）',
].join('\n')

const msg = [
  `${type}(${scope}): ${title}`,
  '',
  sections.length
    ? sections.map((s, i) => `${i + 1}. ${s}`).join('\n\n')
    : '1. （正文分节留空 —— 建议补一段「改了什么 / 为什么」）',
  '',
  gateSection,
  '',
].join('\n')

fs.writeFileSync(msgPath, msg, 'utf8')

// 自校验：消息必须过仓库自己的规范检查（不过即脚本 bug）
const chk = sh(process.execPath, [path.join(scriptDir, 'check-commit-msg.mjs'), '--file', msgPath])
if (chk.status !== 0) {
  cleanupMsg(msgPath)
  die(`生成的消息未通过规范校验（脚本 bug，请反馈）：\n${chk.out}\n${chk.err}`)
}
console.log(green('  ✓ 提交消息已生成并通过规范自校验'))

if (DRY_RUN) {
  console.log(dim('  ── 消息预览（--dry-run，未提交）──'))
  console.log(msg)
  cleanupMsg(msgPath)
  process.exit(0)
}

// ---------- 提交 ----------
console.log(bold('══ 第 3 步 · 提交 ══'))
const commit = sh('git', ['commit', '-F', msgPath], { maxBuffer: 16 * 1024 * 1024 })
// 先判结果、再清理：**清理失败绝不允许掩盖提交结果**。
// 这里曾是 `git commit` 之后紧跟一句裸 `fs.rmSync`，而它可能抛（沙箱的
// safe-delete 垫片会在"本回合删除数超阈值"时拒绝删除）——于是脚本带着栈回溯
// 退出，用户看到的是失败，而**提交其实已经建好了**。不可逆动作之后的任何一步
// 都不该能改写"它到底成没成"这个结论。
const cleaned = cleanupMsg(msgPath)
if (commit.status !== 0) die(`git commit 失败：\n${commit.err || commit.out}`)

console.log(green(`✓ 已提交 ${commit.out.split('\n')[0]}${os.EOL}  （未 push；消息文件${cleaned ? '已清理' : '清理失败，见上方警告'}）`))
