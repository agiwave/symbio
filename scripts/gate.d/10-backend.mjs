// backend 阶段：根 workspace（仓库根的 `Cargo.toml`）统一检查链。
//
// 仓库根现在有一个 `Cargo.toml` `[workspace]`，成员为 `symbio`、`cli`、`tauri/src-tauri`，
// 三者共用一个 `target/`。因此 `cargo … --workspace` 一次就能覆盖全部，不必再像
// 从前那样分三个独立 workspace 各跑一遍（`symbio`/cli/tauri 在 A 下跑不到 B 的测试）。
//
// 测试通过数仍**按 crate 分包**判定（`rustTests` / `cliRustTests` / `tauriRustTests` 三套
// 基线），本地用 `cargo test -p <pkg>` 各自比对；CI 跑一次 `--workspace` 全量求和即可
// （CI 分支只信退出码，不比较基线）。分包的动机不变：壳 `use symbio::…` 是跨 crate 消费方，
// 它的编译期契约（`check`/`clippy`）最该被守，而 `tauriRustTests` 基线从 0 起——有人加
// 用例时棘轮立刻要求更新基线，避免「加了测试却没人跑」。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { BASELINE, autoWork, cargoTestRatchet } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
// 根 workspace：所有 cargo 命令都从仓库根发起，靠 `--workspace` 覆盖全部成员。
const backendDir = repoRoot

// 本地按 crate 分包跑测试以保留各自基线；CI 跑一次全量。
// 包名取自各成员 Cargo.toml 的 [package].name。
const TEST_PKGS = [
  { pkg: 'symbio', baselineName: 'rustTests', baseline: BASELINE.rustTests },
  { pkg: 'symbio-cli', baselineName: 'cliRustTests', baseline: BASELINE.cliRustTests },
  { pkg: 'symbio-tauri', baselineName: 'tauriRustTests', baseline: BASELINE.tauriRustTests },
]

/** `-D rustdoc::broken_intra_doc_links` —— 文档断链也是**可判定**的，见下方注释 */
const DOC_LINT_ENV = { RUSTDOCFLAGS: '-D rustdoc::broken_intra_doc_links' }

export default {
  id: 'backend',
  title: '后端（cargo，根 workspace）',
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
    tasks.push({ label: 'cargo check --tests --workspace', cmd: 'cargo', args: ['check', '--tests', '--workspace'], cwd: backendDir })

    // 测试：CI 跑一次 `--workspace` 全量（只信退出码、求和打印）；本地按 crate 分包比对基线。
    if (ctx.ci) {
      tasks.push(
        cargoTestRatchet(ctx, {
          label: 'cargo test --workspace（全量）',
          cwd: backendDir,
          args: ['test', '--workspace'],
          baseline: BASELINE.rustTests,
          baselineName: 'rustTests',
        }),
      )
    } else {
      for (const { pkg, baselineName, baseline } of TEST_PKGS) {
        tasks.push(
          cargoTestRatchet(ctx, {
            label: `cargo test -p ${pkg}`,
            cwd: backendDir,
            args: ['test', '-p', pkg],
            baseline,
            baselineName,
          }),
        )
      }
    }

    tasks.push({
      label: 'cargo clippy --all-targets --workspace -- -D warnings',
      cmd: 'cargo',
      args: ['clippy', '--all-targets', '--workspace', '--', '-D', 'warnings'],
      cwd: backendDir,
    })

    // 文档断链：`broken_intra_doc_links` 是 **warn-by-default** 的 lint，而没有任何
    // 门禁跑过 `cargo doc` ⇒ 39 条断链长期无人知，其中还混着**指向已删除函数**的
    // （`emit_converge`、`VdfsProvider::write` / `delete` / `action`——该 trait 早已
    // 只剩 `dispatch`）。读文档的人被指到一个不存在的东西上，而这**不产生任何告警**，
    // 只能靠跑一次把它变成红灯。判据是 zero-tolerance（没有「几条以内可接受」）。
    tasks.push({
      label: 'cargo doc --no-deps --workspace（-D rustdoc::broken_intra_doc_links）',
      cmd: 'cargo',
      args: ['doc', '--no-deps', '--workspace'],
      cwd: backendDir,
      env: DOC_LINT_ENV,
    })

    if (ctx.profile) {
      // 仅构建后端（symbio + cli），壳的发布构建由 release.yml 的 tauri-action 负责。
      tasks.push({
        label: `cargo build --profile ${ctx.profile} -p symbio -p symbio-cli`,
        cmd: 'cargo',
        args: ['build', '--profile', ctx.profile, '-p', 'symbio', '-p', 'symbio-cli'],
        cwd: backendDir,
      })
    }
    return tasks
  },
}
