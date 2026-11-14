/**
 * plugin-surface-audit 回归测试 —— 证明它**会红**（HS-001…004，D-253 / D-254）
 *
 * 一个只会亮绿灯的守卫等于没有守卫。逐条注入真实违规并断言脚本变红：
 *   ④ 控制面 `symbio-plugin-loop` 直接依赖 core → exit 0 —— 影响类分档（DE-026）；
 *   ⑤ 门面白名单条数超过基线 → exit 1 —— 只许降（HS-002）；
 *   ⑥ 范围读不出（缺 `crates/`）→ exit 2；
 *   ⑦ 门面里有一条扩展层零引用的符号 → exit 1 —— 白名单不许有空转条目（HS-003）；
 *   ⑧ 胶水层的嵌入臂被设为默认 → exit 1，⑨ 不是 `optional` → exit 1 —— 重臂默认必须关（HS-004，D-254）；
 *   ⑩ 门面自己的 manifest 依赖旧 core 的 crate 名 → exit 1 —— 机制臂归零之后把回头路堵死（HS-005，D-276）；
 *   ⑪ 门面模块级转发条数超过基线 → exit 1 —— 一行换 N 条也得在账上有数（HS-006，D-278）；
 *   ⑫ 机制臂块回到门面 → exit 1 —— 判据按**来源分区**读，不靠一条会失灵的块锚点（D-280）；
 *   ⑬ 门面一条 `pub use …::` 都读不到 → exit 1 —— 「0 条机制臂」与「一条都没读到」不许长得一样（D-280）。
 *
 * 插件的 **manifest 依赖边**不在这里判：它只有 `plugin-entry-audit` 的 E-009 一个
 * owner（按身份分档：扩展层 plugin-api / providers，控制面另许 kernel；`D-418`）。
 * 本脚本只剩源码前缀那一半。
 *
 * 用法：node --test scripts/audits/plugin-surface-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

import {
  CONTROL_PLANE,
  readGrammar,
  readWhitelist,
  MODULE_REEXPORT_BASELINE,
  RETIRED_CORE_CRATE,
  RETIRED_CORE_MODULE,
  SDK_BASELINE,
  WHITELIST_BASELINE,
} from './plugin-surface-audit.mjs'
import { rmTree } from '../fs-walk.mjs'

const script = fileURLToPath(new URL('./plugin-surface-audit.mjs', import.meta.url))


const CLEAN_PLUGIN = `use symbio_plugin_api::{Event, Plugin};

pub fn handle(e: Event) -> Plugin {
    let _ = e;
    Plugin
}
`

const CLEAN_MANIFEST = `[package]
name = "symbio-plugin-fix"
version = "0.1.0"

[dependencies]
symbio-plugin-api = { path = "../symbio-plugin-api" }
serde = "1"
`

/** 造一个最小仓库：门面 + 若干插件目录。 */
function fixture(plugins) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'plugin-surface-'))
  const crates = path.join(root, 'crates')
  fs.mkdirSync(crates)
  for (const [name, spec] of Object.entries(plugins)) {
    const dir = path.join(crates, name)
    fs.mkdirSync(path.join(dir, 'src'), { recursive: true })
    fs.writeFileSync(path.join(dir, 'Cargo.toml'), spec.manifest ?? CLEAN_MANIFEST)
    fs.writeFileSync(path.join(dir, 'src', 'lib.rs'), spec.rs ?? CLEAN_PLUGIN)
  }
  return root
}

/** 胶水层 `symbio-providers` 的最小 manifest：用来判 HS-004（重臂默认必须关）。 */
const PROVIDERS_OK = `[package]
name = "symbio-providers"

[dependencies]
symbio-provider-embedding = { path = "../symbio-provider-embedding", optional = true }

[features]
default = []
embedding = ["dep:symbio-provider-embedding"]
`

function runOn(root) {
  return spawnSync(process.execPath, [script, `--root=${root}`], { encoding: 'utf8' })
}

test('① 干净夹具通过', () => {
  const root = fixture({
    'symbio-plugin-fix': {},
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event, Plugin};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /HS-001 扩展层零直接依赖/)
  } finally {
    rmTree(root)
  }
})

test('② 源码直取 core ⇒ 红', () => {
  const root = fixture({
    'symbio-plugin-fix': { rs: 'use symbio_sdk::Event;\npub fn f(_: Event) {}\n' },
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /symbio_sdk::/)
    assert.match(r.stdout, /HS-001/)
  } finally {
    rmTree(root)
  }
})

test('②b 源码直取 kernel ⇒ 红（D-418：语法也走门面）', () => {
  const root = fixture({
    'symbio-plugin-fix': {
      rs: 'use symbio_kernel::event::ClosureReason;\npub fn f(_: ClosureReason) {}\n',
    },
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /symbio_kernel::/)
    assert.match(r.stdout, /HS-001/)
  } finally {
    rmTree(root)
  }
})

test('②c 控制面源码直取 kernel ⇒ 绿（源码前缀豁免）', () => {
  const root = fixture({
    'symbio-plugin-loop': {
      rs: 'use symbio_kernel::event::ClosureReason;\npub fn f(_: ClosureReason) {}\n',
      manifest: CLEAN_MANIFEST.replace('symbio-plugin-fix', 'symbio-plugin-loop'),
    },
    'symbio-plugin-fix': {},
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('③ 只在注释里提 core ⇒ 绿（判代码不判措辞）', () => {
  const root = fixture({
    'symbio-plugin-fix': {
      rs: '//! 见 `symbio_sdk::vdfs::address` 的改写规则。\n/// 同上：symbio_kernel::actors。\nuse symbio_plugin_api::Event;\n',
    },
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('④ 控制面插件源码直取 sdk ⇒ 绿（源码前缀豁免；manifest 归 E-009）', () => {
  const root = fixture({
    'symbio-plugin-loop': {
      rs: 'use symbio_sdk::Event;\npub fn f(_: Event) {}\n',
      manifest: CLEAN_MANIFEST.replace('symbio-plugin-fix', 'symbio-plugin-loop'),
    },
    // 门面里那条 `Event` 必须有扩展层消费方，否则 HS-003 反过来判它空转
    'symbio-plugin-fix': {},
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.equal(CONTROL_PLANE.includes('symbio-plugin-loop'), true)
  } finally {
    rmTree(root)
  }
})

test('⑦ 白名单超基线 ⇒ 红（HS-002 只许降）', () => {
  const many = Array.from({ length: WHITELIST_BASELINE + 5 }, (_, i) => `S${i}`)
  const root = fixture({
    'symbio-plugin-fix': {},
    'symbio-plugin-api': { rs: `pub use symbio_sdk::{\n    ${many.join(', ')},\n};\n` },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /HS-002/)
  } finally {
    rmTree(root)
  }
})

test('⑦b SDK 臂超基线 ⇒ 红（D-269：机制面同进棘轮）', () => {
  const many = Array.from({ length: SDK_BASELINE + 3 }, (_, i) => `S${i}`)
  const root = fixture({
    // 每一条都有人用（HS-003 不参与本用例），只有条数越界 ⇒ 红只能来自 HS-002。
    'symbio-plugin-fix': {
      rs:
        many.map((s) => `pub fn use_${s}() { symbio_plugin_api::${s}(); }`).join(' ') +
        ' pub fn ev(e: symbio_plugin_api::Event) { let _ = e; }',
    },
    'symbio-plugin-api': {
      rs: `pub use symbio_sdk::{ ${many.join(', ')} };`,
    },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /机制臂（SDK）/, '报的必须是 SDK 臂越界，不是别的错')
  } finally {
    rmTree(root)
  }
})

test('⑦c SDK 臂恰好等于基线 ⇒ 绿（棘轮不动就没事）', () => {
  const many = Array.from({ length: SDK_BASELINE }, (_, i) => `S${i}`)
  const root = fixture({
    'symbio-plugin-fix': {
      rs:
        many.map((s) => `pub fn use_${s}() { symbio_plugin_api::${s}(); }`).join(' ') +
        ' pub fn ev(e: symbio_plugin_api::Event) { let _ = e; }',
    },
    'symbio-plugin-api': {
      rs: `pub use symbio_sdk::{ ${many.join(', ')} };`,
    },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑨ 白名单里的空转条目 ⇒ 红（HS-003）', () => {
  const root = fixture({
    'symbio-plugin-fix': {},
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event, Ghost};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /HS-003/)
    assert.match(r.stdout, /Ghost/)
  } finally {
    rmTree(root)
  }
})

test('⑨b `X as _` 算消费方（HS-003 认 `as` 前的门面符号）', () => {
  const root = fixture({
    // 把 trait 带进作用域只为调方法——既不写别名，消费的也是 `Event` 本身。
    'symbio-plugin-fix': { rs: 'use symbio_plugin_api::{Event as _};\npub fn f() {}\n' },
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑧ 范围读不出 ⇒ exit 2', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'plugin-surface-empty-'))
  try {
    const r = runOn(root)
    assert.equal(r.status, 2, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑩ 胶水层把嵌入臂设为默认 ⇒ 红（HS-004）', () => {
  const root = fixture({
    'symbio-plugin-fix': {},
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event, Plugin};\n' },
    'symbio-providers': { manifest: PROVIDERS_OK.replace('default = []', 'default = ["embedding"]') },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /HS-004/)
    assert.match(r.stdout, /embedding/)
  } finally {
    rmTree(root)
  }
})

test('⑪ 嵌入臂不是 optional ⇒ 红（HS-004）', () => {
  const root = fixture({
    'symbio-plugin-fix': {},
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event, Plugin};\n' },
    'symbio-providers': { manifest: PROVIDERS_OK.replace(', optional = true', '') },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /HS-004/)
    assert.match(r.stdout, /optional/)
  } finally {
    rmTree(root)
  }
})

test('⑫ 嵌入臂默认关闭 ⇒ 绿（HS-004 的正例）', () => {
  const root = fixture({
    'symbio-plugin-fix': {},
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event, Plugin};\n' },
    'symbio-providers': { manifest: PROVIDERS_OK },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /HS-004/)
  } finally {
    rmTree(root)
  }
})

test('⑬ 门面 manifest 依赖旧 core 的 crate 名 ⇒ 红（HS-005，D-276）', () => {
  const root = fixture({
    'symbio-plugin-fix': {},
    'symbio-plugin-api': {
      rs: 'pub use symbio_sdk::{Event, Plugin};\n',
      manifest: CLEAN_MANIFEST + `${RETIRED_CORE_CRATE} = { path = "../${RETIRED_CORE_CRATE}" }\n`,
    },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /HS-005/)
  } finally {
    rmTree(root)
  }
})

test('⑬b HS-005 退化：活跃源码里 `use symbio_core::…` ⇒ 红（守卫不是永远绿）', () => {
  // 被禁名在脚本里是拼出来的常量，若扫描面写错（比如只扫门面 manifest），这一条会变绿
  // 而真仓库里同样写的 `use` 谁也拦不住。因此必须从**活跃 crate 的源码**这一面注入。
  const root = fixture({
    'symbio-plugin-fix': { rs: `use ${RETIRED_CORE_MODULE}::Event;\npub fn f(_: Event) {}\n` },
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event, Plugin};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /HS-005/)
    assert.match(r.stdout, new RegExp(RETIRED_CORE_MODULE))
  } finally {
    rmTree(root)
  }
})

test('⑬c HS-005 退化：两种字面拼写各写一次 ⇒ 各自红', () => {
  for (const spelling of [RETIRED_CORE_MODULE, RETIRED_CORE_CRATE]) {
    const root = fixture({
      'symbio-plugin-fix': {
        manifest: CLEAN_MANIFEST + `${spelling} = "0.1"\n`,
      },
      'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event, Plugin};\n' },
    })
    try {
      const r = runOn(root)
      assert.equal(r.status, 1, `${spelling} 未被判红\n` + r.stdout + r.stderr)
      assert.match(r.stdout, /HS-005/)
      assert.match(r.stdout, new RegExp(spelling))
    } finally {
      rmTree(root)
    }
  }
})

test('⑭ 门面模块级转发超过基线 ⇒ 红（HS-006，D-278）', () => {
  const mods = Array.from({ length: MODULE_REEXPORT_BASELINE + 1 }, (_, i) => `M${i}`)
  const root = fixture({
    'symbio-plugin-fix': {
      rs: mods.map((m) => `pub fn u_${m}() { symbio_plugin_api::${m}::x(); }`).join('\n'),
    },
    'symbio-plugin-api': {
      rs: mods.map((m) => `pub use symbio_sdk::ns::${m};`).join('\n') + '\n',
    },
  })
  // 造出真模块文件：判据看文件不看名字
  const nsDir = path.join(root, 'crates', 'symbio-sdk', 'src', 'ns')
  fs.mkdirSync(nsDir, { recursive: true })
  for (const m of mods) fs.writeFileSync(path.join(nsDir, m + '.rs'), 'pub fn x() {}\n')
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /HS-006/)
    assert.match(r.stdout, /模块级转发/)
  } finally {
    rmTree(root)
  }
})

test('manifest 依赖边不在本脚本判（owner 是 E-009）', () => {
  const root = fixture({
    'symbio-plugin-fix': {
      manifest: CLEAN_MANIFEST.replace(
        'symbio-plugin-api = { path = "../symbio-plugin-api" }',
        'symbio-plugin-api = { path = "../symbio-plugin-api" }\nsymbio-sdk = { path = "../symbio-sdk" }',
      ),
    },
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('readWhitelist 只认机制臂那一个块锚点', () => {
  // ① 锚点块还在（历史形状）⇒ 逐条读出。这一断言必须走**解析**分支，不能被
  //    `i < 0 → []` 的早退蒙混过关——否则「读不出」与「已经归零」在测试里长得一模一样，
  //    正是 D-280 记的空转绿形状（夹具消失，判据扫 0 条，报绿）。
  const old = fixture({
    'symbio-plugin-api': { rs: `pub use ${RETIRED_CORE_MODULE}::{\n    A, B,\n    C,\n};\n` },
  })
  try {
    assert.deepEqual(readWhitelist(old), ['A', 'B', 'C'])
  } finally {
    rmTree(old)
  }

  // ② 只有 SDK 臂 ⇒ 机制臂归零，返回空清单而不是把 SDK 臂顶上来（D-276 / D-279）。
  //    这正是踩过的坑：块锚点被「顺手改成 symbio_sdk」，114 条 SDK 臂当场被当机制臂审。
  const sdkOnly = fixture({
    'symbio-plugin-api': { rs: 'pub use symbio_sdk::{\n    PluginDir,\n};\n' },
  })
  try {
    assert.deepEqual(readWhitelist(sdkOnly), [])
  } finally {
    rmTree(sdkOnly)
  }

  // ③ 门面文件不存在 ⇒ `null`（读不出），与 ② 的空清单分得开：前者是范围缺失，后者是归零。
  const empty = fixture({})
  try {
    assert.equal(readWhitelist(empty), null)
  } finally {
    rmTree(empty)
  }
})

test('readGrammar 收齐多个 kernel 分块，且 `as` 取别名', () => {
  const root = fixture({
    'symbio-plugin-api': {
      rs:
        'pub use symbio_kernel::{\n    Event,\n    Seq,\n};\n' +
        'pub use symbio_kernel::vdfs::{\n    VdfsError,\n    unavailable_reason as vdfs_unavailable_reason,\n};\n' +
        'pub use symbio_sdk::{\n    PluginDir,\n};\n',
    },
  })
  try {
    // 语法臂按来源分块，读的时候必须全收——只读第一块会让多半语法臂逃过 HS-003。
    assert.deepEqual(readGrammar(root), ['Event', 'Seq', 'VdfsError', 'vdfs_unavailable_reason'])
    // 机制臂块（旧 core 来源）在夹具里不存在 ⇒ 空清单，不是「读不出」
    assert.deepEqual(readWhitelist(root), [])
  } finally {
    rmTree(root)
  }
})

test('⑮ 机制臂块回到门面 ⇒ 红（HS-002）：判据靠来源分区，不靠死锚点', () => {
  // 这一条是 D-280 的正例：旧 core 整棵删掉之后，`BLOCK_START` 式的块锚点再也打不中，
  // HS-002 于是恒读到「0 条」、恒等于基线、恒绿。改判「门面把这些 `pub use` 发给了谁」之后，
  // 任何一个四臂之外的来源都算机制臂——不需要锚点，写回来就红。
  const root = fixture({
    'symbio-plugin-fix': { rs: 'use symbio_plugin_api::Event;\npub fn f(_: Event) {}\n' },
    'symbio-plugin-api': {
      rs: `pub use symbio_sdk::{Event};\npub use ${RETIRED_CORE_MODULE}::{LegacyThing};\n`,
    },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /HS-002/)
    assert.match(r.stdout, /LegacyThing/)
  } finally {
    rmTree(root)
  }
})

test('⑯ 门面一条 `pub use …::` 都读不到 ⇒ 读不出即红（D-280）', () => {
  // 「0 条机制臂」与「一条都没读到」在数字上都是 0。前者是归零，后者是判据失效——
  // 两者不许长得一样，否则删掉门面内容就能让 HS-002 永久通过。
  const root = fixture({
    'symbio-plugin-fix': {},
    'symbio-plugin-api': { rs: '// 门面被清空\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /读不出/)
  } finally {
    rmTree(root)
  }
})
