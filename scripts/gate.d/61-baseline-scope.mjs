// 棘轮基线的**变更审**：`baseline-scope-audit` 的 `--base` 由本阶段显式解析并传入。
//
// 为什么不能靠人记得拼 `--base`：脚本自身在省略 `--base` 时退回 `HEAD`，而 `HEAD` 是
// 提交后的现状 —— 「刚登记过的变更」在提交后变成「没有发生的变更」，BS-005 天天假红。
// 假红多了，allow 就没人看理由，门只剩流程。
//
// 基准选择：
// ① 显式 `--base=<ref>`（调用方 / CI 指定）优先；
// ② 否则取阶段范围解析出的基准分支，再退到 `@{upstream}` / `origin/v2` / `origin/main`；
// ③ 对它取 `git merge-base HEAD <ref>` —— 这才是「这次变更从哪儿开始」的地址。
//
// 两条「判不了」必须诚实跳过，不许假绿也不许假红：
// - 合并基里还没有这两份基线文件（分支从很旧的点切出 / 本地上游过期）⇒ 没有可比的
//   前态，判不了；
// - 合并基就是 `HEAD` 且工作树没有基线改动 ⇒ 没有**在途**变更可审，判不了。
//
// 一个基准都取不到 ⇒ 同样跳过（浅克隆 / 无历史不是「通过」）。
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')

const BASELINE_FILES = [
  'scripts/loop-purity-baseline.json',
  'scripts/baselines/control-plane.json',
]

const git = (ctx, args, label) =>
  // `quiet`：探测类命令（`rev-parse` / `merge-base` / `ls-tree`）的非零退出码是**判据的一部分**
  // （「这个基准取不到」），不是失败现场。不静音的话，一次诚实跳过会在阶段头印一行红色的
  // `失败 (exit=128)`——与本项真正的结论（⊘ 判不了）相反。
  ctx.run({ label, cmd: 'git', args, cwd: repoRoot, echo: 'none', quiet: true })

export default {
  id: 'baseline-scope',
  title: '棘轮基线变更审（合并基由阶段显式解析并传入）',
  // 只读：`git ls-tree` / `merge-base` 读对象库，审计脚本只读基线文件。
  parallel: true,
  tasks(ctx) {
    return [
      {
        label: 'baseline-scope-audit --base=<合并基>',
        skipNote: '取不到可比基准（浅克隆 / 无上游 / 分支早于基线文件）⇒ 本项判不了，不是通过',
        run: async () => {
          const explicit = process.argv.map((a) => /^--base=(.+)$/.exec(a)).find(Boolean)?.[1]
          const candidates = [
            explicit,
            ctx.scope?.base,
            '@{upstream}',
            'origin/v2',
            'origin/main',
          ].filter(Boolean)

          let ref = null
          for (const c of candidates) {
            const r = await git(ctx, ['rev-parse', '--verify', '--quiet', `${c}^{commit}`], `git rev-parse ${c}`)
            if (r.ok && r.output.trim()) {
              ref = c
              break
            }
          }
          if (ref === null) return 'skipped'

          const mb = await git(ctx, ['merge-base', 'HEAD', ref], `git merge-base HEAD ${ref}`)
          const base = mb.ok ? mb.output.trim().split('\n')[0] : ''
          if (!base) return 'skipped'

          const head = await git(ctx, ['rev-parse', 'HEAD'], 'git rev-parse HEAD')
          const headSha = head.ok ? head.output.trim() : ''
          if (base === headSha) {
            // 合并基收敛到 HEAD = 没有分支变更；只有工作树里还有基线改动时才有「在途」可审。
            const dirty = await git(ctx, ['status', '--porcelain', '--', ...BASELINE_FILES], 'git status 基线改动')
            if (!dirty.ok || !dirty.output.trim()) return 'skipped'
          }

          for (const f of BASELINE_FILES) {
            // `ls-tree` 在文件不存在时**退出 0、输出为空**（`cat-file -e` 会退 128，那会把
            // 「没有前态」误呈成命令失败）。空输出就是这里要的「判不了」。
            const tree = await git(ctx, ['ls-tree', '--name-only', base, '--', f], `git ls-tree ${f}@${base.slice(0, 8)}`)
            if (!tree.ok || !tree.output.trim()) return 'skipped'
          }

          const r = await ctx.run({
            label: `baseline-scope-audit --base=${base.slice(0, 8)}`,
            cmd: process.execPath,
            args: [path.join(repoRoot, 'scripts', 'baseline-scope-audit.mjs'), `--base=${base}`],
            cwd: repoRoot,
            echo: 'all',
          })
          return r.ok ? { ok: true } : { ok: false, note: 'baseline-scope-audit 判红（见上方明细）' }
        },
      },
    ]
  },
}
