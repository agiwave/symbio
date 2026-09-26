// core-surface-audit 回归测试
//
// 本脚本是**报告型**（退出码恒 0），所以它的失效形态不是「假绿灯」而是**说假话**：
// 报告少算消费方 ⇒ 在用的东西被列成「下放候选」，读的人顺着去改本来没坏的东西。
// 开发过程中**真的踩了两次**，两条都写成了回归：
//
//   1. **多条通配互相覆盖**：根 `mod.rs` 有 `pub use plugin::*` / `keys::*` / `logger::*`
//      三条通配，第一版把它们存进同一个 Map 键 `'*'` ⇒ 后一条盖掉前一条，
//      公开面从 251 个掉到 134 个，`PLUGIN_*` / `PathKey` / `PluginStopReason` / `KEY_*`
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
      'symbio/src/symbio_core/mod.rs': 'pub use alpha::*;\npub use beta::*;\n',
      'symbio/src/symbio_core/alpha/mod.rs': 'pub struct AlphaThing;\n',
      'symbio/src/symbio_core/beta/mod.rs': 'pub struct BetaThing;\n',
      'symbio/src/plugins/p1/a.rs': 'fn f(_: AlphaThing, _: BetaThing) {}\n',
      'symbio/src/plugins/p2/b.rs': 'fn g(_: AlphaThing) {}\n',
    },
    { verbose: true },
  )
  assert.match(out, /^\s*\[\s*2\] alpha\s+AlphaThing\b/m, out)
  assert.match(out, /^\s*\[\s*1\] beta\s+BetaThing\b/m, out)
})

test('`symbio/src` 直属文件也算消费方，不能被跳过', () => {
  // 反例守卫：第一版对不在 `plugins/<名>/` 下的文件返回 null 单位并跳过 ⇒ 假报 0 消费方。
  const out = audit({
    'symbio/src/symbio_core/mod.rs': 'pub use alpha::*;\n',
    'symbio/src/symbio_core/alpha/mod.rs': 'pub struct AlphaThing;\n',
    'symbio/src/lib.rs': 'use crate::symbio_core::AlphaThing;\n',
  })
  assert.match(out, /AlphaThing\s+->\s+symbio\/\(crate root\)/, out)
  assert.doesNotMatch(out, /--- 0 个消费方[\s\S]*\n\s*alpha\s+AlphaThing\n/, out)
})

test('真的零消费方时，仍如实报出来（不能一律放过）', () => {
  // 反例守卫：修「少算消费方」不能变成「一条候选都不报」——那样这份报告就只剩「无」。
  const out = audit({
    'symbio/src/symbio_core/mod.rs': 'pub use alpha::*;\n',
    'symbio/src/symbio_core/alpha/mod.rs': 'pub struct UsedThing;\npub struct DeadThing;\n',
    'symbio/src/plugins/p1/a.rs': 'fn f(_: UsedThing) {}\n',
  })
  assert.match(out, /--- 0 个消费方[\s\S]*\n\s*alpha\s+DeadThing\n/, out)
  assert.doesNotMatch(out, /--- 0 个消费方[\s\S]*\n\s*alpha\s+UsedThing\n/, out)
})

test('宏生成的键计入公开面（它们没有 `pub` 关键字）', () => {
  // `keys` 的 26 个字符串键由 `define_string_key!` 展开，正则扫 `^pub const` 扫不到，
  // 而它们是消费方最多的一批（真实仓库里 `PATH` 有 17 个消费方）。
  const out = audit(
    {
      'symbio/src/symbio_core/mod.rs': 'pub use keys::*;\n',
      'symbio/src/symbio_core/keys/mod.rs':
        'macro_rules! define_string_key { ($t:ident, $c:ident, $n:expr) => { pub struct $t; pub const $c: $t = $t; }; }\n' +
        'define_string_key!(FooKey, FOO, "foo");\n',
      'symbio/src/plugins/p1/a.rs': 'fn f(_: FooKey) {}\n',
      'symbio/src/plugins/p2/b.rs': 'fn g() { let _ = FOO; }\n',
    },
    { verbose: true },
  )
  assert.match(out, /^\s*\[\s*1\] keys\s+FOO\b/m, out)
  assert.match(out, /^\s*\[\s*1\] keys\s+FooKey\b/m, out)
})

test('插件 id 常量被它自己的插件用，不算下放候选', () => {
  // `PLUGIN_ALPHA` 就是 `plugins/alpha` 自己的名字，它用自己天经地义。
  const out = audit({
    'symbio/src/symbio_core/mod.rs': 'pub use keys::*;\n',
    'symbio/src/symbio_core/keys/mod.rs': 'pub const PLUGIN_ALPHA: &str = "alpha";\n',
    'symbio/src/plugins/alpha/x.rs': 'fn f() { let _ = PLUGIN_ALPHA; }\n',
  })
  assert.doesNotMatch(out, /PLUGIN_ALPHA\s+->/, out)
})
