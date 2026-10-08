/**
 * panic-audit 回归测试
 *
 * 这条守卫的失效形态**只有一种**，而且是最坏的那种：**看不见**。
 *
 * 提取器一坏（正则改了、`stripTestModules` 形状变了、某处 `#[cfg(test)]` 没抹干净），
 * 它不会报错——它会**数出 0 个命中**，然后报「全部已登记」并退出 0。那正是它承诺要
 * 防的静默：守卫绿着，而健壮性面已经没人管。所以第一组用例拿**运行时导出的常量**
 * 做对照，任何提取失配立刻红。
 *
 * 第二组钉 PN-001…003 三个方向，第三组钉那三条「不判范围」的约定。
 *
 * 跑法：node --test scripts/panic-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./panic-audit.mjs', import.meta.url))
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

/** 造一棵最小仓库：`files` 是 `相对路径 → 源码`，`regs` 是附在源码尾部的登记 */
function audit ({ files, regs = {} } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'panic-audit-'))
  try {
    for (const [rel, body] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      const reg = (regs[rel] ?? [])
        .map(([fn, why]) => `\n// panic-allow ${rel.replace(/\\/g, '/')}::${fn}: ${why}\n`)
        .join('')
      fs.writeFileSync(abs, body + reg, 'utf8')
    }
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

const F = 'symbio/src/x.rs'
const unwrapFn = 'pub fn f(a: Option<u32>) -> u32 {\n    a.unwrap()\n}\n'

// ── ① 提取器不许悄悄失配 ─────────────────────────────────────────────

test('真实仓库当前状态通过（防守卫在真仓库上误报）', () => {
  const r = spawnSync(process.execPath, [script], { env: { ...process.env, NO_COLOR: '1' }, encoding: 'utf8', timeout: 120000 })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  const m = /panic-audit 通过：(\d+) 个登记项全部有非空理由，(\d+) 处/.exec(r.stdout)
  assert.ok(m, '通过信息里没有登记项/命中数 ⇒ 提取器可能已失配')
  assert.ok(Number(m[1]) > 0 && Number(m[2]) > 0, '真实仓库里数出 0 ⇒ 提取器与源码形状脱节')
})

test('提取器不许把命中数数成 0：注入一处 unwrap 必须被数到', () => {
  const r = audit({ files: { [F]: unwrapFn }, regs: { [F]: [['f', '测试用登记']] } })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /1 个登记项全部有非空理由，1 处/)
})

// ── ② PN-001…003 ─────────────────────────────────────────────────────

test('PN-001 未登记 ⇒ 红', () => {
  const r = audit({ files: { [F]: unwrapFn } })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /PN-001 symbio\/src\/x\.rs::f 有 1 处 panic 面未登记/)
})

test('PN-002 理由为空 ⇒ 红（空理由的登记等于没有登记）', () => {
  const r = audit({ files: { [F]: unwrapFn }, regs: { [F]: [['f', '']] } })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /PN-002 symbio\/src\/x\.rs::f 的登记理由为空/)
})

test('PN-003 登记了而代码里已无对应 panic 面 ⇒ 红（防豁免只增不减）', () => {
  const r = audit({
    files: { [F]: 'pub fn g() -> u32 {\n    1\n}\n' },
    regs: { [F]: [['g', '理由']] },
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /PN-003 symbio\/src\/x\.rs::g 有登记但代码里已无对应 panic 面/)
})

test('登记必须写在同一个文件里（不能靠别处一张表蒙混）', () => {
  const r = audit({
    files: {
      [F]: unwrapFn,
      'symbio/src/y.rs': 'pub fn h() {}\n// panic-allow symbio/src/x.rs::f: 写在别的文件\n',
    },
  })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /PN-001 symbio\/src\/x\.rs::f/)
})

test('同文件内多个 panic 归并成一个登记项', () => {
  const body = 'pub fn f(a: Option<u32>, b: Option<u32>) -> (u32, u32) {\n    (a.unwrap(), b.expect("x"))\n}\n'
  const r = audit({ files: { [F]: body }, regs: { [F]: [['f', '理由']] } })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /1 个登记项全部有非空理由，2 处/)
})

// ── ③ 三条「不判范围」的约定 ─────────────────────────────────────────

test('不判测试文件（*.test.rs / tests.rs）', () => {
  const r = audit({
    files: {
      'symbio/src/x.rs': 'pub fn f() {}\n',
      'symbio/src/x.test.rs': unwrapFn,
      'symbio/src/tests.rs': unwrapFn,
    },
  })
  assert.equal(r.status, 0, r.stdout)
})

test('不判内联 #[cfg(test)] mod', () => {
  const r = audit({
    files: {
      [F]: 'pub fn f() {}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn t() {\n        let a: Option<u32> = None;\n        a.unwrap();\n    }\n}\n',
    },
  })
  assert.equal(r.status, 0, r.stdout)
})

test('不判**单独带 #[cfg(test)] 的项**（stripTestModules 抓不到的那一层）', () => {
  const r = audit({
    files: {
      [F]:
        'pub fn f() {}\n\n#[cfg(test)]\npub(crate) fn assert_helper(p: &dyn T) {\n    p.open().expect("本协议应实现增量提取");\n}\n\n#[cfg(test)]\npub(crate) fn assert_other(p: &dyn T) {\n    p.open().expect("同上");\n}\n',
    },
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /0 个登记项/)
})

test('不判注释里提到的形态（写「这里不能用 unwrap」不得自我命中）', () => {
  const r = audit({
    files: { [F]: '// 这里不能用 unwrap()，那会 panic!()\n/* 也不该 todo!() */\npub fn f() {}\n' },
  })
  assert.equal(r.status, 0, r.stdout)
})

test('所属函数取「向上最近的 fn 定义行」', () => {
  const r = audit({
    files: { [F]: 'pub fn a() -> u32 {\n    let x: Option<u32> = None;\n    x.unwrap()\n}\n\npub fn b() {}\n' },
    regs: { [F]: [['a', '理由']] },
  })
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /1 个登记项/)
})

test('两个 fn 各自的 panic 不许共用一条登记', () => {
  const body = 'pub fn a(x: Option<u32>) -> u32 {\n    x.unwrap()\n}\n\npub fn b(y: Option<u32>) -> u32 {\n    y.unwrap()\n}\n'
  const r = audit({ files: { [F]: body }, regs: { [F]: [['a', '理由']] } })
  assert.equal(r.status, 1, r.stdout)
  assert.match(r.stdout, /PN-001 symbio\/src\/x\.rs::b/)
})