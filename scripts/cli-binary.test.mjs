// `cli-binary.mjs` 的回归测试。
//
// 它值得有回归测试的理由与 `gate.d/_shared.test.mjs` 同源：**一个只会亮绿灯的机制
// 等于没有机制**。这个机制的失效方式特别隐蔽——它「看起来在工作」（e2e 跑起来了、
// 断言也执行了），只是跑的是**过期产物**，于是失败信息与眼前的源码直接矛盾，
// 把排查方向引到源码上。这正是它被写出来的原因。
//
// 所以这里钉的是判据本身：**指纹必须随构建输入变**。若哪天有人把指纹换成
// 「文件存在就算新鲜」，这些用例会红。
//
// 全程不启动 cargo：只测纯函数与状态判定，`ensureCliBinary` 只测它**拒绝**的那条路
// （`build: false` 时不新鲜 ⇒ 抛错）。真的去构建属于「跑一次门禁」的事。
//
// 跑法：node --test scripts/cli-binary.test.mjs

import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {
  cliBinaryCandidates,
  cliBinaryPath,
  cliBinaryState,
  cliBuildFingerprint,
  cliBuildInputs,
  cliStampPath,
  ensureCliBinary,
} from './cli-binary.mjs'

const EXE = `symbio-cli${process.platform === 'win32' ? '.exe' : ''}`

// 一次性临时「仓库」：只需要被指纹遍历的那几个路径，不必是真 cargo 工程。
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-clibin-'))

const write = (rel, content) => {
  const p = path.join(root, rel)
  fs.mkdirSync(path.dirname(p), { recursive: true })
  fs.writeFileSync(p, content)
}

write('cli/src/main.rs', 'fn main() { println!("v1"); }\n')
write('cli/Cargo.toml', '[package]\nname = "symbio-cli"\nversion = "0.1.0"\n')
write('symbio/src/lib.rs', 'pub fn f() -> u32 { 1 }\n')
write('symbio/Cargo.toml', '[package]\nname = "symbio"\nversion = "0.1.0"\n')

after(() => {
  try {
    fs.rmSync(root, { recursive: true, force: true, maxRetries: 3 })
  } catch {
    /* 沙箱 safe-delete 垫片可能拒删；临时目录交给 OS 回收 */
  }
})

// ================= 指纹：必须随输入变 =================

test('构建输入清单覆盖两个 crate 的源码与清单', () => {
  const rels = cliBuildInputs(root).map((p) => path.relative(root, p).split(path.sep).join('/'))
  assert.deepEqual(rels, [
    'cli/Cargo.toml',
    'cli/src/main.rs',
    'symbio/Cargo.toml',
    'symbio/src/lib.rs',
  ])
})

test('同一份输入 ⇒ 同一个指纹（否则「新鲜」永远判不出来）', () => {
  assert.equal(cliBuildFingerprint(root), cliBuildFingerprint(root))
})

test('源码改一个字节 ⇒ 指纹变', () => {
  const before = cliBuildFingerprint(root)
  write('cli/src/main.rs', 'fn main() { println!("v2"); }\n')
  assert.notEqual(cliBuildFingerprint(root), before, 'CLI 侧源码改动必须让指纹失效')
})

test('symbio 侧源码改动同样让指纹失效（CLI 复用同一套插件树）', () => {
  const before = cliBuildFingerprint(root)
  write('symbio/src/lib.rs', 'pub fn f() -> u32 { 2 }\n')
  assert.notEqual(cliBuildFingerprint(root), before)
})

test('新增一个源文件 ⇒ 指纹变', () => {
  const before = cliBuildFingerprint(root)
  write('symbio/src/plugins/new.rs', 'pub const X: u8 = 1;\n')
  assert.notEqual(cliBuildFingerprint(root), before)
})

test('清单（Cargo.toml / Cargo.lock）改动 ⇒ 指纹变', () => {
  const before = cliBuildFingerprint(root)
  write('symbio/Cargo.toml', '[package]\nname = "symbio"\nversion = "0.2.0"\n')
  assert.notEqual(cliBuildFingerprint(root), before)
})

test('大文件（内嵌资产）按 (大小, mtime) 判：只改 mtime 也算输入变了', () => {
  const big = path.join(root, 'symbio/src/asset.bin')
  fs.writeFileSync(big, Buffer.alloc(1024 * 1024 + 1, 7))
  const before = cliBuildFingerprint(root)
  fs.utimesSync(big, new Date('2020-01-01T00:00:00Z'), new Date('2020-01-01T00:00:00Z'))
  assert.notEqual(cliBuildFingerprint(root), before, '大文件不读内容，但时间变了必须反映出来')
  fs.rmSync(big)
})

// ================= 状态：新鲜 = 存在 且 戳 == 指纹 =================

test('没有二进制 ⇒ 不新鲜，原因是「不存在」', () => {
  const st = cliBinaryState(root)
  assert.equal(st.exists, false)
  assert.equal(st.fresh, false)
  assert.equal(st.staleReason, '二进制不存在')
})

test('二进制在但没有构建戳 ⇒ 不新鲜（来源不明，不许当最新用）', () => {
  const bin = cliBinaryCandidates(root)[1] // cli/target/release/
  fs.mkdirSync(path.dirname(bin), { recursive: true })
  fs.writeFileSync(bin, 'fake binary')
  const st = cliBinaryState(root)
  assert.equal(st.exists, true)
  assert.equal(st.fresh, false)
  assert.match(st.staleReason, /构建戳/)
})

test('戳与当前指纹一致 ⇒ 新鲜', () => {
  const st = cliBinaryState(root)
  fs.writeFileSync(cliStampPath(st.binaryPath), st.fingerprint, 'utf8')
  const after = cliBinaryState(root)
  assert.equal(after.fresh, true)
  assert.equal(after.staleReason, '')
})

test('源码再改 ⇒ 已写好的戳立刻失效（这就是它要防的那一刻）', () => {
  write('cli/src/main.rs', 'fn main() { println!("v3"); }\n')
  const st = cliBinaryState(root)
  assert.equal(st.fresh, false)
  assert.equal(st.staleReason, '构建输入自上次构建后已改变')
})

test('不许构建时（build: false）不新鲜 ⇒ 抛错，绝不静默用旧产物', () => {
  assert.throws(
    () => ensureCliBinary(root, { build: false }),
    /CLI release 二进制不可信[\s\S]*cli-binary\.mjs/,
    '错误信息要直接给出重建命令——e2e 假失败最贵的是排查方向',
  )
})

// ================= 路径：两个候选取最新的那个 =================

test('两个候选都存在时取**最新**的那份，不是第一个', () => {
  const [a, b] = cliBinaryCandidates(root)
  fs.mkdirSync(path.dirname(a), { recursive: true })
  fs.writeFileSync(a, 'stale from months ago')
  fs.utimesSync(a, new Date('2020-01-01T00:00:00Z'), new Date('2020-01-01T00:00:00Z'))
  fs.writeFileSync(b, 'just built')
  assert.equal(cliBinaryPath(root), b, '换过 .cargo/config.toml 后旧位置可能残留一份，不能按顺序取')
})

test('候选路径与平台一致（win32 带 .exe）', () => {
  for (const p of cliBinaryCandidates(root)) assert.equal(path.basename(p), EXE)
})
