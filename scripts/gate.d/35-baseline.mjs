// 棘轮基线**只增**：`BASELINE` 与「触碰过它的那两个版本」逐格比，任一格被削即红。
//
// 为什么单独立一个阶段：`BASELINE` 是源码常量，而它是「有测试被删就红」这条判据本身。
// 把它改小不需要动测试、不需要动守卫代码，门禁照样全绿且不留痕迹——与「测试消失」同形。
// 所以守它的判据必须**读基准版本**，而不是读同一份源码。
//
// 基准取「最近触碰过该文件的提交及其父」：只比最近一版会漏掉「已经提交的那次下调」，
// 比到分支根则会被历史里那些**本来就低**的值天天判红——那只会逼人写出空理由的豁免。
// `--base=<ref>` 是唯一覆盖入口（CI 上要指 PR base 就用它）；一个基准都取不到 ⇒ 诚实跳过。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow } from '../color.mjs'
import { baselineErosion, parseBaselineCells } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')
/** 相对仓库根：`git show` 与 `git log -- <path>` 只认这个口径 */
const SHARED = 'scripts/gate.d/_shared.mjs'

const git = (ctx, args, label) => ctx.run({ label, cmd: 'git', args, cwd: repoRoot, echo: 'none' })

/** 基准版本：`--base=<ref>` 优先；否则取最近触碰过该文件的提交及其父 */
async function baseVersions(ctx) {
  const explicit = process.argv.map((a) => /^--base=(.+)$/.exec(a)).find(Boolean)
  const refs = []
  if (explicit) {
    refs.push(explicit[1])
  } else {
    const last = await git(ctx, ['log', '-1', '--format=%H', 'HEAD', '--', SHARED], 'git log：最近触碰基线文件的提交')
    const sha = last.ok ? last.output.trim().split('\n')[0] : ''
    if (sha) refs.push(sha, `${sha}~1`)
  }
  const out = []
  for (const ref of refs) {
    // 显示口径与判据口径一致：`<sha>~1` 若按 `slice(0,8)` 打印会和子提交同名，
    // 日志里就成了「和 f9bdcbf9 / f9bdcbf9 比对」——看不出确实读了两版。
    const alias = ref.endsWith('~1') ? `${ref.slice(0, 8)}^` : ref.slice(0, 8)
    const r = await git(ctx, ['show', `${ref}:${SHARED}`], `git show ${alias}：基线文件`)
    // 基准里**没有 BASELINE 对象**的不算可比基准：拿它比会恒绿，那是一条假守卫。
    if (r.ok && r.output && parseBaselineCells(r.output).size > 0) {
      out.push({ ref: alias, text: r.output })
    }
  }
  return out
}

export default {
  id: 'baseline',
  title: '棘轮基线只增',
  tasks(ctx) {
    return [
      {
        label: 'BASELINE 相对基准版本不得下调',
        skipNote: '没有可比基准（浅克隆、无历史，或基准版本里还没有 BASELINE）⇒ 本阶段判不了',
        run: async () => {
          const bases = await baseVersions(ctx)
          if (bases.length === 0) return 'skipped'

          const current = fs.readFileSync(path.join(repoRoot, SHARED), 'utf8')
          const cells = parseBaselineCells(current)
          // 一格都没取到 ⇒ 解析口径已随 `BASELINE` 的形状一起失效。宁红不静默。
          if (cells.size === 0) {
            console.log(red('      ↳ 当前源码里取不到任何 BASELINE 数值格 ⇒ 本阶段失去判据'))
            return { ok: false, note: '解析不到 BASELINE 的数值格（形状变了？），判据失效' }
          }

          const erosion = baselineErosion(current, bases)
          if (erosion.length === 0) {
            console.log(green(`      ↳ ${cells.size} 格与 ${bases.map((b) => b.ref).join(' / ')} 逐格比对：无下调、无缺失`))
            return { ok: true }
          }
          for (const e of erosion) {
            const line = `      ↳ ${e.key}: ${e.from} → ${e.to ?? '整格被删'}（基准 ${e.source}）${e.waived ? ' · 已登记豁免' : ''}`
            console.log(e.waived ? yellow(line) : red(line))
          }
          const unwaived = erosion.filter((e) => !e.waived)
          if (unwaived.length === 0) {
            return { ok: true, note: `${erosion.length} 格下调已登记 baseline-allow 豁免` }
          }
          return {
            ok: false,
            note: `${unwaived.length} 格基线被削：${unwaived.map((e) => `${e.key} ${e.from}→${e.to ?? '删格'}`).join('、')}`,
          }
        },
      },
    ]
  },
}
