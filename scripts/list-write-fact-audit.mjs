#!/usr/bin/env node
/**
 * list-write-fact-audit — 列表写原语的**结构性**判定
 *
 * ADR-054 的目标态**已经落地**：`MessageChange` +
 * `apply`（`chat_session/write.rs`）是消息列表的唯一写入口，顺序反转成
 * 「先落事实，列表是折叠出来的」，「改了列表却没落事实」由**类型**表达不出来。
 * 因此本守卫从过渡期的「就地登记」（v1，指令行 + pending 棘轮）升级为**结构性判定**
 * （v2）——它不再问「你登记了吗」，它问「结构还对吗」。这次升级是**换职责，不是过渡**：
 * v1 钉的是存量缺口（缺口归零后确实无事可做），v2 钉的是**持续不变量**
 * 「存储里的列表 == `fold(WAL)`」的唯一入口——类型强制只管得住 `apply` 那一侧，
 * 管不住「有没有第二条路」。故本守卫是**永久守卫**（ADR-054 实施补记 4 的裁定），
 * 不随「类型到位」退役；v1 的 LWF-003 / LWF-004 两码随之退役。
 *
 * ## 判定口径
 *
 * 扫描 session 插件的非测试 Rust 源码：
 *
 * 1. **节点级写原语**（`append_messages`）只能在 `chat_session/write.rs` 内被调用
 *    ——那里是原语的实现与 `apply` 的唯一汇聚点。在外面出现 ⇒ **LWF-001**：
 *    绕过了唯一写入口，落不落事实全凭自觉，正是 ADR-054 要消灭的形态。
 * 2. **列表级取代原语**（`replace_all`）出了 `write.rs`，必须在 ±NEAR 行内带着
 *    **寄存锚**（`consolidated = Some(...)` 的真实赋值，不是注释）：压缩现场不持
 *    写者令牌，取代事实只能寄存给下一个 `execute_turn` 落格。没有锚 ⇒ **LWF-002**
 *    ——一份被取代的前缀在事实源里没有下文，切主读后它原地复活。
 * 3. **事实落格函数**（`message_events::{added,edited,removed}`）只能在 `write.rs`
 *    被调用——它们是 `apply` 的实现细节。在外面出现 ⇒ **LWF-001**（第二个写入口）。
 *
 * 锚判定**剥掉注释**再匹配：注释里写「我会寄存」不算寄存——判据曾经被自己的
 * 回归测试抓出来过一次同义反复（登记声明的事实符号拿登记行自己当「在现场」的
 * 证据），v2 的锚必须是真代码。
 *
 * ## 为什么回归测试仍然要带
 *
 * 结构判定的失效形态是**判据自身的钝化**：窗口算错、正则过宽、测试文件漏排除，
 * 守卫都会静默地变成永远绿。`list-write-fact-audit.test.mjs` 用构造出的好/坏样本
 * 逐条钉住每个分支（含窗口边界）。
 *
 * 用法：
 *   node scripts/list-write-fact-audit.mjs              # 审计 symbio/src/plugins/session
 *   node scripts/list-write-fact-audit.mjs ROOT=<dir>   # 审计指定目录（回归测试用）
 *
 * 退出码：0 = 通过；1 = 发现违规。
 * 平台无关（纯 Node，不依赖 bash / ripgrep），带回归测试（list-write-fact-audit.test.mjs）。
 */

// 判据码命名空间（登记表 docs/006-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns LWF 列表写入口事实
// @codes LWF-001 LWF-002

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

/** 节点级写原语：只能经 `MessageChange::apply` 触达。 */
const NODE_PRIMITIVES = ['append_messages']
/** 列表级取代原语：出 `write.rs` 必须带寄存锚。 */
const SNAPSHOT_PRIMITIVES = ['replace_all']
/** 事实落格函数：`apply` 的实现细节，不得有第二个调用方。 */
const FACT_OWNERS = ['added', 'edited', 'removed']

const ENTRY_CALL_RE = new RegExp(`\\b(?:${[...NODE_PRIMITIVES, ...SNAPSHOT_PRIMITIVES].join('|')})\\s*\\(`)
const FN_DEF_RE = new RegExp(`\\bfn\\s+(?:${[...NODE_PRIMITIVES, ...SNAPSHOT_PRIMITIVES].join('|')})\\b`)
const FACT_OWNER_CALL_RE = new RegExp(`\\bmessage_events::(?:${FACT_OWNERS.join('|')})\\s*\\(`)
/** 寄存锚：取代事实的**真实寄存赋值**（注释里提一嘴不算）。 */
const PARK_RE = /\bconsolidated\s*=\s*Some\s*\(/
/** 端口转发不是原语调用：`self.port.replace_all(...)` 只是把动作交给地址面执行。 */
const PORT_CALL_RE = /\bport\s*\.\s*replace_all\s*\(/
/** 行内豁免（本行或前一行；理由不可为空）——只给「地址面中继」这类无锚可持的层。 */
const WAIVER_RE = /\/\/\s*list-write-fact-allow\s+LWF-002\s*:\s*(\S.*)$/

/** 唯一写入口所在文件（相对扫描根的路径以此为尾）。 */
const WRITER_SUFFIX = `chat_session${path.sep}write.rs`

/** 寄存锚与调用点的最大距离（行）。压缩现场的寄存赋值紧贴调用点（当前 9 行）；
 *  取窄不取宽：锚离得远，说明它多半不是这一次调用的锚。 */
const NEAR = 20

/** 递归收集 `.rs`（测试文件排除：测试是排演驱动方，不受写入口纪律约束）。 */
function walk(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walk(p, out)
    else if (e.name.endsWith('.rs') && !e.name.endsWith('.test.rs')) out.push(p)
  }
  return out
}

/** 一个文件里的原语调用与事实落格调用（行号；**注释剥除后**匹配）。 */
function parseFile(text) {
  const calls = []
  const factCalls = []
  text.split('\n').forEach((line, i) => {
    const code = line.split('//')[0]
    if (ENTRY_CALL_RE.test(code) && !FN_DEF_RE.test(code)) calls.push({ line: i + 1, text: line.trim() })
    if (FACT_OWNER_CALL_RE.test(code)) factCalls.push({ line: i + 1, text: line.trim() })
  })
  return { calls, factCalls }
}

function audit(root, prefix = '') {
  const violations = []

  for (const file of walk(root)) {
    const rel = [prefix, path.relative(root, file).split(path.sep).join('/')].filter(Boolean).join('/')
    const text = fs.readFileSync(file, 'utf8')
    const lines = text.split('\n')
    const inWriter = file.endsWith(WRITER_SUFFIX)
    const { calls, factCalls } = parseFile(text)

    for (const call of calls) {
      if (inWriter) continue
      const fn = ENTRY_CALL_RE.exec(call.text)?.[0].replace(/\s*\(.*$/, '')
      if (NODE_PRIMITIVES.includes(fn)) {
        violations.push(
          `LWF-001 ${rel}:${call.line}  节点级写原语 \`${fn}\` 绕过了唯一写入口：${call.text}\n` +
            '        改走 `MessageChange` + `apply`（chat_session/write.rs）——列表变更只有经它才落事实',
        )
        continue
      }
      // 端口转发（`self.port.replace_all`）不是原语本身：真正落库的动作在地址面
      // 另一端执行，锚的义务属于**逻辑调用方**（压缩现场）。
      if (PORT_CALL_RE.test(call.text)) continue
      // 地址面中继无锚可持：它拿不到 `SessionContext`，寄存发生在调用方。
      if (WAIVER_RE.test(call.text) || (call.line > 1 && WAIVER_RE.test(lines[call.line - 2]))) continue
      // 取代原语：找寄存锚（剥注释的窗口——注释里提「寄存」不算寄存）。
      const lo = Math.max(0, call.line - 1 - NEAR)
      const hi = Math.min(lines.length, call.line - 1 + NEAR)
      const window = lines.slice(lo, hi).map((l) => l.split('//')[0]).join('\n')
      if (!PARK_RE.test(window)) {
        violations.push(
          `LWF-002 ${rel}:${call.line}  取代原语 \`replace_all\` 没有寄存锚（±${NEAR} 行内找不到 ` +
            '`consolidated = Some(...)` 的真实赋值）：' + call.text + '\n' +
            '        压缩现场不持写者令牌，取代事实必须寄存给 `SessionContext::consolidated`、由下一个 `execute_turn` 落格',
        )
      }
    }

    for (const call of factCalls) {
      if (!inWriter) {
        violations.push(
          `LWF-001 ${rel}:${call.line}  事实落格函数在 \`write.rs\` 之外被调用（第二个写入口）：${call.text}\n` +
            '        `message_events::{added,edited,removed}` 是 `apply` 的实现细节——从外面走 `MessageChange` + `apply`',
        )
      }
    }
  }

  return { violations }
}

const rootArg = process.argv.find((a) => a.startsWith('ROOT='))
// 写原语与中继在 session；取代原语的**逻辑调用点**（压缩现场）在 loop——
// 两棵树一起扫，锚才真的有人看。`ROOT=` 供回归测试指向夹具树。
const roots = rootArg
  ? [{ dir: path.resolve(rootArg.slice(5)), prefix: '' }]
  : ['session', 'loop'].map((p) => ({
      dir: path.join(repoRoot, 'symbio', 'src', 'plugins', p),
      prefix: p,
    }))
for (const { dir } of roots) {
  if (!fs.existsSync(dir)) {
    console.error(red(`✗ 扫描根不存在：${dir}`))
    process.exit(1)
  }
}
const violations = roots.flatMap(({ dir, prefix }) => audit(dir, prefix).violations)
if (violations.length > 0) {
  console.error(red(`✗ 列表写原语结构扫描：${violations.length} 处违规`))
  for (const v of violations) console.error('  ' + v)
  process.exit(1)
}
console.log(
  green(
    '✓ 列表写原语结构扫描：节点级经 apply、取代带寄存锚、落格函数不出 write.rs（ADR-054 类型强制的守卫面）',
  ),
)
