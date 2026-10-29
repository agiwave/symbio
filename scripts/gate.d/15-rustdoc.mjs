// rustdoc 阶段：`cargo doc` 的断链 / 伪码块判据。
//
// **为什么是独立阶段而不是 backend 泳道里的一项**：这一项是门禁里**唯一按 crate
// 数线性放大**的检查（34 个 crate 各跑一遍 rustdoc），改一次 core / kernel 就要把
// 全树各渲一遍，实测 2m43s——占全量门禁五六成，且随 crate 数继续长。它与 backend
// 的另外三项（test / clippy / cli release）没有数据依赖，只是共用 `.cargo-lock`。
// 独立成 id 之后，「这一轮不跑它」从**删掉一个任务**变成 `--skip=rustdoc` 的一次
// 显式选择：跳过的人知道自己在跳什么，CI 与默认全量照跑。
//
// 判据本身一个字没放宽——下面这段理由仍然成立，且比位置更重要：
//
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
//      crate 消费方 `crates/symbio-cli/`（5 个 `.rs`）与 `tauri/`（4 个）对这 13 个名字**命中 0**——
//      消费方全在本 crate 内。为零消费者扩公开面就是 AGENTS.md 说的「为读者根本不
//      需要的东西扩 API」。
//   2. 真正的问题是另一件：分诊过程中查出 `Reasoner` 主体带 `reply` / `reply_timed` /
//      `reply_streaming` 三条生成入口，**零生产调用方**（生产路径调
//      `llm.generate_turn(...)`；`reply_streaming` 只被同文件的 `reply_timed` 调用）。
//      按「没有调用方的死代码必须清除」删掉之后，C-003 单消费方从 57 **涨到 59**——
//      删死代码揭开了两条被它掩盖的**判据错误**（`LlmAdapter` / `DeltaSink` 被判成
//      「单消费方接缝、该下沉」，而它们在 core 里有 `impl`，下沉会造成依赖反转）。
//      守卫的修法见 `scripts/core-surface.mjs` 的 `collectCoreBindings`。
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
// 只是本地那批 crate 换了个 rustdoc 指纹重编一遍。
//
// 为什么值得记这一笔：`97a53c23` 写下那条依据时**没有附实测数字**，它是被当成显然的
// 事写下的。代价主张必须能被一次测量推翻，否则它就只是一句听起来合理的话——
// 而「本地跳过」的形式是**任何时候都没人跑**：守卫存在，却不在开发者的回路里，
// 断链要等到 push 后的 CI 才被拦下。判据要留在人**马上会看到**的地方。
//
// 单独跑（不想等门禁编排时）：
//   RUSTDOCFLAGS='-D rustdoc::broken_intra_doc_links -D rustdoc::invalid_rust_codeblocks' `
//     cargo doc --no-deps --workspace
//
// ── 拆 crate 之后这两个数字都变了（2026-10-27 复测，2026-10-29 复测一致）────
//
//   cargo doc --no-deps --workspace → **1m36s → 2m43s**（同一台机器 / 20 核）
//   backend 整阶段 → **3m1s**（其中 doc 1m36s、cargo test 1m7s、cli 11s）
//   docs 阶段 → 49s
//
// 原因不是「rmeta 不复用了」，而是 rustdoc **按 crate 跑**：D-191 把树从 1 个
// 库 crate 拆成 33 个，这一步就按 crate 数线性放大。连带的结论是本段上面那句
// 「并发批墙钟由 docs 决定、doc 的 15.7s 被完全吃掉」**正好反了**：现在是 doc
// 顶着其余所有阶段，它直接压在关键路径上。
//
// 处置：判据留着（断链要在开发者回路里），代价用**内容指纹**摊掉——交给
// `scripts/cargo-doc-gate.mjs`：rustdoc 的全部输入（`crates/**` 与
// `tauri/src-tauri/src/**` 的 `.rs`、各清单、工具链与 lint 档位）哈希一致
// ⇒ 上一轮的通过仍然成立，跳过；不一致 ⇒ 照跑，且**只在成功后落戳**。
//
// 指纹只买得到「内容没变而 mtime 变了」那一档；**真的改了 core / kernel** 那一档
// 是判据的固有成本（每个 crate 各渲一遍 core 的公开面），不是浪费——压它的办法
// 只有减少 crate 数或减少 re-export，那是架构问题，不是门禁调参。
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')

/**
 * 被升为 `-D` 的两条 rustdoc lint —— 各自为什么值得判红，见文件头注释。
 * ⚠️ **故意不在名单里的第三类**（`private_intra_doc_links`）也有它的理由，
 * 同样写在文件头。加进名单前先读那段，别只加名字。
 */
const DOC_LINT_ENV = {
  RUSTDOCFLAGS: '-D rustdoc::broken_intra_doc_links -D rustdoc::invalid_rust_codeblocks',
}

export default {
  id: 'rustdoc',
  title: 'rustdoc 断链与伪码块（cargo doc）',
  // 阶段级并发：本阶段只写 `target/`（`target/doc` 与 cargo 的指纹目录），与批内
  // 其他泳道无数据依赖。与 backend 同处一个 `.cargo-lock` 是**已知且可接受**的：
  // 两条 cargo 命令本来就会互斥排队，墙钟等于两者之和（与把它们串在一条泳道里
  // 相同），拆开只是为了 `--skip=rustdoc` 有一个名字。
  // ⚠️ 因此**不要**把这里的 cargo 命令换成任何需要「先跑完 backend」的判据——
  // 锁只保证互斥、不保证次序。
  parallel: true,
  tasks() {
    return [
      {
        label:
          'cargo doc --no-deps --workspace（-D broken_intra_doc_links / invalid_rust_codeblocks；输入未变则跳过）',
        cmd: process.execPath,
        args: [path.join(scriptDir, '..', 'cargo-doc-gate.mjs')],
        cwd: repoRoot,
        env: DOC_LINT_ENV,
      },
    ]
  },
}
