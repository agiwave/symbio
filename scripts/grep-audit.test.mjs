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
