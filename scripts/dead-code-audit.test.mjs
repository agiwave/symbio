import assert from 'node:assert/strict'
import { test } from 'node:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

// `dead-code-audit.mjs` 的 R-001（Rust 声明级）是**判定型**守卫，按本仓教条必须
// 有「注入真实违规并断言脚本变红」的回归测试——否则规则写错了也不会有人发现。
//
// 脚本的仓库根由 `--root=` 注入，故这里在临时目录造一棵**最小仓库**：
//   <root>/tauri/index.html  → /src/main.ts   （前端侧需要一个可达入口，否则
//   <root>/tauri/src/main.ts                   前端分支会把 fixture 当死代码）
//   <root>/symbio/src/*.rs                    （Rust 侧被检对象）
const script = fileURLToPath(new URL('./dead-code-audit.mjs', import.meta.url))

function audit(rustFiles, { waiver = null } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'dead-code-audit-'))
  try {
    fs.mkdirSync(path.join(root, 'tauri/src'), { recursive: true })
    fs.writeFileSync(
      path.join(root, 'tauri/index.html'),
      '<script type="module" src="/src/main.ts"></script>\n',
    )
    fs.writeFileSync(path.join(root, 'tauri/src/main.ts'), 'export const app = 1\n')
    fs.mkdirSync(path.join(root, 'symbio/src'), { recursive: true })
    for (const [name, src] of Object.entries(rustFiles)) {
      const text = waiver === null ? src : src.replace('WAIVER', waiver)
      fs.writeFileSync(path.join(root, 'symbio/src', name), text)
    }
    const env = { ...process.env, NO_COLOR: '1' }
    const result = spawnSync(process.execPath, [script, `--root=${root}`], {
      cwd: root,
      env,
      encoding: 'utf8',
      timeout: 20000,
    })
    assert.ifError(result.error)
    return result
  } finally {
    fs.rmSync(root, { recursive: true, force: true })
  }
}

test('R-001 fires on a pub declaration nobody mentions', () => {
  const r = audit({ 'a.rs': 'pub fn orphan_helper() -> i32 {\n    1\n}\n' })
  assert.equal(r.status, 1)
  assert.match(r.stdout, /R-001/)
  assert.match(r.stdout, /a\.rs :: fn orphan_helper/)
})

test('R-001 stays silent when the name is mentioned outside its own line', () => {
  // `pub use` 不是 DECL_RE 认的声明形态，因此参照文件只贡献"名字出现过一次"
  const r = audit({
    'a.rs': 'pub fn shared_helper() -> i32 {\n    1\n}\n',
    'b.rs': 'pub use a::shared_helper;\n',
  })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /无全仓零引用的 pub 声明/)
})

test('R-001 counts a same-file mention as a real use', () => {
  // 同一文件内的引用同样是真引用（`#[serde(default = "…")]` 那类只在声明文件里出现）
  const r = audit({
    'a.rs': 'pub fn local_default() -> i32 {\n    1\n}\npub fn other() -> i32 {\n    local_default()\n}\n',
  })
  assert.equal(r.status, 1, 'other 自身零引用，仍应被抓——用它反证 local_default 没被误报')
  assert.doesNotMatch(r.stdout, /local_default/)
})

test('R-001 waiver requires a non-empty reason', () => {
  const src = 'pub const KEPT_THING: &str = "x"; WAIVER\n'
  assert.equal(
    audit({ 'a.rs': src }, { waiver: '// dead-code-allow R-001: 消费方在前端' }).status,
    0,
  )
  assert.equal(
    audit({ 'a.rs': src }, { waiver: '// dead-code-allow R-001:   ' }).status,
    1,
    '空理由视为未承认——否则加个注释就能过，守卫退化成橡皮图章',
  )
})

test('R-001 reports waived items separately from violations', () => {
  const src = 'pub const KEPT_THING: &str = "x"; WAIVER\n'
  const r = audit({ 'a.rs': src }, { waiver: '// dead-code-allow R-001: 闭集成员' })
  assert.equal(r.status, 0)
  assert.match(r.stdout, /已承认保留/)
  assert.match(r.stdout, /闭集成员/)
})

test('an empty fixture passes (guard is not vacuously red)', () => {
  const r = audit({})
  assert.equal(r.status, 0)
})
