// frontend-build 阶段：`vite build`（**只**这一步）。
//
// 文件名 56- 与「不进并发批」（2026-10-09 拆分）：backend 编译 CLI 时
// `build.rs` 在**编译期**内嵌 `packages/ui/dist` 进二进制，而 `vite build` 会
// **清空重写**同一目录 ⇒ 并发就是「编译读到半截 dist ⇒ 假红」。所以构建必须留在
// 批后；批前的静态检查在 `20-frontend-static.mjs`。
//
// 单一产物：web（静态部署）与 CLI（build.rs 编译期内嵌）都读 `packages/ui/dist`
// 这一份；desktop 原生窗口壳不内嵌 UI，它加载的 CLI serve 提供同一份产物。不再构建
// 两份、也不再有 desktop/web 内容分叉。
//
// 也**不与 58-e2e 并发**：e2e 用例对 CPU 竞争时序敏感，再叠加外部负载会把
// 「等实时面收敛」的用例推过超时线。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import { maybeSandboxDeleteHint, blockedBySandboxDelete } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const frontendDir = path.join(repoRoot, 'packages', 'ui')
const dist = path.join(frontendDir, 'dist')
const bin = (p) => path.join(frontendDir, 'node_modules', p)

export default {
  id: 'frontend-build',
  title: '前端构建（vite build）',
  // Rust / e2e 改动也必须构建：T116 会从 CLI 内嵌资源验证完整 UI，而 backend 可能先于
  // 本阶段把占位页编进 release CLI；e2e 前的指纹复核会据此再构建一次真实产物。
  needs: ['frontend', 'rust', 'e2e'],
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
            console.log('      ↳ 构建失败：本地复现用 `npm run build`（在 packages/ui 下）')
            maybeSandboxDeleteHint(r.output)
            if (blockedBySandboxDelete(r.output)) return 'skipped'
            return { ok: false, note: `exit=${r.code}`, logFile: r.logFile }
          }
          // 产物必须可用：index.html 是 web 静态部署与 CLI 编译期内嵌的共同入口。
          if (!fs.existsSync(path.join(dist, 'index.html'))) {
            return { ok: false, note: 'vite build 未产出 packages/ui/dist/index.html' }
          }
          return { ok: true }
        },
      },
    ]
  },
}
