// e2e 阶段：CLI 端到端回归（mock LLM / mock MCP / 临时 homedir，见 e2e/README.md）。
//
// 以机制接入：用例清单**不在这里维护**——`e2e/cases/*.mjs` 按文件名序逐个以
// 独立子进程运行（与 `node e2e/run-tests.mjs` 同一套发现逻辑），新增用例文件
// 自动纳入门控。前置：`cli/` 的 release 二进制（缺失则先构建）。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { dim, yellow } from '../color.mjs'
import { cliBinaryExists } from './_shared.mjs'

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

    // 被测系统先就位：二进制缺失或 --ci 时显式构建（平时增量 cargo build 很快）。
    tasks.push({
      label: 'cli: cargo build --release',
      when: () => ctx.ci || !cliBinaryExists(repoRoot),
      skipNote: '已有 release 二进制（删除 symbio/target/release 或 --ci 可强制重建）',
      cmd: 'cargo',
      args: ['build', '--release'],
      cwd: cliDir,
    })

    tasks.push({
      label: 'e2e 用例集',
      // 自定义任务：逐用例子进程隔离运行（与 run-tests.mjs 同构），聚合判定。
      run: async () => {
        const cases = discoverCases()
        if (cases.length === 0) return { ok: false, note: '未发现任何用例（e2e/cases/）' }
        let pass = 0
        const failures = []
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
            console.log(`      ✗ ${c.name}（详见日志）`)
            if (ctx.ci !== true) console.log(dim(r.output.split('\n').slice(-6).join('\n      ')))
          }
        }
        const note =
          failures.length === 0
            ? `${pass}/${cases.length} 通过`
            : `通过 ${pass}/${cases.length}，失败: ${failures.join('、')}`
        console.log(`      ${failures.length === 0 ? '' : yellow('')}${note}`)
        return { ok: failures.length === 0, note }
      },
    })
    return tasks
  },
}
