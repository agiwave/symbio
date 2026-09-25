#!/usr/bin/env node
/**
 * plugin-entry-audit — `Plugin` trait 两个分形入口（`route` / `traverse`）的地址守卫
 *
 * ## 它守的是什么
 *
 * `Plugin` trait 只有三个方法：`meta` / `route` / `traverse`（`symbio_core/plugin/mod.rs`）。
 * 后两个是全仓**仅有的两条**跨插件寻址通道，而它们的地址是**字符串**——
 * 字符串不会因为改名而编译失败，只会静默地指向一个不存在的地方。
 *
 * 2026-09-18 的审计抓到三处真实漂移，都是这个形状：
 *
 * - `hook` 插件的 `PluginMeta::new("hooks", …)` 与目录名 `hook` 不一致
 *   → `docs/CURRENT.md` 的生成器与两处文档照抄出 **`hooks/fire` 这类不存在的路由**；
 * - Telegram 用 `SESSION_CHAT`（`"session/chat"`）路由——**该路径不存在**，
 *   session 只认 `chat/send` / `chat/abort`，所以那处调用以前必定落到 `NotFound`；
 * - `symbio_core::keys::paths` 里的 `AGENT_CHAT` / `AGENT_CREATE` 是**幽灵常量**：
 *   零调用方，而 `agent` 的 `route` 恒返回 `NotFound`——它们描述的路由不存在。
 *
 * 三处的共同点是：**没有任何测试会因此变红**。本脚本把它们写成可执行的规则。
 *
 * ## 规则
 *
 * | 编号  | 规则                                                          | 为什么 |
 * |-------|---------------------------------------------------------------|--------|
 * | E-001 | `PluginMeta::new` 首参必须 == 插件目录名                        | 目录名才是路由前缀（`composite.rs`「目录名 = 实例名」）；首参不参与路由（ADR-032 之后它只是**出厂 id**，身份取自 `PLUGIN.yml`），不一致就会让文档写出幽灵路由 |
 * | E-002 | 代码里路由路径字面量的首段必须是插件目录名（或容器前缀 `worker`） | 抓「幽灵命名空间」：`hooks/...` 这种写错了前缀的路径 |
 * | E-003 | `set(PATH, "<字面量>")` 一律违规                                | 调用侧的绝对地址必须来自 `symbio_core::keys::paths` 常量，否则改名不会编译失败 |
 * | E-004 | `traverse` 内不得出现 `available_tools` / `available_options` 字面量 | 协议端点只有两个，且必须是常量（`TRAVERSE_AVAILABLE_*`） |
 * | E-005 | 引用的路径必须对应到某条真实 `route` 臂                        | 抓「路径写错一截」：`session/chat` 少了 `/send` |
 * | E-006 | **权威清单**（`ROUTES.md` / `CURRENT.md` / 插件 README）里的路径前缀必须合法 | `hooks/fire` 只出现在文档里，只扫代码的守卫会完整地漏掉它 |
 * | E-007 | 插件不得按**强引用**持有兄弟插件实例（`Arc<dyn Plugin>` 字段）  | 跨插件调用必须经 `ctx.parent()` 走容器；按值持有会绕过地址分发、并在插件重建后钉住旧实例（`telegram` 的 `llm_plugin` 就是这么烂掉的） |
 * | E-008 | 文档里标了 `<!-- vocab:PREFIX_ -->` 的**词表行**必须与代码常量逐字一致 | 闭集的第二份真相常驻文档：`vdfs.md` 的 status 行曾一直写 `error`，而代码早已改名为 `failed`——漂移会从文档**流回**代码 |
 *
 * E-001 ~ E-004、E-007 与 E-008 是 **ERROR**（判据 airtight，可进 `--strict` 门禁）；
 * E-005 / E-006 是 **WARNING**（需要「动态命名空间」白名单配合，宁可先报给人看）。
 *
 * 报告段另给一张表：**每条路由 → 消费方计数**。`refs=0` 的行是「定义了但没人用」
 * 的候选——**它不是判决**，理由有两条，都很容易把人骗过去：
 * ① 运行期拼路径（工具名、子插件名）数不出来；
 * ② **网关会把外部 `path` 原样转发给容器 `route`**（`gateway/server.rs` 的
 * `dispatch_once` / `handle_ws`，只过一层只读白名单）⇒ 对外 API 天然是 `refs=0`。
 * 第一版把 `telegram/*` 六条读成「休眠」，就是漏了第 ② 条。
 *
 * ## 地址规则（本脚本的依据）
 *
 * 见 [`symbio/src/symbio_core/keys/paths.rs`](../symbio/src/symbio_core/keys/paths.rs) 的模块文档：
 * 地址只有「绝对地址」与「相对臂」两种形态，前缀是**插件目录名**，
 * 过路由才设 `PATH`，`traverse` 的 `PATH` 只有两个合法值。
 *
 * ## 已知边界（写清楚，免得把它的绿灯当成证明）
 *
 * 判定基于**去注释后的文本**，不是 AST。因此：
 * - 运行期拼出来的路径（`format!("{}/{}", …)`、`local/<工具名>`）看不见；
 *   这类命名空间列在 `DYNAMIC_NAMESPACES` 里，E-005 对它们**不判定**；
 * - 变量别名、`concat!` 能绕过——它挡的是「顺手写回去」，不是「刻意绕过」；
 * - 消费方计数是**文本计数**，用于人看，不参与判定。
 *
 * ## 用法
 *
 *   node scripts/plugin-entry-audit.mjs                 # 审计全仓
 *   node scripts/plugin-entry-audit.mjs --strict        # WARNING 也算失败
 *   node scripts/plugin-entry-audit.mjs --root=<dir>    # 换根目录（回归测试用）
 *
 * 单行豁免：`// plugin-entry-allow E-00X: 理由`（**理由不可为空**，空理由视为未豁免——
 * 与 `grep-audit` / `mechanism-audit` 同一约定：豁免必须留痕）。
 *
 * 退出码：0 = 通过；1 = 有 ERROR；2 = 仅 WARNING 且带 --strict。
 *
 * 平台无关（Windows / macOS / Linux 通用）：纯 Node，不依赖 bash / ripgrep。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')

const rootArg = process.argv.find((a) => a.startsWith('--root='))
const repoRoot = rootArg ? path.resolve(rootArg.slice(7)) : defaultRoot
const STRICT = process.argv.includes('--strict')

let errors = 0
let warnings = 0
const hitsByRule = new Map()

function report(rule, severity, file, line, message) {
  const mark = severity === 'error' ? red('[ERROR]') : yellow('[WARN] ')
  const where = file ? `${file}${line ? `:${line}` : ''}` : '（全仓）'
  console.log(`${mark} ${rule} ${where}  ${message}`)
  if (severity === 'error') errors++
  else warnings++
  hitsByRule.set(rule, (hitsByRule.get(rule) ?? 0) + 1)
}

// ── 去注释（保留行结构，因此行号仍然准确）────────────────────────────────
//
// 逐字符扫描并跟踪字符串状态，而不是按行 `split('//')`：后者会把字符串里的 `//`
// 当成注释起点，把该行后半段整段吃掉 —— 那是**漏报**，比误报更危险。
// 三种注释形式都要剥：`//`、`/* */`、以及 **`<!-- -->`**（`.vue` 的组件文档写在
// 顶部 HTML 注释里，而那正是最常出现路径字面量的地方；不剥它会把满篇文档判成违规，
// 一个只会误报的守卫最后一定会被人用豁免注释喂到失效）。
function stripComments(source) {
  const out = []
  let line = ''
  let inBlock = false
  let inHtml = false
  let quote = null
  for (let i = 0; i < source.length; i++) {
    const c = source[i]
    const n = source[i + 1]
    if (c === '\n') {
      out.push(line)
      line = ''
      quote = null
      continue
    }
    if (inBlock) {
      if (c === '*' && n === '/') {
        inBlock = false
        i++
      }
      continue
    }
    if (inHtml) {
      if (c === '-' && n === '-' && source[i + 2] === '>') {
        inHtml = false
        i += 2
      }
      continue
    }
    if (quote) {
      line += c
      if (c === '\\') {
        line += n ?? ''
        i++
      } else if (c === quote) {
        quote = null
      }
      continue
    }
    if (c === '/' && n === '*') {
      inBlock = true
      i++
      continue
    }
    if (c === '<' && n === '!' && source[i + 2] === '-' && source[i + 3] === '-') {
      inHtml = true
      i += 3
      continue
    }
    if (c === '/' && n === '/') {
      i++
      while (i + 1 < source.length && source[i + 1] !== '\n') i++
      continue
    }
    if (c === "'" || c === '"' || c === '`') quote = c
    line += c
  }
  out.push(line)
  return out
}

/** 从 `{` 起做字符串感知的花括号配对，返回对应 `}` 的下标（失败返回文本末尾） */
function matchBrace(txt, openIdx) {
  let depth = 0
  let quote = null
  for (let i = openIdx; i < txt.length; i++) {
    const c = txt[i]
    if (quote) {
      if (c === '\\') i++
      else if (c === quote) quote = null
      continue
    }
    if (c === "'" || c === '"' || c === '`') {
      quote = c
      continue
    }
    if (c === '{') depth++
    else if (c === '}') {
      depth--
      if (depth === 0) return i
    }
  }
  return txt.length
}

/**
 * 把 `#[cfg(test)] mod … { … }` 的内容**抹成空白但保留换行**。
 *
 * 为什么不直接删掉：删掉会把后面的代码整体上移，**行号全错**——而报告里的
 * `file:line` 是给人去核对的唯一线索。抹成空白后，索引与原始行一一对应，
 * 豁免注释（写在原始行上）也能按同一个下标取到。
 */
function blankTestModules(txt) {
  const MARKER = '#[cfg(test)]'
  const out = []
  let i = 0
  for (;;) {
    const idx = txt.indexOf(MARKER, i)
    if (idx < 0) {
      out.push(txt.slice(i))
      return out.join('')
    }
    out.push(txt.slice(i, idx))
    const after = txt.slice(idx + MARKER.length)
    const modHead = after.match(/^\s*(?:#\[[^\]]*\]\s*)*mod\s+[A-Za-z0-9_]+\s*\{/)
    if (modHead) {
      const open = idx + MARKER.length + modHead[0].length - 1
      const end = Math.max(matchBrace(txt, open) + 1, open + 1)
      const span = txt.slice(idx, end)
      out.push(span.replace(/[^\n]/g, ' '))
      i = end
    } else {
      i = idx + MARKER.length
    }
  }
}

// ── 文件收集 ─────────────────────────────────────────────────────────────
const SKIP_DIRS = new Set(['node_modules', 'dist', '.git', 'target', 'docs', 'archive'])

function walk(dir, filter) {
  const out = []
  let entries
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true })
  } catch {
    return out
  }
  for (const e of entries) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) {
      if (SKIP_DIRS.has(e.name)) continue
      out.push(...walk(p, filter))
    } else if (e.isFile() && filter(p)) out.push(p)
  }
  return out
}

const isRs = (p) => p.endsWith('.rs')
const isCode = (p) => p.endsWith('.rs') || p.endsWith('.ts') || p.endsWith('.vue')

/**
 * 测试文件：**生产地址规则不适用于它们**。
 *
 * 测试会**故意**写出不存在的路径来验证 `NotFound`（`work/plugin.test.rs` 的
 * `work/whatever`）、用假会话 id 当路径片段（`sessions.spec.ts` 的 `session/s9`）、
 * 或给假插件塞路径。把这些当违规会逼出一堆豁免注释——而一个靠豁免活着的守卫
 * 等于没有守卫（`mechanism-audit` 文件头有同一段论证）。
 *
 * 因此 E-003 / E-004 / E-005 只扫生产文件。E-001（meta 首参）本来就只看生产实现
 * ——测试替身在 `#[cfg(test)] mod` 里，已被 `stripTestModules` 剥掉。
 */
const isTestFile = (p) =>
  p.endsWith('.test.rs') ||
  /(^|[/\\])tests\.rs$/.test(p) ||
  /(^|[/\\])__tests__[/\\]/.test(p) ||
  /\.(?:spec|test)\.(?:ts|js)$/.test(p)

/** 生产 `.rs`（用于提取插件事实） */
const isProdRs = (p) => isRs(p) && !isTestFile(p)

/** 读文件并返回「去注释后的文本」（行号与原始文件一致） */
function readCode(abs) {
  const raw = fs.readFileSync(abs, 'utf8')
  let txt = stripComments(raw).join('\n')
  if (abs.endsWith('.rs')) txt = blankTestModules(txt)
  return txt
}

/** 读文件并返回 { raw, code } 两套行——**豁免看 raw，判定看 code** */
function readLines(abs) {
  const raw = fs.readFileSync(abs, 'utf8').split(/\r?\n/)
  let code = stripComments(raw.join('\n'))
  if (abs.endsWith('.rs')) code = blankTestModules(code.join('\n')).split('\n')
  return { raw, code }
}

const rel = (abs) => path.relative(repoRoot, abs).split(path.sep).join('/')

// ── 常量表：`NAME` → 字符串值（Rust `const` / TS `export const`）──────────
//
// 需要它是因为「引用一条路由」有两种写法：直接写字面量，或经常量
// （`SESSION_CHAT_SEND` / `CHAT_ABORT`）。只查字面量会**漏报**，
// 而漏报会让审计给出「这条路由没人用」的错误结论——2026-09-18 第一版
// 手查就踩过：`session/chat/abort` 字面量计数为 0，实际前端一直在用。
function buildConstTable(files) {
  const table = new Map() // name → value
  for (const abs of files) {
    const txt = readCode(abs)
    if (abs.endsWith('.rs')) {
      // `&str` 与 `&'static str` 都要认——只写 `&'?static\s+str` 会漏掉前者，
      // 于是 `pub const PLUGIN_AGENT: &str = "agent";` 整表取不到，
      // E-001 会把每个用常量声明 meta 的插件都报成「未取到首参」。
      for (const m of txt.matchAll(
        /const\s+([A-Z][A-Z0-9_]*)\s*:\s*&\s*(?:'static\s+)?str\s*=\s*"([^"\n]*)"/g,
      )) {
        table.set(m[1], m[2])
      }
    } else {
      // TS：`const A = 'x'` / ``const A = `${B}/c` ``（按定义顺序单次代换，够用）
      for (const m of txt.matchAll(/(?:export\s+)?const\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^\n]+)/g)) {
        const value = resolveTsExpr(m[2], table)
        if (value !== null) table.set(m[1], value)
      }
    }
  }
  return table
}

/** 解析 TS 常量右值：字符串字面量或 `${已解析常量}/后缀` 链；解析不了返回 null */
function resolveTsExpr(expr, table) {
  let e = expr.trim().replace(/\s+as\s+const\s*$/, '').replace(/;$/, '').trim()
  const tpl = e.match(/^`([^`]*)`$/)
  if (tpl) e = tpl[1]
  else {
    const lit = e.match(/^'([^']*)'$/) || e.match(/^"([^"]*)"$/)
    return lit ? lit[1] : null
  }
  let out = ''
  let rest = e
  for (;;) {
    const m = rest.match(/^\$\{([A-Za-z_][A-Za-z0-9_]*)\}([\s\S]*)$/)
    if (!m) break
    const v = table.get(m[1])
    if (v === undefined) return null
    out += v
    rest = m[2]
  }
  if (rest.includes('${')) return null
  return out + rest
}

/** 收集某常量名在全仓代码里的引用次数（用于报告「这条路由有没有人用」） */
function countConstRefs(name, files) {
  let n = 0
  const re = new RegExp(`\\b${name}\\b`, 'g')
  for (const abs of files) {
    for (const _ of readCode(abs).matchAll(re)) n++
  }
  return n
}

// ── 动态命名空间：按运行期规则分发，静态无法枚举 → E-005 不判定 ──────────
//
// 判据是「该插件的 `route` 有没有静态 `match` 臂」。下面这些插件的分发键来自
// 运行期集合（工具名 / 子插件名 / `VDFS_OPS`），因此任何子路径都可能是合法的。
const DYNAMIC_NAMESPACES = new Set(['local', 'web', 'vdfs', 'composite', 'worker', 'home'])

/**
 * 「运行期分发」的代码特征——`route` 体里没有静态臂，但也不是死路由。
 *
 * - `parse_path(` —— 容器按子插件名分发（`composite`）；
 * - `.find(|t| t.name() ==` —— 按已注册工具名分发（`local` / `web`）；
 * - `VDFS_OPS` —— 按 13 个操作的常量表校验后分发（`vdfs`）。
 *
 * 与「恒 `Err`」区分开很重要：把 `local` 判成死路由会让审计立刻失去可信度。
 */
const DYNAMIC_DISPATCH = /parse_path\(|\.find\(\|t\| t\.name\(\)|VDFS_OPS/

// ── 插件事实提取 ─────────────────────────────────────────────────────────
const PLUGINS_DIR = path.join(repoRoot, 'symbio', 'src', 'plugins')

/** 该绝对路径是否落在 `plugins/` 之下（E-007 的适用范围） */
const isInPluginsDir = (abs) => abs.startsWith(PLUGINS_DIR + path.sep)

/**
 * 提取 `async fn <name>(…)` 的函数体。
 *
 * 只认**第一个** `async fn <name>(`（生产实现总在测试替身之前，且测试模块已被
 * `stripTestModules` 剥掉）。
 */
function fnBody(txt, name) {
  const i = txt.indexOf(`async fn ${name}(`)
  if (i < 0) return null
  const open = txt.indexOf('{', i)
  if (open < 0) return null
  return txt.slice(open, matchBrace(txt, open) + 1)
}

/**
 * 从 `route` 体里提取**相对臂**：`match` 臂左侧的字符串字面量。
 *
 * 只认带 `=>` 的行——不整段抓字符串，否则会把 `get("approved")` 这类参数名当成路由
 * （`gen-current-facts.mjs` 实测踩过同一个坑）。
 */
function routeArms(body) {
  const out = new Set()
  for (const line of body.split('\n')) {
    const eq = line.indexOf('=>')
    if (eq < 0) continue
    for (const m of line.slice(0, eq).matchAll(/"([a-z][a-z0-9_/-]*)"/g)) {
      if (m[1] === '_') continue
      out.add(m[1])
    }
  }
  return [...out].sort()
}

/** 从 `traverse` 体里提取**协议端点**（只可能有两个合法值） */
function traverseEndpoints(body) {
  const out = new Set()
  if (/\bTRAVERSE_AVAILABLE_TOOLS\b/.test(body)) out.add('available_tools')
  if (/\bTRAVERSE_AVAILABLE_OPTIONS\b/.test(body)) out.add('available_options')
  return [...out].sort()
}

function discoverPlugins() {
  let dirs
  try {
    dirs = fs
      .readdirSync(PLUGINS_DIR, { withFileTypes: true })
      .filter((e) => e.isDirectory())
      .map((e) => e.name)
      .sort()
  } catch {
    return []
  }
  const out = []
  for (const dirName of dirs) {
    const dir = path.join(PLUGINS_DIR, dirName)
    // 只取**生产** `.rs`：测试替身（`composite/vdfs.rs` 的 `FakeChild`、
    // `vdfs/host.rs` 的 `FakeContainer`）与测试用例里的假路径都不该参与判定。
    // 顺序陷阱：`walk` 先深度再同级，`chat_loop/state.test.rs` 会排在 `plugin.rs`
    // 之前——不过滤掉它，`indexOf('async fn route(')` 会取到测试里的那个 `route`，
    // 于是 session 被误判成「恒 NotFound、零路由臂」。
    const files = walk(dir, isProdRs)
    const txts = files.map((f) => ({ abs: f, txt: readCode(f) }))
    const implFile = txts.find((t) => t.txt.includes(`impl Plugin for`))
    if (!implFile) continue // 不是插件目录（如 providers/ 之类的辅助模块）

    const all = txts.map((t) => t.txt).join('\n')
    const metaMatch = all.match(/PluginMeta::new\(\s*([A-Za-z_][A-Za-z0-9_]*|"[^"]*")/)
    const metaRaw = metaMatch ? metaMatch[1] : null

    const routeBody = fnBody(all, 'route')
    const travBody = fnBody(all, 'traverse')
    const arms = routeBody ? routeArms(routeBody) : []

    out.push({
      dirName,
      metaRaw,
      metaRef: implFile.abs,
      routeBody,
      travBody,
      arms,
      endpoints: travBody ? traverseEndpoints(travBody) : [],
      // 分派形态：静态 `match` 臂 / 运行期规则 / 恒 `Err`
      //
      // 「恒 `Err`」的判据是**既没有臂、也没有运行期分发**——`agent` / `mcp` /
      // `model` / `setting` / `work` 是这一类（`route` 直接返回 `NotFound` 并指路 VDFS）。
      // `composite` / `local` / `web` / `vdfs` 有臂为零但按运行期集合分发，不是死路由。
      kind: arms.length
        ? 'static'
        : routeBody && DYNAMIC_DISPATCH.test(routeBody)
          ? 'dynamic'
          : routeBody
            ? 'always-not-found'
            : 'no-route',
    })
  }
  return out
}

// ── 扫描主体 ─────────────────────────────────────────────────────────────
const pluginDirs = discoverPlugins()
const dirNames = new Set(pluginDirs.map((p) => p.dirName))

// 常量表：先收全仓（`ids.rs` 的 `PLUGIN_*`、`paths.rs` 的路由常量、
// 前端 `pluginPaths.ts` 的模板串链）
const CODE_ROOTS = ['symbio/src', 'cli/src', 'tauri/src']
const codeFiles = CODE_ROOTS.flatMap((r) => walk(path.join(repoRoot, r), isCode))
const consts = buildConstTable(codeFiles)
// 同名前缀在**两侧可能不是同一套词表**：前端 `@/schemas/vdfs` 另有自己的
// `VDFS_STATUS_*` 镜像，且它把会话节点透传的 `MessageStatus` 词（`pending` /
// `streaming` / `waiting_user_action` / `completed`）也算进去，取值集合比 Rust 侧大。
// E-008 判的是**设计文档与 Rust 契约**是否一致，故只用 Rust 侧的常量：
const rustConsts = buildConstTable(codeFiles.filter((f) => f.endsWith('.rs')))

/** 解析 `PluginMeta::new` 首参：可能是 `"字面量"` 或常量名 */
function resolveMeta(raw) {
  if (!raw) return null
  if (raw.startsWith('"')) return raw.slice(1, -1)
  return consts.get(raw) ?? null
}

const metaIdToDirs = new Map() // meta id → 声明它的插件目录（用于 E-001 报错定位）
for (const p of pluginDirs) {
  const id = resolveMeta(p.metaRaw)
  p.metaId = id
  if (id) metaIdToDirs.set(id, p.dirName)
}

// 合法绝对路径集：插件目录名 + 相对臂。
// `home` 是系统根（不挂在任何前缀下），它的臂已经是全名，原样收。
const validPaths = new Set()
for (const p of pluginDirs) {
  for (const arm of p.arms) {
    validPaths.add(p.dirName === 'home' ? arm : `${p.dirName}/${arm}`)
  }
}

// ── E-001：`PluginMeta::new` 首参必须 == 插件目录名 ─────────────────────
for (const p of pluginDirs) {
  if (p.metaId === null) {
    report('E-001', 'warning', rel(p.metaRef), 0, `未取到 \`PluginMeta::new\` 首参（跳过）`)
    continue
  }
  if (p.metaId !== p.dirName) {
    report(
      'E-001',
      'error',
      rel(p.metaRef),
      0,
      `插件目录名是 \`${p.dirName}\`，而 \`PluginMeta::new\` 首参是 \`${p.metaId}\`；` +
        `容器按**目录名**分发，首参不一致会让生成器/文档写出 \`${p.metaId}/…\` 这类不存在的路由`,
    )
  }
}

// ── 逐行扫描代码，跑 E-002 / E-003 / E-004 ───────────────────────────────
//
// 「路由形状」的判据：`a/b` 或更长的全小写段，首段是标识符。
// 首段必须落在「插件目录名 ∪ 容器前缀」内——只查这个集合能天然排除
// `docs/...`、`.vdfs/...`、`src/...` 这类非路由路径，无需额外白名单。
const CONTAINER_PREFIXES = new Set(['worker'])
const ROUTE_SHAPE = /^[a-z][a-z0-9_]*(?:\/[a-z0-9_-]+)+$/
const EXEMPT_RE = /plugin-entry-allow\s+(E-\d{3})\s*:\s*(\S.*)$/

/**
 * 常量**定义行**（`const NAME: &'static str = "…"`）。
 *
 * 定义行上的字面量是「这条路由的单一真相源」本身，不是「对它的引用」——
 * 判 E-005（路径是否存在）会自指成假阳性，判 E-004 会把
 * `TRAVERSE_AVAILABLE_TOOLS` 的定义处本身报成「写了字面量」。
 * 常量有没有消费方由报告段的 `refs=` 负责，不归 E-005。
 */
const CONST_DEF_RE = /(?:pub\s+)?const\s+[A-Z][A-Z0-9_]*\s*:\s*&\s*(?:'static\s+)?str\s*=/

/**
 * 字段/形参声明行：`name: Type,`（可带 `pub`）。
 *
 * E-007 用它取「字段名 + 类型」。**形参也会命中**——这是刻意的：Rust 里
 * 「按值持有」与「按参数收下」在语法上长得一样，只能用类型形态与字段名区分，
 * 见 `isSiblingPluginRef` 的四条豁免。
 */
const FIELD_DECL_RE = /^\s*(?:pub\s+)?([a-z_][a-z0-9_]*)\s*:\s*(.+?),?\s*$/

/**
 * 这两个名字在本仓**专指「向上引用父」**（`gateway` / `hook` / `local` /
 * `vdfs::host` / `session` 的编排链都用它们指向容器）。
 * 容器挂载子插件时把自身 `Weak` 塞进子 ctx，`ctx.parent()` 取的就是它——
 * 父引用是**允许**按值持有的（弱引用拿不到时才能退化为错误，强引用不会）。
 */
const UPWARD_FIELD_NAMES = new Set(['parent', 'router'])

/**
 * 该「`name: Type`」行是否构成**按值持有兄弟插件实例**（E-007 命中）。
 *
 * 四条豁免，逐条都有实据（不是猜的）：
 * 1. 类型含 `Weak` —— 向上引用父，且**不阻止**实例释放；
 * 2. 类型含 `HashMap` / `BTreeMap` / `Vec` —— 容器按名持有**多个子实例**
 *    （`composite.instances`、`home.instances`、`agent.host.sub_agents`）；
 * 3. 字段名是 `parent` / `router` —— 向上引用，见 `UPWARD_FIELD_NAMES`；
 * 4. 类型以 `&` 开头 —— 借用，**字段不可能长这样**（没有生命周期的结构体字段
 *    编译不过），所以这类行必然是形参（`agent.host::…(tree: &Arc<dyn Plugin>)`）。
 *
 * 另有第五类不算豁免而是**前置过滤**：行首在圆括号内 ⇒ 形参 / 实参，不是字段
 * （多行签名里的 `plugin: Arc<dyn Plugin>,`）。判据在调用点的 `parenDepthAtLineStart`。
 */
function isSiblingPluginRef(name, typeText) {
  if (!/Arc<\s*dyn\s+Plugin\s*>/.test(typeText)) return false
  if (/Weak/.test(typeText)) return false
  if (/(?:HashMap|BTreeMap|Vec)\s*</.test(typeText)) return false
  if (UPWARD_FIELD_NAMES.has(name)) return false
  if (typeText.trimStart().startsWith('&')) return false
  return true
}

/**
 * 该行净增的圆括号数（先剥字符字面量与字符串，免得 `'('` / `"("` 被当成括号）。
 *
 * 顺序不能反：字符字面量先剥，否则 `'"'` 会被字符串规则吃掉一半。
 */
function parenDelta(line) {
  const cleaned = line.replace(/'(?:\\.|[^\\'])'/g, "''").replace(/"(?:\\.|[^"\\])*"/g, '""')
  let d = 0
  for (const c of cleaned) {
    if (c === '(') d += 1
    else if (c === ')') d -= 1
  }
  return d
}

/**
 * 每行**行首**的圆括号深度（按去注释后的代码行累计）。
 *
 * 用途：E-007 只该命中**结构体字段**。但多行函数签名的每个形参也长成
 * `plugin: Arc<dyn Plugin>,`——与字段行**逐字同形**，`FIELD_DECL_RE` 分不出来，
 * 于是 `composite::broadcast_collect(plugin: Arc<dyn Plugin>, …)` 被误判成
 * 「按值持有兄弟插件实例」（2026-09-23 实测：门禁因此变红）。
 *
 * 判据：**字段不可能出现在圆括号里**。行首深度 > 0 ⇒ 本行是形参 / 实参，不是字段。
 * 单行元组结构体（`pub struct Newtype(Arc<dyn Plugin>);`）不受影响——它本来就匹配
 * 不上 `FIELD_DECL_RE`（该正则要求整行就是 `name: Type,`）。
 */
function parenDepthAtLineStart(lines) {
  const out = []
  let depth = 0
  for (const line of lines) {
    out.push(depth)
    depth += parenDelta(line)
  }
  return out
}

/**
 * 该行是否对某条规则留了豁免（理由不可为空）。
 *
 * **必须在原始行上找**：豁免写在注释里，去注释之后它自己也消失了——
 * 用去注释的行去找豁免，结果是「豁免永远不生效」，而失败信息只会说「命中」，
 * 没人看得出是豁免机制坏了（`mechanism-audit` 踩过同一个坑，见其 `auditFiles` 注释）。
 *
 * 豁免注释写在**上一行**或**本行行尾**：多行语句（`ctx.set(\n  PATH,\n  "…"\n)`）
 * 的行尾放不下注释，只支持行尾会逼着人把语句挤成一行。
 */
function exempted(rawLines, lineIdx, rule) {
  for (const i of [lineIdx - 1, lineIdx]) {
    if (i < 0 || i >= rawLines.length) continue
    const m = rawLines[i].match(EXEMPT_RE)
    if (m && m[1] === rule) return true
  }
  return false
}

for (const abs of codeFiles) {
  // 测试文件不参与地址判定（见 isTestFile 的说明）
  if (isTestFile(abs)) continue
  const { raw, code } = readLines(abs)
  const isRust = abs.endsWith('.rs')
  const txt = code
  const parenDepth = parenDepthAtLineStart(txt)

  for (let i = 0; i < txt.length; i++) {
    const line = txt[i]
    const isConstDef = isRust && CONST_DEF_RE.test(line)

    // E-003：调用侧不得写字面量 PATH（绝对地址必须来自 symbio_core::keys::paths 常量）
    if (isRust && /\.set\(\s*(?:crate::symbio_core::)?PATH\s*,\s*"/.test(line)) {
      if (!exempted(raw, i, 'E-003')) {
        report(
          'E-003',
          'error',
          rel(abs),
          i + 1,
          `\`set(PATH, "<字面量>")\` —— 绝对地址必须取 \`symbio_core::keys::paths\` 常量` +
            `（字面量不会因改名而编译失败）`,
        )
      }
    }

    // E-004：`traverse` 端点必须用常量
    if (isRust && !isConstDef && /"available_(?:tools|options)"/.test(line)) {
      if (!exempted(raw, i, 'E-004')) {
        report(
          'E-004',
          'error',
          rel(abs),
          i + 1,
          `协议端点写了字面量 —— 必须用 \`TRAVERSE_AVAILABLE_TOOLS\` / \`TRAVERSE_AVAILABLE_OPTIONS\``,
        )
      }
    }

    // E-007：插件不得按值持有兄弟插件实例
    //
    // 只扫 `plugins/` 之下：`session/chat_loop/state.rs` 等虽在 `plugins/` 里，
    // 但其 `parent` 字段是向上引用（豁免 3），不会误报。
    // **行首括号深度 > 0 ⇒ 形参，不是字段**（见 `parenDepthAtLineStart`）——
    // 多行函数签名里的 `plugin: Arc<dyn Plugin>,` 与字段行逐字同形。
    if (isRust && isInPluginsDir(abs) && parenDepth[i] === 0) {
      const m = line.match(FIELD_DECL_RE)
      if (m && isSiblingPluginRef(m[1], m[2]) && !exempted(raw, i, 'E-007')) {
        report(
          'E-007',
          'error',
          rel(abs),
          i + 1,
          `\`${m[1]}: ${m[2]}\` —— 插件不得按值持有兄弟插件实例；` +
            `跨插件调用请用 \`ctx.parent()\` 取容器后 \`parent.route(ctx)\`` +
            `（绝对地址由容器分发，见 \`docs/design/plugin-route-address.md\`）`,
        )
      }
    }

    // E-002 / E-005：路径字面量
    for (const m of line.matchAll(/"([^"\n]+)"|'([^'\n]+)'/g)) {
      const lit = m[1] ?? m[2]
      if (!lit || !ROUTE_SHAPE.test(lit)) continue
      const first = lit.slice(0, lit.indexOf('/'))
      const isDir = dirNames.has(first)
      const isContainer = CONTAINER_PREFIXES.has(first)
      const isMeta = metaIdToDirs.has(first)

      // E-002：首段既不是插件目录名也不是容器前缀 —— 幽灵命名空间
      if (!isDir && !isContainer) {
        if (isMeta && !exempted(raw, i, 'E-002')) {
          report(
            'E-002',
            'error',
            rel(abs),
            i + 1,
            `\`${lit}\` 的首段 \`${first}\` 是 \`${metaIdToDirs.get(first)}\` 插件的 **meta id**，` +
              `不是它的目录名 —— 容器按目录名分发，这条路径不存在`,
          )
        }
        continue
      }

      // E-005：路径存在性（首段合法才继续判；常量定义行自指，跳过）
      if (isConstDef) continue
      const normalized = isContainer ? lit.slice(first.length + 1) : lit
      const head = normalized.slice(0, normalized.indexOf('/'))
      if (DYNAMIC_NAMESPACES.has(head)) continue
      if (validPaths.has(normalized)) continue
      if (exempted(raw, i, 'E-005')) continue
      report(
        'E-005',
        'warning',
        rel(abs),
        i + 1,
        `\`${lit}\` 不对应任何一条真实 \`route\` 臂（\`${head}\` 插件是静态分派的）`,
      )
    }
  }
}

// ── E-006：路由**权威清单**里的路径前缀必须是插件目录名（文档侧）─────────
//
// 为什么只扫这几个文件：`hooks/fire` 这个幽灵路径**不在任何代码里**——它出现在
// `docs/CURRENT.md`、`docs/reference/ROUTES.md` 与 `hook/README.md` 三处文档中。
// 只扫代码的守卫会**完整地漏掉它**。
//
// 为什么只判**前缀**、不判整条路径是否存在：权威清单里**必须**能提到已退役的路由
// （`session/append`、`model/chat`、`telegram/config/get`…），否则迁移记录就无从写起。
// 第一版把「整条路径必须存在」也加上，结果 22 条告警**全部**是这类正当的历史提及
// ——一个只会误报的守卫最后一定会被人用豁免注释喂到失效。前缀判据没有这个问题：
// 退役路由的前缀仍然是对的（`session/append` 的 `session` 合法），
// 而 `hooks/fire` 的 `hooks` 一眼就是错的。
//
// 又为什么不是所有 `.md`：`docs/archive/` 与各插件的迁移记录里会有大量历史路径，
// 只扫「自称是当前权威清单」的那几份。
const ROUTE_AUTHORITY_FILES = [
  path.join(repoRoot, 'docs', 'CURRENT.md'),
  path.join(repoRoot, 'docs', 'reference', 'ROUTES.md'),
  ...(() => {
    try {
      return fs
        .readdirSync(PLUGINS_DIR, { withFileTypes: true })
        .filter((e) => e.isDirectory())
        .map((e) => path.join(PLUGINS_DIR, e.name, 'README.md'))
        .filter((p) => fs.existsSync(p))
    } catch {
      return []
    }
  })(),
]

for (const abs of ROUTE_AUTHORITY_FILES) {
  if (!fs.existsSync(abs)) continue
  const lines = fs.readFileSync(abs, 'utf8').split('\n')
  for (let i = 0; i < lines.length; i++) {
    // markdown 里路径都写在反引号内
    for (const m of lines[i].matchAll(/`([^`\n]+)`/g)) {
      const lit = m[1]
      if (!ROUTE_SHAPE.test(lit)) continue
      const first = lit.slice(0, lit.indexOf('/'))
      if (dirNames.has(first) || CONTAINER_PREFIXES.has(first)) continue
      if (!metaIdToDirs.has(first)) continue // 连插件都不沾的路径不判（如 `docs/…`）
      if (exempted(lines, i, 'E-006')) continue
      report(
        'E-006',
        'warning',
        rel(abs),
        i + 1,
        `权威清单里的 \`${lit}\` 首段 \`${first}\` 是 \`${metaIdToDirs.get(first)}\` 插件的 **meta id**，` +
          `不是目录名 —— 容器按目录名分发，这条路径不存在`,
      )
    }
  }
}

// ── E-008：文档标了 `<!-- vocab:PREFIX_ -->` 的词表行必须与代码常量逐字一致 ──
//
// 为什么需要：词表是**闭集**，而闭集的第二份真相最常驻在文档里。真实事故：
// `docs/design/vdfs.md` §3.2 的 status 行一直写 `error`，而代码早已把该词改名为
// `failed`（理由见 `vdfs/words.rs::VDFS_STATUS_FAILED`）——两边各说各话，没有任何
// 测试因此变红。而人读文档写的代码会照 `error` 写，于是漂移**从文档流回代码**
//（`schemas/options.rs` 里那枚 `OPTION_STATUS_ERROR = "error"` 就是这么活下来的；
// 该文件已于 2026-09-23 随会话选项 schema 化删除，但这条例子的教训与文件无关）。
//
// 为什么要显式标记、而不是猜散文位置：文档是自然语言，靠"这行像在枚举词表"来判
// 必然误报，而一个只会误报的守卫最后一定会被人用豁免注释喂到失效（E-006 的教训）。
// 故要求文档在那一行写明 `<!-- vocab:<常量前缀> -->`——**标了才判，没标不猜**。
//
// 判据（双向，两侧都静态已知）：
//   ① **枚举段**里的小写词必须是代码词表成员 → 抓「文档落后于代码的改名/删词」；
//   ② 代码词表的每个**非空取值**都必须在该行出现 → 抓「代码新增词而文档没跟上」。
//
// 什么是「枚举段」：反引号词组成的 `/` 分隔串（`` `a` / `b` / `c` ``）——本仓文档
// 枚举词表就是这个写法。不这么限定就会把**字段名**（行首的 `status`）和夹在散文里
// 的常量名一并当成取值（首版实测：`status` 被误判为词表外的词）。
//
// 取值只取 **Rust 侧**常量（见上方 `rustConsts` 的说明）：前端 `schemas/vdfs.ts`
// 有自己的同名前缀镜像且取值更宽，混进来会把文档判成"漏列"。前端的词表自持性由
// `mechanism-audit` 的 M-007 那条线管（地址常量只能在 `schemas/vdfs.ts` 定义）。
const VOCAB_MARK_RE = /<!--\s*vocab:\s*([A-Z][A-Z0-9_]*)\s*-->/
const ENUM_RUN_RE = /`[a-z][a-z0-9_.-]*`(?:\s*\/\s*`[a-z][a-z0-9_.-]*`)+/g
const ENUM_WORD_RE = /`([^`\n]+)`/g
const VOCAB_MD_FILES = (() => {
  const out = []
  const scan = (dir) => {
    let ents
    try {
      ents = fs.readdirSync(dir, { withFileTypes: true })
    } catch {
      return
    }
    for (const e of ents) {
      const p = path.join(dir, e.name)
      // `docs/archive/` 是历史归档（改写即篡改历史，见 doc-link-audit 的同款豁免）
      if (e.isDirectory()) {
        if (e.name !== 'archive') scan(p)
        continue
      }
      if (e.name.endsWith('.md')) out.push(p)
    }
  }
  scan(path.join(repoRoot, 'docs'))
  return out
})()

for (const abs of VOCAB_MD_FILES) {
  const lines = fs.readFileSync(abs, 'utf8').split('\n')
  for (let i = 0; i < lines.length; i++) {
    const mark = lines[i].match(VOCAB_MARK_RE)
    if (!mark || exempted(lines, i, 'E-008')) continue
    const prefix = mark[1]
    const values = [...rustConsts.entries()]
      .filter(([n, v]) => n.startsWith(prefix) && v !== '')
      .map(([, v]) => v)
    if (!values.length) {
      report(
        'E-008',
        'error',
        rel(abs),
        i + 1,
        `词表标记 \`${prefix}\` 在代码里找不到任何同名常量（前缀写错了？）`,
      )
      continue
    }
    const documented = [...lines[i].matchAll(ENUM_RUN_RE)].flatMap((run) =>
      [...run[0].matchAll(ENUM_WORD_RE)].map((w) => w[1]),
    )
    for (const w of documented.filter((w) => !values.includes(w))) {
      report(
        'E-008',
        'error',
        rel(abs),
        i + 1,
        `文档写着 \`${w}\`，但 \`${prefix}*\` 词表里没有它 —— 文档落后于代码`,
      )
    }
    for (const v of values.filter((v) => !documented.includes(v))) {
      report('E-008', 'error', rel(abs), i + 1, `代码词表有 \`${v}\`，本行未列出 —— 文档落后于代码`)
    }
  }
}

// ── 报告：路由表 + 消费方 ───────────────────────────────────────────────
console.log('')
console.log(dim('── 路由表（绝对地址 → 消费方）─────────────────────────────────'))
const KIND_LABEL = {
  static: '静态分派',
  dynamic: '运行期动态',
  'always-not-found': '恒 NotFound',
  'no-route': '未实现 route',
}
for (const p of pluginDirs) {
  const eps = p.endpoints.length ? p.endpoints.join(' · ') : '（无）'
  console.log(dim(`  ${p.dirName}  [${KIND_LABEL[p.kind] ?? p.kind}]  traverse: ${eps}`))
  for (const arm of p.arms) {
    const abs = p.dirName === 'home' ? arm : `${p.dirName}/${arm}`
    const dynamic = DYNAMIC_NAMESPACES.has(p.dirName)
    // 消费方 = 字面量出现次数 + 解析到该路径的常量引用次数
    let refs = 0
    const re = new RegExp(`"${abs.replace(/[/\\]/g, '\\$&')}"`, 'g')
    for (const f of codeFiles) refs += [...readCode(f).matchAll(re)].length
    for (const [name, value] of consts) {
      const v = value.replace(/^worker\//, '')
      if (v !== abs) continue
      // 常量自身的定义处不计
      refs += Math.max(0, countConstRefs(name, codeFiles) - 1)
    }
    const flag = refs === 0 && !dynamic ? yellow(' ← 无消费方') : ''
    console.log(`      ${abs}  ${dim(`refs=${refs}`)}${flag}`)
  }
}

// ── 汇总 ─────────────────────────────────────────────────────────────────
console.log('')
const ruleNames = {
  'E-001': 'meta 首参 == 目录名',
  'E-002': '首段是目录名',
  'E-003': '调用侧用常量',
  'E-004': 'traverse 端点用常量',
  'E-005': '路径真实存在',
  'E-006': '权威清单前缀合法',
  'E-007': '不按值持有兄弟插件',
  'E-008': '文档词表 == 代码词表',
}
for (const [rule, name] of Object.entries(ruleNames)) {
  const n = hitsByRule.get(rule) ?? 0
  const mark = n === 0 ? green('✓') : rule === 'E-005' || rule === 'E-006' ? yellow('!') : red('✗')
  console.log(`  ${mark} ${rule}  ${name}  ${n === 0 ? '' : `(${n})`}`)
}
console.log('')

if (errors > 0) {
  console.log(red(`  ${errors} 个 ERROR，${warnings} 个 WARNING`))
  process.exit(1)
}
if (warnings > 0 && STRICT) {
  console.log(yellow(`  ${warnings} 个 WARNING（--strict 视为失败）`))
  process.exit(2)
}
// 条数**从规则表推导**，不手写——手写的「七条」在加规则时必然漂成一句谎话
console.log(green(`  ${Object.keys(ruleNames).length} 条规则全部通过${warnings ? `（${warnings} 个 WARNING）` : ''}`))
process.exit(0)
