// e2e 阶段：CLI 端到端回归（mock LLM / mock MCP / 临时 homedir，见 e2e/README.md）。
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

    tasks.push({
      label: 'e2e 用例集',
      // 自定义任务：逐用例子进程隔离运行（与 run-tests.mjs 同构），聚合判定。
      run: async () => {
        const cases = discoverCases()
        if (cases.length === 0) return { ok: false, note: '未发现任何用例（e2e/cases/）' }
        let pass = 0
        const failures = []
        const failedLogs = []
        for (const c of cases) {
          const r = await ctx.run({
            label: `e2e: ${c.name}`,
            cmd: process.execPath,
            args: [c.file],
            cwd: repoRoot,
            timeoutMs: 180_000,
            echo: 'none',
            env: { E2E_CASE_SELF: '1' },
          })
          if (r.ok) {
            pass++
            console.log(`      ✓ ${c.name}`)
          } else {
            failures.push(c.name)
            if (r.logFile) failedLogs.push(r.logFile)
            console.log(`      ✗ ${c.name}（详见日志）`)
            // ⚠️ 这行**原先带 `if (ctx.ci !== true)`**：本地打印、CI 不打印。
            // 方向恰好反了——本地有 `.workbuddy-ai/gate-logs/` 可以打开，CI 上那个
            // 目录在 runner 里、没人上传，于是「（详见日志）+ 一条本机路径」就是一条
            // **零信息的红**。2026-10-07 首次在 v2-plan 上跑 CI 正是如此：42/42 全红，
            // 日志里只有 42 个文件名，真正的原因（缺 `tauri/node_modules`）一行没露。
            // 失败项才打印、只取末 6 行，撑不爆 CI 日志。
            console.log(dim(r.output.split('\n').slice(-6).join('\n      ')))
          }
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
