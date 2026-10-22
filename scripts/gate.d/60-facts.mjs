// facts 复核阶段（**必须最后**）：「跑完这一整轮，事实还和代码一致吗」。
//
// ## 它与 `06-facts-write` 的分工
//
// 生成挪到了 `06-facts-write`（静态审计的前置——`30-docs` 的 D-007 / RF-003 要看
// 已展开的最终形态，详见那篇的注释），本阶段只剩**复核**。
//
// 为什么复核必须留到最后：它问的是「这一整轮有没有谁改了代码」。放在生成之后立刻跑，
// 它只能证明「生成器刚刚跑过」——那是恒真的；放在最后，它才是真正的漂移门禁。
//
// 本机这一步恒绿（生成器跑过之后没人改代码）；CI 侧它与 autoWork 的差异判定互为两道。
//
// ## 复核为什么**不能**改成「把 `facts-write` 也带上跑一遍 `--ci`」
//
// 看着最省事的修法是让 CI / pre-push 直接跑生成链（差异即红），实测**恒红**：链上
// `registry-audit --write` 先把汇总页写成 `[[编号]]` 记号，`gen-doc-refs` 再展开成地址
// （顺序不能换，见 `06-facts-write` 的头注释），于是**中间那一格的产物本身就是差异**——
// `autoWork` 按「这一步有没有改写文件」判红，它看到的是半成品。
//
// 所以 CI 侧要的是一条**只读**的末态判据：树里不该有未展开的记号。这一格原先没有执行方
// （`gen-doc-refs --check` 早就存在，但三个 gate 调用点了 `facts` 都没点到它），
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
