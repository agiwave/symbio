// `scripts/gate.mjs` 的回归测试。
//
// 这个脚本的失效方式不是「报错」而是「代价」：`--help` 不被识别时，它会照常跑完
// **全量门禁**（10+ 分钟）并把 `.workbuddy-ai/gate-logs/*.log` 覆盖掉——查一次用法，
// 代价是丢掉上一次的失败日志（最需要它的那一刻它没了，只能重跑）。
//
// 与 `commit.mjs` 同类陷阱（那里 `--help` 会**真的提交一次**）。两处的判据形态相同：
// 退出 0 + 打印用法 + **没有发生真实动作**（这里 = 没进门禁主流程 / 没跑任何阶段）。
//
// 跑法：node --test scripts/gate.test.mjs

import { test } from 'node:test'
import assert from 'node:assert/strict'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { stripAnsi } from './color.mjs'

const repoRoot = path.resolve(import.meta.dirname, '..')
const gateScript = path.join(repoRoot, 'scripts', 'gate.mjs')

/**
 * 跑 `gate.mjs`：stdin 给 `'ignore'`（真交互 / 真等待会挂），并给**显式 timeout**——
 * 失败要快，不许把「白跑十分钟门禁」这个 bug 拖成一次十分钟的测试。
 */
function runGate(args = []) {
  return spawnSync(process.execPath, [gateScript, ...args], {
    cwd: repoRoot,
    encoding: 'utf8',
    timeout: 60_000,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
}

test('★ `--help` 只打印用法并退出 0，不跑任何阶段（否则查用法 = 白跑十分钟 + 覆盖日志）', () => {
  const r = runGate(['--help'])
  assert.equal(r.status, 0, `--help 应退出 0：${r.stdout}\n${r.stderr}`)
  const out = stripAnsi(r.stdout)
  assert.match(out, /用法：node scripts\/gate\.mjs/, '应打印用法')
  assert.match(out, /--only=/, '用法应列出主要开关')
  assert.match(out, /--list/, '用法应提到 --list')

  // 这两条才是补 `--help` 的**理由**：原先它会照常走完全量门禁。
  assert.doesNotMatch(out, /══ 门禁 ══/, '不得进入门禁主流程')
  assert.doesNotMatch(out, /── 阶段 \d+\/\d+/, '不得执行任何阶段')
})

test('`--list` 仍只列阶段与任务、不执行（用法自述不改变既有开关）', () => {
  const r = runGate(['--list'])
  assert.equal(r.status, 0, `--list 应退出 0：${r.stdout}\n${r.stderr}`)
  const out = stripAnsi(r.stdout)
  assert.match(out, /^1\. /m, `应列出阶段：\n${out}`)
  assert.doesNotMatch(out, /══ 门禁 ══/, '--list 不该进入主流程')
})
