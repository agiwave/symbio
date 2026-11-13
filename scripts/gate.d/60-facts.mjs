// facts 复核阶段（**必须最后**）：「跑完这一整轮，事实还和代码一致吗」。
// 「必须最后」由 `default.recheck` 自声明、`gate-wiring-audit` 的 GW-005 执行，不靠文件名巧合。
//
// ## 复核哪几份：没人在这里登记
//
// 由生成器自己声明（`// @gens <序数> check`，读法见 `_shared.gensDeclarations`），本阶段
// 遍历声明、按序数排序、逐个以 `--check` 起进程。**生成器有几份、叫什么，这里不知道**——
// 与 `06-facts-write` 同一个源，两处都不会因为新增一步而漏。
//
// ## 它与 `06-facts-write` 的分工
//
// 生成在 `06-facts-write`（静态审计的前置——`30-docs` 的 D-007 / RF-003 要看已展开的末态）。
// 复核必须留到最后：它问的是「这一整轮有没有谁改了代码」——`30-docs` 的自动修复器排在
// 生成之后，所以末态复核看的是**整轮**跑完，不是链跑完。放在生成之后立刻跑，它只能证明
// 「生成器刚刚跑过」——那是恒真的。
//
// ## 这两道不重复
//
// `06-facts-write` 的 `autoChain` 只在**链跑完**那一刻比一次差异；链绿着但产物被后续阶段
// 改脏，只有这里看得见。而「逐格判差异」在链上不可行：`registry-audit --write` 的产物
// （`[[编号]]` 记号）本身就是下一格的输入，逐格判在 `--ci` 下恒红。
//
// `--check` 是只读的，本地与 `--ci` 都跑得了（生成链的中间态在 `--ci` 下则不能）。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { gensDeclarations } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const scriptsDir = path.join(scriptDir, '..')

export default {
  id: 'facts',
  title: '事实复核（必须最后）',
  // 只读复核阶段的自声明，消费方是 `gate-wiring-audit` 的 GW-005（判「生成先于复核」）。
  recheck: true,
  tasks(ctx) {
    const decl = gensDeclarations(repoRoot) ?? { entries: [], problems: [] }
    const checkers = decl.entries.filter((e) => e.modes.includes('check'))
    // 读空即全绿：扫不出声明时本阶段会「0 项全绿」，而那与「全都一致」同形。
    if (checkers.length === 0) {
      throw new Error('facts 阶段读不出任何 `// @gens <序数> check` 声明 ⇒ 末态复核会空转')
    }
    // `args` 留在任务顶层（与 `30-docs` 同形）：`gate-wiring-audit` 的 GW-008 靠物化出的
    // 任务表判「声明了的是不是真的被跑到」，args 藏在 `run` 的闭包里就判不了。
    return checkers.map((e) => {
      const args = [path.join(scriptsDir, `${e.name}.mjs`), '--check']
      return {
        label: `${e.name} --check`,
        args,
        run: () => ctx.run({ label: `${e.name} --check`, cmd: process.execPath, args, cwd: repoRoot }),
      }
    })
  },
}
