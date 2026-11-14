#!/usr/bin/env node
/**
 * port-type-coupling-audit — 通用端口契约不绑模型协议类型（D-447 ②）
 *
 * ## 它判什么
 *
 * | 码 | 判据 | 范围 |
 * |---|---|---|
 * | PTC-001 | 通用端口契约 / 驱动 / 环事实三个文件不得出现 `PromptMessage`（`Vec<PromptMessage>` 由同一条命中） | `crates/symbio-sdk/src/capability/port.rs`、`crates/symbio-plugin-api/src/drive.rs`、`crates/symbio-plugin-loop/src/ring/facts.rs` |
 * | PTC-002 | 通用端口契约四个文件不得出现 `needs_wording_generation` | 上述三个 + `crates/symbio-plugin-api/src/port.rs` |
 *
 * 违规**逐条点名 `文件:行号`**。退出码：0 = 无耦合；1 = 有耦合，或判据没有输入。
 *
 * ## 为什么
 *
 * D-444 / D-446 把 `PortReply` 定成「不透明承载」：结果先成为事实、再投影成模型消息。
 * 通用端口契约若出现 `PromptMessage`（模型侧消息类型）或 `needs_wording_generation`
 * （模型侧措辞判定），就反向依赖了模型协议——加一种模型协议要改共享层，D-440「端口名
 * 不进声明表」的零成本加端口随之作废。这条判据把「类型解绑」从散文落成可重跑的文本
 * 扫描，与 `loop-purity-audit` 同一立场：**不靠注释声明，只认真实代码行**。
 *
 * ## PTC-002 的边界与未闭合项（2026-11-11 team-lead 修正）
 *
 * PTC-002 **不扫整个 `crates/symbio-sdk/src/`**，只扫上面四个通用端口契约文件；与
 * PTC-001 共用同一份常量表（前三个），第 4 个 `port.rs` 只进措辞判定面。`sdk/dialog`
 * 里 `Verdict::needs_wording_generation` 是**会话域判定**（消费方是 compose / session
 * 两个插件），把它迁出需要改成「模型插件落事实、控制面读事实」，属于**行为闭包迁移**，
 * 本轮明确不做——在报告与裁决里如实标注**未闭合**。
 *
 * 留下一条永远红的门禁是更坏的结果：这里收窄的是**判据边界**（本就只该覆盖通用端口
 * 契约），不是为消红放宽形状判据。`PromptMessage` 那条仍按原要求扫那三个通用端口
 * 契约文件，一条没动。
 *
 * ## 已知边界（写在绿灯里，别把它当证明）
 *
 * - **纯文本判据，不做 Rust 类型解析**：`use ... as P` 起别名、`macro_rules!` 拼出的
 *   名字、字符串字面量里的同名文本都不在语义内（别名会漏、字符串会误报；本仓当前
 *   两类皆 0）。要关掉别名这个缝得做类型解析，那是 `arch-shape-audit` 那一层的事。
 * - **剥 `//` 行注释（含 `///` / `//!`）与 C 风格块注释**：doc comment 里写「`PromptMessage`
 *   已移出」是叙述，不是耦合，不该把一次已完成的解耦判成红；反过来，在真实代码行
 *   上面补一句「已删除」也骗不过本判据——它只认剩下的代码。字符串字面量里的注释
 *   起始符会被当成注释起点（本仓当前 0 处影响本判据）。
 * - **PTC-001 只扫三个、PTC-002 扫四个**：`symbio-plugin-api/src/port.rs` 里的
 *   `PromptMessage` 不在 PTC-001 面内（按原要求）。要覆盖应立新判据，不是悄悄把一张
 *   表改宽。两个判据都不再遍历目录，故 `sdk/dialog/**` 与其测试文件整体在扫描面外。
 * - **单个具名文件不存在只给提示、不判红**（并行解耦期间文件可能被搬走）；但
 *   PTC-001 三个全不在 / PTC-002 四个全不在都按「读不出 ⇒ 红」处理，不许
 *   「没什么可判 ⇒ 通过」。
 * - **当下状态**：`PromptMessage` 已从三个契约文件消除（并行解耦成果）；
 *   `sdk/dialog` 的 `needs_wording_generation` 仍在、但在本判据扫描面外（未闭合项）。
 *   按 D-447 ②「不为消红放宽判据」，判据边界不因某处绿 / 红而动。
 *
 * 用法：node scripts/audits/port-type-coupling-audit.mjs [--root=<仓库根>]
 * 退出码：0 = 通过；1 = 发现耦合或判据没有输入。
 */
// @gate guard

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, dim } from '../color.mjs'
import { blankComments as blankSourceComments } from '../rust-scan.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const rootArg = process.argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg ? path.resolve(rootArg.slice('--root='.length)) : path.resolve(scriptDir, '..', '..')

/** PTC-001：通用端口契约 / 驱动 / 环事实三个具名文件（相对 ROOT）。 */
const PORT_CONTRACT_FILES = [
  'crates/symbio-sdk/src/capability/port.rs',
  'crates/symbio-plugin-api/src/drive.rs',
  'crates/symbio-plugin-loop/src/ring/facts.rs',
]
/** PTC-002：同一份通用端口契约表 + plugin-api 的 port.rs（共四个文件）。 */
const WORDING_SCOPE_FILES = [
  ...PORT_CONTRACT_FILES,
  'crates/symbio-plugin-api/src/port.rs',
]

/** 模型协议类型：`Vec<PromptMessage>` 也由这一条命中（`PromptMessage` 是其子串）。 */
const MODEL_MESSAGE_TYPE = /\bPromptMessage\b/
/** 模型侧措辞判定：不进通用端口契约。 */
const WORDING_VERDICT = /\bneeds_wording_generation\b/

/** 注释剥离：唯一真源 rust-scan.mjs（保留行结构，返回按行数组）。 */
const blankComments = (text) => blankSourceComments(text).join('\n')

/** 在一份文件里逐行找 `re`；返回 `{ rel, line, text }`（`text` 取原文，供点名展示）。 */
function hitsIn(rel, abs, re) {
  const text = fs.readFileSync(abs, 'utf8').replace(/\r\n/g, '\n')
  const original = text.split('\n')
  const stripped = blankComments(text).split('\n')
  const out = []
  for (let i = 0; i < stripped.length; i++) {
    if (re.test(stripped[i])) {
      out.push({ rel, line: i + 1, text: (original[i] ?? stripped[i]).trim() })
    }
  }
  return out
}

function main() {
  const failures = []
  const notes = []

  /** 扫一份具名文件表；返回实际读到的文件数（缺文件只记提示，不判红）。 */
  const scan = (code, files, re, label) => {
    let scanned = 0
    for (const rel of files) {
      const abs = path.join(ROOT, rel)
      if (!fs.existsSync(abs)) {
        notes.push(`${code} 文件不存在（跳过）：${rel}`)
        continue
      }
      scanned++
      for (const h of hitsIn(rel, abs, re)) {
        failures.push(`${code} ${h.rel}:${h.line}  ${label}：${h.text}`)
      }
    }
    return scanned
  }

  const c1 = scan('PTC-001', PORT_CONTRACT_FILES, MODEL_MESSAGE_TYPE, '通用端口契约出现模型协议类型 PromptMessage')
  const c2 = scan('PTC-002', WORDING_SCOPE_FILES, WORDING_VERDICT, '通用端口契约出现模型措辞判定 needs_wording_generation')

  // 读不出 ⇒ 红（不许「没什么可判 ⇒ 通过」）。
  if (c1 === 0) failures.push('PTC-001 读不出：三个具名端口 / 驱动 / 环事实文件都不存在——判据没有输入')
  if (c2 === 0) failures.push('PTC-002 读不出：四个通用端口契约文件都不存在——判据没有输入')

  console.log('--- port-type-coupling-audit: 通用端口契约不绑模型协议类型（PTC-001/002）---')
  for (const n of notes) console.log(dim(`    ${n}`))
  if (failures.length > 0) {
    console.log(red(`  ✗ port-type-coupling-audit 红：${failures.length} 处类型耦合 / 读不出`))
    for (const f of failures) console.log(red(`    ${f}`))
    console.log(dim('    D-447 ②：模型协议类型 / 措辞判定不进通用端口契约；判据边界不因消红放宽。'))
    process.exit(1)
  }
  console.log(
    green(
      `  ✓ port-type-coupling-audit 通过：PTC-001 ${c1} 份 + PTC-002 ${c2} 份通用端口契约文件里 ` +
        '0 处模型协议类型 / 措辞判定耦合',
    ),
  )
  process.exit(0)
}

main()
