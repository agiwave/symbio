// facts 阶段：事实文件由代码生成（**必须最后**——前面任何自动修复都可能改代码）。
//
// 生成是**门禁自己做的事**，不是判它「有没有做过」：`gen(code) → CURRENT.md` 是
// 确定性的，写成 `--check` 只会因为「人忘了重跑」而红。见 `_shared.autoWork`。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { autoWork } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const gen = path.join(scriptDir, '..', 'gen-current-facts.mjs')
const genCodes = path.join(scriptDir, '..', 'gen-gate-codes.mjs')
const genRoutes = path.join(scriptDir, '..', 'gen-routes-ts.mjs')

export default {
  id: 'facts',
  title: '事实文件（必须最后）',
  tasks(ctx) {
    return [
      {
        label: 'gen-current-facts（自动重新生成）',
        run: (c) =>
          autoWork(c, {
            label: 'gen-current-facts',
            cmd: process.execPath,
            args: [gen],
            cwd: repoRoot,
          }),
      },
      {
        label: 'gen-gate-codes（自动重新生成）',
        run: (c) =>
          autoWork(c, {
            label: 'gen-gate-codes',
            cmd: process.execPath,
            args: [genCodes],
            cwd: repoRoot,
          }),
      },
      {
        // 前端的路由常量是后端 `route()` 臂的投影（真源只有一个）。手写那份是抄本：
        // 它漂移时没有任何测试会红，只在运行期表现为后端回 `NotFound`。
        label: 'gen-routes-ts（自动重新生成 tauri/src/constants/routes.gen.ts）',
        run: (c) =>
          autoWork(c, {
            label: 'gen-routes-ts',
            cmd: process.execPath,
            args: [genRoutes],
            cwd: repoRoot,
          }),
      },
    ]
  },
}
