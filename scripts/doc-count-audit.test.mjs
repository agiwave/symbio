/**
 * doc-count-audit 回归测试
 *
 * 这条守卫从**代码**提取三个数（`check_all` 条数、机制表顶层键数、含子键键数），
 * 再去扫活跃文档里「这几个名字紧邻的数」。它最危险的失效形态不是误报，而是
 * **漏报**：窗口放宽一点、标点判断错一点，都会让「改错数字反而不红」——而门禁全绿。
 * 所以这组用例把三件事各钉一遍：
 *
 *   ① **抓得住**：把代码里的数改了 / 把文档里的数改了，都必须变红；
 *   ② **不误报**：本仓真有的三种邻近形态（数在前、数在后带逗号、箭头叙事）不得判红；
 *   ③ **读不出即失败**：来源读不出来必须 exit 2，而不是「什么都没检查 ⇒ 绿灯」。
 *
 * 跑法：node --test scripts/doc-count-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./doc-count-audit.mjs', import.meta.url))

/** 造一棵最小仓库：`docs/` 一份文档，`invariants` 与 `facts` 两处来源可覆写 */
function audit ({ doc = 'docs/a.md', body = '# A\n', checkAll = 8, facts = { mechanisms: 10, paramKeys: 11 } } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-count-'))
  try {
    const write = (rel, content) => {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    write(doc, body)
    // `check_all` 的形状照代码写：第一条是 `let mut all = …`，其余是 `all.extend(…)`
    write(
      'symbio/src/symbio_core/invariants/mod.rs',
      [
        'pub fn check_all(events: &[Event]) -> Vec<Violation> {',
        '    let mut all = first(events);',
        ...Array.from({ length: checkAll - 1 }, (_, i) => `    all.extend(n${i + 2}(events));`),
        '    all',
        '}',
      ].join('\n'),
    )
    write(
      'docs/plan/verify/facts/mod.rs',
      [
        `pub const MECHANISMS: &[&str] = &[${Array.from({ length: facts.mechanisms }, (_, i) => `"k${i}"`).join(', ')}];`,
        `pub const PARAM_KEYS: &[&str] = &[${Array.from({ length: facts.paramKeys }, (_, i) => `"k${i}"`).join(', ')}];`,
      ].join('\n'),
    )
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.ifError(r.error)
    return r
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

const OK = '# A\n\n机制表 10 个顶层键。\n'

// ── ① 抓得住 ──────────────────────────────────────────────────────────

test('文档说对了一个数 ⇒ 通过（exit 0）', () => {
  const r = audit({ body: OK })
  assert.equal(r.status, 0, r.stdout + r.stderr)
})

test('DC-001 文档把 check_all 的条数写错 ⇒ 红', () => {
  const r = audit({ body: '# A\n\n`check_all` 八条。\n', checkAll: 7 })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /DC-001 .*说 `check_all` 有 8，而代码里是 7/)
})

test('DC-002 文档把顶层键数写错 ⇒ 红（数在名字之前）', () => {
  const r = audit({ body: '# A\n\n机制表 9 个顶层键。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /DC-002/)
})

test('DC-003 「登记的键（含子键）」数写错 ⇒ 红（数在名字之后）', () => {
  const r = audit({ body: '# A\n\n§8 登记的键（含子键）12 项。\n' })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /DC-003/)
})

test('代码里的数变了而文档没跟 ⇒ 红（这是它存在的理由）', () => {
  const r = audit({ body: OK, checkAll: 9, facts: { mechanisms: 11, paramKeys: 12 } })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /DC-002/)
})

// ── ② 不误报（本仓真有的三种邻近形态） ───────────────────────────────

test('不误报：数后面紧跟逗号（本仓 ROUTES.md 的真形：`check_all` 八条，空 = 全绿）', () => {
  const r = audit({ body: '# A\n\n不变量清单 `invariants`（`check_all` 八条，空 = 全绿）——纯读。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('不误报：顿号之后是另一个计数（机制表 10 个顶层键、三条不变量）', () => {
  const r = audit({ body: '# A\n\n机制表 10 个顶层键、三条不变量。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('不误报：箭头叙事归 D-004，这里不判（五条 → 七条）', () => {
  const r = audit({ body: '# A\n\n`check_all` 五条 → 七条（并入两条新断言）。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('不误报：数距名字太远，中间是散文（每 turn 至多一条开轮）', () => {
  const r = audit({ body: '# A\n\n`check_all`（每 turn 至多一条开轮）。\n' })
  assert.equal(r.status, 0, r.stdout)
})

test('不误报：只提名字不给数', () => {
  const r = audit({ body: '# A\n\n`check_all` 是唯一读出口。\n' })
  assert.equal(r.status, 0, r.stdout)
})

// ── ③ 读不出即失败 ──────────────────────────────────────────────────

test('来源读不出（invariants 不存在）⇒ exit 2，不是「零违规」的绿灯', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-count-'))
  try {
    fs.mkdirSync(path.join(root, 'docs'), { recursive: true })
    fs.writeFileSync(path.join(root, 'docs', 'a.md'), '# A\n')
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.equal(r.status, 2, r.stdout + r.stderr)
    assert.match(r.stdout, /审计范围读不出/)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})

test('来源读不出（facts 生成物缺失 ⇒ 机制键数无从取得）⇒ exit 2', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'doc-count-'))
  try {
    const abs = path.join(root, 'symbio', 'src', 'symbio_core', 'invariants')
    fs.mkdirSync(abs, { recursive: true })
    fs.writeFileSync(path.join(abs, 'mod.rs'), 'pub fn check_all(e: &[u8]) -> u8 {\n let mut all = a(e);\n all.extend(b(e));\n all\n}\n')
    fs.mkdirSync(path.join(root, 'docs'), { recursive: true })
    fs.writeFileSync(path.join(root, 'docs', 'a.md'), '# A\n')
    const r = spawnSync(process.execPath, [script, `--root=${root}`], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30000,
    })
    assert.equal(r.status, 2, r.stdout + r.stderr)
    assert.match(r.stdout, /审计范围读不出/)
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
})