// facts 复核阶段（**必须最后**）：「跑完这一整轮，事实还和代码一致吗」。
//
// ## 它与 `06-facts-write` 的分工
//
// 生成挪到了 `06-facts-write`（静态审计的前置——`30-docs` 的 D-007 / RF-003 要看
// 已展开的最终形态，详见那篇的注释），本阶段只剩**复核**。
//
// 为什么复核必须留到最后：它问的是「这一整轮有没有谁改了代码」——`30-docs` 的自动修复器
// 排在生成之后，所以末态复核看的是**整轮**跑完，不是链跑完。放在生成之后立刻跑，
// 它只能证明「生成器刚刚跑过」——那是恒真的。
//
// 本机这一步恒绿（生成器跑过之后没人改代码）；CI 侧它与生成链的末态差异判定互为两道。
//
// ## 这两格 `--check` 不负责「让 CI 看见生成物漂移」——那件事以前没人做
//
// 2026-10-23 的形态：生成链按 `autoWork` **逐格**判差异，而 `registry-audit --write` 的产物
// （`[[编号]]` 记号）本身就是下一格 `gen-doc-refs` 的输入 ⇒ `--ci` 下**恒红**。
// 于是三个 gate 调用点都没敢点 `facts-write`，CI / pre-push 完全看不见
// 「改了 front-matter 没重生成汇总页」这一类漂移。
// 现在链合成了一个 `autoChain` 工作（只在**末态**判差异，见 `06-facts-write` 头注释），
// CI 点得动它了；本阶段留着这两条只读复核，是因为它们覆盖的是**生成之后那一截**。
// 登记续接见 [T-250-生成链阶段id失配](../../docs/009-task/250-生成链阶段id失配.md)。
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const genFacts = path.join(scriptDir, '..', 'gen-facts.mjs')
const docRefs = path.join(scriptDir, '..', 'gen-doc-refs.mjs')

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
      {
        // 引用地址复核（RF-001…004）：树里不该有未展开的 `[[编号]]` 记号，也不该有
        // 与编号不一致的手写地址。它是**只读**的，所以在 `--ci` 下与本地都跑得了——
        // 与上面那条不成立的原因正相反（见头注释：生成链的中间态在 `--ci` 下恒红）。
        // 单独实测 0.27s（本阶段两格合跑实测 1s），提交点带得上。
        label: 'gen-doc-refs --check（引用地址与编号一致，含登记汇总页的展开末态）',
        run: (c) =>
          ctx.run({
            label: 'gen-doc-refs --check',
            cmd: process.execPath,
            args: [docRefs, '--check'],
            cwd: repoRoot,
          }),
      },
    ]
  },
}
