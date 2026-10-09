/**
 * core-export-audit 回归测试 —— 证明它**会红**
 *
 * 一个只会亮绿灯的守卫等于没有守卫。本用例注入真实违规并断言脚本变红：
 *   ① 干净夹具（根私有 `mod` + 根显式 `pub use` + 两个插件消费）→ exit 0；
 *   ② C-001：根 `pub mod <域>;`（子目录开模块门）→ exit 1；
 *   ③ C-002：deep 引 `symbio_core::<域>::…`、裸 `use crate::symbio_core::<域>;` → exit 1；
 *   ④ C-002 放行面：注释里提到、core **内部**深引 → 不误报；
 *   ⑤ 行内豁免：`core-export-allow: 理由` 放行、**空理由**不放行；
 *   ⑥ C-003：0 消费方 / 单消费方 → exit 1（棘轮基线压到 0:0 注入）；
 *   ⑦ C-003 豁免账本：有理由放行、空理由红、**失效豁免**（已 ≥2 消费方）红；
 *   ⑧ 自引用（`PLUGIN_ID_ALPHA` 被同名插件用）不算单消费方。
 *
 * 用法：node --test scripts/core-export-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./core-export-audit.mjs', import.meta.url))

/** 删临时目录 —— 逐个删，不用 rmSync(recursive)（沙箱会拦截批量删除，见 no-direct-call-audit.test.mjs）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

/**
 * 建夹具仓库 → 跑审计 → 返回 spawnSync 结果。
 *
 * 默认把棘轮压到 `0:0`（`CORE_EXPORT_BASELINE`）——夹具只有几个符号，
 * 不压基线就测不到 C-003。要测「基线放行」的用例显式传 `baseline: '10:10'`。
 * 豁免账本同理默认注入**空**对象；要测账本行为显式传 `waivers`。
 */
function audit(files, { baseline = '0:0', waivers } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'core-export-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const env = { ...process.env, NO_COLOR: '1', CORE_EXPORT_BASELINE: baseline }
    // 夹具与真仓的**豁免账本**解耦（与 BASELINE 同理：账本是仓库数据，不是守卫逻辑）。
    // 不隔离的话，真仓每登记一条豁免，就会让每个「没有那个符号」的夹具误报「已不在公开面」。
    env.CORE_EXPORT_WAIVERS = waivers ?? '{}'
    const result = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env,
      encoding: 'utf8',
      timeout: 30_000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    rmTree(root)
  }
}

const CORE_MOD = 'symbio/src/symbio_core/mod.rs'
const ACTORS_MOD = 'symbio/src/symbio_core/actors/mod.rs'

/** 干净夹具：`mod actors;`（私有）+ 根显式导出 + 两个插件消费 ⇒ ≥2 消费方 */
function clean(extra = {}) {
  return {
    [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing};\n',
    [ACTORS_MOD]: 'pub struct AlphaThing;\n',
    'symbio/src/plugins/alpha/a.rs': 'use crate::symbio_core::AlphaThing;\npub fn f(_: AlphaThing) {}\n',
    'symbio/src/plugins/beta/b.rs': 'use crate::symbio_core::AlphaThing;\npub fn g(_: AlphaThing) {}\n',
    ...extra,
  }
}

test('干净夹具通过（根私有 mod + 根显式导出 + 双消费方）', () => {
  const r = audit(clean())
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('根 `pub mod <域>;` → C-001 变红', () => {
  const r = audit(clean({ [CORE_MOD]: 'pub mod actors;\npub use actors::{AlphaThing};\n' }))
  assert.equal(r.status, 1, '子目录开模块门必须红')
  assert.match(r.stderr, /C-001/)
  assert.match(r.stderr, /mod\.rs:1/)
})

test('deep 引 `symbio_core::<域>::…` → C-002 变红', () => {
  const r = audit(
    clean({
      'symbio/src/plugins/gamma/g.rs':
        'use crate::symbio_core::actors::AlphaThing;\npub fn h(_: AlphaThing) {}\n',
    }),
  )
  assert.equal(r.status, 1, '绕过根出口的深引必须红')
  assert.match(r.stderr, /C-002/)
  assert.match(r.stderr, /gamma\/g\.rs:1/)
})

test('裸模块导入 `use crate::symbio_core::<域>;` → C-002 变红（旧 E-010 漏网的那种）', () => {
  const r = audit(
    clean({ 'symbio/src/plugins/gamma/g.rs': 'use crate::symbio_core::actors;\npub fn h() {}\n' }),
  )
  assert.equal(r.status, 1, '没有尾 :: 的裸模块导入必须红')
  assert.match(r.stderr, /C-002/)
})

test('C-002 覆盖跨 crate（cli）与壳侧（tauri/src-tauri）', () => {
  const cli = audit(
    clean({ 'cli/src/client.rs': 'use symbio::symbio_core::actors::AlphaThing;\n' }),
  )
  assert.equal(cli.status, 1, 'cli 的深引必须红')
  assert.match(cli.stderr, /C-002/)

  const shell = audit(
    clean({ 'tauri/src-tauri/src/commands.rs': 'use symbio::symbio_core::actors::AlphaThing;\n' }),
  )
  assert.equal(shell.status, 1, '壳侧的深引必须红（整棵插件树编译进壳）')
  assert.match(shell.stderr, /C-002/)
})

test('C-002 不再豁免 `schemas::`（根 `mod schemas;` 已私有化，编译器兜底）', () => {
  const r = audit(
    clean({
      'symbio/src/symbio_core/schemas/mod.rs': 'pub mod session;\n',
      'symbio/src/symbio_core/schemas/session/mod.rs': 'pub mod chat_message;\n',
      'symbio/src/plugins/gamma/g.rs':
        'use crate::symbio_core::schemas::session::chat_message::ChatMessage;\npub fn h() {}\n',
    }),
  )
  assert.equal(r.status, 1, 'schemas 深引过去是 §1.4 豁免，现在必须红')
  assert.match(r.stderr, /C-002/)
})

test('注释里提到、core 内部深引 → 放行（不误报）', () => {
  const r = audit(
    clean({
      'symbio/src/plugins/gamma/g.rs': '// 规则见 symbio_core::actors::AlphaThing 的文档\npub fn h() {}\n',
      'symbio/src/symbio_core/llm/x.rs':
        '// core 内部：路径照写\nuse crate::symbio_core::actors::AlphaThing;\npub fn y(_: AlphaThing) {}\n',
    }),
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('行内豁免：非空理由放行，空理由不放行', () => {
  const ok = audit(
    clean({
      'symbio/src/plugins/gamma/g.rs':
        'use crate::symbio_core::actors::AlphaThing; // core-export-allow: 桥接期由本插件持有\npub fn h(_: AlphaThing) {}\n',
    }),
  )
  assert.equal(ok.status, 0, ok.stderr + ok.stdout)

  const bad = audit(
    clean({
      'symbio/src/plugins/gamma/g.rs':
        'use crate::symbio_core::actors::AlphaThing; // core-export-allow:\npub fn h(_: AlphaThing) {}\n',
    }),
  )
  assert.equal(bad.status, 1, '空理由不算豁免')
  assert.match(bad.stderr, /C-002/)
})

test('C-003：0 消费方 / 单消费方 → 变红（棘轮压到 0:0）', () => {
  const zero = audit(
    clean({ [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, DeadThing};\n', [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct DeadThing;\n' }),
  )
  assert.equal(zero.status, 1, '根导出但没人用必须红')
  assert.match(zero.stderr, /C-003/)
  assert.match(zero.stderr, /0 消费方/)

  const one = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, LonelyThing};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct LonelyThing;\n',
      'symbio/src/plugins/alpha/lonely.rs': 'use crate::symbio_core::LonelyThing;\npub fn l(_: LonelyThing) {}\n',
    }),
  )
  assert.equal(one.status, 1, '只有一个模块消费必须红（下沉）')
  assert.match(one.stderr, /单消费方/)
})

test('C-003 棘轮：存量在基线内 → 放行；超基线 → 变红', () => {
  const files = clean({
    [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, DeadThing};\n',
    [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct DeadThing;\n',
  })
  assert.equal(audit(files, { baseline: '5:5' }).status, 0, '存量 ≤ 基线放行')
  assert.equal(audit(files, { baseline: '0:0' }).status, 1, '超基线必须红')
})

test('C-003 豁免账本：有理由放行、空理由红、失效豁免红', () => {
  const files = clean({
    [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, DeadThing};\n',
    [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct DeadThing;\n',
  })
  const ok = audit(files, { waivers: JSON.stringify({ DeadThing: '宿主接缝：core 不得反向依赖插件类型' }) })
  assert.equal(ok.status, 0, ok.stderr + ok.stdout)

  const empty = audit(files, { waivers: JSON.stringify({ DeadThing: '  ' }) })
  assert.equal(empty.status, 1, '空理由不算豁免')
  assert.match(empty.stderr, /理由为空/)

  const stale = audit(clean(), { waivers: JSON.stringify({ AlphaThing: '多消费方还挂豁免' }) })
  assert.equal(stale.status, 1, '已 ≥2 消费方的豁免必须判失效')
  assert.match(stale.stderr, /已失效/)

  const notInSurface = audit(clean(), { waivers: JSON.stringify({ NoSuchSymbol: '不在公开面' }) })
  assert.equal(notInSurface.status, 1, '不在公开面的豁免必须判失效')
  assert.match(notInSurface.stderr, /已不在公开面/)
})

test('自引用：`PLUGIN_ID_ALPHA` 被同名插件用，不算单消费方', () => {
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, PLUGIN_ID_ALPHA};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub const PLUGIN_ID_ALPHA: &str = "alpha";\n',
      'symbio/src/plugins/alpha/id.rs': 'pub const ID: &str = PLUGIN_ID_ALPHA;\n',
    }),
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('C-003 口径：测试文件不算架构消费方（生产 1 + 测试 1 ⇒ 单消费方）', () => {
  // 第二个「消费方」只来自 `*.test.rs`。旧口径把它算成 ≥2 ⇒ 放行；那正是 `TurnRunner`
  // 长期逃过下沉的机制（生产只有 `plugins/session`，第二个单位来自 model 的彩排测试）。
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, LonelyThing};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct LonelyThing;\n',
      'symbio/src/plugins/alpha/lonely.rs':
        'use crate::symbio_core::LonelyThing;\npub fn l(_: LonelyThing) {}\n',
      'symbio/src/plugins/beta/lonely.test.rs':
        'use crate::symbio_core::LonelyThing;\n#[test]\nfn t() { let _ = size_of::<LonelyThing>(); }\n',
    }),
  )
  assert.equal(r.status, 1, '测试不算消费方 ⇒ 仍应判单消费方')
  assert.match(r.stderr, /单消费方/)
})

test('C-003 口径：仅测试在用 ⇒ 合法根出口（放行，不收窄）', () => {
  // 收窄它会把那个测试编译坏（C-002 又不许它深引）⇒「测试在用」足以让它留在根出口。
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, TestOnlyThing};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct TestOnlyThing;\n',
      'symbio/src/plugins/beta/only.test.rs':
        'use crate::symbio_core::TestOnlyThing;\n#[test]\nfn t() { let _ = size_of::<TestOnlyThing>(); }\n',
    }),
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})
