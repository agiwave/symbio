/**
 * sink-row-audit 回归测试
 *
 * 这一族的失效形态是**读不出 ⇒ 没什么可判 ⇒ 通过**，以及更隐蔽的一种：
 * **统计口径写宽**——把注释里的 `by: None`、`produced_by: None`、测试夹具都算进来。
 * 两者都会让「生产里有几条归宿行」这个读数失去意义，而门禁照样全绿。
 *
 * 因此下面每条判据都注入真实违规并断言「红的是那一条码」，另有三条**不该误红**的
 * 路径：注释 / 测试 / `produced_by` 不计数、常量与别名能解析、`--print-baseline` 可读。
 *
 * 跑法：node --test scripts/sink-row-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./sink-row-audit.mjs', import.meta.url))

const DECL = 'crates/demo/src/decl.rs'
const BASELINE_REL = 'scripts/baselines/sink-rows.json'
const DOC = 'docs/003-design/000-fixture.md'

/** 干净夹具：两条生产归宿行（一条字面量、一条常量），与基线逐条相等。 */
function cleanFiles() {
  return {
    [DECL]: [
      'pub const EV: &str = "demo.event";',
      '',
      'pub(crate) const ROWS: &[Decl] = &[',
      '    Decl { name: "demo.literal", by: None },',
      '    Decl { name: EV, by: None },',
      '];',
      '',
    ].join('\n'),
    [DOC]: '### D-397 夹具决策\n',
  }
}

function cleanBaseline() {
  return {
    note: 'fixture',
    rows: [
      { name: 'demo.literal', why: '夹具：写即闭合', adr: 'D-397' },
      { name: 'demo.event', why: '夹具：写即闭合', adr: 'D-397' },
    ],
  }
}

function sandbox(files, baseline = cleanBaseline()) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'sink-row-'))
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
  if (baseline !== null) {
    const abs = path.join(root, BASELINE_REL)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, JSON.stringify(baseline, null, 2))
  }
  return { root, cleanup: () => fs.rmSync(root, { recursive: true, force: true }) }
}

function run(root, extra = []) {
  const r = spawnSync(process.execPath, [script, `--root=${root}`, ...extra], {
    encoding: 'utf8',
    env: { ...process.env, NO_COLOR: '1' },
    timeout: 30_000,
  })
  return { status: r.status, out: `${r.stdout}${r.stderr}` }
}

test('干净夹具：生产两条 = 基线两条 ⇒ 通过，且读数写清楚', () => {
  const box = sandbox(cleanFiles())
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 0, out)
    assert.match(out, /生产 2 条归宿行，基线 2 条/)
    assert.match(out, /棘轮读数 2\/2/)
  } finally {
    box.cleanup()
  }
})

test('注释 / produced_by / *.test.rs / #[cfg(test)] 都不计数', () => {
  const files = cleanFiles()
  files[DECL] += [
    '// Decl { name: "comment.row", by: None } —— 这是注释，不是声明',
    'const NOISE: &str = "produced_by: None";',
    'Decl { name: "produced.by", produced_by: None },',
    '',
  ].join('\n')
  files['crates/demo/src/decl.test.rs'] = 'const R: &[Decl] = &[Decl { name: "test.row", by: None }];\n'
  files['crates/demo/src/other.rs'] =
    '#[cfg(test)]\nmod tests {\n    const R: &[Decl] = &[Decl { name: "cfg.test", by: None }];\n}\n'
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 0, out)
    assert.match(out, /生产 2 条归宿行/)
    assert.doesNotMatch(out, /comment\.row|produced\.by|test\.row|cfg\.test/)
  } finally {
    box.cleanup()
  }
})

test('SR-001：新增一条未登记的归宿行 ⇒ 红，并指出「为什么不需要派发」必须回答', () => {
  const files = cleanFiles()
  files[DECL] = files[DECL].replace(
    '    Decl { name: EV, by: None },',
    '    Decl { name: EV, by: None },\n    Decl { name: "demo.extra", by: None },',
  )
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /SR-001/)
    assert.match(out, /demo\.extra/)
    assert.match(out, /为什么不需要派发/)
  } finally {
    box.cleanup()
  }
})

test('SR-001 按**出现次数**判：同名行多出一条也红（不是按名字集合）', () => {
  const files = cleanFiles()
  files[DECL] = files[DECL].replace(
    '    Decl { name: EV, by: None },',
    '    Decl { name: EV, by: None },\n    Decl { name: "demo.literal", by: None },',
  )
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /SR-001.*demo\.literal/)
    assert.match(out, /生产 2 条 \/ 基线 1 条/)
  } finally {
    box.cleanup()
  }
})

test('SR-002：基线有而生产没有 ⇒ 红（防「删一条再加一条」抹平总数）', () => {
  const files = cleanFiles()
  files[DECL] = files[DECL].replace('    Decl { name: "demo.literal", by: None },\n', '')
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /SR-002/)
    assert.match(out, /demo\.literal/)
  } finally {
    box.cleanup()
  }
})

test('SR-003：基线理由为空、决策号不存在 ⇒ 两条都红', () => {
  const baseline = cleanBaseline()
  baseline.rows[0].why = '   '
  baseline.rows[1].adr = 'D-999'
  const box = sandbox(cleanFiles(), baseline)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /SR-003.*demo\.literal.*why/)
    assert.match(out, /SR-003.*demo\.event.*D-999/)
  } finally {
    box.cleanup()
  }
})

test('SR-004：name 是解析不出的常量 ⇒ 红（不许静默跳过这一行）', () => {
  const files = cleanFiles()
  files[DECL] = files[DECL].replace(
    '    Decl { name: EV, by: None },',
    '    Decl { name: EV, by: None },\n    Decl { name: UNKNOWN_CONST, by: None },',
  )
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /SR-004/)
    assert.match(out, /UNKNOWN_CONST/)
  } finally {
    box.cleanup()
  }
})

test('常量别名可以解析：`use … as ALIAS` 与 `const … = \"…\"` 对得上 ⇒ 绿', () => {
  const files = {
    'crates/demo/src/consts.rs': 'pub const REAL: &str = "demo.aliased";\n',
    [DECL]: [
      'use crate::consts::REAL as ALIAS;',
      'pub(crate) const ROWS: &[Decl] = &[',
      '    Decl { name: ALIAS, by: None },',
      '];',
      '',
    ].join('\n'),
    [DOC]: '### D-397 夹具决策\n',
  }
  const baseline = { note: 'fixture', rows: [{ name: 'demo.aliased', why: '夹具', adr: 'D-397' }] }
  const box = sandbox(files, baseline)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 0, out)
    assert.match(out, /生产 1 条归宿行/)
    assert.match(out, /demo\.aliased/)
  } finally {
    box.cleanup()
  }
})

test('基线缺失 ⇒ 退 2（读不出不许当 0 条通过）', () => {
  const box = sandbox(cleanFiles(), null)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 2, out)
    assert.match(out, /范围读不出/)
    assert.match(out, /基线缺失/)
  } finally {
    box.cleanup()
  }
})

test('crates/ 缺失 ⇒ 退 2', () => {
  const files = { [DOC]: '### D-397 夹具决策\n' }
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 2, out)
    assert.match(out, /范围读不出/)
    assert.match(out, /crates\//)
  } finally {
    box.cleanup()
  }
})

test('--print-baseline：输出当前生产清单，已有 why/adr 原样保留', () => {
  const box = sandbox(cleanFiles())
  try {
    const { status, out } = run(box.root, ['--print-baseline'])
    assert.equal(status, 0, out)
    const parsed = JSON.parse(out)
    assert.deepEqual(
      parsed.rows.map((r) => r.name).sort(),
      ['demo.event', 'demo.literal'],
    )
    assert.equal(parsed.rows.find((r) => r.name === 'demo.event').adr, 'D-397')
  } finally {
    box.cleanup()
  }
})
