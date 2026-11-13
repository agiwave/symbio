#!/usr/bin/env node
/**
 * gate-wiring-audit — 门禁接线对账（GW-001…GW-008）
 *
 * 治的病：**脚本存在 ≠ 会被门禁跑**。
 *
 * 接线由每个脚本自己声明：头注释里一行 `// @gate guard|report|test|lib|off`
 * （读法只有一处：`gate.d/_shared.mjs::gateDeclarations`），`30-docs` 扫声明分组执行。
 * 自述的好处是加一条守卫不用改任何名单；代价是「声明了 ≠ 跑了」——本脚本就是那个对账方。
 *
 * 判据（声明、目录、任务表三方互为对账）：
 *   - **GW-001** `scripts/**` 下每份非测试、非阶段脚本**有且只有一条** `@gate` 声明，
 *     且值在闭集内。缺声明 = 隐形脚本（谁也不会跑它）；拼错 = 落进 `off` 那一桶，
 *     同样静默不跑；两条声明 = 「到底算哪一类」没有答案。
 *   - **GW-002** 声明 `guard` / `report` / `test` 的必须有同名 `.test.mjs`。守卫没有回归
 *     测试就只是一个「会亮绿灯的脚本」——它腐烂的方式恰恰是规则写错所以永不命中，
 *     而这件事只有注入违规才能发现。`lib` / `off` 不要求。
 *   - **GW-003** 每条 `guard` 声明必须真的出现在 `30-docs` 物化出的 `--strict` 任务里。
 *     这一条守的是本机制唯一的坏法：**枚举写窄**（漏扫一层子目录、正则只认 `-audit`、
 *     跳过规则误伤）时，声明着的守卫会静默不跑，而门禁表上一切正常。
 *   - **GW-004** 每份 `.test.mjs` 必须真的有 `--test` 任务（GW-003 的反方向）。
 *     实证：`windows-restart` 两份测试躺在仓里从没被跑过，其中一份早就红了
 *     （`prepare` 用 realpath 建 stage，测试拿未归一化的临时目录名比前缀）。
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
 *   - **GW-007**（目录即分组）名字以 `-audit` 结尾的脚本（含它的 `.test.mjs`）必须住在
 *     `scripts/audits/` 下。目录分层是给人用的检索面，但它**同时**是 `30-docs` 枚举的范围
 *     （`scripts/**`）：留在根目录不会被漏跑，于是没有任何东西会提醒「这份没搬家」——
 *     实测就是这么留下四份的（`baseline-scope` / `prompt-shape` / `schema` /
 *     `session-dead-code`），而 CONTRIBUTING 里那句「静态审计脚本都在 `scripts/audits/`」
 *     当时已经是一句假话。判据只做单向（`-audit` ⇒ 在 `audits/`）：`projection-purity` 这类
 *     不带 `-audit` 的守卫照样要在里面，反向规则会把名字变成判据的输入。
 *   - **GW-008**（生成链的声明卫生与对账）`// @gens <序数> write|check` 是生成器给
 *     `06-facts-write`（生成）与 `60-facts`（复核）的**同一份**声明：两个阶段都遍历它、
 *     按序数排序执行，谁都不点名生成器。所以「加了生成器却没进链」只能在本条判——
 *     序数撞车 / 模式名拼错 / 声明了却不在物化出的任务里（枚举写窄的同一形状）。
 *     反向也判：某个阶段跑了它却没声明，那一步就不在机制的账上。
 *
 * 用法：node scripts/audits/gate-wiring-audit.mjs [--root=<仓库根>] [--strict]
 *      （`--strict` 与其他判定型守卫同参数，本脚本没有 WARNING 级判定，带不带同义）
 * 退出码：0 = 通过；1 = 有失效；2 = 审计范围读不出（声明表或目录都取不到）。
 */
// @gate guard
// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns GW 门禁接线（脚本自声明与门禁执行的对账）
// @codes GW-001 GW-002 GW-003 GW-004 GW-005 GW-006 GW-007 GW-008

import fs from 'node:fs'
import path from 'node:path'
import { pathToFileURL, fileURLToPath } from 'node:url'
import { green, red } from '../color.mjs'
import { gateDeclarations, gensDeclarations, scriptFilesOf } from '../gate.d/_shared.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const REPO_ROOT = rootArg ? path.resolve(rootArg.slice(7)) : path.resolve(scriptDir, '..', '..')

/**
 * 门禁阶段：`scripts/gate.d/` 下非 `_` 前缀的 `.mjs`。它们由 `gate.mjs` **按目录扫描**
 * 接入（`export default { id }` 就是接线），因此不写 `@gate` 声明；顺序由 GW-005 判。
 */
function stageFilesOf(root) {
  const dir = path.join(root, 'scripts', 'gate.d')
  if (!fs.existsSync(dir)) return null
  return fs
    .readdirSync(dir)
    .filter((f) => f.endsWith('.mjs') && !f.startsWith('_'))
    .sort()
}

/** `scripts/` 下全部 `.test.mjs` 的相对名（`gate.d/_shared`） */
function testNamesOf(root) {
  const files = scriptFilesOf(root)
  if (files === null) return null
  return files.filter((f) => f.endsWith('.test.mjs')).map((f) => f.replace(/\.test\.mjs$/, ''))
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

/**
 * 物化一个阶段的任务表。读不出返回 `{ error }`，不当成「0 项」。
 * `root` 是仓库根：夹具可以只放它自己那份阶段文件。
 */
async function stageTasksOf(root, file) {
  const modPath = path.join(root, 'scripts', 'gate.d', file)
  if (!fs.existsSync(modPath)) return { missing: true }
  let mod
  try {
    mod = await import(pathToFileURL(modPath).href)
  } catch (e) {
    return { error: (e && e.message ? e.message : String(e)).split('\n')[0] }
  }
  if (typeof mod.default?.tasks !== 'function') return { error: `${file} 没有导出 default.tasks` }
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
    // `when` 在物化期不求值，给个能应答的范围只是为了让未来若提前求值仍可读。
    scope: { hits: () => true },
  }
  try {
    return { mod, tasks: Array.from(mod.default.tasks(ctx)) }
  } catch (e) {
    return { mod, error: (e && e.message ? e.message : String(e)).split('\n')[0] }
  }
}

/**
 * 物化 `30-docs` 的任务表 —— GW-003 / GW-004 拿它判「声明的东西真的被跑到没有」。
 * 与 `stageTasksOf` 同源，只是调用点多一层壳（保住既有调用形状）。
 */
async function docsTasksOf(root) {
  const r = await stageTasksOf(root, '30-docs.mjs')
  return r.missing ? { error: 'scripts/gate.d/30-docs.mjs 不存在' } : r
}


console.log('--- gate-wiring-audit: 门禁接线对账（GW-001…008） ---')

const decl = gateDeclarations(REPO_ROOT)
const files = testNamesOf(REPO_ROOT)
const scripts = scriptFilesOf(REPO_ROOT)
const stages = stageFilesOf(REPO_ROOT)
if (!decl || !files || !scripts || !stages) {
  console.log(red('[ERROR] GW-001 审计范围读不出：`scripts/` 目录或声明表取不到（' + REPO_ROOT + '）'))
  process.exit(2)
}

// GW-001：声明齐全、唯一、值在闭集内。
const undeclared = decl.problems.filter((p) => p.error === 'missing')
const duplicateDecl = decl.problems.filter((p) => p.error === 'duplicate')
const unknownDecl = decl.problems.filter((p) => p.error === 'unknown')
const unreadableScript = decl.problems.filter((p) => p.error === 'unreadable')

// GW-002：需要回归测试的三个桶。
const needsTest = decl.entries.filter((e) => ['guard', 'report', 'test'].includes(e.bucket))
const noTest = needsTest.filter((e) => !files.includes(e.name))

// GW-003 / GW-004：声明与目录事实**真的被 30-docs 跑到**。
const docs = await docsTasksOf(REPO_ROOT)
const relOf = (abs) =>
  path
    .relative(path.join(REPO_ROOT, 'scripts'), abs)
    .split(path.sep)
    .join('/')
    .replace(/\.mjs$/, '')
const strictRun = new Set()
const testedRun = new Set()
for (const t of docs.tasks ?? []) {
  const a = t?.args ?? []
  if (a.includes('--strict') && a[0]) strictRun.add(relOf(a[0]))
  if (a[0] === '--test' && a[1]) testedRun.add(relOf(a[1]).replace(/\.test$/, ''))
}
const guards = decl.entries.filter((e) => e.bucket === 'guard')
const notRunAsGuard = docs.tasks ? guards.filter((e) => !strictRun.has(e.name)) : []
const notRunAsTest = docs.tasks ? files.filter((n) => !testedRun.has(n)) : []

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
// 导出的阶段文件是**隐形文件**——它在阶段目录里、被 GW-001 放行（按扫描接入，不写声明），
// 而扫描不认它。这与「产出 0 项」是两种病，各报各的。
const notStages = meta.filter((m) => !m.isStage && !m.error)
const emptyStages = meta.filter((m) => m.isStage && m.taskCount === 0 && !hasReason(m))
const badMayBeEmpty = meta.filter((m) => m.mayBeEmpty != null && !hasReason(m))
const totalTasks = meta.reduce((n, m) => n + (m.taskCount ?? 0), 0)
const countOf = (b) => decl.entries.filter((e) => e.bucket === b).length

// GW-007：`-audit` 命名的脚本必须住在 `scripts/audits/`（含它的 `.test.mjs`）。
// 单向：`audits/` 里可以有别的名字（`projection-purity`），因为「名字」不是判据的输入。
const strays = scripts.filter((rel) => /(^|\/)[^/]*-audit(\.test)?\.mjs$/.test(rel) && !rel.startsWith('audits/'))

// GW-008：`@gens` 声明 ↔ 生成链 / 复核阶段物化出的**任务**。
// 两个阶段都遍历同一份声明，所以「声明了却没被跑到」只能在**物化结果**上判——
// 与 GW-003 同一条道理：枚举写窄（模式名拼错、过滤条件写错）时声明还在，跑的是空集。
const gens = gensDeclarations(REPO_ROOT) ?? { entries: [], problems: [] }
const gensBadShape = gens.problems
const gensWriterDecl = gens.entries.filter((e) => e.modes.includes('write'))
const gensCheckerDecl = gens.entries.filter((e) => e.modes.includes('check'))
const writeRun = await stageTasksOf(REPO_ROOT, '06-facts-write.mjs')
const checkRun = await stageTasksOf(REPO_ROOT, '60-facts.mjs')
// 06 把整条链合成**一格** `autoChain` 工作，步骤自述在 `STEPS`；60 每份生成器一格。
const stepsRan = Array.isArray(writeRun.mod?.STEPS)
  ? writeRun.mod.STEPS.map((s) => (s?.args?.[0] ? relOf(s.args[0]) : null)).filter(Boolean)
  : []
const checksRan = (checkRun.tasks ?? []).map((t) => (t?.args?.[0] ? relOf(t.args[0]) : null)).filter(Boolean)
const gensErrors = [
  ...(writeRun.error ? [{ where: '06-facts-write', detail: writeRun.error }] : []),
  ...(checkRun.error ? [{ where: '60-facts', detail: checkRun.error }] : []),
  // 「整条链一条声明都没有」不在这里判：两个阶段自己会红（`writers.length === 0` 时
  // `06-facts-write` 在模块加载期就抛，`60-facts` 在物化期抛）。这条判据只管
  // **声明与物化结果对不上**——那才是「加了生成器却没进链」的形状。
]
const writeNotRun = writeRun.mod ? gensWriterDecl.filter((e) => !stepsRan.includes(e.name)) : []
const checkNotRun = checkRun.tasks ? gensCheckerDecl.filter((e) => !checksRan.includes(e.name)) : []
const runNotDeclared = [
  ...stepsRan.filter((n) => !gensWriterDecl.some((e) => e.name === n)).map((n) => ({ n, where: '06-facts-write' })),
  ...checksRan.filter((n) => !gensCheckerDecl.some((e) => e.name === n)).map((n) => ({ n, where: '60-facts' })),
]

let errors = 0
for (const p of undeclared) {
  errors++
  console.log(
    red(
      `[ERROR] GW-001 scripts/${p.rel} 没有 \`// @gate guard|report|test|lib|off\` 声明 ⇒ ` +
        `它不会被任何阶段跑到，是一份隐形脚本（加一行声明，或者把它删掉）`,
    ),
  )
}
for (const p of duplicateDecl) {
  errors++
  console.log(
    red(`[ERROR] GW-001 scripts/${p.rel} 有两条以上 \`@gate\` 声明（${p.detail}）⇒ 「到底算哪一类」没有答案`),
  )
}
for (const p of unknownDecl) {
  errors++
  console.log(
    red(
      `[ERROR] GW-001 scripts/${p.rel} 的 \`@gate ${p.detail}\` 不在闭集内（guard / report / test / lib / off）⇒ ` +
        `拼错的声明会落进「不跑」那一桶，静默消失`,
    ),
  )
}
for (const p of unreadableScript) {
  errors++
  console.log(red(`[ERROR] GW-001 scripts/${p.rel} 读不出（${p.detail}）⇒ 声明无法核验`))
}
for (const e of noTest) {
  errors++
  console.log(
    red(
      `[ERROR] GW-002 scripts/${e.name}.mjs 声明 \`@gate ${e.bucket}\`，却没有 scripts/${e.name}.test.mjs ⇒ ` +
        `${e.bucket === 'guard' ? '没有回归测试的守卫只是一个会亮绿灯的脚本' : '这一步跑的是空气'}`,
    ),
  )
}
if (docs.error) {
  errors++
  console.log(red(`[ERROR] GW-003 / GW-004 30-docs 的任务表读不出（${docs.error}）⇒ 无法判「声明的东西有没有被跑到」`))
}
for (const e of notRunAsGuard) {
  errors++
  console.log(
    red(
      `[ERROR] GW-003 scripts/${e.name}.mjs 声明 \`@gate guard\`，但 30-docs 没有产出它的 \`--strict\` 任务 ⇒ ` +
        `守卫声明着却从不跑（查枚举：漏扫目录 / 跳过规则 / 桶名拼错）`,
    ),
  )
}
for (const n of notRunAsTest) {
  errors++
  console.log(
    red(`[ERROR] GW-004 scripts/${n}.test.mjs 存在，但 30-docs 没有产出 \`--test\` 任务 ⇒ 门禁从不跑它`),
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
        `它躺在阶段目录里却不是一个阶段：\`gate.mjs\` 按 \`mod.default?.id\` 收文件，收不到就跳过，而 GW-001 已经因为它在 \`gate.d/\` 下放行了声明`,
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

for (const n of strays) {
  errors++
  console.log(
    red(
      `[ERROR] GW-007 scripts/${n} 名字以 \`-audit\` 结尾却不在 \`scripts/audits/\` 下 ⇒ ` +
        `「审计脚本都住在那个目录」这句话就是假的（它照样会被 30-docs 跑到，所以没有任何东西会提醒你搬家）`,
    ),
  )
}

for (const p of gensBadShape) {
  errors++
  console.log(
    red(
      `[ERROR] GW-008 scripts/${p.rel} 的 \`@gens\` 声明不合格（${p.error}：${p.detail}）⇒ ` +
        `序数不唯一 / 模式拼错都会让这一步**落进链外**，而两个事实阶段都还是绿的`,
    ),
  )
}
for (const g of gensErrors) {
  errors++
  console.log(red(`[ERROR] GW-008 ${g.where}：${g.detail}`))
}
for (const e of writeNotRun) {
  errors++
  console.log(
    red(
      `[ERROR] GW-008 scripts/${e.name}.mjs 声明了 \`@gens ${e.order} write\`，但 06-facts-write 的链里没有它 ⇒ ` +
        `生成器永不执行（两个事实阶段都不会红：它们遍历的就是同一份声明，过滤条件写错也照样绿）`,
    ),
  )
}
for (const e of checkNotRun) {
  errors++
  console.log(
    red(`[ERROR] GW-008 scripts/${e.name}.mjs 声明了 \`@gens ${e.order} check\`，但 60-facts 没有产出它的复核任务 ⇒ 末态漂移无人判`),
  )
}
for (const u of runNotDeclared) {
  errors++
  console.log(
    red(
      `[ERROR] GW-008 ${u.where} 跑了 scripts/${u.n}.mjs，它却没有对应的 \`@gens\` 声明 ⇒ ` +
        `这一步不在机制的账上（下一个读声明的人看不到它）`,
    ),
  )
}

if (errors) {
  console.log(
    `gate-wiring-audit 未通过：${errors} 处接线失效（${decl.entries.length} 份声明 / ` +
      `${files.length} 份测试 / ${scripts.length} 份脚本 / 阶段 ${stages.length} 个）`,
  )
  process.exit(1)
}
console.log(
  green(
    `gate-wiring-audit 通过：${decl.entries.length} 份脚本全部声明（guard ${countOf('guard')} / report ${countOf('report')} / ` +
      `test ${countOf('test')} / lib ${countOf('lib')} / off ${countOf('off')}），${files.length} 份测试与 ` +
      `${guards.length} 条守卫全部被 30-docs 跑到；生成链 ${gensWriterDecl.length} 步与复核 ${gensCheckerDecl.length} 步` +
      `（@gens 声明 ↔ 物化任务双向对账）；生成阶段 ${lastRegen ? lastRegen.file : '（无）'} 先于只读复核 ` +
      `${recheckStages.map((m) => m.file).join(' / ') || '（无）'}；${stages.length} 个阶段物化出 ${totalTasks} 项，无空转阶段`,
  ),
)
