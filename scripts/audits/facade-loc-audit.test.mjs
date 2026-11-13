/**
 * facade-loc-audit 回归测试
 *
 * 这一族的失效形态是**读不出 ⇒ 没什么可判 ⇒ 通过**，以及「统计口径写宽」——
 * 把 `*.test.rs` 的测试代码算进实现行数，或反过来把生产注释漏掉。
 * 两者都会让「门面自有实现还剩多少行」这个读数失去意义，而门禁照样全绿。
 *
 * 核心用例是用户点名的那条：**给实现加 5 行 ⇒ 必须红**（FA-001），
 * 并指出超了几行；同时钉住「删行不同批下调基线也红」（FA-002），
 * 否则「删 N 行再加 N 行」能把棘轮抹平。
 *
 * 跑法：node --test scripts/audits/facade-loc-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./facade-loc-audit.mjs', import.meta.url))

const SRC = 'crates/symbio-plugin-api/src'
const BASELINE_REL = 'scripts/baselines/plugin-api-loc.json'
const DOC = 'docs/003-design/000-fixture.md'

/** 恰有 `n` 个物理行的 Rust 文件（末尾换行不算多一行）。 */
function rustLines(n) {
  return `${Array.from({ length: n }, (_, i) => `// line ${i + 1}`).join('\n')}\n`
}

/** 干净夹具：effects 10 行 + port 5 行 + 未登记的 other 3 行 = 合计 18。 */
function cleanFiles() {
  return {
    [`${SRC}/effects.rs`]: rustLines(10),
    [`${SRC}/port.rs`]: rustLines(5),
    [`${SRC}/other.rs`]: rustLines(3),
    [DOC]: '### D-399 夹具决策\n',
  }
}

function cleanBaseline() {
  return {
    note: 'fixture',
    crate: SRC,
    files: [
      { path: 'effects.rs', lines: 10, adr: 'D-399' },
      { path: 'port.rs', lines: 5, adr: 'D-399' },
    ],
    total: { lines: 18, adr: 'D-399' },
  }
}

function sandbox(files, baseline = cleanBaseline()) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'facade-loc-'))
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

function run(root) {
  const r = spawnSync(process.execPath, [script, `--root=${root}`, '--strict'], {
    encoding: 'utf8',
    env: { ...process.env, NO_COLOR: '1' },
    timeout: 30_000,
  })
  return { status: r.status, out: `${r.stdout}${r.stderr}` }
}

test('干净夹具：逐项与合计都等于基线 ⇒ 通过，并报出读数', () => {
  const box = sandbox(cleanFiles())
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 0, out)
    assert.match(out, /生产合计 18 行/)
    assert.match(out, /基线合计 18 行/)
    assert.match(out, /effects\.rs {2}10 行/)
  } finally {
    box.cleanup()
  }
})

test('FA-001：实现加 5 行 ⇒ 红，并指出「超 5 行」', () => {
  const files = cleanFiles()
  files[`${SRC}/effects.rs`] = rustLines(15)
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /FA-001.*effects\.rs.*实测 15 行 \/ 基线 10 行（超 5 行）/)
  } finally {
    box.cleanup()
  }
})

test('FA-001：未登记文件变长也会被合计抓住', () => {
  const files = cleanFiles()
  files[`${SRC}/other.rs`] = rustLines(8)
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /FA-001.*合计.*实测 23 行 \/ 基线 18 行（超 5 行）/)
  } finally {
    box.cleanup()
  }
})

test('FA-002：删了行却没同批下调基线 ⇒ 红（防「删 N 行再加 N 行」抹平）', () => {
  const files = cleanFiles()
  files[`${SRC}/port.rs`] = rustLines(3)
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /FA-002.*port\.rs.*少 2 行/)
    assert.match(out, /同批下调/)
  } finally {
    box.cleanup()
  }
})

test('FA-003：adr 打不开、登记的文件不在生产里 ⇒ 两条都红', () => {
  const baseline = cleanBaseline()
  baseline.files[0].adr = 'D-998'
  baseline.files[1].path = 'ghost.rs'
  const box = sandbox(cleanFiles(), baseline)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /FA-003.*effects\.rs.*D-998 不存在/)
    assert.match(out, /FA-003.*ghost\.rs.*没有这个生产文件/)
  } finally {
    box.cleanup()
  }
})

test('退役条目：文件已删、lines 钉 0 ⇒ 通过（等效收紧，不是删格）', () => {
  const files = cleanFiles()
  delete files[`${SRC}/effects.rs`]
  const baseline = cleanBaseline()
  baseline.files[0] = {
    path: 'effects.rs',
    lines: 0,
    adr: 'D-399',
    retired: 'crates/symbio-sdk/src/plugin/effects.rs',
  }
  baseline.total = { lines: 8, adr: 'D-399' }
  const box = sandbox(files, baseline)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 0, out)
    assert.match(out, /effects\.rs {2}0 行/)
  } finally {
    box.cleanup()
  }
})

test('退役条目：文件又出现 ⇒ 红（退役没做完或实现长回来了）', () => {
  const baseline = cleanBaseline()
  baseline.files[0] = {
    path: 'effects.rs',
    lines: 0,
    adr: 'D-399',
    retired: 'crates/symbio-sdk/src/plugin/effects.rs',
  }
  const box = sandbox(cleanFiles(), baseline)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /FA-003.*effects\.rs.*仍在 .* 下/)
  } finally {
    box.cleanup()
  }
})

test('退役条目：lines 不是 0 或新家为空 ⇒ 红（退役不留下限余量，也不许无主）', () => {
  const files = cleanFiles()
  delete files[`${SRC}/effects.rs`]
  const baseline = cleanBaseline()
  baseline.files[0] = { path: 'effects.rs', lines: 3, adr: 'D-399', retired: ' ' }
  baseline.total = { lines: 8, adr: 'D-399' }
  const box = sandbox(files, baseline)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /FA-003.*effects\.rs.*lines 必须是 0/)
    assert.match(out, /FA-003.*effects\.rs.*retired 必须是非空字符串/)
  } finally {
    box.cleanup()
  }
})

test('测试代码不计：*.test.rs 再长也不进读数', () => {
  const files = cleanFiles()
  files[`${SRC}/effects.test.rs`] = rustLines(5_000)
  files[`${SRC}/tests.rs`] = rustLines(5_000)
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 0, out)
    assert.match(out, /生产合计 18 行/)
  } finally {
    box.cleanup()
  }
})

test('清单镜像不计：描述符声明的生成物再长也不进读数（D-470）', () => {
  const files = cleanFiles()
  files['scripts/facts/mirror.mjs'] = "const OUT_REL = 'crates/symbio-plugin-api/src/routes.rs'\n"
  files[`${SRC}/routes.rs`] = rustLines(500)
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 0, out)
    assert.match(out, /生产合计 18 行/)
  } finally {
    box.cleanup()
  }
})

test('清单镜像不计只认描述符：同一份文件没有声明时照旧计件（D-470）', () => {
  const files = cleanFiles()
  files[`${SRC}/routes.rs`] = rustLines(500)
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 1, out)
    assert.match(out, /FA-001.*合计.*实测 518 行 \/ 基线 18 行/)
  } finally {
    box.cleanup()
  }
})

test('基线缺失 ⇒ 退 2（读不出不许当 0 行通过）', () => {
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

test('crate 目录缺失 ⇒ 退 2', () => {
  const files = { [DOC]: '### D-399 夹具决策\n' }
  const box = sandbox(files)
  try {
    const { status, out } = run(box.root)
    assert.equal(status, 2, out)
    assert.match(out, /范围读不出/)
    assert.match(out, /crates\/symbio-plugin-api\/src/)
  } finally {
    box.cleanup()
  }
})
