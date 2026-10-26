// core-surface-audit 回归测试
//
// 本脚本是**报告型**（退出码恒 0），所以它的失效形态不是「假绿灯」而是**说假话**：
// 报告少算消费方 ⇒ 在用的东西被列成「下放候选」，读的人顺着去改本来没坏的东西。
// 开发过程中**真的踩了两次**，两条都写成了回归：
//
//   1. **多条通配互相覆盖**：根 `mod.rs` 有 `pub use plugin::*` / `keys::*` / `logger::*`
//      三条通配，第一版把它们存进同一个 Map 键 `'*'` ⇒ 后一条盖掉前一条，
//      公开面从 251 个掉到 134 个，`PLUGIN_ID_*` / `PathKey` / `PluginStopReason` / `PLUGIN_KEY_*`
//      全部凭空消失。
//   2. **`symbio/src` 直属文件被跳过**：`lib.rs` / `plugins/mod.rs` 这类**不在插件子目录里**
//      的文件当时返回 `null` 单位被整体跳过 ⇒ 「只在注册表里被用到」的符号被算成
//      **0 个消费方**（`PluginErrorCode` / `PluginIdentity` 一族全部假报）。
//      「数不到」与「真的没人用」是两件事。
//
// 另外两条反例守卫：
//   3. 修完「少算」不能变成「一律不算候选」——真的零消费方仍须如实报出来；
//   4. 宏生成的键（`define_string_key!`）必须计入公开面——它们不带 `pub` 关键字，
//      正则扫不到，而恰恰是**消费方最多**的一批（`PATH` 17 个消费方）。
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), 'core-surface-audit.mjs')

/** 造一棵最小仓库并跑 core-surface-audit */
function audit(files, { verbose = false } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'core-surface-audit-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const p = path.join(root, rel)
      fs.mkdirSync(path.dirname(p), { recursive: true })
      fs.writeFileSync(p, content)
    }
    const r = spawnSync(process.execPath, [script, `--root=${root}`, ...(verbose ? ['--verbose'] : [])], {
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 10000,
    })
    assert.equal(r.status, 0, `脚本应恒为 0（报告型）：\n${r.stderr}`)
    return r.stdout
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

test('多条 `pub use <域>::*` 都要展开，不能互相覆盖', () => {
  // 反例守卫：这正是开发时踩的坑——三条通配存进同一个 Map 键，只活下来一条。
  const out = audit(
    {
      'crates/symbio-core/src/lib.rs': 'pub use alpha::*;\npub use beta::*;\n',
      'crates/symbio-core/src/alpha/mod.rs': 'pub struct AlphaThing;\n',
      'crates/symbio-core/src/beta/mod.rs': 'pub struct BetaThing;\n',
      'crates/symbio-plugin-p1/src/a.rs': 'fn f(_: AlphaThing, _: BetaThing) {}\n',
      'crates/symbio-plugin-p2/src/b.rs': 'fn g(_: AlphaThing) {}\n',
    },
    { verbose: true },
  )
  assert.match(out, /^\s*\[\s*2\] alpha\s+AlphaThing\b/m, out)
  assert.match(out, /^\s*\[\s*1\] beta\s+BetaThing\b/m, out)
})

// D-191 后壳 crate 是 `crates/symbio`：单位取 **crate 名**（`symbio`），不再是
// `symbio/(crate root)`——后者是「`crates/` 底下统统算一个」时期的兜底标签。
test('壳 crate 直属文件也算消费方，不能被跳过', () => {
  // 反例守卫：第一版对不在 `plugins/<名>/` 下的文件返回 null 单位并跳过 ⇒ 假报 0 消费方。
  const out = audit({
    'crates/symbio-core/src/lib.rs': 'pub use alpha::*;\n',
    'crates/symbio-core/src/alpha/mod.rs': 'pub struct AlphaThing;\n',
    'crates/symbio/lib.rs': 'use crate::symbio_core::AlphaThing;\n',
  })
  assert.match(out, /AlphaThing\s+->\s+symbio/, out)
  assert.doesNotMatch(out, /--- 0 个消费方[\s\S]*\n\s*alpha\s+AlphaThing\n/, out)
})

test('真的零消费方时，仍如实报出来（不能一律放过）', () => {
  // 反例守卫：修「少算消费方」不能变成「一条候选都不报」——那样这份报告就只剩「无」。
  const out = audit({
    'crates/symbio-core/src/lib.rs': 'pub use alpha::*;\n',
    'crates/symbio-core/src/alpha/mod.rs': 'pub struct UsedThing;\npub struct DeadThing;\n',
    'crates/symbio-plugin-p1/src/a.rs': 'fn f(_: UsedThing) {}\n',
  })
  assert.match(out, /--- 0 个消费方[\s\S]*\n\s*alpha\s+DeadThing\n/, out)
  assert.doesNotMatch(out, /--- 0 个消费方[\s\S]*\n\s*alpha\s+UsedThing\n/, out)
})

test('宏生成的键计入公开面（它们没有 `pub` 关键字）', () => {
  // `keys` 的 26 个字符串键由 `define_string_key!` 展开，正则扫 `^pub const` 扫不到，
  // 而它们是消费方最多的一批（真实仓库里 `PATH` 有 17 个消费方）。
  const out = audit(
    {
      'crates/symbio-core/src/lib.rs': 'pub use keys::*;\n',
      'crates/symbio-core/src/keys/mod.rs':
        'macro_rules! define_string_key { ($t:ident, $c:ident, $n:expr) => { pub struct $t; pub const $c: $t = $t; }; }\n' +
        'define_string_key!(FooKey, FOO, "foo");\n',
      'crates/symbio-plugin-p1/src/a.rs': 'fn f(_: FooKey) {}\n',
      'crates/symbio-plugin-p2/src/b.rs': 'fn g() { let _ = FOO; }\n',
    },
    { verbose: true },
  )
  assert.match(out, /^\s*\[\s*1\] keys\s+FOO\b/m, out)
  assert.match(out, /^\s*\[\s*1\] keys\s+FooKey\b/m, out)
})

test('id 常量被它的同名 provider 用，不算下放候选', () => {
  // `EMBEDDING_ALPHA` 就是 `providers/alpha` 自己的名字，它用自己天经地义。
  // （插件工厂 id 已搬去装配方，内核不再有 `PLUGIN_ID_*`；嵌入服务 id 仍住 core。）
  const out = audit({
    'crates/symbio-core/src/lib.rs': 'pub use embedding::*;\n',
    'crates/symbio-core/src/embedding/ids.rs': 'pub const EMBEDDING_ALPHA: &str = "alpha";\n',
    'crates/symbio-providers/alpha/x.rs': 'fn f() { let _ = EMBEDDING_ALPHA; }\n',
  })
  assert.doesNotMatch(out, /EMBEDDING_ALPHA\s+->/, out)
})
