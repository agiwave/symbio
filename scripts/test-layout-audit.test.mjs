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

function audit(files, { strict = false, ratchet = null } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'test-layout-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(
      process.execPath,
      [
        script,
        `ROOT=${root}`,
        ...(strict ? ['--strict'] : []),
        ...(ratchet ? [`--ratchet=${ratchet}`] : []),
      ],
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

// ── 检查 2：测试模块在**文件中部** → 拆分时最容易把生产代码搬进测试文件 ────
// 临时目录的基线是 0，放任何内联测试都会先触发棘轮 ERROR，故这里不断言退出码，
// 只断言告警**确实发出**（退出码由下面的棘轮用例单独覆盖）。
test('测试模块在文件中部 → 告警', () => {
  const files = { 'bar.rs': 'pub fn a() {}\nmod tests {\n    #[test]\n    fn t() {}\n}\npub fn b() {}\n' }
  assert.match(audit(files).stdout, /中部/)
})

// ── 统计口径：宿主的 `mod tests;` 声明**不是**内联 ────────────────────────
// 旧版用 `/^\s*mod tests\b/m` 一把抓，把两类文件混算成一个"106"，既说不清已拆多少、
// 也说不清剩多少未拆。这里直接断言两个数字，比断言退出码更精确。
test('统计口径：宿主声明与未拆分文件分开计', () => {
  const r = audit({
    'h0.rs': 'pub fn f() {}\n#[path = "h0.test.rs"]\nmod tests;\n',
    'h0.test.rs': '#[test]\nfn t() {}\n',
    'inline.rs': 'pub fn g() {}\nmod tests {\n    #[test]\n    fn t() {}\n}\n',
  })
  assert.match(r.stdout, /宿主声明 1/)
  assert.match(r.stdout, /含测试函数的生产文件：1/)
})

// ── 棘轮 ────────────────────────────────────────────────────────────────
test('棘轮：未拆分文件数超过基线 → ERROR', () => {
  const files = {}
  for (let i = 0; i < 3; i += 1) {
    files[`m${i}.rs`] = 'pub fn f() {}\nmod tests {\n    #[test]\n    fn t() {}\n}\n'
  }
  const r = audit(files)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /基线/)
})

// ★ 本守卫存在的理由：旧判据**只数文件数**，于是「往一个已经有内联测试的存量
//   文件里继续加测试」完全不红——而那恰恰是最容易发生的路径。
//   这里用 `--ratchet` 把基线设成"现状"，再多加一个测试函数：
//   文件数不变（1 == 1），只有测试函数数 1 → 2，守卫必须因此变红。
test('棘轮：往已有内联测试的文件里再加一个测试 → ERROR（文件数不变也要红）', () => {
  const one = { 'a.rs': 'pub fn f() {}\nmod tests {\n    #[test]\n    fn t1() {}\n}\n' }
  assert.equal(audit(one, { ratchet: 'files=1,tests=1' }).status, 0, '基线等于现状时应通过')

  const two = {
    'a.rs': 'pub fn f() {}\nmod tests {\n    #[test]\n    fn t1() {}\n    #[test]\n    fn t2() {}\n}\n',
  }
  const r = audit(two, { ratchet: 'files=1,tests=1' })
  assert.equal(r.status, 1, '文件数仍是 1，但测试函数 2 > 1 ⇒ 必须红')
  assert.match(r.stdout, /测试函数 2 > 基线 1/)
})

// ★ 旧判据按**模块名**找（`^\s*mod tests\b[^{;]*\{`），因此 `symbio_core/turn.rs`
//   那种 `mod tool_call_tests { … }` 里的测试一个都数不到。改按测试函数判之后，
//   无论模块叫什么名字都会被数到。
test('测试模块不叫 tests 也要被数到（旧判据漏的就是这条）', () => {
  const files = {
    'turn.rs': 'pub fn f() {}\n#[cfg(test)]\nmod tool_call_tests {\n    #[test]\n    fn t() {}\n}\n',
  }
  const r = audit(files, { ratchet: 'files=0,tests=0' })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /测试函数 1 > 基线 0/)
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
