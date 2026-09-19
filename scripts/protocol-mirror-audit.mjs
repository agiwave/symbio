#!/usr/bin/env node
/**
 * protocol-mirror-audit — 跨栈协议常量的**镜像一致性**守卫
 *
 * ## 它守的是什么
 *
 * VDFS 的地址方案由后端定义，前端必须按同一套段名拼地址、解地址。
 * `tauri/src/schemas/vdfs.ts` 的文件头注释写着「地址的拼与解必须成对，
 * 两边各只有一份实现」，但**「成对」这件事本身没有任何检查**——
 * 后端把 `SEG_MESSAGES` 从 `"消息"` 改成别的，前端这份 `'消息'` 不会有任何
 * 测试变红，只会在运行期表现为「消息读不到 / 事件路由不上」。
 *
 * 本脚本把「两边必须同源」从散文变成可执行检查：后端常量与前端常量
 * **取值必须逐字相等**，任一侧缺失或改值即失败。
 *
 * ## 它**不**声称什么
 *
 * 它不消除常量本身（那需要后端把地址作为数据下发，见
 * `docs/archive/frontend-mechanization-review-2026-09-19.md` §7 的 P2-11）。
 * 它只保证：这份镜像**不可能悄悄漂移**。这比「在规范里写一条例外」强——
 * 例外会被后人当成许可，检查会在后人改错时报警。
 *
 * ## 用法
 *
 *   node scripts/protocol-mirror-audit.mjs                # 审计本仓库
 *   node scripts/protocol-mirror-audit.mjs --repo=<dir>   # 换仓库根（回归测试用）
 *
 * 退出码：0 = 全部一致；1 = 存在不一致或缺失。
 */

import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRepo = path.resolve(scriptDir, '..')

function argValue(name) {
  const hit = process.argv.find((a) => a.startsWith(`--${name}=`))
  return hit ? hit.slice(name.length + 3) : null
}

const REPO = argValue('repo') ? path.resolve(argValue('repo')) : defaultRepo

const NO_COLOR = process.env.NO_COLOR === '1'
const red = (s) => (NO_COLOR ? s : `[31m${s}[0m`)
const green = (s) => (NO_COLOR ? s : `[32m${s}[0m`)
const yellow = (s) => (NO_COLOR ? s : `[33m${s}[0m`)
const dim = (s) => (NO_COLOR ? s : `[2m${s}[0m`)

/**
 * 受检的镜像对。
 *
 * 只列**后端定义、前端必须逐字跟随**的常量。`VDFS_ROOT` 之类属于前端自持的
 * 地址代数，后端没有对应常量，不在本表内。
 */
const PAIRS = [
  {
    id: 'X-001',
    what: '会话挂载名（`.vdfs/<session>`）',
    rust: { file: 'symbio/src/symbio_core/ids.rs', name: 'PLUGIN_SESSION' },
    ts: { file: 'tauri/src/schemas/vdfs.ts', name: 'VDFS_SESSION_DIR' },
  },
  {
    id: 'X-002',
    what: '会话转写列表的路径段',
    rust: { file: 'symbio/src/plugins/session/plugin/nodes.rs', name: 'SEG_MESSAGES' },
    ts: { file: 'tauri/src/schemas/vdfs.ts', name: 'VDFS_SEG_MESSAGES' },
  },
  {
    id: 'X-003',
    what: '会话节点的 ext（前端据此选渲染器）',
    rust: { file: 'symbio/src/symbio_core/vdfs_provider.rs', name: 'VDFS_EXT_SESSION' },
    ts: { file: 'tauri/src/schemas/vdfs.ts', name: 'VDFS_EXT_SESSION' },
  },
  {
    id: 'X-004',
    what: '消息节点的 ext（前端据此选渲染器）',
    rust: { file: 'symbio/src/symbio_core/vdfs_provider.rs', name: 'VDFS_EXT_MESSAGE' },
    ts: { file: 'tauri/src/schemas/vdfs.ts', name: 'VDFS_EXT_MESSAGE' },
  },
]

/** Rust：`pub const NAME: &str = "v"` / `pub(crate) const NAME: &str = "v"` */
function extractRust(src, name) {
  const re = new RegExp(`\\bconst\\s+${name}\\s*:\\s*&str\\s*=\\s*"([^"]*)"`)
  const m = src.match(re)
  return m ? m[1] : null
}

/** TS：`export const NAME = 'v'`（本项目一律单引号，见 style-audit） */
function extractTs(src, name) {
  const re = new RegExp(`\\bconst\\s+${name}\\s*(?::[^=]+)?=\\s*'([^']*)'`)
  const m = src.match(re)
  return m ? m[1] : null
}

let errors = 0

console.log('=== protocol-mirror-audit：跨栈协议常量镜像一致性 ===')
console.log(dim(`    仓库根：${REPO}`))
console.log()

for (const p of PAIRS) {
  const rustPath = path.join(REPO, p.rust.file)
  const tsPath = path.join(REPO, p.ts.file)

  let rustVal = null
  let tsVal = null
  const problems = []

  if (!fs.existsSync(rustPath)) {
    problems.push(`后端文件不存在：${p.rust.file}`)
  } else {
    rustVal = extractRust(fs.readFileSync(rustPath, 'utf8'), p.rust.name)
    if (rustVal === null) problems.push(`后端未找到常量 ${p.rust.name}`)
  }

  if (!fs.existsSync(tsPath)) {
    problems.push(`前端文件不存在：${p.ts.file}`)
  } else {
    tsVal = extractTs(fs.readFileSync(tsPath, 'utf8'), p.ts.name)
    if (tsVal === null) problems.push(`前端未找到常量 ${p.ts.name}`)
  }

  if (rustVal !== null && tsVal !== null && rustVal !== tsVal) {
    problems.push(`取值不一致：后端 "${rustVal}" ≠ 前端 '${tsVal}'`)
  }

  if (problems.length === 0) {
    console.log(`  ${green('✓')} ${p.id} ${p.what} = ${JSON.stringify(rustVal)}`)
    continue
  }

  errors += problems.length
  console.log(`  ${red('✗')} ${p.id} ${p.what}`)
  for (const msg of problems) console.log(`      ${red(msg)}`)
  console.log(
    dim(
      `      后端 ${p.rust.file}::${p.rust.name} ↔ 前端 ${p.ts.file}::${p.ts.name}`,
    ),
  )
}

console.log()
console.log(`Errors: ${errors}`)

if (errors > 0) {
  console.log()
  console.log(
    yellow(
      '  两端常量必须逐字相等：改后端段名就要改前端镜像（本守卫就是为此存在的）。',
    ),
  )
  process.exit(1)
}

console.log(green('  四组跨栈常量全部一致'))
process.exit(0)
