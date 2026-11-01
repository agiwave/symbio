// e2e 用例的**起跑顺序**：按上一轮实测耗时降序（LPT），而不是按文件名序。
//
// ## 为什么这件事值一提
//
// 墙钟 = 「最后完成的那条泳道」的耗时，不是总工作量的平均值。用例耗时极度长尾
// （改造前实测 88 例：前 10 例占 72% 的工作量，`t90` 一例 65s、`t36` 45s、`t53` 38s，
// 而后 59 例都在 1s 内）。并发 6 路时按文件名序起跑，那三条慢例在序号 36/53/90
// 才被捞起来——**先跑完的泳道就空转着等它们**。
//
// 实测（本机 20 核，并发 6，同一批用例，慢例已被 `D-275` 削平之后）：
//
// ```
// 文件名序（FIFO）  墙钟 50.7s
// LPT（实测耗时）   墙钟 36.8s   ← 已是 219/6 = 36.5s 的下界
// ```
//
// 排队的成因与「并发度不够」不是一回事：并发度再高也不能让一条 20s 的用例提前结束，
// 只能让它**尽早开始**。所以这里只改起跑顺序，一个用例都没少跑、一条判据都没放宽。
//
// ## 耗时从哪来
//
// 上一轮自己的实测结果（`.workbuddy-ai/e2e-case-times.json`），不是手填的常量：
// 用例变慢/变快时它下一轮自动跟上，不会像硬编码的权重那样悄悄过期。
// 读不到（首次跑、文件损坏）⇒ 全部等权 ⇒ 退化成文件名序，与改动前的行为一致。
//
// ## 为什么端口基址仍按**原序号**
//
// 端口分段是「按用例序号注入 `18080 + i*100`」（`58-e2e.mjs` 的长注释），序号是
// 地址的一部分：换顺序就换号段。这里因此把 `index`（原序号）与 `ms`（排期权重）
// 分开带走——**只改谁先跑，不改谁用哪段端口**。
import fs from 'node:fs'
import path from 'node:path'

/** 实测耗时缓存的位置（相对仓库根）。与门禁日志同目录：都是「本机工具链的账」。 */
export const TIMES_FILE = path.join('.workbuddy-ai', 'e2e-case-times.json')

/**
 * 并发度的**结构上限**（与机器无关）。
 *
 * 机器槽位（`_machine.mjs` 的 `slots.e2e`）是「这台机器开得动几路」，这里是「再多也没用」：
 * 用例里有几条要真等生产定时器（`t36`/`t90` 等 tick、`t53` 等退避阶梯），它们决定了
 * 并发度的收益上限——再多路也不能让一条 20s 的用例提前结束。实测（88 例 / 总工作量
 * 219s / 最慢 20.4s，本机 20 核）：并发 6 → 36.8s，10 → 22.5s，14 → 20.7s。
 * 14 路相对 10 路只再省 1.8s，却把进程数抬了 40%——所以封顶 12。
 *
 * 上报给 `_machine.mjs` 当上限（那边不再自己写一个数）。
 */
export const CONCURRENCY_CEILING = 12

/** 缓存里只认这些形状：正有限数。别的一律当「没测过」。 */
function usable(ms) {
  return typeof ms === 'number' && Number.isFinite(ms) && ms > 0
}

function median(list) {
  const s = [...list].sort((a, b) => a - b)
  const m = Math.floor(s.length / 2)
  return s.length % 2 ? s[m] : Math.round((s[m - 1] + s[m]) / 2)
}

/**
 * 算出起跑顺序。
 *
 * @param cases 用例清单（顺序 = 文件名序，`index` 就是端口分段用的序号）
 * @param times `{ 用例名: 毫秒 }`，来自 {@link loadTimes}
 * @returns `[{ case, index, ms }]`，`ms` 降序；**没测过的按已知耗时的中位数排**，
 *          不排到最后——新用例默认慢是更安全的猜测（宁可早跑，别拖尾）。
 */
export function orderForStart(cases, times = {}) {
  const known = cases.map((c) => times[c.name]).filter(usable)
  const fallback = known.length ? median(known) : 0
  return cases
    .map((c, index) => ({ case: c, index, ms: usable(times[c.name]) ? times[c.name] : fallback }))
    .sort((a, b) => b.ms - a.ms || a.index - b.index)
}

/** 读上一轮的实测耗时；任何异常都当作「没有数据」（退化成文件名序）。 */
export function loadTimes(repoRoot) {
  try {
    const o = JSON.parse(fs.readFileSync(path.join(repoRoot, TIMES_FILE), 'utf8'))
    return o && typeof o === 'object' && !Array.isArray(o) ? o : {}
  } catch {
    return {}
  }
}

/**
 * 建议并发度 = `min(机器槽位, ⌈总工作量 / 最慢单例⌉)`。
 *
 * 这是本仓自己对并发度的判据（`58-e2e.mjs` 的长注释、`scripts/e2e-concurrency.mjs`
 * 的离线复核都是这一条）：**并发吃不掉最慢的那一例**，所以超过「总工作量除以它」
 * 的并发只会买到调度抖动。旧实现把这个数写死成 6（当时算出来是 5.7）——
 * 用例耗时一变它就过期，且过期方向只有「白跑」和「拖尾」两种。
 *
 * 现在它跟着实测走：慢例被优化掉 ⇒ 建议值自动上调；尾部长例回归 ⇒ 自动收缩，
 * 不需要谁记得回来改常量。
 *
 * @param machineSlots 机器档案的 `slots.e2e`（按核数 / 内存算出的上限，含人工覆盖）
 * @param fallback     一条耗时都没有时的默认值（= 改动前的常量）
 */
export function suggestConcurrency({ cases, times = {}, machineSlots, fallback = 6 }) {
  const cap = Math.max(1, Math.floor(machineSlots))
  const known = cases.map((c) => times[c.name]).filter(usable)
  if (known.length === 0) return Math.min(cap, fallback)
  const total = known.reduce((a, b) => a + b, 0)
  const slowest = Math.max(...known)
  return Math.max(1, Math.min(cap, Math.ceil(total / slowest)))
}

/**
 * 记下本轮实测耗时，供下一轮排期。
 *
 * 只写**真跑过**的（`ms > 0`）：被跳过、被杀、超时兜底的那几条不该用自己的意外
 * 时长去影响下一轮的顺序。写失败不抛——排期是优化，不是判据。
 */
export function recordTimes(repoRoot, measured) {
  const out = {}
  for (const [name, ms] of Object.entries(measured ?? {})) if (usable(ms)) out[name] = Math.round(ms)
  try {
    const file = path.join(repoRoot, TIMES_FILE)
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(file, `${JSON.stringify(out, null, 2)}\n`)
  } catch {
    /* 排期是优化，不是判据：写不进去就下一轮按文件名序 */
  }
}
