/**
 * plugin-surface-audit 回归测试 —— 证明它**会红**（HS-001…004，D-253 / D-254）
 *
 * 一个只会亮绿灯的守卫等于没有守卫。逐条注入真实违规并断言脚本变红：
 *   ① 干净夹具 → exit 0（扩展层插件只写门面）；
 *   ② 源码里出现 `symbio_core::Event` → exit 1 —— 绕过门面直接取宿主面；
 *   ③ 只在注释里提 `symbio_core::vdfs::address` → exit 0 —— 判代码不判措辞；
 *   ④ manifest 的 `[dependencies]` 里有 `symbio-core` → exit 1；
 *   ⑤ `[dev-dependencies]` 里有 `symbio-core` → exit 0（测试设施不是运行时面）；
 *   ⑥ 控制面 `symbio-plugin-loop` 直接依赖 core → exit 0 —— 影响类分档（DE-026）；
 *   ⑦ 门面白名单条数超过基线 → exit 1 —— 只许降（HS-002）；
 *   ⑧ 范围读不出（缺 `crates/`）→ exit 2；
 *   ⑨ 门面里有一条扩展层零引用的符号 → exit 1 —— 白名单不许有空转条目（HS-003）；
 *   ⑩ 胶水层的嵌入臂被设为默认 → exit 1，⑪ 不是 `optional` → exit 1 —— 重臂默认必须关（HS-004，D-254）。
 *
 * 用法：node --test scripts/plugin-surface-audit.test.mjs
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
  manifestViolations,
  readGrammar,
  readWhitelist,
  SDK_BASELINE,
  WHITELIST_BASELINE,
} from './plugin-surface-audit.mjs'

const script = fileURLToPath(new URL('./plugin-surface-audit.mjs', import.meta.url))

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
    'symbio-plugin-api': { rs: 'pub use symbio_core::{Event, Plugin};\n' },
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
    'symbio-plugin-fix': { rs: 'use symbio_core::Event;\npub fn f(_: Event) {}\n' },
    'symbio-plugin-api': { rs: 'pub use symbio_core::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /symbio_core::/)
    assert.match(r.stdout, /HS-001/)
  } finally {
    rmTree(root)
  }
})

test('③ 只在注释里提 core ⇒ 绿（判代码不判措辞）', () => {
  const root = fixture({
    'symbio-plugin-fix': {
      rs: '//! 见 `symbio_core::vdfs::address` 的改写规则。\n/// 同上：symbio_kernel::actors。\nuse symbio_plugin_api::Event;\n',
    },
    'symbio-plugin-api': { rs: 'pub use symbio_core::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('④ manifest 的 [dependencies] 有 core ⇒ 红', () => {
  const root = fixture({
    'symbio-plugin-fix': {
      manifest: CLEAN_MANIFEST.replace(
        'symbio-plugin-api = { path = "../symbio-plugin-api" }',
        'symbio-plugin-api = { path = "../symbio-plugin-api" }\nsymbio-core = { path = "../symbio-core" }',
      ),
    },
    'symbio-plugin-api': { rs: 'pub use symbio_core::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /symbio-core/)
  } finally {
    rmTree(root)
  }
})

test('⑤ dev-dependencies 里的 core 不判', () => {
  const root = fixture({
    'symbio-plugin-fix': {
      manifest: `${CLEAN_MANIFEST}\n[dev-dependencies]\nsymbio-core = { path = "../symbio-core" }\n`,
    },
    'symbio-plugin-api': { rs: 'pub use symbio_core::{Event};\n' },
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑥ 控制面插件直接依赖 core ⇒ 绿', () => {
  const root = fixture({
    'symbio-plugin-loop': {
      rs: 'use symbio_core::Event;\npub fn f(_: Event) {}\n',
      manifest: CLEAN_MANIFEST.replace('symbio-plugin-fix', 'symbio-plugin-loop'),
    },
    // 门面里那条 `Event` 必须有扩展层消费方，否则 HS-003 反过来判它空转
    'symbio-plugin-fix': {},
    'symbio-plugin-api': { rs: 'pub use symbio_core::{Event};\n' },
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
    'symbio-plugin-api': { rs: `pub use symbio_core::{\n    ${many.join(', ')},\n};\n` },
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
      rs: `pub use symbio_core::{Event}; pub use symbio_sdk::{ ${many.join(', ')} };`,
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
      rs: `pub use symbio_core::{Event}; pub use symbio_sdk::{ ${many.join(', ')} };`,
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
    'symbio-plugin-api': { rs: 'pub use symbio_core::{Event, Ghost};\n' },
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
    'symbio-plugin-api': { rs: 'pub use symbio_core::{Event, Plugin};\n' },
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
    'symbio-plugin-api': { rs: 'pub use symbio_core::{Event, Plugin};\n' },
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
    'symbio-plugin-api': { rs: 'pub use symbio_core::{Event, Plugin};\n' },
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

test('manifestViolations 只认 [dependencies] 段内的禁用键', () => {
  assert.deepEqual(manifestViolations(CLEAN_MANIFEST), [])
  assert.deepEqual(
    manifestViolations('[dependencies]\nsymbio-kernel = { path = "x" }\nsymbio-deploy = { path = "y" }\n'),
    ['symbio-kernel', 'symbio-deploy'],
  )
})

test('readWhitelist 解析门面块', () => {
  const root = fixture({ 'symbio-plugin-api': { rs: 'pub use symbio_core::{\n    A, B,\n    C,\n};\n' } })
  try {
    assert.deepEqual(readWhitelist(root), ['A', 'B', 'C'])
  } finally {
    rmTree(root)
  }
})

test('readGrammar 收齐多个 kernel 分块，且 `as` 取别名', () => {
  const root = fixture({
    'symbio-plugin-api': {
      rs:
        'pub use symbio_kernel::{\n    Event,\n    Seq,\n};\n' +
        'pub use symbio_kernel::vdfs::{\n    VdfsError,\n    unavailable_reason as vdfs_unavailable_reason,\n};\n' +
        'pub use symbio_core::{\n    PluginDir,\n};\n',
    },
  })
  try {
    // 语法臂按来源分块，读的时候必须全收——只读第一块会让多半语法臂逃过 HS-003。
    assert.deepEqual(readGrammar(root), ['Event', 'Seq', 'VdfsError', 'vdfs_unavailable_reason'])
    assert.deepEqual(readWhitelist(root), ['PluginDir'])
  } finally {
    rmTree(root)
  }
})
