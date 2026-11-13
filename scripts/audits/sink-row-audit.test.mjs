/**
 * sink-row-audit 回归测试
 *
 * 这一族的失效形态是**读不出 ⇒ 没什么可判 ⇒ 通过**，以及更隐蔽的一种：
 * **统计口径写宽**——把注释里的行、测试夹具、`produced_by: None` 都算进来。
 * 两者都会让「生产里有几条归宿行」这个读数失去意义，而门禁照样全绿。
 *
 * `D-306` 之后口径换了对象：归宿行不再是「写了 `by: None` 的行」，而是**一笔债都
 * 开不出来**的行（`owes` 被 `until` 同源同键、出口不带 eq 地完整覆盖）。因此下面除了
 * 各条判据的违规注入，还要断两件事：会开账的行**不许**被算成归宿行；判不了的行
 * （`owes` 或 `until` 不是字面量）**不许**静默消失。
 *
 * 跑法：node --test scripts/audits/sink-row-audit.test.mjs
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

/** 一条「开不出账」的行：`owes` 与 `until` 同源同键，出口不带等值约束。 */
function sinkRow(name, source = '"demo.event"') {
  return [
    `    Decl {`,
    `        name: ${name},`,
    `        reads: &[],`,
    `        tier: LatencyTier::Reflex,`,
    `        expand: None,`,
    `        owes: Pattern { source: ${source}, key: &["id"], eq: &[] },`,
    `        until: &[Pattern { source: ${source}, key: &["id"], eq: &[] }],`,
    `        unless: &[],`,
    `    },`,
  ].join('\n')
}

/** 一条**会开账**的行：出口换了个源 ⇒ 债开得出来 ⇒ 不是归宿行。 */
function liveRow(name) {
  return [
    `    Decl {`,
    `        name: ${name},`,
    `        reads: &[],`,
    `        tier: LatencyTier::Reflex,`,
    `        expand: None,`,
    `        owes: Pattern { source: "demo.owed", key: &["id"], eq: &[] },`,
    `        until: &[Pattern { source: "demo.settled", key: &["id"], eq: &[] }],`,
    `        unless: &[],`,
    `    },`,
  ].join('\n')
}

/** 干净夹具：两条生产归宿行（一条字面量、一条常量），与基线逐条相等。 */
function cleanFiles() {
  return {
    [DECL]: [
      'pub const EV: &str = "demo.event";',
      '',
      'pub(crate) const ROWS: &[Decl] = &[',
      sinkRow('"demo.literal"'),
      sinkRow('EV'),
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

test('会开账的行不算归宿行（出口不同源 ⇒ 债开得出来）', () => {
  const files = cleanFiles()
  files[DECL] = files[DECL].replace(sinkRow('EV'), `${sinkRow('EV')}\n${liveRow('"demo.live"')}`)
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 0, out)
    assert.match(out, /生产 2 条归宿行/)
    assert.doesNotMatch(out, /demo\.live/)
  } finally {
    box.cleanup()
  }
})

test('出口带等值约束的行不算归宿行（只覆盖一个端口，别的照样欠）', () => {
  const files = cleanFiles()
  files[DECL] = files[DECL].replace(
    sinkRow('EV'),
    [
      sinkRow('EV'),
      '    Decl {',
      '        name: "demo.eq",',
      '        reads: &[],',
      '        tier: LatencyTier::Reflex,',
      '        expand: None,',
      '        owes: Pattern { source: "demo.event", key: &["id"], eq: &[] },',
      '        until: &[Pattern { source: "demo.event", key: &["id"], eq: &[("x", Lit::Str("a"))] }],',
      '        unless: &[],',
      '    },',
    ].join('\n'),
  )
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 0, out)
    assert.match(out, /生产 2 条归宿行/)
    assert.doesNotMatch(out, /demo\.eq/)
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
  files['crates/demo/src/decl.test.rs'] = `const R: &[Decl] = &[${sinkRow('"test.row"')}];\n`
  files['crates/demo/src/other.rs'] = `#[cfg(test)]\nmod tests {\n    const R: &[Decl] = &[${sinkRow('"cfg.test"')}];\n}\n`
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

test('类型定义（`struct Decl { … }`）不是行：内层没有 `owes` 字段，不许误判', () => {
  const files = cleanFiles()
  files['crates/demo/src/decl_type.rs'] = [
    'pub struct Decl {',
    '    pub name: &\'static str,',
    '    pub owes: Pattern,',
    '    pub until: &\'static [Pattern],',
    '}',
    '',
  ].join('\n')
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 0, out)
    assert.match(out, /生产 2 条归宿行/)
  } finally {
    box.cleanup()
  }
})

test('SR-001：新增一条未登记的归宿行 ⇒ 红，并指出「为什么不需要派发」必须回答', () => {
  const files = cleanFiles()
  files[DECL] = files[DECL].replace(sinkRow('EV'), `${sinkRow('EV')}\n${sinkRow('"demo.extra"')}`)
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
  files[DECL] = files[DECL].replace(sinkRow('EV'), `${sinkRow('EV')}\n${sinkRow('"demo.literal"')}`)
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
  files[DECL] = files[DECL].replace(`${sinkRow('"demo.literal"')}\n`, '')
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
  files[DECL] = files[DECL].replace(sinkRow('EV'), `${sinkRow('EV')}\n${sinkRow('UNKNOWN_CONST')}`)
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

test('SR-004：`until` 不是全字面量 ⇒ 判不了也要红（换个写法不能绕过棘轮）', () => {
  const files = cleanFiles()
  files[DECL] = files[DECL].replace(
    sinkRow('EV'),
    [
      sinkRow('EV'),
      '    Decl {',
      '        name: "demo.opaque",',
      '        reads: &[],',
      '        tier: LatencyTier::Reflex,',
      '        expand: None,',
      '        owes: Pattern { source: "demo.owed", key: &[], eq: &[] },',
      '        until: EXITS,',
      '        unless: &[],',
      '    },',
    ].join('\n'),
  )
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /SR-004/)
    assert.match(out, /until/)
  } finally {
    box.cleanup()
  }
})

test('常量别名可以解析：`use … as ALIAS` 与 `const … = "…"` 对得上 ⇒ 绿', () => {
  const files = {
    'crates/demo/src/consts.rs': 'pub const REAL: &str = "demo.aliased";\n',
    [DECL]: ['use crate::consts::REAL as ALIAS;', 'pub(crate) const ROWS: &[Decl] = &[', sinkRow('ALIAS'), '];', ''].join(
      '\n',
    ),
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
