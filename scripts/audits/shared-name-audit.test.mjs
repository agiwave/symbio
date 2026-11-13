/**
 * shared-name-audit 回归测试 —— 证明 SV-001 / SV-003 / SV-004 **会红**（D-364 / D-462）
 *
 * 一个只会亮绿灯的守卫等于没有守卫。逐条注入真实违规并断言脚本变红：
 *   ① 干净夹具（两侧只 import）→ exit 0；
 *   ② 同一个有结构名字（`vdfs/write`）在两个插件各写一次字面量 → exit 1；
 *   ③ 同名 `const`（`task_table`）在两个插件各声明一次 → exit 1；
 *   ④ 标准 MIME 类型在两个插件都出现 → exit 0（语法形状不是词汇名）；
 *   ⑤ 测试文件里的重复 → exit 0（测试不是生产消费方）；
 *   ⑥ 范围读不出 → exit 2；
 *   ⑦ 机制层手写词汇层事件名 → exit 1（SV-001 ③ 扩围）；
 *   ⑧ 带「生成，请勿手改」标记的 `test_fixtures.rs` → exit 0（单源投影）；
 *   ⑬ 无标记的 `test_fixtures.rs` 手抄 → exit 1（第二份定义）；
 *   ⑨ 登记与 024 契约表逐行相等 → exit 0（SV-003）；
 *   ⑩ 契约表有、登记没有 → exit 1；⑪ 登记了、表里没有 → exit 1；
 *   ⑫ 同一 slot 的 `fleet` 落点不一致 → exit 1；
 *   ⑭ 读点读的键都在写方账上 → exit 0（SV-004）；
 *   ⑮ 读点读了写方不写的键 → exit 1；⑯ 写方改名而读方没跟 → exit 1；
 *   ⑰ 写方源码缺失 → exit 2；⑱ 追加格与装配格重名 → exit 1。
 *
 * 用法：node --test scripts/audits/shared-name-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./shared-name-audit.mjs', import.meta.url))

/** 删临时目录 —— 逐个删，不用 rmSync(recursive)（沙箱会拦截批量删除）。 */
function rmTree(dir) {
  if (!fs.existsSync(dir)) return
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) rmTree(p)
    else fs.unlinkSync(p)
  }
  fs.rmdirSync(dir)
}

const PLAN_REL = 'docs/003-design/024-能力横向扩充的通路.md'
/** 默认契约表：表头在、零数据行。各用例要判 SV-003 时自行覆盖。 */
const DEFAULT_PLAN = '# fixture\n\n| `slot` | 顶层路径 | 实例级落点 |\n|---|---|---|\n'

/** 默认词汇层：`events.rs` 是规则源，缺了判据只能 exit 2。带生成标记（它本来就是投影）。 */
const API_EVENTS_REL = 'crates/symbio-plugin-api/src/events.rs'
const DEFAULT_API_EVENTS =
  '//! 跨插件事件名常量 —— **由 `scripts/facts/events-rs.mjs` 生成，请勿手改**。\n\n' +
  'pub const EVENT_VDFS_WRITE: &str = "vdfs/write";\n'

/** 机制层三 crate：判据要求它们都在（SV-001 ③ 的扫描范围）。 */
const DEFAULT_CORE = {
  'crates/symbio-sdk/src/lib.rs': '',
  'crates/symbio-kernel/src/lib.rs': '',
  'crates/symbio/src/lib.rs': '',
}

/**
 * SV-004 的规则源：写方（`artifact_payload` / `dispatch_calls`）、行为（`annotate`）、
 * 登记的读点（产物格三个 + 派发清单格六个，`D-471` 的「谁消费」表全表）。
 *
 * 判据不设「词汇表」——它从写方源码读出账本，再拿读点去对。夹具因此只需要写方
 * 和读点：判的是「读的键在不在账上」，不是「键有几格」。
 */
const DEFAULT_ARTIFACT = {
  'crates/symbio-plugin-tool/src/port.rs':
    'fn artifact_payload() -> serde_json::Value {\n' +
    '    serde_json::json!({\n        "tool": 1,\n        "success": 2,\n    })\n}\n' +
    'fn read_requested() {\n' +
    '    let _ = fact.payload.get("calls");\n' +
    '    let _ = call.get("id");\n    let _ = call.get("name");\n' +
    '    let _ = call.get("node_id");\n    let _ = call.get("arguments");\n' +
    '    let _ = call.get("overflow_notice");\n}\n',
  'crates/symbio-plugin-vdfs/src/behavior.rs':
    'impl ToolBehavior for FileChange {\n' +
    '    fn annotate(&self) -> Vec<(&\'static str, serde_json::Value)> {\n' +
    '        vec![("file_op", serde_json::json!("write"))]\n' +
    '    }\n}\n',
  'crates/symbio-sdk/src/projection/rebuild.rs':
    'fn artifact_nodes() {\n    let _ = e.payload.get("tool");\n}\n' +
    'fn call_tables() {\n    let _ = call.get("id");\n    let _ = e.payload.get("root_id");\n}\n',
  'crates/symbio-plugin-session/src/projections/changes.rs':
    'fn change_of() {\n    let _ = payload.get("success");\n}\n',
  'crates/symbio-plugin-session/src/transcript/live.rs':
    'fn partners() {\n    let _ = e.payload.get("tool_call_id");\n' +
    '    let _ = call.get("id");\n}\n' +
    'fn remember_request() {\n    let _ = e.payload.get("calls");\n' +
    '    let _ = call.get("id");\n}\n',
  'crates/symbio-plugin-tool/src/round_done.rs':
    'fn observe() {\n    let _ = request.payload.get("calls");\n' +
    '    let _ = call.get("id");\n}\n',
  'crates/symbio-plugin-model/src/port.rs':
    'fn turn_input() {\n    match kind {\n        EVENT_ARTIFACT_ADDED => {\n' +
    '            let _ = payload.get("tool");\n        }\n        _ => {}\n    }\n}\n' +
    'fn dispatch_calls() -> Vec<serde_json::Value> {\n' +
    '    let mut entry = serde_json::json!({\n        "id": 1,\n        "name": 2,\n        "arguments": 3,\n    });\n' +
    '    entry["node_id"] = serde_json::json!(4);\n' +
    '    entry["overflow_notice"] = serde_json::json!(5);\n    entry\n}\n' +
    'fn execute() {\n    let _ = payload.get("answer");\n    let _ = call.get("id");\n}\n',
}

/** 造一个最小仓库：`crates/<插件>/src/<文件>`，内容由调用方给。 */
function fixture(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'shared-name-'))
  for (const [rel, content] of Object.entries({
    [PLAN_REL]: DEFAULT_PLAN,
    [API_EVENTS_REL]: DEFAULT_API_EVENTS,
    ...DEFAULT_CORE,
    ...DEFAULT_ARTIFACT,
    ...files,
  })) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
  return root
}

function runOn(root) {
  return spawnSync(process.execPath, [script, `--root=${root}`], { encoding: 'utf8' })
}

test('① 干净夹具：两侧都 import，不写字面量 ⇒ 绿', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs':
      'use symbio_plugin_api::vocab::PORT_MODEL_ASSIST;\npub fn a() { let _ = PORT_MODEL_ASSIST; }\n',
    'crates/symbio-plugin-b/src/lib.rs':
      'use symbio_plugin_api::vocab::PORT_MODEL_ASSIST as P;\npub fn b() { let _ = P; }\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-001/)
  } finally {
    rmTree(root)
  }
})

test('② 同一个有结构名字在两个插件各写一次字面量 ⇒ 红', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'pub const A: &str = "vdfs/write";\n',
    'crates/symbio-plugin-b/src/lib.rs': 'pub fn b() -> &\'static str { "vdfs/write" }\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-001/)
    assert.match(r.stdout, /vdfs\/write/)
  } finally {
    rmTree(root)
  }
})

test('③ 同名 const 在两个插件各声明一次 ⇒ 红（无分隔符的名字也抓）', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'const TASK_TABLE: &str = "task_table";\n',
    'crates/symbio-plugin-b/src/lib.rs': 'pub(crate) const TASK_TABLE: &str = "task_table";\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-001/)
    assert.match(r.stdout, /task_table/)
  } finally {
    rmTree(root)
  }
})

test('④ 标准 MIME 类型在两个插件都出现 ⇒ 绿（语法形状不算词汇名）', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'pub const A: &str = "application/json";\n',
    'crates/symbio-plugin-b/src/lib.rs': 'pub fn b() -> &\'static str { "application/json" }\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑤ 测试文件里的重复 ⇒ 绿（测试不是生产消费方）', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'pub fn a() {}\n',
    'crates/symbio-plugin-a/src/foo.test.rs': 'pub const A: &str = "vdfs/write";\n',
    'crates/symbio-plugin-b/src/lib.rs': 'pub fn b() {}\n',
    'crates/symbio-plugin-b/src/bar.test.rs': 'pub const B: &str = "vdfs/write";\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑦ 机制层手写词汇层事件名 ⇒ 红（SV-001 ③）', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'pub fn a() {}\n',
    'crates/symbio-sdk/src/foo.rs': 'pub const WRITE: &str = "vdfs/write";\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-001/)
    assert.match(r.stdout, /vdfs\/write/)
    assert.match(r.stdout, /symbio-sdk/)
  } finally {
    rmTree(root)
  }
})

test('⑧ 带生成标记的 test_fixtures.rs ⇒ 绿（单源投影不是第二份定义）', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'pub fn a() {}\n',
    'crates/symbio-sdk/src/test_fixtures.rs':
      '//! core 用例事件拼写 —— **由 `scripts/facts/test-fixtures-rs.mjs` 生成，请勿手改**。\n\n' +
      'pub const EVENT_VDFS_WRITE: &str = "vdfs/write";\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑬ 手抄 test_fixtures.rs（无生成标记）⇒ 红（正是本判据要拦的第二份定义）', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'pub fn a() {}\n',
    'crates/symbio-sdk/src/test_fixtures.rs': 'pub const EVENT_VDFS_WRITE: &str = "vdfs/write";\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-001/)
    assert.match(r.stdout, /test_fixtures/)
  } finally {
    rmTree(root)
  }
})

/** 契约表正文（含表头与分隔行）。 */
function plan(rows) {
  return ['| `slot` | 顶层路径 | 实例级落点 |', '|---|---|---|', ...rows, ''].join('\n')
}

/** 一条 `submit_reports!` 登记；`fleet = null` ⇒ `fleet: None`。 */
function reportDecl(slot, fleet) {
  const value = fleet === null ? 'None' : `Some("${fleet}")`
  return (
    'pub fn f() { symbio_plugin_api::submit_reports!([ReportDecl { owner: "o", ' +
    `slot: "${slot}", order: 1, views: &[], fleet: ${value}, render: r, ` +
    'fleet_render: None, panel: None }]); }\n'
  )
}

test('⑨ 登记与 024 契约表逐行相等 ⇒ 绿（SV-003）', () => {
  const root = fixture({
    [PLAN_REL]: plan([
      '| `tiers` | `session/stats.tiers` | `fleet.by_tier` |',
      '| `cost` | `session/stats.cost` | — |',
    ]),
    'crates/symbio-plugin-slo/src/decl.rs': reportDecl('tiers', 'by_tier'),
    'crates/symbio-plugin-cost/src/decl.rs': reportDecl('cost', null),
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-003/)
  } finally {
    rmTree(root)
  }
})

test('⑩ 契约表有、登记没有 ⇒ 红（SV-003 双向）', () => {
  const root = fixture({
    [PLAN_REL]: plan([
      '| `tiers` | `session/stats.tiers` | `fleet.by_tier` |',
      '| `cost` | `session/stats.cost` | — |',
    ]),
    'crates/symbio-plugin-slo/src/decl.rs': reportDecl('tiers', 'by_tier'),
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-003/)
    assert.match(r.stdout, /cost/)
  } finally {
    rmTree(root)
  }
})

test('⑪ 登记了、契约表里没有 ⇒ 红（SV-003 双向）', () => {
  const root = fixture({
    [PLAN_REL]: plan(['| `tiers` | `session/stats.tiers` | `fleet.by_tier` |']),
    'crates/symbio-plugin-slo/src/decl.rs': reportDecl('tiers', 'by_tier'),
    'crates/symbio-plugin-cost/src/decl.rs': reportDecl('cost', null),
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-003/)
    assert.match(r.stdout, /cost/)
  } finally {
    rmTree(root)
  }
})

test('⑫ 同一 slot 的 fleet 落点不一致 ⇒ 红', () => {
  const root = fixture({
    [PLAN_REL]: plan(['| `tiers` | `session/stats.tiers` | `fleet.by_tier` |']),
    'crates/symbio-plugin-slo/src/decl.rs': reportDecl('tiers', 'axes.span'),
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-003/)
    assert.match(r.stdout, /by_tier/)
  } finally {
    rmTree(root)
  }
})

test('⑥ 范围读不出 ⇒ exit 2', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'shared-name-empty-'))
  try {
    const r = runOn(root)
    assert.equal(r.status, 2, r.stdout + r.stderr)
  } finally {
    rmTree(root)
  }
})

test('⑭ 产物的读点用常量取键 ⇒ 绿（SV-004）', () => {
  const root = fixture({
    'crates/symbio-plugin-a/src/lib.rs': 'pub fn a() {}\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-004/)
  } finally {
    rmTree(root)
  }
})

test('⑮ 读点读了一格写方不写的键 ⇒ 红（写方改名后读方静默读空）', () => {
  const root = fixture({
    'crates/symbio-plugin-session/src/projections/changes.rs':
      'fn change_of() {\n    let _ = payload.get("ghost");\n}\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-004/)
    assert.match(r.stdout, /ghost/)
  } finally {
    rmTree(root)
  }
})

test('⑯ 写方把键改名 ⇒ 红（读方仍在读旧键）', () => {
  const root = fixture({
    // 同一份文件里还住着 calls 格的登记读点 `read_requested`，覆盖时不能弄丢它，
    // 否则判据报「规则源读不出」（exit 2）而不是对账红（exit 1）。
    'crates/symbio-plugin-tool/src/port.rs':
      'fn artifact_payload() -> serde_json::Value {\n' +
      '    serde_json::json!({\n        "succeeded": 1,\n    })\n}\n' +
      'fn read_requested() {\n' +
      '    let _ = fact.payload.get("calls");\n' +
      '    let _ = call.get("id");\n    let _ = call.get("name");\n' +
      '    let _ = call.get("node_id");\n    let _ = call.get("arguments");\n' +
      '    let _ = call.get("overflow_notice");\n}\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-004/)
    assert.match(r.stdout, /tool/)
  } finally {
    rmTree(root)
  }
})

test('⑰ 写方缺失 ⇒ exit 2（规则源读不出不是「没有违规」）', () => {
  const root = fixture({ 'crates/symbio-plugin-a/src/lib.rs': 'pub fn a() {}\n' })
  try {
    fs.unlinkSync(path.join(root, 'crates/symbio-plugin-tool/src/port.rs'))
    const r = runOn(root)
    assert.equal(r.status, 2, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-004/)
  } finally {
    rmTree(root)
  }
})

test('⑱ 行为追加格与装配格重名 ⇒ 红（两个写点争同一格）', () => {
  const root = fixture({
    'crates/symbio-plugin-vdfs/src/behavior.rs':
      'impl ToolBehavior for FileChange {\n' +
      '    fn annotate(&self) -> Vec<(&\'static str, serde_json::Value)> {\n' +
      '        vec![("tool", serde_json::json!(op))]\n' +
      '    }\n}\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-004/)
  } finally {
    rmTree(root)
  }
})

test('⑲ calls 读点读了元素账上没有的键 ⇒ 红（wire_id 混进派发清单的读段）', () => {
  const root = fixture({
    'crates/symbio-sdk/src/projection/rebuild.rs':
      'fn artifact_nodes() {\n    let _ = e.payload.get("tool");\n}\n' +
      'fn call_tables() {\n    let _ = call.get("id");\n    let _ = call.get("wire_id");\n}\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-004/)
    assert.match(r.stdout, /wire_id/)
    assert.match(r.stdout, /派发清单格/)
  } finally {
    rmTree(root)
  }
})

test('⑳ calls 写方把条件格改名 ⇒ 红（读方仍读 node_id，静默读空）', () => {
  const root = fixture({
    'crates/symbio-sdk/src/projection/rebuild.rs':
      'fn artifact_nodes() {\n    let _ = e.payload.get("tool");\n}\n' +
      'fn call_tables() {\n    let _ = call.get("id");\n    let _ = call.get("node_id");\n}\n',
    'crates/symbio-plugin-model/src/port.rs':
      'fn turn_input() {\n    match kind {\n        EVENT_ARTIFACT_ADDED => {\n' +
      '            let _ = payload.get("tool");\n        }\n        _ => {}\n    }\n}\n' +
      'fn dispatch_calls() -> Vec<serde_json::Value> {\n' +
      '    let mut entry = serde_json::json!({\n        "id": 1,\n        "name": 2,\n        "arguments": 3,\n    });\n' +
      '    entry["card_id"] = serde_json::json!(4);\n    entry\n}\n' +
      // 登记读点 `execute` 也住在这份文件里，覆盖时保留，否则 exit 2 而非对账红。
      'fn execute() {\n    let _ = payload.get("answer");\n    let _ = call.get("id");\n}\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-004/)
    assert.match(r.stdout, /node_id/)
  } finally {
    rmTree(root)
  }
})

test('㉑ calls 读段函数不在 ⇒ exit 2（读段改名后判据会恒真）', () => {
  const root = fixture({
    'crates/symbio-sdk/src/projection/rebuild.rs':
      'fn artifact_nodes() {\n    let _ = e.payload.get("tool");\n}\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 2, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-004/)
  } finally {
    rmTree(root)
  }
})

test('㉒ tool_calls 补写段读 wire_id ⇒ 绿（不登记、不并账——D-471 的同形异义边界）', () => {
  const root = fixture({
    'crates/symbio-sdk/src/projection/rebuild.rs':
      'fn artifact_nodes() {\n    let _ = e.payload.get("tool");\n}\n' +
      'fn call_tables() {\n    let _ = call.get("id");\n    let _ = e.payload.get("root_id");\n}\n' +
      'fn backfill_node_ids() {\n    let _ = call.get("wire_id");\n}\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 0, r.stdout + r.stderr)
    assert.match(r.stdout, /派发清单格/)
  } finally {
    rmTree(root)
  }
})

test('㉓ 会话侧登记读点读账外键 ⇒ 红（D-471 全表登记后，五个读点都在对账）', () => {
  const root = fixture({
    'crates/symbio-plugin-session/src/transcript/live.rs':
      'fn partners() {\n    let _ = e.payload.get("tool_call_id");\n' +
      '    let _ = call.get("card_id");\n}\n' +
      'fn remember_request() {\n    let _ = e.payload.get("calls");\n' +
      '    let _ = call.get("id");\n}\n',
  })
  try {
    const r = runOn(root)
    assert.equal(r.status, 1, r.stdout + r.stderr)
    assert.match(r.stdout, /SV-004/)
    assert.match(r.stdout, /card_id/)
  } finally {
    rmTree(root)
  }
})
