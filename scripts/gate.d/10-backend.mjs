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
import { BASELINE, cargoTestRatchet } from './_shared.mjs'

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
  // 阶段级并发：本阶段只写 `target/`，与批内其他泳道无数据依赖——docs 守卫只读
  // 源码与 scripts，baseline 只读 git 历史与 scripts，msrv 用独立的
  // `.workbuddy-ai/msrv-target`（不与 backend 争 `.cargo-lock`）。fmt 已拆到
  // `05-fmt.mjs` 屏障阶段（rustfmt 就地重写非原子，必须先于一切读 `.rs` 的泳道）。
  // ⚠️ frontend 不能进这批：编译 symbio-tauri 时 `generate_context!` 在编译期读
  //    `tauri/dist`，而 vite build 会重写它——见 `56-frontend.mjs`。
  parallel: true,
  tasks(ctx) {
    const tasks = []
    if (!fs.existsSync(path.join(backendDir, 'Cargo.toml'))) return tasks

    // 格式化在 `05-fmt.mjs`（屏障阶段）：它必须先于本阶段与 docs 守卫完成，
    // 而本阶段现在与 docs 并发，fmt 若还留在这里就等于和读方赛跑。

    // ⚠️ 这里**刻意没有** `cargo check --tests --workspace`——它曾是本阶段的第一步。
    //
    // 它被当成「比 clippy 快的类型检查」放在最前，但两者的产物**互不复用**：clippy
    // 走 `clippy-driver`，fingerprint 带 lint 标记，`check` 编出来的缓存一律不命中。
    // 实测（2026-10-08，本机 Windows / 20 核，改一处 symbio 源码后）：
    //   cargo check  -p symbio --tests       → 13.3s
    //   cargo clippy -p symbio --all-targets → 19.1s（**完全重新 Checking**，非复用）
    // 而 `clippy --all-targets` 的 target 集合是 `check --tests` 的**超集**
    // （lib + bins + tests + examples + benches ⊇ lib + bins + tests），且 clippy
    // 本身含完整类型检查 ⇒ check 那一遍是纯冗余。删掉它，workspace 级每次少一整遍
    // 编译（本次实测该步 31s）。
    //
    // 需要「只要类型错误、不要 lint」的快速档时，正确做法是**单独跑** `cargo check`，
    // 而不是把它塞回门禁的必跑链里。

    // 测试：CI 跑一次 `--workspace` 全量（求和**并比对 CI 口径基线**）；本地按 crate 分包比对基线。
    if (ctx.ci) {
      tasks.push(
        cargoTestRatchet(ctx, {
          label: 'cargo test --workspace（全量）',
          cwd: backendDir,
          args: ['test', '--workspace'],
          // 与下面分包的 `baseline` **不是同一个口径**：`--workspace` 是所有测试目标
          // 各行求和，分包是各取首个 result 行。不给 `ciBaseline` 会直接判红（见
          // `cargoTestRatchet`）——那不是麻烦，是让「CI 没有棘轮」这件事不可能再静默发生。
          ciBaseline: BASELINE.ciRustTestsTotal,
          ciBaselineName: 'ciRustTestsTotal',
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
    //
    // 本地**默认跳过**（2026-10-09）：`cargo doc` 是对 workspace 的又一次**全量重编译**
    // （doc 指纹与 test / clippy 互不复用），是本地门禁最贵的单步之一；它守的
    // 断链由 CI 的 rust-checks job 必跑（`--ci` ⇒ 本任务在那边照常执行）。
    // 本地需要核查时单独跑：
    //   RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links' cargo doc --no-deps --workspace
    tasks.push({
      when: (c) => c.ci,
      skipNote: '本地跳过：doc 是全量重编译，断链判据由 CI 的 backend job 承担（上面有本地跑法）',
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
