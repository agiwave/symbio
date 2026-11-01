// 变更范围求值：**这一轮门禁该跑哪些阶段**。
//
// ## 它治的病
//
// 门禁原先没有「本次改了什么」这个概念：177 项、12 个阶段，每一轮全跑。实测一轮
// 冷缓存 6m58s，其中 `cargo clippy` 3m34s、`cargo doc` 4m50s、e2e 1m40s——而这
// 些阶段的**输入里根本没有 `.md`**。于是「改一句文档」的代价与「改 core 的公开面」
// 完全相同：同一个 7 分钟。`docs(…)` 是本仓最常见的一类提交（远端 v2 最近 21 个
// 提交里有 12 个），这条成本因此不是偶发的，是日常。
//
// 本模块只回答一个问题：**本轮改动触及了哪些「面」**。跑不跑由各阶段自己声明
// （`needs: [...]`），不在这里替它们决定——判据的边界属于判据的 owner。
//
// ## 三条不允许违反的性质
//
// ① **fail-open**：拿不到范围（浅克隆 / 无上游 / git 不可用 / 裸仓库）⇒ `kind: 'all'`，
//    全跑。少跑一个阶段的代价是**假绿**，多跑一轮的代价是几分钟——不对称，所以
//    不确定时一律按「全都可能受影响」处理。
// ② **不改变判定，只改变代价**：任何阶段被跳过时，汇总上有一行 `⊘` 并写明理由
//    （`gate.mjs::runStage` 负责呈现，`D-224` 的「没跑必须与跑过可区分」）。跳过
//    永远不是静默的。
// ③ **门禁自己变了就全跑**：改动 `scripts/gate.mjs` 或 `scripts/gate.d/**` 时
//    `forceAll` 生效。否则「改门禁的那一次」恰恰是唯一不按新门禁跑的一次。
//
// ## 范围怎么算
//
// `base...HEAD` 的三点差分（= merge-base 之后的改动）+ **工作区未提交改动**
// （`git status --porcelain`，含未跟踪文件）。两者缺一都不成立：只算 HEAD 会把
// 「还没提交的改动」当成没改，只算工作区会把「已经提交的」漏掉。
//
// base 的取法按可信度降序：`--base=` 显式给 > 本分支上游（`@{upstream}`）>
// `origin/v2` > `origin/main`。都取不到 ⇒ 全跑。

import { spawnSync } from 'node:child_process'

/**
 * 一个「面」= 一组同类输入。阶段用 `needs: ['rust']` 声明自己吃哪个面。
 *
 * 判据是**输入**（这些文件变了，这个阶段的结论可能变），不是「谁生产的文件」。
 * 例如 `crates/<crate>/PLUGIN.yml` 归 `rust`：它不被编译，但被 `cargo test` 与 e2e
 * **在运行时读**（插件清单是装配事实源），所以它变了就必须重跑那两条。
 */
const SURFACE_RULES = [
  // 前端源（Vue / TS / vite 配置 / 前端依赖锁）：`tauri/` 下除 `src-tauri/` 的部分。
  ['frontend', (f) => f.startsWith('tauri/') && !f.startsWith('tauri/src-tauri/') && !f.endsWith('.md')],
  // Rust 源与它的构建输入。`crates/` 与 `tauri/src-tauri/` 下的**一切非 .md 文件**
  // （含 `PLUGIN.yml` / `Cargo.toml` / 测试夹具）都算：清单在运行时被读。
  ['rust', (f) => !f.endsWith('.md') && (f.startsWith('crates/') || f.startsWith('tauri/src-tauri/'))],
  ['rust', (f) => /^(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml|rustfmt\.toml|clippy\.toml)$/.test(f)],
  // 端到端用例本身（改了用例就得跑用例）。
  ['e2e', (f) => f.startsWith('e2e/')],
  // 门禁与审计脚本：`scripts/` 下的一切。
  ['scripts', (f) => f.startsWith('scripts/')],
  // 文档：`docs/` 与任何位置的 `.md`（含 `crates/*/README.md`——它进 rustdoc 之外的
  // 文档判据，但不进 cargo 编译，故不归 rust）。
  ['docs', (f) => f.startsWith('docs/') || f.endsWith('.md')],
]

/** 门禁自身：改了它就不许再按「范围」少跑，否则这次改动没被验证过。 */
const GATE_SELF = [/^scripts\/gate\.mjs$/, /^scripts\/gate\.d\//]

/** 全部面（用于 `--full` / 范围不可得时的判定式）。 */
export const ALL_SURFACES = Object.freeze([...new Set(SURFACE_RULES.map(([n]) => n)), 'gate'])

function git(cwd, args) {
  const r = spawnSync('git', args, { cwd, encoding: 'utf8', shell: false, windowsHide: true })
  if (r.error || r.status !== 0) return null
  return r.stdout
}

/**
 * `git status --porcelain -z` 的路径列。
 *
 * `-z` 不是可选的：默认格式对含空格 / 中文的路径加引号并转义，而本仓的文档路径
 * 全是中文——按默认格式切行，一半路径会带着引号，`startsWith('docs/')` 全部落空，
 * 于是「改了文档」被判成「什么都没改」，方向恰好是**假绿**。
 */
function worktreePaths(cwd) {
  const raw = git(cwd, ['status', '--porcelain=v1', '-z', '--untracked-files=all'])
  if (raw === null) return []
  // 每条记录：`XY <path>\0`；重命名/复制是 `XY <old>\0<new>\0`（旧名在前）。
  // 两条都要：改名的「新名」才是本轮的形状，而旧名让「删掉了哪个阶段文件」可见。
  const out = []
  const parts = raw.split('\0')
  for (let i = 0; i < parts.length; i++) {
    const rec = parts[i]
    if (!rec || rec.length < 4) continue
    const status = rec.slice(0, 2)
    out.push(rec.slice(3))
    if (/[RC]/.test(status)) {
      const from = parts[++i]
      if (from) out.push(from)
    }
  }
  return out
}

/**
 * 求本轮范围。
 *
 * @param {object} o
 * @param {string} o.repoRoot 仓库根
 * @param {string|null} [o.base] 显式基准（`--base=`）
 * @param {boolean} [o.full] 强制全跑（`--full` / `--ci`）
 * @returns {{kind:'all'|'diff', base:string|null, files:string[], surfaces:string[],
 *            forceAll:boolean, reason:string, hits:(needs:string[])=>boolean, brief:()=>string}}
 */
export function resolveScope({ repoRoot, base = null, full = false }) {
  const decided = (o) => Object.freeze({ files: [], surfaces: [...ALL_SURFACES].sort(), forceAll: true, hits: () => true, ...o })

  if (full) {
    return decided({
      kind: 'all',
      base: null,
      reason: '强制全跑（--full / --ci）',
      brief: () => 'all（强制）',
    })
  }

  // 基准：显式 > 上游 > 两个远端主分支。**显式给了却解析不出来 ⇒ 全跑**：
  // 拼错的 base 若静默退化成「没给」，就变成用别的基准算范围——与 `--only` 拼错
  // 同一个病（把「我要求 X」悄悄执行成「我要求 Y」）。
  const candidates = base ? [base] : ['@{upstream}', 'origin/v2', 'origin/main']
  let picked = null
  for (const c of candidates) {
    if (git(repoRoot, ['rev-parse', '--verify', '--quiet', `${c}^{commit}`]) !== null) {
      picked = c
      break
    }
  }
  if (!picked) {
    return decided({
      kind: 'all',
      base: null,
      reason: base
        ? `基准 \`${base}\` 解析不出来（拼错 / 未 fetch）`
        : '没有可用的基准（无上游、无 origin/v2 / origin/main、或不在 git 仓里）',
      brief: () => 'all（范围不可得）',
    })
  }

  const diff = git(repoRoot, ['diff', '--name-only', '-z', `${picked}...HEAD`])
  if (diff === null) {
    return decided({
      kind: 'all',
      base: picked,
      reason: `\`git diff ${picked}...HEAD\` 失败（浅克隆 / 对象缺失）`,
      brief: () => 'all（差分读不出）',
    })
  }

  const committed = diff.split('\0').filter(Boolean)
  const worktree = worktreePaths(repoRoot)
  const files = [...new Set([...committed, ...worktree])].sort()
  const surfaces = new Set()
  // **只认工作区未提交的门禁改动**，不认「与基准的差分」：后者会让整个分支生命周期
  // 都强制全跑（这条改动一旦提交，它就一直躺在 `base..HEAD` 里），范围收窄对本分支
  // 就永久失效了。而本条要守的是**那一刻**——正在改门禁、还没验证过它的这一轮。
  // 改完并提交之后，那一次全跑已经发生过，结论成立。
  const forceAll = worktree.some((f) => GATE_SELF.some((re) => re.test(f)))
  for (const f of files) {
    for (const [name, test] of SURFACE_RULES) if (test(f)) surfaces.add(name)
  }
  if (forceAll) surfaces.add('gate')

  const list = [...surfaces].sort()
  const hits = (needs) => {
    if (forceAll) return true
    if (!needs || needs.length === 0) return true
    return needs.some((n) => surfaces.has(n))
  }
  const brief = () => (files.length === 0 ? '无改动' : `${files.length} 个文件｜${list.join('/') || '未分类'}`)

  return {
    kind: 'diff',
    base: picked,
    files,
    surfaces: forceAll ? [...ALL_SURFACES] : list,
    forceAll,
    reason: `基准 ${picked}｜${brief()}${forceAll ? '｜门禁自身有未提交改动 ⇒ 全跑' : ''}`,
    hits,
    brief,
  }
}

/** 阶段跳过时的那句话。**理由必须具体到「哪个面没被碰到」**，不能只说「跳过」。 */
export function skipReason(scope, needs) {
  return (
    `本轮改动未触及 ${needs.join(' / ')} ⇒ 本阶段跳过（${scope.brief()}）；` +
    '要强制全跑：`--full`（CI 恒全跑）'
  )
}
