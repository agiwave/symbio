// facts 生成阶段（**在静态审计之前**）：把「代码 → 事实文件 → 汇总页 → 引用地址」
// 这一整条链跑完，让 `30-docs` 的审计看到的是**已生成、已展开**的状态。
//
// ## 为什么它必须在 30-docs 之前（这是修出来的，不是设计出来的）
//
// 原先这三步都在 `60-facts`（最后），于是 `30-docs` 跑的时候：
//   - `gen-doc-refs` 还没跑 ⇒ `[[编号]]` 记号是**未展开**的，`doc-link-audit`
//     的 D-007（锚点存在性）与 RF-003（手工地址）看到的不是最终形态；
//   - `registry-audit --write` 还没跑 ⇒ 汇总页是上一版的。
// 症状是「门禁里 doc-link-audit 红，单独跑却是绿的」——两次看到的状态不同。
// 那不是偶发，是**次序错了**：生成是审计的前置，不是后置。
//
// ## 为什么生成可以提前，而 --check 必须留到最后
//
// 「facts 必须最后」那条规矩的原话是「前面任何自动修复都可能改代码」。逐个看：
// `05-fmt`（cargo fmt）在它之前；`10-backend` / `20-frontend-static` 只写 `target/`；
// `55-verify` 重写 `docs/004-plan/verify/facts/mod.rs`（verify 程序的事实，不是本链的）。
// 没有一个阶段在本阶段之后改**源码**。所以生成提前是安全的。
//
// 而 `gen-facts --check`（重生成后复核）**留在 `60-facts`**：它问的是「这一整轮跑完，
// 事实还和代码一致吗」，只有跑完全程才有意义——放在最后，它才是真正的漂移门禁。
// 拆成「早生成、晚复核」之后，两件事各归其位。
//
// ## 链上三步的顺序不能换
//
//   gen-facts          事实文件是代码的投影
//     → registry-audit --write   汇总页是条目 front-matter 的投影（里面含 `[[…]]` 记号）
//       → gen-doc-refs           引用地址由编号解析生成
//
// 少了任一环，下一环就对着半成品工作。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { autoWork } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const genFacts = path.join(scriptDir, '..', 'gen-facts.mjs')
const regAudit = path.join(scriptDir, '..', 'registry-audit.mjs')
const docRefs = path.join(scriptDir, '..', 'gen-doc-refs.mjs')

export default {
  id: 'facts-write',
  title: '事实生成（静态审计的前置）',
  tasks(ctx) {
    return [
      {
        // 前端的路由常量是后端 `route()` 臂的投影、结构事实表是插件树的投影、
        // 判据码登记表是各脚本声明的投影、清单归属总账是 `lists` 字段的投影——
        // 四件事同源：`gen(输入) = 产物`。runner 遍历 `scripts/facts/` 一次跑完，
        // 契约破损 / 目录读不出都在这一格判红（退出码非零）。
        label: 'gen-facts（遍历 scripts/facts/，自动重新生成全部事实）',
        run: (c) =>
          autoWork(c, {
            label: 'gen-facts',
            cmd: process.execPath,
            args: [genFacts],
            cwd: repoRoot,
          }),
      },
      {
        // 登记汇总页是**条目文件的投影**（同事实文件是代码的投影）：确定性输出，
        // 写成 --check 只会因为「人忘了重跑」而红。校验判据在 30-docs 的 registry-audit。
        // 它必须在 gen-doc-refs **之前**：汇总页里渲染出来的 `[[…]]` 记号要由下一步展开。
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
