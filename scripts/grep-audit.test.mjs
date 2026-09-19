import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const script = fileURLToPath(new URL('./grep-audit.mjs', import.meta.url))
function audit(source, { waiver = '', strict = false, scope = false } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'grep-audit-'))
  try {
    const plugins = path.join(root, 'symbio/src/plugins')
    fs.mkdirSync(path.join(plugins, 'other'), { recursive: true })
    fs.writeFileSync(path.join(plugins, 'other/sample.rs'), source.replace('WAIVER', waiver))
    const env = { ...process.env, NO_COLOR: '1' }
    delete env.SCOPE
    if (scope) env.SCOPE = plugins
    const result = spawnSync(process.execPath, [script, ...(strict ? ['--strict'] : [])], {
      cwd: root, env, encoding: 'utf8', timeout: 10000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}
const suspect = `use std::sync::Mutex;
async fn example() {
    let guard = mutex.lock().unwrap(); WAIVER
    work().await;
    drop(guard);
}
`
test('default scope checks a non-agent plugin', () => {
  const r = audit(suspect)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /other\/sample.rs:3/)
})
test('test markers do not suppress S-002', () => {
  assert.equal(audit('#[tokio::test]\n' + suspect).status, 1)
})
test('explicit scope still works', () => {
  assert.equal(audit(suspect, { scope: true }).status, 1)
})
test('reviewed same-line waiver requires a reason', () => {
  assert.equal(audit(suspect, { waiver: '// grep-audit-allow S-002: reviewed fixture' }).status, 0)
  assert.equal(audit(suspect, { waiver: '// grep-audit-allow S-002:   ' }).status, 1)
})
test('waiver does not suppress another lock', () => {
  const source = suspect + suspect.replace('WAIVER', '')
  assert.equal(audit(source, { waiver: '// grep-audit-allow S-002: reviewed fixture' }).status, 1)
})
test('warnings fail only in strict mode', () => {
  const source = 'async fn example() {\n let _ = work().await;\n}\n'
  assert.equal(audit(source).status, 0)
  assert.equal(audit(source, { strict: true }).status, 2)
})

// ── S-008：VdfsNode.status 不得用裸字面量 ──────────────────────────────
// 词表只有 `VDFS_STATUS_*` 一套；裸字面量在改名时不会编译失败（该词表曾把
// `error` 改名为 `failed`，留下过化石，见 vdfs_provider.rs::VDFS_STATUS_FAILED）。
const statusSuspect = `fn node_of() -> VdfsNode {
    let mut n = VdfsNode::file();
    n.status = "active".to_string(); WAIVER
    n
}
`
test('S-008 fires on a bare status literal', () => {
  const r = audit(statusSuspect)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /sample\.rs:3/)
})
test('S-008 fires on a literal inside an if branch', () => {
  const r = audit(`fn node_of(enabled: bool) -> VdfsNode {
    let mut n = VdfsNode::file();
    n.status = if enabled {
        "active".to_string()
    } else {
        "disabled".to_string()
    };
    n
}
`)
  assert.equal(r.status, 1)
})
test('S-008 fires on with_status receiving a bare literal', () => {
  const r = audit('fn f() -> OptionNode {\n    OptionNode::new("a", "A").with_status("disabled")\n}\n')
  assert.equal(r.status, 1)
})
test('S-008 stays silent when the word comes from a constant', () => {
  const r = audit(
    'fn f() -> VdfsNode {\n    let mut n = VdfsNode::file();\n    n.status = VDFS_STATUS_ACTIVE.to_string();\n    n\n}\n',
  )
  assert.equal(r.status, 0)
})
test('S-008 does not mistake format! / response envelope for a status word', () => {
  // 已知边界：`status: "success"`（响应信封）是另一套词表，按位置判会误报，故只认
  // `.status = ` 与 `with_status(` 两处。`format!` 同样不算字面量赋值。
  const r = audit(
    'fn f(k: &str) -> VdfsNode {\n    let mut n = VdfsNode::file();\n    n.status = format!("{k}");\n    n\n}\n',
  )
  assert.equal(r.status, 0)
})
test('S-008 waiver requires a reason', () => {
  assert.equal(audit(statusSuspect, { waiver: '// grep-audit-allow S-008: reviewed fixture' }).status, 0)
  assert.equal(audit(statusSuspect, { waiver: '// grep-audit-allow S-008:   ' }).status, 1)
})

// ── S-009：事件总线 kind 不得用裸字面量 ────────────────────────────────
// 词表只有 `symbio_core::event_bus::KIND_*` 一套；`kind` 是跨进程字符串，裸字面量
// 改名时不会编译失败。本规则拦的正是「发布点写字面量 → 常量无人引用」的**成因**
// （那正是 R-001 当初报出 `KIND_SESSION` / `KIND_SYSTEM` 的由来）。
const kindSuspect = `async fn emit() {
    EventBus::publish("session", None, data).await; WAIVER
}
`
test('S-009 fires on a bare kind literal', () => {
  const r = audit(kindSuspect)
  assert.equal(r.status, 1)
  assert.match(r.stdout, /sample\.rs:2/)
})
test('S-009 fires on try_publish and on a receiver call', () => {
  assert.equal(audit('fn f() {\n    EventBus::try_publish("vdfs", None, d);\n}\n').status, 1)
  assert.equal(audit('fn f() {\n    bus.publish("system", None, d);\n}\n').status, 1)
})
test('S-009 stays silent when the kind comes from a constant', () => {
  assert.equal(audit('fn f() {\n    EventBus::try_publish(KIND_SESSION, None, d);\n}\n').status, 0)
})
test('S-009 does not mistake a non-literal first argument', () => {
  assert.equal(audit('fn f(kind: &str) {\n    EventBus::try_publish(kind, None, d);\n}\n').status, 0)
})
test('S-009 waiver requires a reason', () => {
  assert.equal(audit(kindSuspect, { waiver: '// grep-audit-allow S-009: reviewed fixture' }).status, 0)
  assert.equal(audit(kindSuspect, { waiver: '// grep-audit-allow S-009:   ' }).status, 1)
})
