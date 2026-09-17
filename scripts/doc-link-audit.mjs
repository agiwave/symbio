#!/usr/bin/env node
/**
 * doc-link-audit — 文档相对链接审计
 *
 * 用途：扫描**全部文档**（系统级 + 模块级）的**站内相对链接**，报告目标不存在者。
 *   文档移动 / 归档（`git mv`）最容易留下静默坏链——阅读时才发现，
 *   而它本可以在提交前被机械地查出来。
 *
 * 扫描范围（与「文档下沉原则」对齐：单模块文档在该模块目录内）：
 *   docs/                系统级文档（**archive/ 除外**，见下）
 *   symbio/src/          插件模块文档（plugins/<plugin>/README.md + plugins/<plugin>/docs/）
 *   tauri/               前端模块文档（README.md + docs/）
 *   cli/                 命令行模块文档（README.md + docs/）
 *   examples/            示例文档
 *   根目录 *.md          README / CONTRIBUTING / CODE_OF_CONDUCT
 *
 * **整体豁免 `docs/archive/`**：归档记录的是**当时形态**，其中指向的兄弟文档可能
 *   早已被合并 / 删除，改写归档链接等于篡改历史。故该目录整体跳过（跳过文件数会打印）。
 *   活文档（其余全部范围）的失效链接照常报。
 *
 * 用法：
 *   node scripts/doc-link-audit.mjs              # 报告全部失效链接
 *   node scripts/doc-link-audit.mjs --strict     # 有失效链接即失败（退出码 1）
 *
 * 退出码：
 *   0 = 无失效链接
 *   1 = --strict 且存在失效链接
 *
 * 与仓库约定一致：纯 Node 实现，不依赖 bash / ripgrep，Windows / macOS / Linux 通用。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

const STRICT = process.argv.includes('--strict')

/** 扫描根（相对 repoRoot）；目录递归，文件直接检查 */
const ROOTS = ['docs', 'symbio/src', 'tauri', 'cli', 'examples']

/** 根目录下的散落 Markdown */
const ROOT_FILES = ['README.md', 'CONTRIBUTING.md', 'CODE_OF_CONDUCT.md']

/**
 * 整体豁免的目录（相对 repoRoot，以 `/` 结尾）。
 * docs/archive/ = 历史归档，其链接指向的是**当时**的兄弟文档，改写等于篡改历史。
 */
const EXEMPT_DIRS = ['docs/archive/']

/** 递归时跳过的目录名（构建产物 / 依赖 / 版本库） */
const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'dist', 'build', '.venv'])

// 形如 [文字](目标)；目标里的括号不常见，按非贪婪取到第一个右括号
const LINK = /\[[^\]]*\]\(([^)]+)\)/g

/** 应跳过的目标：外链 / 协议 / 纯锚点 / 空 */
const isExternal = (t) =>
  t === '' ||
  t.startsWith('#') ||
  /^[a-z][a-z0-9+.-]*:/i.test(t) // http: https: mailto: file:

const bad = []
let total = 0
let skippedFiles = 0

const walk = (dir) => {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (SKIP_DIRS.has(entry.name)) continue
      walk(path.join(dir, entry.name))
    } else if (entry.name.endsWith('.md')) {
      check(path.join(dir, entry.name))
    }
  }
}

function check(file) {
  const rel = path.relative(repoRoot, file).split(path.sep).join('/')
  if (EXEMPT_DIRS.some((d) => rel.startsWith(d))) {
    skippedFiles += 1
    return
  }
  const text = fs.readFileSync(file, 'utf8')
  for (const m of text.matchAll(LINK)) {
    const raw = m[1].trim()
    if (isExternal(raw)) continue
    const target = raw.split('#')[0].trim() // 去锚点
    if (isExternal(target) || target === '') continue
    total += 1
    const resolved = path.resolve(path.dirname(file), target)
    if (!fs.existsSync(resolved)) {
      bad.push({ from: path.relative(repoRoot, file), to: target })
    }
  }
}

for (const rel of ROOTS) {
  const dir = path.join(repoRoot, rel)
  if (!fs.existsSync(dir)) {
    console.error(`找不到扫描根：${dir}`)
    process.exit(1)
  }
  walk(dir)
}
for (const rel of ROOT_FILES) {
  const file = path.join(repoRoot, rel)
  if (fs.existsSync(file)) check(file)
}

const exemptNote = skippedFiles > 0 ? `（豁免 docs/archive/ 下 ${skippedFiles} 个文件）` : ''
console.log(`扫描相对链接 ${total} 条，失效 ${bad.length} 条${exemptNote}`)
for (const { from, to } of bad) {
  console.log(`  ${from}  ->  ${to}`)
}
if (bad.length > 0) {
  console.log('\n提示：活文档的失效链接必须修（多为文档移动 / 改名后未更新入链）。')
}

process.exit(STRICT && bad.length > 0 ? 1 : 0)
