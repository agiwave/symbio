/**
 * shared-event-vocab-audit 回归测试 —— 证明它**会红**
 *
 * 一个只会亮绿灯的守卫等于没有守卫。本用例注入真实违规并断言脚本变红：
 *   ① 干净夹具（owner 清单登记 + owner 的 `EventDecl` + 外部消费方 + 内核表上没它）→ exit 0；
 *   ② 单消费方（除 owner 外没人引用）→ exit 1 —— 共享表退化成一堆预置常量；
 *   ③ 内核 `event/mod.rs` 留着同名常量 / `grid.rs` 留着同名 `CELLS` 行 → 各 exit 1 ——
 *      两处都登记就是两个 owner，正道看起来通了其实没通；
 *   ④ 幽灵名字（清单登记了、owner 没提交 `EventDecl`）与一条名字两个 owner → 非 0；
 *   ⑤ **只报数不判红**的那一类：内核表上被两个插件同时消费、但还没走正道的名字 ⇒
 *      exit 0 + 一条说明。拦住存量就等于把「还没迁完」写成「不许迁」（DE-024 D-203 裁决三）。
 *
 * 用法：node --test scripts/shared-event-vocab-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./shared-event-vocab-audit.mjs', import.meta.url))

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

const KERNEL_MOD = `pub const EVENT_REQUEST: &str = "request";
pub const EVENT_MESSAGE_ADDED: &str = "message.added";
`
const KERNEL_GRID = `pub const CELLS: &[EventDecl] = &[
    EventDecl::new(EVENT_REQUEST, (Entity::Session, Verb::Opened)),
    EventDecl::new(EVENT_MESSAGE_ADDED, (Entity::Message, Verb::Added)),
];
`

/** owner 插件：清单登记 + 自家 `EventDecl`。 */
const OWNER = {
  'crates/symbio-plugin-loop/PLUGIN.yml': 'id: loop\nshared_events: memory.encoded\n',
  'crates/symbio-plugin-loop/src/control_decl.rs': `use symbio_core::{Entity, EventDecl, Verb};
pub const EVENTS: &[EventDecl] = &[
    EventDecl::new("memory.encoded", (Entity::Memory, Verb::Opened)),
];
`,
}
/** 外部消费方：session 在生产代码里引用同一条名字。 */
const CONSUMER = {
  'crates/symbio-plugin-session/PLUGIN.yml': 'id: session\n',
  'crates/symbio-plugin-session/src/stats.rs': `pub fn counts(kind: &str) -> bool {
    kind == "memory.encoded"
}
`,
}

function audit(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'shared-event-'))
  try {
    for (const [rel, content] of Object.entries(files)) {
      const abs = path.join(root, rel)
      fs.mkdirSync(path.dirname(abs), { recursive: true })
      fs.writeFileSync(abs, content)
    }
    const result = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env: { ...process.env, NO_COLOR: '1' },
      encoding: 'utf8',
      timeout: 30_000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    rmTree(root)
  }
}

const clean = (extra = {}) => ({
  'crates/symbio-kernel/src/event/mod.rs': KERNEL_MOD,
  'crates/symbio-kernel/src/event/grid.rs': KERNEL_GRID,
  ...OWNER,
  ...CONSUMER,
  ...extra,
})

test('① 干净夹具：owner 登记 + owner 声明 + 外部消费方 + 内核表无此行 ⇒ 绿', () => {
  const r = audit(clean())
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /✓ CS-007/)
  assert.match(r.stdout, /memory\.encoded←loop→session/)
})

test('② 除 owner 外没人引用 ⇒ 红：共享表不许收单消费方的名字', () => {
  const r = audit(clean({ 'crates/symbio-plugin-session/src/stats.rs': 'pub fn f() {}\n' }))
  assert.equal(r.status, 1)
  assert.match(r.stdout, /✗ CS-007/)
  assert.match(r.stdout, /没有任何插件在生产代码里引用/)
})

test('③ 内核 `event/mod.rs` 还挂同名常量 ⇒ 红：登记口只能有一个', () => {
  const r = audit(
    clean({
      'crates/symbio-kernel/src/event/mod.rs': `${KERNEL_MOD}pub const EVENT_MEMORY_ENCODED: &str = "memory.encoded";\n`,
    }),
  )
  assert.equal(r.status, 1)
  assert.match(r.stdout, /✗ CS-007/)
  assert.match(r.stdout, /还挂在内核/)
})

test('③ 内核 `CELLS` 还挂同名行 ⇒ 红（常量删了、行没删是同一件事的两半）', () => {
  const r = audit(
    clean({
      'crates/symbio-kernel/src/event/grid.rs': `${KERNEL_GRID}    EventDecl::new("memory.encoded", (Entity::Memory, Verb::Opened)),\n`,
    }),
  )
  assert.equal(r.status, 1)
  assert.match(r.stdout, /✗ CS-007/)
})

test('④ 幽灵名字：清单登记了、owner 没提交 `EventDecl` ⇒ 非 0 且点名清单', () => {
  const r = audit(
    clean({ 'crates/symbio-plugin-loop/src/control_decl.rs': 'pub const EVENTS: &[u8] = &[];\n' }),
  )
  assert.notEqual(r.status, 0)
  assert.match(r.stdout + r.stderr, /memory\.encoded/)
  assert.match(r.stdout + r.stderr, /EventDecl/)
})

test('④ 一条名字两个 owner ⇒ 非 0：owner 唯一是这条正道的前提', () => {
  const r = audit(
    clean({
      'crates/symbio-plugin-session/PLUGIN.yml': 'id: session\nshared_events: memory.encoded\n',
      'crates/symbio-plugin-session/src/stats.rs': `pub const EVENTS: &[symbio_core::EventDecl] = &[
    symbio_core::EventDecl::new("memory.encoded", (symbio_core::Entity::Memory, symbio_core::Verb::Opened)),
];
`,
    }),
  )
  assert.notEqual(r.status, 0)
  assert.match(r.stdout + r.stderr, /只能有一个 owner/)
})

test('⑤ 内核表上的多消费方存量：只报数，不判红', () => {
  const r = audit(
    clean({
      'crates/symbio-kernel/src/event/mod.rs': `${KERNEL_MOD}pub const EVENT_TASK_HELD: &str = "task.held";\n`,
      'crates/symbio-plugin-loop/src/hold.rs': 'pub const K: &str = "task.held";\n',
      'crates/symbio-plugin-session/src/hold.rs': 'pub const K: &str = "task.held";\n',
    }),
  )
  assert.equal(r.status, 0, r.stdout + r.stderr)
  assert.match(r.stdout, /内核表上的「task\.held」/)
})
