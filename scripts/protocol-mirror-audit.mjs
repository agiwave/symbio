#!/usr/bin/env node
/**
 * protocol-mirror-audit — 跨栈协议常量的**镜像一致性**守卫
 *
 * ## 它守的是什么
 *
 * VDFS 的地址方案由后端定义，前端必须按同一套段名拼地址、解地址。
 * 两类知识需要跨栈对齐，任一侧漂移都不会有任何测试变红，只会在运行期表现为
 * 「消息读不到 / 渲染器选错」——所以要有专门的检查。
 *
 * **A. 镜像对**（后端与前端必须**逐字相等**）：
 *   目前只剩呈现词表（`kind` / `ext`）——它们是**协议词**，前端拿它选渲染器、
 *   认目录，必须与后端一致。
 *
 * **B. 缺席检查**（前端**不得再出现**）：
 *   会话的两个地址段（挂载段 `session`、转写段 `消息`）已改为**运行期发现**
 *   （见 `tauri/src/services/vdfsScheme.ts`），前端不再持有它们。这两条保证
 *   它们不会悄悄回来——回来就是第二份真相。
 *
 * ## 它**不**声称什么
 *
 * 判定基于正则读源码，不是 AST：注释掉的常量同样会命中（缺席检查因此偏严，
 * 这符合它的意图——连注释里都不该教人写回去）。
 *
 * ## 用法
 *
 *   node scripts/protocol-mirror-audit.mjs                # 审计本仓库
 *   node scripts/protocol-mirror-audit.mjs --repo=<dir>   # 换仓库根（回归测试用）
 *
 * 退出码：0 = 全部通过；1 = 存在不一致 / 缺失 / 不应出现的常量。
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
const red = (s) => (NO_COLOR ? s : `[31m${s}[0m`)
const green = (s) => (NO_COLOR ? s : `[32m${s}[0m`)
const yellow = (s) => (NO_COLOR ? s : `[33m${s}[0m`)
const dim = (s) => (NO_COLOR ? s : `[2m${s}[0m`)

/** A. 后端与前端必须逐字相等的常量 */
const PAIRS = [
  {
    id: 'X-001',
    what: '转写列表的场景类型（前端按它发现转写段）',
    rust: { file: 'symbio/src/symbio_core/vdfs_provider.rs', name: 'VDFS_KIND_MESSAGES' },
    ts: { file: 'tauri/src/schemas/vdfs.ts', name: 'VDFS_KIND_MESSAGES' },
  },
  {
    id: 'X-002',
    what: '会话节点的 ext（前端据此选渲染器）',
    rust: { file: 'symbio/src/symbio_core/vdfs_provider.rs', name: 'VDFS_EXT_SESSION' },
    ts: { file: 'tauri/src/schemas/vdfs.ts', name: 'VDFS_EXT_SESSION' },
  },
  {
    id: 'X-003',
    what: '消息节点的 ext（前端据此选渲染器）',
    rust: { file: 'symbio/src/symbio_core/vdfs_provider.rs', name: 'VDFS_EXT_MESSAGE' },
    ts: { file: 'tauri/src/schemas/vdfs.ts', name: 'VDFS_EXT_MESSAGE' },
  },
]

/**
 * B. 前端**不得再出现**的常量。
 *
 * 会话地址段已改为运行期发现（`services/vdfsScheme.ts`）：挂载段按「挂载点声明
 * 可新建 `ext = session`」认出来，转写段按 `kind = VDFS_KIND_MESSAGES` 认出来。
 * 前端持有它们就是第二份真相——这两条防止它们悄悄回来。
 */
const ABSENT = [
  {
    id: 'X-004',
    what: '会话挂载段常量（应由 vdfsScheme 运行期发现）',
    file: 'tauri/src',
    pattern: /\bVDFS_SESSION_DIR\b/,
  },
  {
    id: 'X-005',
    what: '转写段常量（应由 vdfsScheme 按 kind 发现）',
    file: 'tauri/src',
    pattern: /\bVDFS_SEG_MESSAGES\b/,
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

/**
 * 递归列出目录下的 .ts / .vue 文件。
 *
 * **排除 `__tests__`**：与 mechanism-audit 的既有口径一致——测试持有的是
 * **协议夹具**（把地址钉成字面量正是它的职责），生产代码才不许持有。
 */
function walk(dir) {
  const out = []
  if (!fs.existsSync(dir)) return out
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const abs = path.join(dir, e.name)
    if (e.isDirectory()) {
      if (e.name === '__tests__') continue
      out.push(...walk(abs))
    } else if (/\.(ts|vue)$/.test(e.name)) {
      out.push(abs)
    }
  }
  return out
}

let errors = 0

console.log('=== protocol-mirror-audit：跨栈协议常量 ===')
console.log(dim(`    仓库根：${REPO}`))
console.log()

console.log('--- A. 镜像对（后端 ? 前端必须逐字相等）---')
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
    console.log(`  ${green('?')} ${p.id} ${p.what} = ${JSON.stringify(rustVal)}`)
    continue
  }

  errors += problems.length
  console.log(`  ${red('?')} ${p.id} ${p.what}`)
  for (const msg of problems) console.log(`      ${red(msg)}`)
  console.log(
    dim(`      后端 ${p.rust.file}::${p.rust.name} ? 前端 ${p.ts.file}::${p.ts.name}`),
  )
}

console.log()
console.log('--- B. 缺席检查（前端不得再持有会话地址段）---')
const frontendFiles = walk(path.join(REPO, 'tauri', 'src'))
for (const a of ABSENT) {
  const hits = []
  for (const f of frontendFiles) {
    const lines = fs.readFileSync(f, 'utf8').split(/\r?\n/)
    lines.forEach((line, i) => {
      if (a.pattern.test(line)) {
        hits.push(`${path.relative(REPO, f).replace(/\\/g, '/')}:${i + 1}`)
      }
    })
  }
  if (hits.length === 0) {
    console.log(`  ${green('?')} ${a.id} ${a.what}`)
    continue
  }
  errors += hits.length
  console.log(`  ${red('?')} ${a.id} ${a.what} —— 出现 ${hits.length} 处`)
  for (const h of hits.slice(0, 10)) console.log(`      ${red(h)}`)
  console.log(dim('      这两个段名由后端 provider 决定，前端应运行期发现'))
}

console.log()
console.log(`Errors: ${errors}`)

if (errors > 0) {
  console.log()
  console.log(
    yellow(
      '  A 组：两端常量必须逐字相等（改后端就要改前端镜像）。\n' +
        '  B 组：会话地址段不该出现在前端——它们是运行期发现的数据，不是常量。',
    ),
  )
  process.exit(1)
}

console.log(green(`  三组镜像一致 + 两项缺席检查通过（扫描 ${frontendFiles.length} 个前端文件）`))
process.exit(0)
