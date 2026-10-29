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

export default {
  id: 'backend',
  title: '后端（cargo，根 workspace）',
  // 阶段级并发：本阶段只写 `target/`，与批内其他泳道无数据依赖——docs 守卫只读
  // 源码与 scripts，baseline 只读 git 历史与 scripts，msrv 用独立的
  // `.workbuddy-ai/msrv-target`（不与 backend 争 `.cargo-lock`）。fmt 已拆到
  // `05-fmt.mjs` 屏障阶段（rustfmt 就地重写非原子，必须先于一切读 `.rs` 的泳道）。
  // ⚠️ frontend-build 不能进这批：编译 symbio-tauri 时 `generate_context!` 在编译期读
  //    `tauri/dist`，而 vite build 会重写它——见 `56-frontend-build.mjs`。前端另外三个
  //    任务不碰 `dist`，故拆到 `20-frontend-static.mjs` 进批（上面那句「只写 `target/`」
  //    对它同样成立）。
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
    // 编译（实测约 31s）。
    //
    // 需要「只要类型错误、不要 lint」的快速档时，正确做法是**单独跑** `cargo check`，
    // 而不是把它塞回门禁的必跑链里。

        // 测试：**本地与 CI 同一口径** —— 一次 `cargo test --workspace` 求和比对基线。
    //
    // 为什么要改（D-191 crate 化之后）：`cargo test -p symbio` 现在只跑宿主门面，
    // 实测 `0 passed`——21 个插件、core、providers 的用例全在各自 crate 里，分包跑
    // **一条都跑不到**，而 `rustTests` 基线 2175 守的正是那个空壳。这不是「数字要调」，
    // 是**守卫指向了不存在的对象**：它会在 0 个用例上一直报「通过数 < 基线」，或者
    // 更糟——有人把基线改成 0，于是拆分把一个 2000+ 用例的覆盖洞伪装成绿灯。
    //
    // 顺带解决两件事：
    //   · 口径唯一：`--workspace` 的求和就是全部用例，不再有「分包 vs 求和」两套基线；
    //   · 调用次数：4 次 `cargo test -p …` → 1 次（增量构建下每次启动仍有 ~1s 固定成本）。
    tasks.push(
      cargoTestRatchet({ ...ctx, ci: true }, {
        label: 'cargo test --workspace（全量）',
        cwd: backendDir,
        args: ['test', '--workspace'],
        ciBaseline: BASELINE.ciRustTestsTotal,
        ciBaselineName: 'ciRustTestsTotal',
        baseline: BASELINE.rustTests,
        baselineName: 'rustTests',
      }),
    )

    tasks.push({
      label: 'cargo clippy --all-targets --workspace -- -D warnings',
      cmd: 'cargo',
      args: ['clippy', '--all-targets', '--workspace', '--', '-D', 'warnings'],
      cwd: backendDir,
    })

    // rustdoc 的两条 lint（断链 / 伪码块）不在本泳道——它们是**唯一按 crate 数
    // 线性放大**的判据，单独成 `15-rustdoc.mjs`（id `rustdoc`），理由与代价都在
    // 那个文件头。要临时跳过它用 `--skip=rustdoc`，不要删任务。

    // cli release 二进制在本泳道尾部重建（e2e 的被测件）。放在这里的理由：
    // - `58-e2e` 的用例只等这份产物；原先构建在批后串行——改 Rust 的那一轮，
    //   一整遍 release 编译直接加在总墙钟上。放进泳道尾部，它与其他泳道的
    //   墙钟重叠，且沿用本泳道的 `.cargo-lock` 串行序（不新增跨泳道争用）。
    // - **无条件执行**，判「要不要重建」的是 `cli-binary.mjs` 的内容指纹，不是这里的
    //   `when`（判据同 `58-e2e`，两处共用同一条命令与同一份构建戳）。新鲜时它只是一次
    //   ~10ms 的指纹比对、不启动 cargo——而**这是一次通过，不是未判定**：这一项的判据是
    //   「本轮用的产物对得上源码」，不需要重建恰恰说明它成立。用 `when` 把它记成 ⊘
    //   会让每一轮绿灯都挂着一项「未判定」，而天天看见的红点/灰点最后就是没人看。
    // - CI 跳过（整项不进名单，而不是记一条 ⊘）：e2e job（`--only=e2e`）自备构建；
    //   把 release 加进 backend job 只会让那条 job 白多一遍编译。`58-e2e` 仍保留同一
    //   任务，`--only=e2e` / `--skip=backend` 的路线照旧自足。
    if (!ctx.ci) {
      tasks.push({
        label: 'cli: release 二进制（指纹过期 ⇒ 批内重建）',
        cmd: process.execPath,
        args: [path.join(scriptDir, '..', 'cli-binary.mjs')],
        cwd: repoRoot,
      })
    }

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
