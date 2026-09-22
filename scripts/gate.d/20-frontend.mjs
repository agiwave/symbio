// frontend 阶段：vue-tsc / vitest --coverage / vite build / eslint。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { yellow, dim, stripAnsi } from '../color.mjs'
import {
  BASELINE,
  VITEST_TIMEOUT_MS,
  grabInt,
  coverageLinesPct,
  coverageThreshold,
  blockedBySandboxDelete,
  maybeSandboxDeleteHint,
} from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const frontendDir = path.join(repoRoot, 'tauri')
const bin = (p) => path.join(frontendDir, 'node_modules', p)

export default {
  id: 'frontend',
  title: '前端（vue-tsc / vitest）',
  tasks(ctx) {
    if (!fs.existsSync(path.join(frontendDir, 'package.json'))) return []

    return [
      {
        label: 'vue-tsc --noEmit',
        cmd: process.execPath,
        args: [bin('vue-tsc/bin/vue-tsc.js'), '--noEmit', '-p', 'tsconfig.json'],
        cwd: frontendDir,
      },
      {
        label: 'vitest run --coverage',
        // 自定义任务：沙箱拦截 coverage/ 清理时用 --coverage.clean=false 重跑一次；
        // 成功后做文件/用例基线棘轮与覆盖率阈值「形同虚设」提示。
        run: async () => {
          let r = await ctx.run({
            label: 'vitest run --coverage',
            cmd: process.execPath,
            args: [bin('vitest/vitest.mjs'), 'run', '--coverage'],
            cwd: frontendDir,
            timeoutMs: VITEST_TIMEOUT_MS,
          })
          if (!r.ok && blockedBySandboxDelete(r.output)) {
            console.log('      ↳ 沙箱拦了 coverage/ 的清理 ⇒ 以 --coverage.clean=false 重跑（不跳过本步）')
            r = await ctx.run({
              label: 'vitest run --coverage（clean=false）',
              cmd: process.execPath,
              args: [bin('vitest/vitest.mjs'), 'run', '--coverage', '--coverage.clean=false'],
              cwd: frontendDir,
              timeoutMs: VITEST_TIMEOUT_MS,
            })
          }
          if (!r.ok) {
            const coverageRed = /Coverage for .* does not meet/.test(stripAnsi(r.output))
            if (!coverageRed) maybeSandboxDeleteHint(r.output)
            if (!coverageRed && blockedBySandboxDelete(r.output)) {
              return 'skipped'
            }
            const note = r.timedOut
              ? '超时终止'
              : coverageRed
                ? '覆盖率低于阈值（见 tauri/vitest.config.ts 的 coverage.thresholds）'
                : `exit=${r.code}, signal=${r.signal}`
            return { ok: false, note }
          }

          const files = grabInt(r.output, /Test Files\s+(\d+) passed/)
          const tests = grabInt(r.output, /Tests\s+(\d+) passed/)
          if (files === null || tests === null) return { ok: false, note: '无法解析用例数' }
          if (files < BASELINE.vitestFiles || tests < BASELINE.vitestTests) {
            return {
              ok: false,
              note: `文件/用例数未达基线：${files}/${tests}（基线 ${BASELINE.vitestFiles}/${BASELINE.vitestTests}）`,
            }
          }
          const grew = files > BASELINE.vitestFiles || tests > BASELINE.vitestTests
          console.log(
            grew
              ? `      ⚠ ${files} 文件 / ${tests} 用例 > 基线 ${BASELINE.vitestFiles}/${BASELINE.vitestTests}：请更新 scripts/gate.d/_shared.mjs 的 BASELINE.vitestFiles / vitestTests`
              : `      ${files} 文件 / ${tests} 用例（基线 ${BASELINE.vitestFiles}/${BASELINE.vitestTests}）`,
          )
          const pct = coverageLinesPct(r.output)
          const thr = coverageThreshold(frontendDir)
          if (pct !== null && thr !== null && pct - thr >= 10) {
            console.log(
              yellow(
                `      ⚠ 行覆盖率 ${pct}% 已高出阈值 ${thr}% ${(pct - thr).toFixed(1)} 个点：阈值形同虚设，建议上调`,
              ),
            )
          } else if (pct !== null) {
            console.log(`      行覆盖率 ${pct}%${thr === null ? '' : `（阈值 ${thr}%）`}`)
          }
          return { ok: true, note: grew ? `文件/用例数 ${files}/${tests}（基线待更新）` : '' }
        },
      },
      {
        label: 'vite build',
        // 类型检查过了不代表打包得过（循环依赖、动态导入、chunk 配置错误只在 build 暴露）。
        run: async () => {
          const r = await ctx.run({
            label: 'vite build',
            cmd: process.execPath,
            args: [bin('vite/bin/vite.js'), 'build'],
            cwd: frontendDir,
          })
          if (!r.ok) {
            console.log('      ↳ 构建失败：本地复现用 `npm run build`（在 tauri/ 下）')
            maybeSandboxDeleteHint(r.output)
            if (blockedBySandboxDelete(r.output)) return 'skipped'
            return { ok: false, note: `exit=${r.code}` }
          }
          return { ok: true }
        },
      },
      {
        label: 'eslint（分层约束）',
        cmd: process.execPath,
        args: [bin('eslint/bin/eslint.js'), '.'],
        cwd: frontendDir,
      },
    ]
  },
}
