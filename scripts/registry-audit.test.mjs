// `registry-audit` 的回归测试（连带覆盖 `registry-lib` 与 `registry-board`）。
//
// 这套登记机制的失效形态只有一种且最坏：**判定写宽 ⇒ 违规条目照样过，日志一片绿**。
// 所以每条判据都配一个注入的真实违规（必须红、且只红这一条）与一个反向用例
// （合法夹具不得误报）；报告型的 `registry-board` 与 `schema-audit` 同理——
// 坏掉的报告天天被读，比没有报告更坏，所以它「有违规时拒绝打印」也要钉住。
//
// 跑法：node --test scripts/registry-audit.test.mjs

import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { stripAnsi } from './color.mjs'
import { ALL_KEYS, KINDS, parseEntryText, loadEntries, auditEntries, renderKindIndex } from './registry-lib.mjs'
import { runRegistryAudit } from './registry-audit.mjs'
import { runRegistryBoard } from './registry-board.mjs'

const repoRoot = path.resolve(import.meta.dirname, '..')

// ================= 夹具 =================
//
// 用真实文件系统而不是内存对象：`loadEntries` 判的就是「registry/ 下有哪些文件、
// 叫什么名字」，打桩等于把被测的目录事实换掉。

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-registry-'))
let caseNo = 0

const TASK_BODY = '## 内容\n说明。\n\n## 处理过程\n- 待补\n'
const SPEC_BODY = '## 规范\n条文。\n\n## 判据\n人工核对。\n'
const ADR_BODY = '## 决策\n一句话结论。\n\n## 状态\n- status：现行。\n'

/** front-matter 序列化（夹具写文件用）。 */
function fm(meta) {
  const lines = ['---']
  for (const [k, v] of Object.entries(meta)) {
    if (v === null || v === undefined) lines.push(`${k}:`)
    else if (Array.isArray(v)) lines.push(`${k}: [${v.join(', ')}]`)
    else lines.push(`${k}: ${v}`)
  }
  lines.push('---', '')
  return lines.join('\n')
}

function taskDoc(id, over = {}) {
  const { body, ...meta } = over
  return (
    fm({ kind: 'task', id, title: `${id} 任务`, status: 'todo', created: '2026-10-11', track: 'meta', deps: [], ...meta }) +
    (body ?? TASK_BODY)
  )
}

function specDoc(id, over = {}) {
  const { body, ...meta } = over
  return (
    fm({ kind: 'spec', id, title: `${id} 规范`, status: 'active', created: '2026-10-11', scope: '夹具', deps: [], ...meta }) +
    (body ?? SPEC_BODY)
  )
}

/** adr 夹具：`adr` 序号默认从 id 去前缀推得（正向用例不该因手抄不对账）。 */
function adrDoc(id, over = {}) {
  const { body, ...meta } = over
  return (
    fm({
      kind: 'adr',
      id,
      title: `${id} 决策`,
      status: 'active',
      impl: 'done',
      domain: '平台基座',
      created: '2026-10-11',
      adr: id.replace(/^ADR-/, ''),
      deps: [],
      ...meta,
    }) + (body ?? ADR_BODY)
  )
}

/** SP-001 夹具：字段表默认从 ALL_KEYS 生成（正向用例不该因手抄表腐烂）。 */
function sp001Doc(rows = ALL_KEYS.map((k) => `| \`${k}\` | 说明 |`)) {
  return (
    fm({ kind: 'spec', id: 'SP-001', title: '字段规范（夹具）', status: 'active', created: '2026-10-11', scope: '夹具' }) +
    '\n## 规范\n\n条文。\n\n## 字段\n\n| 字段 | 说明 |\n|---|---|\n' +
    rows.join('\n') +
    '\n\n## 判据\n\n人工核对。\n'
  )
}

/**
 * 文档类夹具：**在位存放**的条目——编号 = 仓根相对路径，title = 正文首个 H1。
 * 写到 `docs/<rel>`（rel 不带 `docs/` 前缀时补上；以 `docs/` 开头则原样）。
 */
function docDoc(relWithDocs, over = {}) {
  const rel = relWithDocs.startsWith('docs/') ? relWithDocs : `docs/${relWithDocs}`
  const { body, ...meta } = over
  const title = over.title ?? `${rel.replace(/\.md$/, '')} 文档`
  return (
    fm({ kind: 'arch', id: rel.replace(/\.md$/, ''), title, status: 'active', created: '2026-10-11', ...meta }) +
    (body ?? `# ${title}\n\n正文。\n`)
  )
}

/**
 * 一个用例一座目录：扫描范围是**整个仓**，夹具共用目录会互相串断言。
 * 默认自带 SP-001（RG-011 对它有要求）；`noSp` 用于测 SP-001 缺失。
 * 键以 `docs/` 开头 ⇒ 按仓根相对路径写；否则写到 `docs/<kind>/` 下（条目与文档同构：
 * **kind 只来自 front-matter，目录不参与判定**）。
 */
function caseRoot(files, { noSp = false } = {}) {
  const base = path.join(tmp, `case${++caseNo}`)
  const all = { ...files }
  if (!noSp && !all['spec/SP-001.md']) all['spec/SP-001.md'] = sp001Doc()
  for (const [rel, content] of Object.entries(all)) {
    // 键以 `/` 开头 = 仓根相对（如 `/AGENTS.md`）；以 `docs/` 开头 = 仓根相对；否则住 `docs/<kind>/`
    const p = path.join(base, rel.startsWith('/') ? rel.slice(1) : rel.startsWith('docs/') ? rel : path.join('docs', rel))
    fs.mkdirSync(path.dirname(p), { recursive: true })
    fs.writeFileSync(p, content)
  }
  return base
}

function errsOf(base) {
  const { entries, errors } = loadEntries(base)
  return [...errors, ...auditEntries(entries, base)]
}

/** 只允许这一条错（其余夹具必须干净，否则测不出"这一条判据在工作"）。 */
function expectOnly(errs, re) {
  assert.equal(errs.length, 1, `期望只有一条违规，实得：\n${errs.join('\n')}`)
  assert.match(errs[0], re, errs[0])
}

after(() => {
  try {
    fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 3 })
  } catch {
    /* 沙箱的 safe-delete 垫片可能拒删，交 OS 回收 */
  }
})

// ================= 解析（RG-001） =================

test('parseEntryText：受限 YAML 四种值形态（裸 / 引号 / 行内列表 / 块列表）', () => {
  const text = [
    '---',
    'kind: task',
    'title: "带: 冒号与 # 井号的标题"',
    "track: '单引号'",
    'deps: [T-001, T-002]',
    'where:',
    '  - docs/plan/14.md',
    '  - scripts/x.mjs',
    '---',
    'body',
  ].join('\n')
  const { meta, errors } = parseEntryText(text, 'x.md')
  assert.deepEqual(errors, [])
  assert.equal(meta.title, '带: 冒号与 # 井号的标题')
  assert.equal(meta.track, '单引号')
  assert.deepEqual(meta.deps, ['T-001', 'T-002'])
  assert.deepEqual(meta.where, ['docs/plan/14.md', 'scripts/x.mjs'])
})

test('RG-001：首行不是 --- / 未闭合 / 行形状 / 重复键 / 未知字段 各自变红', () => {
  assert.match(parseEntryText('kind: task\n', 'x.md').errors[0], /RG-001 首行/)
  assert.match(parseEntryText('---\nkind: task\n', 'x.md').errors[0], /RG-001 .*闭合/)

  const badLine = caseRoot({ 'task/T-001.md': '---\nkind: task\n这不是键值\nid: T-001\ntitle: x\nstatus: todo\ncreated: 2026-10-11\ntrack: meta\ndeps: []\n---\n' + TASK_BODY })
  expectOnly(errsOf(badLine), /RG-001 第 3 行不是/)

  const dup = caseRoot({ 'task/T-001.md': '---\nkind: task\nkind: task\nid: T-001\ntitle: x\nstatus: todo\ncreated: 2026-10-11\ntrack: meta\ndeps: []\n---\n' + TASK_BODY })
  expectOnly(errsOf(dup), /RG-001 字段 `kind` 重复/)

  const unknown = caseRoot({ 'task/T-001.md': taskDoc('T-001', { bogus: 1 }) })
  expectOnly(errsOf(unknown), /RG-001 未知字段 `bogus`/)
})

test('RG-001：错域字段被指名（task 里写 scope）', () => {
  const base = caseRoot({ 'task/T-001.md': taskDoc('T-001', { scope: 'x' }) })
  expectOnly(errsOf(base), /RG-001 未知字段 `scope`/)
})

// ================= 字段与编号（RG-002…005） =================

test('RG-002：必填字段缺失变红；**目录不参与判定**（kind 只来自 front-matter）', () => {
  const missing = caseRoot({ 'task/T-001.md': fm({ kind: 'task', id: 'T-001', title: 'x', status: 'todo', created: '2026-10-11', deps: [] }) + TASK_BODY })
  expectOnly(errsOf(missing), /RG-002 必填字段 `track`/)

  // 反向：条目放在别的目录下不再判红——目录只是组织，kind 是数据（机制不写死目录结构）。
  const elsewhere = caseRoot({ 'docs/plan/T-001.md': taskDoc('T-001') })
  assert.deepEqual(errsOf(elsewhere), [], 'task 放在 docs/plan/ 下必须合法')
})

test('RG-003：id 形状 / 文件名对账 / 跨 kind 重复 各自变红', () => {
  const shape = caseRoot({ 'task/bad!id.md': taskDoc('bad!id') })
  expectOnly(errsOf(shape), /RG-003 id 不合形状/)

  // 编号的名称段允许 Unicode 字母数字（圈号等）——编号段本身仍是 <前缀>-<三位序号>
  const circle = caseRoot({ 'task/T-041-结束①.md': taskDoc('T-041-结束①') })
  assert.deepEqual(errsOf(circle), [], '名称段含 Unicode（圈号）必须合法')

  const renamed = caseRoot({ 'task/wrong.md': taskDoc('T-040') })
  expectOnly(errsOf(renamed), /RG-003 文件名必须等于编号（应为 `T-040\.md`）/)

  // 跨 kind 撞号：新规范下同前缀只属于一个 kind，所以撞号必然伴随「前缀与 kind 不符」
  // ——两条判据都在工作（前缀对账 + 编号重复）
  const dup = caseRoot({ 'spec/T-050.md': specDoc('T-050'), 'task/T-050.md': taskDoc('T-050') })
  const dupErrs = errsOf(dup)
  assert.equal(dupErrs.length, 2, `期望两条违规，实得：\n${dupErrs.join('\n')}`)
  assert.match(dupErrs.join('\n'), /RG-022 spec 的编号前缀必须是 `SP-`/)
  assert.match(dupErrs.join('\n'), /RG-003 编号 `T-050` 重复/)
})

test('RG-004 / RG-005：状态词与日期变红', () => {
  const badStatus = caseRoot({ 'task/T-001.md': taskDoc('T-001', { status: 'nope' }) })
  expectOnly(errsOf(badStatus), /RG-004 status="nope" 不在 task 的状态词/)

  const badDate = caseRoot({ 'task/T-001.md': taskDoc('T-001', { created: '2026-13-40' }) })
  expectOnly(errsOf(badDate), /RG-005 created 不是有效日期/)
})

test('RG-014：prio 词表外变红；合法值放行（反向）', () => {
  const bad = caseRoot({ 'task/T-001.md': taskDoc('T-001', { prio: 'urgent' }) })
  expectOnly(errsOf(bad), /RG-014 prio 不在词表/)

  const ok = caseRoot({ 'task/T-001.md': taskDoc('T-001', { prio: 'now' }) })
  assert.deepEqual(errsOf(ok), [], 'prio: now 合法，不判红')
})

// ================= 依赖（RG-006 / RG-008） =================

test('RG-006：deps 形状 / 不存在的编号 / 重复登记 变红', () => {
  const shape = caseRoot({ 'task/T-001.md': taskDoc('T-001', { deps: 'oops' }) })
  expectOnly(errsOf(shape), /RG-006 deps 必须是/)

  const missing = caseRoot({ 'task/T-001.md': taskDoc('T-001', { deps: ['T-999'] }) })
  expectOnly(errsOf(missing), /RG-006 deps 指到不存在的编号 `T-999`/)

  const twice = caseRoot({ 'task/T-001.md': taskDoc('T-001', { deps: ['T-002', 'T-002'] }), 'task/T-002.md': taskDoc('T-002') })
  expectOnly(errsOf(twice), /RG-006 deps 重复登记 `T-002`/)
})

test('RG-006：自指与成环变红（环上的编号永远等不到前置闭环）', () => {
  const self = caseRoot({ 'task/T-001.md': taskDoc('T-001', { deps: ['T-001'] }) })
  const selfErrs = errsOf(self)
  assert.ok(selfErrs.length >= 1 && selfErrs.every((e) => /RG-006/.test(e)), selfErrs.join('\n'))
  assert.ok(selfErrs.some((e) => /含自身/.test(e)), selfErrs.join('\n'))

  const cyc = caseRoot({ 'task/T-020.md': taskDoc('T-020', { deps: ['T-021'] }), 'task/T-021.md': taskDoc('T-021', { deps: ['T-020'] }) })
  expectOnly(errsOf(cyc), /RG-006 依赖成环：T-0(20|21) -> T-0(20|21)/)
})

test('RG-008：doing 要求前置已闭环；前置 done 则放行（反向）', () => {
  const blocked = caseRoot({ 'task/T-001.md': taskDoc('T-001', { status: 'doing', deps: ['T-002'] }), 'task/T-002.md': taskDoc('T-002') })
  expectOnly(errsOf(blocked), /RG-008 status=doing 要求前置 `T-002` 已闭环，而它是 todo/)

  const ok = caseRoot({
    'task/T-001.md': taskDoc('T-001', { status: 'doing', deps: ['T-002'] }),
    'task/T-002.md': taskDoc('T-002', { status: 'done', evidence: 'abc1234' }),
  })
  assert.deepEqual(errsOf(ok), [], '前置已闭环 ⇒ 开工不判')
})

test('RG-008：依赖机制全部 kind 同一套——文档类 / adr 的 active 同样要求前置已闭环', () => {
  // 文档类不搞特殊化：`active` 断言「这篇现行」，其 `deps` 前置就必须已闭环
  const blockedDoc = caseRoot({
    'docs/plan/PL-001-a.md': docDoc('docs/plan/PL-001-a.md', { kind: 'plan', id: 'PL-001-a', deps: ['PL-002-b'] }),
    'docs/plan/PL-002-b.md': docDoc('docs/plan/PL-002-b.md', { kind: 'plan', id: 'PL-002-b', status: 'draft' }),
  })
  expectOnly(errsOf(blockedDoc), /RG-008 status=active 要求前置 `PL-002-b` 已闭环，而它是 draft/)

  const okDoc = caseRoot({
    'docs/plan/PL-001-a.md': docDoc('docs/plan/PL-001-a.md', { kind: 'plan', id: 'PL-001-a', deps: ['PL-002-b'] }),
    'docs/plan/PL-002-b.md': docDoc('docs/plan/PL-002-b.md', { kind: 'plan', id: 'PL-002-b' }),
  })
  assert.deepEqual(errsOf(okDoc), [], '前置 active ⇒ 放行')

  // 跨 kind 同样强制：文档的 active 前置可以是一条已闭环的 task
  const okTask = caseRoot({
    'docs/plan/PL-001-a.md': docDoc('docs/plan/PL-001-a.md', { kind: 'plan', id: 'PL-001-a', deps: ['T-002'] }),
    'task/T-002.md': taskDoc('T-002', { status: 'done', evidence: 'abc1234' }),
  })
  assert.deepEqual(errsOf(okTask), [], '前置 task 已 done ⇒ 放行')

  // adr 与 spec / 文档类同构：active 要求前置已闭环
  const blockedAdr = caseRoot({
    'adr/ADR-090.md': adrDoc('ADR-090', { deps: ['ADR-091'] }),
    'adr/ADR-091.md': adrDoc('ADR-091', { status: 'withdrawn', reason: '夹具' }),
  })
  expectOnly(errsOf(blockedAdr), /RG-008 status=active 要求前置 `ADR-091` 已闭环，而它是 withdrawn/)

  // draft 文档可以引草案——门槛只档「生效中」状态
  const draft = caseRoot({
    'docs/plan/PL-001-a.md': docDoc('docs/plan/PL-001-a.md', { kind: 'plan', id: 'PL-001-a', status: 'draft', deps: ['PL-002-b'] }),
    'docs/plan/PL-002-b.md': docDoc('docs/plan/PL-002-b.md', { kind: 'plan', id: 'PL-002-b', status: 'draft' }),
  })
  assert.deepEqual(errsOf(draft), [], 'draft 引 draft 不判')
})

test('RG-022：全仓一套编号规范——编号条目 <前缀>-<三位序号>[-名称]；导航文档 id 是路径', () => {
  // 合规编号条目放行（前缀在闭集、序号三位、名称段中文合法）
  const okEntry = caseRoot({
    'docs/plan/PL-001-核心架构.md': docDoc('docs/plan/PL-001-核心架构.md', {
      kind: 'plan',
      id: 'PL-001-核心架构',
    }),
    'task/T-016.md': taskDoc('T-016', { track: 'S' }),
  })
  assert.deepEqual(errsOf(okEntry), [], '合规编号条目（带名称段 / 纯编号）放行')

  // 历史遗留前缀（轨前缀 S2.7 / 无连字符）不合新语法 ⇒ 红
  const badPrefix = caseRoot({
    'docs/plan/S2.7.md': docDoc('docs/plan/S2.7.md', { kind: 'plan', id: 'S2.7' }),
  })
  expectOnly(errsOf(badPrefix), /RG-022 编号条目 id 必须匹配/)

  // 前缀不在闭集 ⇒ 红
  const badKind = caseRoot({
    'docs/plan/XX-001-a.md': docDoc('docs/plan/XX-001-a.md', { kind: 'plan', id: 'XX-001-a' }),
  })
  expectOnly(errsOf(badKind), /RG-022 编号条目 id 必须匹配/)

  // 语法合法但前缀与 kind 不符（plan 用了 T-）⇒ 红——编号空间才可盲信
  const wrongPrefix = caseRoot({
    'docs/plan/T-056.md': docDoc('docs/plan/T-056.md', { kind: 'plan', id: 'T-056' }),
  })
  expectOnly(errsOf(wrongPrefix), /RG-022 plan 的编号前缀必须是 `PL-`/)

  // 编号条目：文件名与编号不符 ⇒ 红
  const mismatch = caseRoot({
    'docs/plan/别的名字.md': docDoc('docs/plan/别的名字.md', { kind: 'plan', id: 'PL-001-核心架构' }),
  })
  expectOnly(errsOf(mismatch), /RG-003 文件名必须等于编号/)

  // 导航文档：id 含 / 但不是仓根相对路径 ⇒ 红
  const badNav = caseRoot({
    'docs/README.md': docDoc('docs/README.md', { kind: 'guide', id: 'docs/guide/README' }),
  })
  expectOnly(errsOf(badNav), /RG-003 导航文档 id 必须等于仓根相对路径/)

  // 导航文档：README / INDEX 的 id = 路径 ⇒ 放行（不进 deps，见 RG-023）
  const okNav = caseRoot({
    'docs/README.md': docDoc('docs/README.md', { kind: 'guide', id: 'docs/README' }),
  })
  assert.deepEqual(errsOf(okNav), [], '导航 README 放行')
})

test('RG-023：deps 是依赖图的边，只收编号条目——导航文档（id 含 /）判红', () => {
  const bad = caseRoot({
    'docs/plan/PL-001-a.md': docDoc('docs/plan/PL-001-a.md', { kind: 'plan', id: 'PL-001-a', deps: ['docs/README'] }),
    'docs/README.md': docDoc('docs/README.md', { kind: 'guide', id: 'docs/README' }),
  })
  expectOnly(errsOf(bad), /RG-023 deps 目标必须是编号条目/)

  const ok = caseRoot({
    'docs/plan/PL-001-a.md': docDoc('docs/plan/PL-001-a.md', { kind: 'plan', id: 'PL-001-a', deps: ['PL-002-b'] }),
    'docs/plan/PL-002-b.md': docDoc('docs/plan/PL-002-b.md', { kind: 'plan', id: 'PL-002-b' }),
  })
  assert.deepEqual(errsOf(ok), [], '编号条目互依赖放行')
})

test('RG-015：supersedes 目标不存在 / 指向 task / 自身 / 双 active / 成环 变红；旧条 retired 则放行', () => {
  const missing = caseRoot({ 'spec/SP-010.md': specDoc('SP-010', { supersedes: ['SP-999'] }) })
  expectOnly(errsOf(missing), /RG-015 supersedes 指到不存在的编号 `SP-999`/)

  const atTask = caseRoot({ 'spec/SP-010.md': specDoc('SP-010', { supersedes: ['T-050'] }), 'task/T-050.md': taskDoc('T-050') })
  expectOnly(errsOf(atTask), /RG-015 supersedes 指向 task/)

  const self = caseRoot({ 'spec/SP-010.md': specDoc('SP-010', { supersedes: ['SP-010'] }) })
  expectOnly(errsOf(self), /RG-015 supersedes 含自身/)

  const bothActive = caseRoot({
    'spec/SP-010.md': specDoc('SP-010', { supersedes: ['SP-020'] }),
    'spec/SP-020.md': specDoc('SP-020'),
  })
  expectOnly(errsOf(bothActive), /RG-015 status=active 的规范不得取代另一条 active 规范/)
  const retiredTarget = caseRoot({
    'spec/SP-010.md': specDoc('SP-010', { supersedes: ['SP-020'] }),
    'spec/SP-020.md': specDoc('SP-020', { status: 'retired', reason: '被 SP-010 取代' }),
  })
  assert.deepEqual(errsOf(retiredTarget), [], '旧条已 retired ⇒ 取代链合法（反向）')

  const cyc = caseRoot({
    'spec/SP-010.md': specDoc('SP-010', { status: 'retired', reason: '链', supersedes: ['SP-020'] }),
    'spec/SP-020.md': specDoc('SP-020', { status: 'retired', reason: '链', supersedes: ['SP-010'] }),
  })
  expectOnly(errsOf(cyc), /RG-015 supersedes 成环/)
})

test('RG-016：clears 指到不存在 / 指向非 task 变红；指向开放 task 放行（反向）', () => {
  const missing = caseRoot({ 'spec/SP-010.md': specDoc('SP-010', { clears: ['W99'] }) })
  expectOnly(errsOf(missing), /RG-016 clears 指到不存在的编号/)

  const wrongKind = caseRoot({
    'spec/SP-011.md': specDoc('SP-011', { clears: ['SP-012'] }),
    'spec/SP-012.md': specDoc('SP-012'),
  })
  expectOnly(errsOf(wrongKind), /RG-016 clears 指向 spec/)

  // 反向：清除任务**开放**（todo）不判红——clears 是指认不是前置，与 deps 的语义相反
  const base = caseRoot({
    'task/T-060.md': taskDoc('T-060'),
    'spec/SP-013.md': specDoc('SP-013', { clears: ['T-060'] }),
  })
  assert.deepEqual(errsOf(base), [], 'clears 指到 task 合法，且不要求其闭环')

  // 渲染：spec INDEX 的 active 表带「清除」列，clears 编号落列（列被删要有回归兜住）
  const { entries } = loadEntries(base)
  const idx = renderKindIndex('spec', entries)
  assert.match(idx, /\| 编号 \| 规范 \| 适用范围 \| 判据 \| 清除 \| 取代 \|/, 'active 表含清除列')
  assert.match(idx, /\| 人工 \| T-060 \| — \|/, 'clears 编号渲染进清除列')
})

// ================= adr kind（RG-017 / RG-018） =================
//
// `adr` 是与 `task` / `spec` 并列的第三个 kind，存在的核心理由只有一条：
// **把「这条决策还算不算数」（`status`）与「做到哪一步」（`impl`）拆成两个字段**。
// 所以它必须有独立的回归——退化成一列自由文本就等于这个 kind 白开了。

test('RG-017：adr 的 impl 词表外变红；合法值放行（反向）', () => {
  const bad = caseRoot({ 'adr/ADR-100.md': adrDoc('ADR-100', { impl: '半成品' }) })
  expectOnly(errsOf(bad), /RG-017 impl 不在词表/)

  const ok = caseRoot({ 'adr/ADR-101.md': adrDoc('ADR-101', { impl: 'partial' }) })
  assert.deepEqual(errsOf(ok), [], 'impl=partial 是合法词 ⇒ 放行（现行但只落地了一半）')
})

test('RG-017：adr 状态机（active / superseded / withdrawn）——superseded 必填 reason', () => {
  const badStatus = caseRoot({ 'adr/ADR-110.md': adrDoc('ADR-110', { status: 'retired', reason: 'x' }) })
  expectOnly(errsOf(badStatus), /RG-004 status="retired" 不在 adr 的状态词/)

  const noReason = caseRoot({ 'adr/ADR-111.md': adrDoc('ADR-111', { status: 'superseded' }) })
  expectOnly(errsOf(noReason), /RG-007 status=superseded 必须填 `reason`/)

  const withdrawn = caseRoot({ 'adr/ADR-112.md': adrDoc('ADR-112', { status: 'withdrawn', reason: '已回退' }) })
  assert.deepEqual(errsOf(withdrawn), [], 'withdrawn + reason ⇒ 放行（反向）')
})

test('RG-018：adr 字段与 id 不一致变红；一致则放行（反向）', () => {
  const idMismatch = caseRoot({ 'adr/ADR-120.md': adrDoc('ADR-120', { adr: '999' }) })
  expectOnly(errsOf(idMismatch), /RG-018 adr=`999` 与 id `ADR-120` 不一致/)

  const ok = caseRoot({ 'adr/ADR-122.md': adrDoc('ADR-122') })
  assert.deepEqual(errsOf(ok), [], 'id ↔ adr 对上 ⇒ 放行（反向）')
})

test('adr 渲染：status 分组、impl 成列（「现行但只落地一半」能在机读面表达）', () => {
  const base = caseRoot({
    'adr/ADR-200.md': adrDoc('ADR-200', { impl: 'partial', title: '半落地的现行决策' }),
    'adr/ADR-201.md': adrDoc('ADR-201', { status: 'superseded', reason: '被 ADR-200 取代' }),
  })
  const { entries } = loadEntries(base)
  const a = renderKindIndex('adr', entries)
  assert.equal(a, renderKindIndex('adr', entries), '同输入逐字节同输出')
  assert.match(a, /现行决策（active）/)
  assert.match(a, /已被取代（superseded）/)
  assert.match(a, /\| 编号 \| 决策 \| 域 \| 落地 \| 取代 \|/, 'active 表含「落地」列')
  assert.match(a, /ADR-200.*部分实施/, 'impl=partial 渲染成「部分实施」')
  assert.match(a, /计数.*active 1 · superseded 1/)
})

test('RG-015（adr）：取代链方向属于**取代者**——旧条 superseded 不写 supersedes', () => {
  // 方向语义：`supersedes` 是新条指认「我取代了谁」。这条回归钉的是**语义方向**——
  // 把 supersedes 写到被取代的旧条上是本仓真实出过的错（ADR-014/016 互指反了）。
  const ok = caseRoot({
    'adr/ADR-300.md': adrDoc('ADR-300', { supersedes: ['ADR-301'] }),
    'adr/ADR-301.md': adrDoc('ADR-301', { status: 'superseded', reason: '被 ADR-300 取代' }),
  })
  assert.deepEqual(errsOf(ok), [], '新条 supersedes 旧条（旧条 superseded）⇒ 合法')

  // 反向：两条都 active 时 supersedes 仍是未消解的矛盾（与 spec 同判）
  const bothActive = caseRoot({
    'adr/ADR-302.md': adrDoc('ADR-302', { supersedes: ['ADR-303'] }),
    'adr/ADR-303.md': adrDoc('ADR-303'),
  })
  expectOnly(errsOf(bothActive), /RG-015 status=active 的架构决策不得取代另一条 active 架构决策/)

  // 跨 kind 取代没有语义
  const crossKind = caseRoot({
    'adr/ADR-304.md': adrDoc('ADR-304', { supersedes: ['SP-010'] }),
    'spec/SP-010.md': specDoc('SP-010'),
  })
  expectOnly(errsOf(crossKind), /RG-015 supersedes 指向 spec（`SP-010`）——取代关系只在同 kind 之间/)
})

// ================= 状态矩阵（RG-007） =================
test('RG-007：done 必须有 evidence、不得留 signal；wont / retired 必须有 reason', () => {
  const noEvidence = caseRoot({ 'task/T-001.md': taskDoc('T-001', { status: 'done' }) })
  expectOnly(errsOf(noEvidence), /RG-007 status=done 必须填 `evidence`/)

  const withSignal = caseRoot({ 'task/T-001.md': taskDoc('T-001', { status: 'done', evidence: 'abc', signal: '快照' }) })
  expectOnly(errsOf(withSignal), /RG-007 status=done 不得带 `signal`/)

  const noReason = caseRoot({ 'task/T-001.md': taskDoc('T-001', { status: 'wont' }) })
  expectOnly(errsOf(noReason), /RG-007 status=wont 必须填 `reason`/)

  const retiredNoReason = caseRoot({ 'spec/SP-002.md': specDoc('SP-002', { status: 'retired' }) })
  expectOnly(errsOf(retiredNoReason), /RG-007 status=retired 必须填 `reason`/)

  const todoEvidence = caseRoot({ 'task/T-001.md': taskDoc('T-001', { evidence: 'abc' }) })
  expectOnly(errsOf(todoEvidence), /RG-007 status=todo 不得带 `evidence`/)
})

// ================= 正文小节与判据指认（RG-009 / RG-012） =================

test('RG-009：正文缺必有小节变红', () => {
  const base = caseRoot({ 'task/T-001.md': taskDoc('T-001', { body: '## 内容\n只有内容\n' }) })
  expectOnly(errsOf(base), /RG-009 正文缺 `## 处理过程`/)

  const spec = caseRoot({ 'spec/SP-002.md': specDoc('SP-002', { body: '## 规范\n只有条文\n' }) })
  expectOnly(errsOf(spec), /RG-009 正文缺 `## 判据`/)
})

test('RG-012：出口判据指认的脚本不存在变红；存在则放行（反向）', () => {
  const dead = caseRoot({ 'task/T-001.md': taskDoc('T-001', { exit: '由 `scripts/not-here.mjs` 守' }) })
  expectOnly(errsOf(dead), /RG-012 .*`scripts\/not-here\.mjs` 不存在/)

  const live = caseRoot({
    'task/T-001.md': taskDoc('T-001', { exit: '由 `scripts/registry-audit.mjs` 守' }),
  })
  // 占位脚本落在**仓根**（RG-012 按仓根相对路径解析），不是 docs/ 里
  fs.mkdirSync(path.join(live, 'scripts'), { recursive: true })
  fs.writeFileSync(path.join(live, 'scripts', 'registry-audit.mjs'), '// 夹具占位\n')
  assert.deepEqual(errsOf(live), [], '指认的脚本真实存在 ⇒ 放行')
})

// ================= 范围与命名空间（RG-010 / RG-013） =================

test('RG-010：扫不到任何 `.md` ⇒ exit 2（不退化成什么都没校验）', () => {
  const empty = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-registry-none-'))
  const r1 = runRegistryAudit([`--root=${empty}`])
  assert.equal(r1.code, 2)
  assert.match(stripAnsi(r1.lines.join('\n')), /RG-010 登记范围读不出/)
  fs.rmSync(empty, { recursive: true, force: true })

  // 有 `.md` 但没有登记 ⇒ 不是 exit 2 而是 exit 1（RG-020 指名道姓）
  const unregistered = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-registry-empty-'))
  fs.mkdirSync(path.join(unregistered, 'docs'), { recursive: true })
  fs.writeFileSync(path.join(unregistered, 'docs', 'x.md'), '# 裸文档\n')
  const r2 = runRegistryAudit([`--root=${unregistered}`])
  assert.equal(r2.code, 1)
  assert.match(stripAnsi(r2.lines.join('\n')), /docs\/x\.md：RG-020 未登记文档/)
  fs.rmSync(unregistered, { recursive: true, force: true })
})

test('RG-013：front-matter 的 kind 不在 KINDS 闭集变红（命名空间闭合）', () => {
  const base = caseRoot({ 'docs/weird/x.md': '---\nkind: weird\n---\n' })
  expectOnly(errsOf(base), /RG-013 docs\/weird\/x\.md：kind="weird" 不在 KINDS 闭集/)
})

// ================= SP-001 字段表对账（RG-011） =================

test('RG-011：SP-001 缺失 / 字段表缺行 变红；表 = ALL_KEYS 则放行', () => {
  const noSp = caseRoot({ 'task/T-001.md': taskDoc('T-001') }, { noSp: true })
  expectOnly(errsOf(noSp), /RG-011 .*SP-001\.md 不存在/)

  const drifted = caseRoot({ 'spec/SP-001.md': sp001Doc(ALL_KEYS.filter((k) => k !== 'scope').map((k) => `| \`${k}\` | 说明 |`)) })
  expectOnly(errsOf(drifted), /RG-011 .*表里缺：scope/)

  const ok = caseRoot({ 'task/T-001.md': taskDoc('T-001') })
  assert.deepEqual(errsOf(ok), [], '字段表恰好等于代码 schema ⇒ 放行')
})

// ================= 汇总渲染与 CLI =================

test('渲染确定性：同输入同输出；待处理列表里前置排在后继之前', () => {
  const base = caseRoot({
    'task/T-010.md': taskDoc('T-010'),
    'task/T-011.md': taskDoc('T-011', { deps: ['T-010'] }),
  })
  const { entries } = loadEntries(base)
  const a = renderKindIndex('task', entries)
  const b = renderKindIndex('task', entries)
  assert.equal(a, b, '同输入必须逐字节同输出（这是 autoWork 重生成的前提）')
  assert.ok(a.indexOf('`T-010`') < a.indexOf('`T-011`'), '前置任务必须排在后继之前')
  assert.match(a, /计数.*todo 2/)
})

test('排序：拓扑序永远在前；同层按 prio 再按编号（确定性）', () => {
  const base = caseRoot({
    'task/T-030.md': taskDoc('T-030', { prio: 'later' }),
    'task/T-031.md': taskDoc('T-031', { prio: 'now' }),
    'task/T-032.md': taskDoc('T-032'), // 缺席 = 最低档
    'task/T-033.md': taskDoc('T-033', { prio: 'now', deps: ['T-030'] }), // 有依赖 ⇒ 拓扑压过 prio
  })
  const { entries } = loadEntries(base)
  const out = renderKindIndex('task', entries)
  const pos = (id) => out.indexOf(`\`${id}\``)
  assert.ok(pos('T-030') < pos('T-033'), 'T-030 虽 later，但它是前置 ⇒ 排在 T-033 前（deps 是义务）')
  assert.ok(pos('T-031') < pos('T-030'), '同层按 prio：now 在 later 前')
  assert.ok(pos('T-030') < pos('T-032'), '缺席 = 最低档：later 在无 prio 前')
})

// ================= 在位文档（文档类 kind）=================
//
// 文档类与条目类的区别只有两条，各自必须有回归：
//   ① **文档即条目、在位存放**：编号 = 仓根相对路径（RG-003）、title = 正文首个 H1（RG-021）；
//   ② **目录不参与判定**：文档在哪个目录下与 kind 无关，搬迁不破机制（RG-002 反向用例）。
// 另加「全仓 .md 必须登记」的执行方（RG-020）与豁免面。

test('RG-020：未登记文档（无 front-matter / 缺 kind）变红', () => {
  const bare = caseRoot({ 'docs/plan/notes.md': '# 裸文档\n\n没有 front-matter。\n' })
  expectOnly(errsOf(bare), /docs\/plan\/notes\.md：RG-020 未登记文档/)

  const noKind = caseRoot({ 'docs/plan/notes.md': '---\nstatus: active\ncreated: 2026-10-11\n---\n\n# 裸文档\n' })
  expectOnly(errsOf(noKind), /docs\/plan\/notes\.md：RG-020 front-matter 缺 `kind`/)
})

test('RG-003（导航文档）：id 含 / 必须等于仓根相对路径；写错即红（反向：写对放行）', () => {
  const wrong = caseRoot({ 'docs/README.md': docDoc('docs/README.md', { kind: 'guide', id: 'docs/guide/README' }) })
  expectOnly(errsOf(wrong), /RG-003 导航文档 id 必须等于仓根相对路径（去 `\.md`）（应为 `docs\/README`）/)

  const ok = caseRoot({ 'docs/README.md': docDoc('docs/README.md', { kind: 'guide', id: 'docs/README' }) })
  assert.deepEqual(errsOf(ok), [], '导航文档 id = 路径 ⇒ 放行（反向）')
})

test('RG-021：title 必须逐字等于正文首个 H1；缺 H1 或不对账即红', () => {
  const mismatch = caseRoot({ 'docs/plan/x.md': docDoc('docs/plan/x.md', { title: '标题 A', body: '# 标题 B\n\n正文。\n' }) })
  expectOnly(errsOf(mismatch), /RG-021 title="标题 A" 与正文首个 H1 "标题 B" 不一致/)

  const noH1 = caseRoot({ 'docs/plan/x.md': docDoc('docs/plan/x.md', { title: '标题 A', body: '没有标题。\n' }) })
  expectOnly(errsOf(noH1), /RG-021 文档类缺正文首个 `# 标题`/)

  // 反向：代码围栏里的 `# ` 不算 H1（第一行 H1 才是 title 的来源）
  const fenced = caseRoot({ 'docs/plan/x.md': docDoc('docs/plan/x.md', { title: '标题 A', body: '```\n# 不是标题\n```\n\n# 标题 A\n' }) })
  assert.deepEqual(errsOf(fenced), [], '围栏内的 # 不算 H1 ⇒ 放行（反向）')
})

test('RG-020 豁免：archive / 生成页 / 指令面不进登记范围（反向）', () => {
  const base = caseRoot({
    'docs/archive/old.md': '# 历史快照（无 front-matter）\n',
    'docs/INDEX.md': '# 总账（生成页）\n',
    'docs/plan/INDEX.md': '# 计划索引（生成页）\n',
    '/AGENTS.md': '# 指令（无 front-matter）\n',
  })
  assert.deepEqual(errsOf(base), [], '豁免面不得误报')
})

test('文档类渲染：按 status 分组、编号列是可点的相对链接（从索引一步到正文）', () => {
  const base = caseRoot({
    'docs/plan/a.md': docDoc('docs/plan/a.md', { kind: 'plan', title: 'A 文档' }),
    'docs/plan/b.md': docDoc('docs/plan/b.md', { kind: 'plan', title: 'B 草案', status: 'draft' }),
    'docs/plan/c.md': docDoc('docs/plan/c.md', { kind: 'plan', title: 'C 废止', status: 'retired', reason: '被 A 取代' }),
  })
  const { entries } = loadEntries(base)
  const out = renderKindIndex('plan', entries)
  assert.equal(out, renderKindIndex('plan', entries), '同输入逐字节同输出')
  assert.match(out, /现行文档（active）/)
  assert.match(out, /草案（draft）/)
  assert.match(out, /废止（retired）/)
  assert.match(out, /\[`docs\/plan\/a`\]\(a\.md\)/, '同目录文档应渲染成同目录相对链接（从索引一步到正文）')
  // 跨目录：从 docs/plan/INDEX.md 到 docs/arch/x.md 必须写成 ../arch/x.md
  const cross = caseRoot({ 'docs/arch/x.md': docDoc('docs/arch/x.md', { title: 'X 文档' }) })
  assert.match(
    renderKindIndex('arch', loadEntries(cross).entries),
    /\[`docs\/arch\/x`\]\(x\.md\)/,
    '索引到正文的链接必须能从汇总页直接点开',
  )
  assert.match(out, /被 A 取代/, 'retired 表带理由列')
  assert.match(out, /计数.*active 1 · draft 1 · retired 1/)
})

test('--write：生成三个汇总页，再跑一次显示已是最新', () => {
  const base = caseRoot({ 'task/T-001.md': taskDoc('T-001', { status: 'done', evidence: 'abc1234' }) })
  const first = runRegistryAudit([`--root=${base}`, '--write'])
  assert.equal(first.code, 0, stripAnsi(first.lines.join('\n')))
  for (const rel of ['docs/INDEX.md', 'docs/task/INDEX.md', 'docs/spec/INDEX.md']) {
    assert.ok(fs.existsSync(path.join(base, rel)), `${rel} 应被生成`)
  }
  const board = fs.readFileSync(path.join(base, 'docs/task/INDEX.md'), 'utf8')
  assert.match(board, /`T-001`.*abc1234/, '已完成表应带证据列')
  assert.match(fs.readFileSync(path.join(base, 'docs/INDEX.md'), 'utf8'), /task · 任务/)

  const second = runRegistryAudit([`--root=${base}`, '--write'])
  assert.equal(second.code, 0)
  assert.match(stripAnsi(second.lines.join('\n')), /已是最新/, '第二次必须逐字节无变化')
})

test('registry-audit：违规时拒绝写汇总页（exit 1）；参数不识别 exit 2', () => {
  const base = caseRoot({ 'task/T-001.md': taskDoc('T-001', { status: 'done' }) })
  const r = runRegistryAudit([`--root=${base}`, '--write'])
  assert.equal(r.code, 1)
  assert.ok(!fs.existsSync(path.join(base, 'docs/task/INDEX.md')), '有违规不得写汇总页')
  assert.equal(runRegistryAudit(['--bogus']).code, 2)
})

test('registry-board：先校验后打印；有违规只报错不打印看板', () => {
  const bad = caseRoot({ 'task/T-001.md': taskDoc('T-001', { status: 'nope' }) })
  const rb = runRegistryBoard([`--root=${bad}`])
  assert.equal(rb.code, 1)
  const text = stripAnsi(rb.lines.join('\n'))
  assert.ok(!text.includes('登记看板'), '有违规时不得打印结论（报告说假话比没有更坏）')

  const ok = caseRoot({ 'task/T-010.md': taskDoc('T-010', { prio: 'now' }), 'spec/SP-002.md': specDoc('SP-002') })
  const r1 = runRegistryBoard([`--root=${ok}`])
  assert.equal(r1.code, 0, stripAnsi(r1.lines.join('\n')))
  const out = stripAnsi(r1.lines.join('\n'))
  assert.match(out, /登记看板/)
  assert.match(out, /☐ T-010/)
  assert.match(out, /\[meta·now\]/, '看板行应带 prio 标记')
  assert.match(out, /SP-002/)

  const single = runRegistryBoard([`--root=${ok}`, '--id=T-010'])
  assert.equal(single.code, 0)
  assert.match(stripAnsi(single.lines.join('\n')), /=== T-010（task · todo）===/)

  assert.equal(runRegistryBoard([`--root=${ok}`, '--kind=nope']).code, 2)
  assert.equal(runRegistryBoard(['--bogus']).code, 2)
})

// ================= 本仓真实登记（dogfood） =================

// 守卫对自己也要成立：本仓真实登记必须零违规，且**每一类 kind 都真的读到了条目**——
// 只断言「exit 0」挡不住「读空即全绿」（解析口径失效时什么条目都读不到），
// 只数总数也挡不住「某一类整体装载不到」——所以九个 kind 逐个断言非空。
test('本仓当前登记自洽，且每类 kind 都读到了条目', () => {
  const r = runRegistryAudit([`--root=${repoRoot}`])
  assert.equal(r.code, 0, stripAnsi(r.lines.join('\n')))
  const { entries } = loadEntries(repoRoot)
  const tasks = entries.filter((e) => e.kind === 'task')
  const specs = entries.filter((e) => e.kind === 'spec')
  const adrs = entries.filter((e) => e.kind === 'adr')
  assert.ok(tasks.length >= 3, `只读到 ${tasks.length} 个任务——装载口径或种子有问题`)
  assert.ok(specs.length >= 1, `只读到 ${specs.length} 个规范——SP-001 装载不到`)
  assert.ok(adrs.length >= 1, `只读到 ${adrs.length} 个决策——adr kind 装载不到`)
  for (const kind of Object.keys(KINDS)) {
    assert.ok(entries.filter((e) => e.kind === kind).length >= 1, `文档类 kind \`${kind}\` 一个条目都没读到`)
  }
  assert.ok(
    entries.some((e) => e.kind === 'arch' && e.rel.startsWith('docs/arch/')),
    '系统级文档必须真的进了 arch（在位文档装载口径）',
  )
  assert.ok(
    entries.some((e) => e.kind === 'arch' && e.rel.includes('symbio/src/')),
    '模块级文档（模块 README / docs）必须在同一机制内',
  )
  assert.ok(
    specs.some((e) => e.meta.id === 'SP-001'),
    'SP-001 必须在（RG-011 的对账对象）',
  )
})
