#!/usr/bin/env node
/**
 * gen — 显式执行「确定性的机械工作」：格式化 + 事实生成链，并把改写**暂存**。
 *
 * ## 它替掉了什么
 *
 * `gate.mjs` 原先在本地跑时**自动执行并暂存**这两个动作（`cargo fmt` / 事实生成链）。
 * 好处是「不因为忘了按一次按钮而红」；代价是**全绿时工作树已经被改过**，
 * 「绿」与「没被改」分不开，审计也不再可复现。现在拆成两条路：
 *
 * - `gate.mjs`（默认）只判：跑完修复动作、用内容哈希比对，有改写就**还原**并判红；
 * - `node scripts/gen.mjs`（本文件）执行：改写留在工作区并 `git add`，供提交带上。
 *
 * 提交点的 `pre-commit` 钩子先跑本命令、再跑 `gate --preset=pre-commit --ci`，
 * 所以「忘了按按钮」仍然不会红——钩子替你按。
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
