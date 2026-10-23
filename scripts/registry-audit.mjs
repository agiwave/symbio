#!/usr/bin/env node
/**
 * registry-audit — 登记条目守卫：字段 / 状态机 / 依赖 / 判据指认的机械校验
 *
 * ## 它守什么
 *
 * 「单文档 + 元信息 + 自动汇总」这套顶层机制的**全部强制力**都在这里（判据码
 * RG-001…RG-025，逐条语义与消息住 `registry-lib`，登记表见 `docs/004-ref/GATE_CODES.md`）：
 *
 *   - **RG-001** front-matter 可解析（受限 YAML 子集），未知 / 重复字段按 kind 判；
 *   - **RG-002** 必填字段非空（kind 只来自 front-matter，目录不参与判定）；
 *   - **RG-003** 编号形状、编号 ↔ 文件对账（条目类 = 文件名 / 文档类 = 路径）、
 *     全 registry（含跨 kind）唯一；
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
 *   - **RG-013** kind 形状 / 汇总版式的 meta 校验：`prefix` / `label` 必填、状态词
 *     词表封闭、`summary.*` 列名与状态词必须在闭集内；**自定义状态机必须配套声明
 *     汇总版式**（版式由各 kind README meta 自描述，机制不内建条目类版式）。
 *   - **RG-030** kind README 与总账宿主页（docs/README.md）必须恰有一对
 *     `registry:summary` 标记（自动汇总区块的边界）。
 *   - **RG-014** `prio` 在词表内（`now / next / later`）——排序提示不是义务，
 *     只影响同层排序，**不参与冲突裁决**；
 *   - **RG-015** spec 的 `supersedes`（显式取代链）：目标存在且为 spec、不自指、不重复、
 *     不成环，且 `active` 的规范不得取代另一条仍 `active` 的规范——矛盾必须显式消解。
 *   - **RG-016** spec 的 `clears`（豁免的清除任务指认）：目标存在且为 task；
 *     **不要求闭环**——豁免挂在开放的清除任务上是合法语义（`clears` 是指认，`deps` 是前置）。
 *   - **RG-017** adr 的 `impl`（落地进展，与 `status` 正交）：词表封闭
 *     （`planned` / `partial` / `done` / `none`）——把「算不算数」与「做到哪一步」
 *     拆成两个字段，是 `adr` kind 存在的核心理由。
 *   - **RG-018** adr 的 `adr` 序号与 id 两面对账（只差 `ADR-` 前缀）；
 *   - **RG-019** adr 的 `domain` 域闭集；
 *   - **RG-020** **未登记文档**：扫描范围内活跃 `.md` 必须自带 front-matter 且声明 `kind`
 *     ——「全仓文档都在登记机制内」这句话的唯一执行方，豁免住 `registry-lib` 的 `SCAN_SKIP_*`；
 *   - **RG-021** 文档类（在位文档）的 `title` 必须逐字等于正文首个 H1——title 的唯一来源
 *     是 H1，两面对账，漂移即红；
 *   - **RG-023** `deps` 只收编号条目（导航文档不进依赖图，要指它用 `[[...]]` 引用）；
 *   - **RG-024** `parent`（分解归属）：单值编号、必须存在、**同 kind**、不自指、不成环——
 *     分解是同类事物的大小之分（总目标→子目标、大任务→小任务），跨 kind 关联走 deps / 引用；
 *   - **RG-025** 父子状态一致性：子生效 ⇒ 父生效、父退场 ⇒ 子退场（档位单调，状态不许谎报；
 *     父 doing 子 todo、子 wont 父 doing 等中间组合合法）。
 *
 * ## 失效形态
 *
 * 这类守卫唯一的坏死方式是**判定写宽 ⇒ 条目照样过**，而日志一片绿。回归测试
 * （`registry-audit.test.mjs`）逐条注入违规钉「必须红」，再给反向用例钉「不该误报」；
 * 另有本仓 dogfood 断言（条目数非零、SP-001 真表相等），防「读空即全绿」。
 *
 * ## 用法
 *
 *   node scripts/registry-audit.mjs            校验全仓登记条目（条目类 + 在位文档类）
 *   node scripts/registry-audit.mjs --write    校验通过后重生成 docs/README.md 的总账块
 *                                              与 docs/<kind>/README.md 的汇总块
 *   node scripts/registry-audit.mjs --root=<仓库根>   （回归测试夹具用）
 *
 * **扫描范围**：整个仓（`SCAN_SKIP_*` 豁免外的每个活跃 `.md` 都必须登记）——
 * 条目类（task / spec / adr）编号 = 文件名，文档类（goal / stage / plan / arch / ref / guide）
 * **文档即条目、在位存放**，编号 = 仓根相对路径；kind 只来自 front-matter，目录不参与判定。
 *
 * 退出码：0 = 通过；1 = 有违规 / 解析失败；2 = 范围读不出（扫不到任何条目、
 * 参数不识别）。汇总页的「忘更新」**不判红**：那是确定性投影，由门禁
 * `gate.d/06-facts-write.mjs` 的生成链（`autoChain`）自动重生成并暂存（与
 * `docs/004-ref/006-当前事实表.md` 同一条约定）。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { green, red, dim } from './color.mjs'
import { SUMMARY_BEGIN, SUMMARY_END, ROOT_INDEX_FILE, loadEntries, auditEntries, renderKindIndex, renderRootIndex, SCOPE_ERROR } from './registry-lib.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const defaultRoot = path.resolve(scriptDir, '..')

const USAGE =
  '用法：node scripts/registry-audit.mjs [--write] [--root=<仓库根>] [--strict]\n' +
  '  （无参数）校验全仓登记条目（docs/<kind>/ 与在位文档）；--write 校验后重生成各汇总页\n' +
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

  const lines = ['--- registry-audit: 登记条目校验（RG-001…RG-030） ---']
  const docsDir = path.join(root, 'docs')
  const { entries, projections, kinds, errors, notes, missing } = loadEntries(root)
  if (missing) return { code: 2, lines: lines.concat(red(`[ERROR] ${SCOPE_ERROR}`)) }

  // 零条目时只看**装载级**错误（未登记文档 / 未登记 kind）：条目级判据（RG-011 等）在没有任何条目时
  // 只会产生噪音，把「范围读不出」这个 exit 2 挤成 exit 1——那会让测试与 CI 读错故障。
  if (entries.length === 0) {
    if (errors.length) {
      for (const e of errors) lines.push(red(`[ERROR] ${e}`))
      lines.push(red(`registry-audit 未通过：${errors.length} 处违规（条目 0 个）`))
      return { code: 1, lines }
    }
    return { code: 2, lines: lines.concat(red(`[ERROR] ${SCOPE_ERROR}`)) }
  }

  // 投影条目（免登记的生成物，如 CURRENT.md）只参与**清单归属**那一族（RG-026…RG-029）：
  // 它们是清单的 owner，而分工重叠判据若看不见 owner 就等于没有。
  const errs = [...errors, ...auditEntries(entries, root, projections, kinds)]
  if (errs.length) {
    for (const e of errs) lines.push(red(`[ERROR] ${e}`))
    lines.push(red(`registry-audit 未通过：${errs.length} 处违规 / 解析失败（条目 ${entries.length} 个）`))
    return { code: 1, lines }
  }

  const counts = Object.entries(kinds)
    .map(([kind, cfg]) => `${kind} ${entries.filter((e) => e.kind === kind).length}（${cfg.label}）`)
    .join(' · ')
  if (write) {
    // 汇总页 = 各 kind 目录的 README.md（meta 声明 kind/prefix/label，手写区 + 标记区块）。
    // 生成器只重写**标记之间**——手写引言与 front-matter 是 README 的手写面，一概不碰。
    const updated = []
    for (const kind of Object.keys(kinds)) {
      const file = path.join(root, kinds[kind].file)
      const content = renderKindIndex(kind, entries, kinds)
      const old = fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null
      if (old === null) {
        lines.push(red(`[ERROR] ${kinds[kind].file} 不存在——kind README 是登记处，不能由生成器凭空创建`))
        return { code: 1, lines }
      }
      const b = old.indexOf(SUMMARY_BEGIN)
      const e = old.indexOf(SUMMARY_END)
      if (b === -1 || e === -1 || b > e) {
        // RG-030 会把缺标记判红；这里直接拒绝写，避免把标记区外的内容冲掉
        lines.push(red(`[ERROR] ${kinds[kind].file} 缺 \`registry:summary\` 标记对——手写区与自动区边界不明，拒绝写盘（RG-030）`))
        return { code: 1, lines }
      }
      const next = old.slice(0, b + SUMMARY_BEGIN.length) + '\n' + content + '\n' + old.slice(e)
      if (next !== old) {
        fs.writeFileSync(file, next)
        updated.push(path.relative(root, file).split(path.sep).join('/'))
      }
    }
    // 跨 kind 总账块：宿主是文档中心门户（docs/README.md）——总账不是一类文档，
    // 是门户页的一个自动块，与各 kind README 标记区同一套写法。
    const rootFile = path.join(root, ROOT_INDEX_FILE)
    const rootOld = fs.readFileSync(rootFile, 'utf8')
    const rb = rootOld.indexOf(SUMMARY_BEGIN)
    const re = rootOld.indexOf(SUMMARY_END)
    if (rb === -1 || re === -1 || rb > re) {
      lines.push(red(`[ERROR] ${ROOT_INDEX_FILE} 缺 \`registry:summary\` 标记对——总账块无处落笔，拒绝写盘（RG-030）`))
      return { code: 1, lines }
    }
    const rootNext =
      rootOld.slice(0, rb + SUMMARY_BEGIN.length) + '\n' + `<!-- list:kinds owner -->\n\n${renderRootIndex(entries, { kinds })}` + '\n' + rootOld.slice(re)
    if (rootNext !== rootOld) {
      fs.writeFileSync(rootFile, rootNext)
      updated.push(ROOT_INDEX_FILE)
    }
    lines.push(
      updated.length
        ? green(`registry-audit 通过：${entries.length} 个条目零违规；汇总区块已更新：${updated.join('、')}`)
        : green(`registry-audit 通过：${entries.length} 个条目零违规；汇总区块已是最新`),
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
