// facts 生成阶段（**在静态审计之前**）：把「代码 → 事实文件 → 汇总页 → 引用地址」
// 这一整条链跑完，让 `30-docs` 的审计看到的是**已生成、已展开**的状态。
//
// ## 为什么文件名是 `06-`（紧接 fmt 之后，而不是原来的 `28-`）
//
// 阶段顺序 = 文件名序，而**并发批只把「连续标了 `parallel: true` 的阶段」合成一批**
// （见 `gate.mjs` 的泳道调度）。原先它叫 `28-`，正夹在 `20-frontend-static`（并行）与
// `30-docs`（并行）之间，于是门禁实际跑成：
//
//   fmt → [backend ∥ frontend-static] → facts-write → [docs ∥ baseline ∥ msrv] → …
//
// 而不是 `gate.mjs` 文件头声明的那一整批。代价是**整个 docs 阶段（本机实测 6m25s）
// 被完整串行在 backend（9m35s）之后**——两个本该重叠的阶段首尾相接，一轮门禁白等
// 一个 docs 的墙钟。这个形态没有报错、没有告警，只有把阶段墙钟打出来才看得见。
//
// 依赖关系决定它只能在这个位置：它**必须**在 `05-fmt` 之后（它读 `.rs`，而 rustfmt
// 就地重写），也**必须**在 `30-docs` 之前（docs 的 D-007 / RF-003 要看已展开的最终
// 形态，见下）。放在批前串行跑是 10s；放进批里则会让 `tauri/src/constants/routes.gen.ts`
// 在 `20-frontend-static` 读它的同时被重写——那是新的竞态，不值得为 10s 冒。
//
// ## 为什么它必须在 30-docs 之前（这是修出来的，不是设计出来的）
//
// 链上四步都在 `60-facts`（最后）时，`30-docs` 跑的时候：
//   - `gen-doc-refs` 还没跑 ⇒ `[[编号]]` 记号是**未展开**的，`doc-link-audit`
//     的 D-007（锚点存在性）与 RF-003（手工地址）看到的不是最终形态；
//   - `registry-audit --write` 还没跑 ⇒ 汇总页是上一版的；
//   - `gen-verify-facts` 更晚（在批后的 verify 里）⇒ `doc-count-audit`（DC-002）
//     读的 `verify/facts/mod.rs` 永远是**上一轮**的投影，本轮改了 §8 表要下一轮
//     才红——一拍滞后的判据不是判据。
// 那不是偶发，是**次序错了**：生成是审计的前置，不是后置。
//
// ## 为什么生成可以提前，而 --check 必须留到最后
//
// 「facts 必须最后」那条规矩的原话是「前面任何自动修复都可能改代码」。逐个看：
// `05-fmt`（cargo fmt）在它之前；`10-backend` / `20-frontend-static` 只写 `target/`；
// `55-verify` 只写 `.workbuddy-ai/verify-target/`（它的 `gen-verify-facts` 已并入本链，
// 见上）。没有一个阶段在本阶段之后改**源码**。所以生成提前是安全的。
//
// 而 `gen-facts --check`（重生成后复核）**留在 `60-facts`**：它问的是「这一整轮跑完，
// 事实还和代码一致吗」，只有跑完全程才有意义——放在最后，它才是真正的漂移门禁。
// 拆成「早生成、晚复核」之后，两件事各归其位。
//
// ## 链上四步的顺序不能换
//
//   gen-facts          事实文件是代码的投影（`gen-verify-facts` 读 CURRENT §2，故须先跑）
//     → registry-audit --write   汇总页是条目 front-matter 的投影（里面含 `[[…]]` 记号）
//       → gen-doc-refs           引用地址由编号解析生成
//         → gen-verify-facts     verify 程序的期望数据是 01 §8 / 02 / 阶梯表的投影
//
// 少了任一环，下一环就对着半成品工作。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { autoChain } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const genFacts = path.join(scriptDir, '..', 'gen-facts.mjs')
const regAudit = path.join(scriptDir, '..', 'registry-audit.mjs')
const docRefs = path.join(scriptDir, '..', 'gen-doc-refs.mjs')
const verifyFacts = path.join(scriptDir, '..', 'gen-verify-facts.mjs')

// **链上的四步（导出去让 `gate.test.mjs` 判顺序）**：它们是一个 `autoChain` 工作的 `steps`，
// 不是四个门禁任务——差异判定只做在链尾一次（逐格判会在中间态恒红，见 `_shared.autoChain`）。
export const STEPS = [
  {
    // 前端的路由常量是后端 `route()` 臂的投影、结构事实表是插件树的投影、
    // 判据码登记表是各脚本声明的投影、清单归属总账是 `lists` 字段的投影——
    // 四件事同源：`gen(输入) = 产物`（runner 遍历 `scripts/facts/` 一次跑完，
    // 契约破损 / 目录读不出都在这一格判红）。
    label: 'gen-facts',
    cmd: process.execPath,
    args: [genFacts],
    cwd: repoRoot,
  },
  {
    // 登记汇总页是**条目文件的投影**（同事实文件是代码的投影）：确定性输出，
    // 写成 --check 只会因为「人忘了重跑」而红。校验类判据在 30-docs 的 registry-audit。
    // 它必须在 gen-doc-refs **之前**：汇总页里渲染出来的 `[[…]]` 记号要由下一步展开。
    label: 'registry-audit --write',
    cmd: process.execPath,
    args: [regAudit, '--write'],
    cwd: repoRoot,
  },
  {
    // 文档间引用**只写编号**（`[[T-008]]`），地址由本步生成/重算：编号不变则地址永远正确，
    // 文档搬家后入链自愈——「不手写地址」这句话的执行方。未登记的编号由它判红（RF-001）。
    label: 'gen-doc-refs',
    cmd: process.execPath,
    args: [docRefs],
    cwd: repoRoot,
  },
  {
    // verify 程序的期望数据是 01 §8 / 02 坐标系 / 阶梯表的投影（输入含 CURRENT §2
    // 与已展开的引用地址，故排在全链最后）。它原先在 `55-verify` 里生成——那既让
    // `doc-count-audit` 读一拍前的产物，也把整个 verify 阶段锁死在批后串行。
    // 生成前移后，`55-verify` 只剩只读程序 + 写 `verify-target/`，得以进并发批。
    label: 'gen-verify-facts',
    cmd: process.execPath,
    args: [verifyFacts],
    cwd: repoRoot,
  },
]

export default {
  id: 'facts-write',
  title: '事实生成（静态审计的前置）',
  // 会改写并暂存受版本控制文件的自声明，消费方是 `gate-wiring-audit` 的 GW-005。
  regenerates: true,
  // 四步合成**一个** `autoChain` 工作：差异判定只看链跑完的末态。
  // 分成四个 `autoWork` 在本地没区别（都是生成 + 暂存），但在 `--ci` 下必红——
  // `registry-audit --write` 的产物（`[[编号]]` 记号）本身就是下一格的输入差异。
  // 合成一格之后 CI / pre-push 才点得动这条链，登记汇总页**内容**的新鲜度因此
  // 有了末态执行方（续接 [T-250-生成链阶段id失配](../../docs/009-task/250-生成链阶段id失配.md) 口径 4）。
  tasks(ctx) {
    return [
      {
        label: '生成链（gen-facts → registry-audit → gen-doc-refs → gen-verify-facts，末态判差异）',
        run: (c) => autoChain(c, { label: '生成链', steps: STEPS }),
      },
    ]
  },
}
