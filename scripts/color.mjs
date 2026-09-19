#!/usr/bin/env node
/**
 * color — 终端配色（`scripts/` 下所有工具的唯一实现）
 *
 * **本文件是库，不是工具**：没有入口、不读 argv、不写 stdout。`scripts/` 下直接
 * `node color.mjs` 什么也不会发生。
 *
 * ## 为什么收成一个模块
 *
 * 此前这段样板在 6 个脚本里**逐字复制**（`check-commit-msg` / `gate` / `grep-audit` /
 * `mechanism-audit` / `plugin-entry-audit` / `test-layout-audit`），另有 2 个写法各异的
 * 变体（`doc-find` 的对象表、`style-audit` 的预渲染标签）。
 *
 * 复制出来的副本**必然漂移**，而且已经漂了：`protocol-mirror-audit.mjs` 那份把
 * `\x1b` 丢了，写成 `` `[31m${s}[0m` ``——终端里显示的是**字面量** `[31m` 而不是红色。
 * 更麻烦的是它**自己看不见**：那个脚本的两个回归测试都设了 `NO_COLOR=1`，走的正好是
 * 不上色那条分支 ⇒ 坏掉的那条分支永远不被执行。这类「守卫自己的输出坏了但守卫仍绿」
 * 只有把实现收成一份才能根治。
 *
 * ## 两条规则
 *
 * 1. **非 TTY 一律不上色**。输出被管道接走时（`| tail`、CI 日志、被 `gate.mjs`
 *    spawn）`process.stdout.isTTY` 为假。不上色不只是好看：`gate.mjs` 要用正则从
 *    子进程输出里抓数字，转义码会把正则**打断**。
 * 2. **`NO_COLOR` 非空即关闭**。这是 no-color.org 的规定：变量**存在且非空**
 *    （不论取值）即禁用 ANSI 颜色，空串视同未设置。
 *
 * ## 用法
 *
 *   import { red, green, dim } from './color.mjs'
 *   console.log(red('不一致'), dim('（细节）'))
 *
 * `paint` 是工厂（`paint(code)(s)`）；不上色时原样返回，故调用方**不需要**自己判
 * `useColor`——这正是让「两处判断」不会漂成「一处对一处错」的原因。
 */

/** 是否上色：非 TTY、或 `NO_COLOR` 存在且非空 ⇒ 否 */
export const useColor = Boolean(process.stdout.isTTY) && !process.env.NO_COLOR

/**
 * 生成一个上色函数。`code` 是 SGR 参数（`'0;31'` / `'31'` / `'2'` / `'1'` 都可以，
 * `0;` 前缀可省）。
 */
export const paint = (code) => (s) => (useColor ? `\x1b[${code}m${s}\x1b[0m` : s)

export const red = paint('0;31')
export const green = paint('0;32')
export const yellow = paint('0;33')
export const cyan = paint('0;36')
export const dim = paint('2')
export const bold = paint('1')

/**
 * ANSI 转义序列。子进程带颜色输出时，正则会被转义码打断，必须先剥离再匹配
 * （`gate.mjs` 从 cargo / vitest 输出里抓数字就靠它）。
 */
const ANSI_RE = /\x1b\[[0-9;]*[A-Za-z]/g
export const stripAnsi = (s) => s.replace(ANSI_RE, '')
