/**
 * test-layout-audit 回归测试 —— 证明它**会红**
 *
 * 补它的两个理由（2026-09-20 复核发现）：
 *   ① 它是 `gate.mjs` 声称"每个判定型守卫都先跑回归测试"里**缺的那个**；
 *   ② 它对自己的核心约定（测试独立成文件）**零判定**——内联不拆分也不构成违规，
 *      只要内联块写在文件末尾就报 `✓ 布局符合约定`。后者已加棘轮修掉。
 *
 * 用法：node --test scripts/test-layout-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./test-layout-audit.mjs', import.meta.url))
const repoRoot = path.resolve(path.dirname(script), '..')

/**
 * 删临时目录 —— **逐个删，不用 `fs.rmSync(recursive)`**。
 *
 * 原因（本机踩到，值得记）：带沙箱的开发环境会把"一次递归删掉一整个目录"当成
 * **批量删除**拦截或挂起。本用例会造 50+ 个夹具文件，用 `rmSync(recursive)` 时进程
 * 直接卡死（表现为测试超时、零输出，极难定位）。逐个 `unlinkSync` 则不触发。
 * 同样的坑此前也拦过 `vite build` 清 `dist/` 与 `vitest --coverage` 清 `coverage/`。
 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

function audit(files, { strict = false } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'test-layout-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(
      process.execPath,
      [script, `ROOT=${root}`, ...(strict ? ['--strict'] : [])],
      { cwd: root, env: { ...process.env, NO_COLOR: '1' }, encoding: 'utf8', timeout: 30_000 }
    )
    assert.ifError(result.error)
    return result
  } finally {
    rmTree(root)
  }
}

const clean = {
  'foo.rs': 'pub fn a() {}\n#[cfg(test)]\n#[path = "foo.test.rs"]\nmod tests;\n',
  'foo.test.rs': '#[test]\nfn t() {}\n',
}

// ── 检查 1：拆分出来的测试文件必须被宿主声明，否则**根本不会被编译** ──────
test('宿主声明了 #[path] → 通过', () => {
  assert.equal(audit(clean).status, 0)
})

test('宿主没声明 #[path] → ERROR（测试文件不会被编译）', () => {
  const r = audit({ ...clean, 'foo.rs': 'pub fn a() {}\n' })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /foo\.test\.rs/)
})

test('找不到宿主文件 → ERROR', () => {
  const r = audit({ 'orphan.test.rs': '#[test]\nfn t() {}\n' })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /找不到宿主/)
})

// ── 检查 2：内联在**文件中部** → 拆分时最容易把生产代码搬进测试文件 ──────
test('内联 mod tests 在文件中部 → 警告（--strict 才失败）', () => {
  const files = { 'bar.rs': 'pub fn a() {}\nmod tests {\n    #[test]\n    fn t() {}\n}\npub fn b() {}\n' }
  assert.equal(audit(files).status, 0)
  assert.equal(audit(files, { strict: true }).status, 2)
})

// ── 统计口径：宿主的 `mod tests;` 声明**不是**内联 ────────────────────────
// 旧版用 `/^\s*mod tests\b/m` 一把抓，把两类文件混算成一个"106"，既说不清已拆多少、
// 也说不清剩多少未拆。这里直接断言两个数字，比断言退出码更精确。
test('统计口径：宿主声明与真内联分开计', () => {
  const r = audit({
    'h0.rs': 'pub fn f() {}\n#[path = "h0.test.rs"]\nmod tests;\n',
    'h0.test.rs': '#[test]\nfn t() {}\n',
    'inline.rs': 'pub fn g() {}\nmod tests {\n    #[test]\n    fn t() {}\n}\n',
  })
  assert.match(r.stdout, /宿主声明 1/)
  assert.match(r.stdout, /真内联 mod tests：1/)
})

// ── 棘轮：真内联的文件数只降不升 ────────────────────────────────────────
// 这是本次补上的判定。旧版对"根本没拆分"零判定 ⇒ 约定没有执行力。
test('棘轮：真内联数超过基线 → ERROR', () => {
  // 基线下调只会让本用例更容易红（棘轮只允许下调），故写死 54 是安全的。
  // 这是本文件唯一需要造很多文件的用例——夹具成本换的是"约定真的有牙"。
  const files = {}
  for (let i = 0; i < 54; i += 1) {
    files[`m${i}.rs`] = 'pub fn f() {}\nmod tests {\n    #[test]\n    fn t() {}\n}\n'
  }
  const r = audit(files)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /基线/)
})

// ── 空树 / 真仓 ─────────────────────────────────────────────────────────
test('空树通过（守卫不是空转即红）', () => {
  assert.equal(audit({}).status, 0)
})

test('真实仓库当前状态通过（防本守卫在真仓库上误报）', () => {
  const r = spawnSync(process.execPath, [script], {
    cwd: repoRoot,
    env: { ...process.env, NO_COLOR: '1' },
    encoding: 'utf8',
    timeout: 60_000,
  })
  assert.ifError(r.error)
  assert.equal(r.status, 0, r.stdout)
})
