// facts 复核阶段（**必须最后**）：「跑完这一整轮，事实还和代码一致吗」。
//
// ## 它与 `28-facts-write` 的分工
//
// 生成挪到了 `28-facts-write`（静态审计的前置——`30-docs` 的 D-007 / RF-003 要看
// 已展开的最终形态，详见那篇的注释），本阶段只剩**复核**。
//
// 为什么复核必须留到最后：它问的是「这一整轮有没有谁改了代码」。放在生成之后立刻跑，
// 它只能证明「生成器刚刚跑过」——那是恒真的；放在最后，它才是真正的漂移门禁。
//
// 本机这一步恒绿（生成器跑过之后没人改代码）；CI 侧它与 autoWork 的差异判定互为两道。
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const genFacts = path.join(scriptDir, '..', 'gen-facts.mjs')

export default {
  id: 'facts',
  title: '事实复核（必须最后）',
  tasks(ctx) {
    return [
      {
        label: 'gen-facts --check（整轮跑完后复核：事实与代码仍一致）',
        run: (c) =>
          ctx.run({
            label: 'gen-facts --check',
            cmd: process.execPath,
            args: [genFacts, '--check'],
            cwd: repoRoot,
          }),
      },
    ]
  },
}
