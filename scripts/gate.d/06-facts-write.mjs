// facts 生成阶段（**在静态审计之前**）：把「代码 → 事实文件 → 汇总页 → 引用地址」
// 这一整条链跑完，让 `30-docs` 的审计看到的是**已生成、已展开**的状态。
//
// ## 链上有哪几步：**没人在这里登记**
//
// 步骤由生成器自己声明（`// @gens <序数> write`，读法见 `_shared.gensDeclarations`），
// 本阶段遍历一遍声明、按序数排序、逐个以 `--write` 起进程。加一步生成只是
// 「写脚本 + 声明一行」，这里一个字都不用改——此前是四个路径常量 + 一个手写数组，
// 漏登记的失效形态是静默（新生成器永不执行，两个阶段都绿）。
//
// 同理，**复核**（`--check`）在 `60-facts` 里由声明 `check` 的那些生成器驱动。
//
// ## 判定模式会还原
//
// 本阶段总是以 `--write` 起生成器（那是它们唯一的产物形态），但**判定模式的约定是
// 只判不改**：链跑完末态有差异就还原并报红，修复交给 `node scripts/gen.mjs`。
// 所以「绿」等于「生成物已经和代码一致」，而不是「门禁刚刚替你生成了一份」。
//
// ## 为什么它在 `06-`（紧接 fmt 之后）
//
// 阶段顺序 = 文件名序，而**并发批只把「连续标了 `parallel: true` 的阶段」合成一批**。
// 它原先叫 `28-`，正夹在 `20-frontend-static`（并行）与 `30-docs`（并行）之间，于是门禁
// 实际跑成 `fmt → [backend ∥ frontend-static] → facts-write → [docs ∥ …]`，整个 docs
// 阶段被串行在 backend 之后——没有报错、没有告警，只有把阶段墙钟打出来才看得见。
//
// 依赖关系决定它只能在这个位置：**必须**在 `05-fmt` 之后（它读 `.rs`，而 rustfmt 就地
// 重写），也**必须**在 `30-docs` 之前（docs 的 D-007 / RF-003 要看已展开的最终形态）。
// 放在批前串行跑是 10s；放进批里则会让 `tauri/src/constants/routes.gen.ts` 在
// `20-frontend-static` 读它的同时被重写——那是新的竞态，不值得为 10s 冒。
//
// ## 为什么生成可以提前，而 --check 必须留到最后
//
// 「facts 必须最后」的原话是「前面任何自动修复都可能改代码」。逐个看：`05-fmt` 在它之前；
// `10-backend` / `20-frontend-static` 只写 `target/`；`55-verify` 只写
// `.workbuddy-ai/verify-target/`。没有一个阶段在本阶段之后改**源码**，所以生成提前是安全的。
// 而 `--check` 留到 `60-facts`：它问的是「这一整轮跑完，事实还和代码一致吗」。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { autoChain, gensDeclarations } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const scriptsDir = path.join(scriptDir, '..')

const decl = gensDeclarations(repoRoot) ?? { entries: [], problems: [] }
const writers = decl.entries.filter((e) => e.modes.includes('write'))

// 读空即全绿：声明扫不出来时本阶段会「跑完 0 步」，而那与「全都对」同形。
if (writers.length === 0) {
  throw new Error(
    'facts-write 读不出任何 `// @gens <序数> write` 声明 ⇒ 生成链会空转。' +
      '先查 `gate.d/_shared.mjs::gensDeclarations` 的扫描面。',
  )
}

/**
 * 链上的步骤（导出去让 `gate.test.mjs` 判顺序）。它们是一个 `autoChain` 工作的 `steps`，
 * 不是多个门禁任务——差异判定只做在链尾一次（逐格判会在中间态恒红，见 `_shared.autoChain`）。
 */
export const STEPS = writers.map((e) => ({
  label: e.name,
  cmd: process.execPath,
  args: [path.join(scriptsDir, `${e.name}.mjs`), '--write'],
  cwd: repoRoot,
}))

export default {
  id: 'facts-write',
  title: '事实生成（静态审计的前置）',
  // 会改写并暂存受版本控制文件的自声明，消费方是 `gate-wiring-audit` 的 GW-005。
  regenerates: true,
  // 全链合成**一个** `autoChain` 工作：差异判定只看链跑完的末态。分成多个 `autoWork`
  // 在本地没区别，但在 `--ci` 下必红——`registry-audit --write` 的产物（`[[编号]]` 记号）
  // 本身就是下一格的输入差异。合成一格之后 CI / pre-push 才点得动这条链。
  tasks() {
    return [
      {
        label: `生成链（${STEPS.map((s) => s.label).join(' → ')}，末态判差异）`,
        run: (c) => autoChain(c, { label: '生成链', steps: STEPS }),
      },
    ]
  },
}
