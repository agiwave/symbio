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
import { ALL_KEYS, parseEntryText, loadEntries, auditEntries, renderKindIndex } from './registry-lib.mjs'
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
 * 一个用例一座目录：扫描范围是整个 registry/，夹具共用目录会互相串断言。
 * 默认自带 SP-001（RG-011 对它有要求）；`noSp` 用于测 SP-001 缺失。
 */
function caseRoot(files, { noSp = false } = {}) {
  const base = path.join(tmp, `case${++caseNo}`)
  const all = { ...files }
  if (!noSp && !all['spec/SP-001.md']) all['spec/SP-001.md'] = sp001Doc()
  for (const [rel, content] of Object.entries(all)) {
    const p = path.join(base, 'docs', 'registry', rel)
    fs.mkdirSync(path.dirname(p), { recursive: true })
    fs.writeFileSync(p, content)
  }
  return base
}

function errsOf(base) {
  const { entries, errors } = loadEntries(path.join(base, 'docs', 'registry'), base)
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

test('RG-002：必填字段缺失变红；kind 与目录不一致变红', () => {
  const missing = caseRoot({ 'task/T-001.md': fm({ kind: 'task', id: 'T-001', title: 'x', status: 'todo', created: '2026-10-11', deps: [] }) + TASK_BODY })
  expectOnly(errsOf(missing), /RG-002 必填字段 `track`/)

  const mismatch = caseRoot({ 'task/T-001.md': taskDoc('T-001', { kind: 'spec' }) })
  expectOnly(errsOf(mismatch), /RG-002 kind=spec 与所在目录不一致/)
})

test('RG-003：id 形状 / 文件名对账 / 跨 kind 重复 各自变红', () => {
  const shape = caseRoot({ 'task/bad-id.md': taskDoc('1bad', { body: undefined }) })
  expectOnly(errsOf(shape), /RG-003 id 不合形状/)

  // 历史批次号含圈号（S-end①）：编号是身份、不得重排，ID 形状必须放行 Unicode 字母数字
  const circle = caseRoot({ 'task/S-end①.md': taskDoc('S-end①') })
  assert.deepEqual(errsOf(circle), [], 'S-end① 必须合法')

  const renamed = caseRoot({ 'task/wrong.md': taskDoc('T-040') })
  expectOnly(errsOf(renamed), /RG-003 文件名必须等于编号（应为 `T-040\.md`）/)

  const dup = caseRoot({ 'spec/T-050.md': specDoc('T-050'), 'task/T-050.md': taskDoc('T-050') })
  expectOnly(errsOf(dup), /RG-003 编号 `T-050` 重复/)
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
  const missing = caseRoot({ 'spec/WVR-10.md': specDoc('WVR-10', { clears: ['W99'] }) })
  expectOnly(errsOf(missing), /RG-016 clears 指到不存在的编号/)

  const wrongKind = caseRoot({
    'spec/WVR-11.md': specDoc('WVR-11', { clears: ['SP-010'] }),
    'spec/SP-010.md': specDoc('SP-010'),
  })
  expectOnly(errsOf(wrongKind), /RG-016 clears 指向 spec/)

  // 反向：清除任务**开放**（todo）不判红——clears 是指认不是前置，与 deps 的语义相反
  const base = caseRoot({
    'task/T-060.md': taskDoc('T-060'),
    'spec/WVR-12.md': specDoc('WVR-12', { clears: ['T-060'] }),
  })
  assert.deepEqual(errsOf(base), [], 'clears 指到 task 合法，且不要求其闭环')

  // 渲染：spec INDEX 的 active 表带「清除」列，clears 编号落列（列被删要有回归兜住）
  const { entries } = loadEntries(path.join(base, 'docs', 'registry'), base)
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
  const { entries } = loadEntries(path.join(base, 'docs', 'registry'), base)
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
  // 占位脚本落在**仓根**（RG-012 按仓根相对路径解析），不是 registry/ 里
  fs.mkdirSync(path.join(live, 'scripts'), { recursive: true })
  fs.writeFileSync(path.join(live, 'scripts', 'registry-audit.mjs'), '// 夹具占位\n')
  assert.deepEqual(errsOf(live), [], '指认的脚本真实存在 ⇒ 放行')
})

// ================= 范围与命名空间（RG-010 / RG-013） =================

test('RG-010：registry/ 缺失或零条目 ⇒ exit 2（不退化成什么都没校验）', () => {
  const missing = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-registry-none-'))
  const r1 = runRegistryAudit([`--root=${missing}`])
  assert.equal(r1.code, 2)
  assert.match(stripAnsi(r1.lines.join('\n')), /RG-010 docs\/registry\/ 不存在/)
  fs.rmSync(missing, { recursive: true, force: true })

  const emptyRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-registry-empty-'))
  fs.mkdirSync(path.join(emptyRoot, 'docs', 'registry'), { recursive: true })
  const r2 = runRegistryAudit([`--root=${emptyRoot}`])
  assert.equal(r2.code, 2)
  assert.match(stripAnsi(r2.lines.join('\n')), /RG-010 .*没有任何条目/)
  fs.rmSync(emptyRoot, { recursive: true, force: true })
})

test('RG-013：registry/ 下未登记的 kind 目录变红（命名空间闭合）', () => {
  const base = caseRoot({ 'weird/x.md': '---\nkind: weird\n---\n' })
  expectOnly(errsOf(base), /RG-013 docs\/registry\/weird\//)
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
  const { entries } = loadEntries(path.join(base, 'docs', 'registry'), base)
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
  const { entries } = loadEntries(path.join(base, 'docs', 'registry'), base)
  const out = renderKindIndex('task', entries)
  const pos = (id) => out.indexOf(`\`${id}\``)
  assert.ok(pos('T-030') < pos('T-033'), 'T-030 虽 later，但它是前置 ⇒ 排在 T-033 前（deps 是义务）')
  assert.ok(pos('T-031') < pos('T-030'), '同层按 prio：now 在 later 前')
  assert.ok(pos('T-030') < pos('T-032'), '缺席 = 最低档：later 在无 prio 前')
})

test('--write：生成三个汇总页，再跑一次显示已是最新', () => {
  const base = caseRoot({ 'task/T-001.md': taskDoc('T-001', { status: 'done', evidence: 'abc1234' }) })
  const first = runRegistryAudit([`--root=${base}`, '--write'])
  assert.equal(first.code, 0, stripAnsi(first.lines.join('\n')))
  for (const rel of ['docs/registry/INDEX.md', 'docs/registry/task/INDEX.md', 'docs/registry/spec/INDEX.md']) {
    assert.ok(fs.existsSync(path.join(base, rel)), `${rel} 应被生成`)
  }
  const board = fs.readFileSync(path.join(base, 'docs/registry/task/INDEX.md'), 'utf8')
  assert.match(board, /`T-001`.*abc1234/, '已完成表应带证据列')
  assert.match(fs.readFileSync(path.join(base, 'docs/registry/INDEX.md'), 'utf8'), /task · 任务/)

  const second = runRegistryAudit([`--root=${base}`, '--write'])
  assert.equal(second.code, 0)
  assert.match(stripAnsi(second.lines.join('\n')), /已是最新/, '第二次必须逐字节无变化')
})

test('registry-audit：违规时拒绝写汇总页（exit 1）；参数不识别 exit 2', () => {
  const base = caseRoot({ 'task/T-001.md': taskDoc('T-001', { status: 'done' }) })
  const r = runRegistryAudit([`--root=${base}`, '--write'])
  assert.equal(r.code, 1)
  assert.ok(!fs.existsSync(path.join(base, 'docs/registry/task/INDEX.md')), '有违规不得写汇总页')
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

// 守卫对自己也要成立：本仓真实 registry/ 必须零违规，且条目数非零——
// 只断言「exit 0」挡不住「读空即全绿」（解析口径失效时什么条目都读不到）。
test('本仓当前登记自洽，且真的读到了条目', () => {
  const r = runRegistryAudit([`--root=${repoRoot}`])
  assert.equal(r.code, 0, stripAnsi(r.lines.join('\n')))
  const { entries } = loadEntries(path.join(repoRoot, 'docs', 'registry'), repoRoot)
  const tasks = entries.filter((e) => e.kind === 'task')
  const specs = entries.filter((e) => e.kind === 'spec')
  const adrs = entries.filter((e) => e.kind === 'adr')
  assert.ok(tasks.length >= 3, `只读到 ${tasks.length} 个任务——装载口径或种子有问题`)
  assert.ok(specs.length >= 1, `只读到 ${specs.length} 个规范——SP-001 装载不到`)
  assert.ok(adrs.length >= 1, `只读到 ${adrs.length} 个决策——adr kind 装载不到`)
  assert.ok(
    specs.some((e) => e.meta.id === 'SP-001'),
    'SP-001 必须在（RG-011 的对账对象）',
  )
})
