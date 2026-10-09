// frontend-build 阶段：`vite build`（**只**这一步）。
//
// 文件名 56- 与「不进并发批」（2026-10-09 拆分）：backend 编译 `symbio-tauri` 时
// `generate_context!` 在**编译期**读 `tauri.conf.json` 的 `frontendDist`（`../dist`），
// 而 `vite build` 会**清空重写**同一目录 ⇒ 并发就是「编译读到半截 dist ⇒ 假红」。
// 所以构建必须留在批后；批前的静态检查在 `20-frontend-static.mjs`。
//
// 拆分的理由见 `20-frontend-static.mjs` 头部（那里有收益与「为什么不反过来」）。
// 本阶段本身很轻（实测 ~6s，热缓存），留在关键路径上不值得再动它。
//
// 也**不与 58-e2e 并发**：e2e 用例对 CPU 竞争时序敏感（并发 6 是贴着实测最优调的，
// 再叠加外部负载会把「等实时面收敛」的用例推过超时线，2026-10-08 实测）。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { maybeSandboxDeleteHint, blockedBySandboxDelete } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const frontendDir = path.join(repoRoot, 'tauri')
const bin = (p) => path.join(frontendDir, 'node_modules', p)

export default {
  id: 'frontend-build',
  title: '前端构建（vite build）',
  tasks(ctx) {
    if (!fs.existsSync(path.join(frontendDir, 'package.json'))) return []

    return [
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
            return { ok: false, note: `exit=${r.code}`, logFile: r.logFile }
          }
          return { ok: true }
        },
      },
    ]
  },
}
