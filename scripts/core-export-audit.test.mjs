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
function audit(files, { baseline = '0:0', waivers, ledger, oneBaseline } = {}) {
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
    if (ledger !== undefined) env.CORE_LEDGER_BASELINE = String(ledger)
    if (oneBaseline !== undefined) env.CORE_LEDGER_ONE_BASELINE = String(oneBaseline)
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

const CORE_MOD = 'crates/symbio-core/src/lib.rs'
const ACTORS_MOD = 'crates/symbio-core/src/actors/mod.rs'

/** 干净夹具：`mod actors;`（私有）+ 根显式导出 + 两个插件消费 ⇒ ≥2 消费方 */
function clean(extra = {}) {
  return {
    [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing};\n',
    [ACTORS_MOD]: 'pub struct AlphaThing;\n',
    'crates/symbio-plugin-alpha/src/a.rs': 'use crate::symbio_core::AlphaThing;\npub fn f(_: AlphaThing) {}\n',
    'crates/symbio-plugin-beta/src/b.rs': 'use crate::symbio_core::AlphaThing;\npub fn g(_: AlphaThing) {}\n',
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
  assert.match(r.stderr, /lib\.rs:1/)
})

test('deep 引 `symbio_core::<域>::…` → C-002 变红', () => {
  const r = audit(
    clean({
      'crates/symbio-plugin-gamma/src/g.rs':
        'use crate::symbio_core::actors::AlphaThing;\npub fn h(_: AlphaThing) {}\n',
    }),
  )
  assert.equal(r.status, 1, '绕过根出口的深引必须红')
  assert.match(r.stderr, /C-002/)
  assert.match(r.stderr, /symbio-plugin-gamma\/src\/g\.rs:1/)
})

test('裸模块导入 `use crate::symbio_core::<域>;` → C-002 变红（旧 E-010 漏网的那种）', () => {
  const r = audit(
    clean({ 'crates/symbio-plugin-gamma/src/g.rs': 'use crate::symbio_core::actors;\npub fn h() {}\n' }),
  )
  assert.equal(r.status, 1, '没有尾 :: 的裸模块导入必须红')
  assert.match(r.stderr, /C-002/)
})

test('C-002 覆盖跨 crate（cli）与壳侧（tauri/src-tauri）', () => {
  const cli = audit(
    clean({ 'crates/symbio-cli/src/client.rs': 'use symbio::symbio_core::actors::AlphaThing;\n' }),
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
      'crates/symbio-core/src/schemas/mod.rs': 'pub mod session;\n',
      'crates/symbio-core/src/schemas/session/mod.rs': 'pub mod chat_node;\n',
      'crates/symbio-plugin-gamma/src/g.rs':
        'use crate::symbio_core::schemas::session::chat_node::ChatNode;\npub fn h() {}\n',
    }),
  )
  assert.equal(r.status, 1, 'schemas 深引过去是 §1.4 豁免，现在必须红')
  assert.match(r.stderr, /C-002/)
})

test('注释里提到、core 内部深引 → 放行（不误报）', () => {
  const r = audit(
    clean({
      'crates/symbio-plugin-gamma/src/g.rs': '// 规则见 symbio_core::actors::AlphaThing 的文档\npub fn h() {}\n',
      'crates/symbio-core/src/llm/x.rs':
        '// core 内部：路径照写\nuse crate::symbio_core::actors::AlphaThing;\npub fn y(_: AlphaThing) {}\n',
    }),
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('行内豁免：非空理由放行，空理由不放行', () => {
  const ok = audit(
    clean({
      'crates/symbio-plugin-gamma/src/g.rs':
        'use crate::symbio_core::actors::AlphaThing; // core-export-allow: 桥接期由本插件持有\npub fn h(_: AlphaThing) {}\n',
    }),
  )
  assert.equal(ok.status, 0, ok.stderr + ok.stdout)

  const bad = audit(
    clean({
      'crates/symbio-plugin-gamma/src/g.rs':
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
      'crates/symbio-plugin-alpha/src/lonely.rs': 'use crate::symbio_core::LonelyThing;\npub fn l(_: LonelyThing) {}\n',
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
  assert.match(notInSurface.stderr, /已不在(内核)?公共面/)
})

test('自引用：`PLUGIN_ID_ALPHA` 被同名插件用，不算单消费方', () => {
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, PLUGIN_ID_ALPHA};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub const PLUGIN_ID_ALPHA: &str = "alpha";\n',
      'crates/symbio-plugin-alpha/src/id.rs': 'pub const ID: &str = PLUGIN_ID_ALPHA;\n',
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
      'crates/symbio-plugin-alpha/src/lonely.rs':
        'use crate::symbio_core::LonelyThing;\npub fn l(_: LonelyThing) {}\n',
      'crates/symbio-plugin-beta/src/lonely.test.rs':
        'use crate::symbio_core::LonelyThing;\n#[test]\nfn t() { let _ = size_of::<LonelyThing>(); }\n',
    }),
  )
  assert.equal(r.status, 1, '测试不算消费方 ⇒ 仍应判单消费方')
  assert.match(r.stderr, /单消费方/)
})

test('C-003 口径：`#[cfg(test)] mod test_support;` 不算生产消费方', () => {
  // `test_support.rs` 命名不是 `.test.rs`，旧口径只看文件名 ⇒ 把测试助手读成生产消费方。
  // 真仓的 `EventStore` 正是这么进了 C0 工作清单（唯一「生产」消费方是 loop 的测试助手）。
  const fixture = (decl) => ({
    [CORE_MOD]: 'mod actors;\npub use actors::AlphaThing;\n',
    [ACTORS_MOD]: 'pub struct AlphaThing;\n',
    'crates/symbio-plugin-alpha/src/test_support.rs':
      'use crate::symbio_core::AlphaThing;\npub fn s(_: AlphaThing) {}\n',
    'crates/symbio-plugin-alpha/src/mod.rs': decl,
  })
  const cfgTest = audit(fixture('#[cfg(test)]\nmod test_support;\n'))
  assert.equal(cfgTest.status, 0, '只在测试构建里存在 ⇒ 合法根出口（仅测试在用）')

  const production = audit(fixture('mod test_support;\n'))
  assert.equal(production.status, 1, '没有 cfg(test) ⇒ 仍是生产单消费方')
  assert.match(production.stderr, /单消费方/)
})


test('C-003 口径：core 内另一生产模块也在用 ⇒ 不算下放候选（放行，W1）', () => {
  // 一个符号只跨出 core 到一个插件，但**另一个 core 域**也在用它：下沉到那个插件当场
  // 编译不过（core 不得反向依赖插件）⇒ 它本来就该留根出口，不是「单消费方接缝」。
  // 这正是豁免账本大半条目的理由（「core 内另有消费方，审计只数跨出 core 的那一个」）。
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, SharedThing};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct SharedThing;\n',
      'crates/symbio-plugin-alpha/src/shared.rs':
        'use crate::symbio_core::SharedThing;\npub fn s(_: SharedThing) {}\n',
      'crates/symbio-core/src/projection/p.rs':
        'use crate::symbio_core::actors::SharedThing;\npub fn p(_: SharedThing) {}\n',
    }),
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('C-003 口径：**同域使用也是消费方** ⇒ 不是下放候选（放行；2026-10-14 改）', () => {
  // 这条测试原先断言**相反**的答案（「同域提及不算第二个消费模块，仍判红」），
  // **钉错了**，已改。错在哪：**消费就是依赖，自己依赖自己也是依赖。**
  // `LonelyThing` 声明在 `actors`、被 `actors/other.rs` 用着，那就是 `actors` 在消费它——
  // 凭什么把它沉到 `plugins/alpha`？滤掉 core 内部的依赖等于**假装 core 不用它**。
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, LonelyThing};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct LonelyThing;\n',
      'crates/symbio-plugin-alpha/src/lonely.rs':
        'use crate::symbio_core::LonelyThing;\npub fn l(_: LonelyThing) {}\n',
      'crates/symbio-core/src/actors/other.rs':
        'use crate::symbio_core::LonelyThing;\npub fn o(_: LonelyThing) {}\n',
    }),
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('C-003 口径：根 `mod.rs` 的导出清单**不算**消费方（仍判红）', () => {
  // 导出清单要单独排掉：`pub use <域>::{…}` 会把几乎每个符号都写成命中，
  // 算进来等于让每个符号自己给自己凑一个消费方。
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, LonelyThing};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct LonelyThing;\n',
      'crates/symbio-plugin-alpha/src/lonely.rs':
        'use crate::symbio_core::LonelyThing;\npub fn l(_: LonelyThing) {}\n',
    }),
  )
  assert.equal(r.status, 1, '导出清单不是消费方——core 内部确实没人用它，该下沉')
  assert.match(r.stderr, /单消费方/)
})

test('C-003 口径：core 内部**用它**（含实现、约束）⇒ 不是下放候选（放行）', () => {
  // `SoloPort` 的真实形状：trait 声明与 `impl … for …` 同在 `adapters`/`actors` 域。
  // 「自己依赖自己也是依赖」——core 在用它，它就是 ≥2 个单位共享的机制，下沉到那个
  // 唯一外部消费方会让 core 反向依赖插件。
  //
  // ⚠️ 这一版**不再需要**为 trait 实现单开一个探测器（`coreBound` 已删）：同域的
  // 任何非声明引用都会把那个域算成消费方。判据更宽也更简单，且没有恒真的分支。
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, SoloPort};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub trait SoloPort {}\n',
      // 同域的 `use` 与 `impl` 都在——两者都是依赖
      'crates/symbio-core/src/actors/impl_solo.rs':
        'use super::SoloPort;\npub struct Real;\nimpl SoloPort for Real {}\n',
      // 泛型约束同样算依赖：`impl<Foo: SoloPort>` 要求 `SoloPort` 存在
      'crates/symbio-core/src/actors/bounded.rs':
        'pub struct Bar;\npub struct X;\nimpl<Foo: SoloPort> Bar for X {}\n',
      'crates/symbio-plugin-alpha/src/only.rs':
        'use crate::symbio_core::SoloPort;\npub fn s(_: &dyn SoloPort) {}\n',
    }),
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

test('C-003 口径：同域只有**声明点与导出清单** ⇒ 仍判红（声明不是使用）', () => {
  // 删掉口径 3 之后有个副作用必须钉住：域的 `mod.rs` 里 `pub struct X;` 与
  // `pub use …X…` 都是**声明 / 清单**，不是使用。不滤掉它们的话每个符号都会被自己的
  // 声明算成一个 core 消费方 ⇒ 判据恒绿（实测那样会得到 0/0）。
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, LonelyThing};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct LonelyThing;\n',
      'crates/symbio-plugin-alpha/src/lonely.rs':
        'use crate::symbio_core::LonelyThing;\npub fn l(_: LonelyThing) {}\n',
    }),
  )
  assert.equal(r.status, 1, '只有声明与清单 ⇒ core 内部确实没人用它，该下沉')
  assert.match(r.stderr, /单消费方/)
})

test('C-003 规则 4：唯一消费方是 **core 自己** ⇒ 不是下放候选（无外部消费方）', () => {
  // 「消费方 == 1 且那个消费方是 symbio_core」这一档：即便下沉，下沉目标就是 core 自己
  // ⇒ 无需下沉。它在模型里落不到 `one` 档——`units` 数的是**跨 core** 的生产消费方，
  // core 内部的依赖由 `coreConsumers` 单独数。所以 core 自用且无外部消费方 ⇒ 既不是
  // 单消费方接缝，也不会被建议下沉。
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, CoreOnly};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct CoreOnly;\n',
      'crates/symbio-core/src/actors/inner.rs':
        'use crate::symbio_core::CoreOnly;\npub fn f(_: CoreOnly) {}\n',
      'crates/symbio-plugin-alpha/src/uses_other.rs':
        'use crate::symbio_core::AlphaThing;\npub fn g(_: AlphaThing) {}\n',
    }),
  )
  // CoreOnly 无外部消费方 ⇒ 落「0 消费方」档（根出口对它没用）而不是「单消费方」档；
  // 断言点是**它没有被当成下放候选**。
  const out = r.stdout + r.stderr
  assert.doesNotMatch(out, /CoreOnly.*单消费方|单消费方[^\n]*CoreOnly/)
  assert.match(out, /0 消费方/)
})

test('C-003 口径：仅测试在用 ⇒ 合法根出口（放行，不收窄）', () => {
  // 收窄它会把那个测试编译坏（C-002 又不许它深引）⇒「测试在用」足以让它留在根出口。
  const r = audit(
    clean({
      [CORE_MOD]: 'mod actors;\npub use actors::{AlphaThing, TestOnlyThing};\n',
      [ACTORS_MOD]: 'pub struct AlphaThing;\npub struct TestOnlyThing;\n',
      'crates/symbio-plugin-beta/src/only.test.rs':
        'use crate::symbio_core::TestOnlyThing;\n#[test]\nfn t() { let _ = size_of::<TestOnlyThing>(); }\n',
    }),
  )
  assert.equal(r.status, 0, r.stderr + r.stdout)
})

// ==================== C-004：非内核根账本只降不升 ====================

/** A2 之后的形状：根有 `pub use kernel::*`，只有门面里的符号受 C-003 判。 */
function withKernel(extra = {}) {
  return {
    [CORE_MOD]: 'mod actors;\nmod kernel;\npub use actors::DomainThing;\npub use kernel::*;\n',
    [ACTORS_MOD]: 'pub struct DomainThing;\npub struct CoreThing;\n',
    'crates/symbio-core/src/kernel.rs': 'pub use super::actors::{CoreThing};\n',
    'crates/symbio-plugin-a/src/a.rs': 'use crate::symbio_core::CoreThing;\npub fn f(_: CoreThing) {}\n',
    'crates/symbio-plugin-b/src/b.rs': 'use crate::symbio_core::CoreThing;\npub fn g(_: CoreThing) {}\n',
    'crates/symbio-plugin-c/src/c.rs':
      'use crate::symbio_core::DomainThing;\npub fn h(_: DomainThing) {}\n',
    ...extra,
  }
}

test('C-003 收窄：内核之外的单消费方域出口不再被判（只冻结条数）', () => {
  // `DomainThing` 只有一个消费方，但它在门面之外 ⇒ C-003 不看它；内核 `CoreThing`
  // 有两个消费方 ⇒ 判据通过。域账本 1 条，上限给 1。
  const r = audit(withKernel(), { ledger: 1 })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /非内核根账本 1\/1/)
})

test('C-004：域账本条目超上限 ⇒ 红（新增共享类型默认插件本地）', () => {
  const r = audit(withKernel(), { ledger: 0 })
  assert.equal(r.status, 1, '非内核根出口只许减；要加就得改一个提交进仓库的数字')
  assert.match(r.stderr, /C-004/)
  assert.match(r.stderr, /非内核根账本 1 条 > 基线 0/)
})

test('C-004 不在无门面的夹具上生效（避免用空集冒充判据）', () => {
  const r = audit(clean(), { ledger: 0 })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.doesNotMatch(r.stdout, /C-004/)
})

test('C-004：域账本单消费方超上限 ⇒ 红（只有一个消费方的投影该下沉）', () => {
  // `DomainThing` 只有一个插件消费 ⇒ 计入单消费方格；把上限压到 0 即命中。
  const r = audit(withKernel(), { ledger: 1, oneBaseline: 0 })
  assert.equal(r.status, 1, '单消费方变多必须红')
  assert.match(r.stderr, /域账本单消费方 1 条 > 基线 0/)
})

test('C-004：单消费方等于上限 ⇒ 通过，并报出 N/N', () => {
  const r = audit(withKernel(), { ledger: 1, oneBaseline: 1 })
  assert.equal(r.status, 0, r.stderr + r.stdout)
  assert.match(r.stdout, /单消费方 1\/1/)
})
