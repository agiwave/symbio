// `cargo-doc-gate.mjs` 的回归测试。
//
// 它守的是一条**很容易被写成空转**的判据：指纹没变就跳过。如果指纹把不该算的东西
// 算进来（`target/` 里就放着它自己的戳），它会「刚写完戳就不新鲜」，机制自锁、
// 每轮都白跑一遍 1m36s；反过来若漏算输入（比如 `Cargo.toml` 的 feature），
// 它会拿上一轮的绿灯替这一轮的改动背书——**这正是它要防的那件事**。
//
// 所以这里钉两件事：**变了必须变**、**没变必须不变**。
//
// 跑法：node --test scripts/cargo-doc-gate.test.mjs

import { test, after } from 'node:test'
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { docFingerprint } from './cargo-doc-gate.mjs'

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'symbio-docgate-'))

const write = (rel, content) => {
  const p = path.join(root, ...rel.split('/'))
  fs.mkdirSync(path.dirname(p), { recursive: true })
  fs.writeFileSync(p, content, 'utf8')
}

write('crates/a/src/lib.rs', '//! 甲\npub fn f() {}\n')
write('crates/b/src/lib.rs', '//! 乙\n')
write('tauri/src-tauri/src/main.rs', 'fn main() {}\n')
write('Cargo.toml', '[workspace]\n')
write('crates/a/Cargo.toml', '[package]\nname = "a"\n')

after(() => {
  try {
    fs.rmSync(root, { recursive: true, force: true })
  } catch {
    /* 沙箱 safe-delete 垫片可能拒删；临时目录交给 OS 回收 */
  }
})

test('同一份输入 ⇒ 同一个指纹（否则「跳过」永远不成立）', () => {
  assert.equal(docFingerprint(root), docFingerprint(root))
})

test('改一句 doc 注释 ⇒ 指纹变（断链判据的输入就在注释里）', () => {
  const before = docFingerprint(root)
  write('crates/a/src/lib.rs', '//! 甲（改过）\npub fn f() {}\n')
  assert.notEqual(docFingerprint(root), before)
})

test('壳侧源码改动同样让指纹失效（tauri 也是 rustdoc 输入）', () => {
  const before = docFingerprint(root)
  write('tauri/src-tauri/src/main.rs', 'fn main() { let _ = 1; }\n')
  assert.notEqual(docFingerprint(root), before)
})

test('清单改动让指纹失效（feature 会改变被文档化的项）', () => {
  const before = docFingerprint(root)
  write('crates/a/Cargo.toml', '[package]\nname = "a"\n\n[features]\nextra = []\n')
  assert.notEqual(docFingerprint(root), before)
})

test('`target/` 里的东西不算输入（戳就住在那里，算进来会自锁）', () => {
  const before = docFingerprint(root)
  write('target/.symbio-doc-gate.stamp', 'whatever\n')
  write('crates/a/target/release/artifact', 'x')
  assert.equal(docFingerprint(root), before, '构建产物不该影响指纹，否则写完戳立刻判不新鲜')
})

test('非 `.rs` 的无关文件不算输入（改一段说明不该触发 1m36s 重跑）', () => {
  const before = docFingerprint(root)
  write('crates/a/README.md', '# 说明\n')
  assert.equal(docFingerprint(root), before)
})
