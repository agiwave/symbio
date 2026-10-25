#!/usr/bin/env node
/**
 * spec-coverage-audit — 规范条文的**执行方覆盖率**（判定型读数）
 *
 * ## 它回答什么
 *
 * GO-003 E3：「每条规范都写明了执行方（脚本名或判据码），或明确标注『无机器判据』
 * 及其理由。」在这条脚本之前，这句话靠评审——没有一处统计「33 条规范里多少条有
 * 机器判据」。没有读数，「全面」就只是愿望。
 *
 * ## 口径（先定义，再计数）
 *
 * 范围 = 活跃（`status: active`）且 `kind: spec` 的编号条目（`docs/005-spec/` 下的规范文档）。
 * 每条取 `## 判据` 小节，按**优先级**判一次：
 *
 * | 分类 | 条件 |
 * |---|---|
 * | `manual` | 小节里出现 `无机器判据：<理由>`，且理由非空（≥ 8 个非空白字符） |
 * | `machine` | 小节指名一个**存在的** `scripts/**\/*.mjs`（反引号或链接均可），或一个**已登记**的判据码（`@codes` 声明过） |
 * | `missing` | 两者都没有——判据面没有落点，红 |
 *
 * 「有机器执行方」只证明**落点存在**，不证明它够强：报告型脚本（如 `schema-audit`）
 * 也算执行方，但它的强弱是评审的事。这个边界写在 GO-003 §3，不在这里扩张。
 *
 * `manual` 不是违规——它是**诚实的缺口**：计数进基线，只许降（把一条改成机器判据，
 * 这条读数就降一格）。`missing` 是违规，目标恒为 0。
 *
 * ## 判据码
 * - SPC-001：每条活跃规范必须落到 `machine` / `manual` / `missing` 之一；`missing` 即红。
 * - SPC-002：`无机器判据：` 的理由不得为空或纯标点。
 * - SPC-003：范围读不出 / 口径不自洽（文件数为 0、分类数之和不等于活跃数、重复编号）即红。
 *
 * ## 用法
 *   node scripts/spec-coverage-audit.mjs              # 判定
 *   node scripts/spec-coverage-audit.mjs --verbose    # 逐条执行方
 *   node scripts/spec-coverage-audit.mjs --root=<dir> # 指向别处的仓库（测试用）
 * 退出码：0 = 通过，1 = 违规 / 范围读不出。
 *
 * 环境变量 `SPEC_COVERAGE_BASELINE=<manual>:<missing>` **只给回归测试用**；
 * 覆盖值不得比默认值更松（`gate.d/35-baseline.mjs` 另判一次）。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns SPC 规范执行方覆盖
// @codes SPC-001 SPC-002 SPC-003

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, yellow, green, dim } from './color.mjs'
import { parseEntryText, sectionOf } from './registry-lib.mjs'
import { collect } from './gate-codes-audit.mjs'

const argv = process.argv.slice(2)
const VERBOSE = argv.includes('--verbose')
const rootArg = argv.find((a) => a.startsWith('--root='))
const ROOT = rootArg
  ? path.resolve(rootArg.slice('--root='.length))
  : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

const SPEC_DIR_REL = 'docs/005-spec'
/** 债务两格：`manual` = 合法的人工判据存量，`missing` = 违规存量（目标 0）。只许降。 */
const BASELINE = process.env.SPEC_COVERAGE_BASELINE
  ? (() => {
      const [manual, missing] = process.env.SPEC_COVERAGE_BASELINE.split(':').map((n) => Number(n))
      return { manual, missing }
    })()
  : { manual: 3, missing: 0 }

const MANUAL_RE = /无机器判据[：:]\s*([\s\S]*?)(?:\n\s*\n|$)/

/** 小节里的脚本指认：反引号与 Markdown 链接都认（`registry-lib.refsIn` 只认反引号）。 */
function scriptRefs(section) {
  return [...section.matchAll(/(?:^|[\s(`])(scripts\/[A-Za-z0-9_./-]+\.mjs)/gm)].map((m) => m[1])
}

function codeRefs(section) {
  return [...section.matchAll(/\b([A-Z]{1,3}-\d{3})\b/g)].map((m) => m[1])
}

/** 已登记判据码集合（从各脚本 `@codes` 声明读，不读生成物）。 */
function registeredCodes(root) {
  const set = new Set()
  for (const s of collect(root)) for (const code of s.codes.keys()) set.add(code)
  return set
}

export function classify(section, { exists, codes }) {
  const manual = MANUAL_RE.exec(section)
  if (manual) {
    const reason = manual[1].replace(/\s+/g, ' ').trim()
    return { kind: 'manual', reason, validReason: reason.replace(/\s+/g, '').length >= 8 }
  }
  const scripts = scriptRefs(section).filter(exists)
  if (scripts.length) return { kind: 'machine', via: scripts.map((s) => `scripts/${s.slice(8)}`) }
  const hit = codeRefs(section).filter((c) => codes.has(c))
  if (hit.length) return { kind: 'machine', via: hit.map((c) => `code:${c}`) }
  return { kind: 'missing' }
}

export function scan(root) {
  const dir = path.join(root, SPEC_DIR_REL)
  let names
  try {
    names = fs.readdirSync(dir).filter((n) => /^0\d\d\.md$/.test(n)).sort()
  } catch {
    return { error: `读不出 ${SPEC_DIR_REL}` }
  }
  const exists = (rel) => fs.existsSync(path.join(root, rel))
  const codes = registeredCodes(root)
  const entries = []
  const seen = new Set()
  for (const name of names) {
    const rel = `${SPEC_DIR_REL}/${name}`
    const { meta, body, errors } = parseEntryText(fs.readFileSync(path.join(root, rel), 'utf8'), rel)
    if (!meta || meta.kind !== 'spec' || meta.status !== 'active') continue
    if (seen.has(meta.id)) entries.push({ id: meta.id, rel, duplicate: true })
    seen.add(meta.id)
    const section = sectionOf(body, '判据')
    entries.push({ id: meta.id, rel, section: Boolean(section), ...classify(section, { exists, codes }) })
  }
  return { entries }
}

function main() {
  const result = scan(ROOT)
  if (result.error) {
    console.error(red(`✗ ${result.error}——判据没有输入，宁红不静默`))
    return 1
  }
  const entries = result.entries
  const errors = []
  const report = (rule, msg) => errors.push({ rule, msg })

  const active = entries.length
  const machine = entries.filter((e) => e.kind === 'machine' && !e.duplicate)
  const manual = entries.filter((e) => e.kind === 'manual' && !e.duplicate)
  const missing = entries.filter((e) => !e.duplicate && (e.kind === 'missing' || !e.section))

  for (const e of entries) {
    if (e.duplicate) report('SPC-003', `${e.rel} 的编号 ${e.id} 重复——两篇规范共用一个地址`)
    else if (!e.section) report('SPC-001', `${e.id}：没有 \`## 判据\` 小节，执行方无从谈起`)
    else if (e.kind === 'missing')
      report(
        'SPC-001',
        `${e.id}：\`## 判据\` 既没有指名存在的脚本 / 已登记判据码，也没有 \`无机器判据：<理由>\``,
      )
    else if (e.kind === 'manual' && !e.validReason)
      report('SPC-002', `${e.id}：\`无机器判据\` 的理由为空或纯标点——空理由的豁免等于没有判据`)
  }

  if (active === 0) report('SPC-003', `${SPEC_DIR_REL} 里一条活跃规范都没读到——范围读空，不是「全覆盖」`)
  if (machine.length + manual.length + missing.length !== active) {
    report('SPC-003', `口径不自洽：${machine.length}+${manual.length}+${missing.length} ≠ ${active}`)
  }
  if (manual.length > BASELINE.manual)
    report('SPC-001', `人工判据存量 ${manual.length} > 基线 ${BASELINE.manual}——新条目请先落机器执行方`)
  if (missing.length > BASELINE.missing)
    report('SPC-001', `无执行方的规范 ${missing.length} > 基线 ${BASELINE.missing}`)

  if (errors.length === 0) {
    console.log('\n== spec-coverage-audit ==')
    console.log(
      `${green('✓')} SPC-001  活跃 ${active} = 机器 ${machine.length} + 人工 ${manual.length} + 缺失 ${missing.length}`,
    )
    console.log(
      `${green('✓')} SPC-002  ${manual.length} 条 \`无机器判据\` 均带非空理由` +
        (manual.length ? dim(`（${manual.map((e) => e.id).join(' · ')}）`) : ''),
    )
    console.log(`${green('✓')} SPC-003  口径自洽，编号无重复`)
    if (VERBOSE) {
      for (const e of entries) {
        const tag = e.kind === 'machine' ? 'M' : e.kind === 'manual' ? 'H' : '!'
        console.log(
          `  [${tag}] ${e.id.padEnd(7)} ${e.kind === 'machine' ? e.via.join(' · ') : e.kind === 'manual' ? e.reason.slice(0, 90) : '缺失'}`,
        )
      }
    }
    const under = manual.length < BASELINE.manual
    if (under)
      console.log(
        yellow('提示') + `：人工判据存量已降到 ${manual.length}（基线 ${BASELINE.manual}），请同步 SPEC_COVERAGE_BASELINE`,
      )
    return 0
  }

  console.error(`\n== spec-coverage-audit：${errors.length} 处违规 ==`)
  for (const e of errors) console.error(`  ${red(e.rule)}  ${e.msg}`)
  return 1
}

const MAIN =
  process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))
if (MAIN) process.exit(main())
