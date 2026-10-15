// e2e 并发度的**判据**（[58-e2e.mjs](./gate.d/58-e2e.mjs) 默认值 6 的依据）。
//
// ## 它回答什么
//
// 「e2e 该跑几路并发」。不是「机器有几核」——**核数不是约束，慢例才是**。
// 逐例耗时取自最近一轮跑过 e2e 的门禁日志，于是判据随代码一起变：慢用例被优化掉，
// 它自己会给出更小的建议值。
//
// ## 日志在哪（2026-10-15 起每轮一个目录）
//
// 门禁的日志原先是平铺的 `.workbuddy-ai/gate-logs/e2e-*.log`，每轮覆盖同名文件；
// 现在改成每轮一个目录（`.workbuddy-ai/gate-logs/<runId>/`）。取数因此不能只看
// 「最新一轮」——`--only=backend` 跑一轮之后最新目录里没有 e2e 日志，读到空集
// 就只会说「没有数据」。正确的是**按用例名跨轮次取最新的那一份**（`_logs.mjs`
// 的 `latestE2ELogs`，与门内 58-e2e 的同一条判据共用实现）。
//
// ## 为什么要做成脚本而不是注释
//
// 原来的判据写在注释里（「最慢通过例 15.5s vs 最小内层超时 10s，余量 5.5s」），
// 而**那个判据是错的**：它拿 `t36` 的整例耗时去比 `t11` 的内层 `waitFor` 预算，
// 跨用例比预算，算出 −5.5s 这种没有意义的负数。写成注释没人能验；写成脚本
// 下一个人跑一遍就知道对错。
//
// 用法：node scripts/e2e-concurrency.mjs            读最近一轮日志出建议
//      node scripts/e2e-concurrency.mjs --ci       有 CI 时要求不低于默认值
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { latestE2ELogs } from './gate.d/_logs.mjs'

const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const LOG_ROOT = path.join(REPO, '.workbuddy-ai', 'gate-logs')

/** 必须与 `gate.d/58-e2e.mjs` 的 `cpuCap` 上界一致。故意各写一份而不是共用导出：
 *  `gate.d/` 不 import 外部脚本（阶段模块彼此独立），共用一个常量会把两个本该
 *  独立的模块绑在一起，而它们唯一需要一致的东西只有这一个数字。
 *  **不一致时本脚本会一直说「建议上调/下调」**——那就是它该响的时候。 */
const CURRENT_DEFAULT = 6

const cases = latestE2ELogs(LOG_ROOT)
  .filter((c) => c.ms > 0)
  .sort((a, b) => b.ms - a.ms)

if (cases.length === 0) {
  console.error(`没有可读的逐例耗时（${path.relative(REPO, LOG_ROOT)}/<轮次>/e2e-*.log）。先跑一次 node scripts/gate.mjs --only=e2e。`)
  process.exit(2)
}

const total = cases.reduce((s, c) => s + c.ms, 0)
const slowest = cases[0]
const optimal = total / slowest.ms

console.log('e2e 并发度判据')
console.log('='.repeat(52))
console.log(`用例数              ${cases.length}`)
console.log(`总工作量（串行等效） ${(total / 1000).toFixed(1)}s`)
console.log(`最慢的单个用例       ${(slowest.ms / 1000).toFixed(1)}s  ${slowest.name}`)
console.log('')
console.log(`墙钟下界 = 最慢单例 = ${(slowest.ms / 1000).toFixed(1)}s（任何并发度都下不去）`)
console.log(`理论最优并发度      = ${(total / 1000).toFixed(1)} / ${(slowest.ms / 1000).toFixed(1)} ≈ ${optimal.toFixed(1)}`)
console.log('')

const suggest = Math.max(1, Math.min(Math.ceil(optimal), 8))
console.log(`建议并发度          ${suggest}（当前默认 ${CURRENT_DEFAULT}）`)

if (suggest === CURRENT_DEFAULT) {
  console.log('⇒ 与当前默认一致，保持。')
} else if (suggest < CURRENT_DEFAULT) {
  console.log(`⇒ 建议**下调**到 ${suggest}。慢用例变慢时最优并发度会降——` +
    '此时并发只是把 CPU 竞争推过超时线，不产生收益。')
} else {
  console.log(`⇒ 可以上调到 ${suggest}。**先确认失败集稳定**：` +
    `GATE_E2E_CONCURRENCY=${suggest} node scripts/gate.mjs --only=e2e 连跑 3 轮，` +
    '三轮失败集一致才落默认值。')
}

// 慢例占比：想真正提速该优化谁
const top3 = cases.slice(0, 3)
const share = top3.reduce((s, c) => s + c.ms, 0) / total
console.log('')
console.log(`慢例集中度：前三名占 ${(share * 100).toFixed(0)}% 的总工作量`)
for (const c of top3) console.log(`  ${c.name.padEnd(34)} ${(c.ms / 1000).toFixed(1)}s`)
console.log(
  share >= 0.5
    ? '⇒ 加并发治不了它们（墙钟由最慢那个决定）。要提速先优化慢例。'
    : '⇒ 工作量比较均匀，并发收益还在。',
)

if (process.argv.includes('--ci')) {
  if (suggest < CURRENT_DEFAULT) {
    console.error(`\n❌ CI：判据建议 ${suggest} < 当前默认 ${CURRENT_DEFAULT}。` +
      '降并发或优化慢例，别无视它。')
    process.exit(1)
  }
  console.log('\n✓ CI：判据与默认一致。')
}
