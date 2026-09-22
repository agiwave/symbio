// backend 阶段：symbio 与 cli（独立 workspace）的 cargo 检查链。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { yellow } from '../color.mjs'
import { BASELINE, grabInt, sumInt } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const backendDir = path.join(repoRoot, 'symbio')
const cliDir = path.join(repoRoot, 'cli')

export default {
  id: 'backend',
  title: '后端（cargo）',
  tasks(ctx) {
    const tasks = []
    if (!fs.existsSync(path.join(backendDir, 'Cargo.toml'))) return tasks

    if (ctx.fix) {
      tasks.push({ label: 'cargo fmt --all（--fix）', cmd: 'cargo', args: ['fmt', '--all'], cwd: backendDir })
    }
    tasks.push({ label: 'cargo check --tests', cmd: 'cargo', args: ['check', '--tests'], cwd: backendDir })

    const testArgs = ctx.ci ? ['test', '--workspace'] : ['test', '--lib']
    tasks.push({
      label: `cargo ${testArgs.join(' ')}`,
      // 自定义任务而非声明式命令：同一次运行既判退出码又解析通过数做基线棘轮，
      // 避免为拿输出再跑一遍测试。
      run: async () => {
        const r = await ctx.run({
          label: `cargo ${testArgs.join(' ')}`,
          cmd: 'cargo',
          args: testArgs,
          cwd: backendDir,
        })
        if (!r.ok) {
          const note = r.timedOut ? '超时终止' : `exit=${r.code}${r.signal ? `, ${r.signal}` : ''}`
          return { ok: false, note }
        }
        if (ctx.ci) {
          // CI 跑全量（含集成测试）：每个测试目标各打一行 ⇒ 求和；数字仅作信息展示
          const total = sumInt(r.output, /test result: ok\. (\d+) passed/)
          if (total !== null) console.log(`      ${total} passed（--workspace 全量；只信退出码）`)
          return { ok: true }
        }
        const passed = grabInt(r.output, /test result: ok\. (\d+) passed/)
        if (passed === null) return { ok: true, note: '未能解析通过数' }
        if (passed < BASELINE.rustTests) {
          return { ok: false, note: `通过数 ${passed} < 基线 ${BASELINE.rustTests}（有测试被删或失败）` }
        }
        if (passed > BASELINE.rustTests) {
          console.log(
            yellow(
              `      ⚠ 通过数 ${passed} > 基线 ${BASELINE.rustTests}：请更新 scripts/gate.d/_shared.mjs 的 BASELINE.rustTests`,
            ),
          )
          return { ok: true, note: `通过数 ${passed}（基线待更新）` }
        }
        console.log(`      ${passed} passed（基线 ${BASELINE.rustTests}）`)
        return { ok: true }
      },
    })

    tasks.push({
      label: 'cargo clippy --all-targets -- -D warnings',
      cmd: 'cargo',
      args: ['clippy', '--all-targets', '--', '-D', 'warnings'],
      cwd: backendDir,
    })
    tasks.push({
      label: 'cargo fmt --all -- --check',
      cmd: 'cargo',
      args: ['fmt', '--all', '--', '--check'],
      cwd: backendDir,
      when: () => !ctx.fix,
    })
    if (ctx.profile) {
      tasks.push({
        label: `cargo build --profile ${ctx.profile}`,
        cmd: 'cargo',
        args: ['build', '--profile', ctx.profile],
        cwd: backendDir,
      })
    }

    // cli/ 是独立 workspace（仓库根没有 Cargo.toml）：存在就一并检查。
    if (fs.existsSync(path.join(cliDir, 'Cargo.toml'))) {
      if (ctx.fix) {
        tasks.push({ label: 'cli: cargo fmt --all（--fix）', cmd: 'cargo', args: ['fmt', '--all'], cwd: cliDir })
      }
      for (const [label, args] of [
        ['cli: cargo check --tests', ['check', '--tests']],
        ['cli: cargo clippy --all-targets -- -D warnings', ['clippy', '--all-targets', '--', '-D', 'warnings']],
        ['cli: cargo fmt --all -- --check', ['fmt', '--all', '--', '--check']],
      ]) {
        tasks.push({ label, cmd: 'cargo', args, cwd: cliDir })
      }
    }
    return tasks
  },
}
