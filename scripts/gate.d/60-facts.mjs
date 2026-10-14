// facts 阶段：事实文件由代码生成（**必须最后**——前面任何自动修复都可能改代码）。
//
// 生成是**门禁自己做的事**，不是判它「有没有做过」：`gen(代码) → 事实文件` 是
// 确定性的，写成 `--check` 只会因为「人忘了重跑」而红。见 `_shared.autoWork`。
//
// 「事实文件」指 `scripts/facts/` 目录下每份 `.mjs` 投影出来的文档或源码
// （见该目录的 README）。目录即接线：新增一份事实只要建一个文件，
// 这里**不需要**再抄一段调用——那正是把生成收进目录要换掉的东西。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { autoWork } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
const genFacts = path.join(scriptDir, '..', 'gen-facts.mjs')
const regAudit = path.join(scriptDir, '..', 'registry-audit.mjs')
const docRefs = path.join(scriptDir, '..', 'gen-doc-refs.mjs')

export default {
  id: 'facts',
  title: '事实文件（必须最后）',
  tasks(ctx) {
    return [
      {
        // 前端的路由常量是后端 `route()` 臂的投影、结构事实表是插件树的投影、
        // 判据码登记表是各脚本声明的投影——三件事同源：`gen(代码) = 产物`。
        // runner 遍历 `scripts/facts/` 一次跑完，契约破损 / 目录读不出都在
        // 这一格里判红（退出码非零）。
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
        // 「重生成之后仍然一致」在这里才成立——上面那一步刚把事实文件对齐到代码，
        // 本步再核对一遍。漂移的判定**必须**落在重生成之后：门禁的 `30-docs` 阶段跑
        // 事实回归测试时事实文件还是上一版的（那是正常的），所以那条测试刻意不碰漂移。
        // 本机这一步恒绿（生成器刚写过）；CI 侧它与 autoWork 的差异判定互为两道。
        label: 'gen-facts --check（重生成后复核：三份事实与代码一致）',
        run: (c) =>
          ctx.run({
            label: 'gen-facts --check',
            cmd: process.execPath,
            args: [genFacts, '--check'],
            cwd: repoRoot,
          }),
      },
      {
        // 登记汇总页是**条目文件的投影**（同事实文件是代码的投影）：确定性输出，
        // 写成 --check 只会因为「人忘了重跑」而红。校验判据在 30-docs 的 registry-audit。
        // 注意它与 gen-facts 是**不同种类**的工作：输入是 front-matter 而非代码，
        // 判定与投影混在一张表里（registry-audit 一处既判又写）。
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