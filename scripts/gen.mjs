#!/usr/bin/env node
/**
 * gen — 显式执行「确定性的机械工作」：格式化 + 事实生成链，并把改写**暂存**。
 *
 * ## 它替掉了什么
 *
 * `gate.mjs` 现在**本地默认自动执行并暂存**这两个动作，所以「忘了按按钮」不会再红。
 * `gen.mjs` 保留为一条窄入口：只跑格式化与事实生成，不跑其余门禁；`--write` 显式声明
 * 执行模式，避免与 `--check` / `--ci` 的只读语义混淆。
 *
 * 原先门禁把执行与判定拆开，是为了让「绿」等于「工作树没被改过」。实际代价是：生成器
 * 已经跑完，门禁却把产物还原，再要求人重跑同一个动作。机械工作能确定地自动做完，就
 * 不该把「人忘了按按钮」当成红灯；需要可复现的只读检查时显式用 `gate.mjs --check`。
 *
 * ## 为什么带 `--full`
 *
 * 生成物与「这一轮改了哪个目录」无关：改了 front-matter 但没碰 `crates/` 时，
 * 按变更范围收窄会让生成链整段跳过，汇总页就留在旧状态。生成是便宜且幂等的，
 * 显式命令不参与收窄。
 */
// @gate off

import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')
const gate = path.join(scriptDir, 'gate.mjs')

const r = spawnSync(process.execPath, [gate, '--only=fmt,facts-write', '--write', '--full'], {
  cwd: repoRoot,
  stdio: 'inherit',
  env: process.env,
})
if (r.status !== 0) {
  console.error('\ngen 未通过：上面的失败项还没被修复。')
  process.exit(r.status ?? 1)
}
process.exit(0)
