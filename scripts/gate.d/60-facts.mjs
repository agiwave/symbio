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
const regAudit = path.join(scriptDir, '..', 'registry-audit.mjs')
const docRefs = path.join(scriptDir, '..', 'gen-doc-refs.mjs')

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
      {
        // 登记汇总页是**条目文件的投影**（同 CURRENT.md 是代码的投影）：确定性输出，
        // 写成 --check 只会因为「人忘了重跑」而红。校验判据在 30-docs 的 registry-audit。
        label: 'registry-audit --write（自动重新生成登记汇总页）',
        run: (c) =>
          autoWork(c, {
            label: 'registry-audit --write',
            cmd: process.execPath,
            args: [regAudit, '--write'],
            cwd: repoRoot,
          }),
      },
      {
        // 文档间引用**只写编号**（`[[T-008]]`），地址由本步生成/重算：编号不变则地址永远正确，
        // 文档搬家后入链自愈——「不手写地址」这句话的执行方。未登记的编号由它判红（RF-001）。
        // 放在 registry-audit --write 之后：汇总页里的链接是渲染出来的，本步只管正文引用。
        label: 'gen-doc-refs（自动重算文档引用地址）',
        run: (c) =>
          autoWork(c, {
            label: 'gen-doc-refs',
            cmd: process.execPath,
            args: [docRefs],
            cwd: repoRoot,
          }),
      },
    ]
  },
}
