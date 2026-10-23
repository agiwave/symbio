/**
 * obligation-bypass-audit 回归测试
 *
 * 这条守卫判三件事：旁路直调 `owes()` 的**点数 ≤ 基线**（OB-001）、低于基线要提示
 * 下调（OB-002）、每处点名的**行必须在 `ROWS` 里**（OB-003）。它最危险的失效形态
 * 全是**静默变绿**——数字算错、算宽、算不到，报出来都是「通过」，所以逐条钉：
 *
 *   ① **算得对**：调用点计数、多行调用抓得到行名、基线下方要给提示；
 *   ② **不算多**：`fn owes(` 定义行、`*.test.rs`、`drive_steps` 函数体、注释里的
 *      `owes(` 都不得进计数。**这四类每一条都会把基线喂松**，而松了的棘轮等于没有；
 *   ③ **行名只在 `ROWS` 里找**：同一文件的 `ACTORS` / 端口声明也写 `name: "…"`，
 *      整文件抓会让一个**不是行**的名字满足 OB-003（用例用执行体名 `courier` 钉它）；
 *   ④ **读不出即失败**：没有 `.rs`、没有 `ROWS` 都必须 exit 2，不退化成
 *      「没什么可判 ⇒ 通过」。
 *
 * 跑法：node --test scripts/obligation-bypass-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./obligation-bypass-audit.mjs', import.meta.url))

/** 行表：两行 + 一个**不在行表里**的执行体名（OB-003 的作用域用例要用） */
const ROWS_RS = `pub(crate) static ROWS: &[Decl] = &[
    Decl { name: "alpha", by: None },
    Decl { name: "beta", by: None },
];

const ACTOR_COURIER: ActorDecl = ActorDecl { name: "courier", writes: &[] };

pub(crate) fn owes(name: &str, events: &[Event], turn: u64, now: i64) -> bool {
    name.is_empty() && !events.is_empty() && turn == 0 && now == 0
}
`

const ROWS_PATH = 'symbio/src/plugins/loop/loop_rows.rs'

/** 造一棵最小仓库：`files` 是 相对路径 → 内容（`ROWS_PATH` 默认给一份两行表） */
function audit(files, { baseline = '2' } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'obligation-bypass-'))
  try {
    const all = { [ROWS_PATH]: ROWS_RS, ...files }
    for (const [rel, content] of Object.entries(all)) {
      if (content === null) continue
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1', OBLIGATION_BYPASS_BASELINE: baseline },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.ifError(r.error)
    return r
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

const out = (r) => `${r.stdout}${r.stderr}`

const caller = (rows) =>
  `use super::loop_rows::owes;\n\nfn step() {\n${rows.map((r) => `    let _ = owes("${r}", &[], 0, 0);\n`).join('')}}\n`

test('干净仓：2 处直调 / 基线 2 ⇒ 通过', () => {
  const r = audit({ 'symbio/src/plugins/loop/a.rs': caller(['alpha', 'beta']) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /通过.*2\/2 处/)
})

test('OB-001 直调点数超出基线 ⇒ 红，并列出每一处', () => {
  const r = audit({ 'symbio/src/plugins/loop/a.rs': caller(['alpha', 'beta', 'alpha']) })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /OB-001 派发面变宽：直调 `owes\(\)` 的点数 3 > 基线 2/)
  assert.match(out(r), /a\.rs:\d+ {2}alpha/)
})

test('OB-002 低于基线 ⇒ 通过但提示下调（余量不许白留）', () => {
  const r = audit({ 'symbio/src/plugins/loop/a.rs': caller(['alpha']) }, { baseline: '5' })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /提示.*1<5.*BYPASS_BASELINE/)
})

test('`fn owes(` 定义行不算调用点（13 与 12 的那个 off-by-one）', () => {
  // 行表文件里那份定义若被算进来，计数就天生多一格 ⇒ 棘轮永远松着
  const r = audit({ 'symbio/src/plugins/loop/a.rs': caller(['alpha', 'beta']) })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /2\/2 处/)
})

test('`*.test.rs` 里的断言不算调用点', () => {
  const r = audit({
    'symbio/src/plugins/loop/a.rs': caller(['alpha', 'beta']),
    'symbio/src/plugins/loop/a.test.rs': caller(['alpha', 'alpha', 'alpha', 'beta']),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /2\/2 处/)
})

test('`drive_steps` 函数体里的问债不算旁路（它就是派发器本体）', () => {
  const r = audit({
    'symbio/src/plugins/loop/a.rs': caller(['alpha', 'beta']),
    'symbio/src/plugins/loop/turn_exec.rs':
      'fn drive_steps(rows: &[Decl]) {\n    let _ = owes("alpha", &[], 0, 0);\n    let _ = owes("beta", &[], 0, 0);\n}\n',
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /2\/2 处/)
})

test('注释里的 `owes(` 不算调用点', () => {
  const r = audit({
    'symbio/src/plugins/loop/a.rs':
      '// 这里曾经直调 owes("alpha")，收编后改走 drive\n/* owes("beta") */\n' + caller(['alpha', 'beta']),
  })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /2\/2 处/)
})

test('OB-003 点名不存在的行 ⇒ 红（`owes` 恒 false ⇒ 这段旁路永不执行）', () => {
  const r = audit({ 'symbio/src/plugins/loop/a.rs': caller(['alpha', 'beta', 'gamma']) }, { baseline: '9' })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /OB-003 .*owes\("gamma"\).*ROWS.*没有这一行/)
})

test('OB-003 只认 `ROWS` 块：执行体名 / 端口名不是行名', () => {
  // `courier` 在同一文件的 `ActorDecl { name: "courier" }` 里写着，但它不是一行
  const r = audit({ 'symbio/src/plugins/loop/a.rs': caller(['alpha', 'courier']) }, { baseline: '9' })
  assert.equal(r.status, 1, out(r))
  assert.match(out(r), /OB-003 .*owes\("courier"\)/)
})

test('多行调用也能抓到行名', () => {
  const r = audit({
    'symbio/src/plugins/loop/a.rs':
      'fn step() {\n    let _ = super::super::loop_rows::owes(\n        "alpha",\n        &[],\n        0,\n        0,\n    );\n}\n',
  }, { baseline: '1' })
  assert.equal(r.status, 0, out(r))
  assert.match(out(r), /alpha×1/)
})

test('范围读不出（一个 `.rs` 都没有）⇒ exit 2，不退化成通过', () => {
  const r = audit({ [ROWS_PATH]: null, 'README.md': '# 空仓\n' })
  assert.equal(r.status, 2, out(r))
  assert.match(out(r), /审计范围读不出/)
})

test('行表读不出（`ROWS` 不在）⇒ exit 2，宁红不静默', () => {
  const r = audit({
    [ROWS_PATH]: 'pub(crate) static OTHER: &[Decl] = &[];\n',
    'symbio/src/plugins/loop/a.rs': caller(['alpha', 'beta']),
  })
  assert.equal(r.status, 2, out(r))
  assert.match(out(r), /读不出 `ROWS` 行表/)
})
