#!/usr/bin/env node
/**
 * 门禁日志的**落点**与**失败详情呈现**（`gate.mjs` 与门外读者的共享实现）。
 *
 * ## 它治的两个病
 *
 * **① 上一轮的失败证据被这一轮冲掉。** 日志原先一律写进
 * `.workbuddy-ai/gate-logs/<任务名>.log`，每次运行以 `w` 覆盖。于是一次失败之后
 * 再跑一次门禁（哪怕只是想看看别的阶段过没过），**失败那几项的证据就没了**——
 * 而门禁一轮 7 分钟，「再跑一遍看看到底哪错了」是这个仓库最贵的动作。
 * 改成**每轮一个目录**（`.workbuddy-ai/gate-logs/<runId>/`），并只保留最近
 * `KEEP_RUNS` 轮：失败可以留到下一次、下下次，但不会无限堆积。
 *
 * **② 失败只给一行文件名，正文要自己去另一个文件里翻。** 汇总里原先逐项写
 * 「· 某任务 → 某路径」，然后就没有了。真正的错误信息在那 40 行日志里，
 * 而它不出现在终端的失败处——于是人要么手动 `cat`（路径还得自己拼），
 * 要么再跑一次门禁（最坏情况：日志被覆盖，白跑）。`failureDetail` 把
 * **日志的末 N 行直接印在终端的失败处**，路径一并给出；完整输出仍在文件里。
 *
 * ## 为什么是「末 N 行」而不是全文
 *
 * 全文会把 7 分钟的输出一起糊在屏幕上，而失败的那一项往往只占几十行。
 * 末 N 行是有理由的启发式：cargo test 的 `failures:` 汇总块、e2e 的断言差异、
 * 审计脚本的 `[ERROR]` 行都落在尾部。截多少可用 `GATE_FAIL_TAIL` 调。
 * 这不是万能的（有判据把原因印在开头），但它比「什么都不印」好一个数量级，
 * 且**完整日志的路径始终在场**——真漏了也能一键打开。
 *
 * ## 门外的读者
 *
 * `scripts/e2e-concurrency.mjs`（人工复核并发度）与 `gate.d/58-e2e.mjs`（门内
 * 打印）都读 e2e 逐例日志。它们不该知道目录结构——尤其是不能各自拼一份路径。
 * 共用的取数实现是 `latestE2ELogs`：**跨所有轮次目录，按用例名取最新的那一份**。
 * 跨轮次是必要的：`--only=backend` 跑一轮之后，最新轮次里没有 e2e 日志，
 * 读「最新轮次」会得到空集，而读「最新的一份」才对。
 */

import fs from 'node:fs'
import path from 'node:path'

/** `.workbuddy-ai/` 下的日志根目录名（仓根与该常量之间只隔一层）。 */
export const LOGS_DIR_NAME = 'gate-logs'

/** 保留最近几轮（更老的删掉）。10 轮 ≈ 70 分钟的排错窗口，够用且不涨磁盘。 */
export const KEEP_RUNS = 10

/** 失败详情默认印日志的末几行；`GATE_FAIL_TAIL` 覆盖。 */
export const FAIL_TAIL_LINES = 40

/** 单个日志最多印多少字符（防某个用例刷了几万行把终端埋了）。 */
export const FAIL_TAIL_CHARS = 6000

/** 一个失败项最多细印几个日志（聚合任务可能带回几十个子日志，其余只列路径）。 */
export const MAX_DETAILED_LOGS = 3

const RUN_ID_RE = /^\d{8}-\d{6}(-\d+)?$/

/** 稳定短哈希（FNV-1a 的 32 位，取 6 位十六进制）——同一标签永远同一段。 */
function shortHash(s) {
  let h = 0x811c9dc5
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return (h >>> 0).toString(16).padStart(6, '0').slice(-6)
}

/**
 * 日志文件名（**必须唯一**：同名的两个任务会互相覆盖对方的日志）。
 *
 * `slug` 只留 ASCII 字母数字 ⇒ 中文标签被整段丢掉，两个不同的中文标签得到同一个
 * `step`。这不是理论风险：门禁里大量任务的标签是中文（「后端：cargo test」这种
 * 混排还能活，纯中文的直接撞名）。修法是**标签含非 ASCII 时追加一段稳定短哈希**：
 * 可读性由标签本身承担（汇总里印的就是标签），文件名的唯一性比好看重要。
 */
export function logFileName(label) {
  const base = label.replace(/[^a-zA-Z0-9]+/g, '-').replace(/^-|-$/g, '').toLowerCase() || 'step'
  return /^[\x20-\x7e]*$/.test(label) ? base : `${base}-${shortHash(label)}`
}

/**
 * 本轮目录名：本地时间 `YYYYMMDD-HHMMSS`，同秒再来一轮加 `-2` / `-3`。
 *
 * 用**本地时间**而不是 UTC：日志是给跑门禁的那个人看的，他手上的表就是本地时间。
 * `taken` 是注入的探针（测试里传一个假谓词，不必去建目录）。
 */
export function makeRunId(now = new Date(), taken = () => false) {
  const p = (n) => String(n).padStart(2, '0')
  const base = `${now.getFullYear()}${p(now.getMonth() + 1)}${p(now.getDate())}-${p(now.getHours())}${p(now.getMinutes())}${p(now.getSeconds())}`
  if (!taken(base)) return base
  for (let i = 2; ; i++) {
    const id = `${base}-${i}`
    if (!taken(id)) return id
  }
}

/** 删掉最老的轮次目录，只留 `keep` 个。**只删目录、只删名字像 runId 的**——
 *  目录里可能躺着本轮正在写的东西，也可能有人放了别的东西，两者都不该被动。
 *
 *  **失败不阻断**：清理是副产品，它的失败不该让一整轮门禁（10+ 分钟）在**建日志目录
 *  这一步**崩掉——那正是最坏的时机（什么都没跑，却已经红了，而且红在一个和判据毫无
 *  关系的地方）。实测形态：沙箱的批量删除守卫会对「一次删 >500 个文件」抛
 *  `SAFE_DELETE_BULK_CONFIRM_REQUIRED`，而一轮 docs 就有 60+ 个日志文件、攒几轮
 *  就过线。删不掉就留着（`KEEP_RUNS` 只是上限，不是不变量），下一轮再试。 */
export function pruneOldRuns(logRoot, keep = KEEP_RUNS) {
  if (!fs.existsSync(logRoot)) return []
  const runs = fs
    .readdirSync(logRoot, { withFileTypes: true })
    .filter((e) => e.isDirectory() && RUN_ID_RE.test(e.name))
    .map((e) => e.name)
    .sort()
    // 字典序即时间序：`YYYYMMDD-HHMMSS` 定长、同秒加 `-2` 后缀仍排在基名之后
    .reverse()
  const removed = []
  for (const name of runs.slice(keep)) {
    try {
      fs.rmSync(path.join(logRoot, name), { recursive: true, force: true })
      removed.push(name)
    } catch {
      /* 删不掉就留着：清理失败不该改变这一轮的结论 */
    }
  }
  return removed
}

/** 记下「最近一轮是哪个目录」，供门外读者定位（一行一个 id，人能直接读）。 */
export function writeRunPointer(logRoot, runId) {
  fs.mkdirSync(logRoot, { recursive: true })
  fs.writeFileSync(path.join(logRoot, 'latest-run.txt'), `${runId}\n`, 'utf8')
}

/**
 * 最近一轮的目录；没有指针或指针悬空（目录被删了）时返回 null。
 *
 * 返回 null 而不是回退到平铺目录：回退会让「其实没有最近一轮」这件事看不见，
 * 而门外的读者拿不到数据时必须**说一句**（`e2e-concurrency.mjs` 会报「没有可读的
 * 逐例耗时，先跑一次 --only=e2e」并退出 2，而不是静默打印一张空表）。
 */
export function latestRunDir(logRoot) {
  const ptr = path.join(logRoot, 'latest-run.txt')
  if (!fs.existsSync(ptr)) return null
  const id = fs.readFileSync(ptr, 'utf8').trim()
  if (!RUN_ID_RE.test(id)) return null
  const dir = path.join(logRoot, id)
  return fs.existsSync(dir) ? dir : null
}

/** 日志文本的末 N 行（去 `\r`、去尾部空行、超长截断）。 */
export function tailOf(text, { lines = FAIL_TAIL_LINES, maxChars = FAIL_TAIL_CHARS } = {}) {
  const all = String(text).replace(/\r/g, '').split('\n')
  while (all.length && all[all.length - 1].trim() === '') all.pop()
  let out = all.slice(-lines).join('\n')
  if (out.length > maxChars) out = `…（该日志末段过长，只取末 ${maxChars} 字符）\n${out.slice(-maxChars)}`
  return out
}

/**
 * 一个失败项的详情块：`{ header, body }`（都不带 ANSI，交由调用方上色）。
 *
 * `body` 是**已经缩进**的行；调用方逐行打印。没有子进程日志的项（自己的断言
 * 失败）如实说明——那种失败的原因在上方的实时输出里，不假装有文件。
 */
export function failureDetail(r, { repoRoot, lines, maxChars, maxLogs = MAX_DETAILED_LOGS } = {}) {
  const rel = (f) => path.relative(repoRoot, f).replace(/\\/g, '/')
  const list = Array.isArray(r.logFile) ? r.logFile : r.logFile ? [r.logFile] : []
  const header = `✗ ${r.label}${r.note ? ` — ${r.note}` : ''}`
  if (!list.length) {
    return { header, body: ['    （这一项没有子进程日志：失败发生在判定自身，看上方那一轮输出）'] }
  }
  const body = []
  const detailed = list.slice(0, maxLogs)
  const rest = list.slice(maxLogs)
  for (const f of detailed) {
    body.push(`    日志：${rel(f)}`)
    let tail = null
    try {
      tail = tailOf(fs.readFileSync(f, 'utf8'), { lines, maxChars })
    } catch {
      tail = null
    }
    if (!tail) {
      body.push('    （该日志读不出或为空）')
      continue
    }
    body.push(`    ── 末 ${lines} 行 ──`)
    for (const l of tail.split('\n')) body.push(`    ${l}`)
  }
  if (rest.length) {
    body.push(`    其余 ${rest.length} 个日志（同上格式，需时自行打开）：${rest.map(rel).join('、')}`)
  }
  return { header, body }
}

/**
 * 跨所有轮次，按 e2e 用例名取**最新**那份日志的解析结果。
 *
 * 返回 `{ name, ms, ok, file }[]`：`ms` 是该例耗时（日志里最后一处 `(NNNms)`），
 * `ok` 看开头是否有 `✓`。两处读者（门内打印、门外复核）共用它，口径因此不会分叉——
 * 此前同一段取数逻辑在 `58-e2e.mjs` 与 `e2e-concurrency.mjs` 里各写一份。
 */
export function latestE2ELogs(logRoot, opts = {}) {
  const dirs = []
  if (opts.runDir) dirs.push(opts.runDir)
  const latest = latestRunDir(logRoot)
  if (latest && !dirs.includes(latest)) dirs.push(latest)
  if (fs.existsSync(logRoot)) {
    for (const e of fs.readdirSync(logRoot, { withFileTypes: true })) {
      if (e.isDirectory() && RUN_ID_RE.test(e.name)) dirs.push(path.join(logRoot, e.name))
    }
  }
  const byName = new Map()
  for (const dir of dirs) {
    if (!fs.existsSync(dir)) continue
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const m = e.name.match(/^e2e-(.+)\.log$/)
      if (!e.isFile() || !m) continue
      const file = path.join(dir, e.name)
      const prev = byName.get(m[1])
      // 同名取**修改时间更新**的那份：新一轮覆盖不了旧轮的记录，所以「最新」
      // 不能再靠文件名猜，只能看 mtime。
      if (prev && prev.mtimeMs >= fs.statSync(file).mtimeMs) continue
      const text = fs.readFileSync(file, 'utf8')
      const ms = [...text.matchAll(/\((\d+)ms\)/g)].map((x) => +x[1])
      byName.set(m[1], {
        name: m[1],
        ms: ms.length ? ms[ms.length - 1] : 0,
        ok: text.slice(0, 300).includes('✓'),
        file,
        mtimeMs: fs.statSync(file).mtimeMs,
      })
    }
  }
  return [...byName.values()]
}
