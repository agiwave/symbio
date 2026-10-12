#!/usr/bin/env node
/**
 * registry-audit — 登记条目守卫：字段 / 状态机 / 依赖 / 判据指认的机械校验
 *
 * ## 它守什么
 *
 * 「单文档 + 元信息 + 自动汇总」这套顶层机制的**全部强制力**都在这里（判据码
 * RG-001…RG-019，逐条语义与消息住 `registry-lib`，登记表见 `docs/reference/GATE_CODES.md`）：
 *
 *   - **RG-001** front-matter 可解析（受限 YAML 子集），未知 / 重复字段按 kind 判；
 *   - **RG-002** 必填字段非空，`kind` 与所在目录一致；
 *   - **RG-003** 编号形状、文件名 = `<id>.md`、全 registry（含跨 kind）唯一；
 *   - **RG-004** 状态词在该 kind 的状态机内；
 *   - **RG-005** `created` 是有效 `YYYY-MM-DD`；
 *   - **RG-006** `deps` 形状 / 指到存在的编号 / 不自指 / 不重复 / 不成环；
 *   - **RG-007** 状态 × 字段矩阵（`done` 必有 `evidence` 且清空 `signal`，`wont` 必有
 *     `reason`，`retired` 必有 `reason`…）；
 *   - **RG-008** 开工 / 闭环时前置必须已闭环（依赖是「必须先有」）；
 *   - **RG-009** 正文小节齐（task：`## 内容` + `## 处理过程`；spec：`## 规范` + `## 判据`）；
 *   - **RG-010** 范围读不出必须 exit 2——**不退化成「什么都没校验」**；
 *   - **RG-011** SP-001 字段表与代码 schema 恰好相等（文档侧与代码侧互为对账）；
 *   - **RG-012** 出口判据 / 判据小节指认的 `docs/` `scripts/` `e2e/` 文件必须存在
 *     （出口不许依赖尚不存在的东西，13 §0 纪律 4）；
 *   - **RG-013** `registry/` 命名空间闭合：未登记的 kind 目录即红（GW-004 同族）。
 *   - **RG-014** `prio` 在词表内（`now / next / later`）——排序提示不是义务，
 *     只影响同层排序，**不参与冲突裁决**；
 *   - **RG-015** spec 的 `supersedes`（显式取代链）：目标存在且为 spec、不自指、不重复、
 *     不成环，且 `active` 的规范不得取代另一条仍 `active` 的规范——矛盾必须显式消解。
 *   - **RG-016** spec 的 `clears`（豁免的清除任务指认）：目标存在且为 task；
 *     **不要求闭环**——豁免挂在开放的清除任务上是合法语义（`clears` 是指认，`deps` 是前置）。
 *   - **RG-017** adr 的 `impl`（落地进展，与 `status` 正交）：词表封闭
 *     （`planned` / `partial` / `done` / `none`）——把「算不算数」与「做到哪一步」
 *     拆成两个字段，是 `adr` kind 存在的核心理由。
 *
 * ## 失效形态
 *
 * 这类守卫唯一的坏死方式是**判定写宽 ⇒ 条目照样过**，而日志一片绿。回归测试
 * （`registry-audit.test.mjs`）逐条注入违规钉「必须红」，再给反向用例钉「不该误报」；
 * 另有本仓 dogfood 断言（条目数非零、SP-001 真表相等），防「读空即全绿」。
 *
 * ## 用法
 *
 *   node scripts/registry-audit.mjs            校验 registry/ 全部条目
 *   node scripts/registry-audit.mjs --write    校验通过后重生成 docs/registry/INDEX.md
 *                                              与 docs/registry/<kind>/INDEX.md
 *   node scripts/registry-audit.mjs --root=<仓库根>   （回归测试夹具用）
 *
 * 退出码：0 = 通过；1 = 有违规 / 解析失败；2 = 范围读不出（registry/ 缺失或零条目、
 * 参数不识别）。汇总页的「忘更新」**不判红**：那是确定性投影，由门禁
 * `gate.d/60-facts.mjs` 的 `autoWork` 自动重生成并暂存（与 `docs/CURRENT.md` 同一条约定）。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, dim } from './color.mjs'
import { KINDS, loadEntries, auditEntries, renderKindIndex, renderRootIndex, SCOPE_ERROR } from './registry-lib.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')

const USAGE =
  '用法：node scripts/registry-audit.mjs [--write] [--root=<仓库根>] [--strict]\n' +
  '  （无参数）校验 registry/ 下全部登记条目；--write 校验后重生成各汇总页\n' +
  '退出码：0 = 通过；1 = 有违规；2 = 范围读不出 / 参数不识别'

/**
 * CLI 入口（导出供回归测试直接调用，不 spawn）。
 * @param {string[]} argv 不含 node 与脚本路径的 argv 片段
 * @returns {{code: 0|1|2, lines: string[]}}
 */
export function runRegistryAudit(argv) {
  let root = defaultRoot
  let write = false
  for (const a of argv) {
    if (a === '--write') write = true
    else if (a === '--strict') continue // 门禁对所有 GUARDS 传 --strict；本守卫无 WARNING 档
    else if (a.startsWith('--root=')) root = path.resolve(a.slice(7))
    else return { code: 2, lines: [red(`[ERROR] 参数不识别：${a}`), USAGE] }
  }

  const lines = ['--- registry-audit: 登记条目校验（RG-001…RG-019） ---']
  const registryDir = path.join(root, 'docs', 'registry')
  const { entries, errors, notes, missing } = loadEntries(registryDir, root)
  if (missing) return { code: 2, lines: lines.concat(red(`[ERROR] RG-010 docs/registry/ 不存在（${registryDir}）——登记范围读不出`)) }

  // 零条目时只看**目录级**错误（如 RG-013）：条目级判据（RG-011 等）在没有任何条目时
  // 只会产生噪音，把「范围读不出」这个 exit 2 挤成 exit 1——那会让测试与 CI 读错故障。
  if (entries.length === 0) {
    if (errors.length) {
      for (const e of errors) lines.push(red(`[ERROR] ${e}`))
      lines.push(red(`registry-audit 未通过：${errors.length} 处违规（条目 0 个）`))
      return { code: 1, lines }
    }
    return { code: 2, lines: lines.concat(red(`[ERROR] ${SCOPE_ERROR}`)) }
  }

  const errs = [...errors, ...auditEntries(entries, root)]
  if (errs.length) {
    for (const e of errs) lines.push(red(`[ERROR] ${e}`))
    lines.push(red(`registry-audit 未通过：${errs.length} 处违规 / 解析失败（条目 ${entries.length} 个）`))
    return { code: 1, lines }
  }

  const counts = Object.entries(KINDS)
    .map(([kind, cfg]) => `${kind} ${entries.filter((e) => e.kind === kind).length}（${cfg.label}）`)
    .join(' · ')
  if (write) {
    const targets = [
      ...Object.keys(KINDS).map((kind) => ({
        file: path.join(registryDir, kind, 'INDEX.md'),
        content: renderKindIndex(kind, entries),
      })),
      { file: path.join(registryDir, 'INDEX.md'), content: renderRootIndex(entries) },
    ]
    const updated = []
    for (const t of targets) {
      const old = fs.existsSync(t.file) ? fs.readFileSync(t.file, 'utf8') : null
      if (old === t.content) continue
      fs.mkdirSync(path.dirname(t.file), { recursive: true })
      fs.writeFileSync(t.file, t.content)
      updated.push(path.relative(root, t.file).split(path.sep).join('/'))
    }
    lines.push(
      updated.length
        ? green(`registry-audit 通过：${entries.length} 个条目零违规；汇总页已更新：${updated.join('、')}`)
        : green(`registry-audit 通过：${entries.length} 个条目零违规；汇总页已是最新`),
    )
  } else {
    lines.push(green(`registry-audit 通过：${counts}——字段 / 状态 / 依赖 / 判据指认全部自洽`))
  }
  for (const n of notes) lines.push(dim(`   · ${n}`))
  return { code: 0, lines }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { code, lines } = runRegistryAudit(process.argv.slice(2))
  for (const l of lines) console.log(l)
  process.exit(code)
}
