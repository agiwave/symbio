// 棘轮元守卫：**每一处「只许往紧改」的基线都不得被放松**（`RATCHETS` 清单驱动）。
//
// ## 为什么单独立一个阶段
//
// 棘轮基线是源码常量，而它就是「有测试被删就红」这条判据本身。把它改小不需要动测试、
// 不需要动守卫代码，门禁照样全绿且不留痕迹——与「测试消失」同形。所以守它的判据必须
// **读基准版本**，而不是读同一份源码。
//
// 基准取「最近触碰过该文件的提交及其父」：只比最近一版会漏掉「已经提交的那次下调」，
// 比到分支根则会被历史里那些**本来就低**的值天天判红——那只会逼人写出空理由的豁免。
// `--base=<ref>` 是唯一覆盖入口（CI 上要指 PR base 就用它）；一个基准都取不到 ⇒ 诚实跳过。
//
// ## 为什么要一张清单而不是一个写死的路径（批 M12）
//
// 此前 `SHARED` 是常量，只守 `_shared.mjs` 的 `BASELINE` 一个对象。同类棘轮另有三条
// 全在守卫之外，其中两条还是 `process.env` 读的——**可逐次调用覆盖**，于是
// 「默认值钉得死、CI 传个更松的值就绕过」是件不需要改任何文件的事。
//
// 清单每项 = 落点文件 + 提取函数 + 方向 + 豁免标记名。加一条落点不必重新决定一次
// 「什么算削判据」——那一条由 `_shared.ratchetErosion` 定义一次（与 M2 把
// `ratchetVerdict` 收进单点同理由）。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, yellow } from '../color.mjs'
import {
  baselineWaivers,
  parseBaselineCells,
  ratchetErosion,
} from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')

const git = (ctx, args, label) => ctx.run({ label, cmd: 'git', args, cwd: repoRoot, echo: 'none' })

// ── 三个提取函数：各落点的形状不同，判据的**读法**也必须不同 ─────────

/** `scripts/core-export-audit.mjs` 的 `BASELINE`：默认值写在三元里，形如 `{ zero: 0, one: 61 }`。
 *  **只取默认分支**（`: { … }` 那一侧）——env 覆盖值不进基线判定，它由 `envLoosened` 单判。 */
function coreExportCells(text) {
  const cells = new Map()
  const m = /:\s*\{\s*zero:\s*(\d+),\s*one:\s*(\d+)\s*\}/.exec(text.replace(/\r\n/g, '\n'))
  if (m) {
    cells.set('zero', Number(m[1]))
    cells.set('one', Number(m[2]))
  }
  return cells
}

/** `scripts/dead-code-audit.mjs` 的 `R002_BASELINE`：默认值是三元尾部的那个字面量。
 *  注意三元问号那一侧是 `? Number(…)` 而**不是** `: Number(…)`——写成后者会永远取不到
 *  格，于是这一项在日志里显示成「跳过」，看上去无害，实则这条棘轮**已经没人守了**。 */
function deadCodeCells(text) {
  const cells = new Map()
  const norm = text.replace(/\r\n/g, '\n')
  const m = /\?\s*Number\(process\.env\.DEAD_CODE_R002_BASELINE\)\s*\n\s*:\s*(\d+)/.exec(norm)
  if (m) cells.set('R-002', Number(m[1]))
  return cells
}

/** `tauri/vitest.config.ts` 的 `coverage.thresholds`：一个嵌套块，四条全局 + 一条分目录。
 *  **只扫 `thresholds: {` 块**——同一文件里别处也有 `lines:` 这类键。 */
function coverageCells(text) {
  const cells = new Map()
  const norm = text.replace(/\r\n/g, '\n')
  const start = norm.indexOf('thresholds: {')
  if (start < 0) return cells
  const body = norm.slice(start)
  const end = body.indexOf('\n      }')
  const lines = (end < 0 ? body : body.slice(0, end)).split('\n')
  for (const line of lines) {
    // 分目录那条：`'src/registry/**': { lines: 87, statements: 88 }`
    const scoped = /^\s*'([^']+)':\s*\{([^}]*)\}/.exec(line)
    if (scoped) {
      for (const m of scoped[2].matchAll(/(lines|statements|functions|branches):\s*(\d+)/g)) {
        cells.set(`${scoped[1]}.${m[1]}`, Number(m[2]))
      }
      continue
    }
    const m = /^\s*(lines|statements|functions|branches):\s*(\d+),/.exec(line)
    if (m) cells.set(m[1], Number(m[2]))
  }
  return cells
}

/** 豁免注释：沿用各守卫自己的标记名（`baseline-allow` / `core-export-allow` /
 *  `dead-code-allow`），不新造第四种拼写——那会让豁免散在四种语法里。 */
function waiverReader(tag) {
  const re = new RegExp(`^ {0,2}//\\s*${tag}\\s+([A-Za-z0-9_:/-]+):\\s*(\\S[^\\n]*)$`, 'gm')
  return (text) => {
    const out = new Map()
    for (const m of text.replace(/\r\n/g, '\n').matchAll(re)) out.set(m[1], m[2].trim())
    return out
  }
}

// ── 棘轮清单 ──────────────────────────────────────────────────────────
// `dir`：`floor` = 只增（下限），`ceiling` = 只减（上限）。
// `envVar`：该落点的值可经环境变量覆盖 ⇒ 额外判「覆盖值不得比默认值更松」。
//   **这一条不是多余的**：env 覆盖不动任何文件，不留痕迹，而它是 CI 那一跑的
//   实际判据——默认值钉得再死，一行 `--env` 就能绕过去。
const RATCHETS = [
  {
    id: 'baseline',
    file: 'scripts/gate.d/_shared.mjs',
    dir: 'floor',
    cellsOf: parseBaselineCells,
    waivers: waiverReader('baseline-allow'),
    label: 'BASELINE（Rust / CI / 前端用例数、verify 程序数与断言数、e2e 用例数）',
  },
  {
    id: 'core-export',
    file: 'scripts/core-export-audit.mjs',
    dir: 'ceiling',
    cellsOf: coreExportCells,
    waivers: waiverReader('core-export-allow'),
    envVar: 'CORE_EXPORT_BASELINE',
    label: 'core 根出口的 0 / 单消费方符号存量（只许降）',
  },
  {
    id: 'dead-code',
    file: 'scripts/dead-code-audit.mjs',
    dir: 'ceiling',
    cellsOf: deadCodeCells,
    waivers: waiverReader('dead-code-allow'),
    envVar: 'DEAD_CODE_R002_BASELINE',
    label: 'R-002 `#[allow(dead_code)]` 存量（只许降）',
  },
  {
    id: 'coverage',
    file: 'tauri/vitest.config.ts',
    dir: 'floor',
    cellsOf: coverageCells,
    waivers: waiverReader('coverage-allow'),
    label: '前端覆盖率阈值（只许升）',
  },
]

/** 基准版本：`--base=<ref>` 优先；否则取最近触碰过**该落点文件**的提交及其父 */
async function baseVersions(ctx, ratchet) {
  const explicit = process.argv.map((a) => /^--base=(.+)$/.exec(a)).find(Boolean)
  const refs = []
  if (explicit) {
    refs.push(explicit[1])
  } else {
    const last = await git(
      ctx,
      ['log', '-1', '--format=%H', 'HEAD', '--', ratchet.file],
      `git log：最近触碰 ${ratchet.file} 的提交`,
    )
    const sha = last.ok ? last.output.trim().split('\n')[0] : ''
    if (sha) refs.push(sha, `${sha}~1`)
  }
  const out = []
  for (const ref of refs) {
    // 显示口径与判据口径一致：`<sha>~1` 若按 `slice(0,8)` 打印会和子提交同名，
    // 日志里就成了「和 f9bdcbf9 / f9bdcbf9 比对」——看不出确实读了两版。
    const alias = ref.endsWith('~1') ? `${ref.slice(0, 8)}^` : ref.slice(0, 8)
    const r = await git(ctx, ['show', `${ref}:${ratchet.file}`], `git show ${alias}：${ratchet.file}`)
    if (r.ok && r.output && ratchet.cellsOf(r.output).size > 0) {
      out.push({ ref: alias, text: r.output })
    } else if (r.ok && r.output) {
      // ⚠️ 基准**取到了**却一格都解析不出 ⇒ 提取函数与这处落点的形状脱节了。
      // 这不是「判不了」，是「判据已经失效」——而两种情况在日志里长得**一模一样**
      // （都显示「跳过」）。混为一谈的后果是：一条棘轮悄悄没人守，而门禁全绿。
      // 与下面 `sha` 为空（浅克隆 / 从未提交 ⇒ 真判不了）区分开，宁红不静默。
      out.push({ ref: alias, text: r.output, unreadable: true })
    }
  }
  return out
}

/**
 * env 覆盖值是否比默认值更松。
 *
 * 两条 env 棘轮的默认值是**提交进仓库的字面量**，而门禁那一跑读的是 `process.env`。
 * 只守默认值的话，`CORE_EXPORT_BASELINE=0:999` 这种覆盖不需要改任何文件就能把上限
 * 放到天边，且不留痕迹——比改默认值更隐蔽。所以覆盖值本身也要判。
 */
function envLoosened(ratchet, defaults) {
  if (!ratchet.envVar) return null
  const raw = process.env[ratchet.envVar]
  if (raw === undefined || raw === '') return null
  const override = new Map()
  if (raw.includes(':')) {
    const [zero, one] = raw.split(':').map((n) => Number(n))
    override.set('zero', zero).set('one', one)
  } else {
    const n = Number(raw)
    if (Number.isNaN(n)) return { error: `${ratchet.envVar}=${raw} 不是数字` }
    override.set('R-002', n)
  }
  const bad = []
  for (const [key, want] of override) {
    const have = defaults.get(key)
    if (have === undefined) {
      bad.push(`${key} 在默认值里没有这一格（env 覆盖=${want}）`)
      continue
    }
    const looser = ratchet.dir === 'ceiling' ? want > have : want < have
    if (looser) bad.push(`${key} env 覆盖 ${want} 比默认值 ${have} 更松`)
  }
  return bad.length ? { bad } : null
}

export default {
  id: 'baseline',
  title: '棘轮元守卫（每处基线只许往紧改）',
  // 阶段级并发：本阶段**只读**（git log/show + 读落点文件），批次里没有另一个
  // `.git` 写方——autoWork 的 `git add` 只发生在 fmt（05，已跑完）、verify（55，
  // 不在批内）与 facts（60，最后），与本阶段并发的是 backend / frontend-static /
  // docs / msrv 四个泳道（只写 `target/` 或 `.workbuddy-ai/msrv-target/`，或纯读
  // ——frontend-static 对磁盘实际只读，见其文件头）。
  parallel: true,
  tasks(ctx) {
    return RATCHETS.map((ratchet) => ({
      label: `${ratchet.file}：${ratchet.label} 不得比基准版本放松`,
      skipNote: `没有可比基准（浅克隆、无历史，或基准版本里还没有这处基线）⇒ 本项判不了`,
      run: async () => {
        const bases = await baseVersions(ctx, ratchet)
        if (bases.length === 0) return 'skipped'

        const unreadable = bases.filter((b) => b.unreadable)
        if (unreadable.length) {
          console.log(
            red(
              `      ↳ 基准 ${unreadable.map((b) => b.ref).join(' / ')} 里读不出 ${ratchet.file} 的基线格` +
                ` ⇒ 提取函数与这处落点的形状脱节，本项判据已失效（不是「没基准可判」）`,
            ),
          )
          return { ok: false, note: `提取函数读不出 ${ratchet.file} 的基线格，判据失效` }
        }

        const current = fs.readFileSync(path.join(repoRoot, ratchet.file), 'utf8')
        const cells = ratchet.cellsOf(current)
        // 一格都没取到 ⇒ 提取口径已随这处基线的形状一起失效。宁红不静默。
        if (cells.size === 0) {
          console.log(red(`      ↳ 当前 ${ratchet.file} 里取不到任何基线数值格 ⇒ 本项失去判据`))
          return { ok: false, note: `解析不到 ${ratchet.file} 的数值格（形状变了？），判据失效` }
        }

        const problems = []
        const erosion = ratchetErosion({ dir: ratchet.dir }, ratchet.cellsOf, current, bases, ratchet.waivers(current))
        for (const e of erosion) {
          const line = `      ↳ ${e.key}: ${e.from} → ${e.to ?? '整格被删'}（基准 ${e.source}）${e.waived ? ' · 已登记豁免' : ''}`
          console.log(e.waived ? yellow(line) : red(line))
        }
        const unwaived = erosion.filter((e) => !e.waived)
        if (unwaived.length) {
          problems.push(
            `${unwaived.length} 格被放松：${unwaived
              .map((e) => `${e.key} ${e.from}→${e.to ?? '删格'}`)
              .join('、')}`,
          )
        }

        const env = envLoosened(ratchet, cells)
        if (env?.error) {
          console.log(red(`      ↳ ${env.error}`))
          problems.push(env.error)
        } else if (env?.bad) {
          for (const b of env.bad) console.log(red(`      ↳ ${b}`))
          problems.push(`${ratchet.envVar} 覆盖比默认值更松：${env.bad.join('；')}`)
        }

        if (problems.length) return { ok: false, note: problems.join('；') }
        console.log(
          green(
            `      ↳ ${cells.size} 格与 ${bases.map((b) => b.ref).join(' / ')} 逐格比对：无放松` +
              (ratchet.envVar ? `（${ratchet.envVar} 未被更松地覆盖）` : ''),
          ),
        )
        return { ok: true }
      },
    }))
  },
}