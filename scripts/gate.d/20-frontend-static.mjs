// frontend-static 阶段：vue-tsc / vitest --coverage / eslint。
//
// 文件名 `20-` 与 `parallel: true`（2026-10-09）：本阶段进并发批
// `[10-backend ∥ 20-frontend-static ∥ 30-docs ∥ 35-baseline ∥ 50-msrv]`。
//
// ## 为什么它能进批，而同属前端的 `vite build` 不能
//
// backend 编译 `symbio-tauri` 时 `generate_context!` 在**编译期**读 `tauri/dist`，
// 而 `vite build` 会**清空重写**那个目录 ⇒ 并发就是「编译读到半截 dist ⇒ 假红」。
// 所以构建留在 `56-frontend-build.mjs`（批后）。
//
// 本阶段的三个任务**都不碰 `dist`**，而且对磁盘几乎**只读**：
//   - `vue-tsc --noEmit`   只读类型，不产出文件
//   - `vitest run`         coverage 走 `reporter: ['text']`（`tauri/vitest.config.ts`），
//                          只打印不落盘 ⇒ 连 `coverage/` 都不写
//   - `eslint .`           只读（不带 `--fix`）
// 反过来，批内没有哪个阶段会写本阶段要读的东西（backend 只写 `target/`，docs 只读，
// baseline 只读 git，msrv 写 `.workbuddy-ai/msrv-target`）。
//
// ## 收益（2026-10-09 A/B 实测，两轮都先 `touch symbio/src/lib.rs` 强制重编译）
//
//   旧编排（前端整段在批后）  6m19s：backend 4m59s ∥ docs 2m10s ∥ msrv 1m28s
//                                    → verify 16s → frontend 30s → e2e 29s → facts 1s
//   新编排（本阶段进批）      5m21s：backend 4m36s ∥ 本阶段 1m17s ∥ docs 2m49s ∥ msrv 1m51s
//                                    → verify 10s → frontend-build 3s → e2e 28s → facts 1s
//
// 关键路径上的**串行尾**从 `verify + frontend + e2e`（76s）变成
// `verify + vite build + e2e`（42s）：本阶段整段藏在 backend 的墙钟里（1m17s < 4m36s，
// 且 1m17s 已是**与 cargo 争 CPU 之后**的数——它单独跑只要 34s）。
//
// ⚠️ 两轮的 backend 相差 23s（4m59s vs 4m36s），那是 cargo 缓存差异，**不归本次改动**；
// 结构上可归因的收益就是「前端静态检查离开串行尾」，其大小 = 本阶段墙钟，随前端工具链
// 冷热变化很大（热 30s，冷可达 ~2m——`eslint` 冷启动实测过 1m7s）。
//
// ## 为什么任务级**不**并发
//
// 三个 node 进程同时跑只是把它们与吃满全部核的 cargo 一起拖慢，本阶段墙钟不会
// 因此变短（任务彼此无重叠收益，都是 CPU 密集）。顺序保持与原 `56-frontend`
// 一致：vue-tsc → vitest → eslint。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { yellow, red, stripAnsi } from '../color.mjs'
import {
  BASELINE,
  BASELINE_GRACE,
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
  id: 'frontend-static',
  title: '前端静态检查（vue-tsc / vitest / eslint）',
  parallel: true,
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
            return { ok: false, note, logFile: r.logFile }
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
          // 覆盖率**高出阈值**同样要限期红，容差取 `BASELINE_GRACE`（与 `ratchetVerdict`
          // 同一条）：高出的阈值不是无害的——阈值不跟着涨，删掉同样多的测试仍然全绿，
          // 棘轮就被削掉了同样的点数，而提示可以被无限忽略（日志天天有、门禁天天绿）。
          // 这正是 M2 把「高于基线只打黄字」改限期的同一条理由，此前只改了测试数那一半。
          if (pct !== null && thr !== null && pct - thr >= 10) {
            const slack = pct - thr - 10
            if (slack > BASELINE_GRACE) {
              const line =
                `行覆盖率 ${pct}% 已高出阈值 ${thr}% ${(pct - thr).toFixed(1)} 个点，` +
                `超出回填容差 ${BASELINE_GRACE} ⇒ 上调 tauri/vitest.config.ts 的 thresholds.lines`
              console.log(red(`      ↳ ${line}`))
              return { ok: false, note: line }
            }
            console.log(
              yellow(
                `      ⚠ 行覆盖率 ${pct}% 已高出阈值 ${thr}% ${(pct - thr).toFixed(1)} 个点：阈值形同虚设，建议上调（还剩 ${BASELINE_GRACE - slack} 点容差）`,
              ),
            )
          } else if (pct !== null) {
            console.log(`      行覆盖率 ${pct}%${thr === null ? '' : `（阈值 ${thr}%）`}`)
          }
          return { ok: true, note: grew ? `文件/用例数 ${files}/${tests}（基线待更新）` : '' }
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
