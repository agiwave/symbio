#!/usr/bin/env node
/**
 * gen-facts —— 事实生成 runner：遍历 `scripts/facts/`，逐个生成全部事实文件
 *
 * ## 它替掉了什么
 *
 * 此前三个生成器是三份独立脚本，各自带一份「怎么写文件、怎么判漂移、怎么报
 * 溯源」的副本，门禁 `gate.d/60-facts.mjs` 里也是三段手抄的 `autoWork` 调用。
 * 新增一份事实要改四处：写脚本、抄调用段、在 `30-docs` 的 `LIBS` 登记、在各处
 * 文档补指路——漏任何一处的后果不是报错，是**静默**（漏了调用段则那份事实永远
 * 不更新，而门禁全绿）。
 *
 * 现在新增一份事实 = 在 `scripts/facts/` 建一个文件。**目录即接线**。
 *
 * ## 用法
 *
 *   node scripts/gen-facts.mjs           # 生成全部（内容没变的整份沿用，不改写）
 *   node scripts/gen-facts.mjs --check   # 只比对，任一份漂移则非零退出（CI 用）
 *
 * ## 三条退出码，刻意分开
 *
 * | 码 | 含义 |
 * |---|:-:|
 * | 0 | 全部生成成功 / 全部一致 |
 * | 1 | 有漂移（`--check`）或某份 `render` 抛错 |
 * | 2 | **范围读不出**：目录不存在、或一份事实都没找到 |
 *
 * 第 2 条是这份 runner 唯一不能省的判定：本仓反复买到教训的一侧是
 * 「守卫读空了，于是什么都没判、全绿」（`registry-audit` 的 RG-010 是同款）。
 * 一个空目录会让 `60-facts` 的 autoWork 退出 0，什么都不生成——**那正是它
 * 必须在最坏情况下红的方向**。
 *
 * ## 只收「离线可重生成」的代码投影
 *
 * 需要**跑起来**才产出的事实不进本目录——`docs/006-ref/PROMPT_SHAPE.md` 由 e2e `t46`
 * 从真实链路抓，`docs/006-ref/PROMPT_AB.md` 由门外实验生成。它们的生成点在 e2e 而非
 * 门内脚本，搬进来会让 runner 在没有运行时的场合（CI 的 facts 阶段）静默产出
 * 一份错的，或报一个与真相无关的红。这条边界写在 `facts/README.md`。
 */

import { readdirSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { red, green, dim, yellow } from './color.mjs'
import { validateFact, driftOf, emit } from './facts/_lib.mjs'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const FACTS_DIR = path.join(ROOT, 'scripts', 'facts')
const IS_CHECK = process.argv.includes('--check')

/**
 * 发现事实模块：目录下非 `_` 前缀的 `.mjs`（`_` 前缀 = 共享材料，不是一份事实）。
 *
 * ⚠️ **`.test.mjs` 一律不算事实**。它住在同一个目录里（回归测试要贴着被测的
 * 生成器，`facts/routes-ts.test.mjs` 就是），而它导出的是测试函数、没有描述符。
 * 不排除的后果不是「多跑一个文件」，而是 runner 对它判「契约破损」并**让整轮
 * 生成失败**——一个和事实生成毫无关系的测试文件把门禁拖红。
 */
export function discover(root = FACTS_DIR) {
  const names = readdirSync(root, { withFileTypes: true })
    .filter((e) => e.isFile() && e.name.endsWith('.mjs') && !e.name.startsWith('_'))
    .map((e) => e.name.replace(/\.mjs$/, ''))
    .filter((id) => !id.endsWith('.test'))
    .sort()
  return names
}

async function loadFact(id) {
  const mod = await import(pathToFileURL(path.join(FACTS_DIR, `${id}.mjs`)).href)
  return mod.default
}

async function main() {
  let ids
  try {
    ids = discover()
  } catch (e) {
    console.error(red(`✗ 读不出事实目录 \`scripts/facts/\`：${String(e.message ?? e)}`))
    return 2
  }
  if (ids.length === 0) {
    console.error(
      red(
        '✗ `scripts/facts/` 里一份事实都没有——目录被搬走 / 清空，或抽取口径变了？\n' +
          '  这是最坏的静默：门禁会「生成成功」而什么也没生成。事实清单不需要登记，\n' +
          '  目录里有几个文件就生成几份。',
      ),
    )
    return 2
  }

  const contractErrs = []
  const drifts = []
  const wrote = []
  const failed = []

  for (const id of ids) {
    let fact
    try {
      fact = await loadFact(id)
    } catch (e) {
      failed.push(`${id}：加载失败（${String(e.message ?? e)}）`)
      continue
    }
    const errs = validateFact(fact, { id, outRoot: null })
    if (errs.length) {
      contractErrs.push(...errs)
      continue
    }
    let content
    try {
      content = fact.render({ root: ROOT })
    } catch (e) {
      failed.push(`${id}：render() 抛出（${String(e.message ?? e)}）`)
      continue
    }
    if (typeof content !== 'string' || !content.length) {
      failed.push(`${id}：render() 必须返回非空字符串`)
      continue
    }
    const outAbs = path.join(ROOT, ...fact.out.split('/'))
    if (IS_CHECK) {
      const d = driftOf({ out: outAbs, content })
      if (d) drifts.push(d)
      else console.log(green(`  ✓ ${fact.out}（${fact.title}）`))
    } else {
      const r = emit({ out: outAbs, content, stamp: fact.stamp !== false })
      if (r.changed) wrote.push(`${fact.out}（${r.lines} 行）`)
      else console.log(dim(`  · ${fact.out} 已是最新（${r.lines} 行，未改写）`))
    }
  }

  console.log(
    dim(
      `  ${IS_CHECK ? '核对' : '生成'} ${ids.length} 份事实：${ids.join(' · ')}` +
        (IS_CHECK ? '' : wrote.length ? `；已写 ${wrote.length} 份` : '；无变化'),
    ),
  )
  if (wrote.length) for (const w of wrote) console.log(yellow(`  ✎ ${w}`))

  if (contractErrs.length) {
    for (const e of contractErrs) console.error(red(`✗ 事实描述符不合契约 —— ${e}`))
    return 1
  }
  if (failed.length) {
    for (const f of failed) console.error(red(`✗ ${f}`))
    return 1
  }
  if (drifts.length) {
    for (const d of drifts) console.error(red(`✗ ${d}`))
    return 1
  }
  return 0
}

// 只在被**直接执行**时跑；被 import（回归测试要读 `discover`）时什么也不做。
//
// 这道闸不是洁癖：`facts.test.mjs` 要 import 本文件的 `discover`，而顶层
// `process.exit(await main())` 会在 import 期就把测试进程带走——于是测试文件
// 报出「1 个测试通过」而一条断言都没跑，是本仓最难查的那种假绿。
// 同目录的 `routes-ts.mjs` 当年也带着这道闸（原 `facts/routes-ts.mjs`），两处同源。
const MAIN =
  process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))

if (MAIN) process.exit(await main())