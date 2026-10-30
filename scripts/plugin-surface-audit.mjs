#!/usr/bin/env node
/**
 * plugin-surface-audit — **扩展层插件只认一个宿主门面**（HS-001 / HS-002，D-253）
 *
 * ## 它治的病
 *
 * `symbio_kernel` / `symbio_core` / `symbio_plugin_api` 三个 crate 拆完之后，每个插件
 * 仍在 manifest 里列三到四行内部依赖、源码里混写三种前缀。「插件依赖几个架构模块」
 * 于是由**某段代码当初顺手写了哪个前缀**决定，而不是由边界决定——cargo 依赖图表达不了
 * 任何意图，评审也看不出某次改动是不是在扩大插件的可见面。
 *
 * 裁决见 DE-023 D-253：扩展层插件的唯一宿主依赖是 `symbio-plugin-api`，门面里是一份
 * **白名单**（每条都能报出消费方）。本脚本判两件事：
 *
 * | 编号 | 判什么 | 红了怎么办 |
 * |---|---|---|
 * | HS-001 | 非控制面插件的源码（注释外）或 manifest 里出现 `symbio_core` / `symbio-kernel` / `symbio-deploy` | 从门面取那个符号；门面里没有 ⇒ 说明它还不该给插件看，先论证再加白名单 |
 * | HS-002 | 门面白名单条数超过基线（只许降） | 加一条要配一条决策理由，并同步上调基线 |
 * | HS-003 | 门面白名单里有符号在**扩展层插件代码里零引用** | 删掉它——白名单的每一条都要报得出消费方（与 C-003 同一把尺子）；报不出的就是空转条目 |
 * | HS-004 | 插件胶水层 `symbio-providers` 把**嵌入臂**设成默认开启（`default = ["embedding"]` 或依赖不是 `optional`） | 那一条 `default = []` 是有意的：`ort` / `tokenizers` 带来 55 个传递依赖（168 → 113），而插件侧只有 `local` 一个消费者。要开就在宿主与需要它的插件那里显式开（D-254） |
 *
 * ## 为什么按插件身份分档，而不是一刀切
 *
 * `loop` / `session` 是**控制面**（DE-026 的影响类）：它们按轮的顺序运行、认识幕与
 * 容器概念，本来就该直接依赖 core。把它们也压进门面，只会逼出一批「端口形状不说容器词」
 * 的假豁免——EX-008 的边界正是这条（D-253 候选 b 已否）。
 *
 * ## 失效形态
 *
 * ① **剥注释写漏** ⇒ 文档里那句「见 `symbio_core::vdfs::address`」被判红，天天误报的
 * 守卫最后只会被豁免喂死（复用 `extension-vocab-audit` 的 `stripRustCode`，只认代码）；
 * ② **控制面名单写成通配**（`symbio-plugin-*`）⇒ 判据变成空转，谁都能进来；
 * ③ **白名单基线写成大数** ⇒ 只进不出，机制退化成装饰。三者各由回归测试钉住。
 *
 * 用法：node scripts/plugin-surface-audit.mjs [--root=<目录>]
 * 退出码：0 = 通过；1 = 有违规；2 = 扫描范围读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns HS 扩展层插件宿主面
// @codes HS-001 HS-002 HS-003 HS-004

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, yellow } from './color.mjs'
import { stripRustCode } from './extension-vocab-audit.mjs'

/**
 * 控制面：按 DE-026 的影响类运行，认识宿主的容器概念，允许直接依赖 core / kernel。
 * **逐个名字写死**，不写通配——一份通配的白名单就是零张门。
 */
export const CONTROL_PLANE = ['symbio-plugin-api', 'symbio-plugin-loop', 'symbio-plugin-session']

/** 扩展层插件不得直接依赖的三个 crate（manifest 键名）。 */
export const FORBIDDEN_MANIFEST = ['symbio-core', 'symbio-kernel', 'symbio-sdk', 'symbio-deploy']

/** 源码里不得出现的三个前缀。 */
export const FORBIDDEN_PREFIXES = ['symbio_core::', 'symbio_kernel::', 'symbio_sdk::', 'symbio_deploy::']

/**
 * 机制臂条数基线：**只降不升**（HS-002）。实测低于它时打提示要求同步下调。
 *
 * 只数机制臂。门面按来源分三臂（D-255 / D-258）：语法臂来自 `symbio-kernel`（直接依赖
 * 6 个，插件取共享类型不增加任何机制面）、SDK 臂来自 `symbio-sdk`（机制实现，但不认
 * 容器概念，同样不计债）、机制臂来自 `symbio-core`（每条都要控制面支撑）。
 * 把前两臂计进基线，会让「欠账还剩多少」被两个与债无关的数字稀释——248 听起来离 0
 * 很远，133 才是真实的待还量。
 *
 * 115 → 72（D-258 第一批：`text` / `clock` / `logger` / `event_bus` / `creator` /
 * `vdfs` / `keys` 七个域搬进 `symbio-sdk`）→ 25（第二批：`chat` / `capability` /
 * `adapters` / `llm` / `plugin` 五个域；另把 `chat` 的三个子命名空间与内核语法
 * `ring` 各归其臂）。这些条**不是还清了存量**，是搬了家：它们一条不少地留在门面上，
 * 只是改从 `symbio_sdk` / `symbio_kernel` 取（HS-003 照旧逐条核消费方）。
 */
export const WHITELIST_BASELINE = 25

/** 门面白名单所在的文件与块的起止标记。 */
export const FACADE = 'crates/symbio-plugin-api/src/lib.rs'
/**
 * 插件胶水层 `symbio-providers` 的**重臂**：嵌入推理（`ort` + `tokenizers`）。
 * 它默认必须是关的（D-254）——插件要的是 `DefaultToolVisitor` / `MemoryFile` 这些胶水，
 * 不是一套 ONNX 运行时。判据只看 manifest：`optional = true` + `default` 不含它。
 */
export const HEAVY_ARM = {
  manifest: 'crates/symbio-providers/Cargo.toml',
  dep: 'symbio-provider-embedding',
  feature: 'embedding',
}

/** 返回 `symbio-providers` manifest 里重臂的违规项（文件不存在 ⇒ 无此风险，返回空）。 */
export function heavyArmViolations(toml) {
  const errors = []
  const depLine = toml.split(/\r?\n/).find((l) => new RegExp(`^${HEAVY_ARM.dep}\\s*=`).test(l.trim()))
  if (!depLine || !/optional\s*=\s*true/.test(depLine)) {
    errors.push(
      `${HEAVY_ARM.manifest}  里 \`${HEAVY_ARM.dep}\` 不是 \`optional = true\`——` +
        `嵌入臂必须可选（HS-004，D-254）：插件不该为一个胶水符号链接整套 ONNX 运行时`,
    )
  }
  const idx = toml.indexOf('[features]')
  const rest = idx >= 0 ? toml.slice(idx + '[features]'.length) : ''
  const next = rest.search(/^\[/m)
  const fb = next >= 0 ? rest.slice(0, next) : rest
  const dm = /^\s*default\s*=\s*\[([^\]]*)\]/m.exec(fb)
  if (dm && dm[1].split(',').some((s) => s.trim().replace(/["']/g, '') === HEAVY_ARM.feature)) {
    errors.push(
      `${HEAVY_ARM.manifest}  的 \`[features] default\` 含 \`${HEAVY_ARM.feature}\`——` +
        'HS-004 / D-254：默认关，宿主与需要它的插件各自显式开；默认开等于把这条裁决作废',
    )
  }
  return errors
}

const BLOCK_START = 'pub use symbio_core::{'
const BLOCK_END = '};'
/** 语法臂：可能有多块（按 kernel 子模块分组），因此按正则全量取。 */
const GRAMMAR_RE = /pub use symbio_kernel(?:::[A-Za-z0-9_:]+)?::\{([^}]*)\}/g
/**
 * 单名一行的写法（`pub use symbio_kernel::view::Budget;` / `pub use symbio_sdk::clock::clock_now_ms;`）。
 *
 * 花括号组与单名一行是同一张名单的两种排版，**判据必须两种都读到**：
 * 只认花括号组时，一条单名重导出让 HS-003 的「每条都要报得出消费方」整条绕过去——
 * 那正是最容易藏匿空转条目的形状（一行、无逗号、无人注意）。
 */
const GRAMMAR_SINGLE_RE = /pub use symbio_kernel(?:::[A-Za-z0-9_]+)*::([A-Za-z0-9_]+);/g
/**
 * SDK 臂：同样按模块分组、可能有多块（D-258）。它**不计债**——这些机制只依赖内核，
 * 插件取它们不增加「要认识整个宿主」的代价——但它是门面对插件的承诺，所以和语法臂
 * 一样受 HS-003 管（每一条都要报得出消费方）。
 */
const SDK_RE = /pub use symbio_sdk(?:::[A-Za-z0-9_:]+)?::\{([^}]*)\}/g
const SDK_SINGLE_RE = /pub use symbio_sdk(?:::[A-Za-z0-9_]+)*::([A-Za-z0-9_]+);/g

/** 把 `a, b as c` 这类清单切成符号名（取 `as` 之后的别名）。 */
function splitSymbols(body) {
  return body
    .replace(/\n/g, ' ')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => s.split(/\s+as\s+/).pop().trim())
}

/** 读出门面白名单的符号清单（块内逗号分隔）。 */
export function readWhitelist(root) {
  const p = path.join(root, FACADE)
  if (!fs.existsSync(p)) return null
  const src = fs.readFileSync(p, 'utf8')
  const i = src.indexOf(BLOCK_START)
  if (i < 0) return null
  const j = src.indexOf(BLOCK_END, i)
  if (j < 0) return null
  return splitSymbols(src.slice(i + BLOCK_START.length, j))
}

/** 读出门面语法臂的符号清单（`symbio_kernel` 各分块的并集）。 */
export function readGrammar(root) {
  const p = path.join(root, FACADE)
  if (!fs.existsSync(p)) return null
  const src = fs.readFileSync(p, 'utf8')
  const out = []
  for (const m of src.matchAll(GRAMMAR_RE)) out.push(...splitSymbols(m[1]))
  for (const m of src.matchAll(GRAMMAR_SINGLE_RE)) out.push(m[1])
  return out.length ? out : null
}

/** 读出门面 SDK 臂的符号清单（`symbio_sdk` 各分块的并集）。 */
export function readSdkArm(root) {
  const p = path.join(root, FACADE)
  if (!fs.existsSync(p)) return null
  const src = fs.readFileSync(p, 'utf8')
  const out = []
  for (const m of src.matchAll(SDK_RE)) out.push(...splitSymbols(m[1]))
  for (const m of src.matchAll(SDK_SINGLE_RE)) out.push(m[1])
  return out.length ? out : null
}

/** manifest 的 `[dependencies]` 段里出现的禁用键（只认段内，dev-dependencies 不判）。 */
export function manifestViolations(toml) {
  const hits = []
  let inDeps = false
  for (const line of toml.split(/\r?\n/)) {
    const t = line.trim()
    if (/^\[.*\]$/.test(t)) {
      inDeps = t === '[dependencies]'
      continue
    }
    if (!inDeps) continue
    const m = /^([A-Za-z0-9_-]+)\s*=/.exec(t)
    if (m && FORBIDDEN_MANIFEST.includes(m[1])) hits.push(m[1])
  }
  return hits
}

/**
 * 扩展层插件真正引用到的门面符号（剥注释后）：`symbio_plugin_api::X` 与
 * `symbio_plugin_api::{A, B}` 两种形态。测试文件也算消费方——这里只判「这条出口有没有
 * 人用」，不判「只有测试在用」（后者归 C-003 的三档口径，不在这里复判）。
 */
export function collectConsumers(root, pluginNames) {
  const used = new Set()
  for (const name of pluginNames) {
    const srcDir = path.join(root, 'crates', name, 'src')
    if (!fs.existsSync(srcDir)) continue
    for (const f of walkRs(srcDir)) {
      const code = stripRustCode(fs.readFileSync(f, 'utf8'))
      for (const m of code.matchAll(/symbio_plugin_api::\{([^}]*)\}/g)) {
        for (const raw of m[1].split(',')) {
          const s = raw.trim().split(/\s+as\s+/).pop().trim()
          if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(s)) used.add(s)
        }
      }
      for (const m of code.matchAll(/symbio_plugin_api::([A-Za-z_][A-Za-z0-9_]*)/g)) used.add(m[1])
    }
  }
  return used
}

function walkRs(dir, out = []) {  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walkRs(p, out)
    else if (e.name.endsWith('.rs')) out.push(p)
  }
  return out
}

export function run(root) {
  const cratesDir = path.join(root, 'crates')
  if (!fs.existsSync(cratesDir)) {
    console.log(`${red('[ERROR]')} plugin-surface-audit：范围读不出——缺少 crates/`)
    return 2
  }

  const plugins = fs
    .readdirSync(cratesDir, { withFileTypes: true })
    .filter((e) => e.isDirectory() && e.name.startsWith('symbio-plugin-'))
    .map((e) => e.name)
    .sort()
  if (plugins.length === 0) {
    console.log(`${red('[ERROR]')} plugin-surface-audit：crates/ 下一个 symbio-plugin-* 都没有`)
    return 2
  }

  const errors = []
  let scannedFiles = 0
  let scannedPlugins = 0

  for (const name of plugins) {
    if (CONTROL_PLANE.includes(name)) continue
    scannedPlugins++
    const dir = path.join(cratesDir, name)

    const tomlP = path.join(dir, 'Cargo.toml')
    if (fs.existsSync(tomlP)) {
      for (const dep of manifestViolations(fs.readFileSync(tomlP, 'utf8'))) {
        errors.push(
          `crates/${name}/Cargo.toml  [dependencies] 里有 \`${dep}\`——扩展层插件只依赖 \`symbio-plugin-api\`（D-253）；` +
            '门面里没有要的符号时，先论证「该不该给插件看」，再加白名单',
        )
      }
    }

    const srcDir = path.join(dir, 'src')
    if (!fs.existsSync(srcDir)) continue
    for (const f of walkRs(srcDir)) {
      // 测试面不判：`*.test.rs` 是插件的**内部验证**，不是它对外承诺的依赖面。
      // 容器测试要扮演一次「轮次派发」才能验证路由转发，那批符号属于控制面的容器
      // 概念（`TurnSpec` / `TurnDispatch` / …），门面白名单里没有也不该有（D-253）。
      // 这与 EX-008 对 `*.test.rs` 不判是同一条纪律：测试可以把宿主词汇摆上桌。
      if (f.endsWith('.test.rs')) continue
      scannedFiles++
      const code = stripRustCode(fs.readFileSync(f, 'utf8'))
      const lines = code.split('\n')
      for (const [idx, line] of lines.entries()) {
        for (const pfx of FORBIDDEN_PREFIXES) {
          if (!line.includes(pfx)) continue
          errors.push(
            `${path.relative(root, f).split(path.sep).join('/')}:${idx + 1}  \`${pfx}\`——` +
              '扩展层插件只经 `symbio_plugin_api::…` 取宿主面（D-253，HS-001）',
          )
        }
      }
    }
  }

  const wl = readWhitelist(root)
  if (wl === null) {
    console.log(`${red('[ERROR]')} plugin-surface-audit：读不出门面白名单（${FACADE}）`)
    return 1
  }
  const gram = readGrammar(root) ?? []
  const sdk = readSdkArm(root) ?? []
  const n = wl.length
  if (n > WHITELIST_BASELINE) {
    errors.push(
      `门面机制臂 ${n} 条 > 基线 ${WHITELIST_BASELINE}——HS-002 只许降（D-253 / D-255）；` +
        '新增一条要把「谁消费、为什么非给插件不可」写成决策理由，并同步上调 `WHITELIST_BASELINE`',
    )
  }

  const used = collectConsumers(root, plugins.filter((p) => !CONTROL_PLANE.includes(p)))
  // 两臂都判：语法臂不计债，但同样不许有空转条目——它也是门面对插件的承诺。
  for (const s of [...wl, ...gram, ...sdk]) {
    if (used.has(s)) continue
    errors.push(
      `门面白名单 \`${s}\` 在扩展层插件代码里零引用——HS-003：白名单每一条都要报得出消费方` +
        '（与 C-003 同一把尺子），报不出的就是空转条目，删掉它',
    )
  }

  // HS-004：插件胶水层的重臂默认必须是关的。文件不存在 ⇒ 这个仓没有那层胶水，无此风险。
  const heavyP = path.join(root, HEAVY_ARM.manifest)
  const heavy = fs.existsSync(heavyP) ? heavyArmViolations(fs.readFileSync(heavyP, 'utf8')) : []
  errors.push(...heavy)

  if (errors.length) {
    for (const e of errors) console.log(`${red('[ERROR]')} ${e}`)
    console.log(`\n扩展层宿主面：${errors.length} 处违规（HS-001 / HS-002 / HS-004）`)
    return 1
  }

  if (n < WHITELIST_BASELINE) {
    console.log(
      yellow(
        `△ HS-002 门面机制臂 ${n} 条 < 基线 ${WHITELIST_BASELINE}：请把 ` +
          '`WHITELIST_BASELINE` 同步下调（账本只许降，基线跟着降才不是空转）',
      ),
    )
  }
  console.log(
    green(
      `✓ HS-001 扩展层零直接依赖：${scannedPlugins} 个非控制面插件的 ${scannedFiles} 个源文件与 manifest ` +
        `没有 \`symbio_core\` / \`symbio-kernel\` / \`symbio-sdk\` / \`symbio-deploy\`（控制面 ${CONTROL_PLANE.length} 个豁免：` +
        `${CONTROL_PLANE.join(' / ')}）· HS-002 机制臂 ${n} 条（基线 ${WHITELIST_BASELINE}）` +
        `· 语法臂 ${gram.length} 条不计债 · SDK 臂 ${sdk.length} 条不计债 ` +
        `· HS-003 每条都有扩展层消费方（0 空转）· HS-004 胶水层重臂默认关闭（${HEAVY_ARM.feature}）`,
    ),
  )
  return 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const arg = process.argv.find((a) => a.startsWith('--root='))
  const root = arg ? path.resolve(arg.slice('--root='.length)) : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
  process.exit(run(root))
}
