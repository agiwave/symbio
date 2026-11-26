#!/usr/bin/env node
/**
 * decision-literal-audit — 禁止恢复逐案例的架构决策编号（DL-001）
 *
 * 规则只留审计判据码：`D-NNN` 必须能在 scripts/** 的 `// @codes` 声明中找到。
 * 文档登记 ID（DE-/SP-/RE-/T-）与复合审计码（SD-/TD-）不在本判据面内。
 *
 * 判据失效与零违规都退出 1：scripts/ 读不出、@codes 读不出、仓库根不存在，
 * 都不能伪装成通过。测试夹具动态拼出禁止编号，避免在守卫自己的源码里留下样例。
 */
// @gate guard
// @ns DL 决策字面量禁区
// @codes DL-001

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green } from '../color.mjs'
import { walkFiles } from '../fs-walk.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice(7)) : path.resolve(scriptDir, '..', '..')
const SKIP_DIRS = new Set(['.git', 'node_modules', 'target', 'tmp', '.symbio', '.trae', '.workbuddy-ai'])
const EXTS = ['.md', '.rs', '.mjs', '.js', '.ts', '.vue', '.json', '.yml', '.yaml', '.toml']

export function auditDecisionLiterals(root) {
  const scriptsDir = path.join(root, 'scripts')
  if (!fs.existsSync(root) || !fs.statSync(root).isDirectory() || !fs.existsSync(scriptsDir)) {
    return { fatal: '仓库根或 scripts/ 不存在，判据没有输入' }
  }

  let scripts
  try {
    scripts = walkFiles(scriptsDir, { exts: ['.mjs'], skipDirs: SKIP_DIRS })
  } catch (e) {
    return { fatal: `scripts/ 读不出：${e.message}` }
  }
  if (scripts.length === 0) return { fatal: 'scripts/ 没有可读的判据声明' }

  const auditCodes = new Set()
  for (const file of scripts) {
    if (file.endsWith('.test.mjs')) continue
    const lines = fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n').split('\n')
    for (const line of lines) {
      if (/^import\s/.test(line)) break
      if (!/^\/\/\s*@codes\s+/.test(line)) continue
      for (const m of line.matchAll(/\bD-\d{3}[a-z]?\b/g)) auditCodes.add(m[0])
    }
  }
  if (auditCodes.size === 0) return { fatal: '没有读到任何审计 D 判据码，禁止把未知编号当违规放行' }

  let files
  try {
    files = walkFiles(root, { exts: EXTS, skipDirs: SKIP_DIRS })
  } catch (e) {
    return { fatal: `仓库文件读不出：${e.message}` }
  }
  if (files.length === 0) return { fatal: '仓库没有可扫描的文本文件' }

  const violations = []
  for (const file of files.sort()) {
    const rel = path.relative(root, file).split(path.sep).join('/')
    const lines = fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n').split('\n')
    lines.forEach((line, i) => {
      for (const m of line.matchAll(/\bD-\d{3}[a-z]?\b/g)) {
        if (auditCodes.has(m[0])) continue
        const kind = /^\s*#{1,6}\s/.test(line) ? '决策标题' : '决策引用'
        violations.push({ key: `${rel}:${i + 1}`, code: m[0], kind, text: line.trim() })
      }
    })
  }
  return { violations, auditCodeCount: auditCodes.size, scannedFiles: files.length }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { violations = [], fatal, auditCodeCount = 0, scannedFiles = 0 } = auditDecisionLiterals(ROOT)
  if (fatal) {
    console.error(red(`✗ 决策字面量审计：${fatal} ⇒ 本项失去判据`))
    process.exit(1)
  }
  if (violations.length > 0) {
    console.error(red(`✗ 决策字面量审计：${violations.length} 处架构判例残留`))
    for (const v of violations) {
      console.error(`  DL-001 ${v.key} ${v.kind} ${v.code}\n        ${v.text}`)
    }
    process.exit(1)
  }
  console.log(green(`✓ 决策字面量审计：${scannedFiles} 个文件无架构判例；${auditCodeCount} 个审计 D 码保留`))
}
