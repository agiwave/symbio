// docs 阶段：静态审计 —— **目录即清单**。
//
// 每个 `scripts/**` 下的脚本在自己的头注释里写一行 `// @gate <桶>`，本阶段扫描并分组执行：
//
//   guard   本体带 `--strict` 跑 + 同名回归测试
//   report  只跑本体（退出码恒 0，结论要人工复核）
//   test    只跑回归测试（本体要运行时输入 / 由别的阶段执行 / 根本没有本体）
//   lib     只被引用，本阶段不跑
//   off     不进门禁
//
// 回归测试按**目录事实**跑：`scripts/**/*.test.mjs` 一份不漏。
//
// 于是「加一条守卫」= 写脚本 + 写同名 `.test.mjs` + 一行 `@gate guard`，不改本文件。
// 「这条判据为什么存在、拦的是哪一种失效」写在那份脚本自己的头注释里——离代码最近，
// 改代码的人顺手就改到它（此前这里堆了 700 行复述，实测已和脚本漂成两种说法：
// `session-retire-audit` 这里写「八处决策」、脚本写「七处」，没有判据会发现对不上）。
//
// 声明是自述，所以三件事由 `gate-wiring-audit` 反向对账：声明齐不齐（GW-001）、
// 守卫有没有回归测试（GW-002）、声明的东西**是不是真的被跑到**（GW-003 / GW-004）
// ——枚举写窄了在那里红，而不是静默少跑；`-audit` 名的脚本必须在 `scripts/audits/` 下
// （GW-007），否则「审计脚本都在那个目录」只是一句话。
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { gateDeclarations } from './_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')

const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai', '.symbio'])

/** `scripts/` 下全部回归测试，相对名（`gate.d/_shared`） */
function testNames(root) {
  const out = []
  const walk = (dir, prefix) => {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      if (ent.isDirectory()) {
        if (!SKIP_DIRS.has(ent.name)) walk(path.join(dir, ent.name), `${prefix}${ent.name}/`)
        continue
      }
      if (ent.name.endsWith('.test.mjs')) out.push(`${prefix}${ent.name.replace(/\.test\.mjs$/, '')}`)
    }
  }
  walk(path.join(root, 'scripts'), '')
  return out.sort()
}

const decl = gateDeclarations(repoRoot) ?? { entries: [], problems: [] }
const names = (bucket) => decl.entries.filter((e) => e.bucket === bucket).map((e) => e.name)

// 读空即全绿是这套东西唯一的坏法：声明扫不出来时，本阶段会安安静静地只跑回归测试。
// 真仓现在 107 份脚本有声明，低于 80 就是扫描坏了（路径 / 正则 / 跳过规则），不许它绿。
const NAMES = names('guard').length + names('test').length + names('lib').length + names('off').length
if (NAMES < 80) {
  throw new Error(
    `docs 阶段读不出 @gate 声明（只读到 ${NAMES} 份，真仓应 ≥ 80）⇒ 判据会静默不跑。` +
      `先查 \`gate.d/_shared.mjs::gateDeclarations\` 的扫描面。`,
  )
}

// 报告型：退出码恒 0，判定要人工复核。`echo: 'none'` 是刻意的——它的候选会长期存在，
// 每轮刷一遍只会训练人忽略它；要看结论就单独跑一次。
const REPORT = names('report')

// 长杆：换壳重启那两份测试用真实进程起近百次系统 shell（冷启动每次数秒），比其余各项
// 之和还长。标了的先起（LPT），排在声明序中段时那段空窗是纯加项。写错最多白排一次序。
const LONG_TESTS = new Set(['windows-restart'])

export default {
  id: 'docs',
  title: '静态审计',
  // 纯只读（扫 `.rs` / docs / scripts，不写任何文件），与 backend / baseline / msrv /
  // verify 互不干扰；fmt 已在 05 屏障跑完，`verify/facts/mod.rs` 由 `06-facts-write`
  // 在本批之前生成，所以 `doc-count-audit` 读的是本轮投影。
  // ⚠️ 真的只读：往 `scripts/gate.d/` 写临时产物会被 `GW-005` 报成「未登记阶段」。
  parallel: true,
  // 并发度不写数字：任务几乎全是「spawn 一个 node、扫一遍文件」，大头在 readdir / readFile，
  // IO 密集、超订有益。给几路由 `_machine` 的 `slots.io` 定——写死核数会让小机器抢核。
  concurrencyKind: 'io',
  *tasks() {
    // 回归测试与本体各自一个进程、只读扫仓，同批并发。此前为保住每条的结论行把它们
    // 串行，代价是一整段串行尾；现在结论行由 `summary: true` 在批后补印。
    for (const name of testNames(repoRoot)) {
      yield {
        label: `${name} 回归测试`,
        cmd: process.execPath,
        args: ['--test', path.join(scriptDir, '..', `${name}.test.mjs`)],
        cwd: repoRoot,
        parallel: true,
        long: LONG_TESTS.has(name),
        // 回归测试判的是「检查器自己还准不准」，输入是脚本 + 夹具，都在 `scripts/` 下——
        // 不是本轮被检查的 `.rs` / `docs/`。所以「改了一句文档」不必重跑它们；
        // 范围求不出（浅克隆 / 无上游 / `--base` 拼错）就全跑，`--full` 强制重跑。
        when: (ctx) => ctx.scope.hits(['scripts']),
        skipNote:
          '`scripts/` 本轮未改动 ⇒ 检查器自身的回归测试沿用上一轮结论（它们判的是脚本与夹具，' +
          '不是本轮的 `.rs` / `docs/`）；要强制重跑：`--full`',
      }
    }
    for (const name of names('guard')) {
      // `--strict` 不是可选项：不传它，WARNING 级判定只打印、永不红——而那些恰恰是
      // 「需要人看一眼」的一类，于是天天打印、天天没人看。带上它，每个 WARNING 都得
      // 修掉或写下豁免理由，那次人工复核才真的发生。
      yield {
        label: `scripts/${name}.mjs`,
        cmd: process.execPath,
        args: [path.join(scriptDir, '..', `${name}.mjs`), '--strict'],
        cwd: repoRoot,
        echo: 'all',
        parallel: true,
        summary: true, // 批内静音，结论行在批后补印
      }
    }
    for (const name of REPORT) {
      yield {
        label: `scripts/${name}.mjs（报告型）`,
        cmd: process.execPath,
        args: [path.join(scriptDir, '..', `${name}.mjs`)],
        cwd: repoRoot,
        echo: 'none',
        parallel: true,
      }
    }
  },
}
