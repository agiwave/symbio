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

/**
 * 被升为 `-D` 的两条 rustdoc lint —— 各自为什么值得判红，见下方注释。
 * ⚠️ **故意不在名单里的第三类**（`private_intra_doc_links`）也有它的理由，
 * 同样写在下方。加进名单前先读那段，别只加名字。
 */
const DOC_LINT_ENV = {
  RUSTDOCFLAGS: '-D rustdoc::broken_intra_doc_links -D rustdoc::invalid_rust_codeblocks',
}

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

    // 这条任务管两类 rustdoc 告警。**两类都从 warn 升 `-D`，但理由不同，
    // 而且有一类故意不升** —— 名字写在 DOC_LINT_ENV 的注释里，理由在这里。
    //
    // ── ① `broken_intra_doc_links`：升 `-D`，因为它是**事实错误** ──
    //
    // 它是 **warn-by-default** 的 lint，而没有任何门禁跑过 `cargo doc`
    // ⇒ 39 条断链长期无人知，其中还混着**指向已删除函数**的
    // （`emit_converge`、`VdfsProvider::write` / `delete` / `action`——该 trait 早已
    // 只剩 `dispatch`）。读文档的人被指到一个不存在的东西上，而这**不产生任何错误**，
    // 只能靠跑一次把它变成红灯。判据是 zero-tolerance（没有「几条以内可接受」）。
    //
    // ── ② `invalid_rust_codeblocks`：升 `-D`，因为它廉价且无副作用 ──
    //
    // 标着 `ignore` 的代码块 rustdoc 仍会尝试按 Rust 解析（为了高亮），
    // 解析不了就告警。修法只有一种：块里不是 Rust 就标 `text`。**没有取舍**——
    // 不存在「为了消警告而少写文档」的路径。2026-10-14 修掉唯一一条
    // （`provider_adapter.rs` 里带 `…` 的伪码块）。
    //
    // ── ③ `private_intra_doc_links`：**故意不升**。它不是缺陷，是**信号** ──
    //
    // 先说它在本仓的具体形态：`symbio_core` 的子模块全是私有 `mod`，对外只经
    // curated `pub use` 暴露。所以「声明成 `pub` 却没被导出」的项（unreachable pub）
    // 被 rustdoc 按私有处理——**那 25 条全部属于此类**。
    //
    // 为什么不升 `-D`，四条依据：
    //   1. **它指向的页面在渲染结果里根本不存在**。实测：
    //      `target/doc/symbio/symbio_core/adapters/trait.LlmAdapter.html` **存在**
    //      （已 re-export），而 `…/adapters/enum.AdapterError.html` **不存在**。
    //      模块目录存在只是因为已导出的项渲染在自己的定义模块下。
    //      ⇒ rustdoc 早就把这些链接**降级成纯文本**了，`[`X`]` 与 `` `X` `` 的
    //      **渲染输出完全相同**。它不是「指向坏链接」，是「没有链接」。
    //   2. 于是消掉这 25 条警告只剩两条路，**两条都是亏的**：
    //      把内部 helper（`acyclic_deps` / `rework_bounded` / `budget_exceeded` …）
    //      提成公开——**为读者根本不需要的东西扩 API**；
    //      或把 25 条链接降成代码块——**砍掉维护者唯一的导航**。
    //   3. 这些链接的真实消费者是**IDE 里的维护者**（Ctrl+click），不是 rustdoc
    //      产出的 HTML——本仓的 rustdoc HTML 没人看。降级损伤的正是它。
    //   4. `-D` 一条只能靠「让文档变差」来满足的 lint，是**收税**，不是守卫。
    //
    // ⚠️ 但它**确实逮到了东西**——所以别把它当噪音（2026-10-14）。分诊追下去是这样：
//   1. 头一条查的是「这个类型在不在公开签名里」，据此我一度把其中 6 条报成「API 洞、
//      该 re-export」。**那个定性是错的**：逐项核对后，`AdapterError` / `AppendError`
//      是**故意** `pub(crate)` 的（`symbio_core/mod.rs` 有整段理由），而仅有的两个跨
//      crate 消费方 `cli/`（5 个 `.rs`）与 `tauri/`（4 个）对这 13 个名字**命中 0**——
//      消费方全在本 crate 内。为零消费者扩公开面就是 AGENTS.md 说的「为读者根本不
//      需要的东西扩 API」。
//   2. 真正的问题是另一件：分诊过程中查出 `Reasoner` 主体带 `reply` / `reply_timed` /
//      `reply_streaming` 三条生成入口，**零生产调用方**（生产路径调
//      `llm.generate_turn(...)`；`reply_streaming` 只被同文件的 `reply_timed` 调用）。
//      按「没有调用方的死代码必须清除」删掉之后，C-003 单消费方从 57 **涨到 59**——
//      删死代码揭开了两条被它掩盖的**判据错误**（`LlmAdapter` / `DeltaSink` 被判成
//      「单消费方接缝、该下沉」，而它们在 core 里有 `impl`，下沉会造成依赖反转）。
//      守卫的修法见 `scripts/core-surface.mjs` 的 `collectCoreBindings`，处理过程记在
//      `docs/task/T-103-清单归属字段与汇总页登记.md` 旁支二。
//   教训留在判据侧而不是这里：**这 25 条不是噪音，是探测器**。它的正确用法是
//   「照着去查」，查出来的结论要**按调用方核对**——我第一次就是照着告警直接下了结论，
//   下错了。
    //
    // ── 关于「本地也跑」（2026-10-14 由 CI-only 改为无条件）────
    //
    // 原先这里挂着 `when: ctx.ci`，理由是「doc 是对 workspace 的又一次全量重编译，
    // 是本地门禁最贵的单步之一」——**那条依据是错的，本机实测推翻了它**：
    //   cargo doc --no-deps --workspace → **15.7s**（本机 Windows / 20 核）
    // 依赖的 `rmeta` 由同阶段的 clippy / test 编出来、直接复用，所以它**不是**全量重编译，
    // 只是本地那批 crate 换了个 rustdoc 指纹重编一遍；占全量门禁（~2m30s）约 10%。
    // 而且 backend 是 `parallel: true` 的阶段、与 docs（1m30s）同批并发 ⇒ 并发批墙钟
    // 由 docs 决定，backend 整阶段 25s ⇒ 这 15.7s 被并发批**完全吃掉，总墙钟零增加**。
    //
    // 为什么值得记这一笔：`97a53c23` 写下那条依据时**没有附实测数字**，它是被当成显然的
    // 事写下的。代价主张必须能被一次测量推翻，否则它就只是一句听起来合理的话——
    // 而「本地跳过」的形式是**任何时候都没人跑**：守卫存在，却不在开发者的回路里，
    // 断链要等到 push 后的 CI 才被拦下。判据要留在人**马上会看到**的地方。
    //
    // 单独跑（不想等门禁编排时）：
    //   RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links -D rustdoc::invalid_rust_codeblocks' `
    //     cargo doc --no-deps --workspace
    tasks.push({
      label: 'cargo doc --no-deps --workspace（-D broken_intra_doc_links / invalid_rust_codeblocks）',
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
