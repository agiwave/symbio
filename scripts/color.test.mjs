/**
 * color.mjs 的回归测试 + 「不许手写 ANSI 转义」守卫
 *
 * ## 为什么要有后者
 *
 * 这段配色的原始事故不是「写错了」，而是**写错了没人发现**：
 * `protocol-mirror-audit.mjs` 手写了一份，把 `\x1b` 写丢（`` `[31m${s}[0m` ``），
 * 终端与 CI 日志里显示字面量 `[31m`；而它的两个回归测试都设了 `NO_COLOR=1`，
 * 走的正好是**不上色**那条分支 ⇒ 坏掉的分支永远不被执行。
 *
 * 所以这里守两件事：
 * 1. **模块本身**在四种组合（TTY / 非 TTY × NO_COLOR 有 / 无）下行为正确——
 *    特别是**上色那条分支**（旧测试从没跑过它）。
 * 2. **`scripts/` 下不许再手写转义**。收成一份实现是根治，但"收成一份"这件事本身
 *    也会被人遗忘——这条守卫负责让遗忘变红。
 *
 * 跑法：node --test scripts/color.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const colorPath = path.join(scriptDir, 'color.mjs')

/** 真 ESC（U+001B）——手写转义漏掉的就是它 */
const ESC = '\u001b'

/**
 * 在指定环境下**重新求值**一次 `color.mjs`。
 *
 * `useColor` 是模块顶层的常量，同一个模块实例只会算一次；故用查询串打破 ESM 缓存，
 * 拿到一个全新实例。赋值 `process.stdout.isTTY` 必须在 import **之前**——`paint`
 * 闭包捕获的是 import 那一刻的 `useColor`。
 */
async function loadColor({ isTTY, noColor }) {
  const prevTTY = process.stdout.isTTY
  const prevNoColor = process.env.NO_COLOR
  process.stdout.isTTY = isTTY
  if (noColor === undefined) delete process.env.NO_COLOR
  else process.env.NO_COLOR = noColor
  try {
    const tag = `${isTTY ? 'tty' : 'pipe'}-${noColor === undefined ? 'unset' : `nc(${noColor})`}`
    return await import(`${pathToFileURL(colorPath).href}?case=${encodeURIComponent(tag)}`)
  } finally {
    process.stdout.isTTY = prevTTY
    if (prevNoColor === undefined) delete process.env.NO_COLOR
    else process.env.NO_COLOR = prevNoColor
  }
}

// ==================== 模块本身 ====================

test('非 TTY（管道 / CI 日志）→ 一律不上色', async () => {
  const { red, dim, useColor } = await loadColor({ isTTY: false })
  assert.equal(useColor, false)
  assert.equal(red('不一致'), '不一致')
  assert.equal(dim('细节'), '细节')
})

test('TTY 且未设 NO_COLOR → **真的**上色（旧测试从没跑过这条分支）', async () => {
  const { red, dim, useColor } = await loadColor({ isTTY: true, noColor: undefined })
  assert.equal(useColor, true)
  assert.equal(red('x'), `${ESC}[0;31mx${ESC}[0m`)
  assert.equal(dim('x'), `${ESC}[2mx${ESC}[0m`)
})

test('TTY 但 NO_COLOR=1 → 关闭上色', async () => {
  const { red, useColor } = await loadColor({ isTTY: true, noColor: '1' })
  assert.equal(useColor, false)
  assert.equal(red('x'), 'x')
})

test('NO_COLOR 为空串 → 视同未设置（no-color.org 规定「存在且非空」才关闭）', async () => {
  const { red, useColor } = await loadColor({ isTTY: true, noColor: '' })
  assert.equal(useColor, true)
  assert.equal(red('x'), `${ESC}[0;31mx${ESC}[0m`)
})

test('paint 是工厂：`code` 原样透传（`31` 与 `0;31` 都是合法 SGR，视觉等价）', async () => {
  const { paint } = await loadColor({ isTTY: true, noColor: undefined })
  assert.equal(paint('31')('x'), `${ESC}[31mx${ESC}[0m`)
  assert.equal(paint('0;31')('x'), `${ESC}[0;31mx${ESC}[0m`)
})

test('stripAnsi 剥掉转义序列（gate 从子进程输出里抓数字靠它）', async () => {
  const { stripAnsi, red } = await loadColor({ isTTY: true, noColor: undefined })
  assert.equal(stripAnsi(red('abc')), 'abc')
  assert.equal(stripAnsi(`a${ESC}[0;32mb${ESC}[0mc`), 'abc')
  // 正则不能被转义码打断——这正是必须剥离的理由
  assert.equal(stripAnsi(`${ESC}[32mErrors: 3${ESC}[0m`).match(/Errors: (\d+)/)[1], '3')
})

// ==================== 守卫：不许再手写转义 ====================

/**
 * 守卫自身必须能提到这些模式才能检出它们，故**豁免本文件**（`color.mjs` 是唯一实现，
 * 同样豁免）。豁免的是"提到"，不是"使用"——生产脚本里出现真转义照样红。
 */
const GUARD_SELF = 'color.test.mjs'

/**
 * 一行是否"手写了 ANSI"。三种写法都算：
 * 1. 真 ESC 字符（`\u001b[31m`）；
 * 2. `\x1b` 转义文本（`\x1b[31m`）——**复制正确的实现**；
 * 3. SGR 形状的字面量（`[31m` / `[0m` / `[2m` / `[1m`）——**原始事故**：
 *    漏掉 ESC 的写法不含 `\x1b`，规则 2 抓不到它，只有这条能抓。
 *
 * 注释行豁免：解释"不要这么写"时必须能写出这个模式。
 */
function handRolledAnsiLines(src) {
  return src.split(/\r?\n/).filter((line) => {
    const t = line.trim()
    if (t.startsWith('//') || t.startsWith('*') || t.startsWith('/*')) return false
    if (line.includes(ESC) || line.includes('\\x1b')) return true
    return /\[3[0-9]m|\[0m|\[2m|\[1m/.test(line)
  })
}

test('scripts/ 下除 color.mjs 外不得手写 ANSI 转义 → 违规即红', () => {
  const offenders = []
  for (const name of fs.readdirSync(scriptDir)) {
    if (!name.endsWith('.mjs') || name === 'color.mjs' || name === GUARD_SELF) continue
    const hits = handRolledAnsiLines(fs.readFileSync(path.join(scriptDir, name), 'utf8'))
    if (hits.length) offenders.push(`${name}（${hits.length} 行）`)
  }
  assert.deepEqual(
    offenders,
    [],
    `以下脚本手写了 ANSI 转义，应改为 import { red, green, yellow, dim, bold, paint } from './color.mjs'：\n  ${offenders.join('\n  ')}`,
  )
})

test('守卫本身有效：三种手写写法都要检出，正确用法与注释不得误伤', () => {
  // 1. 真 ESC
  assert.equal(handRolledAnsiLines(`const red = (s) => \`${ESC}[31m\${s}${ESC}[0m\``).length, 1)
  // 2. `\x1b` 文本
  assert.equal(handRolledAnsiLines('const red = (s) => `\\x1b[31m${s}\\x1b[0m`').length, 1)
  // 3. **原始事故**：漏了 ESC 的那一份（`[31m` 少了 `\x1b`）
  assert.equal(handRolledAnsiLines('const red = (s) => `[31m${s}[0m`').length, 1)
  // 正确用法：import 共享模块
  assert.equal(handRolledAnsiLines("import { red } from './color.mjs'").length, 0)
  // 注释里的提及不算违规
  assert.equal(handRolledAnsiLines('// 不要写 \\x1b[31m，走 color.mjs').length, 0)
  assert.equal(handRolledAnsiLines(' * 例如 `[31m${s}[0m` 就是坏的').length, 0)
})
