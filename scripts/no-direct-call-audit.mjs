#!/usr/bin/env node
/**
 * no-direct-call-audit — 无直连静态扫描（v2 阶段 S6 第 14 步，
 * [plan/04 §3](docs/plan/04-工程落地.md) 出口判据「直连调用 0」；
 * [roadmap/S08 §5/§6](docs/plan/roadmap/S08-多主体与对等承诺.md) 验收 2）。
 *
 * ## 规则（I1 的直接推论：任意两主体无直连——一切经事实源）
 *
 * 主体到主体的**直接调用**前提是**持有对方的句柄**。本扫描禁止两种形态：
 *
 *   - NDC-001：主体类型名出现在其定义域（`symbio_core/actors`）、根重导出
 *     （`symbio_core/mod.rs`）或**测试文件**（`*.test.rs`——测试是排演驱动方，
 *     组装主体 + Store 正是 S1 彩排的受认可形态，不是运行时路径）之外。
 *     注释不计（文档提到名字 ≠ 对象图边）。运行时代码能拿到主体 ⇒ 持有即可直连。
 *   - NDC-002：**任何文件**（含定义域与测试）都禁止 `Arc<P>` / `Rc<P>` /
 *     `Box<dyn P>` / `&dyn P` 这类**主体句柄**——主体之间不允许互持引用，
 *     协作只走事件。
 *
 * 主体名单是**显式清单**（不是启发式）：加主体（第 14 步的产物本身）时在这里
 * 登记一行，扫描自然覆盖。豁免=名单，不存在注释豁免通道——需要豁免说明设计错了。
 *
 * 用法：
 *   node scripts/no-direct-call-audit.mjs            # 审计 symbio/src
 *   node scripts/no-direct-call-audit.mjs ROOT=<dir> # 审计指定目录（回归测试用）
 *
 * 退出码：0 = 通过；1 = 发现违规。
 * 平台无关（纯 Node，不依赖 bash / ripgrep），带回归测试（no-direct-call-audit.test.mjs）。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

/** 主体类型名单（显式，S6 起维护；新增主体在此登记）。 */
const PRINCIPALS = ['Decider', 'Reasoner', 'RecallTranslator', 'CommitmentKeeper']

/** 相对 ROOT 的豁免路径：定义域 + 根重导出 + 同域测试。 */
const ALLOWED = new Set([
  'symbio_core/actors/mod.rs',
  'symbio_core/actors/mod.test.rs',
  'symbio_core/mod.rs',
])

/** 主体句柄形态（任何文件都禁止——含定义域内部；泛型与 `::new` 构造两种写法都拦）。 */
const HANDLE_RE = new RegExp(
  `\\b(?:Arc|Rc)<\\s*(?:${PRINCIPALS.join('|')})\\b|(?:Arc|Rc)::new\\(\\s*(?:${PRINCIPALS.join('|')})\\b|Box<\\s*dyn\\s+(?:${PRINCIPALS.join('|')})\\b|&dyn\\s+(?:${PRINCIPALS.join('|')})\\b`,
)

const WORD_RES = PRINCIPALS.map((p) => new RegExp(`\\b${p}\\b`))

function walk(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walk(p, out)
    else if (e.name.endsWith('.rs')) out.push(p)
  }
  return out
}

function audit(root) {
  const files = walk(root)
  const violations = []
  for (const file of files) {
    const rel = path.relative(root, file).split(path.sep).join('/')
    const text = fs.readFileSync(file, 'utf8')
    const lines = text.split('\n')
    const allowed = ALLOWED.has(rel) || rel.endsWith('.test.rs')
    lines.forEach((line, i) => {
      // 剥注释：文档提到主体名 ≠ 对象图边。
      const code = line.split('//')[0]
      if (HANDLE_RE.test(code)) {
        violations.push(`NDC-002 ${rel}:${i + 1}  主体句柄（互持引用）：${line.trim()}`)
        return
      }
      if (allowed || code.trim() === '') return
      const hit = WORD_RES.findIndex((re) => re.test(line))
      if (hit >= 0) {
        violations.push(
          `NDC-001 ${rel}:${i + 1}  主体类型 ${PRINCIPALS[hit]} 出现在定义域之外（持有即可直连）：${line.trim()}`,
        )
      }
    })
  }
  return violations
}

const rootArg = process.argv.find((a) => a.startsWith('ROOT='))
const root = rootArg ? path.resolve(rootArg.slice(5)) : path.join(repoRoot, 'symbio', 'src')
if (!fs.existsSync(root)) {
  console.error(red(`✗ ROOT 不存在：${root}`))
  process.exit(1)
}
const violations = audit(root)
if (violations.length > 0) {
  console.error(red(`✗ 无直连扫描：${violations.length} 处违规（出口判据「直连调用 0」）`))
  for (const v of violations) console.error('  ' + v)
  process.exit(1)
}
console.log(green('✓ 无直连扫描：主体间 0 句柄、定义域外 0 引用（直连调用 0）'))
