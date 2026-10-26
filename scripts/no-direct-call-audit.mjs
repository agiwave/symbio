#!/usr/bin/env node
/**
 * no-direct-call-audit — 无直连静态扫描（v2 阶段 S6 第 14 步，
 * [plan/04 §3](../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证") 出口判据「直连调用 0」；
 * [DE-001 的不变量 I1 / DE-014 单环一表](../docs/003-design/001-核心架构.md "DE-001-核心架构") 验收 2）。
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

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns NDC 主体直连
// @codes NDC-001 NDC-002

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green } from './color.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..')

/** 主体类型名单（显式，S6 起维护；新增主体在此登记）。 */
const PRINCIPALS = ['Decider', 'Reasoner', 'RecallTranslator', 'CommitmentKeeper']

/**
 * 定义域 = 整个 `symbio_core/actors` 模块树（不只是 `mod.rs`）：主体类型在**该域内**
 * 被定义与驱动是合法的（持有即可直连的风险只在该域**之外**）。按**目录**判而不是逐
 * 文件列举——域内按顶层类型拆文件（plan/13 批 C3）不该顺带把定义域收窄成一张会漂的
 * 清单（那正是本守卫最该防的：域内新增一个主体文件就被误报成 NDC-001）。
 */
const DEFINITION_DIR = 'symbio_core/actors/'
/** 另外两处合法出口：core 根重导出（`symbio_core/mod.rs`）与测试文件（排演驱动方）。 */
const ALLOWED_FILES = new Set(['symbio_core/mod.rs'])

/** 主体句柄形态（任何文件都禁止——含定义域内部；泛型与 `::new` 构造两种写法都拦）。 */
const HANDLE_RE = new RegExp(
  `\\b(?:Arc|Rc)<\\s*(?:${PRINCIPALS.join('|')})\\b|(?:Arc|Rc)::new\\(\\s*(?:${PRINCIPALS.join('|')})\\b|Box<\\s*dyn\\s+(?:${PRINCIPALS.join('|')})\\b|&dyn\\s+(?:${PRINCIPALS.join('|')})\\b`,
)

const WORD_RES = PRINCIPALS.map((p) => new RegExp(`\\b${p}\\b`))

/**
 * 封闭语法枚举的变体名不算提及主体：`Pattern::Decider` 是 `symbio-kernel` 的
 * **语法变体**（`ActorSpec.pattern` 的取值），不是主体类型的指称——语义上它只说
 * 「按裁决者档位跑」，不产生任何到主体对象的边。把它按主体名判红会把「声明一个
 * 档位」误读成「持有主体」（D-189）。
 */
const SYNTAX_VARIANT_RE = new RegExp(`\\bPattern\\s*::\\s*(?:${PRINCIPALS.join('|')})\\b`, 'g')

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
    const allowed =
      ALLOWED_FILES.has(rel) || rel.startsWith(DEFINITION_DIR) || rel.endsWith('.test.rs')
    lines.forEach((line, i) => {
      // 剥注释：文档提到主体名 ≠ 对象图边。
      const code = line.split('//')[0]
      if (HANDLE_RE.test(code)) {
        violations.push(`NDC-002 ${rel}:${i + 1}  主体句柄（互持引用）：${line.trim()}`)
        return
      }
      if (allowed || code.trim() === '') return
      // 语法变体（`Pattern::Decider`）先剔掉再查主体名：声明档位不是持有主体。
      const namesCode = code.replace(SYNTAX_VARIANT_RE, 'Pattern::<variant>')
      const hit = WORD_RES.findIndex((re) => re.test(namesCode))
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
