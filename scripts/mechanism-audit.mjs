#!/usr/bin/env node
/**
 * mechanism-audit — 前端**机制化**的静态守卫
 *
 * ## 它守的是什么
 *
 * 前端的机制化目标是一句话：**新增一种资源 / 消息类型时，前端不需要改代码**。
 * 这条性质靠「知识集中在少数几个地方」维持，而知识一旦散回去，不会有任何测试变红
 * ——只会慢慢腐烂，直到某天有人发现"加一个类型要改五个文件"。本脚本把那几个
 * 「知识该待的地方」写成可执行的规则。
 *
 * ## 规则（分层见 tauri/src/registry 与 tauri/src/schemas 的文件头注释）
 *
 * | 编号  | 规则                                                  | 为什么                       |
 * |-------|-------------------------------------------------------|------------------------------|
 * | M-001 | 组件不得解释后端 `meta` 字段                           | 后端改字段名不该改视图        |
 * | M-002 | 组件 / 组合式不得硬编码 `.vdfs` 地址                    | 地址构造只有 `schemas/vdfs`   |
 * | M-003 | 不得直接 `invoke`，一律经 `services/`                   | 出站协议切换只改一处          |
 * | M-004 | `registry/` `schemas/` 不得 import 组件                 | 契约与映射必须能被非视图引用  |
 * | M-005 | `schemas/` 不得依赖 `registry/` `components/` `composables/` | 数据契约零呈现依赖（防环） |
 * | M-006 | 组件不得用字面量比较消息词表                            | 词表只有 `schemas/chat_message` |
 * | M-007 | 地址常量只能在 `schemas/vdfs.ts` **定义**                | 段名常量不得有第二份真相      |
 *
 * M-004 的例外是 `*Renderers.ts`：那是**刻意**的唯一组件装配点（把渲染器标识绑到
 * 具体组件），否则「新增一种形态只登记一行」就无从谈起。
 *
 * ## 已知边界（写清楚，免得把它的绿灯当成证明）
 *
 * 判定基于**去注释后的文本**，不是 AST。因此：
 * - 变量别名能绕过（`const m = node.meta; m.recoverable`）——M-001 抓不到；
 * - 字符串里的 `//`（如 URL）会被当成行注释起点，可能造成**漏报**；
 * - 它挡的是「顺手写回去」，不是「刻意绕过」。
 *
 * ## 用法
 *
 *   node scripts/mechanism-audit.mjs                 # 审计 tauri/src
 *   node scripts/mechanism-audit.mjs --strict        # warning 也算失败
 *   node scripts/mechanism-audit.mjs --root=<dir>    # 换根目录（回归测试用）
 *
 * 单行豁免：`// mechanism-allow M-00X: 理由`（**理由不可为空**，空理由视为未豁免——
 * 与 `grep-audit` 的 `grep-audit-allow` 同一约定：豁免必须留痕）。
 *
 * 退出码：0 = 通过；1 = 有 ERROR；2 = 仅 WARNING 且带 --strict。
 *
 * 平台无关（Windows / macOS / Linux 通用）：纯 Node，不依赖 bash / ripgrep。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')

const rootArg = process.argv.find((a) => a.startsWith('--root='))
const repoRoot = rootArg ? path.resolve(rootArg.slice(7)) : defaultRoot
const STRICT = process.argv.includes('--strict')

// ── 输出配色（非 TTY / NO_COLOR 时自动关闭，CI 日志保持纯净）──────────────
const useColor = Boolean(process.stdout.isTTY) && process.env.NO_COLOR === undefined
const paint = (code) => (s) => (useColor ? `\x1b[${code}m${s}\x1b[0m` : s)
const red = paint('0;31')
const yellow = paint('0;33')
const green = paint('0;32')
const dim = paint('2')

let errors = 0
let warnings = 0
const hitsByRule = new Map()

function report(rule, severity, file, line, message) {
  const mark = severity === 'error' ? red('[ERROR]') : yellow('[WARN] ')
  console.log(`${mark} ${rule} ${file}:${line}  ${message}`)
  if (severity === 'error') errors++
  else warnings++
  hitsByRule.set(rule, (hitsByRule.get(rule) ?? 0) + 1)
}

// ── 去注释（保留行结构，因此行号仍然准确）────────────────────────────────
//
// 逐字符扫描并跟踪字符串状态，而不是按行 `split('//')`：后者会把字符串里的
// `//`（URL、正则）误当成注释起点，把该行后半段整段吃掉 —— 那是**漏报**，
// 比误报更危险（守卫静默失效）。
//
// 三种注释形式都要剥：`//`、`/* */`、以及 **`<!-- -->`**。第三种不是可有可无的：
// `.vue` 的组件文档写在文件顶部的 HTML 注释里，而那里正是最常出现 `.vdfs/session`
// 这类地址字面量的地方（举例说明）。不剥它，M-002 会把满篇文档判成违规——
// 一个只会误报的守卫，最后一定会被人用豁免注释喂到失效。
function stripComments(source) {
  const out = []
  let line = ''
  let inBlock = false
  let inHtml = false
  let quote = null // 当前字符串定界符（' / " / `）
  for (let i = 0; i < source.length; i++) {
    const c = source[i]
    const n = source[i + 1]
    if (c === '\n') {
      out.push(line)
      line = ''
      // 块注释与 HTML 注释跨行、字符串不跨行（模板字面量跨行属漏报，见文件头「已知边界」）
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
      // 行注释：吞到行尾（换行由上面的分支处理）
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

// ── 文件收集 ─────────────────────────────────────────────────────────────
const SKIP_DIRS = new Set(['node_modules', 'dist', '__tests__', '.git', 'target'])

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

const isVue = (p) => p.endsWith('.vue')
const isTs = (p) => p.endsWith('.ts') && !p.endsWith('.d.ts')
const isVueOrTs = (p) => isVue(p) || isTs(p)

const SRC = path.join(repoRoot, 'tauri', 'src')

/** 显示用路径：相对仓库根，统一正斜杠（跨平台一致） */
const disp = (p) => path.relative(repoRoot, p).split(path.sep).join('/')

/** 行内豁免：`mechanism-allow M-00X: 理由`（理由为空视为未豁免） */
function waived(rule, rawLine) {
  const re = new RegExp(`mechanism-allow\\s+${rule}\\s*:\\s*(\\S.*)$`)
  const m = rawLine.match(re)
  return Boolean(m && m[1].trim())
}

/**
 * 逐文件逐行跑一组规则。
 *
 * 判定用**去注释**的行，豁免用**原始**行——两者必须分开：
 * 豁免写在注释里，去注释之后它自己也消失了。用去注释的行去找豁免，
 * 结果是「豁免永远不生效」，而失败信息只会说"命中"，没人看得出是豁免机制坏了。
 *
 * @param {string} scopeLabel 报告里显示的 scope
 * @param {string[]} files
 * @param {Array<{rule:string, severity:'error'|'warn', test:(code:string)=>RegExpMatchArray|null, message:string, skip?:(file:string)=>boolean}>} rules
 */
function auditFiles(scopeLabel, files, rules) {
  console.log(dim(`  scope: ${scopeLabel}（${files.length} 个文件）`))
  for (const file of files) {
    const rel = disp(file)
    let raw
    let code
    try {
      raw = fs.readFileSync(file, 'utf8').split(/\r?\n/)
      code = stripComments(fs.readFileSync(file, 'utf8'))
    } catch {
      continue
    }
    for (const rule of rules) {
      if (rule.skip?.(file)) continue
      for (let i = 0; i < code.length; i++) {
        if (!rule.test(code[i])) continue
        if (waived(rule.rule, raw[i] ?? '')) continue
        report(rule.rule, rule.severity, rel, i + 1, rule.message)
      }
    }
  }
}

// ── 规则定义 ─────────────────────────────────────────────────────────────
const MESSAGE_VOCAB =
  'text|reasoning|tool_call|turn|user_prompt|compression|streaming|pending|waiting_user_action|completed|aborted|failed|user|assistant|tool|system'

const RULES = {
  // M-001 组件不得解释后端 meta 字段（两种形态：直接取值 / 先断言再取值）
  metaField: {
    rule: 'M-001',
    severity: 'error',
    test: (c) =>
      c.match(/\.meta\s*\??\.\s*[A-Za-z_$]/) || c.match(/\.meta\s+as\s+[^)]*\)\s*\??\.\s*[A-Za-z_$]/),
    message:
      '组件不得直接解释后端 meta 字段 —— 取值集中到 registry/messageTypes（后端改字段名时只改那一处）',
  },
  // M-002 组件 / 组合式不得硬编码 .vdfs 地址
  vdfsAddr: {
    rule: 'M-002',
    severity: 'error',
    test: (c) => c.match(/['"`]\.vdfs(?:\/|['"`])/),
    message: '不得硬编码 .vdfs 地址 —— 用 schemas/vdfs 的 VDFS_ROOT / vdfsJoin / vdfsSessionAddr 构造',
  },
  // M-003 不得直接 invoke
  directInvoke: {
    rule: 'M-003',
    severity: 'error',
    test: (c) => c.match(/\binvoke\s*\(/) || c.match(/from\s+['"]@tauri-apps\/api\/core['"]/),
    message: '不得直接 invoke —— 出站请求一律经 services/（协议切换只改那一层）',
  },
  // M-004 registry / schemas 不得 import 组件（*Renderers.ts 是刻意的装配点）
  componentImport: {
    rule: 'M-004',
    severity: 'error',
    test: (c) => c.match(/from\s+['"][^'"]*\.vue['"]/),
    message: 'registry/ 与 schemas/ 不得 import 组件（装配点 *Renderers.ts 除外）',
    skip: (file) => /Renderers\.ts$/.test(file),
  },
  // M-005 schemas 不得反向依赖 registry / components / composables
  schemaReverseDep: {
    rule: 'M-005',
    severity: 'error',
    test: (c) =>
      c.match(
        /from\s+['"][^'"]*(?:@\/(?:registry|components|composables)|\.\.\/(?:registry|components|composables))/,
      ),
    message: '数据契约不得依赖 registry / components / composables（零呈现依赖，防循环引用）',
  },
  // M-006 组件不得用字面量比较消息词表
  vocabLiteral: {
    rule: 'M-006',
    severity: 'error',
    test: (c) =>
      c.match(new RegExp(`\\.(?:status|role|type)\\s*(?:===|!==)\\s*['"](?:${MESSAGE_VOCAB})['"]`)),
    message: '不得用字面量比较消息词表 —— 用 schemas/chat_message 的常量或 registry/messageTypes 的判定',
  },
  /**
   * M-007 地址段常量的**定义权**只在 `schemas/vdfs.ts`
   *
   * M-002 只匹配 `.vdfs` 字面量，于是用 `const VDFS_SESSION_DIR = 'session'` +
   * `vdfsJoin(...)` 拼装的写法能绕过它——那正是 `schemas/vdfs.ts` 现在在做的事，
   * 且它是**有意的例外**（provider 的段名是私有知识，前端拿到的是常量而非字面量）。
   *
   * 例外必须**收口**：同样一份常量若在别处再定义一遍，就退化成第二份真相。
   * 本规则不禁止使用（导入是允许的），只禁止**定义**。
   *
   * 只匹配**值为字符串**的常量：地址段名是字符串，而 `VDFS_PAGE_SIZE = 100`
   * 这类数值常量不是地址知识（它就在 `useVdfs.ts` 里，误报会逼人写豁免注释，
   * 一个靠豁免活着的守卫等于没有守卫）。
   *
   * **浏览器路径不判**（值以 `/` 开头）：`schemas/vdfsAddress.ts` 的
   * `VDFS_HOME_PATH = '/vdfs'` 是 vue-router 的路由前缀——那是路由的承载形式，
   * 不是 VDFS 数据地址。数据地址要么是 `<根>` 打头的展示地址，要么是裸段名 /
   * 树内相对路径，**从不以 `/` 开头**（`vdfs.md` §3.1 的 normalize_addr 会剥掉
   * 首部分隔符，且「不强加前导 `/`」）。
   */
  vdfsConstDef: {
    rule: 'M-007',
    severity: 'error',
    test: (c) => c.match(/\b(?:const|let|var)\s+(?:VDFS_[A-Z0-9_]+|vdfs[A-Z]\w*)\s*=\s*['"`](?!\/)/),
    message: '地址常量的定义权在 schemas/vdfs.ts —— 此处不得再定义一份（导入使用是允许的）',
  },
}

// ── 主流程 ───────────────────────────────────────────────────────────────
console.log('=== mechanism-audit.mjs ===')
console.log(`Root:   ${disp(repoRoot) || '.'}`)
console.log(`Strict: ${STRICT}`)
console.log()

if (!fs.existsSync(SRC)) {
  console.log(yellow(`未找到 ${disp(SRC)} —— 无可审计内容（换 --root= 指向仓库根）`))
  process.exit(0)
}

console.log('--- M-001: 组件不得解释后端 meta 字段 ---')
auditFiles(
  'tauri/src/components',
  walk(path.join(SRC, 'components'), isVueOrTs),
  [RULES.metaField],
)

console.log('--- M-002: 组件 / 组合式不得硬编码 .vdfs 地址 ---')
auditFiles(
  'tauri/src/components + tauri/src/composables',
  [...walk(path.join(SRC, 'components'), isVueOrTs), ...walk(path.join(SRC, 'composables'), isTs)],
  [RULES.vdfsAddr],
)

console.log('--- M-003: 不得直接 invoke（services/ 除外） ---')
auditFiles(
  'tauri/src（不含 services/）',
  walk(SRC, isVueOrTs).filter((f) => !f.startsWith(path.join(SRC, 'services') + path.sep)),
  [RULES.directInvoke],
)

console.log('--- M-004: registry / schemas 不得 import 组件 ---')
auditFiles(
  'tauri/src/registry + tauri/src/schemas',
  [...walk(path.join(SRC, 'registry'), isTs), ...walk(path.join(SRC, 'schemas'), isTs)],
  [RULES.componentImport],
)

console.log('--- M-005: schemas 不得依赖 registry / components / composables ---')
auditFiles('tauri/src/schemas', walk(path.join(SRC, 'schemas'), isTs), [RULES.schemaReverseDep])

console.log('--- M-006: 组件 / 组合式 / 服务层不得用字面量比较消息词表 ---')
// 范围必须覆盖 **所有消费消息词表的地方**，而不只是组件：
// 词表字面量一旦出现在 services/（如把 `status` 翻译成活动文案）或 composables/，
// 同样会在后端改词表时静默失配，而 `.vue`-only 的扫描看不见它们。
auditFiles(
  'tauri/src/components + composables + services',
  [
    ...walk(path.join(SRC, 'components'), isVueOrTs),
    ...walk(path.join(SRC, 'composables'), isTs),
    ...walk(path.join(SRC, 'services'), isTs),
  ],
  [RULES.vocabLiteral],
)

console.log('--- M-007: 地址常量只能在 schemas/vdfs.ts 定义（不禁止导入使用） ---')
auditFiles(
  'tauri/src（不含 schemas/vdfs.ts）',
  walk(SRC, isVueOrTs).filter((f) => f !== path.join(SRC, 'schemas', 'vdfs.ts')),
  [RULES.vdfsConstDef],
)

// ── 汇总 ─────────────────────────────────────────────────────────────────
console.log()
console.log('=== 汇总 ===')
if (hitsByRule.size === 0) console.log(green('  七条规则全部通过'))
else for (const [rule, n] of [...hitsByRule].sort()) console.log(`  ${rule}: ${n} 处`)
console.log(`Errors:   ${errors}`)
console.log(`Warnings: ${warnings}`)
console.log()
console.log(
  dim('  边界：判定基于去注释文本而非 AST，变量别名可绕过（见文件头「已知边界」）'),
)

if (errors > 0) process.exit(1)
if (STRICT && warnings > 0) process.exit(2)
process.exit(0)
