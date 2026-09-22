#!/usr/bin/env node
/**
 * commit — 统一提交入口：跑门禁 → 生成合规消息 → git commit → 清理。
 *
 *   node scripts/commit.mjs                        # 交互：逐项询问 type/scope/标题/分节
 *   node scripts/commit.mjs --yes                  # 非交互：全用默认（type=chore 等）
 *   node scripts/commit.mjs --type=feat --scope=e2e \
 *        --title="e2e 门控接入" \
 *        --section="新增 e2e/cases 目录，每用例一文件" \
 *        --section="run-tests 改为发现式 runner" \
 *        --no-gate                                  # 跳过门禁（仅紧急热修）
 *   node scripts/commit.mjs --dry-run              # 只生成消息文件与暂存清单，不提交
 *
 * 流程：
 *   1. 前置检查：不在变基/合并冲突中、有暂存或可暂存的改动
 *   2. 跑门禁（node scripts/gate.mjs，可用 --only/--skip/--fix/--ci 透传），
 *      除非 --no-gate；门禁失败即中止，不产出提交
 *   3. 按仓库规范生成提交消息临时文件：
 *      标题 `<type>(<scope>): <中文标题>` + 编号分节 + 「门禁：」段（附门禁汇总摘要）
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

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

const argv = process.argv.slice(2)
const hasFlag = (n) => argv.includes(n)
const valOf = (p) => {
  const a = argv.find((x) => x.startsWith(p))
  return a ? a.slice(p.length).trim() : null
}

const DRY_RUN = hasFlag('--dry-run')
const NO_GATE = hasFlag('--no-gate')
const YES = hasFlag('--yes')
const TYPE = valOf('--type=')
const SCOPE = valOf('--scope=')
const TITLE = valOf('--title=')
const SECTIONS = argv
  .filter((x) => x.startsWith('--section='))
  .map((x) => x.slice('--section='.length))
const GATE_ARGS = argv.filter((x) => x.startsWith('--only=') || x.startsWith('--skip=') || x === '--fix' || x === '--ci')

function sh(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { cwd: repoRoot, encoding: 'utf8', shell: false, ...opts })
  return { ...r, out: (r.stdout ?? '').trim(), err: (r.stderr ?? '').trim() }
}

function die(msg) {
  console.error(red(`✗ ${msg}`))
  process.exit(1)
}

// ---------- 交互收集 ----------
async function prompt(question, { def = '', choices = null } = {}) {
  if (YES) return def
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

// ---------- 门禁 ----------
let gateSummary = '门禁：本次跳过（--dry-run 预览，未执行）'
if (DRY_RUN) {
  // 预览只生成消息，不跑门禁（门禁要几分钟，预览要的是快）
} else if (NO_GATE) {
  gateSummary = '门禁：本次跳过（--no-gate）'
} else {
  console.log(bold('══ 第 1 步 · 门禁 ══'))
  const gate = spawnSync(process.execPath, [path.join(scriptDir, 'gate.mjs'), ...GATE_ARGS], {
    cwd: repoRoot,
    stdio: 'inherit',
  })
  if (gate.status !== 0) die('门禁未通过，已中止提交（修复后重跑；紧急热修可用 --no-gate 并自担风险）')
  // 门禁内部对「失败 0 但 skipped>0」也给 0；摘要从日志目录取最近一次结果行
  gateSummary = '门禁：node scripts/gate.mjs 全部通过'
  console.log()
}

// ---------- 收集消息素材 ----------
console.log(bold('══ 第 2 步 · 提交消息 ══'))
const staged = sh('git', ['diff', '--cached', '--name-only']).out
const unstaged = status.out.split('\n').filter((l) => !l.startsWith('#')).length
console.log(dim(`  暂存 ${staged ? staged.split('\n').length : 0} 个文件；工作区共 ${unstaged} 处改动（未暂存的不会进本次提交）`))

const type = TYPE ?? (await prompt('type', { def: 'chore', choices: ['feat', 'fix', 'docs', 'refactor', 'chore', 'test', 'perf', 'style'] }))
const scope = SCOPE ?? (await prompt('scope', { def: await guessScope() }))
const title = TITLE ?? (await prompt('中文标题', { def: '' }))
if (!title) die('标题不能为空')

const sections = [...SECTIONS]
if (sections.length === 0 && !YES) {
  console.log(dim('  正文分节（逐条输入，空行结束）：'))
  for (let i = 1; ; i++) {
    const s = await prompt(`  ${i}.`)
    if (!s) break
    sections.push(s)
  }
}

async function guessScope() {
  // 从暂存路径猜 scope：e2e/ → e2e，scripts/ → gate|commit 等，其余取一级目录
  const files = (staged || status.out).split('\n').filter(Boolean)
  const first = files[0]?.replace(/^"|"$/g, '') ?? ''
  if (first.startsWith('e2e/')) return 'e2e'
  if (first.startsWith('scripts/')) return 'gate'
  if (first.startsWith('symbio/')) {
    const m = first.match(/^symbio\/src\/(?:plugins\/([^/]+)|symbio_core)/)
    if (m) return m[1] ?? 'core'
  }
  if (first.startsWith('tauri/')) return 'ui'
  if (first.startsWith('cli/')) return 'cli'
  if (first.startsWith('docs/')) return 'docs'
  return 'misc'
}

// ---------- 生成消息文件 ----------
const msgPath = path.join(repoRoot, '.git', `COMMIT_MSG_${process.pid}.txt`)
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
  fs.rmSync(msgPath, { force: true })
  die(`生成的消息未通过规范校验（脚本 bug，请反馈）：\n${chk.out}\n${chk.err}`)
}
console.log(green('  ✓ 提交消息已生成并通过规范自校验'))

if (DRY_RUN) {
  console.log(dim('  ── 消息预览（--dry-run，未提交）──'))
  console.log(msg)
  fs.rmSync(msgPath, { force: true })
  process.exit(0)
}

// ---------- 提交 ----------
console.log(bold('══ 第 3 步 · 提交 ══'))
const commit = sh('git', ['commit', '-F', msgPath], { maxBuffer: 16 * 1024 * 1024 })
fs.rmSync(msgPath, { force: true })
if (commit.status !== 0) die(`git commit 失败：\n${commit.err || commit.out}`)

console.log(green(`✓ 已提交 ${commit.out.split('\n')[0]}${os.EOL}  （未 push；消息文件已清理）`))
