// e2e 阶段：CLI 端到端回归（mock LLM / mock MCP / 临时 homedir，见 e2e/README.md）。
//
// 文件名 58-（原 40-，2026-10-09）：阶段级并发批 [10-backend / 30-docs / 35-baseline /
// 50-msrv] 结束后本阶段才跑——cli 的 release 构建与 backend 共用 `target/` 的
// `.cargo-lock`，并发只会排成 `Blocking waiting for file lock`；且用例对 CPU 竞争
// 时序敏感，不与 frontend 等重负载叠跑。也不进批的原因同源：顺序敏感，见上。
//
// 以机制接入：用例清单**不在这里维护**——`e2e/cases/*.mjs` 按文件名序逐个以
// 独立子进程运行（与 `node e2e/run-tests.mjs` 同一套发现逻辑），新增用例文件
// 自动纳入门控。前置：`cli/` 的 release 二进制。
//
// ⚠️ **「自动纳入」只管加，不管减**：删掉一份 `tNN.mjs`，这一跑仍是「41/41 通过」、
// 门禁照旧全绿，而 `plan/13` 的 S5 批次正文正引用着「现 42 例」。所以用例数进
// `BASELINE.e2eCases` 棘轮（只许涨），三态判定走 `ratchetVerdict`——与单测数
// 同一条判据，不在这里再决定一次红不红。
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { dim, yellow, red } from '../color.mjs'
import { BASELINE, ratchetVerdict } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const cliDir = path.join(repoRoot, 'cli')
const e2eDir = path.join(repoRoot, 'e2e')
const caseFileRe = /^t\d.*\.mjs$/

function discoverCases() {
  const dir = path.join(e2eDir, 'cases')
  if (!fs.existsSync(dir)) return []
  return fs
    .readdirSync(dir)
    .filter((f) => f.endsWith('.mjs') && !f.startsWith('_') && caseFileRe.test(f))
    .sort()
    .map((f) => ({ file: path.join(dir, f), name: f.replace(/\.mjs$/, '') }))
}

export default {
  id: 'e2e',
  title: '端到端（CLI × mock LLM × mock MCP）',
  tasks(ctx) {
    const tasks = []
    if (!fs.existsSync(path.join(cliDir, 'Cargo.toml'))) return tasks

    // 被测系统先就位。**无条件执行**：判「要不要重建」的是 `cli-binary.mjs` 的
    // 内容指纹，不是这里的 `when`。
    //
    // 这里曾经写 `when: () => ctx.ci || !cliBinaryExists(repoRoot)`——「文件在就跳过」。
    // 于是本机那份过期 exe 被一直用下去，e2e 报出与眼前源码矛盾的断言失败，
    // 排查方向被带偏到源码上。二进制是构建产物的函数：**产物比输入旧就是不可信的**，
    // 而「旧不旧」只能由内容指纹回答。指纹一致时该脚本不启动 cargo（零成本）。
    tasks.push({
      label: 'cli: release 二进制（按源码指纹决定是否重建）',
      cmd: process.execPath,
      args: [path.join(scriptDir, '..', 'cli-binary.mjs')],
      cwd: repoRoot,
    })

    tasks.push({
      label: 'e2e: 用例数棘轮',
      run: async () => {
        const actual = discoverCases().length
        const v = ratchetVerdict({
          actual,
          baseline: BASELINE.e2eCases,
          name: 'e2eCases',
          kind: 'e2e 用例数',
          unit: '用例',
        })
        if (!v.ok) {
          console.log(red(`      ↳ ${v.note}`))
          return { ok: false, note: v.note }
        }
        if (v.warn) console.log(yellow(`      ↳ ${v.note}：请上调 scripts/gate.d/_shared.mjs 的 BASELINE.e2eCases`))
        else console.log(dim(`      ↳ 用例 ${actual}/${BASELINE.e2eCases}`))
        return { ok: true }
      },
    })


    // 并发度判据（离线脚本 `scripts/e2e-concurrency.mjs` 的门内版本）。
    //
    // 为什么**只印不判红**：并发度过低只让门禁慢、不让它说错话；而过高的风险
    // 已由「默认贴着理论最优」消掉——余量在那儿，不靠这条红。判红会让
    // 「某个用例变慢了」直接变成门禁失败，而那时该做的是优化那个用例。
    // 复核：`node scripts/e2e-concurrency.mjs`（`--ci` 供 CI 用）。
    tasks.push({
      label: 'e2e: 并发度判据（理论最优 = 总工作量 / 最慢单例）',
      run: async () => {
        const cases = discoverCases()
        const logsDir = path.join(repoRoot, '.workbuddy-ai', 'gate-logs')
        if (!fs.existsSync(logsDir)) {
          console.log(dim('      ↳ 尚无上一轮日志（首次运行跳过）'))
          return { ok: true }
        }
        const times = fs
          .readdirSync(logsDir)
          .filter((f) => /^e2e-t.*\.log$/.test(f))
          .map((f) => {
            const t = fs.readFileSync(path.join(logsDir, f), 'utf8')
            const ms = [...t.matchAll(/\((\d+)ms\)/g)].map((m) => +m[1])
            return ms.length ? ms[ms.length - 1] : 0
          })
          .filter((ms) => ms > 0)
        if (times.length === 0) return { ok: true }
        const total = times.reduce((a, b) => a + b, 0)
        const slowest = Math.max(...times)
        const optimal = (total / slowest).toFixed(1)
        console.log(
          dim(
            `      ↳ ${cases.length} 例｜总工作量 ${(total / 1000).toFixed(1)}s` +
              `｜最慢单例 ${(slowest / 1000).toFixed(1)}s｜理论最优 ${optimal}` +
              `｜墙钟下界 ${(slowest / 1000).toFixed(1)}s`,
          ),
        )
        return { ok: true }
      },
    })


    tasks.push({
      label: 'e2e 用例集',
      // 自定义任务：用例子进程隔离运行（与 run-tests.mjs 同构），聚合判定。
      //
      // **默认并发 6**（`GATE_E2E_CONCURRENCY` 可覆盖）。并行有**两个**前提，
      // 缺一个就不能开：
      //
      // ① **端口分段**已生效——`E2E_PORT_BASE` 按**用例序号**注入（不是按 worker
      // 序号），所以每个用例的 mock LLM / MCP / gateway 各占一段。少了它，40+ 个
      // 子进程会全从 18080 起算、互相撞端口，症状是「mock-llm 收到 31 次请求」——
      // 比串行更难排查，因为失败点随机漂。
      // ② **并发度已到收益拐点**（见下）。
      //
      // ## 为什么是 6：墙钟下界由**单个慢例**决定，不由总工作量决定
      //
      // 实测（本机 20 核，`.workbuddy-ai/gate-logs/` 逐例耗时）：
      //
      // ```
      // 总工作量（串行等效）= 88.1s
      // 最慢的单个用例      = 15.5s（t36-autonomous-initiator）
      // ⇒ 任何并发度的墙钟下界 = 15.5s；理论最优并发度 = 88.1 / 15.5 ≈ 5.7 → 取 6
      //
      // 实测墙钟：并发 1 → 88s    4 → 22s    6 → 19s    16 → 17.2s
      // ```
      //
      // 读法：**并发吃不掉慢例**。下界由 `t36` 一个用例决定（15.5s），不看总量——
      // 再往上加，门禁耗时由它说了算。6 到 16 只差 1.8s（9%），而进程数翻 2.7 倍、
      // 抢同一份 CPU。**继续加并发买到的是调度抖动，卖出去的是判定稳定性。**
      //
      // ⚠️ 「更高并发会不会改变结论」**实测过**：6 与 16 **各连跑 3 轮，失败集逐轮完全
      // 相同**（32/42，同 10 个红）。所以上限不是「怕它不稳」——**是不值**。选 6 是
      // 贴着理论最优，不是贴着实测上限。
      //
      // ⚠️ 这里换过一次判据。原来写的是「最慢通过例 15.5s vs 最小内层 `waitFor` 10s，
      // 余量 5.5s」——**那是错的**：`t36` 的整例耗时是很多次等待之和，跟 `t11` 的
      // 单次预算不是一回事，跨用例比预算算出 −5.5s 这种没有意义的负数。判据换成
      // 「总工作量 / 最慢单例」后**不依赖「哪个用例最慢」**，只依赖两个总和。
      //
      // ## 若要重算：跑 `node scripts/e2e-concurrency.mjs`
      //
      // 它从上一轮 `.workbuddy-ai/gate-logs/e2e-*.log` 读逐例耗时，算出
      // `总工作量 / 最慢单例` 并与本文件的默认值对照；不一致时按「上调要验失败集
      // 三轮稳定 / 下调直接照办」给处置。**慢用例被优化掉时它自己会给出更小的建议值**
      // ——所以别在这里硬写一个「感觉够用」的数字。
      //
      // 另外：单个用例失败要走满它自己的超时（实测 3s–120s，`t2` 一轮 120s），
      // 所以**失败多的时候**门禁会显著变慢——那是用例失败的表现，不是门禁的锅。
      run: async () => {
        const cases = discoverCases()
        if (cases.length === 0) return { ok: false, note: '未发现任何用例（e2e/cases/）' }

        // 默认 6，理由见上方注释（理论最优 = 总工作量/最慢单例 ≈ 5.7，实测三轮稳定）。
        // 核数少时往下收：核不够时并发再高只是把 CPU 竞争推高超时敏感度。
        const cpuCap = Math.max(1, Math.min(6, os.availableParallelism?.() ?? os.cpus().length))
        const limit = Math.max(1, Number(process.env.GATE_E2E_CONCURRENCY) || cpuCap)
        const results = new Array(cases.length)
        let cursor = 0
        const worker = async () => {
          for (;;) {
            const i = cursor++
            if (i >= cases.length) return
            const c = cases[i]
            const r = await ctx.run({
              label: `e2e: ${c.name}`,
              cmd: process.execPath,
              args: [c.file],
              cwd: repoRoot,
              timeoutMs: 180_000,
              echo: 'none',
              env: { E2E_CASE_SELF: '1', E2E_PORT_BASE: String(18080 + i * 100) },
            })
            results[i] = r
            // 逐例耗时回填到结果上：并发度的判据要用**本轮**的数（见下方
            // `cpuCap` 那段注释）。从上一轮日志取数有个坏处——那轮的耗时是**旧代码**
            // 的，于是「默认值是否还合适」永远在拿过时数据回答。
            r.caseMs = r.ms
            r.caseName = c.name
            console.log(r.ok ? `      ✓ ${c.name}` : `      ✗ ${c.name}（详见日志）`)
          }
        }
        await Promise.all(Array.from({ length: Math.min(limit, cases.length) }, worker))

        let pass = 0
        const failures = []
        const failedLogs = []
        for (const [i, r] of results.entries()) {
          if (r.ok) {
            pass++
            continue
          }
          failures.push(cases[i].name)
          if (r.logFile) failedLogs.push(r.logFile)
          // 失败项才打印、只取末 6 行，撑不爆 CI 日志。
          //
          // ⚠️ 这行**原先带 `if (ctx.ci !== true)`**：本地打印、CI 不打印。方向恰好
          // 反了——本地有 `.workbuddy-ai/gate-logs/` 可以打开，CI 上那个目录在 runner
          // 里、没人上传，于是「（详见日志）+ 一条本机路径」就是一条**零信息的红**。
          // 2026-10-07 首次在 v2-plan 上跑 CI 正是如此：42/42 全红，日志里只有 42 个
          // 文件名，真正的原因（缺 `tauri/node_modules`）一行没露。
          console.log(dim(r.output.split('\n').slice(-6).join('\n      ')))
        }
        // 并发度判据：用**本轮**逐例耗时当场算，结论印出来（不单独判红）。
        //
        // 为什么只印不判：并发度过低只让门禁**慢**，不会让它**说错话**；而过高的
        // 风险（改变判定）已由「默认贴着理论最优」消掉——余量在那儿，不靠这条红。
        // 判红会让「某个用例变慢了」直接变成门禁失败，而那时该做的是优化那个用例。
        // 离线复核：`node scripts/e2e-concurrency.mjs`。
        const times = results.filter((r) => r && r.caseMs > 0).map((r) => r.caseMs)
        if (times.length > 0) {
          const totalMs = times.reduce((a, b) => a + b, 0)
          const slowest = Math.max(...times)
          const optimal = totalMs / slowest
          console.log(
            dim(
              `      ↳ 并发 ${limit}｜总工作量 ${(totalMs / 1000).toFixed(1)}s` +
                `｜最慢单例 ${(slowest / 1000).toFixed(1)}s` +
                `｜理论最优 ${optimal.toFixed(1)}｜墙钟下界 ${(slowest / 1000).toFixed(1)}s`,
            ),
          )
        }

        const note =
          failures.length === 0
            ? `${pass}/${cases.length} 通过`
            : `通过 ${pass}/${cases.length}，失败: ${failures.join('、')}`
        console.log(`      ${failures.length === 0 ? '' : yellow('')}${note}`)
        // 聚合任务有多个子日志：失败时把每一项的文件名都带回去（汇总逐个列出）。
        return { ok: failures.length === 0, note, logFile: failures.length === 0 ? null : failedLogs }
      },
    })
    return tasks
  },
}
