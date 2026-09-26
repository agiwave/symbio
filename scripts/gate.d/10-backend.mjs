// backend 阶段：三个**独立 cargo workspace** 的检查链。
//
// 仓库根没有 `Cargo.toml`：`symbio/`、`cli/`、`tauri/src-tauri/` 各一份清单，
// **在 A 下跑 `cargo` 跑不到 B 的测试**。因此每个 workspace 都必须能指到本阶段里
// 具体某一步——指不到就是「看起来跑了、其实没跑」（本仓最忌讳的那类失效）。
//
// ## 为什么 `tauri/src-tauri` 必须在这里
//
// 它此前**完全不在任何扫描范围**：不 fmt、不 check、不 clippy、不 test。而它
// `use symbio::…`，是 `symbio` 公开面的**跨 crate 消费方**——一次 API 改名可以让
// 壳编译失败而门禁全绿。`scripts/tauri-binary.mjs` 确实会构建壳，但它只在
// **手工**跑它时才构建（门禁只跑它的回归测试），所以那不算覆盖。
//
// 壳自己没有测试（`cargo test` 打 `0 passed`），但 `check` / `clippy` 抓的是
// **编译期契约**，那才是它作为消费方最该被守的东西；`cargo test` 一并接上，
// 于是「以后加了测试却没人跑」这条坑不会重新出现（`tauriRustTests` 基线从 0 起，
// 一旦有人加用例，棘轮立刻要求更新基线）。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { BASELINE, autoWork, cargoTestRatchet } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const backendDir = path.join(repoRoot, 'symbio')

// `label` 是日志/汇总里的前缀；`baselineName` 指向 `_shared.BASELINE` 的字段
const EXTRA_WORKSPACES = [
  { rel: 'cli', label: 'cli', baselineName: 'cliRustTests' },
  { rel: 'tauri/src-tauri', label: 'tauri/src-tauri', baselineName: 'tauriRustTests' },
]

/** `-D rustdoc::broken_intra_doc_links` —— 文档断链也是**可判定**的，见下方注释 */
const DOC_LINT_ENV = { RUSTDOCFLAGS: '-D rustdoc::broken_intra_doc_links' }

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

    // 文档断链：`broken_intra_doc_links` 是 **warn-by-default** 的 lint，而没有任何
    // 门禁跑过 `cargo doc` ⇒ 39 条断链长期无人知，其中还混着**指向已删除函数**的
    // （`emit_converge`、`VdfsProvider::write` / `delete` / `action`——该 trait 早已
    // 只剩 `dispatch`）。读文档的人被指到一个不存在的东西上，而这**不产生任何告警**，
    // 只能靠跑一次把它变成红灯。判据是 zero-tolerance（没有「几条以内可接受」）。
    tasks.push({
      label: 'cargo doc --no-deps（-D rustdoc::broken_intra_doc_links）',
      cmd: 'cargo',
      args: ['doc', '--no-deps'],
      cwd: backendDir,
      env: DOC_LINT_ENV,
    })

    if (ctx.profile) {
      tasks.push({
        label: `cargo build --profile ${ctx.profile}`,
        cmd: 'cargo',
        args: ['build', '--profile', ctx.profile],
        cwd: backendDir,
      })
    }

    // 另外两个独立 workspace：存在就一并检查（判据与 `symbio` 同源，逐条对齐）
    for (const { rel, label, baselineName } of EXTRA_WORKSPACES) {
      const dir = path.join(repoRoot, rel)
      if (!fs.existsSync(path.join(dir, 'Cargo.toml'))) continue

      tasks.push({
        label: `${label}: cargo fmt --all（自动格式化）`,
        run: (c) => autoWork(c, { label: `${label}: cargo fmt --all`, cmd: 'cargo', args: ['fmt', '--all'], cwd: dir }),
      })
      for (const [name, args] of [
        ['check --tests', ['check', '--tests']],
        ['clippy --all-targets -- -D warnings', ['clippy', '--all-targets', '--', '-D', 'warnings']],
        ['doc --no-deps（-D rustdoc::broken_intra_doc_links）', ['doc', '--no-deps']],
      ]) {
        tasks.push({
          label: `${label}: cargo ${name}`,
          cmd: 'cargo',
          args,
          cwd: dir,
          ...(name.startsWith('doc') ? { env: DOC_LINT_ENV } : {}),
        })
      }
      // **必须真的跑一遍**：上面几条都只编译不执行。漏掉这一步，该 workspace 的
      // 测试可以一直失败而门禁全绿（这正是本仓最忌讳的那类「看起来跑了、其实没跑」）。
      const extraTestArgs = ctx.ci ? ['test', '--workspace'] : ['test']
      tasks.push(
        cargoTestRatchet(ctx, {
          label: `${label}: cargo ${extraTestArgs.join(' ')}`,
          cwd: dir,
          args: extraTestArgs,
          baseline: BASELINE[baselineName],
          baselineName,
        }),
      )
    }
    return tasks
  },
}
