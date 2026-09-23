// backend 阶段：symbio 与 cli（独立 workspace）的 cargo 检查链。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { BASELINE, autoWork, cargoTestRatchet } from './_shared.mjs'

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

    // 格式化是**门禁自己做的事**，不是判它「有没有做过」——见 `_shared.autoWork`。
    // 放在最前：后面所有检查都跑在格式化后的代码上，避免「先报 clippy 再格式化」
    // 这种让人以为要改两遍的顺序。
    tasks.push({
      label: 'cargo fmt --all（自动格式化）',
      run: (c) => autoWork(c, { label: 'cargo fmt --all', cmd: 'cargo', args: ['fmt', '--all'], cwd: backendDir }),
    })
    tasks.push({ label: 'cargo check --tests', cmd: 'cargo', args: ['check', '--tests'], cwd: backendDir })

    const testArgs = ctx.ci ? ['test', '--workspace'] : ['test', '--lib']
    tasks.push(
      cargoTestRatchet(ctx, {
        label: `cargo ${testArgs.join(' ')}`,
        cwd: backendDir,
        args: testArgs,
        baseline: BASELINE.rustTests,
        baselineName: 'rustTests',
      }),
    )

    tasks.push({
      label: 'cargo clippy --all-targets -- -D warnings',
      cmd: 'cargo',
      args: ['clippy', '--all-targets', '--', '-D', 'warnings'],
      cwd: backendDir,
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
      tasks.push({
        label: 'cli: cargo fmt --all（自动格式化）',
        run: (c) => autoWork(c, { label: 'cli: cargo fmt --all', cmd: 'cargo', args: ['fmt', '--all'], cwd: cliDir }),
      })
      for (const [label, args] of [
        ['cli: cargo check --tests', ['check', '--tests']],
        ['cli: cargo clippy --all-targets -- -D warnings', ['clippy', '--all-targets', '--', '-D', 'warnings']],
      ]) {
        tasks.push({ label, cmd: 'cargo', args, cwd: cliDir })
      }
      // **必须真的跑一遍**：上面两条都只编译不执行。cli 是独立 workspace，
      // symbio 那条 `cargo test` 跑不到这里 —— 漏掉这一步，cli 的测试可以
      // 一直失败而门禁全绿（这正是本仓最忌讳的那类「看起来跑了、其实没跑」）。
      const cliTestArgs = ctx.ci ? ['test', '--workspace'] : ['test']
      tasks.push(
        cargoTestRatchet(ctx, {
          label: `cli: cargo ${cliTestArgs.join(' ')}`,
          cwd: cliDir,
          args: cliTestArgs,
          baseline: BASELINE.cliRustTests,
          baselineName: 'cliRustTests',
        }),
      )
    }
    return tasks
  },
}
