// facts 阶段：事实文件防漂移（必须最后——它由代码生成，前面任何自动修复都可能改代码）。
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const gen = path.join(scriptDir, '..', 'gen-current-facts.mjs')

export default {
  id: 'facts',
  title: '事实文件（必须最后）',
  *tasks(ctx) {
    if (ctx.fix) {
      yield {
        label: 'gen-current-facts.mjs（--fix 写入）',
        cmd: process.execPath,
        args: [gen],
        cwd: repoRoot,
      }
    }
    yield {
      label: 'gen-current-facts --check',
      cmd: process.execPath,
      args: [gen, '--check'],
      cwd: repoRoot,
    }
  },
}
