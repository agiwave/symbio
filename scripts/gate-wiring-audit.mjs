#!/usr/bin/env node
/**
 * gate-wiring-audit — 门禁接线对账（GW-001 / GW-002 / GW-003）
 *
 * 治的病：**测试文件存在 ≠ 会被门禁跑**。`gate.d/30-docs.mjs` 的 `GUARDS` / `TEST_ONLY`
 * 是两份手写名单（每条都带着「为什么值得跑」的说明，所以它不能被目录扫描替掉），
 * 而新增一份 `.test.mjs` 的人不会知道要去那里登记——于是它红了也没人知道，
 * 比没有测试更糟：仓里躺着一份「看起来有人在跑」的测试。
 *
 * 判据（名单与目录事实互为对账，六个方向都判）：
 *   - **GW-001** `scripts` 目录（递归）里的每一份 `X.test.mjs`，其去后缀名 `X` 必须出现在
 *     `GUARDS ∪ TEST_ONLY`，否则红——「存在但从不运行」的测试。
 *   - **GW-002** 三张名单里指名的每一个 `<名>` 必须在磁盘上存在，
 *     否则红——文件被改名/搬走/删掉后，门禁那一步会静默跳过（`node --test` 找不到
 *     文件时报的是「0 个测试」，不是失败）。
 *   - **GW-003** 同一个名不得同时在 `GUARDS` 与 `TEST_ONLY`：前者跑两遍（回归测试 +
 *     守卫本体），后者只跑一遍，两处语义不同却都算「接了线」。
 *   - **GW-004**（命名空间闭合）`scripts/` 下每一个**非测试** `.mjs` 必须落在
 *     `GUARDS ∪ TEST_ONLY ∪ LIBS ∪ OUT_OF_GATE` 之一，否则红。
 *     这是 GW-001 的镜像缺口：GW-001 扫的是 `.test.mjs`，于是一份**既没有回归测试、
 *     又不在任何名单里**的 `.mjs`（守卫 / 生成器 / 共享库）是完全隐形的——写一份
 *     `foo-audit.mjs` 忘了登记，它就静默地不存在，而门禁照旧全绿。实测的现成例子是
 *     `route-facts.mjs`：M10 把它立为「什么算一条路由臂」的唯一语法实现、被三个判定
 *     共读，它自己却没有一道门禁步骤。**放行对照**：`scripts/gate.d/*.mjs` 的阶段文件
 *     由 `gate.mjs` **按目录扫描**接入（不需要登记），它们的顺序由 GW-005 判。
 *   - **GW-005**（阶段顺序）`gate.mjs` 按**文件名排序**加载 `gate.d/*.mjs`，本条判三件事：
 *     ① 阶段自声明（`regenerates` / `recheck`）与源码里的 `autoWork(` / `autoChain(` **调用形状**
 *     双向对上——声明了却没调用、调用了却没声明都红；② 有 `regenerates` 就必须有 `recheck`；
 *     ③ **最后一个生成阶段必须早于每一个只读复核阶段**。
 *     ②③ 合起来是这条判据真正想保护的东西：**改写工作树的工作全部做完之后，还得有人只读地
 *     对一次末态**，否则「谁碰过文件」只能靠 `git status` 的位置巧合来看。
 *     它**不是**旧写法的那条「重生成阶段必须排在阶段序列末尾」——那条已被实测否证：
 *     生成链（`06-facts-write`）排在静态审计**之前**是刻意的（`30-docs` 的 D-007 / RF-003 要看
 *     展开后的末态，`gate.test.mjs` 钉着 `facts-write < docs` 这条不变量），实测 11 个阶段里
 *     只有 `05-fmt` 与 `06-facts-write` 真的调用重生成原语，其余阶段跑完后 `git status` 不变。
 *     被否决的另一支（把 `06` 挪到末尾以满足旧字面）会让 CI 的静态审计读上一版投影，代价更大。
 *     旧写法还有一个更隐蔽的病：它用 `includes('autoWork')` 认阶段，于是**注释措辞**决定判定——
 *     删掉 `60-facts.mjs` 头里一句叙述（零行为改动）实测报出 5 条假阳性，而反过来把真正在
 *     暂存文件的 `06-facts-write` 那句注释改成人话，这个生成阶段就从判定里消失、判据空转。
 *   - **GW-006**（阶段不空转）每个 `gate.d/*.mjs` 在本机**物化一次 `tasks()`**，产出 0 个检查项
 *     又没声明 `mayBeEmpty: '<一句理由>'` ⇒ 红。治的是「阶段还在、判据没了」：`50-msrv` 写死了
 *     D-191 之前那两个顶层目录，拆成 crate 之后路径不存在，于是每轮产出 0 项、
 *     汇总表上**一行都没有**，与「跑过了、全绿」完全同形，而 CI 那个 msrv job 一直绿——
 *     `rust-version` 重新变回没人验证的一句话。`gate.mjs::runStage` 在真跑那轮有同一条运行时判定
 *     （0 项且无声明 ⇒ 记失败），本条的价值是**不用等一轮七分钟的门禁**才发现。
 *     声明必须是一句理由而不是 `true`：空转要是有名有姓的豁免，不能是一个可按的开关。
 * 登记见 相应批次。
 *
 * 名单仍然是手写的、仍然带注释——本脚本不生成它，只判它有没有覆盖目录事实。
 * 这正是它有意义的原因：**两份真相**（名单 + 目录）各说各话时才红，
 * 若把名单改成目录扫描产物，就再也没有「忘了接线」这回事可检测了。
 *
 * 用法：node scripts/gate-wiring-audit.mjs [--root=<仓库根>] [--strict]
 *      （`--strict` 与其他判定型守卫同参数，本脚本没有 WARNING 级判定，带不带同义）
 * 退出码：0 = 通过；1 = 有失效；2 = 审计范围读不出（名单或目录都取不到）。
 */
// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns GW 门禁接线（测试文件与门禁名单的对账）
// @codes GW-001 GW-002 GW-003 GW-004 GW-005 GW-006

import fs from 'node:fs'
import path from 'node:path'
import { pathToFileURL, fileURLToPath } from 'node:url'
import { green, red } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? path.resolve(rootArg.slice(7)) : path.resolve(scriptDir, '..')

const SKIP_DIRS = new Set(['node_modules', 'target', '.git', 'tmp', '.workbuddy-ai', '.symbio'])

/** 递归收集 `scripts` 下的 `*.test.mjs`，返回相对仓库根、去掉后缀的名字（`gate.d/_shared`） */
function testNamesOf(root) {
  return collect(root, (f) => f.endsWith('.test.mjs')).map((rel) => rel.replace(/\.test\.mjs$/, ''))
}

/**
 * 递归收集 `scripts` 下的 `*.mjs`，返回 `{ rel, name }`（name 去掉 `.mjs`）。
 * GW-004 用它扫「非测试的 `.mjs`」，所以它与 GW-001 共用同一个目录事实来源
 * （`collect` 返回的是**带后缀**的相对路径，剔除测试要在这一层做）。
 */
function scriptFilesOf(root) {
  return collect(root, (f) => f.endsWith('.mjs')).map((rel) => ({
    rel,
    name: rel.replace(/\.mjs$/, ''),
  }))
}

/** 递归收集的统一实现：谓词决定收谁，返回**带后缀**的相对仓库根路径 */
function collect(root, keep) {
  const base = path.join(root, 'scripts')
  if (!fs.existsSync(base)) return null
  const out = []
  const walk = (dir, prefix) => {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      if (ent.isDirectory()) {
        if (!SKIP_DIRS.has(ent.name)) walk(path.join(dir, ent.name), `${prefix}${ent.name}/`)
        continue
      }
      if (!keep(ent.name)) continue
      out.push(`${prefix}${ent.name}`)
    }
  }
  walk(base, '')
  return out.sort()
}

/**
 * 门禁阶段：`scripts/gate.d/` 下非 `_` 前缀的 `.mjs`。它们由 `gate.mjs` **按目录扫描**
 * 接入，因此不进 GW-004 的登记对账（登记它们等于把「按扫描接入」变成「按名单接入」，
 * 那正是 GW-004 想避免的形状）；顺序由 GW-005 判。
 */
function stageFilesOf(root) {
  const dir = path.join(root, 'scripts', 'gate.d')
  if (!fs.existsSync(dir)) return null
  return fs
    .readdirSync(dir)
    .filter((f) => f.endsWith('.mjs') && !f.startsWith('_'))
    .sort()
}

/**
 * 阶段的**自声明元数据**（GW-005 / GW-006 读它）。四条来源各自独立，缺一就判不出真相：
 *  - `regenerates` / `recheck` / `mayBeEmpty`：`export default` 里的字段（动态 import 读，和 `gate.mjs`
 *    加载阶段是同一条路径，不是第二份解析）；
 *  - `calls`：源码里有没有 `autoWork(` / `autoChain(` 的**调用形状**——只认带左括号的，
 *    于是注释里提到原语的名字不影响判定（旧判据 `includes('autoWork')` 恰恰相反，
 * 见文件头 GW-005 那条与 相应批次）。
 *  - `taskCount`：在本机**物化一次 `tasks()`**（与 `gate.mjs --list` 同一条路径），
 *    GW-006 判它是不是 0。物化失败记成 `tasksError`，不当成「0 项」——
 *    「读不出」与「确实没有」是两件事，前者不能伪装成后者的任何一种结论。
 * import 失败不再静默按「没声明」处理：那会让一条真判据悄悄空转，报成 GW-005 失效。
 */
async function stageMetaOf(root, stageList) {
  const out = []
  for (const f of stageList) {
    const abs = path.join(root, 'scripts', 'gate.d', f)
    let regenerates = false
    let recheck = false
    let mayBeEmpty = null
    let taskCount = null
    let tasksError = null
    let isStage = true
    let error = null
    let decl = null
    try {
      decl = (await import(pathToFileURL(abs).href)).default
      isStage = Boolean(decl?.id)
      regenerates = decl?.regenerates === true
      recheck = decl?.recheck === true
      mayBeEmpty = decl?.mayBeEmpty ?? null
    } catch (e) {
      error = (e && e.message ? e.message : String(e)).split('\n')[0]
    }
    if (!error) {
      // 与 `gate.mjs::materialize` 逐字同形的物化：ctx 只给到只读字段，`run` 一旦在
      // 物化期被调用会抛出来（记成 tasksError），所以这里读到的 0 项与真跑那轮的 0 项同源。
      const ctx = {
        repoRoot: root,
        scriptDir: path.join(root, 'scripts'),
        logDir: null,
        ci: true,
        profile: null,
        autoBump: false,
        run: async (o) => {
          throw new Error(`tasks() 在物化期就执行了命令（${o?.label ?? '?'}）`)
        },
        log: () => null,
      }
      try {
        taskCount = Array.from(typeof decl?.tasks === 'function' ? decl.tasks(ctx) : (decl?.tasks ?? [])).length
      } catch (e) {
        tasksError = (e && e.message ? e.message : String(e)).split('\n')[0]
      }
    }
    let calls = false
    try {
      calls = /auto(?:Work|Chain)\(/.test(fs.readFileSync(abs, 'utf8'))
    } catch {
      /* 读不出源码：calls=false，与 import 失败叠成同一条 GW-005 失效 */
    }
    out.push({ file: f, regenerates, recheck, calls, mayBeEmpty, taskCount, tasksError, isStage, error })
  }
  return out
}

/** 读 30-docs 导出的名单；读不出（没导出 / 语法错）即「审计范围读不出」 */
async function listsOf(root) {
  const modPath = path.join(root, 'scripts', 'gate.d', '30-docs.mjs')
  if (!fs.existsSync(modPath)) return null
  const mod = await import(pathToFileURL(modPath).href)
  if (!Array.isArray(mod.GUARDS) || !Array.isArray(mod.TEST_ONLY)) return null
  // `LIBS` / `OUT_OF_GATE` 可选：没导出按空表处理，于是 GW-004 会把它们报成未登记
  // ——**这是刻意的**：想要它们生效就得真的导出来，不能靠「忘了导出」蒙混过关。
  return {
    guards: mod.GUARDS,
    testOnly: mod.TEST_ONLY,
    libs: Array.isArray(mod.LIBS) ? mod.LIBS : [],
    outOfGate: Array.isArray(mod.OUT_OF_GATE) ? mod.OUT_OF_GATE : [],
  }
}

console.log('--- gate-wiring-audit: 门禁接线对账（GW-001…006） ---')

const lists = await listsOf(REPO_ROOT)
const files = testNamesOf(REPO_ROOT)
const scripts = scriptFilesOf(REPO_ROOT)
const stages = stageFilesOf(REPO_ROOT)
if (!lists || !files || !scripts || !stages) {
  console.log(red('[ERROR] GW-002 审计范围读不出：名单或 `scripts/` 目录取不到（' + REPO_ROOT + '）'))
  process.exit(2)
}

const wired = new Set([...lists.guards, ...lists.testOnly])
const unwired = files.filter((n) => !wired.has(n))
const missing = [...lists.guards, ...lists.testOnly].filter((n) => !files.includes(n))
const both = lists.guards.filter((n) => lists.testOnly.includes(n))

// GW-002 扩面：`LIBS` / `OUT_OF_GATE` 指名的**本体**必须在磁盘上。
// 前两张名单指名的是 `.test.mjs`（本体由测试文件的存在性间接保证），这两张指的
// 就是 `.mjs` 本身——指向一个不存在的文件，等于把一份缺失登记成「已覆盖」。
const libBodiesMissing = [...lists.libs, ...lists.outOfGate].filter(
  (n) => !scripts.some((f) => f.name === n),
)
// GW-004：命名空间闭合。阶段文件（`gate.d/` 下非 `_` 前缀）按扫描接入，不参与对账。
const registered = new Set([
  ...lists.guards,
  ...lists.testOnly,
  ...lists.libs,
  ...lists.outOfGate,
  ...stages.map((f) => `gate.d/${f.replace(/\.mjs$/, '')}`),
])
const unregistered = scripts
  .filter((f) => f.name !== 'gate.mjs' || !lists.outOfGate.includes('gate'))
  .filter((f) => !f.rel.endsWith('.test.mjs'))
  .filter((f) => !(f.rel.startsWith('gate.d/') && !f.rel.slice('gate.d/'.length).startsWith('_')))
  .filter((f) => !registered.has(f.name))

// GW-005：阶段顺序。**判据读自声明字段，不读注释措辞**。
// 形状与理由见文件头 GW-005 那条（含它为什么不再是「重生成必须排末尾」）。
const meta = await stageMetaOf(REPO_ROOT, stages)
const unreadable = meta.filter((m) => m.error)
const declaredButSilent = meta.filter((m) => m.regenerates && !m.calls)
const callingButUndeclared = meta.filter((m) => m.calls && !m.regenerates)
const doubleFlagged = meta.filter((m) => m.regenerates && m.recheck)
const regenStages = meta.filter((m) => m.regenerates)
const recheckStages = meta.filter((m) => m.recheck)
// 有生成却没有只读复核 ⇒ 没有人判「整轮跑完树还干净吗」，生成阶段自己那一次末态比对
// 只覆盖到链跑完为止（`30-docs` 的自动修复器排在它后面）。
const noRecheck = regenStages.length > 0 && recheckStages.length === 0
const lastRegen = regenStages[regenStages.length - 1] ?? null
const rechecksBeforeGeneration = lastRegen ? recheckStages.filter((m) => m.file < lastRegen.file) : []

// GW-006：产出 0 项的阶段。`mayBeEmpty` 必须是**一句非空理由**（字符串）——
// 布尔开关会变成「谁都能按、按了就永久隐身」的那种后门。
const hasReason = (m) => typeof m.mayBeEmpty === 'string' && m.mayBeEmpty.trim() !== ''
const unmaterializable = meta.filter((m) => m.tasksError)
// 「不是阶段」单列：`gate.mjs` 只收 `mod.default?.id` 存在的文件，所以一份没写好
// 导出的阶段文件是**隐形文件**——它在阶段目录里、被 GW-004 放行（按扫描接入，不需登记），
// 而扫描不认它。这与「产出 0 项」是两种病，各报各的。
const notStages = meta.filter((m) => !m.isStage && !m.error)
const emptyStages = meta.filter((m) => m.isStage && m.taskCount === 0 && !hasReason(m))
const badMayBeEmpty = meta.filter((m) => m.mayBeEmpty != null && !hasReason(m))
const totalTasks = meta.reduce((n, m) => n + (m.taskCount ?? 0), 0)

let errors = 0
for (const n of unwired) {
  errors++
  console.log(red(`[ERROR] GW-001 scripts/${n}.test.mjs 存在，但不在 30-docs 的 GUARDS / TEST_ONLY 里 ⇒ 门禁从不跑它`))
}
for (const n of new Set(missing)) {
  errors++
  console.log(red(`[ERROR] GW-002 名单指名 scripts/${n}.test.mjs，而磁盘上没有这个文件 ⇒ 那一步跑的是空气`))
}
for (const n of new Set(libBodiesMissing)) {
  errors++
  console.log(red(`[ERROR] GW-002 名单指名 scripts/${n}.mjs，而磁盘上没有这个文件 ⇒ 它「被覆盖」是假的`))
}
for (const n of both) {
  errors++
  console.log(red(`[ERROR] GW-003 \`${n}\` 同时出现在 GUARDS 与 TEST_ONLY ⇒ 回归测试跑两遍，且「是不是守卫」两处说法不一`))
}
for (const f of unregistered) {
  errors++
  console.log(
    red(
      `[ERROR] GW-004 scripts/${f.rel} 既不在 GUARDS / TEST_ONLY，也不在 LIBS / OUT_OF_GATE ⇒ ` +
        `它既没有门禁步骤、也不在任何名单里，是一份隐形脚本`,
    ),
  )
}
for (const m of unreadable) {
  errors++
  console.log(
    red(
      `[ERROR] GW-005 scripts/gate.d/${m.file} 导入失败（${m.error}）⇒ 它的 \`regenerates\` / \`recheck\` 声明读不出来，这条次序判定不可信`,
    ),
  )
}
for (const m of declaredButSilent) {
  errors++
  console.log(
    red(
      `[ERROR] GW-005 scripts/gate.d/${m.file} 声明 \`regenerates: true\`，源码里却没有 \`autoWork(\` / \`autoChain(\` 的调用 ⇒ 声明是假的：生成器已被删掉就该撤声明，` +
        `自己写文件绕过原语则末态判差异就此失效`,
    ),
  )
}
for (const m of callingButUndeclared) {
  errors++
  console.log(
    red(
      `[ERROR] GW-005 scripts/gate.d/${m.file} 调用 \`autoWork\` / \`autoChain\`（会改写并暂存受版本控制文件），却没有 \`regenerates: true\` 声明 ⇒ ` +
        `GW-005 看不见它，「生成先于只读复核」这条次序没人守`,
    ),
  )
}
for (const m of doubleFlagged) {
  errors++
  console.log(
    red(`[ERROR] GW-005 scripts/gate.d/${m.file} 同时声明 \`regenerates\` 与 \`recheck\` ⇒ 一个阶段既是生成又是复核，两条判据各自成立、合起来什么都判不了`),
  )
}
if (noRecheck) {
  errors++
  console.log(
    red(
      `[ERROR] GW-005 ${regenStages.length} 个阶段声明 \`regenerates\`，却没有一个阶段声明 \`recheck\` ⇒ 没有只读执行方判「整轮跑完生成物仍然一致」（生成阶段自己只比到链跑完为止）`,
    ),
  )
}
for (const m of rechecksBeforeGeneration) {
  errors++
  console.log(
    red(
      `[ERROR] GW-005 scripts/gate.d/${m.file}（\`recheck\`）排在生成阶段 scripts/gate.d/${lastRegen.file} 之前 ⇒ 它复核的是上一版投影，恒真；生成之后没人再对末态`,
    ),
  )
}

// GW-006：阶段**本轮产出 0 项**。与 `gate.mjs::runStage` 的运行时判定同一条判据——
// 这里能在 1s 内看到，而不是等一整轮门禁（而且真跑那轮它只会「没有一行」）。
for (const m of unmaterializable) {
  errors++
  console.log(
    red(
      `[ERROR] GW-006 scripts/gate.d/${m.file} 的 tasks() 物化失败（${m.tasksError}）⇒ 无法判定它产出几项，这条判据不可信`,
    ),
  )
}
for (const m of notStages) {
  errors++
  console.log(
    red(
      `[ERROR] GW-006 scripts/gate.d/${m.file} 没有 \`export default { id, tasks }\` ⇒ ` +
        `它躺在阶段目录里却不是一个阶段：\`gate.mjs\` 按 \`mod.default?.id\` 收文件，收不到就跳过，而 GW-004 已经因为它在 \`gate.d/\` 下放行了登记`,
    ),
  )
}
for (const m of emptyStages) {
  errors++
  console.log(
    red(
      `[ERROR] GW-006 scripts/gate.d/${m.file} 产出 0 个检查项，且没有 \`mayBeEmpty\` 声明 ⇒ 这一阶段本轮**什么都没判**，` +
        `而汇总表上它与「全绿」无法区分（派生路径失效时就是这样：先查它的 tasks 从哪些路径/名单算出来）`,
    ),
  )
}
for (const m of badMayBeEmpty) {
  errors++
  console.log(
    red(
      `[ERROR] GW-006 scripts/gate.d/${m.file} 声明了 \`mayBeEmpty\`（${JSON.stringify(m.mayBeEmpty)}），但它不是一句理由 ⇒ ` +
        '空转必须是有名有姓的豁免；`mayBeEmpty: true` 那种开关，三个月后没人记得为什么允许',
    ),
  )
}

if (errors) {
  console.log(
    `gate-wiring-audit 未通过：${errors} 处接线失效（磁盘 ${files.length} 份测试 / ` +
      `${scripts.length - files.length} 份脚本 / 名单 ${registered.size} 项 / 阶段 ${stages.length} 个）`,
  )
  process.exit(1)
}
console.log(
  green(
    `gate-wiring-audit 通过：磁盘 ${files.length} 份测试全部接线，` +
      `名单 ${lists.guards.length} + ${lists.testOnly.length} + ${lists.libs.length} + ${lists.outOfGate.length} 项逐一对应；` +
      `${scripts.length - files.length} 份脚本全部登记；生成阶段 ${lastRegen ? lastRegen.file : '（无）'} 先于只读复核 ${recheckStages.map((m) => m.file).join(' / ') || '（无）'}；` +
      `${stages.length} 个阶段物化出 ${totalTasks} 项，无空转阶段`,
  ),
)
