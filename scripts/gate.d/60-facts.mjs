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
    ]
  },
}
