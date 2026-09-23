// `tauri-binary.mjs` 的回归测试。
//
// 它值得有回归测试的理由与 `cli-binary.test.mjs` 同源：**一个只会亮绿灯的机制等于
// 没有机制**。这个机制的失效方式特别隐蔽——它「看起来在工作」（日志照常打、壳照常
// 启动），只是**回显了一个不对应的指纹**：于是「这段日志是不是当前源码产出的」这个
// 问题得到了一个错误答案，而排查方向会因此被引到源码上（这已经发生过一次）。
//
// 所以这里钉的是判据本身：**指纹必须随构建输入变**。若哪天有人把指纹换成
// 「文件存在就算新鲜」、或把 `symbio/src` 从输入清单里摘掉（壳把整棵插件树编译
// 进去，那正是「日志里有源码中不存在的行」的来源），这些用例会红。
//
// 全程不启动 cargo：只测纯函数与状态判定，`ensureTauriBinary` 只测它**拒绝**的那条
// 路（`build: false` 时不新鲜 ⇒ 抛错）。真的去构建属于「跑一次构建」的事。
//
// 跑法：node --test scripts/tauri-binary.test.mjs

import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {
  profileOfBinary,
  tauriBinaryCandidates,
  tauriBinaryForProfile,
  tauriBinaryPath,
  tauriBinaryState,
  tauriBuildFingerprint,
  tauriBuildInputs,
  tauriStampForProfile,
  tauriStampPath,
  ensureTauriBinary,
  writeTauriStamp,
} from './tauri-binary.mjs'

const EXE = `symbio-tauri${process.platform === 'win32' ? '.exe' : ''}`

// 一次性临时「仓库」：只需要被指纹遍历的那几个路径，不必是真 cargo 工程。
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-tauribin-'))

const write = (rel, content) => {
  const p = path.join(root, rel)
  fs.mkdirSync(path.dirname(p), { recursive: true })
  fs.writeFileSync(p, content)
}

write('tauri/src-tauri/src/main.rs', 'fn main() { println!("v1"); }\n')
write('tauri/src-tauri/capabilities/default.json', '{ "identifier": "default" }\n')
write('tauri/src-tauri/tauri.conf.json', '{ "productName": "symbio" }\n')
write('tauri/src-tauri/Cargo.toml', '[package]\nname = "symbio-tauri"\nversion = "0.1.0"\n')
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

test('构建输入清单覆盖壳自身与它挂载的插件树', () => {
  const rels = tauriBuildInputs(root).map((p) => path.relative(root, p).split(path.sep).join('/'))
  assert.deepEqual(rels, [
    'symbio/Cargo.toml',
    'symbio/src/lib.rs',
    'tauri/src-tauri/Cargo.toml',
    'tauri/src-tauri/capabilities/default.json',
    'tauri/src-tauri/src/main.rs',
    'tauri/src-tauri/tauri.conf.json',
  ])
})

test('同一份输入 ⇒ 同一个指纹（否则「新鲜」永远判不出来）', () => {
  assert.equal(tauriBuildFingerprint(root), tauriBuildFingerprint(root))
})

test('壳源码改一个字节 ⇒ 指纹变', () => {
  const before = tauriBuildFingerprint(root)
  write('tauri/src-tauri/src/main.rs', 'fn main() { println!("v2"); }\n')
  assert.notEqual(tauriBuildFingerprint(root), before, '壳侧源码改动必须让指纹失效')
})

test('symbio 侧源码改动同样让指纹失效（整棵插件树被编译进壳）', () => {
  // 这条是**日志溯源**那次事故的直接回归锚点：日志里出现的行来自 `symbio/src`
  // 下的插件，而它们与壳是两份源码、一次构建。
  const before = tauriBuildFingerprint(root)
  write('symbio/src/lib.rs', 'pub fn f() -> u32 { 2 }\n')
  assert.notEqual(tauriBuildFingerprint(root), before)
})

test('新增一个插件源文件 ⇒ 指纹变', () => {
  const before = tauriBuildFingerprint(root)
  write('symbio/src/plugins/new.rs', 'pub const X: u8 = 1;\n')
  assert.notEqual(tauriBuildFingerprint(root), before)
})

test('capabilities 改动 ⇒ 指纹变（权限集是产物的一部分）', () => {
  const before = tauriBuildFingerprint(root)
  write('tauri/src-tauri/capabilities/default.json', '{ "identifier": "default", "windows": [] }\n')
  assert.notEqual(tauriBuildFingerprint(root), before)
})

test('清单（Cargo.toml / Cargo.lock）改动 ⇒ 指纹变', () => {
  const before = tauriBuildFingerprint(root)
  write('symbio/Cargo.toml', '[package]\nname = "symbio"\nversion = "0.2.0"\n')
  assert.notEqual(tauriBuildFingerprint(root), before)
})

test('大文件（内嵌资产）按 (大小, mtime) 判：只改 mtime 也算输入变了', () => {
  const big = path.join(root, 'symbio/src/asset.bin')
  fs.writeFileSync(big, Buffer.alloc(1024 * 1024 + 1, 7))
  const before = tauriBuildFingerprint(root)
  fs.utimesSync(big, new Date('2020-01-01T00:00:00Z'), new Date('2020-01-01T00:00:00Z'))
  assert.notEqual(tauriBuildFingerprint(root), before, '大文件不读内容，但时间变了必须反映出来')
  fs.rmSync(big)
})

// ================= 状态：新鲜 = 存在 且 戳 == 指纹 且 产物不比戳旧 =================

test('没有产物 ⇒ 不新鲜，原因是「不存在」', () => {
  const st = tauriBinaryState(root)
  assert.equal(st.exists, false)
  assert.equal(st.fresh, false)
  assert.equal(st.staleReason, '产物不存在')
})

test('产物在但没有构建戳 ⇒ 不新鲜（来源不明，不许当最新用）', () => {
  const bin = tauriBinaryForProfile(root, 'debug')
  fs.mkdirSync(path.dirname(bin), { recursive: true })
  fs.writeFileSync(bin, 'fake binary')
  const st = tauriBinaryState(root)
  assert.equal(st.exists, true)
  assert.equal(st.fresh, false)
  assert.match(st.staleReason, /构建戳/)
})

test('戳 == 指纹、且产物不比戳旧 ⇒ 新鲜', () => {
  const st0 = tauriBinaryState(root)
  fs.writeFileSync(tauriStampPath(st0.binaryPath), st0.fingerprint, 'utf8')
  // 顺序即事实：戳是构建**前**写的声明，一次成功的构建随后产出产物
  fs.writeFileSync(st0.binaryPath, 'rebuilt binary')
  const st = tauriBinaryState(root)
  assert.equal(st.fresh, true)
  assert.equal(st.staleReason, '')
})

test('产物比戳旧 ⇒ 不新鲜（写戳之后的那次构建没有产出它）', () => {
  // 这是「构建失败、旧产物留在原处、戳已指向新输入」那一刻。若只看「戳 == 指纹」，
  // 这里会亮绿灯而事实相反——所以判据里必须有这条顺序约束。
  const bin = tauriBinaryForProfile(root, 'debug')
  const { fingerprint } = writeTauriStamp(root, { profile: 'debug' })
  fs.writeFileSync(bin, 'stale binary left behind by a failed build')
  fs.utimesSync(bin, new Date('2020-01-01T00:00:00Z'), new Date('2020-01-01T00:00:00Z'))

  const st = tauriBinaryState(root)
  assert.equal(st.stamped, fingerprint, '内容这一条是过的（正是危险之处）')
  assert.equal(st.fresh, false)
  assert.match(st.staleReason, /比构建戳旧/)
})

test('源码再改 ⇒ 已写好的戳立刻失效（这就是它要防的那一刻）', () => {
  write('tauri/src-tauri/src/main.rs', 'fn main() { println!("v3"); }\n')
  const st = tauriBinaryState(root)
  assert.equal(st.fresh, false)
  assert.equal(st.staleReason, '构建输入自上次构建后已改变')
})

test('不许构建时（build: false）不新鲜 ⇒ 抛错，绝不静默用旧产物', () => {
  assert.throws(
    () => ensureTauriBinary(root, { build: false }),
    /Tauri 壳产物不可信[\s\S]*tauri-binary\.mjs/,
    '错误信息要直接给出重建命令——日志溯源最贵的是排查方向',
  )
})

// ================= 构建戳：内容没变就不重写 =================

test('`--stamp` 写的戳 == 当前指纹，且写在对应 profile 的目录里', () => {
  // `beforeDevCommand` / `beforeBuildCommand` 走的就是这条路。它必须与
  // `ensureTauriBinary` 构建前写的值**同源**，否则壳回显的指纹会与 `--print` 不一致。
  const { stamp, fingerprint } = writeTauriStamp(root, { profile: 'debug' })
  assert.equal(stamp, tauriStampForProfile(root, 'debug'))
  assert.equal(fs.readFileSync(stamp, 'utf8'), fingerprint)
  assert.equal(fingerprint, tauriBuildFingerprint(root))
})

test('构建输入未变 ⇒ 戳不重写（否则「什么都不用重建」会变成假警报）', () => {
  const first = writeTauriStamp(root, { profile: 'debug' })
  // 把 mtime 推到过去，再问一次：若被重写，mtime 会跳回来
  fs.utimesSync(first.stamp, new Date('2020-01-01T00:00:00Z'), new Date('2020-01-01T00:00:00Z'))
  const before = fs.statSync(first.stamp).mtimeMs

  const again = writeTauriStamp(root, { profile: 'debug' })
  assert.equal(again.rewritten, false)
  assert.equal(fs.statSync(again.stamp).mtimeMs, before, '未改写 ⇒ 戳的 mtime 必须原地不动')

  // 输入真的变了才重写
  write('tauri/src-tauri/capabilities/default.json', '{ "identifier": "default", "permissions": [] }\n')
  const changed = writeTauriStamp(root, { profile: 'debug' })
  assert.equal(changed.rewritten, true)
})

test('构建前置写了戳、构建随后产出产物 ⇒ 状态即「新鲜」', () => {
  // 契约闭环：`npm run stamp:dev` 与 `cargo build` 之间没有别的握手，靠的就是
  // 「戳在前、产物在后」这个顺序。
  writeTauriStamp(root, { profile: 'debug' })
  const bin = tauriBinaryForProfile(root, 'debug')
  fs.writeFileSync(bin, 'produced by the build that followed the stamp')
  assert.equal(tauriBinaryState(root).fresh, true)
})

// ================= 路径：候选取最新的那个 =================

test('两个 profile 都存在时取**最新**的那份，不是第一个', () => {
  const debugBin = tauriBinaryForProfile(root, 'debug')
  const releaseBin = tauriBinaryForProfile(root, 'release')
  fs.mkdirSync(path.dirname(releaseBin), { recursive: true })
  fs.writeFileSync(debugBin, 'built long ago')
  fs.utimesSync(debugBin, new Date('2020-01-01T00:00:00Z'), new Date('2020-01-01T00:00:00Z'))
  fs.writeFileSync(releaseBin, 'just packaged')
  assert.equal(tauriBinaryPath(root), releaseBin, 'dev 与打包各留一份时，不能按顺序取')
})

test('候选路径与平台一致（win32 带 .exe）', () => {
  for (const p of tauriBinaryCandidates(root)) assert.equal(path.basename(p), EXE)
})

test('由产物路径反推 profile（戳写在它旁边，故只认目录名）', () => {
  assert.equal(profileOfBinary(tauriBinaryForProfile(root, 'debug')), 'debug')
  assert.equal(profileOfBinary(tauriBinaryForProfile(root, 'release')), 'release')
})
