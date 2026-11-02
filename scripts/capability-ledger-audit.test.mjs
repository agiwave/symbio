/**
 * capability-ledger-audit 回归测试 —— 证明 EX-006 **每种漂移都会红**
 *
 * `EX-006` 的判定读两份文档：`DE-008 §6.1` 的名册与 `DE-026 §2.3` 的逐行处置账。它守的
 * 不是「有没有这张表」，而是两张表**指向同一件事**。这条守卫的失效形态全是
 * 「看起来在算」：
 *
 * - 名册表 / 账本表读不出 ⇒ 最省事的实现报「没有违规」（本文件第一条反向用例钉它）；
 * - 名字对账把括号说明算进名字 ⇒ 第一行永远漏行，守卫被假红喂到关掉；
 * - 合计表只数总数不对类别 ⇒ 一条「拆」写成「留」仍能凑出总数；
 * - 「留守」只判非空 ⇒ 「难拆」也能过关，D-246 的被否项失去判据。
 *
 * 用法：node --test scripts/capability-ledger-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./capability-ledger-audit.mjs', import.meta.url))

const ROSTER = 'docs/003-design/008-能力注册表与能力坐标系.md'
const LEDGER = 'docs/003-design/026-能力拆分与影响类.md'

const ROSTER_TEXT = [
  '# 能力名册（夹具）',
  '',
  '| 能力 | 维 | 轴 | 天梯 | 等级 | 类型 | 落点 | 锚点 |',
  '|---|---|---|:-:|:-:|:-:|:-:|---|',
  '| 甲能力（≈4 槽） | 知 | 抽象层级 | T1 | L1 | 认知 | S01 | — |',
  '| 乙能力 | 知 | 时域跨度 | T3 | L3 | 认知 | S06 | — |',
  '| 丙能力 | 行 | 自主程度 | T4 | L2 | 治理 | S09 | — |',
  '',
  '### 6.1.1 别的表',
  '',
].join('\n')

const LEDGER_TEXT = [
  '---',
  'kind: design',
  'id: DE-026-能力拆分与影响类',
  'created: 2026-10-29',
  '---',
  '',
  '# 处置账（夹具）',
  '',
  '### 账本（3/3）',
  '',
  '| # | 能力 | 今天 | 处置 | 落点 / 理由 | 归 |',
  '|:-:|---|---|---|---|:-:|',
  '| 1 | 甲能力 | loop `a.rs` | 拆 | `symbio-plugin-a`：I-1 | S6 |',
  '| 2 | 乙能力 | core `b.rs` | 留 守门 | 封闭语法；第二事实源不允许 | — |',
  '| 3 | 丙能力 | 未落地 | 待裁决 | 载荷形状未定 | S10 |',
  '',
  '### 合计',
  '',
  '| 处置 | 条数 | 组成 |',
  '|---|:-:|---|',
  '| 拆 | 1 | 1 |',
  '| 留守 | 1 | 2 |',
  '| 待裁决 | 1 | 3 |',
  '',
].join('\n')

function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

function writeTree(root, files) {
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
}

function audit(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ledger-audit-'))
  try {
    writeTree(root, files)
    return spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
    })
  } finally {
    rmTree(root)
  }
}

function cleanTree() {
  return { [ROSTER]: ROSTER_TEXT, [LEDGER]: LEDGER_TEXT }
}

const PLAN_ROW =
  '| 3 | 丙能力 | 未落地 | 拆 | `symbio-plugin-c`：读数视图；骨架（T-090）：期限 2026-12-31；触发 第二条读数型能力进入 S9；承接 T-091 | S9 |'

function taskFile(id, title, status, deps) {
  return [
    '---',
    'kind: task',
    `id: ${id}`,
    `title: ${title}`,
    `status: ${status}`,
    'created: 2026-10-29',
    `deps: [${deps.join(', ')}]`,
    '---',
    '',
    `# ${title}`,
    '',
  ].join('\n')
}

/** 干净夹具 + 一条骨架计划：宿主任务与承接任务（后者的 `deps` 指向宿主）。 */
function planTree() {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER]
    .replace('| 3 | 丙能力 | 未落地 | 待裁决 | 载荷形状未定 | S10 |', PLAN_ROW)
    .replace('| 拆 | 1 | 1 |', '| 拆 | 2 | 1/3 |')
    .replace('| 待裁决 | 1 | 3 |', '| 待裁决 | 0 | |')
  files['docs/007-task/090-骨架宿主.md'] = taskFile('T-090-骨架宿主', '骨架宿主', 'todo', [])
  files['docs/007-task/091-承接任务.md'] = taskFile('T-091-承接任务', '承接任务', 'todo', ['T-090-骨架宿主'])
  return files
}

function assertRed(r, needle) {
  assert.equal(r.status, 1, r.stdout + r.stderr)
  assert.match(r.stdout, /\[ERROR\] EX-006 /, r.stdout)
  assert.match(r.stdout, new RegExp(needle), r.stdout)
}

test('干净夹具：名册与账本逐行对上，读数是 3 行', () => {
  const r = audit(cleanTree())
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /名册 3 行 = 账本 3 行（拆 1 · 留守 1 · 待裁决 1）/)
})

test('括号说明不参与名字对账（工作记忆（≈4 槽）↔ 工作记忆）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('| 1 | 甲能力 |', '| 1 | 甲能力（≈4 槽） |')
  const r = audit(files)
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('名册有一行而账本漏了 ⇒ 红（漏行）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('| 3 | 丙能力 | 未落地 | 待裁决 | 载荷形状未定 | S10 |\n', '')
  assertRed(audit(files), /漏一行|没有处置行/)
})

test('账本多出一行名册没有的能力 ⇒ 红（凭空多一行）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace(
    '| 3 | 丙能力 | 未落地 | 待裁决 | 载荷形状未定 | S10 |',
    '| 3 | 丙能力 | 未落地 | 待裁决 | 载荷形状未定 | S10 |\n| 4 | 丁能力 | 未落地 | 拆 | `symbio-plugin-d` | S9 |',
  )
  assertRed(audit(files), /不在 .*§6\.1 名册里/)
})

test('同一能力两条处置行 ⇒ 红（重复行）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace(
    '| 3 | 丙能力 | 未落地 | 待裁决 | 载荷形状未定 | S10 |',
    '| 3 | 丙能力 | 未落地 | 待裁决 | 载荷形状未定 | S10 |\n| 4 | 乙能力 | core `b.rs` | 留 守门 | 封闭语法 | — |',
  )
  assertRed(audit(files), /有 2 条处置行/)
})

test('处置不在闭集 ⇒ 红（不认识的词不能自己加）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('| 留 守门 |', '| 冻结 |')
  assertRed(audit(files), /不在闭集/)
})

test('记为拆却指不出 crate ⇒ 红（拆只是一句承诺）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('`symbio-plugin-a`：I-1', '后面再说')
  assertRed(audit(files), /落点指不出 crate/)
})

test('与 #N 同包也算落点（共用一份实现是合法的）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('`symbio-plugin-a`：I-1', '与 #2 同包（同一实现）')
  const r = audit(files)
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('留守理由用「难拆」⇒ 红（D-246 的否项不是留守理由）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('封闭语法；第二事实源不允许', '难拆')
  assertRed(audit(files), /用了 D-246 的否项/)
})

test('留守不写理由 ⇒ 红', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('封闭语法；第二事实源不允许', ' ')
  assertRed(audit(files), /留守但没写理由/)
})

test('待裁决的归宿不是 S10 ⇒ 红（悬空的待裁决等于没人管）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('待裁决 | 载荷形状未定 | S10 |', '待裁决 | 载荷形状未定 | — |')
  assertRed(audit(files), /不是 S10/)
})

test('待裁决不写为什么悬着 ⇒ 红', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('载荷形状未定', ' ')
  assertRed(audit(files), /没写为什么悬着/)
})

test('记为拆却不写交付段 ⇒ 红（目标存在但没人认领）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('`symbio-plugin-a`：I-1 | S6 |', '`symbio-plugin-a`：I-1 | — |')
  assertRed(audit(files), /交付段 .* 不在闭集/)
})

test('交付段不在 S6…S9 ⇒ 红（凭空多出一个段）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('`symbio-plugin-a`：I-1 | S6 |', '`symbio-plugin-a`：I-1 | S5 |')
  assertRed(audit(files), /交付段 .*S5.* 不在闭集/)
})

test('留守却挂了交付段 ⇒ 红（不建任务就不能冒充要拆）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('封闭语法；第二事实源不允许 | — |', '封闭语法；第二事实源不允许 | S6 |')
  assertRed(audit(files), /却挂了交付段/)
})

test('合计表的条数与组成列不同源 ⇒ 红', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('| 拆 | 1 | 1 |', '| 拆 | 2 | 1 |')
  assertRed(audit(files), /条数与名单不同源/)
})

test('合计表把小计改大 ⇒ 红（汇总与逐行账不同源）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('| 留守 | 1 | 2 |', '| 留守 | 2 | 2 |')
  assertRed(audit(files), /小计 2 ≠ 逐行账的 1/)
})

test('合计表把「留」写进「拆」的名单 ⇒ 红（类别对不上）', () => {
  const files = cleanTree()
  files[LEDGER] = files[LEDGER].replace('| 拆 | 1 | 1 |', '| 拆 | 2 | 1/2 |').replace('| 留守 | 1 | 2 |', '| 留守 | 0 | |')
  assertRed(audit(files), /归为 拆，逐行账的处置是 `留 守门`/)
})

test('名册表读不出 ⇒ 红（判据没有输入）', () => {
  const files = cleanTree()
  delete files[ROSTER]
  assertRed(audit(files), /名册表读不出/)
})

test('账本表读不出 ⇒ 红', () => {
  const files = cleanTree()
  delete files[LEDGER]
  assertRed(audit(files), /处置账表读不出/)
})

// ── D-294 移除计划 ───────────────────────────────────────────────────

test('骨架计划四处齐全、承接任务可经 deps 到达宿主 ⇒ 绿', () => {
  const r = audit(planTree())
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('写了骨架但格式不齐 ⇒ 红（没有可判计划）', () => {
  const files = planTree()
  files[LEDGER] = files[LEDGER].replace('；触发 第二条读数型能力进入 S9；承接 T-091', '；以后再说')
  assertRed(audit(files), /没有可判计划/)
})

test('骨架宿主任务不存在 ⇒ 红', () => {
  const files = planTree()
  files[LEDGER] = files[LEDGER].replace('骨架（T-090）', '骨架（T-099）')
  assertRed(audit(files), /骨架宿主 T-099 不在/)
})

test('承接任务不存在 ⇒ 红（计划没有接棒的人）', () => {
  const files = planTree()
  files[LEDGER] = files[LEDGER].replace('承接 T-091', '承接 T-999')
  assertRed(audit(files), /承接任务 T-999 不在/)
})

test('承接任务的 deps 闭包到不了骨架宿主 ⇒ 红', () => {
  const files = planTree()
  files['docs/007-task/091-承接任务.md'] = taskFile('T-091-承接任务', '承接任务', 'todo', [])
  assertRed(audit(files), /闭包到不了骨架宿主/)
})

test('承接任务已收口 ⇒ 红（承诺落在一条不会再动的任务上）', () => {
  const files = planTree()
  files['docs/007-task/091-承接任务.md'] = taskFile('T-091-承接任务', '承接任务', 'done', ['T-090-骨架宿主'])
  assertRed(audit(files), /承诺落在一条已收口的任务上/)
})

test('期限不晚于账本登记日 ⇒ 红（等于没有期限）', () => {
  const files = planTree()
  files[LEDGER] = files[LEDGER].replace('期限 2026-12-31', '期限 2026-01-01')
  assertRed(audit(files), /不晚于账本登记日/)
})

test('触发条件用空话 ⇒ 红（不可判定的词不是触发）', () => {
  const files = planTree()
  files[LEDGER] = files[LEDGER].replace('触发 第二条读数型能力进入 S9', '触发 以后再说')
  assertRed(audit(files), /不可判定的词/)
})

test('触发条件没有可观测谓词 ⇒ 红', () => {
  const files = planTree()
  files[LEDGER] = files[LEDGER].replace('触发 第二条读数型能力进入 S9', '触发 第二条读数型能力')
  assertRed(audit(files), /没有可观测谓词/)
})

test('承接任务就是骨架宿主 ⇒ 红（自己接自己等于没人接）', () => {
  const files = planTree()
  files[LEDGER] = files[LEDGER].replace('承接 T-091', '承接 T-090')
  assertRed(audit(files), /自己接自己等于没人接/)
})

test('骨架行不写拆 ⇒ 红（骨架不是留守的别名）', () => {
  const files = planTree()
  files[LEDGER] = files[LEDGER].replace(
    '| 3 | 丙能力 | 未落地 | 拆 | `symbio-plugin-c`：读数视图；骨架（T-090）：期限 2026-12-31；触发 第二条读数型能力进入 S9；承接 T-091 | S9 |',
    '| 3 | 丙能力 | 未落地 | 留 守门 | 骨架（T-090）：期限 2026-12-31；触发 第二条读数型能力进入 S9；承接 T-091 | — |',
  )
  assertRed(audit(files), /骨架是「先留一层、到期拆掉」的拆法/)
})

test('有骨架但任务表里没有宿主 ⇒ 红（计划没有承接面）', () => {
  const files = planTree()
  delete files['docs/007-task/090-骨架宿主.md']
  delete files['docs/007-task/091-承接任务.md']
  files['docs/007-task/000-占位.md'] = taskFile('T-000-占位', '占位', 'todo', [])
  assertRed(audit(files), /骨架宿主 T-090 不在/)
})
