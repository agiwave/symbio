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
 *   echo "feat(x): 标题" | node scripts/check-commit-msg.mjs --stdin
 *
 * 挂成 git hook（一次性）：
 *   printf '#!/bin/sh\nnode scripts/check-commit-msg.mjs --file "$1"\n' > .git/hooks/commit-msg
 *   chmod +x .git/hooks/commit-msg
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
  console.error('用法：node scripts/check-commit-msg.mjs --file <path> | --last | --stdin')
  process.exit(1)
}

const raw = readMessage()
// 去掉注释行（git 给 commit-msg hook 的模板里带 # 注释）
const text = raw
  .split('\n')
  .filter((l) => !l.startsWith('#'))
  .join('\n')
  .trim()

if (!text) {
  console.log(red('✗ 提交信息为空'))
  process.exit(1)
}

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

if (errors.length === 0 && warns.length === 0) {
  console.log(green('✓ 提交信息符合规范'))
  process.exit(0)
}
for (const w of warns) console.log(yellow(`⚠ ${w}`))
for (const e of errors) console.log(red(`✗ ${e}`))
if (errors.length > 0) {
  console.log()
  console.log('提示：本机一次性挂上 hook 后就不必再记这些：')
  console.log('  printf \'#!/bin/sh\\nnode scripts/check-commit-msg.mjs --file "$1"\\n\' > .git/hooks/commit-msg')
  process.exit(1)
}
process.exit(0)
