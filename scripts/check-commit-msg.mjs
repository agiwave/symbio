#!/usr/bin/env node
/**
 * check-commit-msg — 提交信息规范检查（取代「记在记忆里的提交格式要求」）
 *
 * 规范（原先靠人记，现在由本脚本判）：
 *   1. 标题 = `<type>(<scope>): <中文标题>`
 *   2. 正文为编号分节（1. 2. 3.），说明改了什么、为什么
 *   3. 末尾有「门禁：」段，列出本次跑过的检查与结果
 *
 * 用法：
 *   node scripts/check-commit-msg.mjs --file <path>   # 校验提交信息文件（配 git hook）
 *   node scripts/check-commit-msg.mjs --last          # 校验 HEAD 这一次提交
 *   node scripts/check-commit-msg.mjs --range <rev>   # 校验一段提交（CI 用，逐个判）
 *   echo "feat(x): 标题" | node scripts/check-commit-msg.mjs --stdin
 *
 * 挂成 git hook（一次性，本机生效）：
 *   git config core.hooksPath scripts/git-hooks
 *
 * ⚠️ **「机制化」必须真的插上电**：钩子写在这里 ≠ 它生效。要它真的拦人，需要
 *   ① 本机 `git config core.hooksPath scripts/git-hooks`；② CI 里跑 `--range`。
 *   两者缺一，这条规范就退回"靠提交者自觉手动跑一次"——2026-09-20 复核时发现
 *   正是这个状态（钩子文件在、`CONTRIBUTING.md` 写了步骤，但两处都没接上）。
 *
 * 退出码：0 = 通过；1 = 不符合规范。
 */

import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { red, green, yellow } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

const argv = process.argv.slice(2)

const TYPES = ['feat', 'fix', 'docs', 'refactor', 'chore', 'test', 'perf', 'style']
const TITLE_RE = new RegExp(`^(${TYPES.join('|')})\\(([^)]+)\\):\\s*(.+)$`)
/** 标题应含中文（项目约定：中文标题） */
const HAS_CJK = /[一-龥]/

function readMessage() {
  if (argv.includes('--stdin')) return fs.readFileSync(0, 'utf8')
  const fileIdx = argv.indexOf('--file')
  if (fileIdx >= 0 && argv[fileIdx + 1]) return fs.readFileSync(argv[fileIdx + 1], 'utf8')
  if (argv.includes('--last')) {
    const r = spawnSync('git', ['log', '-1', '--pretty=%B'], { cwd: repoRoot, encoding: 'utf8', shell: false })
    if (r.status !== 0) {
      console.error(red('读取最近一次提交失败'))
      process.exit(1)
    }
    return r.stdout
  }
  console.error('用法：node scripts/check-commit-msg.mjs --file <path> | --last | --stdin | --range <rev>')
  process.exit(1)
}

/**
 * 校验一条提交信息，返回 { errors, warns }。
 *
 * 抽成函数是为了让「单条」与「一段范围」共用**同一套判据**——否则 CI 那条路与
 * 本机钩子那条路会各自演化，最后变成两条不同的规范。
 */
function validate(raw) {
  // 去掉注释行（git 给 commit-msg hook 的模板里带 # 注释）
  const text = raw
    .split('\n')
    .filter((l) => !l.startsWith('#'))
    .join('\n')
    .trim()

  if (!text) return { errors: ['提交信息为空'], warns: [] }

  const lines = text.split('\n')
  const title = lines[0].trim()
  const body = lines.slice(1).join('\n')

  const errors = []
  const warns = []

  const m = title.match(TITLE_RE)
  if (!m) {
    const known = title.match(/^(feat|fix|docs|refactor|chore|test|perf|style)/)
    errors.push(
      known
        ? `标题格式应为 \`<type>(<scope>): <中文标题>\`（type ∈ ${TYPES.join(' / ')}），实际：${title}`
        : `标题应以 ${TYPES.join(' / ')} 开头，实际：${title}`
    )
  } else if (!HAS_CJK.test(m[3])) {
    warns.push('标题建议使用中文（项目约定）')
  }

  // 正文：编号分节 + 「门禁」段（只有多段落提交才要求；单行 fix 不强求）
  const hasBodySections = /^\s*\d+\.\s/m.test(body)
  if (body.trim() && !hasBodySections) {
    warns.push('正文建议用编号分节（1. 2. 3.），说明改了什么 / 为什么')
  }
  if (!/门禁[:：]/.test(body)) {
    errors.push('缺少「门禁：」段 —— 需写明本次跑过哪些检查、结果如何')
  }
  return { errors, warns }
}

const HINT = [
  '',
  '提示：本机一次性挂上 hook 后就不必再记这些：',
  '  git config core.hooksPath scripts/git-hooks',
].join('\n')

// ── 一段范围（CI 用）：逐个提交判，任一不合规即失败 ──────────────────────
const rangeIdx = argv.indexOf('--range')
if (rangeIdx >= 0) {
  const range = argv[rangeIdx + 1]
  if (!range) {
    console.error(red('✗ --range 需要一个 rev-range（如 origin/main..HEAD）'))
    process.exit(1)
  }
  // 只看非合并提交：merge commit 的信息由 GitHub 生成，不该由本项目规范来判
  const list = spawnSync('git', ['rev-list', '--no-merges', '--reverse', range], {
    cwd: repoRoot,
    encoding: 'utf8',
    shell: false,
  })
  if (list.status !== 0) {
    // **不静默跳过**：范围解析不了就说清楚，否则这条门禁会变成"看起来跑了、其实没判"
    console.error(red(`✗ 解析提交范围失败：${range}`))
    console.error((list.stderr || '').trim())
    process.exit(1)
  }
  const shas = list.stdout.split('\n').map((s) => s.trim()).filter(Boolean)
  if (shas.length === 0) {
    console.log(green(`✓ 提交范围内没有需要校验的提交：${range}`))
    process.exit(0)
  }

  let bad = 0
  for (const sha of shas) {
    const r = spawnSync('git', ['log', '-1', '--pretty=%B', sha], {
      cwd: repoRoot,
      encoding: 'utf8',
      shell: false,
    })
    const short = sha.slice(0, 8)
    const { errors, warns } = validate(r.stdout ?? '')
    if (errors.length === 0) {
      console.log(green(`✓ ${short}`))
      for (const w of warns) console.log(yellow(`    ⚠ ${w}`))
      continue
    }
    bad += 1
    console.log(red(`✗ ${short}`))
    for (const e of errors) console.log(red(`    ${e}`))
  }

  if (bad > 0) {
    console.log()
    console.log(red(`✗ ${bad}/${shas.length} 个提交信息不合规`))
    console.log(HINT)
    process.exit(1)
  }
  console.log(green(`✓ ${shas.length} 个提交信息全部符合规范（范围 ${range}）`))
  process.exit(0)
}

// ── 单条（本机钩子 / 手工）────────────────────────────────────────────
const { errors, warns } = validate(readMessage())
if (errors.length === 0 && warns.length === 0) {
  console.log(green('✓ 提交信息符合规范'))
  process.exit(0)
}
for (const w of warns) console.log(yellow(`⚠ ${w}`))
for (const e of errors) console.log(red(`✗ ${e}`))
if (errors.length > 0) {
  console.log(HINT)
  process.exit(1)
}
process.exit(0)
