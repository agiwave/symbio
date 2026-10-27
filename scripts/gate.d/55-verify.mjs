// verify 阶段：`docs/003-design/verify/` 的验证程序**自己证明自己**。
//
// 这批程序是 v2 方案的"硬证据"（`docs/003-design/README.md §4`、`04 §4` 的 C29），
// 在此之前它们只能手敲 `rustc --edition 2021 <f>.rs -o <out> && <out>` 跑一遍，
// 输出一份**手工提交的快照**——零断言的守卫等于没有守卫，
// 而"人手跑过一次"同样不是守卫：没有任何东西保证它明天还绿。
// 那份快照已经退出活跃目录（`docs/archive/verify-output-snapshot-2026-10.md`）：
// 它的唯一价值是"给人看"，而它会随任何一次改动过期，于是变成第二处真相。
//
// 两条判据，缺一不可：
// ① **正常档**：16 个程序逐个编译 + 运行，断言 exit 0。
//    exit 0 来自程序自己的 `assert!`（零断言的程序不可能失败——见验证纪律第 1 条），
//    所以这一条判的不是"能编译"，而是"它声称的结论今天仍然成立"。
// ② **反例档（C29）**：带 `should_not_compile` 的 3 个程序（`projection_purity` /
//    `latency_gate` / `conation_minimal`）加该 cfg 编译，断言**非 0 退出**。
//    没有这一条，无法区分"真的编译期强制"与"恰好没写错"——这是验证纪律第 4 条。
//    额外断言 stderr 带 `error[E`：类型/借用/隐私错误都有错误码，而**语法错误没有**。
//    少了这半条，一个被人改坏的文件也会被记成"反例生效"——那正是最坏的假绿。
//
// 输出落在 `.workbuddy-ai/verify-target/`（与 msrv 的 `msrv-target` 同策略：
// 不污染 `target/`，`.workbuddy-ai/` 已在 paths-ignore 与 .gitignore 内）。
//
// **本阶段不产出事实文件**：`verify/facts/mod.rs` 的生成在 `06-facts-write`（与
// `doc-count-audit` 同一条生成链的前置），所以这里只剩「编译 + 运行 + 反例档」，
// 得以进并发批（原先它锁死在批后串行，理由就是那份生成与 docs 读同一文件的竞态）。
// 也**不许**再引入生成链的那个执行原语——`gate-wiring-audit` GW-005 按源码字面串
// 判定「重生成阶段」，引入一次就把本阶段踢回「必须排在阶段序列末尾」那一组。
//
// 文件名 `55-`：并发批只由**连续**标 `parallel` 的阶段组成（`gate.mjs` 的泳道调度），
// 所以它必须紧跟在 `50-msrv` 之后、`56-frontend-build`（串行）之前。
//
// ## 谁耦合了生产（实读表， 起维护）
//
// 「程序数 16」里有两类证据，可信度不同，读者必须能分清：
//
// - **与生产源码耦合**（`#[path]` 包含生产文件）：判据与实现共处一处，
//   改生产、程序必须跟着红或绿——这才是「跑起来的系统遵守判据」的形状：
//
//   | 程序 | 包含的生产文件 | 判什么 |
//   |---|---|---|
//   | `consolidation_fidelity` | `symbio_core/text/merge_kernel.rs` | 巩固合并的保真度下界 |
//   | `recall_eval` | `plugins/loop/recall_score.rs` | 检索排序的 recall@k / 依赖度 |
// | `invariants` | `symbio_core/invariants/kernel.rs` | I1 的 seq 单调无跳号 / 每 turn 一条 final |
// | `termination` | `symbio_core/invariants/kernel.rs` | 终止性前提 2（返工上界）/ 前提 3（依赖图无环） |
//
// - **只与文档/自造数据耦合**（其余 12 个）：证明「判据在自造样本上自洽」，
//   不证明「生产实现遵守判据」。其中有几个的被测对象本来就是文档
//   （`capability_frame` 判名册与轴级的对应）——挂生产只会造出假耦合率，
//   目标是「把伪装成验生产的换成真验生产」，不是把数字刷到 16/16。
//
//   耦合率当前 **4/16**。下一个值得挂的：`invariants` 的 I2/I3 判据
//   （`produced_by_coverage` / `unresolved_turns`——同一内核文件里还有余量）。
import path from 'node:path'
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import { green, red, yellow, dim } from '../color.mjs'
import { BASELINE, ratchetOrBump } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const verifyDir = path.join(repoRoot, 'docs', '003-design', 'verify')
const outDir = path.join(repoRoot, '.workbuddy-ai', 'verify-target')
const genVerifyFacts = path.join(repoRoot, 'scripts', 'gen-verify-facts.mjs')

const exeSuffix = process.platform === 'win32' ? '.exe' : ''

/** 编译档位：反例档必须显式关掉 check-cfg 警告，否则未声明的 feature 会以
 *  warning 淹没输出（本仓用 `--check-cfg 'cfg(feature)'` 声明该 cfg 存在）。 */
const POSITIVE_ARGS = ['--edition', '2021']
const NEGATIVE_ARGS = ['--edition', '2021', '--check-cfg', 'cfg(feature)', '--cfg', 'feature="should_not_compile"']

/** 从 rustc stderr 里摘出 `error[E0593]` 这类**编号**错误——有编号 = 类型/借用/隐私
 *  拒绝；语法错误没有编号。返回去重后的错误码列表（如 `E0593`、`E0277/E0308`）。 */
const matchErrorCodes = (output) => {
  const codes = [...new Set([...String(output).matchAll(/error\[(E\d+)\]/g)].map((m) => m[1]))]
  return codes.length ? codes.join(' / ') : '无编号错误'
}

export default {
  id: 'verify',
  title: '验证程序（docs/003-design/verify · 含编译期反例 C29）',
  // 每个验证程序是**独立的 rustc 调用**（各自一个源文件、各自的输出 exe），
  // 彼此无共享状态 ⇒ 并发跑。批的边界在 `程序数棘轮` 那一步之后：预热与棘轮
  // 串行在泳道头部（各自几毫秒到几秒），程序批次内部并发。
  //
  // 阶段级 `parallel: true`（进 [backend ∥ frontend-static ∥ docs ∥ baseline ∥ msrv] 主批）：
  // 它原先不能进批的唯一理由——`gen-verify-facts` 重写 `verify/facts/mod.rs` 与
  // docs 的 doc-count-audit 竞态——已随生成前移到 `06-facts-write` 一起消掉；
  // 本阶段现在只读源码与已生成的 mod.rs、只写 `verify-target/`，与批内各泳道无数据依赖。
  // 并发度由机器档案的 `slots.cpu` 决定（每个验证程序是一次**真正的 rustc 编译**，
  // 吃满单核 ⇒ 按 CPU 口径给，不是按 IO 口径）。这里曾写死 8——那是当时那台
  // 8 逻辑核机器的核数：4 核机器上它是超订，32 核机器上是浪费。
  parallel: true,
  concurrencyKind: 'cpu',
  *tasks(ctx) {
    if (!fs.existsSync(verifyDir)) {
      yield {
        label: 'verify',
        run: async () => 'skipped',
        skipNote: `docs/003-design/verify/ 不存在（${path.relative(repoRoot, verifyDir)}）`,
      }
      return
    }

    const entries = fs
      .readdirSync(verifyDir)
      .filter((f) => f.endsWith('.rs'))
      .sort()
      .map((file) => {
        // 反例能力由**源码自己声明**，不维护第二份名单——名单会漂移，源码不会。
        const src = fs.readFileSync(path.join(verifyDir, file), 'utf8')
        return {
          file,
          name: file.replace(/\.rs$/, ''),
          hasNegative: src.includes('should_not_compile'),
          // 断言数（棘轮，只许涨）：前两格只到文件级，把某个程序的断言删到只剩
          // 一条时程序数与反例数都不变 ⇒ 全绿。而 `03 §5.1` 纪律 1 写着「零断言的
          // 程序不可能失败」——那条纪律此前没有任何机器判据。
          //
          // 口径是**文本出现次数**，含 `should_not_compile` 模块内正常档不执行的那些：
          // 刻意保守（基线略高于实际执行量），而「删断言必减计数」这个方向不受影响。
          asserts: (src.match(/\bassert!\s*\(|\bassert_eq!\s*\(|\bassert_ne!\s*\(/g) || []).length,
        }
      })
    if (entries.length === 0) {
      yield { label: 'verify', run: async () => 'skipped', skipNote: 'verify/ 下没有 .rs 程序' }
      return
    }

    // 棘轮（只许涨）：**数量本身也是判据**。程序被人删掉、或某个文件里的
    // `should_not_compile` 被摘走时，"少跑一跑"在日志里与"全绿"长得一模一样——
    // C29 存在的理由正是不接受这种静默。三态判定走 `ratchetOrBump`（与测试数
    // 棘轮同一条判据，不在这里再决定一次红不红）。
    const negatives = entries.filter((e) => e.hasNegative).length
    const asserts = entries.reduce((n, e) => n + e.asserts, 0)
    yield {
      label: 'verify: 程序数 · 反例数 · 断言数棘轮',
      run: async () => {
        const verdicts = [
          ratchetOrBump({
            actual: entries.length,
            baseline: BASELINE.verifyPrograms,
            name: 'verifyPrograms',
            kind: '程序',
            unit: '验证程序',
            ctx,
          }),
          ratchetOrBump({
            actual: negatives,
            baseline: BASELINE.verifyNegatives,
            name: 'verifyNegatives',
            kind: '反例',
            unit: '反例档',
            ctx,
          }),
          ratchetOrBump({
            actual: asserts,
            baseline: BASELINE.verifyAsserts,
            name: 'verifyAsserts',
            kind: '断言',
            unit: 'assert!',
            ctx,
          }),
        ]
        const short = verdicts.filter((v) => !v.ok)
        if (short.length) {
          for (const v of short) console.log(red(`      ↳ ${v.note}`))
          return { ok: false, note: short.map((v) => v.note).join('；') }
        }
        for (const v of verdicts) {
          if (v.note) console.log(green(`      ↳ ${v.note}`))
        }
        console.log(
          green(`      ↳ 程序 ${entries.length}/${BASELINE.verifyPrograms} · 反例 ${negatives}/${BASELINE.verifyNegatives} · 断言 ${asserts}/${BASELINE.verifyAsserts}`),
        )
        // 逐文件明细：断言数掉下来时要能**归因到文件**。`_shared.mjs` 里那条经验
        // 是「溯源方法有盲区，不等于来源不存在」——明细是归因的起点。
        for (const e of entries) {
          console.log(dim(`        ${e.name}: ${e.asserts}${e.hasNegative ? ' (+反例档)' : ''}`))
        }
        return { ok: true }
      },
    }

    fs.mkdirSync(outDir, { recursive: true })

    // 期望数据（`verify/facts/mod.rs`）由 `06-facts-write` 生成——本地门禁跑到这里时
    // 它已是本轮投影；但 CI 的 verify job 只跑本阶段（`--only=verify`），没有 06。
    // 这一格 `--check` 补上那道漂移闸：没有它，「程序全绿」只证明「程序与仓库里
    // 那份已提交的 mod.rs 自洽」，不证明 mod.rs 仍是文档的投影——文档改了、
    // 忘了重跑生成器，正是这套「外置期望数据」要防的第一种失效（plan/13 批 M1）。
    yield {
      label: 'verify: facts 漂移复核（gen-verify-facts --check）',
      cmd: process.execPath,
      args: [genVerifyFacts, '--check'],
      cwd: repoRoot,
    }

    // 程序吃的必须是**当前**计划；生成失败（表读不出来、名册与阶文件不配对）
    // 在生成器自身就抛红，不会走到这里。

    // **工具链串行预热（必须早于并发批）** —— 本阶段是全仓唯一用**裸 `rustc` 并发跑**的
    // 地方（backend 走 `cargo`，且 `05-fmt` 的 `cargo fmt --all` 会先串行解析一次工具链，
    // 所以本机与整轮门禁都不暴露）。而 `rust-toolchain.toml` 钉的 1.93.1 在 CI runner 上
    // **并不预装**：rustup 会在**首次**调用时按需下载安装。
    //
    // 2026-10-13 实测（CI run 38372186256 / 38358236876，verify-evidence job）：`--only=verify`
    // 跳过了 fmt ⇒ 无人预热，`concurrency: 8` 让 8 个 `rustc` 同时撞上那次安装。结果 19 个
    // 程序里只有 1 个成功（它耗时 11s —— 正是那次安装），其余 18 个以「toolchain 未安装」退出。
    // 而这条错误**没有 `error[E…]` 编号**，在日志里与「反例档没被类型拒绝」完全同形 ——
    // C29 的三条反向判据因此一起被带红，报出来的原因是错的。
    //
    // 先串行跑一次 `rustc --version`：① 把按需安装做完，此后并发批不再有竞争；
    // ② 把「工具链解析失败」单独归因成一个红项，而不是 19 个同形的「编译失败 exit=1」。
    // 不在 `ci.yml` 侧装第二份工具链 —— 那会造出第二个版本真相源，且**新机器（本机首次）**
    // 跑 `--only=verify` 时竞态依旧存在。判据钉在 `_shared.test.mjs`（预热必须在并发循环之前）。
    yield {
      label: 'verify: 工具链预热（rustc --version · 串行，先于并发批）',
      run: async () => {
        const r = await ctx.run({
          label: 'verify: 工具链预热 · rustc --version',
          cmd: 'rustc',
          args: ['--version'],
          cwd: verifyDir,
          echo: 'none',
          // 首次解析要按 rust-toolchain.toml 下载 1.93.1，给足（实测 ~8s，余量 30 倍）
          timeoutMs: 300_000,
        })
        if (!r.ok) {
          return {
            ok: false,
            note: `rustc 不可用（exit=${r.code}）—— 按 rust-toolchain.toml 解析/安装失败，后续并发批必全红`,
            logFile: r.logFile,
          }
        }
        return { ok: true }
      },
    }

    for (const { file, name, hasNegative } of entries) {
      yield {
        label: `verify: ${name}`,
        parallel: true,
        run: async () => {
          const exe = path.join(outDir, `${name}${exeSuffix}`)

          // ⚠️ 这三个超时**刻意不自适应**（300s / 180s / 60s）：它们的实测值是
          // 8s / 1–3s / <1s，余量 20–60 倍，慢 5 倍的机器也碰不到。给宽余量足够
          // 宽时，再叠一层自校准只是增加机制、不增加可靠性。判据只钉在**余量不足**
          // 的那几处（vitest / e2e 单例 / grep-audit 全仓扫描 / windows-restart）。
          const build = await ctx.run({
            label: `verify: ${name} · rustc`,
            cmd: 'rustc',
            args: [...POSITIVE_ARGS, file, '-o', exe],
            cwd: verifyDir,
            echo: 'none',
            timeoutMs: 180_000,
          })
          if (!build.ok) {
            return { ok: false, note: `编译失败 exit=${build.code}`, logFile: build.logFile }
          }

          const runIt = await ctx.run({
            label: `verify: ${name} · 运行`,
            cmd: exe,
            args: [],
            cwd: verifyDir,
            echo: 'none',
            timeoutMs: 60_000,
          })
          if (!runIt.ok) {
            return {
              ok: false,
              note: runIt.timedOut ? '超时被 kill' : `断言失败 exit=${runIt.code}`,
              logFile: runIt.logFile,
            }
          }
          return { ok: true }
        },
      }

      if (!hasNegative) continue

      yield {
        label: `verify: ${name} · 反例（C29）`,
        parallel: true,
        run: async () => {
          const exe = path.join(outDir, `${name}-should-fail${exeSuffix}`)
          const r = await ctx.run({
            label: `verify: ${name} · 反例档 rustc（期望编译不过）`,
            cmd: 'rustc',
            args: [...NEGATIVE_ARGS, file, '-o', exe],
            cwd: verifyDir,
            echo: 'none',
            timeoutMs: 180_000,
          })
          // `ctx.run` 按退出码印红字，而这一跑**红字才是对的**：紧跟一行绿字把
          // 方向说清楚，否则全绿的日志里嵌着一段"失败"，扫日志的人只会读到它。
          if (!r.ok && !r.timedOut) {
            console.log(green(`      ↳ 非 0 即正确（${matchErrorCodes(r.output)} ⇒ 类型级拒绝，正是 C29 要的）`))
          }
          // 期望非 0：0 意味着"号称编译期强制的东西其实能编译出来"。
          if (r.ok) {
            console.log(yellow(`      ↳ ${name}：应知编译不过却通过了 ⇒ 强制已被放宽（C29）`))
            return { ok: false, note: '反例档编译通过（强制失效）', logFile: r.logFile }
          }
          // 期望是**类型级**拒绝，不是语法错误：`error[E` = rustc 的编号错误。
          // 语法错误形如 `error: expected ...`，没有编号——那种情况说明文件本身坏了，
          // 被记成"反例生效"就是最坏的假绿。
          if (!/error\[E\d+\]/.test(r.output)) {
            console.log(yellow(`      ↳ ${name}：失败但没有 error[E…] ⇒ 不是类型级拒绝（C29）`))
            return { ok: false, note: '反例档失败原因不是类型错误', logFile: r.logFile }
          }
          return { ok: true }
        },
      }
    }
  },
}
