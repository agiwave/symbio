/**
 * internal-model-audit 回归测试 —— 证明它**会红**，且**只在该红的地方红**
 *
 * 判据只有三条，但每条都有「静默失效」的形态，钉的正是那几条：
 *
 *   ① 干净夹具（无 `.execute_turn(`）→ exit 0；
 *   ② 未登记的调用点 → IM-001 红；
 *   ③ 标记写在调用点**上一行** → 绿（这是本判据的登记形态）；
 *   ④ 标记写在调用点**同一行**（行尾）→ 绿；
 *   ⑤ 空理由的标记 → IM-002 红（写了像登记、实际不算，必须显式报出来）；
 *   ⑥ 标记悬空（写了而附近没有调用点）→ IM-003 红；
 *   ⑦ 不误报：注释里提一句 `execute_turn`、`.test.rs` 里的调用、`#[cfg(test)] mod` 里的调用
 *     → 绿（这三处若算进去，豁免会写到满屏）；
 *   ⑧ 扫描根没有 `.rs` ⇒ 判据失效，exit 1（不许「读空即全绿」）；
 *   ⑨ 真仓库：四处存量全部登记，且**正好四处**——第五处会同时打破「全绿」与这个数。
 *
 * 用法：node --test scripts/internal-model-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./internal-model-audit.mjs', import.meta.url))
const repoRoot = path.resolve(script, '..', '..')

/** 删临时目录 —— 逐个删，不用 rmSync(recursive)（沙箱会拦截批量删除，同 SC 测试的理由）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

/** 造一个最小 SRC_ROOT；`files` 形如 `{'plugins/a.rs': '源码'}`。 */
function makeRoot(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'im-audit-'))
  for (const [rel, src] of Object.entries(files)) {
    const p = path.join(root, rel)
    fs.mkdirSync(path.dirname(p), { recursive: true })
    fs.writeFileSync(p, src)
  }
  return root
}

const run = (root) =>
  spawnSync(process.execPath, [script, `SRC_ROOT=${root}`], { encoding: 'utf8' })

const fixture = (body) => ({ 'plugins/session/lib.rs': body })

test('① 干净夹具（没有任何 execute_turn 调用）→ 通过', () => {
  const root = makeRoot(fixture('fn a() { let _: u8 = 1; }\n'))
  try {
    const r = run(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /4\s*处|0\s*处|全部登记/)
  } finally {
    rmTree(root)
  }
})

test('② 未登记的调用点 → IM-001 红', () => {
  const root = makeRoot(
    fixture('async fn go(p: P) {\n    p.execute_turn("s", &[], &[], "r", &env).await;\n}\n'),
  )
  try {
    const r = run(root)
    assert.equal(r.status, 1)
    assert.match(r.stderr, /IM-001/)
  } finally {
    rmTree(root)
  }
})

test('③ 标记写在调用点上一行 → 绿（本判据的登记形态）', () => {
  const root = makeRoot(
    fixture(
      'async fn go(p: P) {\n' +
        '    // internal-model-allow IM-001: 压缩摘要是内部请求\n' +
        '    p.execute_turn("s", &[], &[], "r", &env).await;\n' +
        '}\n',
    ),
  )
  try {
    const r = run(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /存量豁免 1 处/)
  } finally {
    rmTree(root)
  }
})

test('④ 标记写在调用点同一行（行尾）→ 绿', () => {
  const root = makeRoot(
    fixture(
      'async fn go(p: P) {\n' +
        '    p.execute_turn("s", &[], &[], "r", &env).await; // internal-model-allow IM-001: 同上\n' +
        '}\n',
    ),
  )
  try {
    const r = run(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑤ 空理由的标记 → IM-002 红（不静默降级成 IM-001）', () => {
  const root = makeRoot(
    fixture(
      'async fn go(p: P) {\n' +
        '    // internal-model-allow IM-001:\n' +
        '    p.execute_turn("s", &[], &[], "r", &env).await;\n' +
        '}\n',
    ),
  )
  try {
    const r = run(root)
    assert.equal(r.status, 1)
    assert.match(r.stderr, /IM-002/)
  } finally {
    rmTree(root)
  }
})

test('⑥ 标记悬空（写了而附近没有调用点）→ IM-003 红', () => {
  const root = makeRoot(
    fixture(
      '// internal-model-allow IM-001: 代码已经搬走了\nfn b() {}\n',
    ),
  )
  try {
    const r = run(root)
    assert.equal(r.status, 1)
    assert.match(r.stderr, /IM-003/)
  } finally {
    rmTree(root)
  }
})

test('⑦ 不误报：注释、.test.rs 与 #[cfg(test)] mod 里的调用都不判', () => {
  const root = makeRoot({
    'plugins/a.rs':
      '//! 这里在注释里提到 execute_turn 的形状\n' +
      'fn c() { let _ = "execute_turn"; }\n' +
      '#[cfg(test)]\nmod tests {\n    fn d() { p.execute_turn("s", &[], &[], "r", &env); }\n}\n',
    'plugins/a.test.rs': 'fn e() { p.execute_turn("s", &[], &[], "r", &env); }\n',
    'plugins/tests.rs': 'fn f() { p.execute_turn("s", &[], &[], "r", &env); }\n',
  })
  try {
    const r = run(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑧ 扫描根没有 .rs ⇒ 判据失效，exit 1', () => {
  const root = makeRoot({ 'notes.txt': 'nothing here' })
  try {
    const r = run(root)
    assert.equal(r.status, 1)
    assert.match(r.stderr, /失去判据/)
  } finally {
    rmTree(root)
  }
})

test('⑨ 真仓库：四处存量全部登记，且正好四处', () => {
  const r = spawnSync(process.execPath, [script], { encoding: 'utf8' })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /直调 execute_turn 的 4 处全部登记过理由/)
  assert.match(r.stdout, /存量豁免 4 处/)
})
