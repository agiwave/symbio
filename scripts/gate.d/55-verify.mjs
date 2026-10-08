// verify 阶段：`docs/plan/verify/` 的验证程序**自己证明自己**。
//
// 这批程序是 v2 方案的"硬证据"（`docs/plan/README.md §4`、`04 §4` 的 C29），
// 在此之前它们只能手敲 `rustc --edition 2021 <f>.rs -o <out> && <out>` 跑一遍，
// `docs/plan/verify/OUTPUT.md` 是**手工提交的快照**——零断言的守卫等于没有守卫，
// 而"人手跑过一次"同样不是守卫：没有任何东西保证它明天还绿。
//
// 两条判据，缺一不可：
// ① **正常档**：14 个程序逐个编译 + 运行，断言 exit 0。
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
// **为什么文件名是 55- 而不是 70-**：门禁按文件名排序执行，`60-facts` 是
// autoWork（重新生成并暂存 `docs/CURRENT.md`），必须**最后**跑——它要把暂存状态
// 定格成"本次改动的终态"。排在它后面会让暂存发生在 verify 之前，`git status`
// 再也看不出 verify 那一跑有没有碰过东西。同理 msrv 是 50-。
import path from 'node:path'
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import { green, red, yellow, dim } from '../color.mjs'
import { BASELINE, ratchetVerdict, autoWork } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const verifyDir = path.join(repoRoot, 'docs', 'plan', 'verify')
const outDir = path.join(repoRoot, '.workbuddy-ai', 'verify-target')
const genFacts = path.join(repoRoot, 'scripts', 'gen-verify-facts.mjs')

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
  title: '验证程序（docs/plan/verify · 含编译期反例 C29）',
  *tasks(ctx) {
    if (!fs.existsSync(verifyDir)) {
      yield {
        label: 'verify',
        run: async () => 'skipped',
        skipNote: `docs/plan/verify/ 不存在（${path.relative(repoRoot, verifyDir)}）`,
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
    // C29 存在的理由正是不接受这种静默。三态判定走 `ratchetVerdict`（与测试数
    // 棘轮同一条判据，不在这里再决定一次红不红）。
    const negatives = entries.filter((e) => e.hasNegative).length
    const asserts = entries.reduce((n, e) => n + e.asserts, 0)
    yield {
      label: 'verify: 程序数 · 反例数 · 断言数棘轮',
      run: async () => {
        const verdicts = [
          ratchetVerdict({
            actual: entries.length,
            baseline: BASELINE.verifyPrograms,
            name: 'verifyPrograms',
            kind: '程序',
            unit: '验证程序',
          }),
          ratchetVerdict({
            actual: negatives,
            baseline: BASELINE.verifyNegatives,
            name: 'verifyNegatives',
            kind: '反例',
            unit: '反例档',
          }),
          ratchetVerdict({
            actual: asserts,
            baseline: BASELINE.verifyAsserts,
            name: 'verifyAsserts',
            kind: '断言',
            unit: 'assert!',
          }),
        ]
        const short = verdicts.filter((v) => !v.ok)
        if (short.length) {
          for (const v of short) console.log(red(`      ↳ ${v.note}`))
          return { ok: false, note: short.map((v) => v.note).join('；') }
        }
        for (const v of verdicts) {
          if (v.warn) console.log(yellow(`      ↳ ${v.note}：请上调 scripts/gate.d/_shared.mjs 的 BASELINE`))
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

    // **先**把期望数据从文档抽出来，再跑程序：verify 吃的必须是当前计划。
    // 顺序反了就会「文档改了、程序还在验旧的那份」——那正是本批要消灭的形态
    // （plan/13 批 M1）。生成失败（表读不出来、名册与阶文件不配对）直接红。
    yield {
      label: 'verify: facts 生成（docs/plan → verify/facts/mod.rs）',
      run: (c) =>
        autoWork(c, {
          label: 'gen-verify-facts',
          cmd: process.execPath,
          args: [genFacts],
          cwd: repoRoot,
        }),
    }

    for (const { file, name, hasNegative } of entries) {
      yield {
        label: `verify: ${name}`,
        run: async () => {
          const exe = path.join(outDir, `${name}${exeSuffix}`)

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
