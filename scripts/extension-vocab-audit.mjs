#!/usr/bin/env node
/**
 * extension-vocab-audit — 扩展面不说容器词（EX-008，D-252）
 *
 * ## 它治的病
 *
 * `turn`（轮 / 幕）是**宿主的容器概念**。它一旦出现在 `symbio-plugin-api` 的端口形状里
 * （类型名 / trait 名 / 枚举变体 / 字段名 / `pub fn` 形参），每个能力包就被迫认识宿主的
 * 时间单位：能力按 `turn` 算幂等键、按 `turn` 数调用，环的边界一变它就跟着错——
 * 已经退役的「幕」就是这样重新发给每张能力表的。D-252 的三条整改（端口只收不透明词干、
 * 开关走 `session.config` 事实、类型名去容器词）把存量清干净了；本判据防它长回来。
 *
 * 与 `core-change-audit` / `EX-002` 互补：那两条判**提交的意图面**（这次改动留没留理由、
 * 有没有动架构树），本判据判**结果面**——哪怕分十个提交、每个都写了漂亮理由，
 * 只要端口形状上有一格 `turn`，门禁照样红。
 *
 * ## 判什么（形状，不是措辞）
 *
 * 与 `SC-006` 同一条纪律：只认**非注释、非字符串**代码里的标识符。注释与文档里说
 * 「这一轮」不红——那是叙述宿主的行为；端口上有一格 `turn` 才红。`return` 一族关键字
 * 不算（它是控制流，不是容器词）。
 *
 * | 范围 | 处置 |
 * |---|---|
 * | `crates/symbio-plugin-api/src` 下的 `.rs`（递归，非生成、非测试） | 逐文件扫标识符与 CJK 容器字（轮 / 幕） |
 * | `*.test.rs` | 不判——测试可以把宿主词汇摆上桌来断言「交不进去」 |
 * | 生成表 `events.rs` / `routes.rs` / `view_names.rs`（豁免①） | 不判——它们是各插件 `PLUGIN.yml` 清单的镜像，`turn.*` / `Entity::Turn` 是控制面**声明的事实名**，不是端口形状（生成器 `gen-facts --check` 保证内容与清单一致） |
 *
 * D-252 的另两处有名豁免（② loop 的 `t{turn}` 词干生成、③ core 的 `Event.turn` 信封
 * 字段与 I1 / I3 不变量）**不在本判据的扫描范围内**——它们分别住在 `symbio-plugin-loop`
 * 与 `symbio-core`，本判据只扫扩展面。它们的名字登记在 D-252 里，不在这里复述成
 * 第二张表。
 *
 * 豁免通道（`EXEMPTIONS`）：**逐条** `{ path, token, why }`，路径与标识符都**精确等值**
 * （不是前缀、不是 glob），每条必须有理由正文。一条不解释自己的豁免就是永久后门，
 * 通配符在登记那一刻就被判红。现状为零条——存量在 D-252 整改批里已经清干净。
 *
 * 用法：node scripts/extension-vocab-audit.mjs [--root=<目录>]
 * 退出码：0 = 通过；1 = 有违规或豁免清单本身不成立；2 = 范围读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns EX 能力全面拆分（名册处置与迁移边界）
// @codes EX-008

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green } from './color.mjs'

const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const PLUGIN_API_SRC = 'crates/symbio-plugin-api/src'

/**
 * 豁免①：生成表。文件头都写着「由 `scripts/facts/*.mjs` 生成，请勿手改」——它们
 * 是清单的镜像，容器词出现在那里是控制面声明的事实名（D-252 有名豁免①）。
 * **逐个文件名登记**，不写通配：新出现的生成表必须在这里留下名字，否则它连
 * 「我是生成物」都不声明就吃掉了豁免。
 */
export const GENERATED_FILES = [
  // scripts/facts/events-rs.mjs：PLUGIN.yml `shared_events` 的镜像（D-204）
  'events.rs',
  // scripts/facts/routes-rs.mjs：PLUGIN.yml `routes` 的镜像
  'routes.rs',
  // scripts/facts/views-rs.mjs：PLUGIN.yml `shared_views` 的镜像（D-221）
  'view_names.rs',
]

/**
 * 逐条豁免：`{ path, token, why }`。`path` 是相对仓库根的精确路径，`token` 是精确
 * 标识符（含大小写），`why` 必须非空。现状为零条；出现新的豁免时它必须同时指向
 * 一条能打开的决策（D-252 的豁免在扫描范围之外，不需要条目）。
 */
export const EXEMPTIONS = [
  {
    path: 'crates/symbio-plugin-api/src/lib.rs',
    token: 'TurnToolCallInfo',
    why:
      'D-253：与 `TurnOutput` 同族——它是模型端口的**签名类型**（`ModelProvider` 的工具调用形状），' +
      '模型插件必须能命名它；D-252 拦的是「配置 / 开关类型进能力面」，不是端口签名本身。',
  },
  {
    path: 'crates/symbio-plugin-api/src/lib.rs',
    token: 'TurnOutput',
    why:
      'D-253：它是 `ModelProvider` 的签名类型，模型插件必须能命名它；D-252 拦的是「配置 / 开关类型' +
      '进能力面」，不是端口签名本身。改名要动 wire，收益只有脚本好看一点。',
  },
]

/**
 * 豁免清单自身的形状判据：一条不解释自己的豁免就是永久后门，通配就是无限张后门。
 * 路径与标识符都不许含 glob 字符、不许为空、理由正文不许为空。
 */
export function validateExemptions(entries) {
  const errors = []
  for (const e of entries) {
    const where = e?.path ?? '(无 path)'
    if (typeof e?.path !== 'string' || !e.path.trim()) {
      errors.push(`${where}：豁免必须有精确的文件路径`)
      continue
    }
    if (typeof e?.token !== 'string' || !e.token.trim()) {
      errors.push(`${e.path}：豁免必须有精确的标识符（不是前缀、不是正则）`)
      continue
    }
    if (/[*?[\]]/.test(e.path) || /[*?[\]]/.test(e.token)) {
      errors.push(`${e.path}：豁免不许用通配——一条通配豁免就是无限张后门（EX-008）`)
      continue
    }
    if (typeof e?.why !== 'string' || !e.why.trim()) {
      errors.push(`${e.path}：豁免没有理由正文——一条不解释自己的豁免就是永久后门`)
    }
  }
  return errors
}

/**
 * 剥掉 Rust 的注释与字符串字面量（含嵌套块注释、原始字符串、转义）。
 * 判据读的是剩下的**代码**：`SC-006` 同一条纪律——注释里说「这一轮」不是形状。
 */
export function stripRustCode(src) {
  let out = ''
  let i = 0
  const n = src.length
  while (i < n) {
    const c = src[i]
    if (c === '/' && src[i + 1] === '/') {
      while (i < n && src[i] !== '\n') i++
      continue
    }
    if (c === '/' && src[i + 1] === '*') {
      let depth = 1
      i += 2
      while (i < n && depth > 0) {
        if (src[i] === '/' && src[i + 1] === '*') {
          depth++
          i += 2
        } else if (src[i] === '*' && src[i + 1] === '/') {
          depth--
          i += 2
        } else i++
      }
      out += ' '
      continue
    }
    if (c === 'r' && (src[i + 1] === '"' || src[i + 1] === '#')) {
      let j = i + 1
      let hashes = 0
      while (src[j] === '#') {
        hashes++
        j++
      }
      if (src[j] === '"') {
        const close = '"' + '#'.repeat(hashes)
        const end = src.indexOf(close, j + 1)
        i = end < 0 ? n : end + close.length
        out += ' '
        continue
      }
    }
    if (c === '"') {
      i++
      while (i < n && src[i] !== '"') {
        if (src[i] === '\\') i++
        i++
      }
      i++
      out += ' '
      continue
    }
    if (c === "'") {
      const m = /^'(\\.|[^\\'])'/.exec(src.slice(i, i + 12))
      if (m) {
        i += m[0].length
        out += ' '
        continue
      }
    }
    out += c
    i++
  }
  return out
}

/** `return` 一族关键字不算容器词——它是控制流，恰好包含 `turn` 子串。 */
const RETURN_FAMILY = /^(return|returns|returned|returning)$/i

/**
 * 纯判定：一行剥过的代码里，容器词出现在**哪些标识符**上。
 * `turn` 大小写不敏感（`TurnSettings` 与 `turn_counter` 同罪）；
 * CJK 容器字（轮 / 幕）整字判——Rust 标识符容得下它们，不拦就是留了一条写法上的后门。
 */
export function findContainerWordTokens(line) {
  const hits = []
  for (const m of line.matchAll(/[A-Za-z_][A-Za-z0-9_]*/g)) {
    if (/turn/i.test(m[0]) && !RETURN_FAMILY.test(m[0])) hits.push(m[0])
  }
  for (const m of line.matchAll(/[轮幕]/g)) hits.push(m[0])
  return hits
}

/** 环境变量覆盖豁免（回归测试注入口）；与仓库数据解耦，与 `CORE_VOCAB_BASELINE` 同一形状。 */
function exemptionsInForce() {
  const raw = process.env.EXTENSION_VOCAB_EXEMPTIONS
  return raw === undefined ? EXEMPTIONS : JSON.parse(raw)
}

function isExempt(entries, rel, token) {
  return entries.some((e) => e.path === rel && e.token === token)
}

function run(root) {
  const dir = path.join(root, PLUGIN_API_SRC)
  if (!fs.existsSync(dir)) {
    console.log(`${red('[ERROR]')} extension-vocab-audit：范围读不出——缺少 ${PLUGIN_API_SRC}（扩展面只剩核心面了吗？）`)
    return 2
  }

  const exemptions = exemptionsInForce()
  const shapeErrors = validateExemptions(exemptions)
  if (shapeErrors.length) {
    for (const e of shapeErrors) console.log(`${red('[ERROR]')} EX-008 豁免清单：${e}`)
    return 1
  }

  const files = []
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (!e.isFile() || !e.name.endsWith('.rs')) continue
    if (e.name.endsWith('.test.rs')) continue
    if (GENERATED_FILES.includes(e.name)) continue
    files.push(e.name)
  }
  files.sort()

  const errors = []
  let scanned = 0
  for (const name of files) {
    const rel = `${PLUGIN_API_SRC}/${name}`
    const stripped = stripRustCode(fs.readFileSync(path.join(dir, name), 'utf8'))
    const lines = stripped.split('\n')
    scanned++
    for (const [idx, line] of lines.entries()) {
      for (const token of findContainerWordTokens(line)) {
        if (isExempt(exemptions, rel, token)) continue
        errors.push(
          `${rel}:${idx + 1}  标识符 \`${token}\` 带容器词——端口形状不得认识宿主的容器（D-252）：` +
            '幂等只需要宿主给的不透明词干，开关走 `session.config` 事实；' +
            '确属生成事实名请把文件登记进 `GENERATED_FILES`，确属形状豁免请逐条写 `{ path, token, why }`',
        )
      }
    }
  }

  if (errors.length) {
    for (const e of errors) console.log(`${red('[ERROR]')} EX-008 ${e}`)
    console.log(`\n扩展面词汇：${errors.length} 处容器词进了端口形状`)
    return 1
  }
  console.log(
    green(
      `✓ EX-008 扩展面零容器词：symbio-plugin-api 的 ${scanned} 个非生成生产文件里，` +
        '端口形状（类型 / trait / 变体 / 字段 / 形参）没有 turn / 轮 / 幕' +
        `（生成表 ${GENERATED_FILES.length} 个与 *.test.rs 不判，豁免 ${exemptions.length} 条逐名）`,
    ),
  )
  return 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exit(run(ROOT))
}
