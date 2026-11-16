#!/usr/bin/env node
/**
 * frontend-ux-audit — 前端可用性的机械判据（提交门禁）
 *
 * ## 它判什么
 *
 * | 编号 | 规则 | 它拦的是哪一种回退 |
 * |---|---|---|
 * | UX-001 | 前端源码不得调用 `alert` / `confirm` / `prompt` | 浏览器原生弹窗会冻结 WebView、样式与主题脱节，且无法做焦点管理；仓里已有 `ConfirmDialog` / `useToast` 两条统一出口 |
 * | UX-002 | 图标按钮必须有可访问名称（`title` / `aria-label` / `aria-labelledby`） | 纯 SVG 按钮没有文字，读屏与键盘用户只知道「按钮」；鼠标用户也没有悬停说明 |
 * | UX-003 | 表单控件必须有可访问名称（`aria-label` / `aria-labelledby` / `label[for]` / 包裹 `<label>`） | 输入框只剩 placeholder；placeholder 不是标签，输入后即消失 |
 * | UX-004 | `<img>` 必须有 `alt`（空串也算显式声明） | 图片加载失败 / 读屏时没有替代文本 |
 *
 * ## 为什么是这四条
 *
 * 前端体验的大部分问题（加载态、空态、错误展示、防重复提交）**无法**靠静态正则可靠
 * 判断；硬判只会制造大量豁免，最后一条都不红。这里只收「有/没有」两态清晰、且修复
 * 成本低的四条。其余靠 `frontend-static` 的单测 / 组件测试与人工评审——判据宁可少，
 * 不许假。
 *
 * ## 用法
 *   node scripts/audits/frontend-ux-audit.mjs                 # 审计 tauri/src
 *   node scripts/audits/frontend-ux-audit.mjs --root=<dir>    # 换仓库根（回归测试用）
 *
 * 退出码：0 = 通过；1 = 存在 ERROR；2 = 审计范围读不出
 *
 * 约定：平台无关（Node.js ESM、零外部依赖）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns UX 前端可用性（原生弹窗 / 可访问名称 / 图片替代文本）
// @codes UX-001 UX-002 UX-003 UX-004
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { paint } from '../color.mjs'
import { walkFiles } from '../fs-walk.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..', '..')
const SRC = path.join(ROOT, 'tauri', 'src')

const red = paint('31')('ERROR')
const green = paint('32')('OK')
const rel = (p) => path.relative(ROOT, p).split(path.sep).join('/')

const errors = []

/** SFC 的模板区（顶层唯一的 `<template>`；取最后一个闭合标签，避免嵌套 `<template v-if>` 截断） */
function templateOf(src) {
  const start = src.indexOf('<template>')
  const end = src.lastIndexOf('</template>')
  return start === -1 || end === -1 ? '' : src.slice(start + '<template>'.length, end)
}

/** 是否带可访问名称（静态或绑定都算） */
const hasAccessibleName = (attrs) =>
  /(?:^|\s)(?::|v-bind:)?(?:aria-label|aria-labelledby|title)\s*=/.test(attrs)

/** 去掉标签 / mustache 后还剩不剩可见文字 */
function visibleText(inner) {
  return inner
    .replace(/<[^>]*>/g, '')
    .replace(/\{\{[\s\S]*?\}\}/g, 'x')
    .replace(/&[a-z#0-9]+;/gi, 'x')
    .trim()
}

/** 控件是否被 `<label for="id">` 关联 */
function hasLabelFor(template, attrs) {
  const id = /\bid\s*=\s*"([^"]+)"/.exec(attrs)?.[1]
  if (!id) return false
  return new RegExp(`<label\\b[^>]*\\bfor\\s*=\\s*"${id.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}"`).test(
    template,
  )
}

/** 控件是否被 `<label>` 包住（粗略但足够：最近一个 `<label` 之后没有闭合） */
function hasWrappingLabel(template, index) {
  const before = template.slice(0, index)
  return before.lastIndexOf('<label') > before.lastIndexOf('</label>')
}

function auditFile(file) {
  const src = fs.readFileSync(file, 'utf8')
  const template = templateOf(src)
  const templateStart = src.indexOf('<template>')
  const lineBase = templateStart === -1 ? 0 : src.slice(0, templateStart).split('\n').length
  const lineAt = (index) => lineBase + template.slice(0, index).split('\n').length
  const name = rel(file)

  // UX-001：原生阻塞弹窗（脚本区也要查；`confirm(` 前不能是标识符 / 点号）
  const dialog = /(?<![\w.$])(?:window\.)?(?:alert|confirm|prompt)\s*\(/.exec(src)
  if (dialog) {
    const line = src.slice(0, dialog.index).split('\n').length
    errors.push(`${red} ${name}:${line} UX-001 使用了原生弹窗（用 ConfirmDialog / useToast）`)
  }

  // UX-002：图标按钮必须有可访问名称
  for (const m of template.matchAll(/<button\b([^>]*)>([\s\S]*?)<\/button>/g)) {
    const attrs = m[1]
    const text = visibleText(m[2])
    if (!text && !hasAccessibleName(attrs)) {
      errors.push(`${red} ${name}:${lineAt(m.index)} UX-002 无文字按钮缺少 title / aria-label`)
    }
  }

  // UX-003：表单控件的可访问名称
  for (const m of template.matchAll(/<(input|textarea|select)\b([^>]*)>/g)) {
    const tag = m[1]
    const attrs = m[2]
    if (/\btype\s*=\s*"hidden"/.test(attrs)) continue
    if (hasAccessibleName(attrs)) continue
    if (hasLabelFor(template, attrs)) continue
    if (hasWrappingLabel(template, m.index)) continue
    errors.push(
      `${red} ${name}:${lineAt(m.index)} UX-003 <${tag}> 缺少可访问名称（aria-label / label[for] / 包裹 label）`,
    )
  }

  // UX-004：图片替代文本（`alt=""` 是显式声明，合法）
  for (const m of template.matchAll(/<img\b([^>]*)>/g)) {
    if (/(?:^|\s)(?::|v-bind:)?alt\s*=/.test(m[1])) continue
    errors.push(`${red} ${name}:${lineAt(m.index)} UX-004 <img> 缺少 alt（装饰图用 alt=""）`)
  }
}

const files = walkFiles(SRC, {
  exts: ['.vue'],
  skipDirs: new Set(['node_modules', '__tests__', 'dist']),
  skipHidden: true,
  tolerant: true,
})

if (files.length === 0) {
  console.error(`${red}: ${rel(SRC)} 下读不到任何 .vue —— 判据会静默全绿，先查路径 / 跳过规则`)
  process.exit(2)
}

for (const f of files) auditFile(f)

console.log(`\n== 前端可用性审计（${rel(SRC)} 下 ${files.length} 个 .vue）==\n`)
for (const e of errors) console.log(`  ${e}`)
console.log(`\n${green}结果:${paint('0')('')} ${errors.length} 个错误`)

process.exit(errors.length > 0 ? 1 : 0)
