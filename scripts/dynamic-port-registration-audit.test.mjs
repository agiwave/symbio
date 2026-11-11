/**
 * dynamic-port-registration-audit 回归测试
 *
 * 判据面（DPR-001…004，见 D-445 / D-447 ③）：四类动态注册反例形状——`<port:…>` 行名
 * 反推、裸 `executors.insert(` 覆盖、裸 `list(TOOL)` 全量投影、`p2.route(` 宿主路由回落
 * ——都不许出现在生产代码里。这一族守卫最危险的失效是**假绿**：一句「已删除」的注释
 * 就当守卫在、扫不到输入就当通过。所以夹具钉：
 *
 *   ① 干净树 ⇒ 绿；②…⑤ 四类形状各注入一条 ⇒ 红并点名 `文件:行号`；
 *   ⑥ 注释叙述不算违规；⑦ 「靠注释声明已删除」骗不过真实代码行；
 *   ⑧ `.test.rs` 排除；⑨ 读不出 ⇒ 红。
 *
 * 夹具树用临时目录 + 脚本的 `--root=<dir>`（与 engine-ledger-audit.test.mjs 同约定），
 * 不碰真实 crates。
 *
 * 跑法：node --test scripts/dynamic-port-registration-audit.test.mjs
 */

import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./dynamic-port-registration-audit.mjs', import.meta.url))

const PORT_RS = 'crates/symbio-sdk/src/capability/port.rs'
const HOST_LIB = 'crates/symbio-host/src/lib.rs'
const TOOL_PORT = 'crates/symbio-plugin-tool/src/port.rs'
const TURN_RS = 'crates/symbio-plugin-session/src/plugin/turn.rs'
const TOOL_PORT_TEST = 'crates/symbio-plugin-tool/src/port.test.rs'

/**
 * 干净夹具：四类反例形状都不在。刻意让每份文件带上**承接体**的真实代码——按端口键
 * 分组判重、`owners.insert` 冲突检查、按 `bound_ports` 过滤的清单装配——证明判据认的是
 * 形状而不是「有没有写守卫」。
 */
function cleanFiles() {
  return {
    [PORT_RS]: [
      '/// 装配期判重（D-445 ③）：按行上的端口键分组，同键多绑定落错误。',
      'pub fn capability_port_check(rows: &[Row]) {',
      '    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();',
      '    for row in rows {',
      '        groups.entry(row.port.clone()).or_default().push(row.name.clone());',
      '    }',
      '}',
      '',
    ].join('\n'),
    [HOST_LIB]: [
      '/// 折执行体：按端口键判重，同键两绑定当场拒绝，不 insert 覆盖。',
      'pub fn fold(rows: Vec<Row>) -> Result<(), String> {',
      '    let mut owners: BTreeMap<String, String> = BTreeMap::new();',
      '    for row in rows {',
      '        if let Some(previous) = owners.insert(row.port.clone(), row.owner.clone()) {',
      '            if previous != row.owner { return Err(format!("冲突绑定: {}", row.port)); }',
      '        }',
      '    }',
      '    Ok(())',
      '}',
      '',
    ].join('\n'),
    [TOOL_PORT]: [
      '/// 工具清单：显式暴露 ∩ 当前作用域 ∩ 有效绑定。',
      'pub fn exposed_tools(rows: Vec<Row>, bound_ports: &HashSet<String>) -> Vec<Tool> {',
      '    rows.into_iter().filter(|r| bound_ports.contains(&r.port)).collect()',
      '}',
      '',
    ].join('\n'),
    [TURN_RS]: [
      'pub fn assemble(rows: Vec<Row>, bound_ports: &HashSet<String>) -> Vec<Tool> {',
      '    exposed_tools(rows, bound_ports)',
      '}',
      '',
    ].join('\n'),
  }
}

function sandbox(files) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'dpr-audit-'))
  for (const [rel, content] of Object.entries(files)) {
    const abs = path.join(root, rel)
    fs.mkdirSync(path.dirname(abs), { recursive: true })
    fs.writeFileSync(abs, content)
  }
  return { root, cleanup: () => fs.rmSync(root, { recursive: true, force: true }) }
}

function run(root) {
  const r = spawnSync(process.execPath, [script, `--root=${root}`], {
    encoding: 'utf8',
    env: { ...process.env, NO_COLOR: '1' },
    timeout: 30_000,
  })
  assert.ifError(r.error)
  return { status: r.status, out: `${r.stdout}${r.stderr}` }
}

test('干净夹具树：四类反例形状都不在 ⇒ 通过', () => {
  const box = sandbox(cleanFiles())
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
    assert.match(r.out, /通过/)
  } finally {
    box.cleanup()
  }
})

test('DPR-001：capability_port_check 里出现 strip_prefix("<port:") 反推 ⇒ 红并点名 文件:行号', () => {
  const files = cleanFiles()
  files[PORT_RS] += 'fn stray(name: &str) { let _ = name.strip_prefix("<port:"); }\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /DPR-001/)
    assert.match(r.out, /port\.rs:\d+/)
    assert.match(r.out, /strip_prefix/)
  } finally {
    box.cleanup()
  }
})

test('DPR-002：host 折执行体表出现裸 executors.insert( ⇒ 红（已有 owners.insert 冲突检查也不豁免）', () => {
  const files = cleanFiles()
  // HOST_LIB 里已经有 `owners.insert` 的按端口键冲突检查；这一条证明「冲突检查是承接体，
  // 不是保留裸 insert 的许可」——D-445 ③ 的原话是「不 insert 覆盖」。
  files[HOST_LIB] += 'fn stray() { executors.insert(port, executor); }\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /DPR-002/)
    assert.match(r.out, /lib\.rs:\d+/)
    assert.match(r.out, /executors\.insert\(/)
  } finally {
    box.cleanup()
  }
})

test('DPR-003：工具清单装配出现裸 list(TOOL) 全量投影 ⇒ 红并点名 文件:行号', () => {
  const files = cleanFiles()
  files[TOOL_PORT] += 'fn stray(v: &Visitor) { let _ = v.list(TOOL); }\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /DPR-003/)
    assert.match(r.out, /port\.rs:\d+/)
    assert.match(r.out, /list\(TOOL\)/)
  } finally {
    box.cleanup()
  }
})

test('DPR-004：工具派发出现 p2.route( 未命中回落 ⇒ 红并点名 文件:行号', () => {
  const files = cleanFiles()
  files[TOOL_PORT] += 'fn fallback(p2: Arc<Host>) { Box::pin(async move { p2.route(ctx).await }) }\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /DPR-004/)
    assert.match(r.out, /port\.rs:\d+/)
    assert.match(r.out, /p2\.route\(/)
  } finally {
    box.cleanup()
  }
})

test('注释叙述不算违规：doc comment 里写「反推已删除」、无真实代码 ⇒ 通过', () => {
  const files = cleanFiles()
  files[PORT_RS] = [
    '/// 迁移说明：`strip_prefix("<port:")` 那层行名反推已退场（D-445 ③）。',
    'pub fn capability_port_check(rows: &[Row]) {}',
    '',
  ].join('\n')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('「靠注释声明已删除」骗不过真实代码行：注释 + p2.route( 代码 ⇒ 仍红', () => {
  const files = cleanFiles()
  files[TOOL_PORT] = [
    '/// 回落已删除：未命中不再 p2.route 兜底。',
    'pub fn dispatch(p2: Arc<Host>) { p2.route(ctx); }',
    '',
  ].join('\n')
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /DPR-004/)
  } finally {
    box.cleanup()
  }
})

test('.test.rs 排除：反例形状只出现在夹具测试里 ⇒ 通过（不是生产路径）', () => {
  const files = cleanFiles()
  files[TOOL_PORT_TEST] = '#[test]\nfn t() { let _ = p2.route(ctx); }\n'
  const box = sandbox(files)
  try {
    const r = run(box.root)
    assert.equal(r.status, 0, r.out)
  } finally {
    box.cleanup()
  }
})

test('输入失效：空夹具树（crates/ 下 0 个生产 .rs）⇒ 红，不退化成绿灯', () => {
  const box = sandbox({})
  try {
    const r = run(box.root)
    assert.equal(r.status, 1, r.out)
    assert.match(r.out, /读不出/)
  } finally {
    box.cleanup()
  }
})
