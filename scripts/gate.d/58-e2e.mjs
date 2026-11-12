// e2e 阶段：CLI 端到端回归（mock LLM / mock MCP / 临时 homedir，见 e2e/README.md）。
//
// 文件名 58-：并发批（含 55-verify）与紧随其后的 56-frontend-build
// 都结束后本阶段才跑——用例对 CPU 竞争时序敏感（并发贴着理论最优调，再叠加外部负载
// 会把「等实时面收敛」的用例推过超时线），不与任何重负载并发。
// cli release 二进制的**构建**已上移到 `10-backend` 泳道尾部（与批内其他泳道的墙钟
// 重叠）；本阶段第一个任务保留同一条命令作兜底——`--only=e2e` / `--skip=backend`
// 的路线照旧自足，正常轮它只是一次 ~10ms 的构建戳比对。
//
// 以机制接入：用例清单**不在这里维护**——`e2e/cases/*.mjs` 按文件名序逐个以
// 独立子进程运行（与 `node e2e/run-tests.mjs` 同一套发现逻辑），新增用例文件
// 自动纳入门控。前置：`crates/symbio-cli/` 的 release 二进制。
//
// ⚠️ **「自动纳入」只管加，不管减**：删掉一份 `tNN.mjs`，这一跑仍是「41/41 通过」、
// 门禁照旧全绿，而文档正文正引用着「现 42 例」。所以用例数进
// `BASELINE.e2eCases` 棘轮（只许涨），三态判定走 `ratchetOrBump`——向上自动上调、向下才红，与单测数
// 同一条判据，不在这里再决定一次红不红。
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { dim, yellow, red, green } from '../color.mjs'
import { BASELINE, ratchetOrBump } from './_shared.mjs'
import { machine } from './_machine.mjs'
// 逐例耗时的取数实现与门外复核（`scripts/e2e-concurrency.mjs`）共用一份：
// 同一段逻辑曾在这里与那里各写一份，而日志目录一改就两边同时失明。
import { latestE2ELogs } from './_logs.mjs'
// 起跑顺序（LPT）与耗时缓存：与 `e2e/run-tests.mjs` 共用同一份实现与同一个文件。
import { orderForStart, loadTimes, recordTimes, suggestConcurrency } from '../e2e-order.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const cliDir = path.join(repoRoot, 'crates', 'symbio-cli')
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
  // 输入面：`rust`（被测件是 `crates/symbio-cli` 的 release 二进制）+ `e2e`（用例自己）。
  // 前端改动不进这个集合：e2e 跑的是 CLI，不加载任何前端产物。**这是全量门禁里最贵的
  // 单阶段之一（实测 1m40s、88 例）**，而它对「改了一句文档」的结论恒等于上一轮。
  needs: ['rust', 'e2e'],
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
        const v = ratchetOrBump({
          actual,
          baseline: BASELINE.e2eCases,
          name: 'e2eCases',
          kind: 'e2e 用例数',
          unit: '用例',
          ctx,
        })
        if (!v.ok) {
          console.log(red(`      ↳ ${v.note}`))
          return { ok: false, note: v.note }
        }
        if (v.note) console.log(green(`      ↳ ${v.note}`))
        else console.log(dim(`      ↳ 用例 ${actual}/${BASELINE.e2eCases}`))
        return { ok: true }
      },
    })

    // 用例号是**地址**：`node e2e/run-tests.mjs tNN` 按号选，文档与提交信息也按号引。
    // 撞号时两件事一起坏——选一个号跑出两条（读者以为在复现一条判据），而引用它的那句话
    // 指向不明。实测事故：`t71-gateway-boundary`（新增）与既有的
    // `t71-peer-commitment` 各自在文件头自述「T71」，谁也没撞坏任何东西，门禁一路全绿——
    // 因为**没有一条判据数过这件事**。上面那条棘轮只判总数，「总数对得上而号重复」
    // 恰好是它的盲区。
    tasks.push({
      label: 'e2e: 用例号唯一（一个号只指向一条）',
      run: async () => {
        const byNum = new Map()
        for (const c of discoverCases()) {
          const num = /^t(\d+)-/.exec(c.name)?.[1]
          if (!num) return { ok: false, note: `用例 ${c.name} 的编号解析不出（文件名须形如 tNN-主题.mjs）` }
          byNum.set(num, [...(byNum.get(num) ?? []), c.name])
        }
        const dups = [...byNum].filter(([, names]) => names.length > 1)
        if (dups.length) {
          const note = dups.map(([num, names]) => `t${num} ⇒ ${names.join(' / ')}`).join('；')
          console.log(red(`      ↳ 用例号撞车：${note}`))
          return { ok: false, note: `用例号撞车：${note}` }
        }
        console.log(dim(`      ↳ ${byNum.size} 个用例号互不重复`))
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
        // 读**本轮**的日志（`ctx.logDir` 就是本轮目录）：门禁的日志改成每轮一个
        // 目录后，平铺的 `.workbuddy-ai/gate-logs/` 下只剩指针文件，读它会得到
        // 空集——而空集与「没有数据」在这条判据里都返回 ok，于是它会**静默**
        // 从「每次跑完都印一行」退化成「永远不印」。取数口径与门外复核用的
        // `e2e-concurrency.mjs` 共用（`_logs.mjs::latestE2ELogs`），不分叉。
        const logs = latestE2ELogs(path.join(repoRoot, '.workbuddy-ai', 'gate-logs'), { runDir: ctx.logDir })
        const times = logs.map((l) => l.ms).filter((ms) => ms > 0)
        if (times.length === 0) {
          console.log(dim('      ↳ 本轮没有 e2e 逐例耗时（用例未跑或日志缺失，跳过）'))
          return { ok: true }
        }
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
      // **并发度与起跑顺序都由上一轮实测推到**（`GATE_E2E_CONCURRENCY` 可覆盖）。
      // 并行有**两个**前提，缺一个就不能开：
      //
      // ① **端口分段**已生效——`E2E_PORT_BASE` 按**用例序号**注入（不是按 worker
      // 序号），所以每个用例的 mock LLM / MCP / gateway 各占一段。少了它，40+ 个
      // 子进程会全从 18080 起算、互相撞端口，症状是「mock-llm 收到 31 次请求」——
      // 比串行更难排查，因为失败点随机漂。
      // 并发度按「总工作量 / 最慢单例」推导，上限 `CONCURRENCY_CEILING`：并发吃不掉
      // 最慢的那一例，再加路数只买调度 jitter。推导复核见 `scripts/e2e-concurrency.mjs`，
      // 起跑顺序按上一轮耗时降序（LPT）——按文件名序会让慢例太晚起跑、别的泳道空转。
      //
      // 陷阱：别拿整例耗时去比单例 `waitFor` 预算（整例是多次等待之和，跨用例相减
      // 会算出没有意义的负数）；并发上限不是「怕它不稳」，是超过拐点后**不值**。
      // 失败多时门禁显著变慢，是每例要走满自身超时，不是门禁退化。
      run: async () => {
        const cases = discoverCases()
        if (cases.length === 0) return { ok: false, note: '未发现任何用例（e2e/cases/）' }

        // 并发的两个来源，各管一头，缺一不可：
        // - `machine().slots.e2e`：**这台机器**开得动几路（已按逻辑核/内存/启动负载
        //   收过一轮，上限是 `CONCURRENCY_CEILING`）。原先是直接拿核数算，在 2 核机器
        //   上给 2 路却没算内存与负载，遇到同时开着的 IDE/浏览器就会假红。
        // - `suggestConcurrency`：**这批用例**需要几路（总工作量/最慢单例）。尾部长例
        //   回归时它自己收缩，慢例被优化掉时自己上调——不需要谁记得回来改常量。
        const m = await machine()
        // 起跑顺序与耗时表都来自**上一轮**实测（`scripts/e2e-order.mjs`）。
        const prevTimes = loadTimes(repoRoot)
        const limit = Math.max(
          1,
          Number(process.env.GATE_E2E_CONCURRENCY) ||
            suggestConcurrency({ cases, times: prevTimes, machineSlots: m.slots.e2e }),
        )
        // 单个用例的兜底预算必须随机器放大：慢机器上静态 180s 会先到，报出来的
        // 是「超时」，看上去像用例坏了。用例内部自己的 `waitFor` 才是判据，这里只防挂死。
        const caseTimeoutMs = Math.round(180_000 * Math.max(1, m.cpuScale))
        const results = new Array(cases.length)
        // 起跑顺序按上一轮实测耗时降序（LPT），**不是**文件名序：墙钟由最后完成的那条
        // 泳道决定，而耗时是极端长尾（前 10 例占 72% 的工作量）。按文件名序跑，那几条
        // 慢例要到很后面才被捞起来，别的泳道就空转着等它们。判据与端口分段见
        // `scripts/e2e-order.mjs`（`index` 仍是原序号 ⇒ 端口号段不随顺序变）。
        const plan = orderForStart(cases, prevTimes)
        let cursor = 0
        const worker = async () => {
          for (;;) {
            const p = cursor++
            if (p >= plan.length) return
            const { case: c, index: i } = plan[p]
            const r = await ctx.run({
              label: `e2e: ${c.name}`,
              cmd: process.execPath,
              args: [c.file],
              cwd: repoRoot,
              timeoutMs: caseTimeoutMs,
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
        await Promise.all(Array.from({ length: Math.min(limit, plan.length) }, worker))
        // 本轮实测写回，供下一轮排期（同一个文件，`e2e/run-tests.mjs` 也读它）。
        recordTimes(
          repoRoot,
          Object.fromEntries(results.filter(Boolean).map((r) => [r.caseName, r.caseMs])),
        )

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
