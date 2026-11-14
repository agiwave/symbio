// weekly 阶段：周档审计 —— 风格性 / 布局性 / 低频漂移判据的慢车道（DE-016 D-496）。
//
// 执行 `@gate weekly` 桶。这些判据守的是**不随单次提交变化**的性质（样式类纪律、测试
// 文件布局、行尾约定、SLO 指标对账锚）：失效不可逆损失小、每周核查足以在累积成问题前
// 抓住；让它们随每轮提交跑，只会把提交门禁的面积撑大，判不出更多的东西。
//
// 桶的声明与对账和 `guard` 完全同构（GW-001 声明齐全 / GW-002 必有回归测试 /
// GW-003 声明 ↔ 本阶段物化出的任务双向对账），差别只有**执行方与频率**：
//   - 本阶段由 `--preset=weekly`（`weekly.yml` 的 schedule job）与全量门禁执行，
//     `docs-facts` 档位不含它；
//   - 回归测试仍归 `30-docs`（GW-004 按「目录事实」枚举全部 `.test.mjs`，不分桶）。
//
// 纯只读（扫 `tauri/` / `crates/` / git blob），不读任何生成物 ⇒ 可独立成 job，
// 与生成链无次序约束。真的只读：往 `scripts/gate.d/` 写临时产物会被 `GW-005` 报成
// 「未登记阶段」。
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { gateDeclarations } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')

// 周档判据条数的下限。退役一条周档判据（删声明、删脚本）时必须**同批**把这个数改小，
// 并在退役提交里写明理由——「没人跑了」不该静默发生（与 `30-docs` 的 NAMES 下限同一条
// 纪律：读空即全绿是这套东西唯一的坏法，这里是它的周档变体）。
const MIN_WEEKLY = 4

export default {
  id: 'weekly',
  title: '周档审计',
  // ⚠️ 阶段级**不标 `parallel`**：本阶段排在 `60-facts`（必须最后的复核）之后、单独成批，
  // 没有可并发的相邻阶段——标了反而会把 `56-frontend-build` / `58-e2e` / `60-facts`
  // 这段串行尾圈进「并发批」，触发 `gate.test.mjs` 的批连续性判据。批内**任务**仍并发：
  // 任务几乎全是「spawn 一个 node、扫一遍文件」，IO 密集、超订有益，泳道数由
  // `_machine` 的 `slots.io` 定。
  concurrencyKind: 'io',
  *tasks() {
    const decl = gateDeclarations(repoRoot) ?? { entries: [], problems: [] }
    const weekly = decl.entries.filter((e) => e.bucket === 'weekly').map((e) => e.name)
    if (weekly.length < MIN_WEEKLY) {
      throw new Error(
        `weekly 桶只剩 ${weekly.length} 条声明（下限 ${MIN_WEEKLY}）⇒ ` +
          '要么声明扫描坏了（查 `gate.d/_shared.mjs::gateDeclarations` 的扫描面），' +
          '要么有周档判据被退役——退役必须有名有姓：同批把 `MIN_WEEKLY` 改小并写明理由。',
      )
    }
    for (const name of weekly) {
      // `--strict` 与 guard 同参数：WARNING 级判定不带上它就只打印、永不红。
      yield {
        label: `scripts/${name}.mjs（周档）`,
        cmd: process.execPath,
        args: [path.join(scriptDir, '..', `${name}.mjs`), '--strict'],
        cwd: repoRoot,
        parallel: true,
        summary: true, // 批内静音，结论行在批后补印
      }
    }
  },
}
